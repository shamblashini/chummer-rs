//! The campaign protocol, ALPN `chummer-rs/campaign/2`.
//!
//! Between a player's app (client) and the GM's app (host, the campaign
//! authority). Payloads are opaque bytes; the sync layer defines them.
//!
//! On one QUIC connection:
//! 1. The client opens the first bi-directional stream and sends [`Hello`]
//!    (with the public half of its member key, if it has one). The host
//!    answers a [`Challenge`] (a random nonce); the client answers a
//!    [`HelloProof`]: its member key's signature over the campaign, both
//!    node ids and the nonce ([`hello_message`]). The host then answers
//!    [`HelloReply::Welcome`] or [`HelloReply::Denied`] and closes the
//!    stream. On `Denied` the host closes the connection. A member key
//!    whose proof does not check out is refused here, before the handler
//!    sees it; the handler gets the proven key.
//! 2. Each request after that is its own bi-directional stream opened by
//!    the client: one [`Request`] frame, one [`Response`] frame. Streams are
//!    cheap in QUIC and this needs no request ids.
//! 3. The host pushes to the client on uni-directional streams it opens:
//!    one [`Push`] frame each.
//!
//! The peer's [`EndpointId`] is proven by the QUIC/TLS handshake, so the
//! host always knows who sent a request.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::frame::{read_frame, write_frame, MAX_FRAME};
use crate::invite::{CampaignId, Role};
use crate::NetError;

/// ALPN of the campaign protocol.
pub const CAMPAIGN_ALPN: &[u8] = b"chummer-rs/campaign/2";

/// Version of this crate's campaign protocol messages, sent in [`Hello`]
/// and [`Welcome`].
pub const PROTOCOL_VERSION: u32 = 2;

/// Largest frame on campaign streams (snapshots may be big).
pub const MAX_CAMPAIGN_FRAME: usize = 16 * MAX_FRAME;

const HELLO_DOMAIN: &[u8] = b"chummer-rs/campaign-hello/2\0";

/// First message from a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub campaign_id: CampaignId,
    /// The public half of the member key from the invite link. Members the
    /// GM added by node id have none.
    pub member_key: Option<PublicKey>,
    pub client_version: u32,
}

/// The host's answer to [`Hello`]: sign this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub nonce: [u8; 32],
}

/// The client's answer to the [`Challenge`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloProof {
    /// The member key's signature over [`hello_message`]; `None` without
    /// a member key.
    pub sig: Option<Signature>,
}

/// What the member key signs in the handshake: the campaign, the host,
/// the client's node id (proven by QUIC, so the proof cannot be used by
/// another node) and the host's fresh nonce (so it cannot be replayed).
pub fn hello_message(campaign: &CampaignId, host: &EndpointId, client: &EndpointId, nonce: &[u8; 32]) -> Vec<u8> {
    let mut m = Vec::with_capacity(HELLO_DOMAIN.len() + 16 + 64 + 32);
    m.extend_from_slice(HELLO_DOMAIN);
    m.extend_from_slice(&campaign.0);
    m.extend_from_slice(host.as_bytes());
    m.extend_from_slice(client.as_bytes());
    m.extend_from_slice(nonce);
    m
}

/// The host let the client in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub campaign_id: CampaignId,
    pub role: Role,
    /// The name the GM gave this member's invite ("Anna").
    pub label: String,
    pub server_version: u32,
}

/// Why the host refused a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum DenyReason {
    #[error("the GM's app does not host this campaign")]
    NoSuchCampaign,
    #[error("you are not invited to this campaign; ask the GM for an invite link")]
    NotInvited,
    #[error("this invite link was already used on another device; ask the GM to issue you a new link")]
    Claimed,
    #[error("the GM revoked this invite")]
    Revoked,
    #[error("this invite link has expired; ask the GM for a new one")]
    Expired,
    #[error("the GM replaced this invite link with a newer one; use the new link")]
    Superseded,
    #[error("the invite key could not be proven")]
    BadProof,
    #[error("the GM's app speaks campaign protocol {server}, this app {client}; update chummer-rs")]
    Version { client: u32, server: u32 },
    #[error("{0}")]
    Other(String),
}

