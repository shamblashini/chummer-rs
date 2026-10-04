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

/// Bonus handler hook (see `bonus/handlers.rs`). Return false while unsupported.
pub fn bonus_selectquality(_ctx: &mut crate::bonus::Ctx<'_>, _node: &Element) -> bool {
    false
}

/// Bonus handler hook (see `bonus/handlers.rs`). Return false while unsupported.
pub fn bonus_addcontact(_ctx: &mut crate::bonus::Ctx<'_>, _node: &Element) -> bool {
    false
}
