//! Randomised sync: one authority, several replicas, and a network that
//! delays, reorders, duplicates and loses messages, while players edit
//! offline, the GM edits and reverts, and both sides save and restart.
//!
//! After every run the network is healed and everyone reconnects; then all
//! copies must equal the authority's (version and hash), every outbox must
//! be empty, and every command must have run exactly once (karma is a sum
//! of the accepted amounts) or have been reported as refused.
//!
//! Every message goes through the wire encoding (`msg::encode`/`decode`).
//! Seeds are printed in failures; `CHUMMER_CHAOS_SEEDS=n` runs more of them
//! and `CHUMMER_CHAOS_SEED=s` runs one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command};
use chummer_core::dice::Rng;
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Role};
use chummer_net::{EndpointId, SecretKey};
use chummer_sync::msg::{self, ClientMessage, OpId, ServerMessage};
use chummer_sync::{Authority, CharacterId, Event, Replica};

fn engine() -> &'static Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data")))
}

fn munin() -> &'static Character {
    static C: OnceLock<Character> = OnceLock::new();
    C.get_or_init(|| {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let mut ch = Character::load(&p).unwrap();
        ch.karma = START_KARMA;
        ch
    })
}

const START_KARMA: i32 = 20;

fn expense(gain: bool, amount: i32) -> Command {
    Command::ManualExpense { karma: true, gain, expense: ManualExpense { amount: amount as f64, reason: format!("chaos {amount}"), ..Default::default() } }
}

/// The karma change of a command made here.
fn delta(cmd: &Command) -> i64 {
    match cmd {
        Command::ManualExpense { gain, expense, .. } => (if *gain { 1 } else { -1 }) * expense.amount as i64,
        _ => 0,
    }
}

fn wire<T: serde::Serialize + for<'de> serde::Deserialize<'de>>(m: &T) -> T {
    msg::decode(&msg::encode(m)).expect("round trip")
}

struct Player {
    id: EndpointId,
    replica: Replica,
    /// Messages on their way to this player.
    inbox: Vec<ServerMessage>,
    refused: BTreeSet<OpId>,
    /// The player's last save (saves run in the background, so a crash
    /// goes back to it) and the outbox journal written at every edit
    /// since (`PlayerSession`'s `<replica>.outbox`).
    saved: Option<Vec<u8>>,
    journal: Vec<(CharacterId, chummer_sync::replica::Pending)>,
}

struct World {
    rng: Rng,
    seed: u64,
    auth: Authority,
    players: Vec<Player>,
    chars: Vec<CharacterId>,
    /// Each op's karma change, by op id, from the moment it is made.
    made: BTreeMap<OpId, i64>,
    /// Ops the authority accepted with a change, and refused.
    accepted: BTreeSet<OpId>,
    rejected: BTreeSet<OpId>,
    /// Ops whose outcome the authority forgot by going back to a save.
    lost: BTreeSet<OpId>,
    /// Karma expected at the authority (when no op was lost or reverted).
    karma: BTreeMap<CharacterId, i64>,
    /// The authority's last save, for crash-restarts.
    auth_saved: Vec<u8>,
    reverts: bool,
    crashes: bool,
    log: Vec<String>,
}

impl World {
    fn new(seed: u64, players: usize, reverts: bool, crashes: bool) -> World {
        let gm = SecretKey::from_bytes(&[1; 32]).public();
        let mut auth = Authority::new(CampaignId([7; 16]), gm, "GM");
        let mut ps = Vec::new();
        let mut chars = Vec::new();
        let mut karma = BTreeMap::new();
        for i in 0..players {
            let id = SecretKey::from_bytes(&[i as u8 + 2; 32]).public();
            auth.add_member(id, Role::Player, format!("P{i}"));
            let c = CharacterId::new(format!("char-{i}"));
            auth.add_character(c.clone(), Some(id), munin().clone()).unwrap();
            karma.insert(c.clone(), START_KARMA as i64);
            chars.push(c);
            ps.push(Player { id, replica: Replica::new(), inbox: Vec::new(), refused: BTreeSet::new(), saved: None, journal: Vec::new() });
        }
        let auth_saved = auth.to_bytes();
        let mut w = World {
            rng: Rng::seeded(seed),
            seed,
            auth,
            players: ps,
            chars,
            made: BTreeMap::new(),
            accepted: BTreeSet::new(),
            rejected: BTreeSet::new(),
            lost: BTreeSet::new(),
            karma,
            auth_saved,
            reverts,
            crashes,
            log: Vec::new(),
        };
        for p in 0..players {
            w.join(p);
        }
        w
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.rng.next_u64() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.rng.next_u64() % 100 < percent
    }

