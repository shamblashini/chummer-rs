//! A player's side: local copies of their characters and an outbox.
//!
//! For each character a [`Replica`] keeps the last state the authority
//! confirmed (with its version and hash) and the commands made since then
//! that the authority has not answered yet (the outbox). What the player
//! sees ([`Replica::character`]) is the confirmed state with the outbox
//! applied on top, so editing works offline.
//!
//! When an [`Ack`] or [`Push`] arrives, the confirmed state moves forward
//! by the authority's log (or is replaced by a snapshot), the answered
//! commands leave the outbox, and the rest are applied again on top
//! ("rebuild"). If the confirmed state's hash then differs from the
//! authority's, the copy drifted: [`Event::NeedResync`] asks for a
//! snapshot, which replaces it.
//!
//! The outbox does not merge edits: the GUI's [`chummer_core::command::Session`]
//! merges bursts of typing into one undo step, but every envelope it
//! applies must be handed to [`Replica::edit_envelope`] as it happens.
//!
//! Persistence ([`Replica::save`]): the confirmed state as a snapshot, its
//! version and hash, the outbox, refused commands not yet dismissed, the
//! feed and the partial mail, so offline work survives a restart.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, Rejected, Report};
use chummer_core::dice::Rng;
use chummer_core::engine::Engine;
use chummer_net::EndpointId;
use serde::{Deserialize, Serialize};

use crate::feed;
use crate::mail::Inbox;
use crate::msg::{Ack, CharacterId, ClientMessage, Entry, FeedEntry, Hash, Have, Membership, Op, OpId, Push, PushBody, ResyncRequest, ServerMessage, SubmitBatch};
use crate::persist::{self, PersistError};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A command in the outbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub op: Op,
    /// What it did locally.
    pub description: String,
    /// Already sent through the mailbox (it is not mailed again; a live
    /// connection still resubmits it, which is harmless).
    pub mailed: bool,
}

/// A command of ours the authority refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub character: CharacterId,
    pub op: OpId,
    /// What it did locally ("Raised Pistols to 5 (10 karma)").
    pub description: String,
    pub reason: String,
    pub confirm: bool,
}

/// What handling a message did, for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The character changed (new confirmed state, or rebuilt outbox).
    Updated(CharacterId),
    /// The authority refused one of our commands; it was taken out.
    Refused(Refused),
    /// The copy drifted; send this request (live or by mail).
    NeedResync(ResyncRequest),
    /// The character list or members changed.
    Membership,
    /// We no longer see this character (given to someone else, removed).
    Removed(CharacterId),
    /// A message for this member that could not be handled.
    Error(String),
}

struct Copy {
    name: String,
    confirmed: Character,
    version: u64,
    hash: Hash,
    outbox: Vec<Pending>,
    /// `confirmed` with the outbox applied.
    current: Character,
    needs_resync: bool,
    /// `confirmed` compressed, made on the first save after it changed.
    snapshot: std::sync::OnceLock<Vec<u8>>,
    /// Pushes that start after our version: they overtook the answer that
    /// fills the gap (an ack and a push travel on different streams). Kept
    /// until the gap is filled; still there after an ack, they mean a
    /// resync.
    early: Vec<Push>,
}

/// The player's local copies. See the module documentation.
pub struct Replica {
    origin: [u8; 16],
    next_seq: u64,
    seeds: Rng,
    clock: fn() -> i64,
    copies: BTreeMap<CharacterId, Copy>,
    membership: Option<Membership>,
    refused: Vec<Refused>,
    feed: VecDeque<FeedEntry>,
    /// The last log entry added to the feed per character, to merge bursts
    /// of edits into one line as the authority does.
    feed_last: BTreeMap<CharacterId, Entry>,
    pub inbox: Inbox,
}

impl std::fmt::Debug for Replica {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Replica").field("characters", &self.copies.keys().collect::<Vec<_>>()).finish()
    }
}

impl Default for Replica {
    fn default() -> Self {
        Replica::new()
    }
}

impl Replica {
    pub fn new() -> Replica {
        Replica {
            origin: chummer_net::invite::random_id(),
            next_seq: 0,
            seeds: Rng::from_time(),
            clock: now_ms,
            copies: BTreeMap::new(),
            membership: None,
            refused: Vec::new(),
            feed: VecDeque::new(),
            feed_last: BTreeMap::new(),
            inbox: Inbox::default(),
        }
    }

