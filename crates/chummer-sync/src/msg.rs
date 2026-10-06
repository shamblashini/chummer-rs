//! The sync messages: what travels inside chummer-net's opaque campaign
//! payloads and mailbox blobs.
//!
//! Every message is encoded as one version byte ([`SYNC_VERSION`]) and the
//! postcard form of a [`ClientMessage`], [`ServerMessage`] or
//! [`MailMessage`]. A peer refuses a version it does not know rather than
//! misreading it.
//!
//! Flow over a live connection (each `ClientMessage` is one
//! `Request::Submit`, its `ServerMessage` the `Response::Ack` payload):
//! 1. After chummer-net's `Welcome`, the client sends [`ClientMessage::Join`]
//!    with the versions it already has; the host answers
//!    [`ServerMessage::Joined`]: the membership (which characters this
//!    member may see) and a [`Push`] for every character the client is
//!    behind on.
//! 2. [`ClientMessage::Submit`] carries a [`SubmitBatch`]; the answer is an
//!    [`Ack`], which also carries the authoritative log from the batch's
//!    base version, so the client never has to order an ack against pushes.
//! 3. The host sends [`ServerMessage::Push`] on its own when another member
//!    (usually the GM) changed a character the client sees.
//! 4. [`ClientMessage::Resync`] asks for a snapshot after a hash mismatch.

use std::fmt;

use chummer_core::command::Envelope;
use chummer_net::invite::Role;
use chummer_net::EndpointId;
use serde::{Deserialize, Serialize};

/// Version of the sync messages.
pub const SYNC_VERSION: u8 = 1;

/// A BLAKE3 hash of a character's canonical form ([`chummer_core::command::state_hash`]).
pub type Hash = [u8; 32];

/// Identifies a character within a campaign. Chosen by the authority when
/// the character joins the campaign (any unique text; the campaign file's
/// id for it).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CharacterId(pub String);

impl CharacterId {
    pub fn new(id: impl Into<String>) -> CharacterId {
        CharacterId(id.into())
    }
}

impl fmt::Display for CharacterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for CharacterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CharacterId({})", self.0)
    }
}

/// A unique id for one submitted command, so a command delivered twice (a
/// resubmitted outbox, a mailbox replay) runs once. `origin` is random per
/// replica (or authority), `seq` counts up.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OpId {
    pub origin: [u8; 16],
    pub seq: u64,
}

impl fmt::Debug for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpId({:02x}{:02x}{:02x}{:02x}:{})", self.origin[0], self.origin[1], self.origin[2], self.origin[3], self.seq)
    }
}

/// A command as submitted: its id and its envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Op {
    pub id: OpId,
    pub env: Envelope,
}

/// Commands for one character, made on top of the client's confirmed
/// state `base_version` (hash `base_hash`), in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitBatch {
    pub character: CharacterId,
    pub base_version: u64,
    pub base_hash: Hash,
    pub ops: Vec<Op>,
}

/// A command the authority ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    pub op: OpId,
    /// The character's version after it (unchanged when the command found
    /// nothing to do).
    pub version: u64,
    pub hash: Hash,
    /// False when there was nothing to change; no log entry was made.
    pub changed: bool,
    /// True when it ran on a newer state than the client's base.
    pub rebased: bool,
    pub description: String,
}

/// A command the engine refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedOp {
    pub op: OpId,
    pub reason: String,
    /// The reason is a question (see [`chummer_core::command::Rejected::confirm`]).
    pub confirm: bool,
}

/// The answer to a [`SubmitBatch`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ack {
    pub character: CharacterId,
    pub accepted: Vec<Accepted>,
    pub rejected: Vec<RejectedOp>,
    /// The authoritative state from the batch's base to now.
    pub update: Push,
}

/// One command in the authority's log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// The character's version after this command.
    pub version: u64,
    pub op: OpId,
    /// As applied; `author` is the proven sender's node id in hex.
    pub env: Envelope,
    pub author: EndpointId,
    /// What it did ("Raised Pistols to 5 (10 karma)").
    pub description: String,
}

