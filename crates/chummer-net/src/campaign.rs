//! The campaign protocol, ALPN `chummer-rs/campaign/1`.
//!
//! Between a player's app (client) and the GM's app (host, the campaign
//! authority). Payloads are opaque bytes; the sync layer defines them.
//!
//! On one QUIC connection:
//! 1. The client opens the first bi-directional stream and sends [`Hello`].
//!    The host answers [`HelloReply::Welcome`] or [`HelloReply::Denied`]
//!    and closes the stream. On `Denied` the host closes the connection.
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
use iroh::{Endpoint, EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::frame::{read_frame, write_frame, MAX_FRAME};
use crate::invite::{CampaignId, InviteToken, Role};
use crate::NetError;

/// ALPN of the campaign protocol.
pub const CAMPAIGN_ALPN: &[u8] = b"chummer-rs/campaign/1";

/// Version of this crate's campaign protocol messages, sent in [`Hello`]
/// and [`Welcome`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest frame on campaign streams (snapshots may be big).
pub const MAX_CAMPAIGN_FRAME: usize = 16 * MAX_FRAME;

/// First message from a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub campaign_id: CampaignId,
    /// Needed the first time; members are known by their node id afterwards.
    pub invite_token: Option<InviteToken>,
    pub client_version: u32,
}

/// The host let the client in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub campaign_id: CampaignId,
    pub role: Role,
    pub server_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelloReply {
    Welcome(Welcome),
    Denied { reason: String },
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
    /// Admit or refuse `peer`. Returning `Err(reason)` sends `Denied`.
    fn hello(
        &self,
        peer: EndpointId,
        hello: &Hello,
    ) -> impl Future<Output = Result<Welcome, String>> + Send;

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
    sessions: Arc<Mutex<HashMap<EndpointId, Session>>>,
}

impl<H> Clone for CampaignHost<H> {
    fn clone(&self) -> Self {
        CampaignHost {
            handler: self.handler.clone(),
            sessions: self.sessions.clone(),
        }
    }
}

impl<H: CampaignHandler> CampaignHost<H> {
    pub fn new(handler: H) -> Self {
        CampaignHost {
            handler: Arc::new(handler),
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
        let (mut send, mut recv) = conn.accept_bi().await.map_err(NetError::connection)?;
        let hello: Hello = read_frame(&mut recv, MAX_FRAME).await?;
        let role = match self.handler.hello(peer, &hello).await {
            Ok(welcome) => {
                let role = welcome.role;
                write_frame(&mut send, &HelloReply::Welcome(welcome), MAX_FRAME).await?;
                send.finish().map_err(NetError::connection)?;
                role
            }
            Err(reason) => {
                write_frame(&mut send, &HelloReply::Denied { reason }, MAX_FRAME).await?;
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
    /// Connects to `host`, says hello, and returns the client and the
    /// stream of pushes from the host. The receiver ends when the
    /// connection does.
    pub async fn join(
        endpoint: &Endpoint,
        host: impl Into<EndpointAddr>,
        hello: Hello,
    ) -> Result<(CampaignClient, mpsc::Receiver<Vec<u8>>), NetError> {
        let conn = endpoint
            .connect(host, CAMPAIGN_ALPN)
            .await
            .map_err(|e| NetError::Connect(e.to_string()))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(NetError::connection)?;
        write_frame(&mut send, &hello, MAX_FRAME).await?;
        send.finish().map_err(NetError::connection)?;
        let reply: HelloReply = read_frame(&mut recv, MAX_FRAME).await?;
        let welcome = match reply {
            HelloReply::Welcome(w) => w,
            HelloReply::Denied { reason } => {
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