    /// Use another clock (Unix milliseconds).
    pub fn with_clock(mut self, clock: fn() -> i64) -> Replica {
        self.clock = clock;
        self
    }

    // ----- reading -----

    pub fn characters(&self) -> impl Iterator<Item = &CharacterId> {
        self.copies.keys()
    }

    /// What the player sees: the confirmed state with the outbox applied.
    pub fn character(&self, id: &CharacterId) -> Option<&Character> {
        self.copies.get(id).map(|c| &c.current)
    }

    /// The last state the authority confirmed.
    pub fn confirmed(&self, id: &CharacterId) -> Option<&Character> {
        self.copies.get(id).map(|c| &c.confirmed)
    }

    pub fn name(&self, id: &CharacterId) -> Option<&str> {
        self.copies.get(id).map(|c| c.name.as_str())
    }

    /// The confirmed version.
    pub fn version(&self, id: &CharacterId) -> Option<u64> {
        self.copies.get(id).map(|c| c.version)
    }

    pub fn confirmed_hash(&self, id: &CharacterId) -> Option<Hash> {
        self.copies.get(id).map(|c| c.hash)
    }

    pub fn outbox(&self, id: &CharacterId) -> &[Pending] {
        self.copies.get(id).map(|c| c.outbox.as_slice()).unwrap_or_default()
    }

    /// Commands not yet confirmed, over all characters.
    pub fn outbox_len(&self) -> usize {
        self.copies.values().map(|c| c.outbox.len()).sum()
    }

    pub fn needs_resync(&self, id: &CharacterId) -> bool {
        self.copies.get(id).is_some_and(|c| c.needs_resync)
    }

    pub fn membership(&self) -> Option<&Membership> {
        self.membership.as_ref()
    }

    /// Refused commands the player has not dismissed.
    pub fn refused(&self) -> &[Refused] {
        &self.refused
    }

    pub fn dismiss_refused(&mut self) -> Vec<Refused> {
        std::mem::take(&mut self.refused)
    }

    /// The activity feed for our characters, oldest first.
    pub fn feed(&self) -> &VecDeque<FeedEntry> {
        &self.feed
    }

    // ----- editing -----

    /// Makes an envelope for `cmd` (fresh seed, now) and applies it.
    pub fn edit(&mut self, engine: &Engine, id: &CharacterId, cmd: Command) -> Result<Report, Rejected> {
        let env = Envelope::new(cmd, self.seeds.next_u64(), (self.clock)(), "");
        self.edit_envelope(engine, id, env)
    }

    /// Applies an envelope to the local copy and queues it. A command the
    /// engine refuses here is not queued.
    pub fn edit_envelope(&mut self, engine: &Engine, id: &CharacterId, env: Envelope) -> Result<Report, Rejected> {
        let c = self.copies.get_mut(id).ok_or_else(|| Rejected::new(format!("no character {id} here")))?;
        let applied = command::apply(&mut c.current, engine, &env)?;
        let report = Report { description: applied.description.clone(), message: applied.message, count: applied.count, changed: applied.changed };
        if applied.changed {
            self.next_seq += 1;
            let op = Op { id: OpId { origin: self.origin, seq: self.next_seq }, env };
            c.outbox.push(Pending { op, description: applied.description, mailed: false });
        }
        Ok(report)
    }

    // ----- what to send -----

    /// The first message on a live connection.
    pub fn join_message(&self, name: &str) -> ClientMessage {
        let have = self.copies.iter().map(|(id, c)| Have { character: id.clone(), version: c.version, hash: c.hash }).collect();
        ClientMessage::Join { name: name.to_owned(), have }
    }

    /// The whole outbox of `id` as one batch (live connections).
    pub fn batch(&self, id: &CharacterId) -> Option<SubmitBatch> {
        let c = self.copies.get(id)?;
        (!c.outbox.is_empty()).then(|| SubmitBatch { character: id.clone(), base_version: c.version, base_hash: c.hash, ops: c.outbox.iter().map(|p| p.op.clone()).collect() })
    }

    /// Batches for every character with a non-empty outbox.
    pub fn batches(&self) -> Vec<SubmitBatch> {
        self.copies.keys().filter_map(|id| self.batch(id)).collect()
    }

