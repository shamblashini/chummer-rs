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

/// Version of the campaign protocol, sent in [`Hello`] and [`Welcome`].
/// The host refuses a client with another one ([`DenyReason::Version`]).
///
/// It follows the sync format: chummer-sync fails to compile unless this
/// equals its `SYNC_VERSION`, so an app whose sync messages changed is
/// refused here, with a "who must update" message, instead of failing on
/// every message after the handshake. Bump both together. 2: up to
/// 0.6.0 (sync format 5). 5: the app's version in [`Hello`] and
/// [`DenyReason::VersionGap`] (sync format 5).
///
/// The handshake itself (the ALPN, [`Hello`]'s first fields, the
/// challenge, [`DenyReason::Version`]) stays as it is, so every app, old or
/// new, can still read the refusal.
pub const PROTOCOL_VERSION: u32 = 5;

/// The first [`PROTOCOL_VERSION`] whose clients send their app version
/// and read [`DenyReason::VersionGap`].
pub const FIRST_WITH_APP_VERSION: u32 = 5;

/// This app's version (`CARGO_PKG_VERSION`: the crates share the
/// workspace's version).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// The client's app version ("0.7.0"). Since protocol 5; empty from
    /// older clients. Last, so an older host reads the fields before it
    /// and ignores this one.
    pub app_version: String,
}

/// [`Hello`] as clients up to protocol 2 (0.6.0) send it.
#[derive(Deserialize)]
struct LegacyHello {
    campaign_id: CampaignId,
    member_key: Option<PublicKey>,
    client_version: u32,
}

/// Reads a [`Hello`] frame body from any client, old ones too (without
/// the fields added since).
pub fn decode_hello(body: &[u8]) -> Result<Hello, crate::frame::FrameError> {
    match crate::frame::decode::<Hello>(body) {
        Ok(h) => Ok(h),
        Err(e) => match crate::frame::decode::<LegacyHello>(body) {
            Ok(h) => Ok(Hello { campaign_id: h.campaign_id, member_key: h.member_key, client_version: h.client_version, app_version: String::new() }),
            Err(_) => Err(e),
        },
    }
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
    /// Another protocol version. Sent to clients older than
    /// [`FIRST_WITH_APP_VERSION`] (they know no other refusal for it).
    #[error("{}", version_text(*client, *server, "", ""))]
    Version { client: u32, server: u32 },
    #[error("{0}")]
    Other(String),
    /// Another protocol version, with both apps' versions (empty when
    /// unknown). Sent to clients from [`FIRST_WITH_APP_VERSION`] on.
    /// New variants go after this one: older apps read the ones before.
    #[error("{}", version_text(*client, *server, client_app, server_app))]
    VersionGap { client: u32, server: u32, client_app: String, server_app: String },
}

/// The version part of a message: the other app's version, or (unknown,
/// or the same number) the protocol numbers.
fn version_detail(mine: u32, theirs: u32, my_app: &str, their_app: &str) -> String {
    if !their_app.is_empty() && their_app != my_app {
        if my_app.is_empty() {
            format!("version {their_app}")
        } else {
            format!("version {their_app}; yours is {my_app}")
        }
    } else {
        format!("campaign protocol {theirs}; yours is {mine}")
    }
}

/// What a player is told when the GM's app speaks protocol `server` and
/// the player's `client`: who has to update.
pub fn version_text(client: u32, server: u32, client_app: &str, server_app: &str) -> String {
    let detail = version_detail(client, server, client_app, server_app);
    match client.cmp(&server) {
        std::cmp::Ordering::Greater => format!("The GM's chummer-rs is older ({detail}). Ask your GM to update, or use the same version."),
        std::cmp::Ordering::Less => format!("Your chummer-rs is older than the GM's ({detail}). Update chummer-rs."),
        std::cmp::Ordering::Equal => format!("Your chummer-rs and the GM's do not match ({detail}). Use the same version."),
    }
}

