//! Critter powers (`CritterPower.Create` / `CritterPower.Save`).

use super::{apply_bonus, commit, data_notes, find_saved, inner_copy, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::improvement::fmt_num;
use crate::xml::Element;

/// Per-instance state of a critter power.
#[derive(Debug, Clone)]
pub struct CritterState {
    pub rating: i32,
    pub extra: String,
    /// -1 when granted by a bonus.
    pub grade: i32,
    /// Free spirit power points spent on it.
    pub points: f64,
    /// False for powers that come with the metatype.
    pub count_towards_limit: bool,
}

impl Default for CritterState {
    fn default() -> Self {
        CritterState { rating: 0, extra: String::new(), grade: 0, points: 0.0, count_towards_limit: true }
    }
}

/// `CritterPower.Create`: the extra is the selection, else the rating,
/// else the forced value.
pub fn extra_for(rec: Record<'_>, rating: i32, selected: Option<&str>, forced: &str) -> String {
    let has_bonus = rec.el().child("bonus").is_some();
    match selected.filter(|s| !s.is_empty()) {
        Some(s) if has_bonus => s.to_owned(),
        _ if rating != 0 => rating.to_string(),
        _ if has_bonus => String::new(),
        _ => forced.to_owned(),
    }
}

/// Build a `<critterpower>` (`CritterPower.Create` + `CritterPower.Save`).
pub fn element(rec: Record<'_>, guid: &str, st: &CritterState) -> Element {
    let e = rec.el();
    let mut c = Out::new("critterpower");
    c.put("sourceid", rec.id());
    c.put("guid", guid);
    c.put("name", rec.name());
    c.put("extra", st.extra.clone());
    c.put("rating", st.rating.to_string());
    for k in ["category", "type", "action", "range", "duration"] {
        c.put(k, e.get(k));
    }
    c.put("grade", st.grade.to_string());
    c.put("source", rec.source());
    c.put("page", rec.page());
    c.put("karma", e.get_i32("karma").unwrap_or(0).to_string());
    c.put("points", fmt_num(st.points));
    c.flag("counttowardslimit", st.count_towards_limit);
    c.push(inner_copy(e.child("bonus"), "bonus"));
    c.put("notes", data_notes(e));
    c.put("sortorder", "0");
    c.0
}

/// Create a critter power: element and outcome, not yet stored.
pub fn create(ch: &Character, store: &DataStore, rec: Record<'_>, rating: i32, forced: Option<&str>, grade: i32) -> (Element, crate::bonus::Outcome) {
    let guid = super::super::new_guid();
    let src = source("CritterPower", &guid, &rec.name(), rating);
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, forced);
    let extra = extra_for(rec, rating, out.selected.as_deref(), forced.unwrap_or(""));
    let st = CritterState { rating, extra, grade, ..Default::default() };
    (element(rec, &guid, &st), out)
}

/// Add a critter power the player picked.
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, rating: i32, forced: Option<&str>) -> String {
    let (el, out) = create(ch, store, rec, rating, forced, 0);
    let guid = el.get("guid");
    commit(ch, "critterpowers", el, &out);
    guid
}

/// Oracle: rebuild a saved `<critterpower>`.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("critterpowers.xml").ok()?;
    let rec = find_saved(&doc, "powers", "power", saved)?;
    let st = CritterState {
        rating: saved.get_i32("rating").unwrap_or(0),
        extra: saved.get("extra"),
        grade: saved.get_i32("grade").unwrap_or(0),
        points: saved.get_f64("points").unwrap_or(0.0),
        count_towards_limit: saved.get_bool("counttowardslimit").unwrap_or(true),
    };
    Some(element(rec, &saved.get("guid"), &st))
}