    /// The commands not mailed yet, as batches (split for the mailbox
    /// later). Call [`Replica::mark_mailed`] once they are stored.
    pub fn unmailed(&self) -> Vec<SubmitBatch> {
        self.copies
            .iter()
            .filter_map(|(id, c)| {
                let ops: Vec<Op> = c.outbox.iter().filter(|p| !p.mailed).map(|p| p.op.clone()).collect();
                (!ops.is_empty()).then(|| SubmitBatch { character: id.clone(), base_version: c.version, base_hash: c.hash, ops })
            })
            .collect()
    }

    pub fn mark_mailed(&mut self, batch: &SubmitBatch) {
        if let Some(c) = self.copies.get_mut(&batch.character) {
            for p in &mut c.outbox {
                if batch.ops.iter().any(|o| o.id == p.op.id) {
                    p.mailed = true;
                }
            }
        }
    }

    /// Marks commands to be mailed again (their mail was lost).
    pub fn mark_unmailed(&mut self, ops: &[OpId]) {
        for c in self.copies.values_mut() {
            for p in &mut c.outbox {
                if ops.contains(&p.op.id) {
                    p.mailed = false;
                }
            }
        }
    }

    /// Resync requests for drifted copies.
    pub fn resync_requests(&self) -> Vec<ResyncRequest> {
        self.copies.iter().filter(|(_, c)| c.needs_resync || !c.early.is_empty()).map(|(id, c)| ResyncRequest { character: id.clone(), have_version: c.version }).collect()
    }

    // ----- what arrives -----

    /// Handles any message from the authority.
    pub fn handle(&mut self, engine: &Engine, msg: ServerMessage) -> Vec<Event> {
        match msg {
            ServerMessage::Joined { membership, pushes } => {
                let mut ev = self.set_membership(membership);
                for p in pushes {
                    ev.extend(self.apply_push(engine, p));
                }
                ev
            }
            ServerMessage::Ack(a) => self.apply_ack(engine, a),
            ServerMessage::Push(p) => self.apply_push(engine, p),
            ServerMessage::Membership(m) => self.set_membership(m),
            ServerMessage::Error(e) => vec![Event::Error(e)],
        }
    }

    /// Takes a new membership: characters we no longer see are dropped.
    pub fn set_membership(&mut self, m: Membership) -> Vec<Event> {
        let mut ev = vec![Event::Membership];
        let gone: Vec<CharacterId> = self.copies.keys().filter(|id| !m.characters.iter().any(|c| c.id == **id)).cloned().collect();
        for id in gone {
            self.copies.remove(&id);
            ev.push(Event::Removed(id));
        }
        for info in &m.characters {
            if let Some(c) = self.copies.get_mut(&info.id) {
                c.name = info.name.clone();
            }
        }
        self.membership = Some(m);
        ev
    }

    fn members(&self) -> &[crate::msg::MemberInfo] {
        self.membership.as_ref().map(|m| m.members.as_slice()).unwrap_or_default()
    }

