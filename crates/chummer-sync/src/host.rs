//! The authority on chummer-net: [`AuthorityHost`] serves the campaign
//! protocol, pushes changes to connected members and uses the relay
//! mailbox for the others.
//!
//! Usable headless: the GUI's "Host campaign" and the `chummer-authority`
//! binary both build one, register [`AuthorityHost::protocol`] with an
//! iroh `Router` under [`chummer_net::campaign::CAMPAIGN_ALPN`], and call
//! [`AuthorityHost::sync_mail`] on start and every few minutes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chummer_core::command::{Command, Rejected};
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignHandler, CampaignHost, DenyReason, Hello, Welcome, PROTOCOL_VERSION};
use chummer_net::invite::{derive_campaign_key, InviteId, InviteLink, Role};
use chummer_net::mailbox::{MailboxClient, MailboxStatus};
use chummer_net::{EndpointId, NetError, PublicKey, RelayUrl, SecretKey};
use tokio::sync::{broadcast, mpsc};

use crate::authority::{Authority, LocalApplied, SnapshotJob};
use crate::invites::{Invite, InviteOp};
use crate::journal::Journal;
use crate::lockwatch::{self, Guard};
use crate::mail::{self, DEFAULT_BLOB_LIMIT};
use crate::msg::{self, CharacterId, ClientMessage, MailMessage, ServerMessage};

/// How long one push to a connected member may take before the member is
/// taken to be stuck.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// How often unsaved changes are written to disk.
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// What changed at the host, for a UI to refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// A character changed (any author).
    Changed(CharacterId),
    /// Someone joined, or members/characters changed.
    Membership,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The public campaign keys of generation `gen` (and the one before, during
/// a rotation), current first.
fn gm_keys(secret: &SecretKey, campaign: chummer_net::invite::CampaignId, gen: u32) -> Vec<PublicKey> {
    let mut keys = vec![derive_campaign_key(secret, campaign, gen).public()];
    if gen > 0 {
        keys.push(derive_campaign_key(secret, campaign, gen - 1).public());
    }
    keys
}

struct Shared {
    authority: Mutex<Authority>,
    engine: Arc<Engine>,
    secret: SecretKey,
    /// The keys last registered with the relay mailbox (to skip the
    /// round trip when nothing changed) and what the relay said then.
    registered: Mutex<Option<Vec<PublicKey>>>,
    mailbox_status: Mutex<Option<MailboxStatus>>,
    path: Option<PathBuf>,
    dirty: AtomicBool,
    blob_limit: Mutex<usize>,
    events: broadcast::Sender<HostEvent>,
    /// With `path`: the changes made since the last save
    /// ([`crate::journal`]). Locked after the authority, never before.
    journal: Mutex<Option<Journal>>,
    /// Held across a whole save, so saves happen one after the other (the
    /// journal must not be set aside by a newer save that an older one
    /// then overwrites).
    saving: Mutex<()>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").field("path", &self.path).finish_non_exhaustive()
    }
}

