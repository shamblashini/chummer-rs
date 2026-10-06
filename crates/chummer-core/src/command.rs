//! Commands: every change to a character as a value.
//!
//! A [`Command`] says what the user wants ("raise Pistols", "add this gear
//! at rating 4", "set the notes"), with stable identifiers: item, skill and
//! contact guids, data record ids and names. [`apply`] is the one entry
//! point that changes a [`Character`]; the GUI and the CLI both go through
//! it (the GUI through a [`Session`], which adds undo/redo and the log).
//!
//! Determinism: a command travels in an [`Envelope`] with a seed and a
//! time. While [`apply`] runs, [`crate::items::new_guid`] draws from the
//! seed and [`crate::chargen::now_iso`] returns the envelope's time (see
//! [`crate::dice::deterministic`]), so applying the same envelope to the
//! same state gives a byte-identical character on every machine. Nothing
//! else a command runs reads the clock or a random source, and no rule
//! iterates a `HashMap` into its output (the two `HashSet`s in the bonus
//! code only filter duplicates).
//!
//! A rejected command leaves the character exactly as it was: [`apply`]
//! keeps a copy of the state before and puts it back. [`Applied`] carries
//! that copy, which is what undo restores.
//!
//! [`state_hash`] hashes the canonical form (the saved XML), and
//! [`snapshot`]/[`restore`] pack a whole character for resyncing. See
//! `docs/online-design.md`.

mod describe;
mod run;
mod session;

use serde::{Deserialize, Serialize};

use crate::career::{InitiationOptions, ManualExpense};
use crate::character::{Character, LoadError};
use crate::contacts::ContactType;
use crate::custom_improvement::Form;
use crate::engine::Engine;
use crate::gm::custom_spell::SpellDesign;
use crate::items::lifestyle::Options as LifestyleOptions;
use crate::items::magic::spell::SpellOptions;
use crate::items::Purchase;
use crate::play::ammo::FireMode;

pub use session::{LogEntry, Report, Session, HISTORY_LIMIT};

/// A data record named by its `<id>` (preferred) and `<name>` (fallback,
/// for records without an id).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRef {
    pub id: String,
    pub name: String,
}

impl RecordRef {
    pub fn of(rec: crate::data::Record<'_>) -> RecordRef {
        RecordRef { id: rec.id(), name: rec.name() }
    }
}

/// One improvement of the list: its index and its `<sourcename>`, which
/// must still match when the command runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementRef {
    pub index: u32,
    pub source: String,
}

