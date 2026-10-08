//! One app's network node: an iroh endpoint with this machine's key that
//! player sessions dial out from and that serves the campaign protocol
//! while the user hosts a campaign.
//!
//! There is one endpoint per node key: two endpoints with the same key on
//! one relay would push each other off it. So the endpoint always accepts
//! [`CAMPAIGN_ALPN`], and a [`HostSlot`] hands connections to the hosted
//! campaign's [`AuthorityHost`] or, when nothing is hosted, hangs up (the
//! player then falls back to the mailbox).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chummer_net::campaign::{CampaignHost, CAMPAIGN_ALPN};
use chummer_net::config::{NetConfig, DEFAULT_RELAY_URL};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::dial_addr;
use chummer_net::{Endpoint, EndpointId, NetError, RelayUrl, SecretKey};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};

use crate::host::{AuthorityHost, Handler};

/// How long [`Node::start`] waits for a relay before it returns anyway
/// (the endpoint keeps trying in the background).
pub const ONLINE_WAIT: Duration = Duration::from_secs(10);

/// The campaign being served, if any.
#[derive(Debug, Clone, Default)]
pub struct HostSlot(Arc<Mutex<Option<CampaignHost<Handler>>>>);

impl HostSlot {
    fn get(&self) -> Option<CampaignHost<Handler>> {
        self.0.lock().expect("poisoned").clone()
    }

    fn set(&self, host: Option<CampaignHost<Handler>>) -> Option<CampaignHost<Handler>> {
        std::mem::replace(&mut *self.0.lock().expect("poisoned"), host)
    }
}

impl ProtocolHandler for HostSlot {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        match self.get() {
            Some(host) => host.accept(conn).await,
            None => {
                conn.close(4u32.into(), b"no campaign is hosted here right now");
                Ok(())
            }
        }
    }
}

/// The app's endpoint, router and mailbox connection.
#[derive(Debug)]
pub struct Node {
    endpoint: Endpoint,
    router: Router,
    slot: HostSlot,
    secret: SecretKey,
    cfg: NetConfig,
    mailbox: tokio::sync::Mutex<Option<MailboxClient>>,
}

impl Node {
    /// Binds the endpoint with `cfg`'s relays and waits up to
    /// [`ONLINE_WAIT`] for one of them. Must run inside a tokio runtime.
    pub async fn start(secret: SecretKey, cfg: NetConfig) -> Result<Node, NetError> {
        let node = Node::bind(secret, cfg).await?;
        if !node.cfg.relays.is_empty() && tokio::time::timeout(ONLINE_WAIT, node.endpoint.online()).await.is_err() {
            tracing::warn!("no relay reachable yet; still trying");
        }
        Ok(node)
    }

    /// Binds the endpoint without waiting for a relay (it connects in the
    /// background; dialling waits for it).
    pub async fn bind(secret: SecretKey, cfg: NetConfig) -> Result<Node, NetError> {
        let endpoint = chummer_net::node::bind(secret.clone(), &cfg, vec![CAMPAIGN_ALPN.to_vec()]).await?;
        let slot = HostSlot::default();
        let router = Router::builder(endpoint.clone()).accept(CAMPAIGN_ALPN, slot.clone()).spawn();
        Ok(Node { endpoint, router, slot, secret, cfg, mailbox: Default::default() })
    }

    /// Whether a relay connection is up.
    pub fn is_connected(&self) -> bool {
        self.home_relay().is_some()
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn secret(&self) -> &SecretKey {
        &self.secret
    }

    pub fn config(&self) -> &NetConfig {
        &self.cfg
    }

    /// The relay this node is reachable through right now.
    pub fn home_relay(&self) -> Option<RelayUrl> {
        self.endpoint.addr().relay_urls().next().cloned()
    }

    /// The relay hint for invite links: the home relay, unless it is the
    /// project's default one (which every player has).
    pub fn relay_hint(&self) -> Option<RelayUrl> {
        let default: Option<RelayUrl> = DEFAULT_RELAY_URL.parse().ok();
        self.home_relay().filter(|r| Some(r) != default.as_ref())
    }

    /// Serves `host`'s campaign (replacing what was served).
    pub fn serve(&self, host: &AuthorityHost) {
        if let Some(old) = self.slot.set(Some(host.protocol())) {
            old.close_all();
        }
    }

    /// Stops serving: connected players are hung up on and use the
    /// mailbox until the campaign is served again.
    pub fn stop_serving(&self) {
        if let Some(old) = self.slot.set(None) {
            old.close_all();
        }
    }

    pub fn serving(&self) -> bool {
        self.slot.get().is_some()
    }

    /// The mailbox node of the first relay that has one.
    pub fn mailbox_id(&self) -> Option<EndpointId> {
        self.cfg.mailbox().and_then(|r| r.mailbox)
    }

    /// A connection to the mailbox (kept and reused).
    pub async fn mailbox(&self) -> Result<MailboxClient, NetError> {
        let id = self.mailbox_id().ok_or_else(|| NetError::Connect("no relay with a mailbox is configured".into()))?;
        let mut mb = self.mailbox.lock().await;
        if let Some(c) = mb.as_ref() {
            return Ok(c.clone());
        }
        let c = tokio::time::timeout(Duration::from_secs(15), MailboxClient::connect(&self.endpoint, dial_addr(id, None)))
            .await
            .map_err(|_| NetError::Connect("the mailbox did not answer".into()))??;
        *mb = Some(c.clone());
        Ok(c)
    }

    /// One mailbox round for `host` ([`AuthorityHost::sync_mail`]); a
    /// broken mailbox connection is dropped so the next round reconnects.
    pub async fn sync_mail(&self, host: &AuthorityHost) -> Result<crate::host::MailReport, NetError> {
        let mb = self.mailbox().await?;
        let r = host.sync_mail(&mb).await;
        if matches!(r, Err(NetError::Connect(_) | NetError::Connection(_) | NetError::Frame(_))) {
            *self.mailbox.lock().await = None;
        }
        r
    }

    /// Tells the relay mailbox at once which keys may mail `host`'s GM
    /// (after invites changed: a revoked key stops working now, not at
    /// the next mailbox round).
    pub async fn register_mail_keys(&self, host: &AuthorityHost) -> Result<chummer_net::mailbox::MailboxStatus, NetError> {
        let mb = self.mailbox().await?;
        let r = host.register_mail_keys(&mb, false).await;
        if matches!(r, Err(NetError::Connect(_) | NetError::Connection(_) | NetError::Frame(_))) {
            *self.mailbox.lock().await = None;
        }
        r
    }

    /// Closes everything.
    pub async fn shutdown(self) {
        self.stop_serving();
        if let Some(m) = self.mailbox.lock().await.take() {
            m.close();
        }
        let _ = self.router.shutdown().await;
        self.endpoint.close().await;
    }
}
