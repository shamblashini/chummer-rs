//! The mailbox store, on disk in a redb database.
//!
//! Tables:
//! - `messages`: id -> postcard [`Stored`] (recipient, sender, key, time, blob)
//! - `inbox`: (recipient, id) -> (received time, signing key, nonce), for
//!   per-recipient listing, the per-key cap and replay checks
//! - `quota`: (sender, day) -> (messages, bytes), for daily sender limits
//! - `registrations`: (owner, scope) -> postcard list of keys that may put
//!   mail into the owner's mailbox
//! - `allowed`: (owner, key) -> in how many of the owner's scopes the key
//!   is (the lookup each put does)
//! - `meta`: "next_id" -> next message id; "format" -> [`FORMAT`]
//!
//! Refused puts are counted in memory only (per owner and day), so a
//! stranger's flood costs no disk writes.
//!
//! Every call takes `now` (Unix seconds) so limits and expiry can be tested
//! with a fake clock.

use std::path::{Path, PathBuf};

use chummer_net::mailbox::{MailItem, MailboxError, MailboxStatus, Scope, MAX_FETCH_BYTES, MAX_KEYS_PER_SCOPE, MAX_SCOPES};
use iroh::{EndpointId, PublicKey};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};

const MESSAGES: TableDefinition<u64, &[u8]> = TableDefinition::new("messages");
const INBOX: TableDefinition<([u8; 32], u64), (u64, [u8; 32], [u8; 16])> = TableDefinition::new("inbox");
const QUOTA: TableDefinition<([u8; 32], u64), (u64, u64)> = TableDefinition::new("quota");
const REGISTRATIONS: TableDefinition<([u8; 32], [u8; 16]), &[u8]> = TableDefinition::new("registrations");
const ALLOWED: TableDefinition<([u8; 32], [u8; 32]), u32> = TableDefinition::new("allowed");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

/// The database layout. A database of another layout (the first mailbox
/// protocol) is emptied when opened: it only held mail in transit, and
/// clients mail again what was not answered.
pub const FORMAT: u64 = 2;

const DAY: u64 = 24 * 60 * 60;

/// Mailbox limits. All are configurable in the relay's TOML file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Largest sealed blob accepted.
    pub max_blob_bytes: u64,
    /// Most messages waiting for one recipient.
    pub max_messages_per_recipient: u64,
    /// Most messages one sender may upload per UTC day.
    pub max_messages_per_sender_per_day: u64,
    /// Most bytes one sender may upload per UTC day.
    pub max_bytes_per_sender_per_day: u64,
    /// Most messages signed by one key waiting in one mailbox (a second
    /// net behind the sender quotas: a leaked member key cannot fill a
    /// mailbox).
    pub max_messages_per_key: u64,
    /// Messages not collected within this many seconds are deleted.
    pub expiry_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_blob_bytes: 256 * 1024,
            max_messages_per_recipient: 1000,
            max_messages_per_sender_per_day: 2000,
            max_bytes_per_sender_per_day: 64 * 1024 * 1024,
            max_messages_per_key: 200,
            expiry_secs: 30 * DAY,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    recipient: [u8; 32],
    sender: [u8; 32],
    key: [u8; 32],
    received: u64,
    blob: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
#[error("mailbox database error: {0}")]
pub struct StoreError(String);

fn db_err(e: impl std::fmt::Display) -> MailboxError {
    MailboxError::Internal(e.to_string())
}

/// Opens the database again (after an I/O error redb refuses all work
/// until it is reopened).
pub type Opener = Box<dyn Fn() -> Result<Database, redb::DatabaseError> + Send + Sync>;

/// The mailbox database.
pub struct Store {
    /// `None` after a failed reopen; the next call tries again.
    db: std::sync::RwLock<Option<Database>>,
    opener: Opener,
    limits: Limits,
    /// Refused puts per (owner, day), for mailboxes with registrations.
    refused: std::sync::Mutex<std::collections::HashMap<([u8; 32], u64), u64>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("limits", &self.limits).finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (or creates) the database at `path`.
    pub fn open(path: &Path, limits: Limits) -> Result<Store, StoreError> {
        Store::try_open(path, limits).map_err(|(e, _)| e)
    }