/// A change to a character. Variants are grouped as the GUI is: fields,
/// creation, career karma, career actions, items, lifestyles, magic,
/// custom improvements, contacts, calendar and play.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    // ----- character fields and settings -----
    /// A plain document field (`Character::set_field`): info fields,
    /// reputation, alias, `nuyenbp`, the long texts.
    SetField { key: String, value: String },
    SetKarma { value: i32 },
    SetNuyen { value: f64 },
    /// "Change Settings File": use another settings preset.
    SwitchSettings { key: String },

    // ----- creation: attributes, skills, build -----
    SetAttributeBase { attribute: String, value: i32 },
    SetAttributeKarma { attribute: String, value: i32 },
    SetSkillBase { skill: String, value: i32 },
    SetSkillKarma { skill: String, value: i32 },
    SetKnowledgeBase { skill: String, value: i32 },
    SetKnowledgeKarma { skill: String, value: i32 },
    SetGroupBase { group: String, value: i32 },
    SetGroupKarma { group: String, value: i32 },
    AddKnowledgeSkill { name: String, kind: String, native: bool },
    RemoveKnowledgeSkill { skill: String },
    AddSpecialization { skill: String, name: String },
    SetTradition { name: String },
    AddLifeModule { module: String, version: Option<String> },
    /// Leave creation for career mode.
    FinishCreation,

    // ----- career karma -----
    RaiseAttribute { attribute: String },
    RaiseSkill { skill: String },
    RaiseSkillGroup { group: String },
    BuySpecialization { skill: String, name: String },
    LearnKnowledgeSkill { name: String, kind: String },
    Initiate { options: InitiationOptions },
    /// The Karma & Nuyen tab's "Undo" on a ledger entry (a rules-level
    /// refund, not the editor's undo).
    UndoExpense { entry: String },
    /// A manual ledger entry: karma or nuyen, gained or spent.
    ManualExpense { karma: bool, gain: bool, expense: ManualExpense },

    // ----- career actions -----
    SpendEdge,
    RegainEdge,
    BurnEdge,
    /// The Edge boxes in the sidebar: this many points spent.
    SetEdgeUsed { used: i32 },
    RefreshEdge,
    BurnStreetCred,
    SetGroupMember { member: bool },

    // ----- items -----
    /// Add a record of kind `tag` (`items::KINDS`) as the selection dialog
    /// does; in career mode karma or nuyen is spent and logged.
    AddItem { tag: String, record: RecordRef, purchase: Purchase },
    /// Remove a top-level item from a section's tree (qualities are bought
    /// off in career mode).
    RemoveItem { container: String, guid: String },
    /// The item pane's Delete: the item, wherever it is nested.
    DeleteItem { guid: String },
    SellItem { guid: String, fraction: f64 },
    SetItemRating { guid: String, rating: i32 },
    SetItemQuantity { guid: String, qty: f64 },
    SetItemEquipped { guid: String, on: bool },
    SetItemWireless { guid: String, on: bool },
    /// A text field of an item: custom name, location, notes.
    SetItemText { guid: String, field: String, value: String },
    /// Create a location and put the item there.
    AddItemLocation { guid: String, name: String },
    AddWeaponMount { vehicle: String, size: String },
    AddCustomDrug { name: String, grade: String, components: Vec<(String, i32)> },
    /// Add a PACKS kit, given as its XML.
    ApplyKit { kit: String },
    RemoveAiProgram { guid: String },

    // ----- lifestyles -----
    UpdateLifestyle { guid: String, options: LifestyleOptions },
    /// Career mode: pay one more month.
    PayLifestyleMonth { guid: String },
    AddLifestyleQuality { lifestyle: String, quality: String, answer: Option<String>, free: bool },
    RemoveLifestyleQuality { lifestyle: String, quality: String },

    // ----- magic, resonance, martial arts -----
    AddSpell { record: RecordRef, answer: Option<String>, options: SpellOptions },
    AddCustomSpell { design: SpellDesign },
    QuickenSpell { spell: String, karma: i32 },
    SetMentorChoices { mentor: String, choice1: Option<String>, choice2: Option<String> },
    /// Pick the mentor spirit (or paragon) a quality grants.
    ChooseMentor { quality: String, mentor_type: String, name: String },
    /// `kind` is "Metamagic" or "Echo"; `grade` is used in career mode.
    AddMetamagic { kind: String, name: String, answer: Option<String>, grade: i32 },
    LearnTechnique { art: String, technique: String },
    BuyPowerPoint,
    SetPowerRating { power: String, rating: i32 },
    SetSpiritState { spirit: String, force: i32, services: i32, bound: bool, fettered: bool },
    BindFocus { gear: String },
    UnbindFocus { gear: String },

    // ----- custom improvements -----
    /// Create, or with `edit` replace, a custom improvement.
    CreateImprovement { form: Form, group: String, edit: Option<String> },
    RemoveImprovement { source: String },
    SetImprovementEnabled { at: ImprovementRef, on: bool },
    SetImprovementNotes { at: ImprovementRef, notes: String },
    SetImprovementGroup { at: ImprovementRef, group: String },
    SetImprovementGroupEnabled { group: String, on: bool },
    AddImprovementGroup { name: String },
    RenameImprovementGroup { old: String, new: String },
    RemoveImprovementGroup { name: String },

    // ----- contacts, enemies, pets -----
    AddContact { kind: ContactType },
    /// "Add from File": a Chummer contacts XML file's text.
    ImportContacts { xml: String },
    SetContactField { contact: String, key: String, value: String },
    /// The notes dialog: new text (`None` when unchanged) and colour.
    SetContactNotes { contact: String, notes: Option<String>, color: [u8; 3] },
    MoveContact { contact: String, target: String, after: bool },
    MoveContactStep { contact: String, up: bool },
    RemoveContact { contact: String },
    /// Link to a save file; `startup` is the directory relative links are
    /// resolved against.
    LinkContact { contact: String, file: String, startup: String },
    UnlinkContact { contact: String },

    // ----- calendar -----
    AddWeek,
    SetWeekNotes { week: String, notes: String },
    RemoveWeek { week: String },

    // ----- play: damage, ammunition, devices -----
    SetPhysicalDamage { filled: i32 },
    SetStunDamage { filled: i32 },
    SetActiveClip { weapon: String, slot: u32 },
    Fire { weapon: String, mode: FireMode },
    SetAmmoRemaining { weapon: String, count: i32 },
    Reload { weapon: String, ammo: Option<String>, count: i32 },
    Unload { weapon: String },
    SetCharges { weapon: String, count: i32 },
    SetVehicleDamage { vehicle: String, filled: i32 },
    SetMatrixDamage { device: String, filled: i32 },
    SetActiveCommlink { device: String, on: bool },
    SetHomeNode { device: String, on: bool },
}

