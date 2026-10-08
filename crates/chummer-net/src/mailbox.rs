//! The relay mailbox protocol, ALPN `chummer-rs/mailbox/3`, and its client.
//!
//! The mailbox holds sealed blobs ([`crate::seal`]) for peers that are
//! offline. Each request is one bi-directional stream: one
//! [`MailboxRequest`], one [`MailboxResponse`].
//!
//! Fetch, ack and register carry no owner: the relay uses the node id
//! proven by the QUIC handshake, so a connection can only ever read,
//! delete and configure its own mailbox. The relay also counts sender
//! quotas by that id.
//!
//! # Who may put mail
//!
//! Access is by capability, without accounts: each mailbox owner
//! registers which public keys may put mail into its mailbox
//! ([`MailboxRequest::Register`], one replace-the-set list per scope; the
//! sync layer uses one scope per campaign). Every put carries a
//! [`PutAuth`]: a signature by one of those keys over the recipient, the
//! uploading node, a random nonce and the blob's BLAKE3 hash. The relay
//! refuses unsigned puts, bad signatures, keys the recipient did not
//! register, and every put to a mailbox with no registrations. Binding the
//! recipient and the uploader stops a put from being replayed into
//! another mailbox or by another node; the relay also refuses a nonce
//! that is still waiting in the recipient's mailbox.
//!
//! A key can be registered for one uploading node ([`Registration::bound`]):
//! puts signed with it from any other node are refused
//! ([`MailboxError::WrongDevice`]). The sync layer binds a claimed
//! invite's key to the device that claimed it, so a leaked link is useless
//! for mail, and the GM's campaign keys to the GM's node. A key without a
//! node (an invite not claimed yet, which a play-by-post player claims by
//! mail) may have only a few messages waiting
//! (`max_messages_per_unbound_key` on the relay).

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

use crate::frame::{read_frame, write_frame, MAX_FRAME};
use crate::seal::{self, Opened, SealError};
use crate::NetError;

/// ALPN of the mailbox protocol.
pub const MAILBOX_ALPN: &[u8] = b"chummer-rs/mailbox/3";

/// Largest frame on mailbox streams.
pub const MAX_MAILBOX_FRAME: usize = 4 * MAX_FRAME;

/// Most bytes of blobs the relay puts in one fetch reply; more is fetched
/// with another request.
pub const MAX_FETCH_BYTES: usize = 2 * MAX_FRAME;

/// Most keys in one registration scope.
pub const MAX_KEYS_PER_SCOPE: usize = 256;

/// Most registration scopes per mailbox owner.
pub const MAX_SCOPES: usize = 64;

const PUT_DOMAIN: &[u8] = b"chummer-rs/mailbox-put/2\0";

/// A registration scope: which list of allowed keys a [`MailboxRequest::Register`]
/// replaces (the sync layer uses the campaign id).
pub type Scope = [u8; 16];

/// A key that may put mail into a mailbox, and from which uploading node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Registration {
    pub key: PublicKey,
    /// Only puts uploaded by this node are taken; `None`: any node (with
    /// the relay's small cap for unbound keys).
    pub uploader: Option<EndpointId>,
}

impl Registration {
    /// `key`, from any node.
    pub fn any(key: PublicKey) -> Registration {
        Registration { key, uploader: None }
    }

    /// `key`, only when `uploader` uploads the put.
    pub fn bound(key: PublicKey, uploader: EndpointId) -> Registration {
        Registration { key, uploader: Some(uploader) }
    }
}

impl From<PublicKey> for Registration {
    fn from(key: PublicKey) -> Registration {
        Registration::any(key)
    }
}

impl From<&PublicKey> for Registration {
    fn from(key: &PublicKey) -> Registration {
        Registration::any(*key)
    }
}

impl From<&Registration> for Registration {
    fn from(r: &Registration) -> Registration {
        *r
    }
}

/// The proof that a put is allowed: signed by a key the recipient
/// registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutAuth {
    pub key: PublicKey,
    pub nonce: [u8; 16],
    pub sig: Signature,
}

impl PutAuth {
    fn message(recipient: &EndpointId, uploader: &EndpointId, nonce: &[u8; 16], blob: &[u8]) -> Vec<u8> {
        let mut m = Vec::with_capacity(PUT_DOMAIN.len() + 32 + 32 + 16 + 32);
        m.extend_from_slice(PUT_DOMAIN);
        m.extend_from_slice(recipient.as_bytes());
        m.extend_from_slice(uploader.as_bytes());
        m.extend_from_slice(nonce);
        m.extend_from_slice(blake3::hash(blob).as_bytes());
        m
    }

    /// Signs a put of `blob` to `recipient`, uploaded by `uploader` (the
    /// node whose connection sends it), with `signer`.
    pub fn sign(signer: &SecretKey, recipient: &EndpointId, uploader: &EndpointId, blob: &[u8]) -> PutAuth {
        let nonce = crate::invite::random_id();
        let sig = signer.sign(&PutAuth::message(recipient, uploader, &nonce, blob));
        PutAuth { key: signer.public(), nonce, sig }
    }