    fn ctx(&self) -> String {
        if let Ok(path) = std::env::var("CHUMMER_CHAOS_LOG") {
            let _ = std::fs::write(path, self.log.join("\n"));
        }
        let tail: Vec<&String> = self.log.iter().rev().take(25).collect();
        format!("seed {} — last steps (newest first): {tail:#?}", self.seed)
    }

    fn note(&mut self, s: String) {
        self.log.push(s);
    }

    /// The authority's answer to a request, sent through the wire format.
    fn request(&mut self, p: usize, m: ClientMessage) -> ServerMessage {
        let peer = self.players[p].id;
        let m = wire(&m);
        let (reply, notify) = match m {
            ClientMessage::Join { name, have, .. } => match self.auth.join(peer, &name, &have) {
                Ok((membership, pushes)) => (ServerMessage::Joined { membership, pushes }, vec![]),
                Err(e) => (ServerMessage::Error(e), vec![]),
            },
            ClientMessage::Submit(batch) => match self.auth.submit(engine(), peer, batch) {
                Ok(s) => {
                    for a in &s.ack.accepted {
                        self.record_accepted(a.op, a.changed, &s.ack.character);
                    }
                    for r in &s.ack.rejected {
                        if !self.accepted.contains(&r.op) {
                            self.rejected.insert(r.op);
                        }
                    }
                    (ServerMessage::Ack(s.ack), s.notify)
                }
                Err(e) => (ServerMessage::Error(e), vec![]),
            },
            ClientMessage::Resync(r) => match self.auth.resync(peer, &r) {
                Ok(push) => (ServerMessage::Push(push), vec![]),
                Err(e) => (ServerMessage::Error(e), vec![]),
            },
            ClientMessage::Rolls(rolls) => match self.auth.submit_rolls(peer, rolls) {
                Ok((taken, notify)) => (ServerMessage::RollsTaken(taken), notify),
                Err(e) => (ServerMessage::Error(e), vec![]),
            },
        };
        self.notify(notify);
        wire(&reply)
    }

    fn record_accepted(&mut self, op: OpId, changed: bool, c: &CharacterId) {
        if changed && self.accepted.insert(op) {
            let d = self.made.get(&op).copied().unwrap_or(0);
            *self.karma.get_mut(c).unwrap() += d;
        }
    }

    /// Pushes to the given members (as the live host does).
    fn notify(&mut self, peers: Vec<EndpointId>) {
        for peer in peers {
            let Some(p) = self.players.iter().position(|x| x.id == peer) else { continue };
            for m in self.auth.outgoing_for(&peer) {
                self.auth.mark_sent(peer, &m);
                self.players[p].inbox.push(wire(&m));
            }
        }
    }

    fn join(&mut self, p: usize) {
        let m = self.players[p].replica.join_message(&format!("P{p}"), None);
        let reply = self.request(p, m);
        self.deliver_msg(p, reply);
    }

    fn deliver_msg(&mut self, p: usize, m: ServerMessage) {
        let events = self.players[p].replica.handle(engine(), m);
        for e in events {
            match e {
                Event::NeedResync(r) => {
                    // The resync request travels like any other request.
                    let reply = self.request(p, ClientMessage::Resync(r));
                    self.players[p].inbox.push(reply);
                }
                Event::Refused(r) => {
                    assert!(self.players[p].refused.insert(r.op), "op {:?} reported refused twice; {}", r.op, self.ctx());
                }
                Event::Error(e) => self.note(format!("P{p} error event: {e}")),
                _ => {}
            }
        }
    }

