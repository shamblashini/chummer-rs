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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chummer_core::command::{Command, Rejected, Report};
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignClient, Hello, PROTOCOL_VERSION};
use chummer_net::invite::{InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::dial_addr;
use chummer_net::{Endpoint, EndpointId, NetError, SecretKey};
use tokio::sync::mpsc;

use crate::mail::{self, DEFAULT_BLOB_LIMIT};
use crate::msg::{self, CharacterId, ClientMessage, MailMessage, ServerMessage};
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
}

impl PlayerConfig {
    pub fn new(name: impl Into<String>, link: InviteLink) -> PlayerConfig {
        PlayerConfig { name: name.into(), link, mailbox: None, path: None, connect_timeout: Duration::from_secs(15) }
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
            }),
            events: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }

    /// The local copies, locked. Do not hold across an `.await`.
    pub fn replica(&self) -> MutexGuard<'_, Replica> {
        self.inner.replica.lock().expect("replica lock poisoned")
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

    /// Saves the replica (when the session has a path).
    pub fn save(&self) -> std::io::Result<()> {
        match &self.inner.cfg.path {
            Some(p) => {
                let bytes = self.replica().to_bytes();
                crate::persist::write_atomic(p, &bytes)
            }
            None => Ok(()),
        }
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

    fn handle(&self, msg: ServerMessage) -> Vec<crate::msg::ResyncRequest> {
        let events = self.replica().handle(&self.inner.engine, msg);
        let resync = events.iter().filter_map(|e| if let Event::NeedResync(r) = e { Some(r.clone()) } else { None }).collect();
        self.emit(events);
        resync
    }

    // ----- live connection -----

    /// Dials the GM's app and joins. On success the outbox is sent.
    pub async fn connect(&self) -> Result<Role, NetError> {
        let link = &self.inner.cfg.link;
        let hello = Hello { campaign_id: link.campaign, invite_token: link.invite, client_version: PROTOCOL_VERSION };
        let join = CampaignClient::join(&self.inner.endpoint, dial_addr(link.host, link.relay.as_ref()), hello);
        let (client, mut pushes) = tokio::time::timeout(self.inner.cfg.connect_timeout, join).await.map_err(|_| NetError::Connect("timed out".into()))??;
        let client = Arc::new(client);
        let role = client.welcome().role;
        *self.inner.role.lock().expect("poisoned") = Some(role);
        let join_msg = self.replica().join_message(&self.inner.cfg.name);
        let reply = request(&client, &join_msg).await?;
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
    }

    /// Mails the commands not mailed yet (and resync requests) to the GM.
    /// Returns the number of blobs stored.
    pub async fn send_mail(&self) -> Result<usize, NetError> {
        let mb = self.mailbox_client().await?;
        let host = self.inner.cfg.link.host;
        let mut sent = 0;
        let batches = self.replica().unmailed();
        for batch in batches {
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            for part in mail::split_batch(batch.clone(), limit) {
                let msg = MailMessage::Client(ClientMessage::Submit(part.clone()));
                let r = mail::send(&mb, &self.inner.secret, host, &msg, &mut limit).await;
                *self.inner.blob_limit.lock().expect("poisoned") = limit;
                sent += r?;
                self.replica().mark_mailed(&part);
                self.save_logged();
            }
        }
        let resync = self.replica().resync_requests();
        for r in resync {
            let mut limit = *self.inner.blob_limit.lock().expect("poisoned");
            sent += mail::send(&mb, &self.inner.secret, host, &MailMessage::Client(ClientMessage::Resync(r)), &mut limit).await?;
        }
        Ok(sent)
    }

    /// Collects mail from the GM: answers to mailed commands and pushes.
    /// Mail not signed by the GM is dropped. Returns how many mail items
    /// were read.
    pub async fn fetch_mail(&self) -> Result<usize, NetError> {
        let mb = self.mailbox_client().await?;
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
            Ok(_) => return SyncMode::Online,
            Err(e) => tracing::info!("the GM is not reachable ({e}); using the mailbox"),
        }
        if self.inner.cfg.mailbox.is_none() {
            return SyncMode::Offline;
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