    /// Whether the signature is good for this put.
    pub fn verify(&self, recipient: &EndpointId, uploader: &EndpointId, blob: &[u8]) -> bool {
        self.key.verify(&PutAuth::message(recipient, uploader, &self.nonce, blob), &self.sig).is_ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxRequest {
    /// Store `blob` for `recipient`, signed by a key it registered.
    Put {
        recipient: EndpointId,
        blob: Vec<u8>,
        /// Required; `None` is refused (kept optional so the refusal is a
        /// clear answer, not a decoding error).
        auth: Option<PutAuth>,
    },
    /// Return up to `limit` of my oldest messages.
    Fetch { limit: u32 },
    /// Delete these messages of mine (after processing them).
    Ack { ids: Vec<u64> },
    /// The keys that may put mail into my mailbox, for `scope`, each
    /// maybe bound to one uploading node: replaces what that scope had
    /// (idempotent; an empty list removes the scope). A key in several
    /// scopes is unbound if any of them leaves it unbound. Answered with
    /// [`MailboxResponse::Registered`].
    Register { scope: Scope, keys: Vec<Registration> },
}

/// A stored message as delivered to its recipient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailItem {
    pub id: u64,
    /// Who uploaded it, as seen by the relay. The proven author is the
    /// signer inside the sealed blob.
    pub sender: EndpointId,
    /// The registered key the put was signed with.
    pub key: PublicKey,
    /// Unix seconds when the relay received it.
    pub received: u64,
    pub blob: Vec<u8>,
}

/// The state of my mailbox, as the relay sees it (answer to a register).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStatus {
    /// Keys registered in all my scopes.
    pub keys: u32,
    /// Messages waiting for me.
    pub waiting: u64,
    /// Waiting messages per signing key.
    pub by_key: Vec<(PublicKey, u64)>,
    /// Puts to me the relay refused today (UTC): unsigned, bad
    /// signatures, unregistered keys, caps.
    pub refused_today: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxResponse {
    Stored { id: u64 },
    Mail { items: Vec<MailItem>, more: bool },
    Acked { removed: u32 },
    Registered(MailboxStatus),
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
    #[error("too many messages from this key are waiting ({max})")]
    KeyFull { max: u64 },
    #[error("the put is not signed")]
    Unsigned,
    #[error("the put's signature is not valid")]
    BadSignature,
    #[error("the recipient does not take mail signed by this key")]
    NotAllowed,
    #[error("this key may only put mail from another device (the invite was claimed there)")]
    WrongDevice,
    #[error("this put was already stored (replayed)")]
    Replayed,
    #[error("too many keys or scopes ({what})")]
    TooManyKeys { what: String },
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("relay error: {0}")]
    Internal(String),
}

impl MailboxError {
    /// The put was refused for its key (not for space or quotas): sending
    /// it again does not help until the recipient registers the key.
    pub fn is_refusal(&self) -> bool {
        matches!(self, MailboxError::Unsigned | MailboxError::BadSignature | MailboxError::NotAllowed | MailboxError::WrongDevice | MailboxError::Replayed)
    }
}

/// A connection to a relay's mailbox.
#[derive(Debug, Clone)]
pub struct MailboxClient {
    conn: Connection,
    me: EndpointId,
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
        Ok(MailboxClient { conn, me: endpoint.id() })
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

    /// Stores an (already sealed) blob for `recipient`, signed with
    /// `signer` (a key `recipient` registered). Returns its id.
    pub async fn put(&self, recipient: EndpointId, blob: Vec<u8>, signer: &SecretKey) -> Result<u64, NetError> {
        let auth = PutAuth::sign(signer, &recipient, &self.me, &blob);
        self.put_with(recipient, blob, Some(auth)).await
    }

    /// Stores `blob` with the given authorisation, as it is (for tests of
    /// what the relay refuses).
    pub async fn put_with(&self, recipient: EndpointId, blob: Vec<u8>, auth: Option<PutAuth>) -> Result<u64, NetError> {
        match self.request(&MailboxRequest::Put { recipient, blob, auth }).await? {
            MailboxResponse::Stored { id } => Ok(id),
            other => Err(unexpected(other)),
        }
    }

    /// Seals `payload` from `sender` (this node's key) to `recipient` and
    /// stores it, signed with `signer`.
    pub async fn put_sealed(&self, sender: &SecretKey, signer: &SecretKey, recipient: EndpointId, payload: &[u8]) -> Result<u64, NetError> {
        let blob = seal::seal(sender, &recipient, payload)?;
        self.put(recipient, blob, signer).await
    }

    /// Replaces the keys that may put mail into my mailbox for `scope`
    /// (an empty list removes the scope): plain keys (any uploader) or
    /// [`Registration`]s. Returns my mailbox's state.
    pub async fn register<R: Into<Registration>>(&self, scope: Scope, keys: impl IntoIterator<Item = R>) -> Result<MailboxStatus, NetError> {
        let keys = keys.into_iter().map(Into::into).collect();
        match self.request(&MailboxRequest::Register { scope, keys }).await? {
            MailboxResponse::Registered(s) => Ok(s),
            other => Err(unexpected(other)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_auth_binds_recipient_uploader_and_blob() {
        let signer = SecretKey::generate();
        let (rcpt, other, up) = (SecretKey::generate().public(), SecretKey::generate().public(), SecretKey::generate().public());
        let auth = PutAuth::sign(&signer, &rcpt, &up, b"blob");
        assert!(auth.verify(&rcpt, &up, b"blob"));
        assert!(!auth.verify(&other, &up, b"blob"), "replayed into another mailbox");
        assert!(!auth.verify(&rcpt, &other, b"blob"), "uploaded by another node");
        assert!(!auth.verify(&rcpt, &up, b"blob!"), "another blob");
        let mut forged = auth.clone();
        forged.key = SecretKey::generate().public();
        assert!(!forged.verify(&rcpt, &up, b"blob"), "claims another key");
        assert_ne!(PutAuth::sign(&signer, &rcpt, &up, b"blob").nonce, auth.nonce, "fresh nonce per put");
    }
}