impl Shared {
    fn lock(&self) -> Guard<'_, Authority> {
        lockwatch::lock(&self.authority, "authority")
    }

    /// Writes changes taken with [`Authority::take_applied`] to the journal
    /// (synced), before anyone is told about them. Called after the
    /// authority lock is released, so the GUI is not kept waiting for the
    /// disk; replay sorts by version.
    fn journal(&self, applied: Vec<(CharacterId, crate::msg::Entry)>) {
        if applied.is_empty() {
            return;
        }
        if let Some(j) = self.journal.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            if let Err(e) = j.append(&applied) {
                tracing::warn!("could not write the campaign journal (a crash now would lose the last changes): {e}");
            }
        }
    }

    fn touch(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    fn seen(&self, peer: &EndpointId) {
        self.lock().seen(peer, now_secs());
    }

    /// Makes snapshots without holding the lock (they take seconds for a
    /// big character; the GUI reads the authority every frame).
    fn warm(&self, jobs: Vec<SnapshotJob>) {
        if jobs.is_empty() {
            return;
        }
        let made: Vec<_> = jobs.into_iter().map(SnapshotJob::make).collect();
        self.lock().put_snapshots(made);
    }

    /// The snapshots answering `msg` from `peer` needs, made outside the
    /// lock.
    fn warm_for(&self, peer: &EndpointId, msg: &ClientMessage) {
        let jobs = match msg {
            ClientMessage::Join { have, .. } => self.lock().push_snapshot_work(peer, None, Some(have), false),
            ClientMessage::Resync(r) => self.lock().push_snapshot_work(peer, Some(&r.character), None, true),
            ClientMessage::Submit(_) => Vec::new(),
        };
        self.warm(jobs);
    }

    /// The snapshots of what `peer` is behind on, made outside the lock.
    fn warm_peer(&self, peer: &EndpointId) {
        let jobs = self.lock().push_snapshot_work(peer, None, None, false);
        self.warm(jobs);
    }

    fn save_if_dirty(&self) -> std::io::Result<()> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            if let Err(e) = self.save_now() {
                self.dirty.store(true, Ordering::Release);
                return Err(e);
            }
        }
        Ok(())
    }

    fn save_now(&self) -> std::io::Result<()> {
        let Some(p) = &self.path else { return Ok(()) };
        let _one_at_a_time = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        // Compress outside the lock; whatever changes meanwhile is made
        // under it by `to_bytes`.
        let jobs = self.lock().save_snapshot_work();
        self.warm(jobs);
        let bytes = {
            let mut a = self.lock();
            // Changes since the last journal write are in `bytes`; they
            // go into the journal first in case this save fails.
            self.journal(a.take_applied());
            let bytes = a.to_bytes();
            // The journal so far is in `bytes`; set it aside until they
            // are on disk.
            if let Some(j) = self.journal.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                j.rotate()?;
            }
            bytes
        };
        crate::persist::write_atomic(p, &bytes)?;
        if let Some(j) = self.journal.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            j.saved()?;
        }
        Ok(())
    }

    /// One request from `peer` (live or mailed).
    fn handle(&self, peer: EndpointId, msg: ClientMessage) -> (ServerMessage, Vec<EndpointId>, Option<CharacterId>) {
        let mut a = self.lock();
        let (reply, notify, changed) = match msg {
            ClientMessage::Join { name, have, .. } => match a.join(peer, &name, &have) {
                Ok((membership, pushes)) => (ServerMessage::Joined { membership, pushes }, Vec::new(), None),
                Err(e) => (ServerMessage::Error(e), Vec::new(), None),
            },
            ClientMessage::Submit(batch) => match a.submit(&self.engine, peer, batch) {
                Ok(s) => {
                    let changed = s.ack.accepted.iter().any(|x| x.changed).then(|| s.ack.character.clone());
                    (ServerMessage::Ack(s.ack), s.notify, changed)
                }
                Err(e) => (ServerMessage::Error(e), Vec::new(), None),
            },
            ClientMessage::Resync(req) => match a.resync(peer, &req) {
                Ok(p) => (ServerMessage::Push(p), Vec::new(), None),
                Err(e) => (ServerMessage::Error(e), Vec::new(), None),
            },
        };
        let applied = a.take_applied();
        drop(a);
        self.journal(applied);
        self.touch();
        (reply, notify, changed)
    }
}

/// The [`CampaignHandler`] of an [`AuthorityHost`].
#[derive(Debug, Clone)]
pub struct Handler {
    shared: Arc<Shared>,
    sweep: mpsc::UnboundedSender<Vec<EndpointId>>,
}

