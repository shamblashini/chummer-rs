//! A local campaign: the GM's players, NPCs, critters and enemies, their
//! encounters (initiative and damage), and the activity feed.
//!
//! The format is chummer-rs's own (Chummer has no campaign file). A
//! `.chummercampaign` file is one LZMA stream (the `.chum5lz` container,
//! [`crate::chum5lz`]) holding one JSON document:
//!
//! ```text
//! { "format": "chummer-rs campaign", "version": 1,
//!   "id": "<32 hex>", "name": "...", "created": "2026-10-06T12:00:00",
//!   "gm_notes": "...",
//!   "members": [ { "id": "<32 hex>", "kind": "Player", "name": "...",
//!                  "character": { "storage": "embedded", "xml": "<character>…" }
//!                               | { "storage": "linked", "path": "runners/ghost.chum5" },
//!                  "player": "...", "owner": null, "group": "...", "notes": "...",
//!                  "visible_to_players": false } ],
//!   "encounters": [ ... ], "log": [ { "at": 1759750000000, "author": "GM",
//!                  "member": "<32 hex>", "description": "..." } ] }
//! ```
//!
//! An embedded character is its canonical XML ([`crate::command::canonical`]),
//! so loading it back gives the same [`crate::command::state_hash`]; the
//! whole file is compressed once. A linked character stays in its own
//! `.chum5`/`.chum5lz`; a relative path is resolved against the campaign
//! file's folder. Every field has a default and unknown fields are
//! ignored, so older and newer files load; only a wrong `format` fails. A
//! file that is plain JSON (not compressed) also loads, for debugging.
//!
//! Ids are 128 random bits written as 32 lower-case hex digits, the form
//! `chummer_net::invite::CampaignId` uses, so the two convert through
//! `to_string()` / `parse()`.

pub mod damage;
pub mod initiative;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::character::{Character, LoadError};
use crate::command::{self, Command, Envelope};
use crate::engine::Engine;
use crate::xml::{Element, Node};

pub use initiative::{Combatant, Encounter, InitStats};

/// The `format` value of a campaign file.
pub const FORMAT: &str = "chummer-rs campaign";
/// The schema version this build writes.
pub const VERSION: u32 = 1;
/// File extension, without the dot.
pub const EXTENSION: &str = "chummercampaign";

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

/// 128 bits from the process's randomly keyed hasher, the clock and a
/// counter. Not for secrets (invite tokens come from chummer-net's OS
/// random source), but unique enough for ids.
fn random16() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut out = [0u8; 16];
    for (i, half) in out.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(t);
        h.write_u64(n);
        h.write_usize(i);
        let mut rng = crate::dice::Rng::from_time();
        h.write_u64(rng.next_u64());
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    out
}