    /// Moves the confirmed state forward by `push`.
    pub fn apply_push(&mut self, engine: &Engine, push: Push) -> Vec<Event> {
        let mut ev = Vec::new();
        let id = push.character.clone();
        let members = self.members().to_vec();
        if let (PushBody::Entries(_), Some(c)) = (&push.body, self.copies.get_mut(&id)) {
            if !c.needs_resync && push.from_version > c.version {
                c.early.push(push);
                return ev;
            }
        }
        let new_entries: Vec<_> = push.entries().iter().filter(|e| self.copies.get(&id).is_none_or(|c| e.version > c.version)).cloned().collect();
        match push.body {
            PushBody::Snapshot { bytes, .. } => {
                let accept = self.copies.get(&id).is_none_or(|c| c.needs_resync || push.version >= c.version);
                if !accept {
                    return ev;
                }
                let ch = match command::restore(&bytes) {
                    Ok(ch) => ch,
                    Err(e) => return vec![Event::Error(format!("could not read the snapshot of {id}: {e}"))],
                };
                let (outbox, early) = self.copies.remove(&id).map(|c| (c.outbox, c.early)).unwrap_or_default();
                let hash = command::state_hash(&ch);
                let needs_resync = hash != push.hash;
                if needs_resync {
                    ev.push(Event::Error(format!("the snapshot of {id} does not match its hash")));
                }
                self.copies.insert(id.clone(), Copy { name: push.name.clone(), current: ch.clone(), confirmed: ch, version: push.version, hash, outbox, needs_resync, snapshot: std::sync::OnceLock::from(bytes), early });
            }
            PushBody::Entries(entries) => {
                let Some(c) = self.copies.get_mut(&id) else {
                    // A character we have no copy of: ask for all of it.
                    return vec![Event::NeedResync(ResyncRequest { character: id, have_version: 0 })];
                };
                if c.needs_resync {
                    // Still waiting for a snapshot: the request or its
                    // answer may have been lost (a dropped connection), so
                    // ask again; else a live session never recovers.
                    return vec![Event::NeedResync(ResyncRequest { character: id, have_version: c.version })];
                }
                if push.version <= c.version {
                    // Old news; check we agree on where we are.
                    if push.version == c.version && push.hash != c.hash {
                        c.needs_resync = true;
                        return vec![Event::NeedResync(ResyncRequest { character: id, have_version: c.version })];
                    }
                    return ev;
                }
                let have = c.version;
                for e in entries.iter().filter(|e| e.version > have) {
                    if e.version != c.version + 1 || command::apply(&mut c.confirmed, engine, &e.env).is_err() {
                        c.needs_resync = true;
                        break;
                    }
                    c.version = e.version;
                    c.snapshot = std::sync::OnceLock::new();
                }
                c.name = push.name.clone();
                if !c.needs_resync {
                    c.hash = command::state_hash(&c.confirmed);
                    c.needs_resync = c.version != push.version || c.hash != push.hash;
                }
                if c.needs_resync {
                    ev.push(Event::NeedResync(ResyncRequest { character: id.clone(), have_version: c.version }));
                }
            }
        }
        let name = push.name.clone();
        for e in &new_entries {
            let line = feed::from_entry(&members, &id, &name, e);
            match self.feed_last.get(&id) {
                Some(prev) if feed::coalesces(prev, e) => feed::merge(&mut self.feed, line, prev.version),
                _ => feed::push(&mut self.feed, line),
            }
            self.feed_last.insert(id.clone(), e.clone());
        }
        let c = self.copies.get_mut(&id).expect("present");
        // Commands of ours the authority logged are confirmed.
        c.outbox.retain(|p| !new_entries.iter().any(|e| e.op == p.op.id));
        if c.needs_resync {
            // Keep what the user sees until the snapshot comes.
            return ev;
        }
        rebuild(engine, c);
        ev.push(Event::Updated(id.clone()));
        ev.extend(self.drain_early(engine, &id));
        ev
    }

    /// Applies stashed pushes whose start we have reached.
    fn drain_early(&mut self, engine: &Engine, id: &CharacterId) -> Vec<Event> {
        let Some(c) = self.copies.get_mut(id) else { return Vec::new() };
        let (ready, wait): (Vec<Push>, Vec<Push>) = std::mem::take(&mut c.early).into_iter().partition(|p| p.from_version <= c.version);
        c.early = wait;
        ready.into_iter().flat_map(|p| self.apply_push(engine, p)).collect::<Vec<_>>()
    }

    /// Handles the answer to one of our batches.
    pub fn apply_ack(&mut self, engine: &Engine, ack: Ack) -> Vec<Event> {
        let id = ack.character.clone();
        let mut ev = Vec::new();
        let before = self.copies.get(&id).map(|c| c.outbox.len());
        if let Some(c) = self.copies.get_mut(&id) {
            for r in &ack.rejected {
                if let Some(pos) = c.outbox.iter().position(|p| p.op.id == r.op) {
                    let p = c.outbox.remove(pos);
                    let refused = Refused { character: id.clone(), op: r.op, description: p.description, reason: r.reason.clone(), confirm: r.confirm };
                    self.refused.push(refused.clone());
                    ev.push(Event::Refused(refused));
                }
            }
            // Accepted commands that changed nothing have no log entry.
            c.outbox.retain(|p| !ack.accepted.iter().any(|a| a.op == p.op.id && !a.changed));
        }
        ev.extend(self.apply_push(engine, ack.update));
        ev.extend(self.drain_early(engine, &id));
        if let Some(c) = self.copies.get_mut(&id) {
            // Accepted ones are in the log the update carried; any left
            // were applied before our confirmed version.
            c.outbox.retain(|p| !ack.accepted.iter().any(|a| a.op == p.op.id && a.version <= c.version));
            if Some(c.outbox.len()) != before && !c.needs_resync {
                rebuild(engine, c);
                if !ev.contains(&Event::Updated(id.clone())) {
                    ev.push(Event::Updated(id.clone()));
                }
            }
            // The ack brought us to the authority's state of a moment ago;
            // a push still starting after that missed something.
            if !c.early.is_empty() && !c.needs_resync {
                c.early.clear();
                c.needs_resync = true;
                ev.push(Event::NeedResync(ResyncRequest { character: id, have_version: c.version }));
            }
        }
        ev
    }

