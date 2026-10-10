//! A player's connection to a campaign: [`PlayerSession`] keeps a
//! [`Replica`], talks to the GM's [`crate::host::AuthorityHost`] over
//! chummer-net when it is reachable, and falls back to the relay mailbox
//! when it is not (play-by-post).
//!
//! Usage: [`PlayerSession::new`], then [`PlayerSession::sync`] on start and
//! every so often (it connects when it can, sends the outbox, and
//! otherwise mails it and collects mail). Edits go through
//! [`PlayerSession::edit`]; [`PlayerSession::events`] says what changed.
//!
//! Dice rolls ([`PlayerSession::roll_now`]) travel the way edits do, on
//! a path of their own: kept in the replica's roll outbox (and journaled
//! until a save has them), sent live when the GM's app is reachable and
//! mailed when it is not, mailed again when unanswered for
//! [`PlayerConfig::remail_after`], and dropped only once the authority
//! names them in its answer. The authority logs each roll once by its id,
//! so a roll sent twice is no harm. They never touch a character.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chummer_core::command::{Command, Rejected, Report};
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignClient, DenyReason};
use chummer_net::invite::{InviteLink, Role};
use chummer_net::mailbox::{MailboxClient, MailboxError, Registration};
use chummer_net::node::dial_addr;
use chummer_net::{Endpoint, EndpointId, NetError, PublicKey, SecretKey};
use tokio::sync::mpsc;

use crate::mail::{self, DEFAULT_BLOB_LIMIT};
use crate::msg::{self, CharacterId, ClaimProof, ClientMessage, MailMessage, OpId, RollId, RollReport, ServerMessage};
use crate::lockwatch::{self, Guard};
use crate::replica::{Event, Replica};

/// How a player reaches their campaign.
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    /// Shown to the GM.
    pub name: String,
    /// The GM's node id, the campaign, and the invite token for the first
    /// join.
    pub link: InviteLink,
    /// The relay mailbox for play-by-post (`RelayEntry::mailbox`).
    pub mailbox: Option<EndpointId>,
    /// Where the replica is saved.
    pub path: Option<PathBuf>,
    pub connect_timeout: Duration,
    /// Commands mailed this long ago and still not answered are mailed
    /// again (with a join message, so the GM also sends what we missed):
    /// mail can expire on the relay or be lost with its data. The GM runs
    /// each command once however often it arrives.
    pub remail_after: Duration,
}

/// Default [`PlayerConfig::remail_after`].
pub const REMAIL_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

impl PlayerConfig {
    pub fn new(name: impl Into<String>, link: InviteLink) -> PlayerConfig {
        PlayerConfig { name: name.into(), link, mailbox: None, path: None, connect_timeout: Duration::from_secs(15), remail_after: REMAIL_AFTER }
    }
}

/// What [`PlayerSession::sync`] managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Connected to the GM's app; the outbox was sent.
    Online,
    /// The GM's app was not reachable; the outbox went to the mailbox and
    /// the mail was collected.
    Mailbox,
    /// Neither the GM nor a mailbox could be reached; edits stay queued.
    Offline,
}

struct Inner {
    endpoint: Endpoint,
    secret: SecretKey,
    engine: Arc<Engine>,
    cfg: PlayerConfig,
    replica: Mutex<Replica>,
    client: Mutex<Option<Arc<CampaignClient>>>,
    role: Mutex<Option<Role>>,
    mailbox: tokio::sync::Mutex<Option<MailboxClient>>,
    blob_limit: Mutex<usize>,
    events: mpsc::UnboundedSender<Event>,
    /// Wakes the task that sends edits made with [`PlayerSession::edit_now`].
    flush: tokio::sync::Notify,
    /// Wakes [`PlayerSession::keep_synced`] early.
    wake: tokio::sync::Notify,
    last_mode: Mutex<Option<SyncMode>>,
    closed: std::sync::atomic::AtomicBool,
    /// When each mailed command was (last) mailed, as far as this run
    /// knows; and when we last mailed a join message.
    mailed_at: Mutex<std::collections::HashMap<OpId, Instant>>,
    /// The same for mailed rolls.
    rolls_mailed_at: Mutex<std::collections::HashMap<RollId, Instant>>,
    joined_by_mail: Mutex<Instant>,
    /// A join was mailed in this run (a first join by mail is sent once,
    /// then again after `remail_after`).
    mailed_join: std::sync::atomic::AtomicBool,
    /// The GM keys last registered with our relay mailbox.
    registered: tokio::sync::Mutex<Option<Vec<PublicKey>>>,
    /// The label from the last welcome.
    label: Mutex<Option<String>>,
    /// The membership's label, as last read (see [`PlayerSession::label`]).
    membership_label: Mutex<Option<String>>,
    /// The runtime the session was made in: saves asked for by a UI
    /// thread run on its blocking threads.
    rt: Option<tokio::runtime::Handle>,
    /// A background save is asked for and has not started yet.
    save_queued: std::sync::atomic::AtomicBool,
    /// One save at a time (they write the same file), so an older state
    /// never overwrites a newer one.
    saving: Mutex<()>,
    /// The outbox journal (see [`PlayerSession::edit_now`]).
    outbox_log: Mutex<OutboxLog>,
    /// The roll journal (see [`PlayerSession::roll_now`]).
    roll_log: Mutex<OutboxLog>,
}