fn hex16(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("not a valid {0}: expected 32 hex digits")]
pub struct BadId(&'static str);

macro_rules! id16 {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; 16]);

        impl $name {
            pub fn random() -> Self {
                Self(random16())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex16(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self)
            }
        }

        impl FromStr for $name {
            type Err = BadId;
            fn from_str(s: &str) -> Result<Self, BadId> {
                unhex16(s).map(Self).ok_or(BadId(stringify!($name)))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s: std::borrow::Cow<'de, str> = Deserialize::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

id16!(
    /// A campaign; the same form as `chummer_net::invite::CampaignId`.
    CampaignId
);
id16!(
    /// A member of a campaign (one character). Stable for the member's
    /// life, so the sync layer can key versions and logs by it.
    MemberId
);
id16!(
    /// A combatant in an encounter.
    CombatantId
);

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// What a member is at the table. Unknown kinds from newer files load as
/// [`MemberKind::Other`] with their name kept.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum MemberKind {
    #[default]
    Player,
    Npc,
    Critter,
    Enemy,
    Spirit,
    Drone,
    Other(String),
}

impl MemberKind {
    /// The kinds the roster groups by, in its order.
    pub const ALL: [MemberKind; 6] = [MemberKind::Player, MemberKind::Npc, MemberKind::Enemy, MemberKind::Critter, MemberKind::Spirit, MemberKind::Drone];

    pub fn as_str(&self) -> &str {
        match self {
            MemberKind::Player => "Player",
            MemberKind::Npc => "NPC",
            MemberKind::Critter => "Critter",
            MemberKind::Enemy => "Enemy",
            MemberKind::Spirit => "Spirit",
            MemberKind::Drone => "Drone",
            MemberKind::Other(s) => s,
        }
    }

    /// The roster's group heading (English; the GUI translates it).
    pub fn plural(&self) -> &str {
        match self {
            MemberKind::Player => "Players",
            MemberKind::Npc => "NPCs",
            MemberKind::Critter => "Critters",
            MemberKind::Enemy => "Enemies",
            MemberKind::Spirit => "Spirits",
            MemberKind::Drone => "Drones",
            MemberKind::Other(s) => s,
        }
    }
}

impl From<String> for MemberKind {
    fn from(s: String) -> MemberKind {
        match s.as_str() {
            "Player" => MemberKind::Player,
            "NPC" | "Npc" => MemberKind::Npc,
            "Critter" => MemberKind::Critter,
            "Enemy" => MemberKind::Enemy,
            "Spirit" => MemberKind::Spirit,
            "Drone" => MemberKind::Drone,
            _ => MemberKind::Other(s),
        }
    }
}

impl From<MemberKind> for String {
    fn from(k: MemberKind) -> String {
        k.as_str().to_owned()
    }
}

/// Where a member's character lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "lowercase")]
pub enum MemberCharacter {
    /// Inside the campaign file, as canonical XML.
    Embedded {
        #[serde(default)]
        xml: String,
    },
    /// In its own `.chum5`/`.chum5lz`.
    Linked {
        #[serde(default)]
        path: PathBuf,
    },
}

impl Default for MemberCharacter {
    fn default() -> Self {
        MemberCharacter::Embedded { xml: String::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Member {
    pub id: MemberId,
    pub kind: MemberKind,
    /// The roster name (kept in step with the character's name).
    pub name: String,
    pub character: MemberCharacter,
    /// Who plays it ("Anna"); empty for the GM's characters.
    pub player: String,
    /// The owning player's node id (hex), for online campaigns.
    pub owner: Option<String>,
    /// Group or faction ("Halloweeners", "Lone Star").
    pub group: String,
    pub notes: String,
    /// For online campaigns: players may see this member.
    pub visible_to_players: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CampaignError {
    #[error("could not read {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("not a chummer-rs campaign file")]
    NotACampaign,
    #[error("the campaign file is damaged: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the member's character could not be loaded: {0}")]
    Character(#[from] LoadError),
    #[error("the member has no character")]
    Empty,
}

impl Member {
    fn new(kind: MemberKind, name: String, character: MemberCharacter) -> Member {
        Member { id: MemberId::random(), kind, name, character, ..Default::default() }
    }

    /// A member whose character is kept in the campaign file.
    pub fn embedded(kind: MemberKind, ch: &Character) -> Member {
        Member::new(kind, ch.display_name(), MemberCharacter::Embedded { xml: command::canonical(ch) })
    }

    /// A member whose character stays in its own file.
    pub fn linked(kind: MemberKind, path: &Path, ch: &Character) -> Member {
        Member::new(kind, ch.display_name(), MemberCharacter::Linked { path: path.to_owned() })
    }

    pub fn is_linked(&self) -> bool {
        matches!(self.character, MemberCharacter::Linked { .. })
    }

    /// The linked file, resolved against the campaign's folder.
    pub fn linked_path(&self, base: Option<&Path>) -> Option<PathBuf> {
        match &self.character {
            MemberCharacter::Linked { path } if path.is_relative() => Some(base.map_or_else(|| path.clone(), |b| b.join(path))),
            MemberCharacter::Linked { path } => Some(path.clone()),
            MemberCharacter::Embedded { .. } => None,
        }
    }

    /// Load the character. `base` is the campaign file's folder.
    pub fn load_character(&self, base: Option<&Path>) -> Result<Character, CampaignError> {
        match &self.character {
            MemberCharacter::Embedded { xml } if xml.is_empty() => Err(CampaignError::Empty),
            MemberCharacter::Embedded { xml } => Ok(Character::from_str(xml)?),
            MemberCharacter::Linked { .. } => {
                let p = self.linked_path(base).expect("linked");
                Ok(Character::load(&p)?)
            }
        }
    }

    /// Keep `ch` as the member's character: an embedded member stores its
    /// canonical form (a linked one is saved to its file by the caller).
    /// The roster name follows the character.
    pub fn store_character(&mut self, ch: &Character) {
        if let MemberCharacter::Embedded { xml } = &mut self.character {
            *xml = command::canonical(ch);
        }
        self.name = ch.display_name();
    }
}

/// One line of the campaign's activity feed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogItem {
    /// Unix time in milliseconds.
    pub at: i64,
    /// Who made the change ("GM", a player's name); empty for the local user.
    pub author: String,
    pub member: Option<MemberId>,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Campaign {
    pub id: CampaignId,
    pub name: String,
    /// When the campaign was made (ISO, UTC).
    pub created: String,
    pub gm_notes: String,
    pub members: Vec<Member>,
    pub encounters: Vec<Encounter>,
    /// The activity feed, oldest first.
    pub log: Vec<LogItem>,
}

/// The file's envelope around a [`Campaign`].
#[derive(Serialize, Deserialize)]
struct FileDoc {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
    #[serde(flatten)]
    campaign: Campaign,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Campaign {
    pub fn new(name: impl Into<String>) -> Campaign {
        Campaign { id: CampaignId::random(), name: name.into(), created: crate::chargen::now_iso(), ..Default::default() }
    }

    pub fn member(&self, id: MemberId) -> Option<&Member> {
        self.members.iter().find(|m| m.id == id)
    }

    pub fn member_mut(&mut self, id: MemberId) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.id == id)
    }

    /// Add a member; returns its id.
    pub fn add(&mut self, m: Member) -> MemberId {
        let id = m.id;
        self.note(None, "", format!("Added {} ({})", m.name, m.kind.as_str()));
        self.members.push(m);
        id
    }

    /// Remove a member, and its combatants from every encounter.
    pub fn remove(&mut self, id: MemberId) -> Option<Member> {
        let i = self.members.iter().position(|m| m.id == id)?;
        let m = self.members.remove(i);
        for e in &mut self.encounters {
            e.combatants.retain(|c| c.member != Some(id));
        }
        self.note(None, "", format!("Removed {}", m.name));
        Some(m)
    }

    /// Add a line to the feed.
    pub fn note(&mut self, member: Option<MemberId>, author: &str, description: impl Into<String>) {
        self.log.push(LogItem { at: now_ms(), author: author.to_owned(), member, description: description.into() });
    }

    /// The JSON document (uncompressed), as the file holds it.
    pub fn to_json(&self) -> String {
        let doc = FileDoc { format: FORMAT.into(), version: VERSION, campaign: self.clone() };
        serde_json::to_string_pretty(&doc).expect("campaigns serialise")
    }

    pub fn from_json(s: &str) -> Result<Campaign, CampaignError> {
        let doc: FileDoc = serde_json::from_str(s)?;
        if doc.format != FORMAT {
            return Err(CampaignError::NotACampaign);
        }
        Ok(doc.campaign)
    }

    /// Write the campaign (compressed) to `path`, through a temporary file
    /// so a failed write leaves the old file.
    pub fn save(&self, path: &Path) -> Result<(), CampaignError> {
        let io = |e| CampaignError::Io(path.to_owned(), e);
        let bytes = crate::chum5lz::compress(self.to_json().as_bytes()).map_err(io)?;
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        std::fs::write(&tmp, bytes).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    pub fn load(path: &Path) -> Result<Campaign, CampaignError> {
        let bytes = std::fs::read(path).map_err(|e| CampaignError::Io(path.to_owned(), e))?;
        Campaign::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Campaign, CampaignError> {
        let text = match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{') => bytes.to_vec(),
            _ => crate::chum5lz::decompress(bytes).map_err(|_| CampaignError::NotACampaign)?,
        };
        let text = String::from_utf8(text).map_err(|_| CampaignError::NotACampaign)?;
        Campaign::from_json(&text)
    }

    /// Members in roster order: by kind ([`MemberKind::ALL`], then other
    /// kinds by name), then as added.
    pub fn grouped(&self) -> Vec<(MemberKind, Vec<&Member>)> {
        let mut kinds: Vec<MemberKind> = MemberKind::ALL.to_vec();
        let mut others: Vec<MemberKind> = self.members.iter().map(|m| m.kind.clone()).filter(|k| !kinds.contains(k)).collect();
        others.sort();
        others.dedup();
        kinds.extend(others);
        kinds
            .into_iter()
            .map(|k| {
                let ms: Vec<&Member> = self.members.iter().filter(|m| m.kind == k).collect();
                (k, ms)
            })
            .filter(|(_, ms)| !ms.is_empty())
            .collect()
    }

    /// The next free number for copies named "`base` N".
    pub fn next_number(&self, base: &str) -> u32 {
        self.members
            .iter()
            .filter_map(|m| m.name.strip_prefix(base)?.strip_prefix(' ')?.parse::<u32>().ok())
            .max()
            .map_or(1, |n| n + 1)
    }

    /// Add `count` copies of `template` (with `ch`, its character), each
    /// with fresh GUIDs and a numbered name ("Halloweener Ganger 3"). The
    /// copies are embedded, also when the template is linked. Returns the
    /// new members' ids.
    pub fn add_copies(&mut self, engine: &Engine, template: &Member, ch: &Character, count: u32) -> Vec<MemberId> {
        let base = strip_number(&template.name).to_owned();
        let first = self.next_number(&base);
        (0..count)
            .map(|i| {
                let name = format!("{base} {}", first + i);
                let copy = named_copy(engine, ch, &name);
                let mut m = Member::embedded(template.kind.clone(), &copy);
                m.player = template.player.clone();
                m.group = template.group.clone();
                m.notes = template.notes.clone();
                m.visible_to_players = template.visible_to_players;
                self.add(m)
            })
            .collect()
    }
}

/// "Halloweener Ganger 3" → "Halloweener Ganger".
fn strip_number(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((head, n)) if !head.is_empty() && n.parse::<u32>().is_ok() => head,
        _ => name,
    }
}

// ---------------------------------------------------------------------------
// Copies
// ---------------------------------------------------------------------------

/// A copy of a character with new GUIDs for all its items, skills,
/// contacts and ledger entries. Every text that equals an old GUID is
/// replaced (parent links, improvement sources, active clips), so the copy
/// is consistent; data ids (`<sourceid>`, `<suid>`, `<id>`) stay. The copy
/// has no file and is marked modified.
pub fn fresh_copy(ch: &Character) -> Character {
    let mut doc = ch.to_document();
    let mut guids: Vec<String> = Vec::new();
    let mut data_ids: std::collections::HashSet<String> = Default::default();
    collect_ids(&doc, &mut guids, &mut data_ids);
    let map: std::collections::HashMap<String, String> = guids
        .into_iter()
        .filter(|g| !data_ids.contains(g) && g.chars().any(|c| c != '0' && c != '-'))
        .map(|g| (g, crate::items::new_guid()))
        .collect();
    remap(&mut doc, &map);
    let mut copy = Character::from_document(doc).expect("a character's own document loads");
    copy.file = None;
    copy.dirty = true;
    copy
}

fn collect_ids(e: &Element, guids: &mut Vec<String>, data: &mut std::collections::HashSet<String>) {
    for c in e.elements() {
        let t = c.text().trim().to_ascii_lowercase();
        if !t.is_empty() {
            match c.name.as_str() {
                "guid" if !guids.contains(&t) => guids.push(t),
                "sourceid" | "suid" | "id" => {
                    data.insert(t);
                }
                _ => {}
            }
        }
        collect_ids(c, guids, data);
    }
}

fn remap(e: &mut Element, map: &std::collections::HashMap<String, String>) {
    for n in &mut e.children {
        match n {
            Node::Text(t) => {
                if let Some(new) = map.get(&t.trim().to_ascii_lowercase()) {
                    *t = new.clone();
                }
            }
            Node::Element(c) => remap(c, map),
            _ => {}
        }
    }
}

/// A fresh copy of `ch` renamed to `name` (the alias when it has one, else
/// the name), through a command as any edit is.
pub fn named_copy(engine: &Engine, ch: &Character, name: &str) -> Character {
    let mut copy = fresh_copy(ch);
    let key = if copy.field("alias").trim().is_empty() { "name" } else { "alias" };
    let seed = random16();
    let env = Envelope::new(Command::SetField { key: key.into(), value: name.into() }, u64::from_le_bytes(seed[..8].try_into().expect("8 bytes")), now_ms(), "GM");
    // Setting a plain field cannot be refused.
    let _ = command::apply(&mut copy, engine, &env);
    copy.dirty = true;
    copy
}

/// A new NPC: a karma-build character of `metatype` with a PACKS kit
/// (given as its XML) applied, as `Add PACKS Kit` would on a new
/// character. Stays in creation mode, so the GM can adjust it.
pub fn kit_npc(engine: &Engine, metatype: &str, kit_xml: &str, name: &str) -> Result<Character, String> {
    let preset = crate::chargen::creation_presets(engine).into_iter().find(|p| p.build_method() == "Karma").ok_or("no Karma build settings preset")?.key();
    let spec = crate::chargen::NewCharacter {
        settings_id: preset,
        metatype: metatype.to_owned(),
        metavariant: None,
        priorities: crate::chargen::Priorities(['E'; 5]),
        talent: String::new(),
        talent_skills: Vec::new(),
        name: name.to_owned(),
    };
    let mut ch = crate::chargen::create(engine, &spec)?;
    let env = Envelope::new(Command::ApplyKit { kit: kit_xml.to_owned() }, u64::from_le_bytes(random16()[..8].try_into().expect("8 bytes")), now_ms(), "GM");
    command::apply(&mut ch, engine, &env).map_err(|e| e.reason)?;
    Ok(ch)
}

// ---------------------------------------------------------------------------
// The activity feed from command logs
// ---------------------------------------------------------------------------

/// What the feed has taken from one member's [`command::Session`] log, so
/// later calls add only what is new.
#[derive(Debug, Clone, Default)]
pub struct FeedCursor {
    /// (seed, coalesce key, feed text) of the log entries seen, in order.
    seen: Vec<(u64, Option<String>, String)>,
}

/// The feed's text for a log entry. GM awards read "GM gave Ghost 100
/// karma: note"; anything else is the command's own description.
pub fn feed_text(entry: &command::LogEntry, member: &str) -> String {
    if let Command::ManualExpense { karma, gain, expense } = &entry.envelope.cmd {
        let who = if entry.envelope.author.is_empty() { "You" } else { entry.envelope.author.as_str() };
        let amount = if *karma { format!("{} karma", crate::improvement::fmt_num(expense.amount)) } else { format!("{}¥", crate::improvement::fmt_num(expense.amount)) };
        let text = if *gain { format!("{who} gave {member} {amount}") } else { format!("{who} took {amount} from {member}") };
        return if expense.reason.is_empty() { text } else { format!("{text}: {}", expense.reason) };
    }
    entry.description.clone()
}

impl Campaign {
    /// Bring the feed up to date with a member's session log: new entries
    /// are added; an entry a burst of typing merged into the last one
    /// replaces the feed's last line for it; entries that were undone add
    /// "Undone: …".
    pub fn absorb(&mut self, member: MemberId, log: &[command::LogEntry], cursor: &mut FeedCursor) {
        let name = self.member(member).map(|m| m.name.clone()).unwrap_or_default();
        let common = cursor.seen.iter().zip(log).take_while(|(s, e)| s.0 == e.envelope.seed).count();
        let mut start = common;
        let gone = &cursor.seen[common..];
        if gone.len() == 1 && log.len() == common + 1 {
            let e = &log[common];
            let key = e.envelope.cmd.coalesce_key();
            if key.is_some() && gone[0].1 == key {
                // Merged edit: rewrite the feed's last line for this member.
                if let Some(item) = self.log.iter_mut().rev().find(|i| i.member == Some(member)) {
                    item.description = feed_text(e, &name);
                    item.at = e.envelope.at;
                    start = common + 1;
                }
            }
        }
        if start == common {
            for (_, _, desc) in gone.iter().rev() {
                self.log.push(LogItem { at: now_ms(), author: String::new(), member: Some(member), description: format!("Undone: {desc}") });
            }
        }
        for e in &log[start..] {
            self.log.push(LogItem { at: e.envelope.at, author: e.envelope.author.clone(), member: Some(member), description: feed_text(e, &name) });
        }
        cursor.seen = log.iter().map(|e| (e.envelope.seed, e.envelope.cmd.coalesce_key(), feed_text(e, &name))).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_hex_and_parse() {
        let a = CampaignId::random();
        let b = CampaignId::random();
        assert_ne!(a, b);
        let s = a.to_string();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(s.parse::<CampaignId>().unwrap(), a);
        assert!("xyz".parse::<MemberId>().is_err());
        assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{s}\""));
    }

    #[test]
    fn unknown_fields_and_kinds_load() {
        let json = r#"{"format":"chummer-rs campaign","version":7,"name":"Seattle","future":{"x":1},
            "members":[{"kind":"Mech","name":"Big","character":{"storage":"embedded","xml":""},"extra":true}]}"#;
        let c = Campaign::from_json(json).unwrap();
        assert_eq!(c.name, "Seattle");
        assert_eq!(c.members[0].kind, MemberKind::Other("Mech".into()));
        let back = Campaign::from_json(&c.to_json()).unwrap();
        assert_eq!(back.members[0].kind.as_str(), "Mech");
        assert!(matches!(Campaign::from_json(r#"{"format":"other"}"#), Err(CampaignError::NotACampaign)));
    }

    #[test]
    fn numbering() {
        assert_eq!(strip_number("Halloweener Ganger 3"), "Halloweener Ganger");
        assert_eq!(strip_number("Ganger"), "Ganger");
        assert_eq!(strip_number("2"), "2");
        let mut c = Campaign::new("x");
        assert_eq!(c.next_number("Ganger"), 1);
        c.members.push(Member { name: "Ganger 4".into(), ..Default::default() });
        c.members.push(Member { name: "Gangers 9".into(), ..Default::default() });
        assert_eq!(c.next_number("Ganger"), 5);
    }
}