impl Command {
    /// Commands that set a value outright (text boxes, spinners): a later
    /// one with the same key replaces an earlier one, so a [`Session`]
    /// merges a burst of them into one undo step and one log entry.
    pub fn coalesce_key(&self) -> Option<String> {
        use Command::*;
        Some(match self {
            SetField { key, .. } => format!("field:{key}"),
            SetKarma { .. } => "karma".into(),
            SetNuyen { .. } => "nuyen".into(),
            SetAttributeBase { attribute, .. } => format!("attr-base:{attribute}"),
            SetAttributeKarma { attribute, .. } => format!("attr-karma:{attribute}"),
            SetSkillBase { skill, .. } => format!("skill-base:{skill}"),
            SetSkillKarma { skill, .. } => format!("skill-karma:{skill}"),
            SetKnowledgeBase { skill, .. } => format!("kno-base:{skill}"),
            SetKnowledgeKarma { skill, .. } => format!("kno-karma:{skill}"),
            SetGroupBase { group, .. } => format!("group-base:{group}"),
            SetGroupKarma { group, .. } => format!("group-karma:{group}"),
            SetItemQuantity { guid, .. } => format!("qty:{guid}"),
            SetItemText { guid, field, .. } => format!("item-text:{guid}:{field}"),
            UpdateLifestyle { guid, .. } => format!("lifestyle:{guid}"),
            SetPowerRating { power, .. } => format!("power:{power}"),
            SetImprovementNotes { at, .. } => format!("imp-notes:{}", at.source),
            SetContactField { contact, key, .. } => format!("contact:{contact}:{key}"),
            SetWeekNotes { week, .. } => format!("week:{week}"),
            _ => return None,
        })
    }

