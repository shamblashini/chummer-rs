//! Serves the mailbox protocol (`chummer-rs/mailbox/2`) from the [`Store`].
//!
//! A put's signature ([`chummer_net::mailbox::PutAuth`]) is checked here
//! against the recipient, the uploading node (proven by the handshake)
//! and the blob; the store then checks the key against the recipient's
//! registrations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chummer_net::frame::{read_frame, write_frame};
use chummer_net::mailbox::{MailboxError, MailboxRequest, MailboxResponse, MAX_MAILBOX_FRAME};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::EndpointId;

use crate::store::Store;

/// Source of the current time (Unix seconds).
pub trait Clock: Send + Sync + std::fmt::Debug + 'static {
    fn now(&self) -> u64;
}

/// The system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to (for tests).
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(now: u64) -> Self {
        ManualClock(AtomicU64::new(now))
    }

    pub fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }

    pub fn advance(&self, secs: u64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The mailbox protocol handler.
#[derive(Debug, Clone)]
pub struct MailboxService {
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
}

impl MailboxService {
    pub fn new(store: Arc<Store>, clock: Arc<dyn Clock>) -> Self {
        MailboxService { store, clock }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Handles one request from `peer` (the id proven by the handshake).
    pub fn handle(&self, peer: EndpointId, req: MailboxRequest) -> MailboxResponse {
        let now = self.clock.now();
        let result = match req {
            MailboxRequest::Put { recipient, blob, auth } => match auth {
                None => {
                    self.store.note_refused(recipient, now);
                    Err(MailboxError::Unsigned)
                }
                Some(a) if !a.verify(&recipient, &peer, &blob) => {
                    self.store.note_refused(recipient, now);
                    Err(MailboxError::BadSignature)
                }
                Some(a) => self
                    .store
                    .put(peer, recipient, a.key, a.nonce, blob, now)
                    .map(|id| MailboxResponse::Stored { id }),
            },
            MailboxRequest::Register { scope, keys } => self
                .store
                .register(peer, scope, &keys, now)
                .map(MailboxResponse::Registered),
            MailboxRequest::Fetch { limit } => self
                .store
                .fetch(peer, limit, now)
                .map(|(items, more)| MailboxResponse::Mail { items, more }),
            MailboxRequest::Ack { ids } => {
                if ids.len() > 10_000 {
                    Err(MailboxError::BadRequest("too many ids".into()))
                } else {
                    self.store
                        .ack(peer, &ids)
                        .map(|removed| MailboxResponse::Acked { removed })
                }
            }
        };
        result.unwrap_or_else(MailboxResponse::Error)
    }
}

impl ProtocolHandler for MailboxService {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = conn.remote_id();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let this = self.clone();
            tokio::spawn(async move {
                let req: MailboxRequest = match read_frame(&mut recv, MAX_MAILBOX_FRAME).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!("bad mailbox request from {}: {e}", peer.fmt_short());
                        return;
                    }
                };
                let resp = match tokio::task::spawn_blocking(move || this.handle(peer, req)).await {
                    Ok(r) => r,
                    Err(e) => MailboxResponse::Error(MailboxError::Internal(e.to_string())),
                };
                if write_frame(&mut send, &resp, MAX_MAILBOX_FRAME)
                    .await
                    .is_ok()
                {
                    let _ = send.finish();
                }
            });
        }
        Ok(())
    }
}
