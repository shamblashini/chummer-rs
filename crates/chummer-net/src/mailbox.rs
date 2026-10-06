//! The relay mailbox protocol, ALPN `chummer-rs/mailbox/1`, and its client.
//!
//! The mailbox holds sealed blobs ([`crate::seal`]) for peers that are
//! offline. Each request is one bi-directional stream: one
//! [`MailboxRequest`], one [`MailboxResponse`].
//!
//! Fetch and ack carry no recipient: the relay uses the node id proven by
//! the QUIC handshake, so a connection can only ever read and delete its own
//! mail. The relay also counts sender quotas by that id.

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use crate::frame::{read_frame, write_frame, MAX_FRAME};
use crate::seal::{self, Opened, SealError};
use crate::NetError;

/// ALPN of the mailbox protocol.
pub const MAILBOX_ALPN: &[u8] = b"chummer-rs/mailbox/1";

/// Largest frame on mailbox streams.
pub const MAX_MAILBOX_FRAME: usize = 4 * MAX_FRAME;

/// Most bytes of blobs the relay puts in one fetch reply; more is fetched
/// with another request.
pub const MAX_FETCH_BYTES: usize = 2 * MAX_FRAME;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxRequest {
    /// Store `blob` for `recipient`.
    Put {
        recipient: EndpointId,
        blob: Vec<u8>,
    },
    /// Return up to `limit` of my oldest messages.
    Fetch { limit: u32 },
    /// Delete these messages of mine (after processing them).
    Ack { ids: Vec<u64> },
}

/// A stored message as delivered to its recipient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailItem {
    pub id: u64,
    /// Who uploaded it, as seen by the relay. The proven author is the
    /// signer inside the sealed blob.
    pub sender: EndpointId,
    /// Unix seconds when the relay received it.
    pub received: u64,
    pub blob: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxResponse {
    Stored { id: u64 },
    Mail { items: Vec<MailItem>, more: bool },
    Acked { removed: u32 },
    Error(MailboxError),
}

/// Why the relay refused a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum MailboxError {
    #[error("the message is too large ({len} bytes, the relay allows {max})")]
    TooLarge { len: u64, max: u64 },
    #[error("the recipient's mailbox is full ({max} messages)")]
    RecipientFull { max: u64 },
    #[error("daily sending limit reached ({what})")]
    SenderQuota { what: String },
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("relay error: {0}")]
    Internal(String),
}

/// A connection to a relay's mailbox.
#[derive(Debug, Clone)]
pub struct MailboxClient {
    conn: Connection,
}

impl MailboxClient {
    /// Connects to the mailbox node `mailbox` (see
    /// [`crate::config::RelayEntry::mailbox`]).
    pub async fn connect(
        endpoint: &Endpoint,
        mailbox: impl Into<EndpointAddr>,
    ) -> Result<Self, NetError> {
        let conn = endpoint
            .connect(mailbox, MAILBOX_ALPN)
            .await
            .map_err(|e| NetError::Connect(e.to_string()))?;
        Ok(MailboxClient { conn })
    }

    async fn request(&self, req: &MailboxRequest) -> Result<MailboxResponse, NetError> {
        let (mut send, mut recv) = self.conn.open_bi().await.map_err(NetError::connection)?;
        write_frame(&mut send, req, MAX_MAILBOX_FRAME).await?;
        send.finish().map_err(NetError::connection)?;
        match read_frame(&mut recv, MAX_MAILBOX_FRAME).await? {
            MailboxResponse::Error(e) => Err(e.into()),
            r => Ok(r),
        }
    }

    /// Stores an (already sealed) blob for `recipient`. Returns its id.
    pub async fn put(&self, recipient: EndpointId, blob: Vec<u8>) -> Result<u64, NetError> {
        match self
            .request(&MailboxRequest::Put { recipient, blob })
            .await?
        {
            MailboxResponse::Stored { id } => Ok(id),
            other => Err(unexpected(other)),
        }
    }

    /// Seals `payload` from `sender` to `recipient` and stores it.
    pub async fn put_sealed(
        &self,
        sender: &SecretKey,
        recipient: EndpointId,
        payload: &[u8],
    ) -> Result<u64, NetError> {
        let blob = seal::seal(sender, &recipient, payload)?;
        self.put(recipient, blob).await
    }

    /// Up to `limit` of my oldest messages, and whether there are more.
    pub async fn fetch(&self, limit: u32) -> Result<(Vec<MailItem>, bool), NetError> {
        match self.request(&MailboxRequest::Fetch { limit }).await? {
            MailboxResponse::Mail { items, more } => Ok((items, more)),
            other => Err(unexpected(other)),
        }
    }

    /// Fetches my messages and opens them with `me`. Messages that fail to
    /// open are returned with the error, so the caller can ack (drop) them.
    pub async fn fetch_opened(
        &self,
        me: &SecretKey,
        limit: u32,
    ) -> Result<(Vec<(MailItem, Result<Opened, SealError>)>, bool), NetError> {
        let (items, more) = self.fetch(limit).await?;
        let opened = items
            .into_iter()
            .map(|item| {
                let o = seal::open(me, &item.blob);
                (item, o)
            })
            .collect();
        Ok((opened, more))
    }

    /// Deletes my messages `ids`. Returns how many existed.
    pub async fn ack(&self, ids: Vec<u64>) -> Result<u32, NetError> {
        match self.request(&MailboxRequest::Ack { ids }).await? {
            MailboxResponse::Acked { removed } => Ok(removed),
            other => Err(unexpected(other)),
        }
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"bye");
    }
}

fn unexpected(r: MailboxResponse) -> NetError {
    NetError::Protocol(format!("unexpected mailbox reply {r:?}"))
}
