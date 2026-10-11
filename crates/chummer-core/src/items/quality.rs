//! Qualities (`Quality.Create` / `Quality.Save`). This is the worked
//! example for the other item kinds.

use crate::bonus::{self, BonusSource, Choice};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::improvement::bool_str;

use crate::xml::Element;

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
    // Always written, empty when the data has none.
    q.push(e.child("firstlevelbonus").cloned().unwrap_or_else(|| Element::new("firstlevelbonus")));
    q.push(Element::with_text("notes", e.get("notes")));
    q
}

/// Oracle: rebuild a saved `<quality>` from its data record and the saved
/// choices (`qualitysource`, `extra`).
pub fn rebuild(_ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("qualities.xml").ok()?;
    let key = saved.child_text("sourceid").filter(|s| !s.is_empty()).unwrap_or_else(|| saved.get("name"));
    let rec = crate::data::find(&doc, "qualities", "quality", &key)?;
    Some(quality_element(rec, &saved.get("guid"), &saved.get("qualitysource"), &saved.get("extra")))
}

/// Fields the oracle does not compare for qualities.
pub const IGNORE: &[&str] = &[
    // set by the user or by the containing object, not by the data
    "bp", "contributetobp", "contributetolimit", "print", "implemented", "sourcename",
    // legacy names in pre-5.214 saves (`id` was the record id, now `sourceid`)
    "metagenetic", "metagenic", "id",
];

// ---------------------------------------------------------------------------
// Levels
// ---------------------------------------------------------------------------
//
// Chummer has no rating on a quality: each level is one more `<quality>`
// with the same SourceID, Extra, SourceName and Type, and `Quality.Levels`
// counts them. Every copy carries the full karma and applies `<bonus>`;
// `<firstlevelbonus>` applies to the first copy only. The tree shows the
// copies as one node, "Name N".

/// The highest level a quality allows, when it has levels: an integer
/// `<limit>` (in creation `<chargenlimit>` first) and no `<nolevels>`
/// (`UpdateQualityLevelValue`, `SelectQuality`). `None` for `limit` False,
/// a formula, a missing limit or `<nolevels/>`.
pub fn level_limit(rec: &Element, created: bool) -> Option<i32> {
    if rec.child("nolevels").is_some() {
        return None;
    }
    let raw = (!created).then(|| rec.child_text("chargenlimit")).flatten().or_else(|| rec.child_text("limit"))?;
    raw.trim().parse::<i32>().ok().filter(|n| *n > 0)
}

/// What makes two `<quality>` elements levels of one quality (`Levels`).
pub fn level_key(q: &Element) -> (String, String, String, String) {
    let id = q.child_text("sourceid").filter(|s| !s.is_empty()).or_else(|| q.child_text("id")).unwrap_or_else(|| q.get("name"));
    (id.to_lowercase(), q.get("extra"), q.get("sourcename"), q.get("qualitytype"))
}

/// The guids of every level of `q` (itself included), in file order.
pub fn level_guids(ch: &Character, q: &Element) -> Vec<String> {
    let key = level_key(q);
    ch.items("qualities", "quality").into_iter().filter(|o| level_key(o) == key).map(|o| o.get("guid")).collect()
}

/// `Quality.Levels`.
pub fn levels(ch: &Character, q: &Element) -> i32 {
    level_guids(ch, q).len() as i32
}

/// Copies of a data record taken by the player (`SourceID` and an empty
/// `SourceName`, any extra): what `CharacterCreate`'s "Add" subtracts from
/// the chosen level.
pub fn taken(ch: &Character, rec: Record<'_>) -> i32 {
    let id = rec.id().to_lowercase();
    ch.items("qualities", "quality").into_iter().filter(|q| q.get("sourceid").to_lowercase() == id && q.get("sourcename").is_empty()).count() as i32
}

/// Whether the level stepper applies to a quality on the character: it has
/// levels in the data and was not granted by the metatype, the talent or
/// an improvement (`UpdateQualityLevelValue`).
pub fn level_editable(q: &Element, rec: &Element, created: bool) -> Option<i32> {
    if matches!(q.get("qualitysource").as_str(), "Improvement" | "QualityLevelImprovement" | "Metatype" | "Heritage") {
        return None;
    }
    level_limit(rec, created)
}

/// The level a quality's name shows (`Quality.DisplayName`): the count when
/// above 1, else 1 when the data gives it levels; `None` otherwise.
pub fn shown_level(ch: &Character, store: &DataStore, q: &Element) -> Option<i32> {
    let n = levels(ch, q);
    if n > 1 {
        return Some(n);
    }
    let doc = store.doc("qualities.xml").ok()?;
    let key = q.child_text("sourceid").filter(|s| !s.is_empty()).unwrap_or_else(|| q.get("name"));
    let rec = crate::data::find(&doc, "qualities", "quality", &key)?;
    level_limit(rec.el(), ch.created).map(|_| n)
}