impl CampaignHandler for Handler {
    async fn hello(&self, peer: EndpointId, hello: &Hello, member: Option<PublicKey>) -> Result<Welcome, DenyReason> {
        let admitted = self.shared.lock().admit(peer, hello.campaign_id, member, now_secs());
        let admitted = match admitted {
            Ok(a) => a,
            Err(e) => {
                tracing::info!("refused {}: {e}", peer.fmt_short());
                return Err(e);
            }
        };
        if admitted.claimed {
            tracing::info!("{} claimed the invite \"{}\"", peer.fmt_short(), admitted.label);
            let _ = self.shared.events.send(HostEvent::Membership);
        }
        self.shared.touch();
        Ok(Welcome { campaign_id: hello.campaign_id, role: admitted.role, label: admitted.label, server_version: PROTOCOL_VERSION })
    }

    async fn submit(&self, peer: EndpointId, _role: Role, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: ClientMessage = msg::decode(&payload).map_err(|e| e.to_string())?;
        self.shared.warm_for(&peer, &msg);
        let joined = matches!(msg, ClientMessage::Join { .. });
        let (reply, notify, changed) = self.shared.handle(peer, msg);
        if let Some(id) = changed {
            let _ = self.shared.events.send(HostEvent::Changed(id));
        }
        if joined {
            let _ = self.shared.events.send(HostEvent::Membership);
        }
        if !notify.is_empty() {
            let _ = self.sweep.send(notify);
        }
        Ok(msg::encode(&reply))
    }
}

/// The campaign authority served over chummer-net. Cheap to clone.
#[derive(Debug, Clone)]
pub struct AuthorityHost {
    shared: Arc<Shared>,
    host: CampaignHost<Handler>,
    sweep: mpsc::UnboundedSender<Vec<EndpointId>>,
}

/// What a mailbox round did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailReport {
    /// Mail items read (and deleted from the relay).
    pub fetched: usize,
    /// Requests handled from them.
    pub handled: usize,
    /// Mail dropped: not openable, or not from a member.
    pub dropped: usize,
    /// Mailed joins refused (claimed, revoked or expired invites).
    pub refused: usize,
    /// Blobs stored for offline members.
    pub sent: usize,
    /// The GM's mailbox as the relay sees it (after registering the
    /// members' keys): waiting mail per key, puts refused today.
    pub status: Option<MailboxStatus>,
}