    /// As [`Store::open`], but a damaged database file (not a permission
    /// or disk problem) is moved aside to `<file>.damaged-<unix time>` and
    /// an empty one is made, so the relay keeps running: the mailbox only
    /// holds mail in transit, and clients send what was not answered
    /// again. Returns where the damaged file went, if it was moved.
    pub fn open_or_recover(path: &Path, limits: Limits, now: u64) -> Result<(Store, Option<PathBuf>), StoreError> {
        match Store::try_open(path, limits.clone()) {
            Ok(s) => Ok((s, None)),
            Err((e, true)) if path.is_file() => {
                let mut aside = path.as_os_str().to_owned();
                aside.push(format!(".damaged-{now}"));
                let aside = PathBuf::from(aside);
                std::fs::rename(path, &aside).map_err(|r| StoreError(format!("{e}; and moving it aside failed: {r}")))?;
                Ok((Store::open(path, limits)?, Some(aside)))
            }
            Err((e, _)) => Err(e),
        }
    }

    /// Opens; on failure, also says whether the file looks damaged.
    fn try_open(path: &Path, limits: Limits) -> Result<Store, (StoreError, bool)> {
        fn io_damage(e: &std::io::Error) -> bool {
            matches!(e.kind(), std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof)
        }
        let owned = path.to_owned();
        let opener: Opener = Box::new(move || Database::create(&owned));
        Store::open_with(opener, limits).map_err(|e| {
            let damaged = match &e {
                redb::Error::Corrupted(_) | redb::Error::RepairAborted => true,
                redb::Error::Io(io) => io_damage(io),
                _ => false,
            };
            (StoreError(e.to_string()), damaged)
        })
    }

    /// A store on whatever `opener` opens (for tests and other backends).
    pub fn open_with(opener: Opener, limits: Limits) -> Result<Store, redb::Error> {
        let db = Store::prepare(&opener)?;
        Ok(Store { db: std::sync::RwLock::new(Some(db)), opener, limits, refused: Default::default() })
    }

    /// Opens and makes the tables, so read transactions never miss them.
    /// A database of another [`FORMAT`] is emptied first.
    fn prepare(opener: &Opener) -> Result<Database, redb::Error> {
        let db = opener()?;
        let txn = db.begin_write()?;
        let format = match txn.open_table(META) {
            Ok(meta) => meta.get("format")?.map(|g| g.value()),
            Err(_) => None,
        };
        if format != Some(FORMAT) {
            let old = txn.list_tables()?.count();
            for name in ["messages", "inbox", "quota", "registrations", "allowed", "refused", "meta"] {
                txn.delete_table(redb::TableDefinition::<&str, &[u8]>::new(name)).ok();
            }
            if old > 0 {
                tracing::warn!("the mailbox database has an older layout; it was emptied (mail in it is lost; clients send unanswered mail again)");
            }
            txn.open_table(META)?.insert("format", FORMAT)?;
        }
        txn.open_table(MESSAGES)?;
        txn.open_table(INBOX)?;
        txn.open_table(QUOTA)?;
        txn.open_table(REGISTRATIONS)?;
        txn.open_table(ALLOWED)?;
        txn.open_table(META)?;
        txn.commit()?;
        Ok(db)
    }