    /// Examples of every variant, for round-trip tests.
    pub fn examples() -> Vec<Command> {
        use Command::*;
        let s = |v: &str| v.to_owned();
        let g = s("6a4d1c2e-0000-4000-8000-000000000001");
        let at = ImprovementRef { index: 3, source: g.clone() };
        let record = RecordRef { id: g.clone(), name: s("Ares Predator V") };
        vec![
            SetField { key: s("alias"), value: s("Ghost") },
            SetKarma { value: 25 },
            SetNuyen { value: 1234.5 },
            SwitchSettings { key: s("Standard.xml") },
            SetAttributeBase { attribute: s("BOD"), value: 3 },
            SetAttributeKarma { attribute: s("AGI"), value: 1 },
            SetSkillBase { skill: g.clone(), value: 4 },
            SetSkillKarma { skill: g.clone(), value: 1 },
            SetKnowledgeBase { skill: g.clone(), value: 2 },
            SetKnowledgeKarma { skill: g.clone(), value: 1 },
            SetGroupBase { group: s("Firearms"), value: 2 },
            SetGroupKarma { group: s("Firearms"), value: 1 },
            AddKnowledgeSkill { name: s("Seattle Gangs"), kind: s("Street"), native: false },
            RemoveKnowledgeSkill { skill: g.clone() },
            AddSpecialization { skill: g.clone(), name: s("Semi-Automatics") },
            SetTradition { name: s("Hermetic") },
            AddLifeModule { module: g.clone(), version: Some(g.clone()) },
            FinishCreation,
            RaiseAttribute { attribute: s("LOG") },
            RaiseSkill { skill: g.clone() },
            RaiseSkillGroup { group: s("Athletics") },
            BuySpecialization { skill: g.clone(), name: s("Revolvers") },
            LearnKnowledgeSkill { name: s("Matrix Security"), kind: s("Professional") },
            Initiate { options: InitiationOptions { group: true, ordeal: false, schooling: true } },
            UndoExpense { entry: g.clone() },
            ManualExpense { karma: true, gain: true, expense: crate::career::ManualExpense { amount: 5.0, reason: s("Run"), date: None, refund: false, force_career_visible: false, exchange: false } },
            SpendEdge,
            RegainEdge,
            BurnEdge,
            SetEdgeUsed { used: 2 },
            RefreshEdge,
            BurnStreetCred,
            SetGroupMember { member: true },
            AddItem { tag: s("weapon"), record: record.clone(), purchase: Purchase { rating: 2, qty: 1.0, grade: Some(s("Alphaware")), answer: Some(s("Pistols")), parent: Some(g.clone()), free: false, cost_multiplier: 0.9 } },
            RemoveItem { container: s("gears"), guid: g.clone() },
            DeleteItem { guid: g.clone() },
            SellItem { guid: g.clone(), fraction: 0.5 },
            SetItemRating { guid: g.clone(), rating: 3 },
            SetItemQuantity { guid: g.clone(), qty: 10.0 },
            SetItemEquipped { guid: g.clone(), on: false },
            SetItemWireless { guid: g.clone(), on: true },
            SetItemText { guid: g.clone(), field: s("notes"), value: s("Line 1\nLine 2 ¥ «»") },
            AddItemLocation { guid: g.clone(), name: s("Car") },
            AddWeaponMount { vehicle: g.clone(), size: g.clone() },
            AddCustomDrug { name: s("Brew"), grade: s("Standard"), components: vec![(s("Cram"), 1), (s("Jazz"), 2)] },
            ApplyKit { kit: s("<pack><name>Kit</name></pack>") },
            RemoveAiProgram { guid: g.clone() },
            UpdateLifestyle { guid: g.clone(), options: LifestyleOptions { name: s("Home"), months: 2, roommates: 1, percentage: 50.0, area: 1, comforts: 0, security: 2, bonus_lp: 0, trust_fund: false, split_cost_with_roommates: true, style: s("Advanced"), city: s("Seattle"), district: s("Downtown"), borough: String::new() } },
            PayLifestyleMonth { guid: g.clone() },
            AddLifestyleQuality { lifestyle: g.clone(), quality: s("Cramped"), answer: None, free: true },
            RemoveLifestyleQuality { lifestyle: g.clone(), quality: g.clone() },
            AddSpell { record: record.clone(), answer: Some(s("Fire")), options: SpellOptions { limited: true, extended: false, alchemical: false, free_bonus: false, barehanded_adept: false, source: s("Spell"), grade: 0 } },
            AddCustomSpell { design: SpellDesign::default() },
            QuickenSpell { spell: g.clone(), karma: 3 },
            SetMentorChoices { mentor: g.clone(), choice1: Some(s("A")), choice2: None },
            ChooseMentor { quality: g.clone(), mentor_type: s("MentorSpirit"), name: s("Bear") },
            AddMetamagic { kind: s("Metamagic"), name: s("Centering"), answer: None, grade: 1 },
            LearnTechnique { art: g.clone(), technique: s("Called Shot (Disarm)") },
            BuyPowerPoint,
            SetPowerRating { power: g.clone(), rating: 2 },
            SetSpiritState { spirit: g.clone(), force: 5, services: 3, bound: true, fettered: false },
            BindFocus { gear: g.clone() },
            UnbindFocus { gear: g.clone() },
            CreateImprovement { form: Form { type_id: s("Attribute"), name: s("Buff"), val: 1.0, min: 0.0, max: 0.0, aug: 0.0, apply_to_rating: false, free: false, select: s("BOD") }, group: s("Buffs"), edit: None },
            RemoveImprovement { source: g.clone() },
            SetImprovementEnabled { at: at.clone(), on: false },
            SetImprovementNotes { at: at.clone(), notes: s("note") },
            SetImprovementGroup { at, group: s("Buffs") },
            SetImprovementGroupEnabled { group: s("Buffs"), on: true },
            AddImprovementGroup { name: s("Buffs") },
            RenameImprovementGroup { old: s("Buffs"), new: s("Spells") },
            RemoveImprovementGroup { name: s("Spells") },
            AddContact { kind: ContactType::Enemy },
            ImportContacts { xml: s("<contacts />") },
            SetContactField { contact: g.clone(), key: s("loyalty"), value: s("4") },
            SetContactNotes { contact: g.clone(), notes: Some(s("owes me")), color: [10, 20, 30] },
            MoveContact { contact: g.clone(), target: g.clone(), after: true },
            MoveContactStep { contact: g.clone(), up: false },
            RemoveContact { contact: g.clone() },
            LinkContact { contact: g.clone(), file: s("/home/x/fixer.chum5"), startup: s("/opt/chummer") },
            UnlinkContact { contact: g.clone() },
            AddWeek,
            SetWeekNotes { week: g.clone(), notes: s("Run in Redmond") },
            RemoveWeek { week: g.clone() },
            SetPhysicalDamage { filled: 3 },
            SetStunDamage { filled: 0 },
            SetActiveClip { weapon: g.clone(), slot: 2 },
            Fire { weapon: g.clone(), mode: FireMode::ShortBurst },
            SetAmmoRemaining { weapon: g.clone(), count: 0 },
            Reload { weapon: g.clone(), ammo: Some(g.clone()), count: 15 },
            Unload { weapon: g.clone() },
            SetCharges { weapon: g.clone(), count: 6 },
            SetVehicleDamage { vehicle: g.clone(), filled: 4 },
            SetMatrixDamage { device: g.clone(), filled: 1 },
            SetActiveCommlink { device: g.clone(), on: true },
            SetHomeNode { device: g, on: false },
        ]
    }
}

