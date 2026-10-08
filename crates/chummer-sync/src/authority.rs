//! The campaign authority: the GM's copy of every character, which decides
//! the order of all changes.
//!
//! [`Authority`] is plain data and synchronous: no networking, no clock
//! except the one it is given, so it can be tested directly and driven by
//! any transport ([`crate::host::AuthorityHost`] over chummer-net, or the
//! mailbox).
//!
//! Per character it keeps the live [`Character`], its version (commands
//! applied), the hash of its canonical form, a window of the last
//! [`LOG_WINDOW`] log entries (with the hash after each, so a client's base
//! can be checked and short pushes sent) and the outcome of the last
//! [`SEEN_LIMIT`] operation ids (so a resubmitted or replayed command runs
//! once and gets the same answer). The window is the compaction: older
//! entries are dropped, and a client further behind gets a snapshot.
//!
//! Submitting ([`Authority::submit`]): each command runs on the current
//! state. When the client's base is the current version this is a plain
//! apply; otherwise it is the rebase of the design (commands are intents:
//! "raise Pistols" still works after the GM gave karma). A command the
//! engine refuses (not enough karma any more) is rejected with the reason.
//! Nothing is refused for being "not allowed": the only access rule is
//! that a player submits for their own characters.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;

use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, Rejected};
use chummer_core::dice::Rng;
use chummer_core::engine::Engine;
use std::collections::BTreeSet;

use chummer_net::campaign::DenyReason;
use chummer_net::invite::{CampaignId, InviteId, Role};
use chummer_net::{EndpointId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::feed;
use crate::invites::{Claim, Invite, InviteOp};
use crate::mail::Inbox;
use crate::msg::{
    Accepted, Ack, CharacterId, CharacterInfo, ClaimProof, Entry, FeedEntry, Hash, Have, MemberInfo, Membership, Op, OpId, Push, PushBody, RejectedOp, ResyncRequest,
    ServerMessage, SubmitBatch,
};
use crate::persist::{self, PersistError};

/// Log entries kept per character for incremental pushes.
pub const LOG_WINDOW: usize = 256;

/// Operation ids remembered per character for de-duplication.
pub const SEEN_LIMIT: usize = 20_000;

/// How far ahead of the authority a member's version may claim to be
/// before [`CharState::catch_up`] stops believing it.
const MAX_VERSION_JUMP: u64 = 1 << 32;

/// Entries carried in a snapshot push for the feed.
const RECENT_IN_SNAPSHOT: usize = 20;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A member of the campaign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub role: Role,
    pub name: String,
    /// The invite this member claimed; `None` for members the GM added by
    /// node id (they prove themselves by their node key).
    pub invite: Option<InviteId>,
    /// Unix seconds: the last time this member connected or mailed.
    pub last_seen: Option<u64>,
}

impl Member {
    pub fn new(role: Role, name: impl Into<String>) -> Member {
        Member { role, name: name.into(), invite: None, last_seen: None }
    }
}