    /// Runs `f` on the database. After an error the database is opened
    /// again and `f` retried once: redb refuses everything after an I/O
    /// error (a full disk) until it is reopened, which used to leave the
    /// mailbox failing until the relay was restarted.
    fn with_db<T>(&self, f: impl Fn(&Database) -> Result<T, MailboxError>) -> Result<T, MailboxError> {
        {
            let g = self.db.read().expect("poisoned");
            if let Some(db) = g.as_ref() {
                match f(db) {
                    Err(MailboxError::Internal(e)) => tracing::warn!("mailbox database: {e}; opening it again"),
                    r => return r,
                }
            }
        }
        {
            let mut g = self.db.write().expect("poisoned");
            // Close first: the file is locked while open.
            drop(g.take());
            *g = Some(Store::prepare(&self.opener).map_err(db_err)?);
        }
        let g = self.db.read().expect("poisoned");
        f(g.as_ref().expect("just opened"))
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn expired(&self, received: u64, now: u64) -> bool {
        received.saturating_add(self.limits.expiry_secs) <= now
    }

    /// Stores `blob` from `sender` (the uploading node) for `recipient`,
    /// signed by `key` with `nonce` (the signature is checked by the
    /// caller). Refused unless `recipient` registered `key`.
    pub fn put(
        &self,
        sender: EndpointId,
        recipient: EndpointId,
        key: PublicKey,
        nonce: [u8; 16],
        blob: Vec<u8>,
        now: u64,
    ) -> Result<u64, MailboxError> {
        let l = &self.limits;
        let len = blob.len() as u64;
        if len > l.max_blob_bytes {
            self.note_refused(recipient, now);
            return Err(MailboxError::TooLarge {
                len,
                max: l.max_blob_bytes,
            });
        }
        let rcpt = *recipient.as_bytes();
        let snd = *sender.as_bytes();
        let r = self.with_db(|db| self.put_in(db, snd, rcpt, *key.as_bytes(), nonce, &blob, now));
        if matches!(&r, Err(e) if !matches!(e, MailboxError::Internal(_))) {
            self.note_refused(recipient, now);
        }
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn put_in(&self, db: &Database, snd: [u8; 32], rcpt: [u8; 32], key: [u8; 32], nonce: [u8; 16], blob: &[u8], now: u64) -> Result<u64, MailboxError> {
        let l = &self.limits;
        let len = blob.len() as u64;
        let txn = db.begin_write().map_err(db_err)?;
        let id;
        {
            let allowed = txn.open_table(ALLOWED).map_err(db_err)?;
            if allowed.get((rcpt, key)).map_err(db_err)?.is_none() {
                return Err(MailboxError::NotAllowed);
            }
            let mut inbox = txn.open_table(INBOX).map_err(db_err)?;
            let (mut waiting, mut by_key) = (0u64, 0u64);
            for entry in inbox.range((rcpt, 0)..=(rcpt, u64::MAX)).map_err(db_err)? {
                let (_, v) = entry.map_err(db_err)?;
                let (received, k, n) = v.value();
                if self.expired(received, now) {
                    continue;
                }
                if k == key && n == nonce {
                    return Err(MailboxError::Replayed);
                }
                waiting += 1;
                by_key += u64::from(k == key);
            }
            if waiting >= l.max_messages_per_recipient {
                return Err(MailboxError::RecipientFull {
                    max: l.max_messages_per_recipient,
                });
            }
            if by_key >= l.max_messages_per_key {
                return Err(MailboxError::KeyFull { max: l.max_messages_per_key });
            }
            let mut quota = txn.open_table(QUOTA).map_err(db_err)?;
            let day = now / DAY;
            let (count, bytes) = quota
                .get((snd, day))
                .map_err(db_err)?
                .map(|g| g.value())
                .unwrap_or((0, 0));
            if count + 1 > l.max_messages_per_sender_per_day {
                return Err(MailboxError::SenderQuota {
                    what: format!("{} messages per day", l.max_messages_per_sender_per_day),
                });
            }
            if bytes + len > l.max_bytes_per_sender_per_day {
                return Err(MailboxError::SenderQuota {
                    what: format!("{} bytes per day", l.max_bytes_per_sender_per_day),
                });
            }
            quota
                .insert((snd, day), (count + 1, bytes + len))
                .map_err(db_err)?;

            let mut meta = txn.open_table(META).map_err(db_err)?;
            id = meta
                .get("next_id")
                .map_err(db_err)?
                .map(|g| g.value())
                .unwrap_or(1);
            meta.insert("next_id", id + 1).map_err(db_err)?;

            let stored = Stored {
                recipient: rcpt,
                sender: snd,
                key,
                received: now,
                blob: blob.to_vec(),
            };
            let bytes = postcard::to_stdvec(&stored).map_err(db_err)?;
            let mut messages = txn.open_table(MESSAGES).map_err(db_err)?;
            messages.insert(id, bytes.as_slice()).map_err(db_err)?;
            inbox.insert((rcpt, id), (now, key, nonce)).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(id)
    }

    /// Counts a refused put to `recipient` (only for mailboxes with
    /// registrations: a stranger must not make entries for made-up ids).
    pub fn note_refused(&self, recipient: EndpointId, now: u64) {
        let rcpt = *recipient.as_bytes();
        let registered = self.with_db(|db| {
            let txn = db.begin_read().map_err(db_err)?;
            let regs = txn.open_table(REGISTRATIONS).map_err(db_err)?;
            let any = regs.range((rcpt, [0u8; 16])..=(rcpt, [0xffu8; 16])).map_err(db_err)?.next().is_some();
            Ok(any)
        });
        if registered.unwrap_or(false) {
            let day = now / DAY;
            let mut r = self.refused.lock().expect("poisoned");
            r.retain(|(_, d), _| *d >= day);
            *r.entry((rcpt, day)).or_default() += 1;
        }
    }

    /// Replaces the keys that may put mail into `owner`'s mailbox for
    /// `scope` (an empty list removes the scope). Returns the mailbox's
    /// state.
    pub fn register(&self, owner: EndpointId, scope: Scope, keys: &[PublicKey], now: u64) -> Result<MailboxStatus, MailboxError> {
        if keys.len() > MAX_KEYS_PER_SCOPE {
            return Err(MailboxError::TooManyKeys { what: format!("{} keys per scope", MAX_KEYS_PER_SCOPE) });
        }
        let mut new: Vec<[u8; 32]> = keys.iter().map(|k| *k.as_bytes()).collect();
        new.sort_unstable();
        new.dedup();
        let own = *owner.as_bytes();
        self.with_db(|db| {
            let txn = db.begin_write().map_err(db_err)?;
            {
                let mut regs = txn.open_table(REGISTRATIONS).map_err(db_err)?;
                let old: Vec<[u8; 32]> = match regs.get((own, scope)).map_err(db_err)? {
                    Some(g) => postcard::from_bytes(g.value()).map_err(db_err)?,
                    None => {
                        let scopes = regs.range((own, [0u8; 16])..=(own, [0xffu8; 16])).map_err(db_err)?.count();
                        if !new.is_empty() && scopes >= MAX_SCOPES {
                            return Err(MailboxError::TooManyKeys { what: format!("{MAX_SCOPES} scopes") });
                        }
                        Vec::new()
                    }
                };
                if old == new {
                    // Idempotent: nothing to write.
                } else {
                    let mut allowed = txn.open_table(ALLOWED).map_err(db_err)?;
                    for k in old.iter().filter(|k| !new.contains(k)) {
                        let n = allowed.get((own, *k)).map_err(db_err)?.map(|g| g.value()).unwrap_or(0);
                        if n <= 1 {
                            allowed.remove((own, *k)).map_err(db_err)?;
                        } else {
                            allowed.insert((own, *k), n - 1).map_err(db_err)?;
                        }
                    }
                    for k in new.iter().filter(|k| !old.contains(k)) {
                        let n = allowed.get((own, *k)).map_err(db_err)?.map(|g| g.value()).unwrap_or(0);
                        allowed.insert((own, *k), n + 1).map_err(db_err)?;
                    }
                    if new.is_empty() {
                        regs.remove((own, scope)).map_err(db_err)?;
                    } else {
                        let bytes = postcard::to_stdvec(&new).map_err(db_err)?;
                        regs.insert((own, scope), bytes.as_slice()).map_err(db_err)?;
                    }
                }
            }
            txn.commit().map_err(db_err)?;
            self.status_in(db, owner, now)
        })
    }

    /// `owner`'s mailbox: registered keys, waiting mail (per key) and
    /// refused puts today.
    pub fn status(&self, owner: EndpointId, now: u64) -> Result<MailboxStatus, MailboxError> {
        self.with_db(|db| self.status_in(db, owner, now))
    }

    fn status_in(&self, db: &Database, owner: EndpointId, now: u64) -> Result<MailboxStatus, MailboxError> {
        let own = *owner.as_bytes();
        let txn = db.begin_read().map_err(db_err)?;
        let allowed = txn.open_table(ALLOWED).map_err(db_err)?;
        let keys = allowed.range((own, [0u8; 32])..=(own, [0xffu8; 32])).map_err(db_err)?.count() as u32;
        let inbox = txn.open_table(INBOX).map_err(db_err)?;
        let mut by_key: std::collections::BTreeMap<[u8; 32], u64> = Default::default();
        let mut waiting = 0;
        for entry in inbox.range((own, 0)..=(own, u64::MAX)).map_err(db_err)? {
            let (_, v) = entry.map_err(db_err)?;
            let (received, k, _) = v.value();
            if !self.expired(received, now) {
                waiting += 1;
                *by_key.entry(k).or_default() += 1;
            }
        }
        let refused_today = self.refused.lock().expect("poisoned").get(&(own, now / DAY)).copied().unwrap_or(0);
        Ok(MailboxStatus {
            keys,
            waiting,
            by_key: by_key.into_iter().filter_map(|(k, n)| PublicKey::from_bytes(&k).ok().map(|k| (k, n))).collect(),
            refused_today,
        })
    }

    /// Whether `owner` may receive mail signed by `key`.
    pub fn allows(&self, owner: EndpointId, key: &PublicKey) -> Result<bool, MailboxError> {
        self.with_db(|db| {
            let txn = db.begin_read().map_err(db_err)?;
            let allowed = txn.open_table(ALLOWED).map_err(db_err)?;
            Ok(allowed.get((*owner.as_bytes(), *key.as_bytes())).map_err(db_err)?.is_some())
        })
    }

    /// Up to `limit` of `recipient`'s oldest unexpired messages (at most
    /// [`MAX_FETCH_BYTES`] of blobs, but always at least one), and whether
    /// there are more.
    pub fn fetch(&self,
        recipient: EndpointId,
        limit: u32,
        now: u64,) -> Result<(Vec<MailItem>, bool), MailboxError> {
        self.with_db(|db| self.fetch_in(db, recipient, limit, now))
    }

    fn fetch_in(&self, db: &Database,
        recipient: EndpointId,
        limit: u32,
        now: u64,) -> Result<(Vec<MailItem>, bool), MailboxError> {
        let rcpt = *recipient.as_bytes();
        let txn = db.begin_read().map_err(db_err)?;
        let inbox = txn.open_table(INBOX).map_err(db_err)?;
        let messages = txn.open_table(MESSAGES).map_err(db_err)?;
        let mut items = Vec::new();
        let mut total = 0usize;
        for entry in inbox.range((rcpt, 0)..=(rcpt, u64::MAX)).map_err(db_err)? {
            let (key, v) = entry.map_err(db_err)?;
            if self.expired(v.value().0, now) {
                continue;
            }
            let id = key.value().1;
            let Some(raw) = messages.get(id).map_err(db_err)? else {
                continue;
            };
            let stored: Stored = postcard::from_bytes(raw.value()).map_err(db_err)?;
            if items.len() >= limit.max(1) as usize
                || (!items.is_empty() && total + stored.blob.len() > MAX_FETCH_BYTES)
            {
                return Ok((items, true));
            }
            total += stored.blob.len();
            items.push(MailItem {
                id,
                sender: EndpointId::from_bytes(&stored.sender).map_err(db_err)?,
                key: PublicKey::from_bytes(&stored.key).map_err(db_err)?,
                received: stored.received,
                blob: stored.blob,
            });
        }
        Ok((items, false))
    }

    /// Deletes `recipient`'s messages `ids`; ids of other recipients are
    /// ignored. Returns how many were deleted.
    pub fn ack(&self, recipient: EndpointId, ids: &[u64]) -> Result<u32, MailboxError> {
        self.with_db(|db| self.ack_in(db, recipient, ids))
    }

    fn ack_in(&self, db: &Database, recipient: EndpointId, ids: &[u64]) -> Result<u32, MailboxError> {
        let rcpt = *recipient.as_bytes();
        let txn = db.begin_write().map_err(db_err)?;
        let mut removed = 0;
        {
            let mut inbox = txn.open_table(INBOX).map_err(db_err)?;
            let mut messages = txn.open_table(MESSAGES).map_err(db_err)?;
            for &id in ids {
                if inbox.remove((rcpt, id)).map_err(db_err)?.is_some() {
                    messages.remove(id).map_err(db_err)?;
                    removed += 1;
                }
            }
        }
        txn.commit().map_err(db_err)?;
        Ok(removed)
    }

    /// Deletes expired messages and old quota counters. Returns how many
    /// messages were deleted.
    pub fn purge(&self, now: u64) -> Result<u64, MailboxError> {
        self.with_db(|db| self.purge_in(db, now))
    }

    fn purge_in(&self, db: &Database, now: u64) -> Result<u64, MailboxError> {
        let txn = db.begin_write().map_err(db_err)?;
        let mut removed = 0;
        {
            let mut inbox = txn.open_table(INBOX).map_err(db_err)?;
            let mut messages = txn.open_table(MESSAGES).map_err(db_err)?;
            let mut dead = Vec::new();
            for entry in inbox.iter().map_err(db_err)? {
                let (key, v) = entry.map_err(db_err)?;
                if self.expired(v.value().0, now) {
                    dead.push(key.value());
                }
            }
            for key in dead {
                inbox.remove(key).map_err(db_err)?;
                messages.remove(key.1).map_err(db_err)?;
                removed += 1;
            }
            let today = now / DAY;
            let mut quota = txn.open_table(QUOTA).map_err(db_err)?;
            quota.retain(|(_, day), _| day >= today).map_err(db_err)?;

        }
        txn.commit().map_err(db_err)?;
        Ok(removed)
    }

    /// Number of stored messages (expired ones included until purged).
    pub fn len(&self) -> Result<u64, MailboxError> {
        self.with_db(|db| self.len_in(db))
    }

    fn len_in(&self, db: &Database) -> Result<u64, MailboxError> {
        let txn = db.begin_read().map_err(db_err)?;
        let messages = txn.open_table(MESSAGES).map_err(db_err)?;
        messages.len().map_err(db_err)
    }

    pub fn is_empty(&self) -> Result<bool, MailboxError> {
        Ok(self.len()? == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn store(name: &str, limits: Limits) -> (Store, Tmp) {
        let path = std::env::temp_dir().join(format!(
            "chummer-relay-store-{}-{name}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path, limits).unwrap(), Tmp(path))
    }

    fn id() -> EndpointId {
        SecretKey::generate().public()
    }

    const T0: u64 = 1_800_000_000;
    const SCOPE: Scope = [7; 16];

    /// A recipient that takes mail signed by `keys`.
    fn mailbox(s: &Store, keys: &[PublicKey]) -> EndpointId {
        let r = id();
        s.register(r, SCOPE, keys, T0).unwrap();
        r
    }

    fn nonce() -> [u8; 16] {
        chummer_net::invite::random_id()
    }

    /// A put signed (as far as the store knows) by `k`.
    fn put(s: &Store, k: PublicKey, rcpt: EndpointId, blob: Vec<u8>, now: u64) -> Result<u64, MailboxError> {
        s.put(id(), rcpt, k, nonce(), blob, now)
    }

    #[test]
    fn put_fetch_ack() {
        let (s, _t) = store("basic", Limits::default());
        let k = id();
        let (a, b, c) = (id(), mailbox(&s, &[k]), mailbox(&s, &[k]));
        let m1 = s.put(a, b, k, nonce(), vec![1], T0).unwrap();
        let m2 = s.put(c, b, k, nonce(), vec![2], T0 + 1).unwrap();
        s.put(a, c, k, nonce(), vec![3], T0).unwrap();
        let (items, more) = s.fetch(b, 10, T0 + 2).unwrap();
        assert!(!more);
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [m1, m2]);
        assert_eq!(items[0].sender, a);
        assert_eq!(items[0].key, k);
        assert_eq!(items[1].blob, [2]);
        let (first, more) = s.fetch(b, 1, T0 + 2).unwrap();
        assert!(more);
        assert_eq!(first.len(), 1);
        // c cannot delete b's mail.
        assert_eq!(s.ack(c, &[m1, m2]).unwrap(), 0);
        assert_eq!(s.ack(b, &[m1, m2, 999]).unwrap(), 2);
        assert!(s.fetch(b, 10, T0 + 3).unwrap().0.is_empty());
        assert_eq!(s.len().unwrap(), 1);
    }

    #[test]
    fn only_registered_keys_may_put() {
        let (s, _t) = store("keys", Limits::default());
        let (member, stranger) = (id(), id());
        let gm = mailbox(&s, &[member]);
        assert_eq!(put(&s, stranger, gm, vec![1], T0), Err(MailboxError::NotAllowed));
        put(&s, member, gm, vec![1], T0).unwrap();
        // A mailbox with no registrations takes nothing.
        assert_eq!(put(&s, member, id(), vec![1], T0), Err(MailboxError::NotAllowed));
        // Refused puts are counted for the owner.
        let st = s.status(gm, T0).unwrap();
        assert_eq!((st.keys, st.waiting, st.refused_today), (1, 1, 1));
        assert_eq!(st.by_key, [(member, 1)]);
    }

    #[test]
    fn registration_replaces_per_scope_and_is_idempotent() {
        let (s, _t) = store("register", Limits::default());
        let (a, b, c) = (id(), id(), id());
        let owner = id();
        let (one, two) = ([1u8; 16], [2u8; 16]);
        assert_eq!(s.register(owner, one, &[a, b], T0).unwrap().keys, 2);
        assert_eq!(s.register(owner, one, &[b, a, a], T0).unwrap().keys, 2, "the same set again");
        // Another scope (another campaign) may name the same key.
        assert_eq!(s.register(owner, two, &[b, c], T0).unwrap().keys, 3);
        // Replacing scope one drops a; b stays through scope two.
        s.register(owner, one, &[], T0).unwrap();
        assert!(!s.allows(owner, &a).unwrap());
        assert!(s.allows(owner, &b).unwrap());
        assert!(s.allows(owner, &c).unwrap());
        s.register(owner, two, &[c], T0).unwrap();
        assert!(!s.allows(owner, &b).unwrap());
        assert_eq!(put(&s, b, owner, vec![1], T0), Err(MailboxError::NotAllowed));
        // Limits.
        let many: Vec<PublicKey> = (0..=MAX_KEYS_PER_SCOPE).map(|_| id()).collect();
        assert!(matches!(s.register(owner, one, &many, T0), Err(MailboxError::TooManyKeys { .. })));
        let other = id();
        for i in 0..MAX_SCOPES {
            s.register(other, [i as u8; 16], &[a], T0).unwrap();
        }
        assert!(matches!(s.register(other, [0xee; 16], &[a], T0), Err(MailboxError::TooManyKeys { .. })));
    }

    #[test]
    fn replayed_nonce_is_refused() {
        let (s, _t) = store("replay", Limits::default());
        let k = id();
        let gm = mailbox(&s, &[k]);
        let n = nonce();
        s.put(id(), gm, k, n, vec![1], T0).unwrap();
        assert_eq!(s.put(id(), gm, k, n, vec![1], T0), Err(MailboxError::Replayed));
    }

    #[test]
    fn per_key_cap() {
        let limits = Limits { max_messages_per_key: 3, ..Limits::default() };
        let (s, _t) = store("perkey", limits);
        let (leaked, fine) = (id(), id());
        let gm = mailbox(&s, &[leaked, fine]);
        for _ in 0..3 {
            put(&s, leaked, gm, vec![0], T0).unwrap();
        }
        assert_eq!(put(&s, leaked, gm, vec![0], T0), Err(MailboxError::KeyFull { max: 3 }));
        // Other keys still get through; the cap is per mailbox.
        put(&s, fine, gm, vec![0], T0).unwrap();
        let other = mailbox(&s, &[leaked]);
        put(&s, leaked, other, vec![0], T0).unwrap();
        // Collecting frees room.
        let (items, _) = s.fetch(gm, 1, T0).unwrap();
        s.ack(gm, &[items[0].id]).unwrap();
        put(&s, leaked, gm, vec![0], T0).unwrap();
    }

    #[test]
    fn blob_size_limit() {
        let (s, _t) = store("size", Limits::default());
        let k = id();
        let r = mailbox(&s, &[k]);
        let err = put(&s, k, r, vec![0; 256 * 1024 + 1], T0).unwrap_err();
        assert!(matches!(err, MailboxError::TooLarge { max: 262144, .. }));
        put(&s, k, r, vec![0; 256 * 1024], T0).unwrap();
    }

    #[test]
    fn recipient_limit() {
        let limits = Limits {
            max_messages_per_recipient: 3,
            ..Limits::default()
        };
        let (s, _t) = store("rcpt", limits);
        let k = id();
        let b = mailbox(&s, &[k]);
        for _ in 0..3 {
            put(&s, k, b, vec![0], T0).unwrap();
        }
        assert_eq!(put(&s, k, b, vec![0], T0), Err(MailboxError::RecipientFull { max: 3 }));
        // Others are unaffected.
        let c = mailbox(&s, &[k]);
        put(&s, k, c, vec![0], T0).unwrap();
        // Collecting mail frees room.
        let (items, _) = s.fetch(b, 1, T0).unwrap();
        s.ack(b, &[items[0].id]).unwrap();
        put(&s, k, b, vec![0], T0).unwrap();
    }

    #[test]
    fn sender_daily_limits() {
        let limits = Limits {
            max_messages_per_sender_per_day: 2,
            max_bytes_per_sender_per_day: 100,
            ..Limits::default()
        };
        let (s, _t) = store("sender", limits);
        let k = id();
        let r = mailbox(&s, &[k]);
        let a = id();
        let day_start = (T0 / DAY + 1) * DAY;
        s.put(a, r, k, nonce(), vec![0; 10], day_start).unwrap();
        s.put(a, r, k, nonce(), vec![0; 10], day_start + 10).unwrap();
        assert!(matches!(s.put(a, r, k, nonce(), vec![0; 10], day_start + 20), Err(MailboxError::SenderQuota { .. })));
        // Another sender is fine.
        s.put(id(), r, k, nonce(), vec![0; 10], day_start + 20).unwrap();
        // The next day the counter starts again; bytes are limited too.
        let next = day_start + DAY;
        assert!(matches!(s.put(a, r, k, nonce(), vec![0; 101], next), Err(MailboxError::SenderQuota { .. })));
        s.put(a, r, k, nonce(), vec![0; 100], next).unwrap();
        assert!(matches!(s.put(a, r, k, nonce(), vec![0; 1], next), Err(MailboxError::SenderQuota { .. })));
    }

    #[test]
    fn expiry_with_fake_clock() {
        let limits = Limits {
            expiry_secs: 100,
            max_messages_per_recipient: 1,
            ..Limits::default()
        };
        let (s, _t) = store("expiry", limits);
        let k = id();
        let b = mailbox(&s, &[k]);
        put(&s, k, b, vec![1], T0).unwrap();
        assert_eq!(s.fetch(b, 10, T0 + 99).unwrap().0.len(), 1);
        // Expired: not delivered, and no longer counts against the limit.
        assert!(s.fetch(b, 10, T0 + 100).unwrap().0.is_empty());
        put(&s, k, b, vec![2], T0 + 100).unwrap();
        assert_eq!(s.len().unwrap(), 2);
        assert_eq!(s.purge(T0 + 150).unwrap(), 1);
        assert_eq!(s.len().unwrap(), 1);
        assert_eq!(s.fetch(b, 10, T0 + 150).unwrap().0[0].blob, [2]);
        assert_eq!(s.purge(T0 + 200).unwrap(), 1);
        assert!(s.is_empty().unwrap());
    }

    #[test]
    fn survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "chummer-relay-store-{}-reopen.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _t = Tmp(path.clone());
        let k = id();
        let (b, first) = {
            let s = Store::open(&path, Limits::default()).unwrap();
            let b = mailbox(&s, &[k]);
            (b, put(&s, k, b, vec![9], T0).unwrap())
        };
        // Mail and registrations are both still there.
        let s = Store::open(&path, Limits::default()).unwrap();
        assert_eq!(s.fetch(b, 10, T0).unwrap().0[0].id, first);
        assert!(s.allows(b, &k).unwrap());
        assert!(put(&s, k, b, vec![9], T0).unwrap() > first);
    }

    #[test]
    fn older_layout_is_emptied() {
        let path = std::env::temp_dir().join(format!("chummer-relay-store-{}-oldlayout.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _t = Tmp(path.clone());
        {
            // The first protocol's tables: inbox values were plain times.
            let db = Database::create(&path).unwrap();
            let txn = db.begin_write().unwrap();
            txn.open_table(TableDefinition::<([u8; 32], u64), u64>::new("inbox")).unwrap().insert(([1; 32], 1), 5).unwrap();
            txn.open_table(TableDefinition::<&str, u64>::new("meta")).unwrap().insert("next_id", 2).unwrap();
            txn.commit().unwrap();
        }
        let s = Store::open(&path, Limits::default()).unwrap();
        assert!(s.is_empty().unwrap());
        let k = id();
        let b = mailbox(&s, &[k]);
        put(&s, k, b, vec![1], T0).unwrap();
    }
}