impl DenyReason {
    /// The refusal holds until the GM does something (a new link):
    /// trying again, or mailing, does not help.
    pub fn is_final(&self) -> bool {
        !matches!(self, DenyReason::Other(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelloReply {
    Welcome(Welcome),
    Denied(DenyReason),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Submit(Vec<u8>),
    Ping(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Ack(Vec<u8>),
    Pong(u64),
    /// The host could not handle the request.
    Error(String),
}

/// Host-to-client message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Push(pub Vec<u8>);

/// What the host application decides. Implemented by the sync layer.
pub trait CampaignHandler: Send + Sync + std::fmt::Debug + 'static {
    /// Admit or refuse `peer`. `member` is the member key the peer proved
    /// (`hello.member_key`, checked). Returning `Err(reason)` sends
    /// `Denied`.
    fn hello(
        &self,
        peer: EndpointId,
        hello: &Hello,
        member: Option<PublicKey>,
    ) -> impl Future<Output = Result<Welcome, DenyReason>> + Send;

    /// Handle a submission from an admitted peer. `Err` becomes
    /// [`Response::Error`]; rejections that the client should show belong in
    /// the `Ok` payload.
    fn submit(
        &self,
        peer: EndpointId,
        role: Role,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;

    /// `peer`'s connection ended.
    fn disconnected(&self, _peer: EndpointId) -> impl Future<Output = ()> + Send {
        async {}
    }
}

#[derive(Debug, Clone)]
struct Session {
    conn: Connection,
    role: Role,
}

/// Serves the campaign protocol. Register with an iroh `Router`:
/// `Router::builder(ep).accept(CAMPAIGN_ALPN, host.clone()).spawn()`.
#[derive(Debug)]
pub struct CampaignHost<H> {
    handler: Arc<H>,
    /// This host's node id (what clients sign in the handshake).
    me: EndpointId,
    sessions: Arc<Mutex<HashMap<EndpointId, Session>>>,
}

impl<H> Clone for CampaignHost<H> {
    fn clone(&self) -> Self {
        CampaignHost {
            handler: self.handler.clone(),
            me: self.me,
            sessions: self.sessions.clone(),
        }
    }
}

impl<H: CampaignHandler> CampaignHost<H> {
    /// A host for `handler` on the endpoint with id `me`.
    pub fn new(handler: H, me: EndpointId) -> Self {
        CampaignHost {
            handler: Arc::new(handler),
            me,
            sessions: Arc::default(),
        }
    }

    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Peers that are connected and admitted, with their roles.
    pub fn connected(&self) -> Vec<(EndpointId, Role)> {
        let s = self.sessions.lock().expect("poisoned");
        s.iter().map(|(id, s)| (*id, s.role)).collect()
    }

    /// Hangs up on every connected peer (the host stops serving; they fall
    /// back to the mailbox).
    pub fn close_all(&self) {
        let sessions: Vec<Session> = self.sessions.lock().expect("poisoned").drain().map(|(_, s)| s).collect();
        for s in sessions {
            s.conn.close(3u32.into(), b"the host stopped serving this campaign");
        }
    }

    /// Hangs up on `peer` (it is not taking pushes); it can connect again.
    pub fn disconnect(&self, peer: &EndpointId) {
        let session = self.sessions.lock().expect("poisoned").remove(peer);
        if let Some(s) = session {
            s.conn.close(5u32.into(), b"not taking pushes");
        }
    }

    /// Sends `payload` to `peer`. Fails if the peer is not connected; the
    /// caller then falls back to the mailbox.
    pub async fn push(&self, peer: EndpointId, payload: Vec<u8>) -> Result<(), NetError> {
        let conn = {
            let s = self.sessions.lock().expect("poisoned");
            s.get(&peer).map(|s| s.conn.clone())
        }
        .ok_or_else(|| NetError::Connection(format!("{} is not connected", peer.fmt_short())))?;
        let mut send = conn.open_uni().await.map_err(NetError::connection)?;
        write_frame(&mut send, &Push(payload), MAX_CAMPAIGN_FRAME).await?;
        send.finish().map_err(NetError::connection)?;
        Ok(())
    }

    async fn serve(&self, conn: Connection) -> Result<(), NetError> {
        let peer = conn.remote_id();
        let me = self.me;
        let (mut send, mut recv) = conn.accept_bi().await.map_err(NetError::connection)?;
        let hello: Hello = read_frame(&mut recv, MAX_FRAME).await?;
        let nonce: [u8; 32] = {
            use crypto_box::aead::rand_core::RngCore;
            let mut n = [0u8; 32];
            crypto_box::aead::OsRng.fill_bytes(&mut n);
            n
        };
        write_frame(&mut send, &Challenge { nonce }, MAX_FRAME).await?;
        let proof: HelloProof = read_frame(&mut recv, MAX_FRAME).await?;
        let verdict = if hello.client_version != PROTOCOL_VERSION {
            Err(DenyReason::Version { client: hello.client_version, server: PROTOCOL_VERSION })
        } else {
            match (hello.member_key, proof.sig) {
                (None, _) => Ok(None),
                (Some(k), Some(sig)) if k.verify(&hello_message(&hello.campaign_id, &me, &peer, &nonce), &sig).is_ok() => Ok(Some(k)),
                (Some(_), _) => Err(DenyReason::BadProof),
            }
        };
        let verdict = match verdict {
            Ok(member) => self.handler.hello(peer, &hello, member).await,
            Err(e) => Err(e),
        };
        let role = match verdict {
            Ok(welcome) => {
                let role = welcome.role;
                write_frame(&mut send, &HelloReply::Welcome(welcome), MAX_FRAME).await?;
                send.finish().map_err(NetError::connection)?;
                role
            }
            Err(reason) => {
                write_frame(&mut send, &HelloReply::Denied(reason), MAX_FRAME).await?;
                send.finish().map_err(NetError::connection)?;
                // Give the reply time to arrive, then hang up.
                let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
                conn.close(1u32.into(), b"denied");
                return Ok(());
            }
        };
        let old = self.sessions.lock().expect("poisoned").insert(
            peer,
            Session {
                conn: conn.clone(),
                role,
            },
        );
        if let Some(old) = old {
            old.conn.close(2u32.into(), b"replaced by a new connection");
        }

        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(s) => s,
                Err(_) => break,
            };
            let handler = self.handler.clone();
            tokio::spawn(async move {
                let req: Request = match read_frame(&mut recv, MAX_CAMPAIGN_FRAME).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!("bad campaign request from {}: {e}", peer.fmt_short());
                        return;
                    }
                };
                let resp = match req {
                    Request::Ping(n) => Response::Pong(n),
                    Request::Submit(bytes) => match handler.submit(peer, role, bytes).await {
                        Ok(ack) => Response::Ack(ack),
                        Err(e) => Response::Error(e),
                    },
                };
                if write_frame(&mut send, &resp, MAX_CAMPAIGN_FRAME)
                    .await
                    .is_ok()
                {
                    let _ = send.finish();
                }
            });
        }

        let removed = {
            let mut s = self.sessions.lock().expect("poisoned");
            if s.get(&peer)
                .is_some_and(|s| s.conn.stable_id() == conn.stable_id())
            {
                s.remove(&peer);
                true
            } else {
                false
            }
        };
        if removed {
            self.handler.disconnected(peer).await;
        }
        Ok(())
    }
}

impl<H: CampaignHandler> ProtocolHandler for CampaignHost<H> {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.serve(conn).await.map_err(AcceptError::from_err)
    }
}