/// A command with what makes it deterministic: the seed new GUIDs come
/// from, the time (Unix milliseconds) ledger dates and calendar entries
/// use, and who made it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub cmd: Command,
    pub seed: u64,
    /// Unix time in milliseconds.
    pub at: i64,
    /// Who made the change; empty for the local user.
    pub author: String,
}

impl Envelope {
    pub fn new(cmd: Command, seed: u64, at: i64, author: impl Into<String>) -> Envelope {
        Envelope { cmd, seed, at, author: author.into() }
    }

    /// The envelope's time as ledger dates write it.
    pub fn at_iso(&self) -> String {
        crate::chargen::iso_from_unix(self.at.div_euclid(1000))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("commands serialise")
    }

    pub fn from_json(s: &str) -> Result<Envelope, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// The compact binary form, for the wire.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("commands serialise")
    }

    pub fn from_bytes(b: &[u8]) -> Result<Envelope, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// A command that ran.
#[derive(Debug, Clone)]
pub struct Applied {
    /// For the history and the activity feed, e.g. "Raised Pistols to 5
    /// (10 karma)".
    pub description: String,
    /// A status line the core produced ("Bought X for 250¥").
    pub message: Option<String>,
    /// How many things a bulk command added (kits, imported contacts).
    pub count: Option<usize>,
    /// False when the command found nothing to change; the character is
    /// then exactly as before and nothing needs recording.
    pub changed: bool,
    /// The character before the command: what undo restores.
    pub before: Box<Character>,
}

/// A command that could not run, with the reason. The character is
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub reason: String,
    /// The reason is a question ("Not enough Ammunition. Treat as Single
    /// Shot?"): the UI may ask it and send a follow-up command.
    pub confirm: bool,
}