/// `<replica>.outbox`: every command made, appended as it is made, until
/// a save has it (and `<replica>.rolls` the same for rolls, records of
/// [`RollReport`]). Saves run in the background, so without it an app that
/// crashed right after an edit lost the edit (found by the e2e
/// `player-crash` scenario). Each record is a 4-byte big-endian length and
/// the postcard form of `(CharacterId, Pending)`; a torn last record is
/// ignored. Written without fsync: it covers the app crashing, and the
/// save that follows within moments syncs to disk.
#[derive(Debug, Default)]
struct OutboxLog {
    file: Option<std::fs::File>,
    /// The highest sequence number appended since the log was emptied.
    max_seq: u64,
}

fn outbox_log_path(replica: &std::path::Path) -> PathBuf {
    let mut p = replica.as_os_str().to_owned();
    p.push(".outbox");
    PathBuf::from(p)
}

fn roll_log_path(replica: &std::path::Path) -> PathBuf {
    let mut p = replica.as_os_str().to_owned();
    p.push(".rolls");
    PathBuf::from(p)
}

fn read_outbox_log<T: for<'de> serde::Deserialize<'de>>(path: &std::path::Path) -> Vec<T> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    let mut out = Vec::new();
    let mut rest = &bytes[..];
    while rest.len() >= 4 {
        let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if rest.len() < 4 + len {
            break;
        }
        match postcard::from_bytes(&rest[4..4 + len]) {
            Ok(r) => out.push(r),
            Err(_) => break,
        }
        rest = &rest[4 + len..];
    }
    out
}

/// A player's session. Cheap to clone.
#[derive(Clone)]
pub struct PlayerSession {
    inner: Arc<Inner>,
    events: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Event>>>,
}

impl std::fmt::Debug for PlayerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerSession").field("host", &self.inner.cfg.link.host.fmt_short().to_string()).field("online", &self.is_online()).finish()
    }
}

impl PlayerSession {
    /// A session on `endpoint` (bound with this machine's `secret`). The
    /// replica is loaded from `cfg.path` when that file exists, else a new
    /// one is made.
    pub fn new(endpoint: Endpoint, secret: SecretKey, engine: Arc<Engine>, cfg: PlayerConfig) -> Result<PlayerSession, crate::persist::PersistError> {
        let replica = match &cfg.path {
            Some(p) if p.exists() => {
                let mut r = Replica::load(&engine, p)?;
                // Commands made after the last save (the app crashed).
                let log = outbox_log_path(p);
                let made = read_outbox_log(&log);
                if !made.is_empty() {
                    let n = r.recover_outbox(&engine, made);
                    if n > 0 {
                        tracing::info!("took back {n} change(s) made after the last save");
                    }
                    r.save(p)?;
                }
                let _ = std::fs::remove_file(&log);
                // Rolls made after the last save.
                let log = roll_log_path(p);
                let rolled = read_outbox_log(&log);
                if !rolled.is_empty() {
                    let n = r.recover_rolls(rolled);
                    if n > 0 {
                        tracing::info!("took back {n} roll(s) made after the last save");
                    }
                    r.save(p)?;
                }
                let _ = std::fs::remove_file(&log);
                r
            }
            _ => Replica::new(),
        };
        Ok(PlayerSession::with_replica(endpoint, secret, engine, cfg, replica))
    }

    pub fn with_replica(endpoint: Endpoint, secret: SecretKey, engine: Arc<Engine>, cfg: PlayerConfig, replica: Replica) -> PlayerSession {
        let s = PlayerSession::make(endpoint, secret, engine, cfg, replica);
        s.start_sender();
        s
    }

