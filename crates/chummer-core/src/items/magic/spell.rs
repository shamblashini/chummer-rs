//! Spells (`Spell.Create` / `Spell.Save`, the `SelectSpell` form).

use super::{apply_bonus, commit, data_notes, find_saved, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// The variants `SelectSpell` offers, plus where the spell came from.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpellOptions {
    pub limited: bool,
    pub extended: bool,
    pub alchemical: bool,
    /// "Free" checkbox: does not count against free spells or karma.
    pub free_bonus: bool,
    pub barehanded_adept: bool,
    /// `ImprovementSource`, normally "Spell".
    pub source: String,
    /// Initiation grade it was learned at; -1 when granted by a bonus.
    pub grade: i32,
}

impl Default for SpellOptions {
    fn default() -> Self {
        SpellOptions { limited: false, extended: false, alchemical: false, free_bonus: false, barehanded_adept: false, source: "Spell".into(), grade: 0 }
    }
}

/// `Spell.HashDescriptors` contains "Extended Area".
fn has_extended_area(descriptors: &str) -> bool {
    descriptors.split(',').any(|d| d.trim().eq_ignore_ascii_case("Extended Area"))
}

/// Build a `<spell>` (`Spell.Create` + `Spell.Save`).
pub fn element(rec: Record<'_>, guid: &str, extra: &str, o: &SpellOptions) -> Element {
    let e = rec.el();
    let descriptors = e.get("descriptor");
    let mut s = Out::new("spell");
    s.put("sourceid", rec.id());
    s.put("guid", guid);
    s.put("name", rec.name());
    s.put("descriptors", descriptors.clone());
    for k in ["category", "type", "range", "damage", "duration", "dv", "useskill"] {
        s.put(k, e.get(k));
    }
    s.flag("limited", o.limited);
    s.flag("extended", o.extended);
    s.flag("customextended", o.extended && !has_extended_area(&descriptors));
    s.flag("alchemical", o.alchemical);
    s.put("source", rec.source());
    s.put("page", rec.page());
    s.put("extra", extra);
    s.put("notes", data_notes(e));
    s.flag("freebonus", o.free_bonus);
    s.flag("barehandedadept", o.barehanded_adept);
    s.put("improvementsource", o.source.clone());
    s.put("grade", o.grade.to_string());
    s.0
}

/// Add a spell the player picked. `extra` answers its bonus selection
/// (e.g. the attribute of "Increase [Attribute]").
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, extra: Option<&str>, o: &SpellOptions) -> String {
    let guid = super::super::new_guid();
    let src = source("Spell", &guid, &rec.name(), 1);
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, extra);
    let extra = out.selected.clone().or(extra.map(str::to_owned)).unwrap_or_default();
    commit(ch, store, "spells", element(rec, &guid, &extra, o), &out);
    guid
}

/// Oracle: rebuild a saved `<spell>` from spells.xml and its saved choices.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("spells.xml").ok()?;
    let rec = find_saved(&doc, "spells", "spell", saved)?;
    let b = |k: &str| saved.get_bool(k).unwrap_or(false);
    let o = SpellOptions {
        limited: b("limited"),
        extended: b("extended"),
        alchemical: b("alchemical"),
        free_bonus: b("freebonus"),
        barehanded_adept: b("barehandedadept") || b("usesunarmed"),
        source: saved.child_text("improvementsource").unwrap_or_else(|| "Spell".into()),
        grade: saved.get_i32("grade").unwrap_or(0),
    };
    let mut e = element(rec, &saved.get("guid"), &saved.get("extra"), &o);
    super::add_legacy_aliases(&mut e, saved, &[("usesunarmed", "barehandedadept")]);
    Some(e)
}

/// Spells the selection dialog offers in `category` (empty = all),
/// hiding `<hide>` records and ", Extended" duplicates like `SelectSpell`.
pub fn offered<'a>(doc: &'a Element, category: &str) -> Vec<Record<'a>> {
    crate::data::records(doc, "spells", "spell")
        .into_iter()
        .filter(|r| !r.hidden())
        .filter(|r| category.is_empty() || r.category() == category)
        .filter(|r| !r.name().contains(", Extended"))
        .collect()
}
