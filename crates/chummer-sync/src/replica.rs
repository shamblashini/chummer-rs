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
//! Dice rolls ([`Replica::roll`]) are not commands: they wait in a roll
//! outbox of their own until the authority names them in
//! [`ServerMessage::RollsTaken`], then join the table's roll log
//! ([`Replica::table_rolls`]) with the rolls of others the authority
//! sends ([`ServerMessage::Rolls`]).
//!
//! Persistence ([`Replica::save`]): the confirmed state as a snapshot, its
//! version and hash, the outbox, refused commands not yet dismissed, the
//! feed, the rolls and the partial mail, so offline work survives a
//! restart.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, Rejected, Report};
use chummer_core::dice::{RollRecord, Rng};
use chummer_core::engine::Engine;
use chummer_net::EndpointId;
use serde::{Deserialize, Serialize};

use crate::feed;
use crate::mail::Inbox;
use crate::msg::{Ack, CharacterId, ClientMessage, Entry, FeedEntry, Hash, Have, Membership, Op, OpId, Push, PushBody, ResyncRequest, RollId, RollReport, ServerMessage, SubmitBatch, TableRoll};

/// Rolls kept in the table's roll log.
pub const ROLL_LOG: usize = crate::authority::ROLL_LOG;
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

/// A roll of ours the authority has not answered yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRoll {
    pub report: RollReport,
    /// Already sent through the mailbox (as [`Pending::mailed`]).
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
    /// The GM's app refused to let us join (by mail).
    Denied(chummer_net::campaign::DenyReason),
    /// Rolls were answered or arrived ([`Replica::table_rolls`]).
    Rolls,
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
    /// Why the GM's app last refused us, until a join works.
    denied: Option<chummer_net::campaign::DenyReason>,
    /// Our roll ids count up from here.
    next_roll: u64,
    roll_outbox: Vec<PendingRoll>,
    /// Rolls at the table (ours once answered, and the others' we may
    /// see), in the order taken, oldest first.
    rolls: VecDeque<TableRoll>,
    /// Counts changes to the rolls (not saved), for a UI's cache.
    rolls_rev: u64,
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
            denied: None,
            next_roll: 0,
            roll_outbox: Vec::new(),
            rolls: VecDeque::new(),
            rolls_rev: 0,
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

    // ----- dice rolls -----

    /// Queues a roll made for `id` (our character) to be sent to the
    /// authority; returns it as reported.
    pub fn roll(&mut self, id: &CharacterId, roll: RollRecord) -> Result<RollReport, String> {
        if !self.copies.contains_key(id) {
            return Err(format!("no character {id} here"));
        }
        self.next_roll += 1;
        let report = RollReport { id: RollId { origin: self.origin, seq: self.next_roll }, character: Some(id.clone()), roll };
        self.roll_outbox.push(PendingRoll { report: report.clone(), mailed: false });
        self.rolls_rev += 1;
        Ok(report)
    }

    /// Our rolls not answered yet, oldest first.
    pub fn roll_outbox(&self) -> &[PendingRoll] {
        &self.roll_outbox
    }

    /// The table's rolls we have (ours once the authority took them, and
    /// the others' it sent), in the order taken, oldest first. Ours still
    /// waiting are in [`Replica::roll_outbox`].
    pub fn table_rolls(&self) -> &VecDeque<TableRoll> {
        &self.rolls
    }

    /// Changes whenever the rolls do.
    pub fn rolls_rev(&self) -> u64 {
        self.rolls_rev
    }

    /// The last roll sequence number made here.
    pub fn last_roll_seq(&self) -> u64 {
        self.next_roll
    }

    /// Every roll not answered yet, as one message (live connections).
    pub fn rolls_message(&self) -> Option<ClientMessage> {
        (!self.roll_outbox.is_empty()).then(|| ClientMessage::Rolls(self.roll_outbox.iter().map(|p| p.report.clone()).collect()))
    }

    /// The rolls not mailed yet. Call [`Replica::mark_rolls_mailed`] once
    /// they are stored.
    pub fn unmailed_rolls(&self) -> Vec<RollReport> {
        self.roll_outbox.iter().filter(|p| !p.mailed).map(|p| p.report.clone()).collect()
    }

    pub fn mark_rolls_mailed(&mut self, ids: &[RollId]) {
        for p in &mut self.roll_outbox {
            if ids.contains(&p.report.id) {
                p.mailed = true;
            }
        }
    }

    /// Marks rolls to be mailed again (their mail was lost).
    pub fn mark_rolls_unmailed(&mut self, ids: &[RollId]) {
        for p in &mut self.roll_outbox {
            if ids.contains(&p.report.id) {
                p.mailed = false;
            }
        }
    }

    /// Puts back rolls made after this replica was saved (from the
    /// session's roll journal, after a crash). Returns how many.
    pub fn recover_rolls(&mut self, mut made: Vec<RollReport>) -> usize {
        made.sort_by_key(|r| r.id.seq);
        let mut n = 0;
        for r in made {
            if r.id.origin != self.origin || r.id.seq <= self.next_roll {
                continue;
            }
            self.next_roll = r.id.seq;
            self.roll_outbox.push(PendingRoll { report: r, mailed: false });
            n += 1;
        }
        self.rolls_rev += 1;
        n
    }

    fn add_table_roll(&mut self, r: TableRoll) {
        if self.rolls.iter().any(|x| x.id == r.id) {
            return;
        }
        self.rolls.push_back(r);
        while self.rolls.len() > ROLL_LOG {
            self.rolls.pop_front();
        }
    }

    /// The authority has these rolls of ours: they join the table's log.
    fn rolls_taken(&mut self, ids: &[RollId]) -> Vec<Event> {
        let (taken, rest): (Vec<PendingRoll>, Vec<PendingRoll>) = std::mem::take(&mut self.roll_outbox).into_iter().partition(|p| ids.contains(&p.report.id));
        self.roll_outbox = rest;
        if taken.is_empty() {
            return Vec::new();
        }
        let m = self.membership.as_ref();
        let me = m.map(|m| m.you);
        let role = m.map_or(chummer_net::invite::Role::Player, |m| m.role);
        let my_name = m.and_then(|m| m.members.iter().find(|x| x.id == m.you)).map(|x| x.name.clone()).unwrap_or_default();
        for p in taken {
            let Some(author) = me else { continue };
            let who = p.report.character.as_ref().and_then(|c| self.name(c)).unwrap_or(&my_name).to_owned();
            let r = p.report;
            self.add_table_roll(TableRoll { seq: 0, id: r.id, author, author_name: my_name.clone(), author_role: role, character: r.character, who, open: false, roll: r.roll });
        }
        self.rolls_rev += 1;
        vec![Event::Rolls]
    }

    /// The sequence number of the last command made here (commands made
    /// later have higher ones).
    pub fn last_seq(&self) -> u64 {
        self.next_seq
    }

    /// Puts back commands made after this replica was saved (from the
    /// session's outbox journal, after a crash): those of this replica
    /// newer than [`Replica::last_seq`], in order. Returns how many.
    pub fn recover_outbox(&mut self, engine: &Engine, mut made: Vec<(CharacterId, Pending)>) -> usize {
        made.sort_by_key(|(_, p)| p.op.id.seq);
        let mut n = 0;
        for (id, p) in made {
            if p.op.id.origin != self.origin || p.op.id.seq <= self.next_seq {
                continue;
            }
            let Some(c) = self.copies.get_mut(&id) else { continue };
            self.next_seq = p.op.id.seq;
            c.outbox.push(p);
            rebuild(engine, c);
            n += 1;
        }
        n
    }

    // ----- what to send -----

    /// The first message on a live connection.
    pub fn join_message(&self, name: &str, claim: Option<crate::msg::ClaimProof>) -> ClientMessage {
        let have = self.copies.iter().map(|(id, c)| Have { character: id.clone(), version: c.version, hash: c.hash }).collect();
        ClientMessage::Join { name: name.to_owned(), have, claim }
    }

    /// Why the GM's app refused us (cleared when a join works).
    pub fn denied(&self) -> Option<&chummer_net::campaign::DenyReason> {
        self.denied.as_ref()
    }

    pub fn set_denied(&mut self, d: Option<chummer_net::campaign::DenyReason>) {
        self.denied = d;
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
                self.denied = None;
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
            ServerMessage::Denied(d) => {
                self.denied = Some(d.clone());
                vec![Event::Denied(d)]
            }
            ServerMessage::RollsTaken(ids) => self.rolls_taken(&ids),
            ServerMessage::Rolls(rolls) => {
                for r in rolls {
                    self.add_table_roll(r);
                }
                self.rolls_rev += 1;
                vec![Event::Rolls]
            }
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

    /// The snapshots [`Replica::to_bytes`] would have to make: (character,
    /// version and hash of the confirmed state, a copy of it). The session
    /// makes them without holding its lock (compressing a character takes
    /// up to seconds) and hands them back with [`Replica::put_snapshots`].
    pub fn snapshot_work(&self) -> Vec<(CharacterId, (u64, Hash), Character)> {
        self.copies.iter().filter(|(_, c)| c.snapshot.get().is_none()).map(|(id, c)| (id.clone(), (c.version, c.hash), c.confirmed.clone())).collect()
    }

    /// Takes snapshots made by [`Replica::snapshot_work`] (those of a
    /// confirmed state that changed meanwhile are dropped).
    pub fn put_snapshots(&mut self, made: Vec<(CharacterId, (u64, Hash), Vec<u8>)>) {
        for (id, at, bytes) in made {
            if let Some(c) = self.copies.get_mut(&id) {
                if (c.version, c.hash) == at {
                    let _ = c.snapshot.set(bytes);
                }
            }
        }
    }

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
            denied: self.denied.clone(),
            next_roll: self.next_roll,
            roll_outbox: self.roll_outbox.clone(),
            rolls: self.rolls.iter().cloned().collect(),
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
            denied: f.denied,
            next_roll: f.next_roll,
            roll_outbox: f.roll_outbox,
            rolls: f.rolls.into(),
            rolls_rev: 0,
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
/// 2: member keys (the membership's label and GM keys, refusals).
/// 3: dice rolls (the roll outbox and the table's rolls).
const FORMAT: u16 = 3;

#[derive(Serialize, Deserialize)]
struct ReplicaFile {
    origin: [u8; 16],
    next_seq: u64,
    characters: Vec<StoredCopy>,
    membership: Option<Membership>,
    refused: Vec<Refused>,
    feed: Vec<FeedEntry>,
    inbox: Inbox,
    denied: Option<chummer_net::campaign::DenyReason>,
    next_roll: u64,
    roll_outbox: Vec<PendingRoll>,
    rolls: Vec<TableRoll>,
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