/// What the GM is told when `who` came with protocol `client` (app
/// `client_app`, empty when unknown) and the GM's app speaks `server`.
/// `what` says what they tried ("tried to join", "mailed").
pub fn gm_version_text(who: &str, what: &str, client: u32, server: u32, client_app: &str, server_app: &str) -> String {
    let detail = version_detail(server, client, server_app, client_app);
    match client.cmp(&server) {
        std::cmp::Ordering::Less => format!("{who} {what} with an older chummer-rs ({detail}). They need to update chummer-rs, or use the same version as you."),
        std::cmp::Ordering::Greater => format!("{who} {what} with a newer chummer-rs ({detail}). Update chummer-rs, or ask them to use the same version as you."),
        std::cmp::Ordering::Equal => format!("{who} {what} with a chummer-rs that does not match yours ({detail}). Use the same version."),
    }
}

impl DenyReason {
    /// The refusal holds until the GM does something (a new link):
    /// trying again, or mailing, does not help.
    pub fn is_final(&self) -> bool {
        !matches!(self, DenyReason::Other(_))
    }

    /// The apps speak different versions (nothing the GM does to the
    /// invite helps; one of them has to update).
    pub fn is_version(&self) -> bool {
        matches!(self, DenyReason::Version { .. } | DenyReason::VersionGap { .. })
    }

