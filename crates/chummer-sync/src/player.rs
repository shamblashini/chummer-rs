//! A player's connection to a campaign: [`PlayerSession`] keeps a
//! [`Replica`], talks to the GM's [`crate::host::AuthorityHost`] over
//! chummer-net when it is reachable, and falls back to the relay mailbox
//! when it is not (play-by-post).
//!
//! Usage: [`PlayerSession::new`], then [`PlayerSession::sync`] on start and
//! every so often (it connects when it can, sends the outbox, and
//! otherwise mails it and collects mail). Edits go through
//! [`PlayerSession::edit`]; [`PlayerSession::events`] says what changed.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chummer_core::command::{Command, Rejected, Report};
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignClient, DenyReason};
use chummer_net::invite::{InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::dial_addr;
use chummer_net::{Endpoint, EndpointId, NetError, PublicKey, SecretKey};
use tokio::sync::mpsc;

use crate::mail::{self, DEFAULT_BLOB_LIMIT};
use crate::msg::{self, CharacterId, ClaimProof, ClientMessage, MailMessage, OpId, ServerMessage};
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
    joined_by_mail: Mutex<Instant>,
    /// A join was mailed in this run (a first join by mail is sent once,
    /// then again after `remail_after`).
    mailed_join: std::sync::atomic::AtomicBool,
    /// The GM keys last registered with our relay mailbox.
    registered: tokio::sync::Mutex<Option<Vec<PublicKey>>>,
    /// The label from the last welcome.
    label: Mutex<Option<String>>,
    /// The runtime the session was made in: saves asked for by a UI
    /// thread run on its blocking threads.
    rt: Option<tokio::runtime::Handle>,
    /// A background save is asked for and has not started yet.
    save_queued: std::sync::atomic::AtomicBool,
    /// One save at a time (they write the same file), so an older state
    /// never overwrites a newer one.
    saving: Mutex<()>,
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
            Some(p) if p.exists() => Replica::load(&engine, p)?,
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
                joined_by_mail: Mutex::new(Instant::now()),
                mailed_join: std::sync::atomic::AtomicBool::new(false),
                registered: tokio::sync::Mutex::new(None),
                label: Mutex::new(None),
                rt: tokio::runtime::Handle::try_current().ok(),
                save_queued: std::sync::atomic::AtomicBool::new(false),
                saving: Mutex::new(()),
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
        let report = self.replica().edit(&self.inner.engine, id, cmd)?;
        self.save_soon();
        if report.changed {
            self.inner.flush.notify_one();
        }
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
        let from_membership = self.try_replica().and_then(|r| r.membership().map(|m| m.label.clone())).filter(|l| !l.is_empty());
        from_membership.or_else(|| self.inner.label.lock().expect("poisoned").clone().filter(|l| !l.is_empty()))
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
    /// (scope: the campaign), when that changed since the last time.
    async fn ensure_registered(&self, mb: &MailboxClient) -> Result<(), NetError> {
        let keys = self.gm_keys();
        if keys.is_empty() {
            return Ok(());
        }
        let mut reg = self.inner.registered.lock().await;
        if reg.as_ref() != Some(&keys) {
            mb.register(self.inner.cfg.link.campaign.0, keys.clone()).await?;
            *reg = Some(keys);
        }
        Ok(())
    }

    /// Stops the GM's keys from putting mail into our mailbox (leaving the
    /// campaign). Best effort.
    pub async fn unregister(&self) -> Result<(), NetError> {
        let mb = self.mailbox_client().await?;
        mb.register(self.inner.cfg.link.campaign.0, Vec::new()).await?;
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
                let bytes = self.replica().to_bytes();
                crate::persist::write_atomic(p, &bytes)
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
        let report = self.replica().edit(&self.inner.engine, id, cmd)?;
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
    pub async fn send_mail(&self) -> Result<usize, NetError> {
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
        // Never joined (the GM has been offline since we got the link): a
        // mailed join with the claim goes first, or the GM drops our mail.
        let first = self.replica().membership().is_none() && !self.inner.mailed_join.load(std::sync::atomic::Ordering::Acquire);
        let rejoin = first || !stale.is_empty() || self.inner.joined_by_mail.lock().expect("poisoned").elapsed() >= self.inner.cfg.remail_after;
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

async fn request(client: &CampaignClient, msg: &ClientMessage) -> Result<ServerMessage, NetError> {
    let bytes = client.submit(msg::encode(msg)).await?;
    msg::decode(&bytes).map_err(|e| NetError::Protocol(e.to_string()))
}