/// How a [`Push`] brings the client up to date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PushBody {
    /// The commands after `from_version`, in order.
    Entries(Vec<Entry>),
    /// The whole character at `version` ([`chummer_core::command::snapshot`]),
    /// for a client that is too far behind or diverged; `recent` are the
    /// last commands before it, for the activity feed only.
    Snapshot { bytes: Vec<u8>, recent: Vec<Entry> },
}

/// The authority's state of one character, from `from_version` to `version`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Push {
    pub character: CharacterId,
    pub name: String,
    pub from_version: u64,
    pub body: PushBody,
    pub version: u64,
    pub hash: Hash,
}

impl Push {
    pub fn is_snapshot(&self) -> bool {
        matches!(self.body, PushBody::Snapshot { .. })
    }

    /// The log entries this push carries (the feed part, for a snapshot).
    pub fn entries(&self) -> &[Entry] {
        match &self.body {
            PushBody::Entries(e) => e,
            PushBody::Snapshot { recent, .. } => recent,
        }
    }
}

/// "My copy of this character drifted: send me a snapshot."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResyncRequest {
    pub character: CharacterId,
    pub have_version: u64,
}

/// What the client already has of one character.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Have {
    pub character: CharacterId,
    pub version: u64,
    pub hash: Hash,
}

/// A character as listed in the membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterInfo {
    pub id: CharacterId,
    pub name: String,
    /// The player who owns it; `None` for the GM's own (NPCs).
    pub owner: Option<EndpointId>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub id: EndpointId,
    pub role: Role,
    pub name: String,
}

/// Who is in the campaign and which characters the receiving member sees:
/// a player their own, the GM all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// The campaign's name, as the GM calls it.
    pub campaign_name: String,
    pub you: EndpointId,
    pub role: Role,
    pub members: Vec<MemberInfo>,
    pub characters: Vec<CharacterInfo>,
}

/// One line of the activity feed: "<author>: <text>".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedEntry {
    /// Unix milliseconds (the command's time).
    pub at: i64,
    pub character: CharacterId,
    pub character_name: String,
    pub author: EndpointId,
    pub author_name: String,
    pub author_role: Role,
    pub text: String,
    /// The character's version after it, for applied commands.
    pub version: Option<u64>,
    /// Set for a refused command: the reason.
    pub rejected: Option<String>,
}

impl fmt::Display for FeedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.author_name, self.text)?;
        if let Some(r) = &self.rejected {
            write!(f, " (refused: {r})")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// First message after chummer-net's welcome. `name` is shown to the
    /// GM and other members.
    Join { name: String, have: Vec<Have> },
    Submit(SubmitBatch),
    Resync(ResyncRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Joined { membership: Membership, pushes: Vec<Push> },
    Ack(Ack),
    Push(Push),
    /// The membership or character list changed (a character was given to
    /// or taken from this member).
    Membership(Membership),
    /// The request could not be handled at all (not a member, not your
    /// character, unknown character).
    Error(String),
}

/// A message sent through the relay mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MailMessage {
    /// Player to authority.
    Client(ClientMessage),
    /// Authority to player.
    Server(ServerMessage),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("empty message")]
    Empty,
    #[error("unsupported sync message version {0} (this program speaks {SYNC_VERSION})")]
    Version(u8),
    #[error("could not decode the message: {0}")]
    Postcard(#[from] postcard::Error),
}

/// One version byte, then postcard.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let mut out = vec![SYNC_VERSION];
    out.extend(postcard::to_stdvec(msg).expect("sync messages serialise"));
    out
}

pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, DecodeError> {
    match bytes.split_first() {
        None => Err(DecodeError::Empty),
        Some((&SYNC_VERSION, rest)) => Ok(postcard::from_bytes(rest)?),
        Some((&v, _)) => Err(DecodeError::Version(v)),
    }
}