/// A player's connection to a campaign host.
#[derive(Debug)]
pub struct CampaignClient {
    conn: Connection,
    welcome: Welcome,
}

impl CampaignClient {
    /// Connects to `host`, says hello to `campaign` (proving `member`, the
    /// invite's member key, when given), and returns the client and the
    /// stream of pushes from the host. The receiver ends when the
    /// connection does.
    pub async fn join(
        endpoint: &Endpoint,
        host: impl Into<EndpointAddr>,
        campaign: CampaignId,
        member: Option<&SecretKey>,
    ) -> Result<(CampaignClient, mpsc::Receiver<Vec<u8>>), NetError> {
        let conn = endpoint
            .connect(host, CAMPAIGN_ALPN)
            .await
            .map_err(|e| NetError::Connect(e.to_string()))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(NetError::connection)?;
        let hello = Hello { campaign_id: campaign, member_key: member.map(SecretKey::public), client_version: PROTOCOL_VERSION };
        write_frame(&mut send, &hello, MAX_FRAME).await?;
        let Challenge { nonce } = read_frame(&mut recv, MAX_FRAME).await?;
        let sig = member.map(|m| m.sign(&hello_message(&campaign, &conn.remote_id(), &endpoint.id(), &nonce)));
        write_frame(&mut send, &HelloProof { sig }, MAX_FRAME).await?;
        send.finish().map_err(NetError::connection)?;
        let reply: HelloReply = read_frame(&mut recv, MAX_FRAME).await?;
        let welcome = match reply {
            HelloReply::Welcome(w) => w,
            HelloReply::Denied(reason) => {
                conn.close(0u32.into(), b"");
                return Err(NetError::Denied(reason));
            }
        };
        let (tx, rx) = mpsc::channel(256);
        let push_conn = conn.clone();
        tokio::spawn(async move {
            while let Ok(mut recv) = push_conn.accept_uni().await {
                match read_frame::<_, Push>(&mut recv, MAX_CAMPAIGN_FRAME).await {
                    Ok(Push(bytes)) => {
                        if tx.send(bytes).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::debug!("bad push frame: {e}"),
                }
            }
        });
        Ok((CampaignClient { conn, welcome }, rx))
    }

    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    pub fn host_id(&self) -> EndpointId {
        self.conn.remote_id()
    }

    async fn request(&self, req: &Request) -> Result<Response, NetError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(NetError::connection)?;
        write_frame(&mut send, req, MAX_CAMPAIGN_FRAME).await?;
        send.finish().map_err(NetError::connection)?;
        Ok(read_frame(&mut recv, MAX_CAMPAIGN_FRAME).await?)
    }

    /// Sends `payload` and waits for the host's acknowledgement payload.
    pub async fn submit(&self, payload: Vec<u8>) -> Result<Vec<u8>, NetError> {
        match self.request(&Request::Submit(payload)).await? {
            Response::Ack(b) => Ok(b),
            Response::Error(e) => Err(NetError::Remote(e)),
            other => Err(NetError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Round-trip time to the host.
    pub async fn ping(&self) -> Result<Duration, NetError> {
        use crypto_box::aead::rand_core::RngCore;
        let n = crypto_box::aead::OsRng.next_u64();
        let start = Instant::now();
        match self.request(&Request::Ping(n)).await? {
            Response::Pong(m) if m == n => Ok(start.elapsed()),
            other => Err(NetError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Hangs up.
    pub fn close(&self) {
        self.conn.close(0u32.into(), b"bye");
    }

    /// Waits until the connection has ended.
    pub async fn closed(&self) {
        self.conn.closed().await;
    }
}