impl AuthorityHost {
    /// Serves `authority`. `secret` is this machine's node key (it opens
    /// mail and signs what is mailed). With `path`, the authority is saved
    /// there (every couple of seconds when it changed, and by
    /// [`AuthorityHost::save`]). Must be called inside a tokio runtime.
    pub fn new(mut authority: Authority, engine: Arc<Engine>, secret: SecretKey, path: Option<PathBuf>) -> AuthorityHost {
        let (events, _) = broadcast::channel(256);
        let mut replayed = 0;
        if let Some(p) = &path {
            // Changes acknowledged after the last save (a crash).
            for (id, entry) in Journal::read(p) {
                match authority.replay(&engine, &id, &entry) {
                    Ok(true) => replayed += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!("campaign journal: {e}"),
                }
            }
            if replayed > 0 {
                tracing::info!("took back {replayed} change(s) made after the last save from the journal");
            }
            // They are journaled already.
            authority.take_applied();
        }
        authority.set_gm_keys(gm_keys(&secret, authority.campaign(), authority.key_generation()));
        let me_id = authority.gm();
        let journal = Mutex::new(path.as_deref().map(Journal::new));
        let shared = Arc::new(Shared {
            authority: Mutex::new(authority),
            engine,
            secret,
            registered: Mutex::new(None),
            mailbox_status: Mutex::new(None),
            path,
            dirty: AtomicBool::new(replayed > 0),
            blob_limit: Mutex::new(DEFAULT_BLOB_LIMIT),
            events,
            journal,
            saving: Mutex::new(()),
        });
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<EndpointId>>();
        let host = CampaignHost::new(Handler { shared: shared.clone(), sweep: tx.clone() }, me_id);
        let me = AuthorityHost { shared: shared.clone(), host: host.clone(), sweep: tx };
        // Pushes to connected members, in order.
        let pusher = Pusher { shared: shared.clone() };
        tokio::spawn(async move {
            while let Some(mut peers) = rx.recv().await {
                while let Ok(more) = rx.try_recv() {
                    peers.extend(more);
                }
                peers.sort();
                peers.dedup();
                pusher.push_live(&host, &peers).await;
            }
        });
        // Saves.
        let saver = Arc::downgrade(&shared);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SAVE_EVERY).await;
                let Some(s) = saver.upgrade() else { break };
                if let Err(e) = s.save_if_dirty() {
                    tracing::warn!("could not save the campaign: {e}");
                }
            }
        });
        me
    }

    /// The protocol handler to register with an iroh `Router` under
    /// [`chummer_net::campaign::CAMPAIGN_ALPN`].
    pub fn protocol(&self) -> CampaignHost<Handler> {
        self.host.clone()
    }

    /// Members connected right now.
    pub fn connected(&self) -> Vec<(EndpointId, Role)> {
        self.host.connected()
    }

    /// Changes, for a UI.
    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.shared.events.subscribe()
    }

    /// The authority, locked. Do not hold it across an `.await`. After
    /// changing members or characters through it, call
    /// [`AuthorityHost::changed`].
    pub fn authority(&self) -> Guard<'_, Authority> {
        self.shared.lock()
    }

    /// The authority, when no other thread holds it (a UI thread polls
    /// with this so it never waits for a network task).
    pub fn try_authority(&self) -> Option<Guard<'_, Authority>> {
        lockwatch::try_lock(&self.shared.authority, "authority")
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.shared.engine
    }

    /// After a change made through [`AuthorityHost::authority`] (a character
    /// added or given away, a member removed): saves later and tells the
    /// connected members.
    pub fn changed(&self) {
        self.shared.touch();
        let _ = self.shared.events.send(HostEvent::Membership);
        let peers = self.host.connected().into_iter().map(|(p, _)| p).collect();
        let _ = self.sweep.send(peers);
    }

    /// The link of invite `id` (its current key), with `relay` as hint.
    pub fn invite_link(&self, id: &InviteId, relay: Option<RelayUrl>) -> Option<InviteLink> {
        let a = self.shared.lock();
        let i = a.invite(id)?;
        Some(InviteLink { host: a.gm(), campaign: a.campaign(), member: Some(i.secret.clone()), gm_key: a.gm_keys().first().copied(), relay })
    }

    /// A new invite for one player ("Anna"), optionally giving them
    /// `assign` when they claim it, and expiring unclaimed at `expires`
    /// (Unix seconds). Returns the invite and its link.
    pub fn create_invite(&self, role: Role, label: &str, assign: Option<crate::msg::CharacterId>, expires: Option<u64>, relay: Option<RelayUrl>) -> (Invite, InviteLink) {
        let invite = self.shared.lock().create_invite(role, label, assign, expires, now_secs()).clone();
        self.after_invites(None);
        let link = self.invite_link(&invite.id, relay).expect("just made");
        (invite, link)
    }

    /// Revokes invite `id`: the member's live connection is closed, its
    /// key no longer joins, and (with the next registration, which a
    /// mailbox round does first) no longer puts mail into the GM's
    /// mailbox.
    pub fn revoke_invite(&self, id: &InviteId) -> Result<(), String> {
        let node = self.shared.lock().revoke_invite(id, now_secs())?;
        self.after_invites(node);
        Ok(())
    }

    /// A new link for the member of invite `id` (a new device); the old
    /// link and the device that used it stop working.
    pub fn reissue_invite(&self, id: &InviteId, expires: Option<u64>, relay: Option<RelayUrl>) -> Result<InviteLink, String> {
        let node = self.shared.lock().reissue_invite(id, expires, now_secs())?;
        self.after_invites(node);
        Ok(self.invite_link(id, relay).expect("exists"))
    }

    /// Deletes invite `id` from the list (and takes its member out).
    pub fn remove_invite(&self, id: &InviteId) {
        let node = self.shared.lock().remove_invite(id);
        self.after_invites(node);
    }

    /// Takes a member out of the campaign (one added by node id, or any).
    pub fn remove_member(&self, peer: &EndpointId) -> bool {
        let gone = self.shared.lock().remove_member(peer);
        if gone {
            self.after_invites(Some(*peer));
        }
        gone
    }

    /// Applies changes from the invites file. Returns how many changed
    /// something.
    pub fn apply_invite_ops(&self, ops: Vec<InviteOp>) -> usize {
        let mut n = 0;
        for op in ops {
            let (changed, node) = {
                let mut a = self.shared.lock();
                let r = a.apply_invite_op(op, now_secs());
                let (campaign, gen) = (a.campaign(), a.key_generation());
                a.set_gm_keys(gm_keys(&self.shared.secret, campaign, gen));
                r
            };
            if changed {
                n += 1;
                self.after_invites(node);
            }
        }
        n
    }

    /// A new GM campaign key generation: members are told with the next
    /// membership (live or by mail) and their apps register it.
    pub fn rotate_campaign_key(&self) -> u32 {
        let gen = {
            let mut a = self.shared.lock();
            let gen = a.rotate_campaign_key();
            let campaign = a.campaign();
            a.set_gm_keys(gm_keys(&self.shared.secret, campaign, gen));
            gen
        };
        self.changed();
        gen
    }

    fn after_invites(&self, hang_up: Option<EndpointId>) {
        if let Some(p) = hang_up {
            self.host.disconnect(&p);
        }
        self.changed();
    }

    /// The relay's view of the GM's mailbox at the last mailbox round.
    pub fn mailbox_status(&self) -> Option<MailboxStatus> {
        self.shared.mailbox_status.lock().expect("poisoned").clone()
    }

    /// Registers the keys that may mail the GM ([`Authority::mail_keys`])
    /// with the relay mailbox, when they changed since the last time (or
    /// `force`). Revoked and re-issued keys stop working at once.
    pub async fn register_mail_keys(&self, mailbox: &MailboxClient, force: bool) -> Result<MailboxStatus, NetError> {
        let (campaign, keys) = {
            let a = self.shared.lock();
            (a.campaign(), a.mail_keys(now_secs()))
        };
        if !force {
            let same = self.shared.registered.lock().expect("poisoned").as_ref() == Some(&keys);
            if let (true, Some(st)) = (same, self.mailbox_status()) {
                return Ok(st);
            }
        }
        let st = mailbox.register(campaign.0, keys.clone()).await?;
        *self.shared.registered.lock().expect("poisoned") = Some(keys);
        *self.shared.mailbox_status.lock().expect("poisoned") = Some(st.clone());
        Ok(st)
    }

    /// The GM edits a character: applied at once, logged with the GM as
    /// author, pushed to its owner (or mailed by the next
    /// [`AuthorityHost::sync_mail`]).
    pub fn gm_edit(&self, id: &CharacterId, cmd: Command) -> Result<LocalApplied, Rejected> {
        let r = {
            let mut a = self.shared.lock();
            let r = a.apply_local(&self.shared.engine, id, cmd);
            let applied = a.take_applied();
            drop(a);
            self.shared.journal(applied);
            r?
        };
        self.shared.touch();
        if r.accepted.changed {
            let _ = self.shared.events.send(HostEvent::Changed(id.clone()));
        }
        if !r.notify.is_empty() {
            let _ = self.sweep.send(r.notify.clone());
        }
        Ok(r)
    }

    /// The GM reverts the change that made `version` of `id`
    /// ([`Authority::revert`]); pushed or mailed like any GM edit.
    pub fn gm_revert(&self, id: &CharacterId, version: u64) -> Result<crate::authority::Reverted, String> {
        let r = {
            let mut a = self.shared.lock();
            let r = a.revert(&self.shared.engine, id, version);
            let applied = a.take_applied();
            drop(a);
            self.shared.journal(applied);
            r?
        };
        self.shared.touch();
        let _ = self.shared.events.send(HostEvent::Changed(id.clone()));
        if !r.applied.notify.is_empty() {
            let _ = self.sweep.send(r.applied.notify.clone());
        }
        Ok(r)
    }

    /// Writes the authority to its file now (when it has one).
    pub fn save(&self) -> std::io::Result<()> {
        self.shared.touch();
        self.shared.save_if_dirty()
    }

    /// One mailbox round: reads all mail (requests from offline members,
    /// checked to be signed by a member), handles it, saves, deletes it on
    /// the relay, then mails everything offline members have not been sent.
    pub async fn sync_mail(&self, mailbox: &MailboxClient) -> Result<MailReport, NetError> {
        let mut report = MailReport::default();
        let mut changed = Vec::new();
        // Who may mail us: always sent (it is idempotent), so a relay that
        // lost its database learns it again.
        self.register_mail_keys(mailbox, true).await?;
        let mut denials: Vec<(EndpointId, DenyReason)> = Vec::new();
        loop {
            let (items, more) = mail::fetch_page(mailbox, &self.shared.secret).await?;
            let mut ids = Vec::new();
            for (id, opened) in items {
                ids.push(id);
                report.fetched += 1;
                let Ok(o) = opened else {
                    report.dropped += 1;
                    continue;
                };
                let member = self.shared.lock().role(&o.sender).is_some();
                let msg = if member {
                    self.shared.seen(&o.sender);
                    self.shared.lock().inbox.accept(o.sender, &o.payload)
                } else {
                    // Not a member (yet): only a join that proves an
                    // invite's key, complete in one blob.
                    match mail::single(&o.payload) {
                        Some(MailMessage::Client(ClientMessage::Join { name, have, claim: Some(proof) })) => {
                            let admitted = self.shared.lock().admit_by_mail(o.sender, Some(&proof), now_secs());
                            match admitted {
                                Ok(a) => {
                                    tracing::info!("{} claimed the invite \"{}\" by mail", o.sender.fmt_short(), a.label);
                                    let _ = self.shared.events.send(HostEvent::Membership);
                                    Ok(Some(MailMessage::Client(ClientMessage::Join { name, have, claim: Some(proof) })))
                                }
                                Err(e) => {
                                    tracing::info!("refused a mailed join from {}: {e}", o.sender.fmt_short());
                                    report.refused += 1;
                                    denials.push((o.sender, e));
                                    continue;
                                }
                            }
                        }
                        _ => {
                            report.dropped += 1;
                            continue;
                        }
                    }
                };
                self.shared.touch();
                match msg {
                    Ok(Some(MailMessage::Client(cm))) => {
                        report.handled += 1;
                        let resync = match &cm {
                            ClientMessage::Resync(r) => Some(r.character.clone()),
                            _ => None,
                        };
                        self.shared.warm_for(&o.sender, &cm);
                        let (reply, notify, ch) = self.shared.handle(o.sender, cm);
                        let mut a = self.shared.lock();
                        match (resync, reply) {
                            // The snapshot goes out with the other pushes.
                            (Some(c), ServerMessage::Push(_)) => a.forget_delivered(&o.sender, &c),
                            (_, reply) => a.queue_mail(o.sender, reply),
                        }
                        drop(a);
                        changed.extend(notify);
                        if let Some(c) = ch {
                            let _ = self.shared.events.send(HostEvent::Changed(c));
                        }
                    }
                    Ok(Some(MailMessage::Server(_))) | Err(_) => report.dropped += 1,
                    Ok(None) => {}
                }
            }
            if let Err(e) = self.shared.save_if_dirty() {
                tracing::warn!("could not save the campaign: {e}");
            }
            if !ids.is_empty() {
                mailbox.ack(ids).await?;
            }
            if !more {
                break;
            }
        }
        // Connected members get theirs live; the rest by mail.
        let online: Vec<EndpointId> = self.host.connected().into_iter().map(|(p, _)| p).collect();
        changed.extend(online.iter().copied());
        if !changed.is_empty() {
            let _ = self.sweep.send(changed);
        }
        // Refused joins are told why (they registered the GM's key from
        // the link), once per round.
        denials.sort_by_key(|(p, _)| *p);
        denials.dedup_by_key(|(p, _)| *p);
        for (peer, reason) in denials {
            let signer = self.signer_for(&peer);
            let mut limit = *self.shared.blob_limit.lock().expect("poisoned");
            match mail::send(mailbox, &self.shared.secret, &signer, peer, &MailMessage::Server(ServerMessage::Denied(reason)), &mut limit).await {
                Ok(n) => report.sent += n,
                Err(e) => tracing::info!("could not tell {} why it was refused: {e}", peer.fmt_short()),
            }
        }
        let behind = self.shared.lock().members_behind();
        for peer in behind.into_iter().filter(|p| !online.contains(p)) {
            self.shared.warm_peer(&peer);
            let msgs = self.shared.lock().outgoing_for(&peer);
            let mut failed = None;
            for m in msgs {
                if failed.is_some() {
                    self.shared.lock().requeue(peer, m);
                    continue;
                }
                let mut limit = *self.shared.blob_limit.lock().expect("poisoned");
                let signer = self.signer_for(&peer);
                let r = mail::send(mailbox, &self.shared.secret, &signer, peer, &MailMessage::Server(m.clone()), &mut limit).await;
                *self.shared.blob_limit.lock().expect("poisoned") = limit;
                match r {
                    Ok(n) => {
                        report.sent += n;
                        self.shared.lock().mark_sent(peer, &m);
                    }
                    Err(e) => {
                        tracing::info!("could not mail {}: {e}", peer.fmt_short());
                        self.shared.lock().requeue(peer, m);
                        failed = Some(e);
                    }
                }
            }
            self.shared.touch();
            // A full mailbox for one member must not stop the others.
            if let Some(e) = failed {
                if !matches!(e, NetError::Mailbox(_)) {
                    let _ = self.shared.save_if_dirty();
                    return Err(e);
                }
            }
        }
        if let Err(e) = self.shared.save_if_dirty() {
            tracing::warn!("could not save the campaign: {e}");
        }
        report.status = self.mailbox_status();
        Ok(report)
    }

    /// The campaign key that signs mail to `peer`.
    fn signer_for(&self, peer: &EndpointId) -> SecretKey {
        let (campaign, gen) = {
            let a = self.shared.lock();
            (a.campaign(), a.mail_key_generation(peer))
        };
        derive_campaign_key(&self.shared.secret, campaign, gen)
    }
}