    // ----- persistence -----

    pub fn to_bytes(&self) -> Vec<u8> {
        let file = ReplicaFile {
            origin: self.origin,
            next_seq: self.next_seq,
            characters: self
                .copies
                .iter()
                .map(|(id, c)| StoredCopy { id: id.clone(), name: c.name.clone(), snapshot: c.snapshot.get_or_init(|| command::snapshot(&c.confirmed)).clone(), version: c.version, hash: c.hash, outbox: c.outbox.clone(), needs_resync: c.needs_resync })
                .collect(),
            membership: self.membership.clone(),
            refused: self.refused.clone(),
            feed: self.feed.iter().cloned().collect(),
            inbox: self.inbox.clone(),
        };
        persist::to_bytes(MAGIC, FORMAT, &file)
    }

    /// Loads a saved replica; the outbox is applied again on the confirmed
    /// states, which needs the engine.
    pub fn from_bytes(engine: &Engine, bytes: &[u8]) -> Result<Replica, PersistError> {
        let f: ReplicaFile = persist::from_bytes(MAGIC, "campaign replica", FORMAT, bytes)?;
        let mut copies = BTreeMap::new();
        for s in f.characters {
            let ch = command::restore(&s.snapshot)?;
            let mut c = Copy { name: s.name, current: ch.clone(), confirmed: ch, version: s.version, hash: s.hash, outbox: s.outbox, needs_resync: s.needs_resync, snapshot: std::sync::OnceLock::from(s.snapshot), early: Vec::new() };
            rebuild(engine, &mut c);
            copies.insert(s.id, c);
        }
        Ok(Replica {
            origin: f.origin,
            next_seq: f.next_seq,
            seeds: Rng::from_time(),
            clock: now_ms,
            copies,
            membership: f.membership,
            refused: f.refused,
            feed: f.feed.into(),
            feed_last: BTreeMap::new(),
            inbox: f.inbox,
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        persist::write_atomic(path, &self.to_bytes())
    }

    pub fn load(engine: &Engine, path: &Path) -> Result<Replica, PersistError> {
        Replica::from_bytes(engine, &std::fs::read(path)?)
    }

    /// The node id this replica's membership was issued to, once joined.
    pub fn me(&self) -> Option<EndpointId> {
        self.membership.as_ref().map(|m| m.you)
    }
}

/// `current` = `confirmed` with the outbox applied again. Commands that no
/// longer apply locally stay queued (the authority decides); they are just
/// not shown.
fn rebuild(engine: &Engine, c: &mut Copy) {
    let mut cur = c.confirmed.clone();
    for p in &c.outbox {
        let _ = command::apply(&mut cur, engine, &p.op.env);
    }
    c.current = cur;
}

const MAGIC: &[u8; 4] = b"CRSR";
const FORMAT: u16 = 1;

#[derive(Serialize, Deserialize)]
struct ReplicaFile {
    origin: [u8; 16],
    next_seq: u64,
    characters: Vec<StoredCopy>,
    membership: Option<Membership>,
    refused: Vec<Refused>,
    feed: Vec<FeedEntry>,
    inbox: Inbox,
}

#[derive(Serialize, Deserialize)]
struct StoredCopy {
    id: CharacterId,
    name: String,
    snapshot: Vec<u8>,
    version: u64,
    hash: Hash,
    outbox: Vec<Pending>,
    needs_resync: bool,
}
