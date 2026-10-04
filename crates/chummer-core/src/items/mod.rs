//! Adding items from game data to a character.
//!
//! Each kind of item has its own save format in Chummer5a. Every kind
//! lives in its own module with the same functions:
//!
//! - `element(ch, store, rec, purchase, guid) -> Result<Element, String>`:
//!   build the saved element from a data record (`<X>.Create` + `<X>.Save`).
//! - `add(ch, store, rec, purchase) -> Result<String, String>`: add it to
//!   the character (bonus, nested items, cost) and return its guid.
//! - `choices(ch, store, rec, purchase) -> Vec<Choice>`: selections the
//!   bonus needs before `add`.
//! - `rebuild(ch, store, saved) -> Option<Element>`: oracle hook. Rebuild a
//!   saved element from its data record plus the choices stored in it.
//! - `IGNORE`: fields the oracle does not compare for this kind.
//!
//! [`quality`] is the worked example.

pub mod armor;
pub mod cyberware;
pub mod drug;
pub mod gear;
pub mod lifestyle;
pub mod magic;
pub mod quality;
pub mod vehicle;
pub mod weapon;

pub use quality::{quality_choices, quality_element};

use crate::bonus::{Choice, Outcome};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// What the buyer chose when adding an item.
#[derive(Debug, Clone, Default)]
pub struct Purchase {
    /// Rating, for items that have one (0 = none).
    pub rating: i32,
    /// Quantity (gear, ammunition). 0 is treated as 1.
    pub qty: f64,
    /// Cyberware/bioware grade name, e.g. "Alphaware".
    pub grade: Option<String>,
    /// Answer to the bonus selection, if any (becomes `<extra>`).
    pub answer: Option<String>,
    /// GUID of the parent item for nested items (gear in gear, mods in
    /// armor, accessories on weapons, ware in cyberlimbs, mods on vehicles).
    pub parent: Option<String>,
    /// Bought for free (no nuyen/karma cost), e.g. granted by a quality.
    pub free: bool,
    /// Black-market discount and similar, as a cost multiplier (1 = none).
    pub cost_multiplier: f64,
}

impl Purchase {
    pub fn qty(&self) -> f64 {
        if self.qty <= 0.0 { 1.0 } else { self.qty }
    }
}

/// An item kind the GUI and the oracle can work with.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    /// Saved element name, e.g. "gear".
    pub tag: &'static str,
    /// Top-level container in the .chum5, e.g. "gears".
    pub container: &'static str,
    /// Data file and its container/item, for the selection dialog.
    pub file: &'static str,
    pub data_container: &'static str,
    pub data_item: &'static str,
    pub label: &'static str,
}

pub const KINDS: &[Kind] = &[
    Kind { tag: "quality", container: "qualities", file: "qualities.xml", data_container: "qualities", data_item: "quality", label: "Quality" },
    Kind { tag: "gear", container: "gears", file: "gear.xml", data_container: "gears", data_item: "gear", label: "Gear" },
    Kind { tag: "cyberware", container: "cyberwares", file: "cyberware.xml", data_container: "cyberwares", data_item: "cyberware", label: "Cyberware" },
    Kind { tag: "bioware", container: "cyberwares", file: "bioware.xml", data_container: "biowares", data_item: "bioware", label: "Bioware" },
    Kind { tag: "armor", container: "armors", file: "armor.xml", data_container: "armors", data_item: "armor", label: "Armor" },
    Kind { tag: "armormod", container: "armors", file: "armor.xml", data_container: "mods", data_item: "mod", label: "Armor mod" },
    Kind { tag: "weapon", container: "weapons", file: "weapons.xml", data_container: "weapons", data_item: "weapon", label: "Weapon" },
    Kind { tag: "accessory", container: "weapons", file: "weapons.xml", data_container: "accessories", data_item: "accessory", label: "Weapon accessory" },
    Kind { tag: "vehicle", container: "vehicles", file: "vehicles.xml", data_container: "vehicles", data_item: "vehicle", label: "Vehicle" },
    Kind { tag: "mod", container: "vehicles", file: "vehicles.xml", data_container: "mods", data_item: "mod", label: "Vehicle mod" },
    Kind { tag: "lifestyle", container: "lifestyles", file: "lifestyles.xml", data_container: "lifestyles", data_item: "lifestyle", label: "Lifestyle" },
    Kind { tag: "drug", container: "drugs", file: "drugcomponents.xml", data_container: "drugs", data_item: "drug", label: "Drug" },
    Kind { tag: "spell", container: "spells", file: "spells.xml", data_container: "spells", data_item: "spell", label: "Spell" },
    Kind { tag: "power", container: "powers", file: "powers.xml", data_container: "powers", data_item: "power", label: "Adept power" },
    Kind { tag: "complexform", container: "complexforms", file: "complexforms.xml", data_container: "complexforms", data_item: "complexform", label: "Complex form" },
    Kind { tag: "spirit", container: "spirits", file: "critters.xml", data_container: "metatypes", data_item: "metatype", label: "Spirit / sprite" },
    Kind { tag: "metamagic", container: "metamagics", file: "metamagic.xml", data_container: "metamagics", data_item: "metamagic", label: "Metamagic" },
    Kind { tag: "martialart", container: "martialarts", file: "martialarts.xml", data_container: "martialarts", data_item: "martialart", label: "Martial art" },
    Kind { tag: "critterpower", container: "critterpowers", file: "critterpowers.xml", data_container: "powers", data_item: "power", label: "Critter power" },
];

pub fn kind(tag: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.tag == tag)
}

