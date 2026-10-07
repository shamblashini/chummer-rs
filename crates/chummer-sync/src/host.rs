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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chummer_core::command::{Command, Rejected};
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignHandler, CampaignHost, Hello, Welcome, PROTOCOL_VERSION};
use chummer_net::invite::{InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::{EndpointId, NetError, RelayUrl, SecretKey};
use tokio::sync::{broadcast, mpsc};

use crate::authority::{Authority, LocalApplied};
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

struct Shared {
    authority: Mutex<Authority>,
    engine: Arc<Engine>,
    secret: SecretKey,
    path: Option<PathBuf>,
    dirty: AtomicBool,
    blob_limit: Mutex<usize>,
    events: broadcast::Sender<HostEvent>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").field("path", &self.path).finish_non_exhaustive()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Authority> {
        self.authority.lock().expect("authority lock poisoned")
    }

    fn touch(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    fn save_if_dirty(&self) -> std::io::Result<()> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            if let Some(p) = &self.path {
                let bytes = self.lock().to_bytes();
                if let Err(e) = crate::persist::write_atomic(p, &bytes) {
                    self.dirty.store(true, Ordering::Release);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// One request from `peer` (live or mailed).
    fn handle(&self, peer: EndpointId, msg: ClientMessage) -> (ServerMessage, Vec<EndpointId>, Option<CharacterId>) {
        let mut a = self.lock();
        let (reply, notify, changed) = match msg {
            ClientMessage::Join { name, have } => match a.join(peer, &name, &have) {
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
        drop(a);
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
    async fn hello(&self, peer: EndpointId, hello: &Hello) -> Result<Welcome, String> {
        let role = {
            let mut a = self.shared.lock();
            let known = a.role(&peer).is_some();
            let role = a.admit(peer, hello.campaign_id, hello.invite_token.as_ref())?;
            if !known {
                let _ = self.shared.events.send(HostEvent::Membership);
            }
            role
        };
        self.shared.touch();
        Ok(Welcome { campaign_id: hello.campaign_id, role, server_version: PROTOCOL_VERSION })
    }

    async fn submit(&self, peer: EndpointId, _role: Role, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: ClientMessage = msg::decode(&payload).map_err(|e| e.to_string())?;
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
    /// Blobs stored for offline members.
    pub sent: usize,
}

impl AuthorityHost {
    /// Serves `authority`. `secret` is this machine's node key (it opens
    /// mail and signs what is mailed). With `path`, the authority is saved
    /// there (every couple of seconds when it changed, and by
    /// [`AuthorityHost::save`]). Must be called inside a tokio runtime.
    pub fn new(authority: Authority, engine: Arc<Engine>, secret: SecretKey, path: Option<PathBuf>) -> AuthorityHost {
        let (events, _) = broadcast::channel(256);
        let shared = Arc::new(Shared { authority: Mutex::new(authority), engine, secret, path, dirty: AtomicBool::new(false), blob_limit: Mutex::new(DEFAULT_BLOB_LIMIT), events });
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<EndpointId>>();
        let host = CampaignHost::new(Handler { shared: shared.clone(), sweep: tx.clone() });
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
    pub fn authority(&self) -> MutexGuard<'_, Authority> {
        self.shared.lock()
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

    /// An invite link for a new member.
    pub fn invite(&self, role: Role, label: &str, relay: Option<RelayUrl>) -> InviteLink {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut a = self.shared.lock();
        let token = a.invites_mut().create(role, label, now);
        let link = InviteLink { host: a.gm(), campaign: a.campaign(), invite: Some(token), relay };
        drop(a);
        self.shared.touch();
        link
    }

    /// The GM edits a character: applied at once, logged with the GM as
    /// author, pushed to its owner (or mailed by the next
    /// [`AuthorityHost::sync_mail`]).
    pub fn gm_edit(&self, id: &CharacterId, cmd: Command) -> Result<LocalApplied, Rejected> {
        let r = self.shared.lock().apply_local(&self.shared.engine, id, cmd)?;
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
        let r = self.shared.lock().revert(&self.shared.engine, id, version)?;
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
                if !member {
                    report.dropped += 1;
                    continue;
                }
                let msg = self.shared.lock().inbox.accept(o.sender, &o.payload);
                self.shared.touch();
                match msg {
                    Ok(Some(MailMessage::Client(cm))) => {
                        report.handled += 1;
                        let resync = match &cm {
                            ClientMessage::Resync(r) => Some(r.character.clone()),
                            _ => None,
                        };
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
        let behind = self.shared.lock().members_behind();
        for peer in behind.into_iter().filter(|p| !online.contains(p)) {
            let msgs = self.shared.lock().outgoing_for(&peer);
            let mut failed = None;
            for m in msgs {
                if failed.is_some() {
                    self.shared.lock().requeue(peer, m);
                    continue;
                }
                let mut limit = *self.shared.blob_limit.lock().expect("poisoned");
                let r = mail::send(mailbox, &self.shared.secret, peer, &MailMessage::Server(m.clone()), &mut limit).await;
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
        Ok(report)
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