    fn make(endpoint: Endpoint, secret: SecretKey, engine: Arc<Engine>, cfg: PlayerConfig, replica: Replica) -> PlayerSession {
        let (tx, rx) = mpsc::unbounded_channel();
        PlayerSession {
            inner: Arc::new(Inner {
                endpoint,
                secret,
                engine,
                cfg,
                replica: Mutex::new(replica),
                client: Mutex::new(None),
                role: Mutex::new(None),
                mailbox: tokio::sync::Mutex::new(None),
                blob_limit: Mutex::new(DEFAULT_BLOB_LIMIT),
                events: tx,
                flush: tokio::sync::Notify::new(),
                wake: tokio::sync::Notify::new(),
                last_mode: Mutex::new(None),
                closed: std::sync::atomic::AtomicBool::new(false),
                mailed_at: Mutex::default(),
                rolls_mailed_at: Mutex::default(),
                joined_by_mail: Mutex::new(Instant::now()),
                mailed_join: std::sync::atomic::AtomicBool::new(false),
                registered: tokio::sync::Mutex::new(None),
                label: Mutex::new(None),
                membership_label: Mutex::new(None),
                rt: tokio::runtime::Handle::try_current().ok(),
                save_queued: std::sync::atomic::AtomicBool::new(false),
                saving: Mutex::new(()),
                outbox_log: Mutex::default(),
                roll_log: Mutex::default(),
            }),
            events: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }

    fn from_weak(inner: &std::sync::Weak<Inner>, events: &std::sync::Weak<tokio::sync::Mutex<mpsc::UnboundedReceiver<Event>>>) -> Option<PlayerSession> {
        let s = PlayerSession { inner: inner.upgrade()?, events: events.upgrade()? };
        (!s.is_closed()).then_some(s)
    }

    /// The link this session joins with.
    pub fn link(&self) -> &InviteLink {
        &self.inner.cfg.link
    }

    pub fn name(&self) -> &str {
        &self.inner.cfg.name
    }

    /// How the last [`PlayerSession::sync`] went (`None` before the first).
    pub fn last_mode(&self) -> Option<SyncMode> {
        if self.is_online() {
            return Some(SyncMode::Online);
        }
        *self.inner.last_mode.lock().expect("poisoned")
    }

    /// Ends [`PlayerSession::keep_synced`] and the send task, and hangs up.
    pub fn close(&self) {
        self.inner.closed.store(true, std::sync::atomic::Ordering::Release);
        self.inner.wake.notify_waiters();
        self.inner.flush.notify_waiters();
        self.disconnect();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Asks [`PlayerSession::keep_synced`] to sync now.
    pub fn sync_soon(&self) {
        self.inner.wake.notify_one();
    }

    /// Syncs on start and then every `every` (sooner after
    /// [`PlayerSession::sync_soon`], and as soon as a live connection
    /// drops), until [`PlayerSession::close`]. Holds the session weakly, so
    /// dropping every other handle ends it too.
    pub async fn keep_synced(self, every: Duration) {
        let (inner, events) = (Arc::downgrade(&self.inner), Arc::downgrade(&self.events));
        drop(self);
        loop {
            let Some(s) = PlayerSession::from_weak(&inner, &events) else { return };
            let mode = s.sync().await;
            let wake = s.inner.clone();
            drop(s);
            let wait = async {
                if mode == SyncMode::Online {
                    // Until the connection ends (or `every`, to be safe).
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        match inner.upgrade() {
                            Some(i) if i.client.lock().expect("poisoned").is_some() && !i.closed.load(std::sync::atomic::Ordering::Acquire) => {}
                            _ => break,
                        }
                    }
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                _ = wait => {}
                _ = tokio::time::sleep(every) => {}
                _ = wake.wake.notified() => {}
            }
        }
    }

    /// As [`PlayerSession::edit`], without waiting: applied to the local
    /// copy and saved right after on a background thread (for a UI
    /// thread); sent by a background task
    /// when online, else kept for the next [`PlayerSession::sync`]. Must be
    /// called after the session was made inside a tokio runtime.
    pub fn edit_now(&self, id: &CharacterId, cmd: Command) -> Result<Report, Rejected> {
        let report = self.edit_logged(id, cmd)?;
        self.save_soon();
        if report.changed {
            self.inner.flush.notify_one();
        }
        Ok(report)
    }

    /// Applies `cmd` to the replica and appends the queued command to the
    /// outbox journal before anything else can happen to it.
    fn edit_logged(&self, id: &CharacterId, cmd: Command) -> Result<Report, Rejected> {
        let (report, made) = {
            let mut r = self.replica();
            let report = r.edit(&self.inner.engine, id, cmd)?;
            let made = if report.changed { r.outbox(id).last().cloned() } else { None };
            (report, made)
        };
        if let (Some(p), Some(path)) = (made, &self.inner.cfg.path) {
            if let Err(e) = self.append_outbox_log(path, id, &p) {
                tracing::warn!("could not write the outbox journal (a crash before the next save would lose this change): {e}");
            }
        }
        Ok(report)
    }

    fn append_outbox_log(&self, replica: &std::path::Path, id: &CharacterId, p: &crate::replica::Pending) -> std::io::Result<()> {
        append_log(&self.inner.outbox_log, &outbox_log_path(replica), &(id, p), p.op.id.seq)
    }

    /// A save with commands up to `seq` is on disk: empty the journal
    /// when it holds nothing newer.
    fn trim_outbox_log(&self, replica: &std::path::Path, seq: u64) {
        trim_log(&self.inner.outbox_log, &outbox_log_path(replica), seq);
    }

    /// Rolls `roll` (made for `id`, our character) into the replica's roll
    /// outbox and the roll journal, and sends it as an edit would be: at
    /// once when online, else with the next [`PlayerSession::sync`]
    /// (mailed when the GM's app is not reachable). For a UI thread, as
    /// [`PlayerSession::edit_now`].
    pub fn roll_now(&self, id: &CharacterId, roll: chummer_core::dice::RollRecord) -> Result<RollReport, String> {
        let report = self.replica().roll(id, roll)?;
        if let Some(path) = &self.inner.cfg.path {
            if let Err(e) = append_log(&self.inner.roll_log, &roll_log_path(path), &report, report.id.seq) {
                tracing::warn!("could not write the roll journal (a crash before the next save would lose this roll): {e}");
            }
        }
        self.save_soon();
        self.inner.flush.notify_one();
        Ok(report)
    }

    /// Takes the refused commands out of the list (the player saw them).
    pub fn dismiss_refused(&self) -> Vec<crate::replica::Refused> {
        let r = self.replica().dismiss_refused();
        self.save_soon();
        r
    }

    fn start_sender(&self) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let (inner, events) = (Arc::downgrade(&self.inner), Arc::downgrade(&self.events));
        rt.spawn(async move {
            loop {
                let Some(i) = inner.upgrade() else { return };
                let notified = async move { i.flush.notified().await };
                notified.await;
                let Some(s) = PlayerSession::from_weak(&inner, &events) else { return };
                if let Some(c) = s.client() {
                    if let Err(e) = s.flush_live(&c).await {
                        tracing::info!("could not send the change, kept for later: {e}");
                        s.drop_client(&c);
                    }
                }
            }
        });
    }

    /// The local copies, locked. Do not hold across an `.await`.
    pub fn replica(&self) -> Guard<'_, Replica> {
        lockwatch::lock(&self.inner.replica, "replica")
    }

