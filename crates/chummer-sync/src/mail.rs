//! Play-by-post through the relay mailbox.
//!
//! A [`MailMessage`] is encoded, cut into [`Chunk`]s that fit the relay's
//! blob limit, and each chunk is sealed to the recipient
//! ([`chummer_net::seal`]) and stored with a mailbox `put`. The recipient
//! opens each blob, checks the signer, and puts the chunks back together
//! in an [`Inbox`] (which is saved with the rest of its state, since mail
//! is acked, and so deleted on the relay, as soon as it is read).
//!
//! Outboxes are split into several [`SubmitBatch`]es before chunking
//! ([`split_batch`]), so one large command does not make every other one
//! wait for reassembly; only a single oversized command or a snapshot is
//! chunked.

use std::collections::BTreeMap;

use chummer_net::mailbox::{MailboxClient, MailboxError};
use chummer_net::seal::{Opened, SealError};
use chummer_net::{EndpointId, NetError, SecretKey};
use serde::{Deserialize, Serialize};

use crate::msg::{self, DecodeError, MailMessage, SubmitBatch};

/// The relay's default largest blob (`max_blob_bytes`).
pub const DEFAULT_BLOB_LIMIT: usize = 256 * 1024;

/// Room left in each blob for the seal (sealed box, sender id, signature)
/// and the chunk header.
pub const SEAL_OVERHEAD: usize = 512;

/// Smallest blob limit we work with; a relay that allows less is unusable.
pub const MIN_BLOB_LIMIT: usize = 4 * 1024;

/// Partial messages kept at most (oldest dropped first).
const PARTIAL_LIMIT: usize = 256;

/// Bytes of chunks kept at most over all partial messages (oldest
/// messages dropped first). Partial mail is saved with the replica or the
/// authority, so a sender whose messages never complete must not make it
/// grow without bound.
pub const MAX_PARTIAL_BYTES: usize = 64 * 1024 * 1024;

/// Most chunks a message may have; mail claiming more is dropped. (A
/// 16 MiB snapshot in blobs of [`MIN_BLOB_LIMIT`] needs about 4600.)
pub const MAX_CHUNKS: u32 = 8192;

/// Messages fetched per mailbox request.
const FETCH_LIMIT: u32 = 64;

/// One piece of an encoded [`MailMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Random per message.
    pub message: [u8; 16],
    pub index: u32,
    pub total: u32,
    pub data: Vec<u8>,
}

/// The sealed payloads (before sealing) for `msg`: each fits in a blob of
/// `blob_limit` bytes once sealed.
pub fn split(msg: &MailMessage, blob_limit: usize) -> Vec<Vec<u8>> {
    let bytes = msg::encode(msg);
    let room = blob_limit.max(MIN_BLOB_LIMIT) - SEAL_OVERHEAD;
    let id = chummer_net::invite::random_id();
    let pieces: Vec<&[u8]> = if bytes.is_empty() { vec![&[][..]] } else { bytes.chunks(room).collect() };
    let total = pieces.len() as u32;
    pieces.into_iter().enumerate().map(|(i, data)| msg::encode(&Chunk { message: id, index: i as u32, total, data: data.to_vec() })).collect()
}

/// Splits a batch into batches that each fit in one blob when they can (a
/// single command larger than that stays alone, and is chunked).
pub fn split_batch(batch: SubmitBatch, blob_limit: usize) -> Vec<SubmitBatch> {
    let room = blob_limit.max(MIN_BLOB_LIMIT) - SEAL_OVERHEAD - 64;
    let empty = SubmitBatch { ops: Vec::new(), ..batch.clone() };
    let base = msg::encode(&MailMessage::Client(msg::ClientMessage::Submit(empty.clone()))).len();
    let mut out = Vec::new();
    let mut cur = empty.clone();
    let mut size = base;
    for op in batch.ops {
        let len = postcard::to_stdvec(&op).map(|v| v.len()).unwrap_or(0);
        if !cur.ops.is_empty() && size + len > room {
            out.push(std::mem::replace(&mut cur, empty.clone()));
            size = base;
        }
        size += len;
        cur.ops.push(op);
    }
    if !cur.ops.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Partial {
    total: u32,
    parts: BTreeMap<u32, Vec<u8>>,
    /// Arrival order, to drop the oldest partials first.
    order: u64,
}

/// Chunks of messages still being put together, per sender.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inbox {
    partial: BTreeMap<(EndpointId, [u8; 16]), Partial>,
    counter: u64,
}