impl Rejected {
    pub fn new(reason: impl Into<String>) -> Rejected {
        Rejected { reason: reason.into(), confirm: false }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Rejected {}

/// Apply one command. This is the only way the GUI and the CLI change a
/// character. On success the derived state (essence loss) is refreshed as
/// after any edit and the character is marked modified; on failure it is
/// left exactly as it was.
pub fn apply(ch: &mut Character, engine: &Engine, env: &Envelope) -> Result<Applied, Rejected> {
    let before = ch.clone();
    let _scope = crate::dice::deterministic(env.seed, env.at_iso());
    let essence_before = essence_key(ch, engine);
    match run::run(ch, engine, &env.cmd) {
        Ok(run::Done::Unchanged) => {
            *ch = before.clone();
            Ok(Applied { description: String::new(), message: None, count: None, changed: false, before: Box::new(before) })
        }
        Ok(run::Done::Changed { message, count }) => {
            refresh_derived(ch, engine, essence_before);
            ch.dirty = true;
            let description = describe::describe(&env.cmd, &before, ch, engine);
            Ok(Applied { description, message, count, changed: true, before: Box::new(before) })
        }
        Err(e) => {
            *ch = before;
            Err(e)
        }
    }
}

/// Apply envelopes in order, stopping at the first rejection.
pub fn replay(ch: &mut Character, engine: &Engine, log: &[Envelope]) -> Result<(), (usize, Rejected)> {
    for (i, env) in log.iter().enumerate() {
        apply(ch, engine, env).map_err(|e| (i, e))?;
    }
    Ok(())
}

/// What decides whether career mode refreshes essence loss: Chummer
/// refreshes when the essence or the essence at special start changed.
fn essence_key(ch: &Character, engine: &Engine) -> (f64, Option<f64>) {
    (engine.sheet(ch).essence, crate::essence_loss::essence_at_special_start(ch))
}

/// What the GUI did after every change: creation mode regenerates the
/// essence-loss improvements; career mode does when the essence key moved
/// (it may burn karma levels, so not on every change).
fn refresh_derived(ch: &mut Character, engine: &Engine, essence_before: (f64, Option<f64>)) {
    let store = engine.store_for_character(ch);
    let rules = engine.rules_for(ch);
    if !ch.created || essence_key(ch, engine) != essence_before {
        crate::essence_loss::refresh(ch, &store, &rules);
    }
}

/// The canonical form: the saved XML.
pub fn canonical(ch: &Character) -> String {
    ch.to_xml_string()
}

/// BLAKE3 of the canonical form. Equal hashes mean equal saved files.
pub fn state_hash(ch: &Character) -> [u8; 32] {
    *blake3::hash(canonical(ch).as_bytes()).as_bytes()
}

/// A hash as lowercase hex.
pub fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// The whole character, compressed (LZMA, as `.chum5lz`).
pub fn snapshot(ch: &Character) -> Vec<u8> {
    crate::chum5lz::compress(canonical(ch).as_bytes()).expect("compressing in memory")
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("not a snapshot: {0}")]
    Data(#[from] std::io::Error),
    #[error("snapshot is not UTF-8")]
    Utf8,
    #[error(transparent)]
    Load(#[from] LoadError),
}

/// A character from a [`snapshot`].
pub fn restore(bytes: &[u8]) -> Result<Character, RestoreError> {
    let xml = crate::chum5lz::decompress(bytes)?;
    let xml = String::from_utf8(xml).map_err(|_| RestoreError::Utf8)?;
    Ok(Character::from_str(&xml)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_round_trips() {
        for c in Command::examples() {
            let env = Envelope::new(c, 42, 1_700_000_000_123, "gm");
            let json = env.to_json();
            let back = Envelope::from_json(&json).unwrap();
            assert_eq!(back.to_json(), json);
            let bytes = env.to_bytes();
            let back = Envelope::from_bytes(&bytes).unwrap();
            assert_eq!(back.to_json(), json, "postcard {json}");
        }
    }

    #[test]
    fn examples_cover_every_variant() {
        // A new variant needs an example: count the variant names in the
        // JSON tags against the enum's.
        let names: std::collections::BTreeSet<String> = Command::examples()
            .iter()
            .map(|c| {
                let v = serde_json::to_value(c).unwrap();
                match v {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Object(m) => m.keys().next().unwrap().clone(),
                    _ => unreachable!(),
                }
            })
            .collect();
        let src = include_str!("command.rs");
        let body = &src[src.find("pub enum Command {").unwrap()..src.find("impl Command {").unwrap()];
        let declared = body
            .lines()
            .map(str::trim)
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
            .map(|l| l.split(|c: char| !c.is_alphanumeric()).next().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names, declared);
    }

    #[test]
    fn scoped_guids_and_time_follow_the_seed() {
        let one = {
            let _s = crate::dice::deterministic(7, "2070-01-02T03:04:05".into());
            (crate::items::new_guid(), crate::items::new_guid(), crate::chargen::now_iso())
        };
        let two = {
            let _s = crate::dice::deterministic(7, "2070-01-02T03:04:05".into());
            (crate::items::new_guid(), crate::items::new_guid(), crate::chargen::now_iso())
        };
        assert_eq!(one, two);
        assert_ne!(one.0, one.1);
        assert_eq!(one.2, "2070-01-02T03:04:05");
        // Outside the scope GUIDs are random again.
        assert_ne!(crate::items::new_guid(), one.0);
        let g = &one.0;
        assert_eq!(g.len(), 36);
        assert_eq!(&g[14..15], "4");
    }
}