    fn step(&mut self) {
        let p = self.pick(self.players.len());
        let c = self.chars[p].clone();
        // The player's background save runs now and then.
        if self.chance(30) {
            let pl = &mut self.players[p];
            pl.saved = Some(pl.replica.to_bytes());
            let seq = pl.replica.last_seq();
            pl.journal.retain(|(_, x)| x.op.id.seq > seq);
        }
        match self.pick(100) {
            // A player edits offline.
            0..=24 => {
                let gain = self.chance(60);
                let amount = 1 + self.pick(8) as i32;
                let cmd = expense(gain, amount);
                let d = delta(&cmd);
                let before = self.players[p].replica.outbox(&c).len();
                if self.players[p].replica.edit(engine(), &c, cmd).is_ok() {
                    let outbox = self.players[p].replica.outbox(&c);
                    if outbox.len() > before {
                        let pending = outbox.last().unwrap().clone();
                        let op = pending.op.id;
                        self.players[p].journal.push((c.clone(), pending));
                        self.made.insert(op, d);
                        self.note(format!("P{p} edit {d:+} ({op:?})"));
                    }
                }
            }
            // A player sends their outbox.
            25..=39 => {
                if let Some(b) = self.players[p].replica.batch(&c) {
                    self.note(format!("P{p} submit {} ops from v{}", b.ops.len(), b.base_version));
                    let reply = self.request(p, ClientMessage::Submit(b));
                    self.players[p].inbox.push(reply);
                }
            }
            // A message arrives (any of the queued ones: reordering).
            40..=64 => {
                if !self.players[p].inbox.is_empty() {
                    let i = self.pick(self.players[p].inbox.len());
                    let m = self.players[p].inbox.remove(i);
                    if self.chance(10) {
                        // Delivered twice.
                        self.players[p].inbox.push(m.clone());
                    }
                    self.note(format!("P{p} gets {}", kind(&m)));
                    self.deliver_msg(p, m);
                }
            }
            // A message is lost.
            65..=69 => {
                if !self.players[p].inbox.is_empty() {
                    let i = self.pick(self.players[p].inbox.len());
                    let m = self.players[p].inbox.remove(i);
                    self.note(format!("P{p} loses {}", kind(&m)));
                }
            }
            // The GM edits.
            70..=79 => {
                let gain = self.chance(70);
                let amount = 1 + self.pick(6) as i32;
                let cmd = expense(gain, amount);
                let d = delta(&cmd);
                if let Ok(r) = self.auth.apply_local(engine(), &c, cmd) {
                    self.made.insert(r.accepted.op, d);
                    self.record_accepted(r.accepted.op, r.accepted.changed, &c);
                    self.note(format!("GM edit {c} {d:+} -> v{}", r.accepted.version));
                    self.notify(r.notify);
                }
            }
            // The GM reverts a recent change.
            80..=82 if self.reverts => {
                let v = self.auth.version(&c).unwrap();
                if v > 0 {
                    let back = 1 + self.pick(v.min(5) as usize) as u64;
                    let target = v + 1 - back;
                    if let Ok(r) = self.auth.revert(engine(), &c, target) {
                        self.note(format!("GM revert {c} v{target} -> v{}", r.applied.accepted.version));
                        self.notify(r.applied.notify);
                    }
                }
            }
            // A player reconnects (join with what they have).
            83..=87 => {
                self.note(format!("P{p} reconnects"));
                self.join(p);
            }
            // A player's app restarts (it saves after every change, as
            // `PlayerSession` does): only what was in flight is lost.
            88..=92 => {
                let crash = self.chance(60) && self.players[p].saved.is_some();
                let bytes = if crash { self.players[p].saved.clone().unwrap() } else { self.players[p].replica.to_bytes() };
                let mut r = Replica::from_bytes(engine(), &bytes).unwrap_or_else(|e| panic!("replica reload: {e}; {}", self.ctx()));
                // Commands made after the save come back from the journal.
                let n = r.recover_outbox(engine(), std::mem::take(&mut self.players[p].journal));
                self.players[p].replica = r;
                self.players[p].saved = Some(self.players[p].replica.to_bytes());
                self.players[p].inbox.clear();
                // Refusals not saved are reported again.
                self.players[p].refused = self.players[p].replica.refused().iter().map(|r| r.op).collect();
                self.note(format!("P{p} {} ({n} edit(s) from the journal)", if crash { "crashes back to its last save" } else { "restarts" }));
            }
            // The authority saves and restarts cleanly.
            93..=96 => {
                let bytes = self.auth.to_bytes();
                self.auth = Authority::from_bytes(&bytes).unwrap_or_else(|e| panic!("authority reload: {e}; {}", self.ctx()));
                self.auth_saved = bytes;
                self.note("authority saves and restarts".into());
            }
            // The authority crashes and restarts from its last save.
            97..=99 if self.crashes => {
                self.crash_authority();
            }
            _ => {}
        }
    }