/// What [`Authority::admit`] let in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    pub role: Role,
    /// The invite's label (or the member's name).
    pub label: String,
    /// This admission claimed an invite (a new member).
    pub claimed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Outcome {
    Accepted(Accepted),
    Rejected(RejectedOp),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogItem {
    entry: Entry,
    /// The hash after this entry.
    hash: Hash,
}

#[derive(Debug, Clone, Default)]
struct Seen {
    order: VecDeque<OpId>,
    map: HashMap<OpId, Outcome>,
}

impl Seen {
    fn get(&self, id: &OpId) -> Option<&Outcome> {
        self.map.get(id)
    }

    fn insert(&mut self, id: OpId, o: Outcome) {
        if self.map.insert(id, o).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > SEEN_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

struct CharState {
    name: String,
    owner: Option<EndpointId>,
    ch: Character,
    version: u64,
    hash: Hash,
    log: VecDeque<LogItem>,
    /// The hash at `version - log.len()`.
    base_hash: Hash,
    /// The character at `version - log.len()`, for reverting entries in
    /// the window; and its snapshot, made on the first save after it moved.
    base: Character,
    base_snapshot: Option<Vec<u8>>,
    seen: Seen,
    /// The last snapshot made, and its version (compressing is slow).
    snapshot: Option<(u64, Vec<u8>)>,
}

impl CharState {
    fn new(name: String, owner: Option<EndpointId>, ch: Character) -> CharState {
        let hash = command::state_hash(&ch);
        CharState { name, owner, base: ch.clone(), ch, version: 0, hash, log: VecDeque::new(), base_hash: hash, base_snapshot: None, seen: Seen::default(), snapshot: None }
    }

    fn window_base(&self) -> u64 {
        self.version - self.log.len() as u64
    }

    fn hash_at(&self, v: u64) -> Option<Hash> {
        let base = self.window_base();
        if v == base {
            Some(self.base_hash)
        } else if v > base && v <= self.version {
            Some(self.log[(v - base - 1) as usize].hash)
        } else {
            None
        }
    }

    fn snapshot(&mut self) -> Vec<u8> {
        match &self.snapshot {
            Some((v, b)) if *v == self.version => b.clone(),
            _ => {
                let b = command::snapshot(&self.ch);
                self.snapshot = Some((self.version, b.clone()));
                b
            }
        }
    }

    fn push_log(&mut self, engine: &Engine, item: LogItem) {
        self.log.push_back(item);
        while self.log.len() > LOG_WINDOW {
            let old = self.log.pop_front().expect("not empty");
            // Commands are deterministic: the base moves forward exactly as
            // the live state did.
            if command::apply(&mut self.base, engine, &old.entry.env).is_err() {
                tracing::warn!("the log window's base did not take an entry it applied before");
            }
            self.base_hash = old.hash;
            self.base_snapshot = None;
        }
    }

    /// A member has `claimed` (a version, with what it contains), which is
    /// newer than ours: this authority went back to an older save (it
    /// crashed between acknowledging changes and saving them). Versions
    /// only go forward, so move past theirs: the current state becomes
    /// that newer version with an empty log window, and every copy gets
    /// it as a snapshot, which it takes because it is newer. Absurd claims
    /// (more than [`MAX_VERSION_JUMP`] ahead) are ignored.
    fn catch_up(&mut self, id: &CharacterId, claimed: u64) {
        if claimed <= self.version || claimed - self.version > MAX_VERSION_JUMP {
            return;
        }
        tracing::warn!("{id}: a member has version {claimed}, this authority only {}; it lost changes (a crash before saving?) and moves on to {}", self.version, claimed + 1);
        self.version = claimed + 1;
        self.log.clear();
        self.base = self.ch.clone();
        self.base_hash = self.hash;
        self.base_snapshot = None;
        self.snapshot = None;
    }

    fn base_snapshot(&mut self) -> Vec<u8> {
        self.base_snapshot.get_or_insert_with(|| command::snapshot(&self.base)).clone()
    }

    /// From `from` (with hash `from_hash`, when known) to now: the entries
    /// when the window reaches back that far, else a snapshot.
    fn push_from(&mut self, id: &CharacterId, from: u64, from_hash: Option<Hash>, force_snapshot: bool) -> Push {
        let fits = !force_snapshot && self.hash_at(from).is_some_and(|h| from_hash.is_none_or(|f| f == h));
        let body = if fits {
            let base = self.window_base();
            PushBody::Entries(self.log.iter().skip((from - base) as usize).map(|i| i.entry.clone()).collect())
        } else {
            let recent = self.log.iter().rev().take(RECENT_IN_SNAPSHOT).rev().map(|i| i.entry.clone()).collect();
            PushBody::Snapshot { bytes: self.snapshot(), recent }
        };
        Push { character: id.clone(), name: self.name.clone(), from_version: if fits { from } else { self.version }, body, version: self.version, hash: self.hash }
    }
}

/// A snapshot to make without holding the authority's lock
/// ([`Authority::save_snapshot_work`]).
#[derive(Debug, Clone)]
pub struct SnapshotJob {
    id: CharacterId,
    /// The log window's base (else the current state).
    base: bool,
    /// The version it is of.
    version: u64,
    ch: Character,
}

/// A made [`SnapshotJob`], for [`Authority::put_snapshots`].
#[derive(Debug, Clone)]
pub struct MadeSnapshot {
    id: CharacterId,
    base: bool,
    version: u64,
    bytes: Vec<u8>,
}

impl SnapshotJob {
    /// Compresses the character (slow: call it without the lock).
    pub fn make(self) -> MadeSnapshot {
        MadeSnapshot { bytes: command::snapshot(&self.ch), id: self.id, base: self.base, version: self.version }
    }
}

/// What a submission did, for the transport to deliver.
#[derive(Debug, Clone)]
pub struct Submitted {
    pub ack: Ack,
    /// Other members who see this character and should get a push
    /// ([`Authority::push_for`]).
    pub notify: Vec<EndpointId>,
}

/// What [`Authority::revert`] did.
#[derive(Debug, Clone)]
pub struct Reverted {
    pub applied: LocalApplied,
    /// The versions taken back.
    pub reverted: std::ops::RangeInclusive<u64>,
    /// Later changes that no longer applied ("Raised Pistols to 6 (12
    /// karma) (not enough karma)").
    pub dropped: Vec<String>,
}

/// A GM edit made at the authority.
#[derive(Debug, Clone)]
pub struct LocalApplied {
    pub accepted: Accepted,
    pub notify: Vec<EndpointId>,
}

/// The campaign authority. See the module documentation.
pub struct Authority {
    campaign: CampaignId,
    /// The campaign's name, shown to members.
    name: String,
    /// The GM running this authority.
    me: EndpointId,
    origin: [u8; 16],
    next_seq: u64,
    seeds: Rng,
    clock: fn() -> i64,
    members: BTreeMap<EndpointId, Member>,
    invites: BTreeMap<InviteId, Invite>,
    /// Nodes that were members and were revoked, removed or replaced by a
    /// re-issued link: refused, and not made members again from the
    /// campaign file's owners.
    retired: BTreeSet<EndpointId>,
    /// The generation of the GM's campaign key
    /// ([`chummer_net::invite::derive_campaign_key`]), and the membership
    /// revision when it last changed.
    key_gen: u32,
    key_rev: u64,
    /// The public campaign keys (current first), set by the host, which
    /// has the GM's secret.
    gm_keys: Vec<PublicKey>,
    /// Characters whose owner the authority changed (a claim) and the
    /// campaign file has not taken yet ([`Authority::take_owner_changes`]).
    owner_changes: BTreeSet<CharacterId>,
    chars: BTreeMap<CharacterId, CharState>,
    /// The version of each character each member has been sent.
    delivered: BTreeMap<(EndpointId, CharacterId), u64>,
    /// Counts membership changes; `membership_sent` is what each member
    /// has been told.
    membership_rev: u64,
    membership_sent: BTreeMap<EndpointId, u64>,
    feed: VecDeque<FeedEntry>,
    /// Chunks of mailed messages being put together.
    pub inbox: Inbox,
    /// Answers to mailed submissions, waiting to be mailed back.
    mail_out: Vec<(EndpointId, ServerMessage)>,
    /// Log entries made since [`Authority::take_applied`] (for the
    /// journal; not saved).
    applied: Vec<(CharacterId, Entry)>,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authority").field("campaign", &self.campaign).field("me", &self.me).field("members", &self.members).field("characters", &self.chars.keys().collect::<Vec<_>>()).finish()
    }
}

impl Authority {
    /// A new campaign hosted by `gm` (this machine's node id).
    pub fn new(campaign: CampaignId, gm: EndpointId, gm_name: impl Into<String>) -> Authority {
        let mut members = BTreeMap::new();
        members.insert(gm, Member::new(Role::Gm, gm_name));
        Authority {
            campaign,
            name: String::new(),
            me: gm,
            origin: chummer_net::invite::random_id(),
            next_seq: 0,
            seeds: Rng::from_time(),
            clock: now_ms,
            members,
            invites: BTreeMap::new(),
            retired: BTreeSet::new(),
            key_gen: 0,
            key_rev: 0,
            gm_keys: Vec::new(),
            owner_changes: BTreeSet::new(),
            chars: BTreeMap::new(),
            delivered: BTreeMap::new(),
            membership_rev: 0,
            membership_sent: BTreeMap::new(),
            feed: VecDeque::new(),
            inbox: Inbox::default(),
            mail_out: Vec::new(),
            applied: Vec::new(),
        }
    }

    /// Use another clock (Unix milliseconds) for the GM's own commands.
    pub fn with_clock(mut self, clock: fn() -> i64) -> Authority {
        self.clock = clock;
        self
    }

    pub fn campaign(&self) -> CampaignId {
        self.campaign
    }

    pub fn gm(&self) -> EndpointId {
        self.me
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Renames the campaign (members are told with the next membership).
    pub fn set_name(&mut self, name: &str) {
        if self.name != name {
            self.name = name.to_owned();
            self.membership_rev += 1;
        }
    }

    // ----- members and invites -----

    pub fn members(&self) -> &BTreeMap<EndpointId, Member> {
        &self.members
    }

    pub fn role(&self, peer: &EndpointId) -> Option<Role> {
        self.members.get(peer).map(|m| m.role)
    }

    /// Adds a member directly by node id (the GM adding a known player, or
    /// a campaign file naming them as an owner). They prove themselves by
    /// their node key. A retired node is let back in (callers that act on
    /// their own, like [`crate::hosted::reconcile`], check
    /// [`Authority::is_retired`] first).
    pub fn add_member(&mut self, peer: EndpointId, role: Role, name: impl Into<String>) {
        self.retired.remove(&peer);
        self.members.insert(peer, Member::new(role, name));
        self.membership_rev += 1;
    }

    /// Whether `peer` was a member and was revoked, removed or replaced.
    pub fn is_retired(&self, peer: &EndpointId) -> bool {
        self.retired.contains(peer)
    }

    /// Takes `peer` out of the campaign: refused from now on (until a new
    /// invite of theirs is claimed). Their characters keep them as owner
    /// (the GM gives them to someone else). Returns whether they were a
    /// member.
    pub fn remove_member(&mut self, peer: &EndpointId) -> bool {
        if *peer == self.me {
            return false;
        }
        let gone = self.members.remove(peer);
        if let Some(id) = gone.as_ref().and_then(|m| m.invite) {
            if let Some(i) = self.invites.get_mut(&id) {
                if i.claimed.as_ref().is_some_and(|c| c.node == *peer) && i.revoked.is_none() {
                    i.revoked = Some(now_secs());
                }
            }
        }
        self.retire(peer);
        gone.is_some()
    }

    fn retire(&mut self, peer: &EndpointId) {
        if *peer == self.me {
            return;
        }
        self.members.remove(peer);
        self.retired.insert(*peer);
        self.delivered.retain(|(p, _), _| p != peer);
        self.membership_sent.remove(peer);
        self.mail_out.retain(|(p, _)| p != peer);
        self.membership_rev += 1;
    }

    pub fn invites(&self) -> &BTreeMap<InviteId, Invite> {
        &self.invites
    }

    pub fn invite(&self, id: &InviteId) -> Option<&Invite> {
        self.invites.get(id)
    }

    /// The invite `peer` claimed, if any.
    pub fn invite_of(&self, peer: &EndpointId) -> Option<&Invite> {
        self.members.get(peer).and_then(|m| m.invite).and_then(|i| self.invites.get(&i))
    }

    /// A new invite (link) for one player.
    pub fn create_invite(&mut self, role: Role, label: &str, assign: Option<CharacterId>, expires: Option<u64>, now: u64) -> &Invite {
        let i = Invite::new(role, label, assign, expires, now);
        let id = i.id;
        self.invites.insert(id, i);
        &self.invites[&id]
    }

    /// Adds an invite made elsewhere (`chummer-authority invite create`).
    /// Returns false when its id is known already.
    pub fn insert_invite(&mut self, invite: Invite) -> bool {
        if self.invites.contains_key(&invite.id) {
            return false;
        }
        self.invites.insert(invite.id, invite);
        true
    }

    /// Revokes an invite: its key no longer joins or mails, and the node
    /// that claimed it is out (returned, to hang up on).
    pub fn revoke_invite(&mut self, id: &InviteId, now: u64) -> Result<Option<EndpointId>, String> {
        let i = self.invites.get_mut(id).ok_or("no such invite")?;
        if i.revoked.is_none() {
            i.revoked = Some(now);
        }
        let node = i.claimed.as_ref().map(|c| c.node);
        if let Some(n) = node {
            self.retire(&n);
        }
        Ok(node)
    }

    /// A new link for the same member (a new device): a new key; the old
    /// link and the node that claimed it (returned, to hang up on) stop
    /// working. Whoever claims the new link gets that node's characters.
    pub fn reissue_invite(&mut self, id: &InviteId, expires: Option<u64>, now: u64) -> Result<Option<EndpointId>, String> {
        let i = self.invites.get_mut(id).ok_or("no such invite")?;
        let node = i.reissue(expires, now);
        if let Some(n) = node {
            self.retire(&n);
        }
        Ok(node)
    }

    /// As [`Authority::reissue_invite`] with the key made elsewhere.
    pub fn reissue_invite_with(&mut self, id: &InviteId, secret: chummer_net::invite::MemberSecret, expires: Option<u64>, now: u64) -> Result<Option<EndpointId>, String> {
        let i = self.invites.get(id).ok_or("no such invite")?;
        if i.secret == secret {
            return Ok(None); // applied already
        }
        let node = self.reissue_invite(id, expires, now)?;
        self.invites.get_mut(id).expect("checked").secret = secret;
        Ok(node)
    }

    /// Deletes an invite from the list; the node that claimed it is out
    /// (returned).
    pub fn remove_invite(&mut self, id: &InviteId) -> Option<EndpointId> {
        let i = self.invites.remove(id)?;
        let node = i.claimed.map(|c| c.node);
        if let Some(n) = node {
            self.retire(&n);
        }
        node
    }

    /// Applies a change from the invites file. Returns the node to hang
    /// up on, if any, and whether anything changed.
    pub fn apply_invite_op(&mut self, op: InviteOp, now: u64) -> (bool, Option<EndpointId>) {
        match op {
            InviteOp::Create { invite } => (self.insert_invite(invite), None),
            InviteOp::Revoke { id } => match self.invites.get(&id) {
                Some(i) if i.revoked.is_none() => (true, self.revoke_invite(&id, now).ok().flatten()),
                _ => (false, None),
            },
            InviteOp::Reissue { id, secret, expires } => match self.invites.get(&id) {
                Some(i) if i.secret != secret => (true, self.reissue_invite_with(&id, secret, expires, now).ok().flatten()),
                _ => (false, None),
            },
            InviteOp::Remove { id } => {
                let had = self.invites.contains_key(&id);
                (had, self.remove_invite(&id))
            }
            InviteOp::RotateKey { generation } => {
                let behind = self.key_gen < generation;
                while self.key_gen < generation {
                    self.rotate_campaign_key();
                }
                (behind, None)
            }
        }
    }

    /// Lets `peer` in. `key` is the member key it proved (live: the
    /// handshake; by mail: [`Authority::admit_by_mail`]). A member is let
    /// in with its invite's current key (or, added by node id, without
    /// one); a new node with an active invite's key claims it. Records
    /// when the member was last seen.
    pub fn admit(&mut self, peer: EndpointId, campaign: CampaignId, key: Option<PublicKey>, now: u64) -> Result<Admitted, DenyReason> {
        if campaign != self.campaign {
            return Err(DenyReason::NoSuchCampaign);
        }
        if let Some(m) = self.members.get(&peer) {
            let (role, mut label, invite) = (m.role, m.name.clone(), m.invite);
            if let Some(id) = invite {
                let i = self.invites.get(&id).ok_or(DenyReason::Revoked)?;
                if i.revoked.is_some() {
                    return Err(DenyReason::Revoked);
                }
                match key {
                    Some(k) if k == i.key() => {}
                    Some(k) if i.superseded.contains(&k) => return Err(DenyReason::Superseded),
                    _ => return Err(DenyReason::BadProof),
                }
                label = i.label.clone();
            }
            self.members.get_mut(&peer).expect("member").last_seen = Some(now);
            return Ok(Admitted { role, label, claimed: false });
        }
        let Some(key) = key else {
            return Err(if self.retired.contains(&peer) { DenyReason::Revoked } else { DenyReason::NotInvited });
        };
        let Some(id) = self.invites.values().find(|i| i.key() == key).map(|i| i.id) else {
            if self.invites.values().any(|i| i.superseded.contains(&key)) {
                return Err(DenyReason::Superseded);
            }
            return Err(if self.retired.contains(&peer) { DenyReason::Revoked } else { DenyReason::NotInvited });
        };
        let i = &self.invites[&id];
        if let Some(r) = i.refusal(&peer, now) {
            return Err(r);
        }
        if i.claimed.is_some() {
            // Claimed by this node, which was then taken out.
            return Err(DenyReason::Revoked);
        }
        // Claim it.
        let i = self.invites.get_mut(&id).expect("found");
        i.claimed = Some(Claim { node: peer, at: now });
        let (role, label, previous, assign) = (i.role, i.label.clone(), i.previous.take(), i.assign.take());
        self.retired.remove(&peer);
        self.members.insert(peer, Member { role, name: label.clone(), invite: Some(id), last_seen: Some(now) });
        self.membership_rev += 1;
        if let Some(old) = previous {
            let theirs: Vec<CharacterId> = self.chars.iter().filter(|(_, c)| c.owner == Some(old)).map(|(id, _)| id.clone()).collect();
            for c in theirs {
                self.set_owner(&c, Some(peer));
                self.owner_changes.insert(c);
            }
        }
        if let Some(c) = assign.filter(|c| self.chars.contains_key(c)) {
            self.set_owner(&c, Some(peer));
            self.owner_changes.insert(c);
        }
        Ok(Admitted { role, label, claimed: true })
    }

    /// [`Authority::admit`] for a mailed join: `proof` (when the sender is
    /// not a member yet) shows it holds an invite's member key.
    pub fn admit_by_mail(&mut self, peer: EndpointId, proof: Option<&ClaimProof>, now: u64) -> Result<Admitted, DenyReason> {
        let key = match proof {
            Some(p) if p.verify(&self.campaign, &self.me, &peer) => Some(p.key),
            Some(_) => return Err(DenyReason::BadProof),
            None => {
                // Members prove themselves by the sealed mail's signature;
                // take their invite's key as proven.
                self.invite_of(&peer).map(Invite::key)
            }
        };
        self.admit(peer, self.campaign, key, now)
    }

    /// Notes that `peer` was heard from (mail).
    pub fn seen(&mut self, peer: &EndpointId, now: u64) {
        if let Some(m) = self.members.get_mut(peer) {
            m.last_seen = Some(now);
        }
    }

    /// The keys that may put mail into the GM's relay mailbox: every
    /// active invite's key (claimed, or unclaimed and not expired: a
    /// play-by-post player joins by mail) and the node keys of members
    /// added by node id. Sorted.
    pub fn mail_keys(&self, now: u64) -> Vec<PublicKey> {
        let mut keys: Vec<PublicKey> = self.invites.values().filter(|i| i.active(now)).map(Invite::key).collect();
        keys.extend(self.members.iter().filter(|(p, m)| **p != self.me && m.invite.is_none()).map(|(p, _)| *p));
        keys.sort();
        keys.dedup();
        keys
    }

    /// Characters whose owner a claim changed since the last call, for
    /// the campaign file.
    pub fn take_owner_changes(&mut self) -> BTreeSet<CharacterId> {
        std::mem::take(&mut self.owner_changes)
    }

    pub fn has_owner_changes(&self) -> bool {
        !self.owner_changes.is_empty()
    }

    /// Whether a claim changed `id`'s owner and the campaign file has not
    /// taken it yet.
    pub fn has_owner_change(&self, id: &CharacterId) -> bool {
        self.owner_changes.contains(id)
    }

    // ----- the GM's campaign key -----

    /// The generation of the GM's campaign key.
    pub fn key_generation(&self) -> u32 {
        self.key_gen
    }

    /// The public campaign keys (current first) members are told; the host
    /// sets them from the GM's secret ([`chummer_net::invite::derive_campaign_key`]).
    pub fn set_gm_keys(&mut self, keys: Vec<PublicKey>) {
        if self.gm_keys != keys {
            self.gm_keys = keys;
            self.membership_rev += 1;
        }
    }

    pub fn gm_keys(&self) -> &[PublicKey] {
        &self.gm_keys
    }

    /// A new campaign key generation (the host then sets the keys). Members
    /// get it with the next membership; until a member has it, mail to it
    /// is signed with the previous key ([`Authority::mail_key_generation`]).
    pub fn rotate_campaign_key(&mut self) -> u32 {
        self.key_gen += 1;
        self.membership_rev += 1;
        self.key_rev = self.membership_rev;
        self.key_gen
    }

    /// Which campaign key generation to sign mail to `peer` with: the
    /// current one once `peer` was sent a membership naming it.
    pub fn mail_key_generation(&self, peer: &EndpointId) -> u32 {
        if self.key_gen == 0 || self.membership_sent.get(peer).is_some_and(|v| *v >= self.key_rev) {
            self.key_gen
        } else {
            self.key_gen - 1
        }
    }

    fn member_infos(&self) -> Vec<MemberInfo> {
        self.members.iter().map(|(id, m)| MemberInfo { id: *id, role: m.role, name: if m.name.is_empty() { feed::short_name(id) } else { m.name.clone() } }).collect()
    }

    /// What `peer` is told about the campaign.
    pub fn membership(&self, peer: &EndpointId) -> Option<Membership> {
        let role = self.role(peer)?;
        let characters = self
            .chars
            .iter()
            .filter(|(_, c)| self.sees(peer, role, c))
            .map(|(id, c)| CharacterInfo { id: id.clone(), name: c.name.clone(), owner: c.owner, version: c.version })
            .collect();
        let label = self.invite_of(peer).map(|i| i.label.clone()).unwrap_or_default();
        Some(Membership { campaign_name: self.name.clone(), you: *peer, role, label, gm_keys: self.gm_keys.clone(), members: self.member_infos(), characters })
    }

    fn sees(&self, peer: &EndpointId, role: Role, c: &CharState) -> bool {
        role == Role::Gm || c.owner.as_ref() == Some(peer)
    }

    /// Whether `peer` may see and edit `id`.
    pub fn can_see(&self, peer: &EndpointId, id: &CharacterId) -> bool {
        match (self.role(peer), self.chars.get(id)) {
            (Some(role), Some(c)) => self.sees(peer, role, c),
            _ => false,
        }
    }

    /// The characters `peer` sees.
    pub fn visible(&self, peer: &EndpointId) -> Vec<CharacterId> {
        let Some(role) = self.role(peer) else { return Vec::new() };
        self.chars.iter().filter(|(_, c)| self.sees(peer, role, c)).map(|(id, _)| id.clone()).collect()
    }

    /// Members other than this authority's GM who see `id`: its owner and
    /// any other GMs.
    pub fn interested(&self, id: &CharacterId) -> Vec<EndpointId> {
        let Some(c) = self.chars.get(id) else { return Vec::new() };
        self.members.iter().filter(|(p, m)| **p != self.me && (m.role == Role::Gm || c.owner.as_ref() == Some(*p))).map(|(p, _)| *p).collect()
    }

    // ----- characters -----

    /// Adds a character to the campaign, owned by `owner` (`None`: the
    /// GM's own). It is normalised through its canonical form first, so it
    /// is exactly what a client restores from a snapshot. Returns the
    /// members to send it to.
    pub fn add_character(&mut self, id: CharacterId, owner: Option<EndpointId>, ch: Character) -> Result<Vec<EndpointId>, command::RestoreError> {
        // `restore(snapshot(ch))` without the compression (seconds for a
        // character with big mugshots): the same canonical text, parsed.
        let ch = Character::from_str(&command::canonical(&ch)).map_err(command::RestoreError::Load)?;
        let name = ch.display_name();
        self.chars.insert(id.clone(), CharState::new(name, owner, ch));
        self.delivered.retain(|(_, c), _| *c != id);
        self.membership_rev += 1;
        Ok(self.interested(&id))
    }

    /// Gives a character to another player (or to the GM with `None`).
    pub fn set_owner(&mut self, id: &CharacterId, owner: Option<EndpointId>) -> bool {
        let Some(c) = self.chars.get_mut(id) else { return false };
        if let Some(old) = c.owner {
            self.delivered.remove(&(old, id.clone()));
        }
        c.owner = owner;
        self.membership_rev += 1;
        true
    }

    pub fn remove_character(&mut self, id: &CharacterId) -> Option<Character> {
        let c = self.chars.remove(id)?;
        self.delivered.retain(|(_, k), _| k != id);
        self.membership_rev += 1;
        Some(c.ch)
    }

    pub fn characters(&self) -> impl Iterator<Item = &CharacterId> {
        self.chars.keys()
    }

    pub fn character(&self, id: &CharacterId) -> Option<&Character> {
        self.chars.get(id).map(|c| &c.ch)
    }

    pub fn owner(&self, id: &CharacterId) -> Option<EndpointId> {
        self.chars.get(id).and_then(|c| c.owner)
    }

    pub fn version(&self, id: &CharacterId) -> Option<u64> {
        self.chars.get(id).map(|c| c.version)
    }

    pub fn hash(&self, id: &CharacterId) -> Option<Hash> {
        self.chars.get(id).map(|c| c.hash)
    }

    /// The log window of a character, oldest first.
    pub fn log(&self, id: &CharacterId) -> Vec<Entry> {
        self.chars.get(id).map(|c| c.log.iter().map(|i| i.entry.clone()).collect()).unwrap_or_default()
    }

    /// The activity feed, oldest first: every applied and refused command.
    pub fn feed(&self) -> &VecDeque<FeedEntry> {
        &self.feed
    }

    // ----- applying -----

    fn run_op(&mut self, engine: &Engine, id: &CharacterId, author: EndpointId, op: &Op, rebased: bool) -> Outcome {
        let members = self.member_infos();
        let c = self.chars.get_mut(id).expect("checked by the caller");
        if let Some(o) = c.seen.get(&op.id) {
            return o.clone();
        }
        let mut env = op.env.clone();
        env.author = author.to_string();
        let (author_name, author_role) = feed::member_label(&members, &author);
        let outcome = match command::apply(&mut c.ch, engine, &env) {
            Ok(applied) if applied.changed => {
                c.version += 1;
                c.hash = command::state_hash(&c.ch);
                let entry = Entry { version: c.version, op: op.id, env, author, description: applied.description.clone() };
                let line = feed::from_entry(&members, id, &c.name, &entry);
                self.applied.push((id.clone(), entry.clone()));
                match c.log.back() {
                    Some(prev) if feed::coalesces(&prev.entry, &entry) => feed::merge(&mut self.feed, line, prev.entry.version),
                    _ => feed::push(&mut self.feed, line),
                }
                c.push_log(engine, LogItem { entry, hash: c.hash });
                if let Some(name) = Some(c.ch.display_name()).filter(|n| *n != c.name) {
                    c.name = name;
                    self.membership_rev += 1;
                }
                Outcome::Accepted(Accepted { op: op.id, version: c.version, hash: c.hash, changed: true, rebased, description: applied.description })
            }
            Ok(_) => Outcome::Accepted(Accepted { op: op.id, version: c.version, hash: c.hash, changed: false, rebased, description: String::new() }),
            Err(Rejected { reason, confirm }) => {
                feed::push(
                    &mut self.feed,
                    FeedEntry {
                        at: env.at,
                        character: id.clone(),
                        character_name: c.name.clone(),
                        author,
                        author_name,
                        author_role,
                        text: describe_intent(&env),
                        version: None,
                        rejected: Some(reason.clone()),
                    },
                );
                Outcome::Rejected(RejectedOp { op: op.id, reason, confirm })
            }
        };
        c.seen.insert(op.id, outcome.clone());
        outcome
    }

    fn check_access(&self, peer: &EndpointId, id: &CharacterId) -> Result<(), String> {
        if self.role(peer).is_none() {
            return Err("not a member of this campaign".into());
        }
        if !self.chars.contains_key(id) {
            return Err(format!("no character {id} in this campaign"));
        }
        if !self.can_see(peer, id) {
            return Err(format!("character {id} is not yours"));
        }
        Ok(())
    }

    /// Runs a member's batch. `Err` means the batch could not be handled
    /// at all (not a member, not their character); rejections of single
    /// commands are in the [`Ack`].
    pub fn submit(&mut self, engine: &Engine, peer: EndpointId, batch: SubmitBatch) -> Result<Submitted, String> {
        self.check_access(&peer, &batch.character)?;
        let id = batch.character.clone();
        self.chars.get_mut(&id).expect("checked").catch_up(&id, batch.base_version);
        let (start, diverged) = {
            let c = &self.chars[&id];
            (c.version, c.hash_at(batch.base_version).is_none_or(|h| h != batch.base_hash))
        };
        let rebased = start != batch.base_version || diverged;
        let (mut accepted, mut rejected) = (Vec::new(), Vec::new());
        for op in &batch.ops {
            match self.run_op(engine, &id, peer, op, rebased) {
                Outcome::Accepted(a) => accepted.push(a),
                Outcome::Rejected(r) => rejected.push(r),
            }
        }
        let c = self.chars.get_mut(&id).expect("checked");
        let changed = c.version != start;
        let update = c.push_from(&id, batch.base_version, Some(batch.base_hash), diverged);
        let version = c.version;
        self.mark_delivered(peer, &id, version);
        let notify = if changed { self.interested(&id).into_iter().filter(|p| *p != peer).collect() } else { Vec::new() };
        Ok(Submitted { ack: Ack { character: id, accepted, rejected, update }, notify })
    }

    /// A GM edit made at this authority: applied at once and logged with
    /// the GM as author.
    pub fn apply_local(&mut self, engine: &Engine, id: &CharacterId, cmd: Command) -> Result<LocalApplied, Rejected> {
        let env = Envelope::new(cmd, self.seeds.next_u64(), (self.clock)(), self.me.to_string());
        self.apply_local_envelope(engine, id, env)
    }

    /// As [`Authority::apply_local`], with an envelope made by the caller.
    pub fn apply_local_envelope(&mut self, engine: &Engine, id: &CharacterId, env: Envelope) -> Result<LocalApplied, Rejected> {
        if !self.chars.contains_key(id) {
            return Err(Rejected::new(format!("no character {id} in this campaign")));
        }
        self.next_seq += 1;
        let op = Op { id: OpId { origin: self.origin, seq: self.next_seq }, env };
        match self.run_op(engine, id, self.me, &op, false) {
            Outcome::Accepted(accepted) => {
                let notify = if accepted.changed { self.interested(id) } else { Vec::new() };
                Ok(LocalApplied { accepted, notify })
            }
            Outcome::Rejected(r) => Err(Rejected { reason: r.reason, confirm: r.confirm }),
        }
    }

    /// Which log entries reverting `version` takes back: the entry and the
    /// earlier ones of its burst (a text box or spinner edited in one go,
    /// which the feed shows as one line). `None` when `version` is not in
    /// the log window.
    pub fn revert_range(&self, id: &CharacterId, version: u64) -> Option<std::ops::RangeInclusive<u64>> {
        let c = self.chars.get(id)?;
        let base = c.window_base();
        if version <= base || version > c.version {
            return None;
        }
        let at = |v: u64| &c.log[(v - base - 1) as usize].entry;
        let mut first = version;
        while first > base + 1 && feed::coalesces(at(first - 1), at(first)) {
            first -= 1;
        }
        Some(first..=version)
    }

    /// Whether the change that made `version` can still be reverted.
    pub fn can_revert(&self, id: &CharacterId, version: u64) -> bool {
        self.revert_range(id, version).is_some()
    }

    /// The GM reverts the change that made `version` (with the rest of its
    /// burst, [`Authority::revert_range`]): the state before it is rebuilt
    /// from the log window's base, every later entry is applied again on
    /// top (a rebase; entries that no longer apply are dropped and named),
    /// and the result becomes a new version through a
    /// [`Command::Revert`] logged with the GM as author. Versions only go
    /// forward, so replicas take it like any other entry.
    pub fn revert(&mut self, engine: &Engine, id: &CharacterId, version: u64) -> Result<Reverted, String> {
        let range = self.revert_range(id, version).ok_or_else(|| format!("this change is too old to revert (only the last {LOG_WINDOW} changes of a character can be)"))?;
        let c = self.chars.get(id).expect("checked");
        let base = c.window_base();
        let entries: Vec<&Entry> = c.log.iter().map(|i| &i.entry).collect();
        // Which entries are taken back: this range, and those of earlier
        // reverts that are still in force (newest first, so reverting a
        // revert brings its entries back).
        let mut excluded: std::collections::BTreeSet<u64> = range.clone().collect();
        for e in entries.iter().rev() {
            if let Command::Revert { from, to, .. } = &e.env.cmd {
                if !excluded.contains(&e.version) && *from > base {
                    excluded.extend(*from..=*to);
                }
            }
        }
        // Rebuild from the window's base without them. A revert reaching
        // behind the base is kept as it is (its state is all we have).
        let mut state = c.base.clone();
        let mut dropped = Vec::new();
        for e in &entries {
            let skip = excluded.contains(&e.version) || matches!(&e.env.cmd, Command::Revert { from, .. } if *from > base);
            if skip {
                continue;
            }
            if let Err(r) = command::apply(&mut state, engine, &e.env) {
                dropped.push(format!("{} ({})", feed::text(&e.env, &e.description), r.reason));
            }
        }
        let last = entries[(*range.end() - base - 1) as usize];
        let mut what = match &last.env.cmd {
            Command::Revert { what, .. } => format!("the revert of {what}"),
            _ => feed::text(&last.env, &last.description),
        };
        if !dropped.is_empty() {
            what = format!("{what}, and {} later change{} that needed it", dropped.len(), if dropped.len() == 1 { "" } else { "s" });
        }
        let reverted = range;
        let cmd = Command::Revert { snapshot: command::snapshot(&state), what, from: *reverted.start(), to: *reverted.end() };
        let applied = self.apply_local(engine, id, cmd).map_err(|r| r.reason)?;
        if !applied.accepted.changed {
            return Err("reverting it changes nothing (a later change already undid it)".into());
        }
        Ok(Reverted { applied, reverted, dropped })
    }

    /// The log entries made since the last call, oldest first (what
    /// [`crate::journal`] keeps until the next save).
    pub fn take_applied(&mut self) -> Vec<(CharacterId, Entry)> {
        std::mem::take(&mut self.applied)
    }

    /// Applies a journal entry again after a restart: `Ok(true)` when it
    /// was the next change of its character, `Ok(false)` when the saved
    /// state already has it (or the character is gone), `Err` when it
    /// does not fit (a gap, or it no longer applies the same way).
    pub fn replay(&mut self, engine: &Engine, id: &CharacterId, entry: &Entry) -> Result<bool, String> {
        let Some(c) = self.chars.get(id) else { return Ok(false) };
        if entry.version <= c.version {
            return Ok(false);
        }
        if entry.version != c.version + 1 {
            return Err(format!("{id}: the journal has version {} but the saved state is at {}", entry.version, c.version));
        }
        if entry.op.origin == self.origin {
            self.next_seq = self.next_seq.max(entry.op.seq);
        }
        let op = Op { id: entry.op, env: entry.env.clone() };
        match self.run_op(engine, id, entry.author, &op, false) {
            Outcome::Accepted(a) if a.changed && a.version == entry.version => Ok(true),
            other => Err(format!("{id}: version {} did not apply again ({other:?})", entry.version)),
        }
    }

    // ----- what members are sent -----

    /// A member joined (or rejoined) with what they have. Returns the
    /// membership and a push for every visible character they are behind
    /// on, and counts both as delivered.
    pub fn join(&mut self, peer: EndpointId, name: &str, have: &[Have]) -> Result<(Membership, Vec<Push>), String> {
        if !name.trim().is_empty() && peer != self.me {
            if let Some(m) = self.members.get_mut(&peer) {
                if m.name != name.trim() {
                    m.name = name.trim().to_owned();
                    self.membership_rev += 1;
                }
            }
        }
        let membership = self.membership(&peer).ok_or("not a member of this campaign")?;
        self.membership_sent.insert(peer, self.membership_rev);
        let mut pushes = Vec::new();
        for id in self.visible(&peer) {
            let c = self.chars.get_mut(&id).expect("visible");
            if let Some(h) = have.iter().find(|h| h.character == id) {
                c.catch_up(&id, h.version);
            }
            let push = match have.iter().find(|h| h.character == id) {
                Some(h) if h.version == c.version && h.hash == c.hash => None,
                Some(h) => Some(c.push_from(&id, h.version, Some(h.hash), false)),
                None => Some(c.push_from(&id, 0, None, true)),
            };
            let v = c.version;
            if let Some(p) = push {
                pushes.push(p);
            }
            self.mark_delivered(peer, &id, v);
        }
        Ok((membership, pushes))
    }

    /// A snapshot for a client whose copy drifted.
    pub fn resync(&mut self, peer: EndpointId, req: &ResyncRequest) -> Result<Push, String> {
        self.check_access(&peer, &req.character)?;
        let c = self.chars.get_mut(&req.character).expect("checked");
        c.catch_up(&req.character, req.have_version);
        let push = c.push_from(&req.character, req.have_version, None, true);
        let v = c.version;
        self.mark_delivered(peer, &req.character, v);
        Ok(push)
    }

    /// The push that brings `peer` up to date on `id` from what it was
    /// last sent, if it is behind. Call [`Authority::mark_delivered`] once
    /// it was sent.
    pub fn push_for(&mut self, peer: &EndpointId, id: &CharacterId) -> Option<Push> {
        if !self.can_see(peer, id) {
            return None;
        }
        let have = self.delivered.get(&(*peer, id.clone())).copied();
        let c = self.chars.get_mut(id)?;
        match have {
            Some(v) if v >= c.version => None,
            Some(v) => Some(c.push_from(id, v, None, false)),
            None => Some(c.push_from(id, 0, None, true)),
        }
    }

    pub fn mark_delivered(&mut self, peer: EndpointId, id: &CharacterId, version: u64) {
        let e = self.delivered.entry((peer, id.clone())).or_insert(0);
        *e = (*e).max(version);
    }

    /// Whether `peer` has not been told the latest membership.
    pub fn membership_stale(&self, peer: &EndpointId) -> bool {
        self.membership_sent.get(peer).is_none_or(|v| *v < self.membership_rev)
    }

    pub fn mark_membership_sent(&mut self, peer: EndpointId) {
        self.membership_sent.insert(peer, self.membership_rev);
    }

    /// Everything `peer` has not been sent yet: the membership when it
    /// changed, answers to their mailed requests (taken out of the queue),
    /// and pushes. Call [`Authority::mark_sent`] for each message that was
    /// delivered and [`Authority::requeue`] for each that was not.
    pub fn outgoing_for(&mut self, peer: &EndpointId) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        if self.role(peer).is_none() || *peer == self.me {
            return out;
        }
        if self.membership_stale(peer) {
            if let Some(m) = self.membership(peer) {
                out.push(ServerMessage::Membership(m));
            }
        }
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.mail_out).into_iter().partition(|(p, _)| p == peer);
        self.mail_out = rest;
        out.extend(mine.into_iter().map(|(_, m)| m));
        for id in self.visible(peer) {
            if let Some(p) = self.push_for(peer, &id) {
                out.push(ServerMessage::Push(p));
            }
        }
        out
    }

    /// Whether [`Authority::outgoing_for`] would return anything.
    pub fn has_outgoing(&self, peer: &EndpointId) -> bool {
        if self.role(peer).is_none() || *peer == self.me {
            return false;
        }
        self.membership_stale(peer)
            || self.mail_out.iter().any(|(p, _)| p == peer)
            || self.visible(peer).iter().any(|id| self.delivered.get(&(*peer, id.clone())).is_none_or(|v| *v < self.chars[id].version))
    }

    /// `msg` reached `peer` (live or by mail): count what it carried as
    /// delivered.
    pub fn mark_sent(&mut self, peer: EndpointId, msg: &ServerMessage) {
        match msg {
            ServerMessage::Membership(_) | ServerMessage::Joined { .. } => self.mark_membership_sent(peer),
            ServerMessage::Push(p) => self.mark_delivered(peer, &p.character, p.version),
            ServerMessage::Ack(a) => self.mark_delivered(peer, &a.character, a.update.version),
            ServerMessage::Error(_) | ServerMessage::Denied(_) => {}
        }
    }

    /// `msg` from [`Authority::outgoing_for`] could not be delivered.
    /// Answers go back in the queue; pushes and memberships are made
    /// again next time anyway.
    pub fn requeue(&mut self, peer: EndpointId, msg: ServerMessage) {
        if matches!(msg, ServerMessage::Ack(_) | ServerMessage::Error(_) | ServerMessage::Denied(_)) {
            self.mail_out.push((peer, msg));
        }
    }

    /// Queues an answer to a mailed request for the mailbox.
    pub fn queue_mail(&mut self, peer: EndpointId, msg: ServerMessage) {
        self.mail_out.push((peer, msg));
    }

    /// Forget what `peer` was sent of `id`, so the next push is a
    /// snapshot (a mailed resync request).
    pub fn forget_delivered(&mut self, peer: &EndpointId, id: &CharacterId) {
        self.delivered.remove(&(*peer, id.clone()));
    }

    /// Members (other than this GM) with something not yet sent.
    pub fn members_behind(&self) -> Vec<EndpointId> {
        self.members.keys().copied().filter(|p| self.has_outgoing(p)).collect()
    }

    // ----- snapshots outside the lock -----

    /// The snapshots [`Authority::to_bytes`] would have to make. A host
    /// makes them ([`SnapshotJob::make`]) without holding its lock, then
    /// hands them back with [`Authority::put_snapshots`]: compressing a
    /// character takes up to seconds, and everything else waits for the
    /// lock meanwhile.
    pub fn save_snapshot_work(&self) -> Vec<SnapshotJob> {
        let mut out = Vec::new();
        for (id, c) in &self.chars {
            if c.base_snapshot.is_none() {
                out.push(SnapshotJob { id: id.clone(), base: true, version: c.window_base(), ch: c.base.clone() });
            }
            if !matches!(&c.snapshot, Some((v, _)) if *v == c.version) {
                out.push(SnapshotJob { id: id.clone(), base: false, version: c.version, ch: c.ch.clone() });
            }
        }
        out
    }

    /// The snapshots a push to `peer` would make: of the characters it
    /// sees (or only `only`) that it has no usable base for. `have` is
    /// what it says it has (a join); `None` goes by what it was sent.
    /// `force`: a snapshot in any case (a resync).
    pub fn push_snapshot_work(&self, peer: &EndpointId, only: Option<&CharacterId>, have: Option<&[Have]>, force: bool) -> Vec<SnapshotJob> {
        let mut out = Vec::new();
        for id in self.visible(peer) {
            if only.is_some_and(|o| *o != id) {
                continue;
            }
            let Some(c) = self.chars.get(&id) else { continue };
            if matches!(&c.snapshot, Some((v, _)) if *v == c.version) {
                continue;
            }
            let from = match have {
                Some(have) => have.iter().find(|h| h.character == id).map(|h| (h.version, Some(h.hash))),
                None => self.delivered.get(&(*peer, id.clone())).map(|v| (*v, None)),
            };
            let needs = force
                || match from {
                    None => true,
                    Some((v, h)) => !c.hash_at(v).is_some_and(|x| h.is_none_or(|h| h == x)),
                };
            if needs {
                out.push(SnapshotJob { id: id.clone(), base: false, version: c.version, ch: c.ch.clone() });
            }
        }
        out
    }

    /// Takes snapshots made outside the lock (those of a state that is
    /// gone meanwhile are dropped).
    pub fn put_snapshots(&mut self, made: Vec<MadeSnapshot>) {
        for m in made {
            let Some(c) = self.chars.get_mut(&m.id) else { continue };
            if m.base {
                if c.base_snapshot.is_none() && c.window_base() == m.version {
                    c.base_snapshot = Some(m.bytes);
                }
            } else if c.version == m.version {
                c.snapshot = Some((m.version, m.bytes));
            }
        }
    }

    // ----- persistence -----

    pub fn to_bytes(&mut self) -> Vec<u8> {
        for c in self.chars.values_mut() {
            c.base_snapshot();
        }
        let file = AuthorityFile {
            campaign: self.campaign,
            name: self.name.clone(),
            me: self.me,
            origin: self.origin,
            next_seq: self.next_seq,
            members: self.members.iter().map(|(k, v)| (*k, v.clone())).collect(),
            invites: self.invites.values().cloned().collect(),
            retired: self.retired.iter().copied().collect(),
            key_gen: self.key_gen,
            key_rev: self.key_rev,
            owner_changes: self.owner_changes.iter().cloned().collect(),
            characters: self
                .chars
                .iter()
                .map(|(id, c)| StoredCharacter {
                    id: id.clone(),
                    name: c.name.clone(),
                    owner: c.owner,
                    snapshot: match &c.snapshot {
                        Some((v, b)) if *v == c.version => b.clone(),
                        _ => command::snapshot(&c.ch),
                    },
                    version: c.version,
                    hash: c.hash,
                    base_hash: c.base_hash,
                    base: c.base_snapshot.clone().expect("made above"),
                    log: c.log.iter().cloned().collect(),
                    seen: c.seen.order.iter().filter_map(|id| c.seen.map.get(id).map(|o| (*id, o.clone()))).collect(),
                })
                .collect(),
            delivered: self.delivered.iter().map(|((p, c), v)| (*p, c.clone(), *v)).collect(),
            membership_rev: self.membership_rev,
            membership_sent: self.membership_sent.iter().map(|(k, v)| (*k, *v)).collect(),
            feed: self.feed.iter().cloned().collect(),
            inbox: self.inbox.clone(),
            mail_out: self.mail_out.clone(),
        };
        persist::to_bytes(MAGIC, FORMAT, &file)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Authority, PersistError> {
        let f: AuthorityFile = persist::from_bytes(MAGIC, "campaign authority", FORMAT, bytes)?;
        let mut chars = BTreeMap::new();
        for s in f.characters {
            let ch = command::restore(&s.snapshot)?;
            let base = command::restore(&s.base)?;
            let mut seen = Seen::default();
            for (id, o) in s.seen {
                seen.insert(id, o);
            }
            chars.insert(
                s.id,
                CharState {
                    name: s.name,
                    owner: s.owner,
                    ch,
                    version: s.version,
                    hash: s.hash,
                    log: s.log.into(),
                    base_hash: s.base_hash,
                    base,
                    base_snapshot: Some(s.base),
                    seen,
                    snapshot: Some((s.version, s.snapshot)),
                },
            );
        }
        Ok(Authority {
            campaign: f.campaign,
            name: f.name,
            me: f.me,
            origin: f.origin,
            next_seq: f.next_seq,
            seeds: Rng::from_time(),
            clock: now_ms,
            members: f.members.into_iter().collect(),
            invites: f.invites.into_iter().map(|i| (i.id, i)).collect(),
            retired: f.retired.into_iter().collect(),
            key_gen: f.key_gen,
            key_rev: f.key_rev,
            gm_keys: Vec::new(),
            owner_changes: f.owner_changes.into_iter().collect(),
            chars,
            delivered: f.delivered.into_iter().map(|(p, c, v)| ((p, c), v)).collect(),
            membership_rev: f.membership_rev,
            membership_sent: f.membership_sent.into_iter().collect(),
            feed: f.feed.into(),
            inbox: f.inbox,
            mail_out: f.mail_out,
            applied: Vec::new(),
        })
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        persist::write_atomic(path, &self.to_bytes())
    }

    pub fn load(path: &Path) -> Result<Authority, PersistError> {
        Authority::from_bytes(&std::fs::read(path)?)
    }
}

/// What a refused command tried to do, for the feed.
pub(crate) fn describe_intent(env: &Envelope) -> String {
    let dbg = format!("{:?}", env.cmd);
    let name = dbg.split([' ', '{', '(']).next().unwrap_or_default();
    feed::text(env, name)
}

const MAGIC: &[u8; 4] = b"CRSA";
/// 2: the log window's base state is stored (for reverts).
/// 3: per-player invites with member keys, retired nodes, the campaign
/// key generation.
const FORMAT: u16 = 3;

/// The authority file: everything above, characters as snapshots.
#[derive(Serialize, Deserialize)]
struct AuthorityFile {
    campaign: CampaignId,
    name: String,
    me: EndpointId,
    origin: [u8; 16],
    next_seq: u64,
    members: Vec<(EndpointId, Member)>,
    invites: Vec<Invite>,
    retired: Vec<EndpointId>,
    key_gen: u32,
    key_rev: u64,
    owner_changes: Vec<CharacterId>,
    characters: Vec<StoredCharacter>,
    delivered: Vec<(EndpointId, CharacterId, u64)>,
    membership_rev: u64,
    membership_sent: Vec<(EndpointId, u64)>,
    feed: Vec<FeedEntry>,
    inbox: Inbox,
    mail_out: Vec<(EndpointId, ServerMessage)>,
}

#[derive(Serialize, Deserialize)]
struct StoredCharacter {
    id: CharacterId,
    name: String,
    owner: Option<EndpointId>,
    snapshot: Vec<u8>,
    version: u64,
    hash: Hash,
    base_hash: Hash,
    /// The state at the log window's base, as a snapshot.
    base: Vec<u8>,
    log: Vec<LogItem>,
    seen: Vec<(OpId, Outcome)>,
}
