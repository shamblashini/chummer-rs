//! The mailbox store, on disk in a redb database.
//!
//! Tables:
//! - `messages`: id -> postcard [`Stored`] (recipient, sender, time, blob)
//! - `inbox`: (recipient, id) -> received time, for per-recipient listing
//! - `quota`: (sender, day) -> (messages, bytes), for daily sender limits
//! - `meta`: "next_id" -> next message id
//!
//! Every call takes `now` (Unix seconds) so limits and expiry can be tested
//! with a fake clock.

use std::path::Path;

use chummer_net::mailbox::{MailItem, MailboxError, MAX_FETCH_BYTES};
use iroh::EndpointId;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};

const MESSAGES: TableDefinition<u64, &[u8]> = TableDefinition::new("messages");
const INBOX: TableDefinition<([u8; 32], u64), u64> = TableDefinition::new("inbox");
const QUOTA: TableDefinition<([u8; 32], u64), (u64, u64)> = TableDefinition::new("quota");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

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
            expiry_secs: 30 * DAY,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    recipient: [u8; 32],
    sender: [u8; 32],
    received: u64,
    blob: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
#[error("mailbox database error: {0}")]
pub struct StoreError(String);

fn db_err(e: impl std::fmt::Display) -> MailboxError {
    MailboxError::Internal(e.to_string())
}

/// The mailbox database.
#[derive(Debug)]
pub struct Store {
    db: Database,
    limits: Limits,
}

