//! Adding items from game data to a character.
//!
//! Each kind of item has its own save format in Chummer5a; this module
//! builds those elements from data records and runs their bonuses.

use crate::bonus::{self, BonusSource, Choice, Outcome};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::improvement::bool_str;
use crate::xml::Element;

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

/// The result of adding an item.
#[derive(Debug, Clone)]
pub struct Added {
    pub guid: String,
    pub outcome: Outcome,
}

/// Choices needed before adding a quality.
pub fn quality_choices(ch: &Character, store: &DataStore, rec: Record<'_>) -> Vec<Choice> {
    let src = BonusSource { kind: "Quality".into(), guid: String::new(), name: rec.name(), rating: 1 };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Build a `<quality>` element from its data record (`Quality.Create` +
/// `Quality.Save`).
pub fn quality_element(rec: Record<'_>, guid: &str, source: &str, extra: &str) -> Element {
    let e = rec.el();
    let flag = |k: &str, default: bool| bool_str(e.get_bool(k).unwrap_or(default));
    let mut q = Element::new("quality");
    let mut put = |k: &str, v: String| q.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("extra", extra.to_owned());
    put("bp", e.get_i32("karma").unwrap_or(0).to_string());
    put("implemented", flag("implemented", true));
    put("contributetobp", flag("contributetobp", true));
    put("contributetolimit", flag("contributetolimit", true));
    put("stagedpurchase", flag("stagedpurchase", false));
    put("doublecareer", flag("doublecareer", true));
    put("canbuywithspellpoints", flag("canbuywithspellpoints", false));
    put("metagenic", bool_str(e.get_bool("metagenic").or_else(|| e.get_bool("metagenetic")).unwrap_or(false)));
    put("print", flag("print", true));
    put("qualitytype", rec.category());
    put("qualitysource", source.to_owned());
    put("mutant", bool_str(e.child("mutant").is_some()));
    put("source", rec.source());
    put("page", rec.page());
    put("sourcename", String::new());
    let mut bonus = e.child("bonus").cloned().unwrap_or_else(|| Element::new("bonus"));
    bonus.name = "bonus".into();
    q.push(bonus);
    if let Some(f) = e.child("firstlevelbonus") {
        q.push(f.clone());
    }
    q.push(Element::with_text("notes", e.get("notes")));
    q
}

/// Add a quality, applying its bonus. `answer` resolves any selection.
pub fn add_quality(ch: &mut Character, store: &DataStore, rec: Record<'_>, answer: Option<&str>) -> Added {
    let guid = new_guid();
    let src = BonusSource { kind: "Quality".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
    let outcome = match rec.el().child("bonus") {
        Some(b) => bonus::apply(ch, store, b, &src, answer),
        None => Outcome::default(),
    };
    let extra = outcome.selected.clone().unwrap_or_default();
    let el = quality_element(rec, &guid, "Selected", &extra);
    ch.items_mut("qualities").push(el);
    apply_outcome(ch, &outcome);
    Added { guid, outcome }
}

/// Store improvements and flag changes from a bonus.
pub fn apply_outcome(ch: &mut Character, outcome: &Outcome) {
    ch.improvements.list.extend(outcome.improvements.iter().cloned());
    for (k, v) in &outcome.flags {
        ch.set_field(k, v.clone());
    }
    ch.dirty = true;
}