impl Inbox {
    /// Takes one opened payload from `sender`. Returns the message when
    /// this chunk completed it.
    pub fn accept(&mut self, sender: EndpointId, payload: &[u8]) -> Result<Option<MailMessage>, DecodeError> {
        let chunk: Chunk = msg::decode(payload)?;
        if chunk.total <= 1 {
            return msg::decode(&chunk.data).map(Some);
        }
        if chunk.index >= chunk.total || chunk.total > MAX_CHUNKS || chunk.data.len() > MAX_PARTIAL_BYTES {
            return Ok(None);
        }
        let key = (sender, chunk.message);
        self.counter += 1;
        let order = self.counter;
        let p = self.partial.entry(key).or_insert_with(|| Partial { total: chunk.total, parts: BTreeMap::new(), order });
        if chunk.index >= p.total {
            // Chunks of one message disagree on how many there are.
            return Ok(None);
        }
        p.parts.insert(chunk.index, chunk.data);
        if p.parts.len() as u32 >= p.total {
            let p = self.partial.remove(&key).expect("present");
            let bytes: Vec<u8> = p.parts.into_values().flatten().collect();
            return msg::decode(&bytes).map(Some);
        }
        let mut bytes: usize = self.partial.values().flat_map(|p| p.parts.values()).map(Vec::len).sum();
        while self.partial.len() > PARTIAL_LIMIT || bytes > MAX_PARTIAL_BYTES {
            let oldest = self.partial.iter().min_by_key(|(_, p)| p.order).map(|(k, _)| *k).expect("not empty");
            let gone = self.partial.remove(&oldest).expect("present");
            bytes -= gone.parts.values().map(Vec::len).sum::<usize>();
        }
        Ok(None)
    }

    /// Messages waiting for more chunks.
    pub fn pending(&self) -> usize {
        self.partial.len()
    }
}

/// The message in `payload` when it is complete in one chunk (mail from
/// someone who is not a member yet: nothing of theirs is kept).
pub fn single(payload: &[u8]) -> Option<MailMessage> {
    let chunk: Chunk = msg::decode(payload).ok()?;
    (chunk.total <= 1).then(|| msg::decode(&chunk.data).ok()).flatten()
}

/// Seals and stores `msg` for `recipient`, in as many blobs as it takes:
/// sealed by `me` (this node), each put signed by `signer` (a key the
/// recipient registered with its mailbox). `blob_limit` is lowered when
/// the relay says it allows less, and the message is sent again in
/// smaller pieces. Returns the number of blobs.
pub async fn send(client: &MailboxClient, me: &SecretKey, signer: &SecretKey, recipient: EndpointId, msg: &MailMessage, blob_limit: &mut usize) -> Result<usize, NetError> {
    'again: loop {
        let parts = split(msg, *blob_limit);
        let n = parts.len();
        for p in parts {
            match client.put_sealed(me, signer, recipient, &p).await {
                Ok(_) => {}
                Err(NetError::Mailbox(MailboxError::TooLarge { max, .. })) if (max as usize) < *blob_limit && max as usize >= MIN_BLOB_LIMIT => {
                    *blob_limit = max as usize;
                    continue 'again;
                }
                Err(e) => return Err(e),
            }
        }
        return Ok(n);
    }
}

/// One fetched mail item: its mailbox id and what opening it gave.
pub type Fetched = (u64, Result<Opened, SealError>);

/// Fetches up to one page of my mail. The caller processes it, saves, and
/// then acks the ids; `more` says whether to fetch again.
pub async fn fetch_page(client: &MailboxClient, me: &SecretKey) -> Result<(Vec<Fetched>, bool), NetError> {
    let (items, more) = client.fetch_opened(me, FETCH_LIMIT).await?;
    Ok((items.into_iter().map(|(item, o)| (item.id, o)).collect(), more))
}