    fn crash_authority(&mut self) {
        let restored = Authority::from_bytes(&self.auth_saved).unwrap();
        // Everything accepted since the save is forgotten.
        for c in self.chars.clone() {
            let before = self.auth.version(&c).unwrap();
            let after = restored.version(&c).unwrap();
            if before != after {
                self.note(format!("authority crash: {c} v{before} -> v{after}"));
            }
        }
        self.auth = restored;
        self.lost.extend(self.accepted.iter().copied());
        self.lost.extend(self.rejected.iter().copied());
        self.note("authority crashes back to its last save".into());
        for p in &mut self.players {
            p.inbox.clear();
        }
    }

    /// Heals the network: everything in flight arrives, everyone reconnects
    /// and sends until nothing is left.
    fn settle(&mut self) {
        for round in 0..20 {
            for p in 0..self.players.len() {
                while !self.players[p].inbox.is_empty() {
                    let m = self.players[p].inbox.remove(0);
                    self.deliver_msg(p, m);
                }
                self.join(p);
                for b in self.players[p].replica.batches() {
                    self.note(format!("settle: P{p} submits {} ops from v{}", b.ops.len(), b.base_version));
                    let reply = self.request(p, ClientMessage::Submit(b));
                    self.note(format!("settle: P{p} gets {}", kind(&reply)));
                    self.deliver_msg(p, reply);
                    let c = &self.chars[p];
                    let (v, n, rs) = (self.players[p].replica.version(c), self.players[p].replica.outbox(c).len(), self.players[p].replica.needs_resync(c));
                    self.note(format!("settle: P{p} now v{v:?}, outbox {n}, resync {rs}, authority v{:?}", self.auth.version(c)));
                }
                while !self.players[p].inbox.is_empty() {
                    let m = self.players[p].inbox.remove(0);
                    self.deliver_msg(p, m);
                }
            }
            let done = self.players.iter().enumerate().all(|(i, p)| {
                let c = &self.chars[i];
                p.replica.outbox_len() == 0 && !p.replica.needs_resync(c) && p.replica.version(c) == self.auth.version(c) && p.replica.confirmed_hash(c) == self.auth.hash(c)
            });
            if done {
                self.note(format!("settled after {} round(s)", round + 1));
                return;
            }
        }
    }