impl Store {
    /// Opens (or creates) the database at `path`.
    pub fn open(path: &Path, limits: Limits) -> Result<Store, StoreError> {
        let db = Database::create(path).map_err(|e| StoreError(e.to_string()))?;
        let txn = db.begin_write().map_err(|e| StoreError(e.to_string()))?;
        {
            // Create the tables so read transactions never miss them.
            txn.open_table(MESSAGES)
                .map_err(|e| StoreError(e.to_string()))?;
            txn.open_table(INBOX)
                .map_err(|e| StoreError(e.to_string()))?;
            txn.open_table(QUOTA)
                .map_err(|e| StoreError(e.to_string()))?;
            txn.open_table(META)
                .map_err(|e| StoreError(e.to_string()))?;
        }
        txn.commit().map_err(|e| StoreError(e.to_string()))?;
        Ok(Store { db, limits })
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn expired(&self, received: u64, now: u64) -> bool {
        received.saturating_add(self.limits.expiry_secs) <= now
    }

    /// Stores `blob` from `sender` for `recipient`.
    pub fn put(
        &self,
        sender: EndpointId,
        recipient: EndpointId,
        blob: Vec<u8>,
        now: u64,
    ) -> Result<u64, MailboxError> {
        let l = &self.limits;
        let len = blob.len() as u64;
        if len > l.max_blob_bytes {
            return Err(MailboxError::TooLarge {
                len,
                max: l.max_blob_bytes,
            });
        }
        let rcpt = *recipient.as_bytes();
        let snd = *sender.as_bytes();
        let txn = self.db.begin_write().map_err(db_err)?;
        let id;
        {
            let mut inbox = txn.open_table(INBOX).map_err(db_err)?;
            let mut waiting = 0u64;
            for entry in inbox.range((rcpt, 0)..=(rcpt, u64::MAX)).map_err(db_err)? {
                let (_, received) = entry.map_err(db_err)?;
                if !self.expired(received.value(), now) {
                    waiting += 1;
                }
            }
            if waiting >= l.max_messages_per_recipient {
                return Err(MailboxError::RecipientFull {
                    max: l.max_messages_per_recipient,
                });
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
                received: now,
                blob,
            };
            let bytes = postcard::to_stdvec(&stored).map_err(db_err)?;
            let mut messages = txn.open_table(MESSAGES).map_err(db_err)?;
            messages.insert(id, bytes.as_slice()).map_err(db_err)?;
            inbox.insert((rcpt, id), now).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(id)
    }

    /// Up to `limit` of `recipient`'s oldest unexpired messages (at most
    /// [`MAX_FETCH_BYTES`] of blobs, but always at least one), and whether
    /// there are more.
    pub fn fetch(
        &self,
        recipient: EndpointId,
        limit: u32,
        now: u64,
    ) -> Result<(Vec<MailItem>, bool), MailboxError> {
        let rcpt = *recipient.as_bytes();
        let txn = self.db.begin_read().map_err(db_err)?;
        let inbox = txn.open_table(INBOX).map_err(db_err)?;
        let messages = txn.open_table(MESSAGES).map_err(db_err)?;
        let mut items = Vec::new();
        let mut total = 0usize;
        for entry in inbox.range((rcpt, 0)..=(rcpt, u64::MAX)).map_err(db_err)? {
            let (key, received) = entry.map_err(db_err)?;
            if self.expired(received.value(), now) {
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
                received: stored.received,
                blob: stored.blob,
            });
        }
        Ok((items, false))
    }

    /// Deletes `recipient`'s messages `ids`; ids of other recipients are
    /// ignored. Returns how many were deleted.
    pub fn ack(&self, recipient: EndpointId, ids: &[u64]) -> Result<u32, MailboxError> {
        let rcpt = *recipient.as_bytes();
        let txn = self.db.begin_write().map_err(db_err)?;
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
        let txn = self.db.begin_write().map_err(db_err)?;
        let mut removed = 0;
        {
            let mut inbox = txn.open_table(INBOX).map_err(db_err)?;
            let mut messages = txn.open_table(MESSAGES).map_err(db_err)?;
            let mut dead = Vec::new();
            for entry in inbox.iter().map_err(db_err)? {
                let (key, received) = entry.map_err(db_err)?;
                if self.expired(received.value(), now) {
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
        let txn = self.db.begin_read().map_err(db_err)?;
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

    #[test]
    fn put_fetch_ack() {
        let (s, _t) = store("basic", Limits::default());
        let (a, b, c) = (id(), id(), id());
        let m1 = s.put(a, b, vec![1], T0).unwrap();
        let m2 = s.put(c, b, vec![2], T0 + 1).unwrap();
        s.put(a, c, vec![3], T0).unwrap();
        let (items, more) = s.fetch(b, 10, T0 + 2).unwrap();
        assert!(!more);
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [m1, m2]);
        assert_eq!(items[0].sender, a);
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
    fn blob_size_limit() {
        let (s, _t) = store("size", Limits::default());
        let err = s.put(id(), id(), vec![0; 256 * 1024 + 1], T0).unwrap_err();
        assert!(matches!(err, MailboxError::TooLarge { max: 262144, .. }));
        s.put(id(), id(), vec![0; 256 * 1024], T0).unwrap();
    }

    #[test]
    fn recipient_limit() {
        let limits = Limits {
            max_messages_per_recipient: 3,
            ..Limits::default()
        };
        let (s, _t) = store("rcpt", limits);
        let b = id();
        for _ in 0..3 {
            s.put(id(), b, vec![0], T0).unwrap();
        }
        assert_eq!(
            s.put(id(), b, vec![0], T0),
            Err(MailboxError::RecipientFull { max: 3 })
        );
        // Others are unaffected.
        s.put(id(), id(), vec![0], T0).unwrap();
        // Collecting mail frees room.
        let (items, _) = s.fetch(b, 1, T0).unwrap();
        s.ack(b, &[items[0].id]).unwrap();
        s.put(id(), b, vec![0], T0).unwrap();
    }

    #[test]
    fn sender_daily_limits() {
        let limits = Limits {
            max_messages_per_sender_per_day: 2,
            max_bytes_per_sender_per_day: 100,
            ..Limits::default()
        };
        let (s, _t) = store("sender", limits);
        let a = id();
        let day_start = (T0 / DAY + 1) * DAY;
        s.put(a, id(), vec![0; 10], day_start).unwrap();
        s.put(a, id(), vec![0; 10], day_start + 10).unwrap();
        assert!(matches!(
            s.put(a, id(), vec![0; 10], day_start + 20),
            Err(MailboxError::SenderQuota { .. })
        ));
        // Another sender is fine.
        s.put(id(), id(), vec![0; 10], day_start + 20).unwrap();
        // The next day the counter starts again; bytes are limited too.
        let next = day_start + DAY;
        assert!(matches!(
            s.put(a, id(), vec![0; 101], next),
            Err(MailboxError::SenderQuota { .. })
        ));
        s.put(a, id(), vec![0; 100], next).unwrap();
        assert!(matches!(
            s.put(a, id(), vec![0; 1], next),
            Err(MailboxError::SenderQuota { .. })
        ));
    }

    #[test]
    fn expiry_with_fake_clock() {
        let limits = Limits {
            expiry_secs: 100,
            max_messages_per_recipient: 1,
            ..Limits::default()
        };
        let (s, _t) = store("expiry", limits);
        let (a, b) = (id(), id());
        s.put(a, b, vec![1], T0).unwrap();
        assert_eq!(s.fetch(b, 10, T0 + 99).unwrap().0.len(), 1);
        // Expired: not delivered, and no longer counts against the limit.
        assert!(s.fetch(b, 10, T0 + 100).unwrap().0.is_empty());
        s.put(a, b, vec![2], T0 + 100).unwrap();
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
        let b = id();
        let first = {
            let s = Store::open(&path, Limits::default()).unwrap();
            s.put(id(), b, vec![9], T0).unwrap()
        };
        let s = Store::open(&path, Limits::default()).unwrap();
        assert_eq!(s.fetch(b, 10, T0).unwrap().0[0].id, first);
        assert!(s.put(id(), b, vec![9], T0).unwrap() > first);
    }
}