    /// The local copies, when no other thread holds them (a UI thread
    /// polls with this so it never waits for a network task).
    pub fn try_replica(&self) -> Option<Guard<'_, Replica>> {
        lockwatch::try_lock(&self.inner.replica, "replica")
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.inner.engine
    }

    pub fn is_online(&self) -> bool {
        self.inner.client.lock().expect("poisoned").is_some()
    }

    /// The role the GM gave us (known after the first connection).
    pub fn role(&self) -> Option<Role> {
        *self.inner.role.lock().expect("poisoned")
    }

    /// The name the GM gave our invite ("Anna"), once known.
    pub fn label(&self) -> Option<String> {
        // Once joined, the membership says (and keeps saying, as the GM
        // renames or re-labels): empty for members the GM added by node
        // id. The Welcome's label is only a first answer; for those
        // members it is their member name, which went stale when the join
        // renamed them.
        // While the replica is busy (a background save), the last answer.
        let mut seen = self.inner.membership_label.lock().expect("poisoned");
        if let Some(r) = self.try_replica() {
            if let Some(m) = r.membership() {
                *seen = Some(m.label.clone());
            }
        }
        match &*seen {
            Some(l) => Some(l.clone()).filter(|l| !l.is_empty()),
            None => self.inner.label.lock().expect("poisoned").clone().filter(|l| !l.is_empty()),
        }
    }

    /// Why the GM's app refused us, if it did (until a join works).
    pub fn denied(&self) -> Option<DenyReason> {
        self.try_replica().and_then(|r| r.denied().cloned())
    }

    /// The key our mailbox puts are signed with: the invite's member key,
    /// or (for members the GM added by node id) our node key.
    fn signer(&self) -> SecretKey {
        self.inner.cfg.link.member.as_ref().map(|m| m.key()).unwrap_or_else(|| self.inner.secret.clone())
    }

    /// The GM's campaign keys our mailbox should take mail from: the
    /// membership's (they follow rotations), else the link's.
    fn gm_keys(&self) -> Vec<PublicKey> {
        let from_membership = self.replica().membership().map(|m| m.gm_keys.clone()).unwrap_or_default();
        if !from_membership.is_empty() {
            return from_membership;
        }
        self.inner.cfg.link.gm_key.into_iter().collect()
    }

    /// A mailed join's proof of the invite's member key.
    fn claim(&self) -> Option<ClaimProof> {
        let link = &self.inner.cfg.link;
        link.member.as_ref().map(|m| ClaimProof::new(&m.key(), &link.campaign, &link.host, &self.inner.endpoint.id()))
    }

    /// Lets the GM's campaign keys put mail into our relay mailbox
    /// (scope: the campaign), from the GM's node only, when that changed
    /// since the last time.
    /// A relay-side refusal (a full disk) is logged, not returned: our own
    /// mail still goes out.
    async fn ensure_registered(&self, mb: &MailboxClient) -> Result<(), NetError> {
        let keys = self.gm_keys();
        if keys.is_empty() {
            return Ok(());
        }
        let mut reg = self.inner.registered.lock().await;
        if reg.as_ref() != Some(&keys) {
            let host = self.inner.cfg.link.host;
            match mb.register(self.inner.cfg.link.campaign.0, keys.iter().map(|k| Registration::bound(*k, host))).await {
                Ok(_) => *reg = Some(keys),
                Err(NetError::Mailbox(e)) => tracing::warn!("could not register the GM's key with the mailbox: {e}"),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Stops the GM's keys from putting mail into our mailbox (leaving the
    /// campaign). Best effort.
    pub async fn unregister(&self) -> Result<(), NetError> {
        let mb = self.mailbox_client().await?;
        mb.register(self.inner.cfg.link.campaign.0, Vec::<Registration>::new()).await?;
        *self.inner.registered.lock().await = None;
        Ok(())
    }

    /// What happened since the last call: refused commands, updates,
    /// membership changes. Empty while [`PlayerSession::next_event`] waits.
    pub fn events(&self) -> Vec<Event> {
        let mut out = Vec::new();
        if let Ok(mut rx) = self.events.try_lock() {
            while let Ok(e) = rx.try_recv() {
                out.push(e);
            }
        }
        out
    }

    /// Waits for the next event.
    pub async fn next_event(&self) -> Option<Event> {
        self.events.lock().await.recv().await
    }

    /// Saves the replica (when the session has a path). The snapshots
    /// (slow) are made without holding the replica's lock.
    pub fn save(&self) -> std::io::Result<()> {
        match &self.inner.cfg.path {
            Some(p) => {
                let _one = self.inner.saving.lock().unwrap_or_else(|e| e.into_inner());
                let work = self.replica().snapshot_work();
                if !work.is_empty() {
                    let made = work.into_iter().map(|(id, at, ch)| (id, at, chummer_core::command::snapshot(&ch))).collect();
                    self.replica().put_snapshots(made);
                }
                let (bytes, seq, roll_seq) = {
                    let r = self.replica();
                    (r.to_bytes(), r.last_seq(), r.last_roll_seq())
                };
                crate::persist::write_atomic(p, &bytes)?;
                self.trim_outbox_log(p, seq);
                trim_log(&self.inner.roll_log, &roll_log_path(p), roll_seq);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// [`PlayerSession::save`] on a blocking thread of the session's
    /// runtime (a UI thread must not wait for the disk); saves asked for
    /// while one waits to start are one save.
    fn save_soon(&self) {
        let Some(rt) = self.inner.rt.clone() else { return self.save_logged() };
        if self.inner.save_queued.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        rt.spawn_blocking(move || {
            me.inner.save_queued.store(false, std::sync::atomic::Ordering::Release);
            me.save_logged();
        });
    }

    fn save_logged(&self) {
        if let Err(e) = self.save() {
            tracing::warn!("could not save the campaign copy: {e}");
        }
    }

    fn emit(&self, events: Vec<Event>) {
        for e in events {
            let _ = self.inner.events.send(e);
        }
    }

    /// Handles a message, saves, then tells the UI (so an event means the
    /// change is on disk).
    fn handle(&self, msg: ServerMessage) -> Vec<crate::msg::ResyncRequest> {
        let events = self.replica().handle(&self.inner.engine, msg);
        let resync = events.iter().filter_map(|e| if let Event::NeedResync(r) = e { Some(r.clone()) } else { None }).collect();
        self.save_logged();
        self.emit(events);
        resync
    }

    // ----- live connection -----

    /// Dials the GM's app and joins. On success the outbox is sent. A
    /// refusal is kept ([`PlayerSession::denied`]) and reported as an
    /// [`Event::Denied`].
    pub async fn connect(&self) -> Result<Role, NetError> {
        let link = &self.inner.cfg.link;
        let member = link.member.as_ref().map(|m| m.key());
        let join = CampaignClient::join(&self.inner.endpoint, dial_addr(link.host, link.relay.as_ref()), link.campaign, member.as_ref());
        let joined = tokio::time::timeout(self.inner.cfg.connect_timeout, join).await.map_err(|_| NetError::Connect("timed out".into()))?;
        let (client, mut pushes) = match joined {
            Ok(x) => x,
            Err(NetError::Denied(reason)) => {
                let changed = self.replica().denied() != Some(&reason);
                if changed {
                    self.replica().set_denied(Some(reason.clone()));
                    self.save_logged();
                    self.emit(vec![Event::Denied(reason.clone())]);
                }
                return Err(NetError::Denied(reason));
            }
            Err(e) => return Err(e),
        };
        let client = Arc::new(client);
        let role = client.welcome().role;
        *self.inner.role.lock().expect("poisoned") = Some(role);
        *self.inner.label.lock().expect("poisoned") = Some(client.welcome().label.clone());
        let join_msg = self.replica().join_message(&self.inner.cfg.name, None);
        let reply = match request(&client, &join_msg).await {
            Ok(r) => r,
            Err(e) => {
                client.close();
                return Err(e);
            }
        };
        *self.inner.client.lock().expect("poisoned") = Some(client.clone());
        let resync = self.handle(reply);
        self.save_logged();

        // Pushes from the GM, until the connection ends.
        let me = self.clone();
        let conn = client.clone();
        tokio::spawn(async move {
            while let Some(bytes) = pushes.recv().await {
                match msg::decode::<ServerMessage>(&bytes) {
                    Ok(m) => {
                        let resync = me.handle(m);
                        me.save_logged();
                        if !resync.is_empty() {
                            let _ = me.resync_live(&conn, resync).await;
                        }
                    }
                    Err(e) => tracing::debug!("bad push: {e}"),
                }
            }
            let mut c = me.inner.client.lock().expect("poisoned");
            if c.as_ref().is_some_and(|c| Arc::ptr_eq(c, &conn)) {
                *c = None;
            }
        });

        self.resync_live(&client, resync).await?;
        self.flush_live(&client).await?;
        Ok(role)
    }

    async fn resync_live(&self, client: &CampaignClient, reqs: Vec<crate::msg::ResyncRequest>) -> Result<(), NetError> {
        for r in reqs {
            let reply = request(client, &ClientMessage::Resync(r)).await?;
            let _ = self.handle(reply);
        }
        self.save_logged();
        Ok(())
    }

    async fn flush_live(&self, client: &CampaignClient) -> Result<(), NetError> {
        let batches = self.replica().batches();
        for b in batches {
            let reply = request(client, &ClientMessage::Submit(b)).await?;
            let resync = self.handle(reply);
            self.save_logged();
            self.resync_live(client, resync).await?;
        }
        // Rolls after the edits they may have been made after.
        let rolls = self.replica().rolls_message();
        if let Some(m) = rolls {
            let reply = request(client, &m).await?;
            let _ = self.handle(reply);
        }
        Ok(())
    }

    fn client(&self) -> Option<Arc<CampaignClient>> {
        self.inner.client.lock().expect("poisoned").clone()
    }

    /// Hangs up (the session keeps working offline).
    pub fn disconnect(&self) {
        if let Some(c) = self.inner.client.lock().expect("poisoned").take() {
            c.close();
        }
    }

    // ----- editing -----

    /// Applies `cmd` to the local copy and queues it; when online it is
    /// sent at once. Network trouble does not fail the edit: it stays in
    /// the outbox for the next [`PlayerSession::sync`].
    pub async fn edit(&self, id: &CharacterId, cmd: Command) -> Result<Report, Rejected> {
        let report = self.edit_logged(id, cmd)?;
        self.save_logged();
        if let Some(c) = self.client() {
            if let Err(e) = self.flush_live(&c).await {
                tracing::info!("could not send the change, kept for later: {e}");
                self.drop_client(&c);
            }
        }
        Ok(report)
    }

    fn drop_client(&self, c: &Arc<CampaignClient>) {
        let mut cur = self.inner.client.lock().expect("poisoned");
        if cur.as_ref().is_some_and(|x| Arc::ptr_eq(x, c)) {
            *cur = None;
        }
    }

    // ----- mailbox -----

    async fn mailbox_client(&self) -> Result<MailboxClient, NetError> {
        let id = self.inner.cfg.mailbox.ok_or_else(|| NetError::Connect("no mailbox configured".into()))?;
        let mut mb = self.inner.mailbox.lock().await;
        if let Some(c) = mb.as_ref() {
            return Ok(c.clone());
        }
        let c = tokio::time::timeout(self.inner.cfg.connect_timeout, MailboxClient::connect(&self.inner.endpoint, dial_addr(id, None)))
            .await
            .map_err(|_| NetError::Connect("mailbox timed out".into()))??;
        *mb = Some(c.clone());
        Ok(c)
    }

    async fn forget_mailbox(&self) {
        *self.inner.mailbox.lock().await = None;
        // A new connection registers again (the relay may have lost it).
        *self.inner.registered.lock().await = None;
    }

    /// Mails the commands not mailed yet (and resync requests) to the GM.
    /// Returns the number of blobs stored.
    ///
    /// The relay refuses our invite key from this device when another
    /// device claimed the invite ([`MailboxError::WrongDevice`]): that is
    /// kept as the GM's "claimed" refusal ([`PlayerSession::denied`]).
    pub async fn send_mail(&self) -> Result<usize, NetError> {
        let r = self.send_mail_inner().await;
        if let Err(NetError::Mailbox(MailboxError::WrongDevice)) = &r {
            if self.replica().denied() != Some(&DenyReason::Claimed) {
                tracing::info!("the relay refused our invite key: the invite was claimed on another device");
                self.replica().set_denied(Some(DenyReason::Claimed));
                self.save_logged();
                self.emit(vec![Event::Denied(DenyReason::Claimed)]);
            }
        }
        r
    }

    async fn send_mail_inner(&self) -> Result<usize, NetError> {
        let mb = self.mailbox_client().await?;
        self.ensure_registered(&mb).await?;
        let host = self.inner.cfg.link.host;
        let signer = self.signer();
        let mut sent = 0;
        let stale = self.stale_mail();
        if !stale.is_empty() {
            tracing::info!("{} command(s) mailed long ago are still not answered; mailing them again", stale.len());
            self.replica().mark_unmailed(&stale);
        }
        let stale_rolls = self.stale_rolls();
        if !stale_rolls.is_empty() {
            tracing::info!("{} roll(s) mailed long ago are still not answered; mailing them again", stale_rolls.len());
            self.replica().mark_rolls_unmailed(&stale_rolls);
        }
        let stale = !stale.is_empty() || !stale_rolls.is_empty();
        // Never joined (the GM has been offline since we got the link): a
        // mailed join with the claim goes first, or the GM drops our mail.
        let first = self.replica().membership().is_none() && !self.inner.mailed_join.load(std::sync::atomic::Ordering::Acquire);
        let rejoin = first || stale || self.inner.joined_by_mail.lock().expect("poisoned").elapsed() >= self.inner.cfg.remail_after;
        if rejoin {
            let join = self.replica().join_message(&self.inner.cfg.name, self.claim());
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            sent += mail::send(&mb, &self.inner.secret, &signer, host, &MailMessage::Client(join), &mut limit).await?;
            *self.inner.joined_by_mail.lock().expect("poisoned") = Instant::now();
            self.inner.mailed_join.store(true, std::sync::atomic::Ordering::Release);
        }
        let batches = self.replica().unmailed();
        for batch in batches {
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            for part in mail::split_batch(batch.clone(), limit) {
                let msg = MailMessage::Client(ClientMessage::Submit(part.clone()));
                let r = mail::send(&mb, &self.inner.secret, &signer, host, &msg, &mut limit).await;
                *self.inner.blob_limit.lock().expect("poisoned") = limit;
                sent += r?;
                self.replica().mark_mailed(&part);
                let now = Instant::now();
                self.inner.mailed_at.lock().expect("poisoned").extend(part.ops.iter().map(|o| (o.id, now)));
                self.save_logged();
            }
        }
        let rolls = self.replica().unmailed_rolls();
        if !rolls.is_empty() {
            let ids: Vec<RollId> = rolls.iter().map(|r| r.id).collect();
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            let r = mail::send(&mb, &self.inner.secret, &signer, host, &MailMessage::Client(ClientMessage::Rolls(rolls)), &mut limit).await;
            *self.inner.blob_limit.lock().expect("poisoned") = limit;
            sent += r?;
            self.replica().mark_rolls_mailed(&ids);
            let now = Instant::now();
            self.inner.rolls_mailed_at.lock().expect("poisoned").extend(ids.iter().map(|id| (*id, now)));
            self.save_logged();
        }
        let resync = self.replica().resync_requests();
        for r in resync {
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            sent += mail::send(&mb, &self.inner.secret, &signer, host, &MailMessage::Client(ClientMessage::Resync(r)), &mut limit).await?;
        }
        Ok(sent)
    }

    /// Commands marked mailed whose mailing is older than
    /// [`PlayerConfig::remail_after`] (counted from when this run first
    /// saw them, for those mailed before a restart).
    fn stale_mail(&self) -> Vec<OpId> {
        let now = Instant::now();
        let r = self.replica();
        let mut at = self.inner.mailed_at.lock().expect("poisoned");
        let pending: Vec<OpId> = r.characters().flat_map(|c| r.outbox(c).iter().filter(|p| p.mailed).map(|p| p.op.id)).collect();
        at.retain(|id, _| pending.contains(id));
        pending.into_iter().filter(|id| now.duration_since(*at.entry(*id).or_insert(now)) >= self.inner.cfg.remail_after).collect()
    }

    /// Rolls marked mailed and still unanswered after
    /// [`PlayerConfig::remail_after`] (as [`PlayerSession::stale_mail`]).
    fn stale_rolls(&self) -> Vec<RollId> {
        let now = Instant::now();
        let r = self.replica();
        let mut at = self.inner.rolls_mailed_at.lock().expect("poisoned");
        let pending: Vec<RollId> = r.roll_outbox().iter().filter(|p| p.mailed).map(|p| p.report.id).collect();
        at.retain(|id, _| pending.contains(id));
        pending.into_iter().filter(|id| now.duration_since(*at.entry(*id).or_insert(now)) >= self.inner.cfg.remail_after).collect()
    }

    /// Collects mail from the GM: answers to mailed commands and pushes.
    /// Mail not signed by the GM is dropped. Returns how many mail items
    /// were read.
    pub async fn fetch_mail(&self) -> Result<usize, NetError> {
        let mb = self.mailbox_client().await?;
        self.ensure_registered(&mb).await?;
        let host = self.inner.cfg.link.host;
        let mut n = 0;
        loop {
            let (items, more) = mail::fetch_page(&mb, &self.inner.secret).await?;
            let mut ids = Vec::new();
            for (id, opened) in items {
                ids.push(id);
                n += 1;
                let Ok(o) = opened else { continue };
                if o.sender != host {
                    continue;
                }
                let m = self.replica().inbox.accept(o.sender, &o.payload);
                if let Ok(Some(MailMessage::Server(sm))) = m {
                    // Resync requests made here go out with the next mail.
                    let _ = self.handle(sm);
                }
            }
            self.save().map_err(|e| NetError::Protocol(format!("could not save: {e}")))?;
            if !ids.is_empty() {
                mb.ack(ids).await?;
            }
            if !more {
                break;
            }
        }
        Ok(n)
    }

    /// Connects when the GM is reachable and sends the outbox; otherwise
    /// mails the outbox and collects mail.
    pub async fn sync(&self) -> SyncMode {
        let mode = self.sync_inner().await;
        *self.inner.last_mode.lock().expect("poisoned") = Some(mode);
        mode
    }

    async fn sync_inner(&self) -> SyncMode {
        if let Some(c) = self.client() {
            match self.flush_live(&c).await {
                Ok(()) => return SyncMode::Online,
                Err(e) => {
                    tracing::info!("lost the connection to the GM: {e}");
                    self.drop_client(&c);
                }
            }
        }
        match self.connect().await {
            Ok(_) => {
                // So the GM can mail us once we are offline again.
                if self.inner.cfg.mailbox.is_some() {
                    if let Ok(mb) = self.mailbox_client().await {
                        if let Err(e) = self.ensure_registered(&mb).await {
                            tracing::info!("could not register the GM's key with the mailbox: {e}");
                            self.forget_mailbox().await;
                        }
                    }
                }
                return SyncMode::Online;
            }
            Err(NetError::Denied(reason)) if reason.is_final() => {
                tracing::info!("the GM's app refused us: {reason}");
                return SyncMode::Offline;
            }
            Err(e) => tracing::info!("the GM is not reachable ({e}); using the mailbox"),
        }
        if self.inner.cfg.mailbox.is_none() {
            return SyncMode::Offline;
        }
        if self.replica().denied().is_some_and(DenyReason::is_final) {
            // Refused by mail: only collect (a refusal may be followed by
            // nothing else); do not mail more.
            return match self.fetch_mail().await {
                Ok(_) => SyncMode::Mailbox,
                Err(_) => {
                    self.forget_mailbox().await;
                    SyncMode::Offline
                }
            };
        }
        let sent = self.send_mail().await;
        let got = match &sent {
            Ok(_) => self.fetch_mail().await,
            Err(_) => Err(NetError::Connect("mailbox".into())),
        };
        match (sent, got) {
            (Ok(_), Ok(_)) => SyncMode::Mailbox,
            (s, g) => {
                if let Err(e) = s.and(g) {
                    tracing::info!("mailbox failed: {e}");
                }
                self.forget_mailbox().await;
                SyncMode::Offline
            }
        }
    }
}

/// Appends one record (length, postcard) to a journal, opened on first
/// use; `seq` is the newest sequence number it holds.
fn append_log<T: serde::Serialize>(log: &Mutex<OutboxLog>, path: &std::path::Path, record: &T, seq: u64) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = postcard::to_stdvec(record).map_err(std::io::Error::other)?;
    let mut rec = (bytes.len() as u32).to_be_bytes().to_vec();
    rec.extend(bytes);
    let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
    if log.file.is_none() {
        log.file = Some(std::fs::OpenOptions::new().create(true).append(true).open(path)?);
    }
    log.file.as_mut().expect("opened").write_all(&rec)?;
    log.max_seq = log.max_seq.max(seq);
    Ok(())
}

/// A save with records up to `seq` is on disk: empty the journal when it
/// holds nothing newer.
fn trim_log(log: &Mutex<OutboxLog>, path: &std::path::Path, seq: u64) {
    let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
    if log.max_seq > seq || (log.file.is_none() && log.max_seq == 0) {
        return;
    }
    log.file = None;
    log.max_seq = 0;
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("could not empty the journal {}: {e}", path.display());
        }
    }
}

async fn request(client: &CampaignClient, msg: &ClientMessage) -> Result<ServerMessage, NetError> {
    let bytes = client.submit(msg::encode(msg)).await?;
    msg::decode(&bytes).map_err(|e| NetError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::Authority;
    use chummer_core::career::ManualExpense;
    use chummer_net::invite::CampaignId;

    fn gain(amount: f64) -> Command {
        Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount, reason: "test".into(), ..Default::default() } }
    }

    /// An edit made just before the app is killed, while its save had not
    /// run yet (saves run in the background), used to be lost: the
    /// e2e `player-crash` scenario saw a player end one edit short. The
    /// outbox journal brings it back on the next start.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_survives_a_crash_before_its_save() {
        let engine = Arc::new(Engine::load().unwrap());
        let dir = std::env::temp_dir().join(format!("chummer-sync-outbox-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let crash = dir.join("crash");
        std::fs::create_dir_all(&crash).unwrap();
        let path = dir.join("campaign.replica");

        // A replica with one character, saved.
        let key = SecretKey::from_bytes(&[2; 32]);
        let p = key.public();
        let mut auth = Authority::new(CampaignId([1; 16]), SecretKey::from_bytes(&[1; 32]).public(), "GM");
        auth.add_member(p, Role::Player, "Alice");
        let c = CharacterId::new("c");
        let ch = chummer_core::character::Character::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5")).unwrap();
        auth.add_character(c.clone(), Some(p), ch).unwrap();
        let mut r = Replica::new();
        let ClientMessage::Join { have, .. } = r.join_message("Alice", None) else { unreachable!() };
        let (membership, pushes) = auth.join(p, "Alice", &have).unwrap();
        r.handle(&engine, ServerMessage::Joined { membership, pushes });
        r.save(&path).unwrap();

        let ep = chummer_net::node::bind(key.clone(), &chummer_net::config::NetConfig::with_relays([]), vec![]).await.unwrap();
        let link = InviteLink { host: auth.gm(), campaign: auth.campaign(), member: None, gm_key: None, relay: None };
        let mut cfg = PlayerConfig::new("Alice", link);
        cfg.path = Some(path.clone());
        let s = PlayerSession::new(ep.clone(), key.clone(), engine.clone(), cfg.clone()).unwrap();
        {
            // The background save has not run when the app is killed.
            let _hold = s.inner.saving.lock().unwrap();
            s.edit_now(&c, gain(3.0)).unwrap();
            // A roll right after, the same way.
            s.roll_now(&c, chummer_core::dice::RollRecord { label: "Soak".into(), pool: 2, dice: vec![5, 1], ..Default::default() }).unwrap();
            for f in std::fs::read_dir(&dir).unwrap().flatten().filter(|f| f.path().is_file()) {
                std::fs::copy(f.path(), crash.join(f.file_name())).unwrap();
            }
        }
        let karma = s.replica().character(&c).unwrap().karma;
        s.close();

        // The next start takes the edit back, queued to be sent.
        let mut cfg2 = cfg.clone();
        cfg2.path = Some(crash.join("campaign.replica"));
        let back = PlayerSession::new(ep.clone(), key.clone(), engine.clone(), cfg2.clone()).unwrap();
        assert_eq!(back.replica().outbox(&c).len(), 1, "the edit is back in the outbox");
        assert_eq!(back.replica().roll_outbox().len(), 1, "so is the roll");
        assert_eq!(back.replica().character(&c).unwrap().karma, karma);
        // It was saved; the journal is empty, and a further restart does
        // not add it twice.
        assert!(!outbox_log_path(&crash.join("campaign.replica")).exists());
        assert!(!roll_log_path(&crash.join("campaign.replica")).exists());
        back.close();
        let again = PlayerSession::new(ep.clone(), key, engine, cfg2).unwrap();
        assert_eq!(again.replica().outbox(&c).len(), 1);
        assert_eq!(again.replica().roll_outbox().len(), 1);
        // New commands go on from there (no sequence number reused).
        again.edit_now(&c, gain(1.0)).unwrap();
        let seqs: Vec<u64> = again.replica().outbox(&c).iter().map(|p| p.op.id.seq).collect();
        assert!(seqs[1] > seqs[0], "{seqs:?}");
        // The label: the welcome's (here a stale member name, as when a
        // join renamed the member) counts only until the membership says;
        // members added by node id have none.
        *again.inner.label.lock().unwrap() = Some("P2".into());
        // (The replica may be busy with a background save for a moment.)
        let end = Instant::now() + Duration::from_secs(5);
        while again.label().is_some() && Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(again.label(), None, "membership label is empty for a member added by node id");
        again.close();
        ep.close().await;
        // Best effort: a background save may still be finishing a file.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