/// The task that pushes to connected members (it runs as long as the
/// runtime does).
struct Pusher {
    shared: Arc<Shared>,
}

impl Pusher {
    async fn push_live(&self, host: &CampaignHost<Handler>, peers: &[EndpointId]) {
        let shared = &self.shared;
        let online: Vec<EndpointId> = host.connected().into_iter().map(|(p, _)| p).collect();
        for peer in peers.iter().filter(|p| online.contains(p)) {
            shared.warm_peer(peer);
            let msgs = shared.lock().outgoing_for(peer);
            let mut stuck = false;
            for m in msgs {
                if stuck {
                    shared.lock().requeue(*peer, m);
                    continue;
                }
                match tokio::time::timeout(PUSH_TIMEOUT, host.push(*peer, msg::encode(&m))).await {
                    Ok(Ok(())) => shared.lock().mark_sent(*peer, &m),
                    Ok(Err(e)) => {
                        tracing::debug!("push to {} failed: {e}", peer.fmt_short());
                        shared.lock().requeue(*peer, m);
                    }
                    Err(_) => {
                        // The member's app stopped taking pushes (hung, or
                        // far behind): hang up, so it rejoins and catches
                        // up, and the others are not kept waiting.
                        tracing::info!("{} is not taking pushes; hanging up", peer.fmt_short());
                        shared.lock().requeue(*peer, m);
                        host.disconnect(peer);
                        stuck = true;
                    }
                }
            }
            shared.touch();
        }
    }
}