    fn check(&mut self) {
        self.settle();
        for (i, p) in self.players.iter().enumerate() {
            let c = &self.chars[i];
            let ctx = || format!("P{i} {c}: {}", self.ctx());
            assert_eq!(p.replica.outbox_len(), 0, "outbox not empty; {}", ctx());
            assert!(!p.replica.needs_resync(c), "still needs a resync; {}", ctx());
            assert_eq!(p.replica.version(c), self.auth.version(c), "version; {}", ctx());
            assert_eq!(p.replica.confirmed_hash(c), self.auth.hash(c), "hash; {}", ctx());
            assert_eq!(command::state_hash(p.replica.character(c).unwrap()), self.auth.hash(c).unwrap(), "shown state; {}", ctx());
            assert_eq!(command::state_hash(p.replica.confirmed(c).unwrap()), self.auth.hash(c).unwrap(), "recomputed hash; {}", ctx());
            // The log never holds an op twice.
            let log = self.auth.log(c);
            let ids: BTreeSet<OpId> = log.iter().map(|e| e.op).collect();
            assert_eq!(ids.len(), log.len(), "an op is in the log twice; {}", ctx());
            for w in log.windows(2) {
                assert_eq!(w[1].version, w[0].version + 1, "log versions have a gap; {}", ctx());
            }
            // Refused ops were really refused (or forgotten by a crash).
            for op in &p.refused {
                assert!(self.rejected.contains(op) || self.lost.contains(op), "{op:?} reported refused but was not; {}", ctx());
                assert!(!self.accepted.contains(op) || self.lost.contains(op), "{op:?} accepted and refused; {}", ctx());
            }
        }
        // Every op made ran once or was refused.
        for op in self.made.keys() {
            assert!(self.accepted.contains(op) || self.rejected.contains(op), "{op:?} never reached the authority; {}", self.ctx());
            assert!(!(self.accepted.contains(op) && self.rejected.contains(op)) || self.lost.contains(op), "{op:?} both accepted and refused; {}", self.ctx());
        }
        if !self.reverts && !self.crashes {
            for c in &self.chars {
                let have = self.auth.character(c).unwrap().karma as i64;
                assert_eq!(have, self.karma[c], "karma of {c} is not the sum of the accepted changes; {}", self.ctx());
            }
        }
    }
}

fn kind(m: &ServerMessage) -> String {
    match m {
        ServerMessage::Joined { pushes, .. } => format!("joined ({} pushes)", pushes.len()),
        ServerMessage::Ack(a) => format!("ack ({} ok, {} refused, to v{})", a.accepted.len(), a.rejected.len(), a.update.version),
        ServerMessage::Push(p) => format!("push {} v{}->v{}{}", p.character, p.from_version, p.version, if p.is_snapshot() { " (snapshot)" } else { "" }),
        ServerMessage::Membership(_) => "membership".into(),
        ServerMessage::Error(e) => format!("error {e}"),
        ServerMessage::Denied(d) => format!("denied {d}"),
        ServerMessage::RollsTaken(r) => format!("{} rolls taken", r.len()),
        ServerMessage::Rolls(r) => format!("{} rolls", r.len()),
    }
}

fn seeds(default: u64) -> Vec<u64> {
    if let Some(s) = std::env::var("CHUMMER_CHAOS_SEED").ok().and_then(|s| s.parse().ok()) {
        return vec![s];
    }
    let n = std::env::var("CHUMMER_CHAOS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default);
    (1..=n).collect()
}

fn run(seed_salt: u64, steps: usize, players: usize, reverts: bool, crashes: bool, default_seeds: u64) {
    for s in seeds(default_seeds) {
        let seed = s.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed_salt;
        let mut w = World::new(seed, players, reverts, crashes);
        for _ in 0..steps {
            w.step();
        }
        w.check();
    }
}

/// Lossy, reordering, duplicating network; offline edits; replica restarts;
/// clean authority restarts. Karma is checked against the accepted ops.
#[test]
fn replicas_converge_and_ops_run_once() {
    run(0x5eed_0001, 100, 3, false, false, 2);
}

/// As above, with GM reverts (only convergence is checked).
#[test]
fn replicas_converge_with_reverts() {
    run(0x5eed_0002, 100, 2, true, false, 1);
}

/// As above, with the authority crashing back to its last save: changes
/// it acknowledged after that are lost, but every copy still converges.
#[test]
fn replicas_converge_after_authority_crashes() {
    run(0x5eed_0003, 120, 2, true, true, 2);
}