/// Oracle dispatch: rebuild a saved element of kind `tag`.
pub fn rebuild(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    match tag {
        "quality" => quality::rebuild(ch, store, saved),
        "gear" => gear::rebuild(ch, store, saved),
        "lifestyle" => lifestyle::rebuild(ch, store, saved),
        "cyberware" => cyberware::rebuild(ch, store, saved),
        "drug" => drug::rebuild(ch, store, saved),
        "armor" | "armormod" => armor::rebuild(tag, ch, store, saved),
        "weapon" | "accessory" => weapon::rebuild(tag, ch, store, saved),
        "vehicle" | "mod" | "weaponmount" => vehicle::rebuild(tag, ch, store, saved),
        "spell" | "power" | "complexform" | "spirit" | "metamagic" | "martialart" | "critterpower" | "mentorspirit" => magic::rebuild(tag, ch, store, saved),
        _ => None,
    }
}

/// Fields the oracle ignores for `tag`, on top of the common list.
pub fn ignored(tag: &str) -> &'static [&'static str] {
    match tag {
        "quality" => quality::IGNORE,
        "gear" => gear::IGNORE,
        "lifestyle" => lifestyle::IGNORE,
        "cyberware" => cyberware::IGNORE,
        "drug" => drug::IGNORE,
        "armor" | "armormod" => armor::IGNORE,
        "weapon" | "accessory" => weapon::IGNORE,
        "vehicle" | "mod" | "weaponmount" => vehicle::IGNORE,
        _ => magic::IGNORE,
    }
}

/// Selections needed before adding a record of kind `tag`.
pub fn choices(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    match tag {
        "quality" => quality_choices(ch, store, rec),
        "gear" => gear::choices(ch, store, rec, p),
        "lifestyle" => lifestyle::choices(ch, store, rec, p),
        "cyberware" | "bioware" => cyberware::choices(tag, ch, store, rec, p),
        "drug" => drug::choices(ch, store, rec, p),
        "armor" | "armormod" => armor::choices(tag, ch, store, rec, p),
        "weapon" | "accessory" => weapon::choices(tag, ch, store, rec, p),
        "vehicle" | "mod" | "weaponmount" => vehicle::choices(tag, ch, store, rec, p),
        _ => magic::choices(tag, ch, store, rec, p),
    }
}

/// Add a record of kind `tag` to the character. Returns the new guid.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    match tag {
        "quality" => Ok(crate::chargen::add_quality(ch, store, rec, p.answer.as_deref())),
        "gear" => gear::add(ch, store, rec, p),
        "lifestyle" => lifestyle::add(ch, store, rec, p),
        "cyberware" | "bioware" => cyberware::add(tag, ch, store, rec, p),
        "drug" => drug::add(ch, store, rec, p),
        "armor" | "armormod" => armor::add(tag, ch, store, rec, p),
        "weapon" | "accessory" => weapon::add(tag, ch, store, rec, p),
        "vehicle" | "mod" | "weaponmount" => vehicle::add(tag, ch, store, rec, p),
        _ => magic::add(tag, ch, store, rec, p),
    }
}

/// Find a saved item anywhere in the document by guid (for parents).
pub fn find_by_guid_mut<'a>(e: &'a mut Element, guid: &str) -> Option<&'a mut Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    for c in e.elements_mut() {
        if let Some(f) = find_by_guid_mut(c, guid) {
            return Some(f);
        }
    }
    None
}

/// A random (v4) GUID, formatted like .NET's `Guid.ToString()`.
pub fn new_guid() -> String {
    let mut rng = crate::dice::Rng::from_time();
    // Mix in a process-wide counter so GUIDs made in the same instant differ.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    for _ in 0..(n % 17) + 1 {
        rng.next_u64();
    }
    let a = rng.next_u64() ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let b = rng.next_u64();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}


/// Store improvements and flag changes from a bonus.
pub fn apply_outcome(ch: &mut Character, outcome: &Outcome) {
    ch.improvements.list.extend(outcome.improvements.iter().cloned());
    for (k, v) in &outcome.flags {
        ch.set_field(k, v.clone());
    }
    ch.dirty = true;
}

/// Put objects a bonus created (`Outcome.added`) where they belong.
/// Most go into their top-level container. Specializations ride along as
/// `skillspecializations` with a `<skill>` name and are attached to that
/// skill; knowsofts go under `newskills/skilljackknowledgeskills`.
pub fn place_added(ch: &mut Character, store: &DataStore, added: &[(String, Element)]) {
    for (container, el) in added {
        match container.as_str() {
            "skillspecializations" => {
                let skill = el.get("skill");
                let id = store
                    .doc("skills.xml")
                    .ok()
                    .and_then(|d| crate::data::find(&d, "skills", "skill", &skill).map(|r| r.id().to_ascii_lowercase()));
                let spec = crate::skills::Specialization {
                    guid: el.get("guid"),
                    name: el.get("name"),
                    free: el.get_bool("free").unwrap_or(false),
                    expertise: el.get_bool("expertise").unwrap_or(false),
                };
                if let Some(s) = ch.skills.iter_mut().find(|s| id.as_deref() == Some(s.suid.to_ascii_lowercase().as_str())) {
                    s.specs.push(spec);
                } else if let Some(k) = ch.knowledge_skills.iter_mut().find(|k| k.name == skill) {
                    k.specs.push(spec);
                }
            }
            "skilljackknowledgeskills" => {
                ch.doc.child_or_insert("newskills").child_or_insert("skilljackknowledgeskills").push(el.clone());
            }
            c => ch.items_mut(c).push(el.clone()),
        }
    }
    ch.dirty = true;
}