    /// The refusal for a client with protocol `client` (app `client_app`)
    /// at this host, in a form that client can read.
    pub fn version(client: u32, client_app: &str) -> DenyReason {
        if client >= FIRST_WITH_APP_VERSION {
            DenyReason::VersionGap { client, server: PROTOCOL_VERSION, client_app: client_app.to_owned(), server_app: APP_VERSION.to_owned() }
        } else {
            DenyReason::Version { client, server: PROTOCOL_VERSION }
        }
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

    /// `peer` was refused because its app speaks another protocol version
    /// (`hello.client_version`, `hello.app_version`). `member` is the
    /// member key it proved, if any. For the host's UI ("Anna tried to
    /// join with an older chummer-rs"); called on every such attempt.
    fn wrong_version(&self, _peer: EndpointId, _hello: &Hello, _member: Option<PublicKey>) -> impl Future<Output = ()> + Send {
        async {}
    }

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
        let hello = decode_hello(&crate::frame::read_frame_body(&mut recv, MAX_FRAME).await?)?;
        let nonce: [u8; 32] = {
            use crypto_box::aead::rand_core::RngCore;
            let mut n = [0u8; 32];
            crypto_box::aead::OsRng.fill_bytes(&mut n);
            n
        };
        write_frame(&mut send, &Challenge { nonce }, MAX_FRAME).await?;
        let proof: HelloProof = read_frame(&mut recv, MAX_FRAME).await?;
        // The proof is the same in every protocol version so far.
        let member = match (hello.member_key, proof.sig) {
            (None, _) => Ok(None),
            (Some(k), Some(sig)) if k.verify(&hello_message(&hello.campaign_id, &me, &peer, &nonce), &sig).is_ok() => Ok(Some(k)),
            (Some(_), _) => Err(DenyReason::BadProof),
        };
        let verdict = if hello.client_version != PROTOCOL_VERSION {
            self.handler.wrong_version(peer, &hello, member.clone().ok().flatten()).await;
            Err(DenyReason::version(hello.client_version, &hello.app_version))
        } else {
            match member {
                Ok(member) => self.handler.hello(peer, &hello, member).await,
                Err(e) => Err(e),
            }
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
        let hello = Hello { campaign_id: campaign, member_key: member.map(SecretKey::public), client_version: PROTOCOL_VERSION, app_version: APP_VERSION.to_owned() };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`Hello`] and [`DenyReason`] as 0.6.0 (protocol 2) has them.
    #[derive(Serialize, Deserialize)]
    struct OldHello {
        campaign_id: CampaignId,
        member_key: Option<PublicKey>,
        client_version: u32,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum OldDenyReason {
        NoSuchCampaign,
        NotInvited,
        Claimed,
        Revoked,
        Expired,
        Superseded,
        BadProof,
        Version { client: u32, server: u32 },
        Other(String),
    }

    #[test]
    fn handshake_reads_across_versions() {
        let campaign = CampaignId::random();
        let key = SecretKey::generate().public();
        // A new host reads an old client's hello.
        let old = crate::frame::encode(&OldHello { campaign_id: campaign, member_key: Some(key), client_version: 2 }).unwrap();
        let h = decode_hello(&old).unwrap();
        assert_eq!((h.campaign_id, h.member_key, h.client_version, h.app_version.as_str()), (campaign, Some(key), 2, ""));
        // An old host reads a new client's hello (the app version is ignored).
        let new = crate::frame::encode(&Hello { campaign_id: campaign, member_key: None, client_version: PROTOCOL_VERSION, app_version: APP_VERSION.into() }).unwrap();
        let h: OldHello = crate::frame::decode(&new).unwrap();
        assert_eq!((h.campaign_id, h.member_key, h.client_version), (campaign, None, PROTOCOL_VERSION));
        assert_eq!(decode_hello(&new).unwrap().app_version, APP_VERSION);
        // An old client reads the refusal it is sent; a new client reads an old host's.
        let to_old = DenyReason::version(2, "");
        assert_eq!(to_old, DenyReason::Version { client: 2, server: PROTOCOL_VERSION });
        let bytes = postcard::to_stdvec(&HelloReply::Denied(to_old)).unwrap();
        #[derive(Debug, PartialEq, Deserialize)]
        enum OldReply {
            #[allow(dead_code)]
            Welcome(()),
            Denied(OldDenyReason),
        }
        assert_eq!(postcard::from_bytes::<OldReply>(&bytes).unwrap(), OldReply::Denied(OldDenyReason::Version { client: 2, server: PROTOCOL_VERSION }));
        let from_old = postcard::to_stdvec(&OldDenyReason::Version { client: PROTOCOL_VERSION, server: 2 }).unwrap();
        assert_eq!(postcard::from_bytes::<DenyReason>(&from_old).unwrap(), DenyReason::Version { client: PROTOCOL_VERSION, server: 2 });
        // New clients get both app versions.
        assert_eq!(DenyReason::version(PROTOCOL_VERSION + 1, "9.0.0"), DenyReason::VersionGap { client: PROTOCOL_VERSION + 1, server: PROTOCOL_VERSION, client_app: "9.0.0".into(), server_app: APP_VERSION.into() });
    }

    #[test]
    fn version_messages_say_who_updates() {
        // What a player sees.
        assert_eq!(DenyReason::Version { client: 5, server: 2 }.to_string(), "The GM's chummer-rs is older (campaign protocol 2; yours is 5). Ask your GM to update, or use the same version.");
        assert_eq!(DenyReason::Version { client: 2, server: 5 }.to_string(), "Your chummer-rs is older than the GM's (campaign protocol 5; yours is 2). Update chummer-rs.");
        let gap = |c, s| DenyReason::VersionGap { client: c, server: s, client_app: "0.7.0".into(), server_app: "0.8.0".into() };
        assert_eq!(gap(5, 6).to_string(), "Your chummer-rs is older than the GM's (version 0.8.0; yours is 0.7.0). Update chummer-rs.");
        assert_eq!(gap(6, 5).to_string(), "The GM's chummer-rs is older (version 0.8.0; yours is 0.7.0). Ask your GM to update, or use the same version.");
        // The same app version (two builds of it): the protocol numbers.
        let same = DenyReason::VersionGap { client: 6, server: 5, client_app: "0.7.0".into(), server_app: "0.7.0".into() };
        assert_eq!(same.to_string(), "The GM's chummer-rs is older (campaign protocol 5; yours is 6). Ask your GM to update, or use the same version.");
        assert!(same.is_final() && same.is_version() && !DenyReason::Claimed.is_version());
        // What the GM sees.
        assert_eq!(gm_version_text("Anna", "tried to join", 2, 5, "", "0.7.0"), "Anna tried to join with an older chummer-rs (campaign protocol 2; yours is 5). They need to update chummer-rs, or use the same version as you.");
        assert_eq!(gm_version_text("Anna", "tried to join", 6, 5, "0.8.0", "0.7.0"), "Anna tried to join with a newer chummer-rs (version 0.8.0; yours is 0.7.0). Update chummer-rs, or ask them to use the same version as you.");
    }
}
