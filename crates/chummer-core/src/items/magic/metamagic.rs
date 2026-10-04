//! Metamagics and echoes (`Metamagic.Create` / `Metamagic.Save`) and arts
//! (`Art`).

use super::{apply_bonus, commit, data_notes, outer_copy, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Data file and record path for `ImprovementSource` "Metamagic" or "Echo".
pub fn data_path(improvement_source: &str) -> (&'static str, &'static str, &'static str) {
    if improvement_source == "Echo" { ("echoes.xml", "echoes", "echo") } else { ("metamagic.xml", "metamagics", "metamagic") }
}

/// Build a `<metamagic>` (`Metamagic.Create` + `Metamagic.Save`).
/// A selection made by its bonus is appended to the name: "Name (Value)".
pub fn element(rec: Record<'_>, guid: &str, improvement_source: &str, selected: Option<&str>, grade: i32, paid_with_karma: bool) -> Element {
    let e = rec.el();
    let name = match selected.filter(|s| !s.is_empty()) {
        Some(s) => format!("{} ({s})", rec.name()),
        None => rec.name(),
    };
    let mut m = Out::new("metamagic");
    m.put("sourceid", rec.id());
    m.put("guid", guid);
    m.put("name", name);
    m.put("source", rec.source());
    m.flag("paidwithkarma", paid_with_karma);
    m.put("page", rec.page());
    m.put("grade", grade.to_string());
    m.push(outer_copy(e.child("bonus")));
    m.put("improvementsource", improvement_source);
    m.put("notes", data_notes(e));
    m.0
}

/// The character's current initiation grade, or submersion grade for
/// technomancers (the rating metamagic bonuses use).
pub fn current_grade(ch: &Character) -> i32 {
    let sub = ch.doc.get_i32("submersiongrade").unwrap_or(0);
    if sub > 0 { sub } else { ch.doc.get_i32("initiategrade").unwrap_or(0) }
}

/// Create a metamagic or echo: element and outcome, not yet stored.
pub fn create(ch: &Character, store: &DataStore, rec: Record<'_>, improvement_source: &str, forced: Option<&str>, grade: i32) -> (Element, crate::bonus::Outcome) {
    let guid = super::super::new_guid();
    let src = source(improvement_source, &guid, &rec.name(), current_grade(ch));
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, forced);
    let el = element(rec, &guid, improvement_source, out.selected.as_deref(), grade, false);
    (el, out)
}

/// Add a metamagic (or an echo, for technomancers) to the lowest
/// initiation/submersion grade that has none yet (one per grade, as the
/// Initiation tab offers). Fails when every grade is filled.
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, forced: Option<&str>) -> Result<String, String> {
    let echo = ch.is_technomancer() && !ch.is_magician();
    let grade = current_grade(ch);
    let taken = ch.items("metamagics", "metamagic").iter().filter(|m| m.get_i32("grade").unwrap_or(0) > 0).count() as i32;
    if taken >= grade {
        return Err(if grade == 0 { "initiate or submerge first".into() } else { "every grade already has a metamagic".into() });
    }
    let src = if echo { "Echo" } else { "Metamagic" };
    let (el, out) = create(ch, store, rec, src, forced, taken + 1);
    let guid = el.get("guid");
    commit(ch, store, "metamagics", el, &out);
    Ok(guid)
}

/// Add a metamagic or echo (`improvement_source`) at a given grade, with
/// no slot check. Returns its guid.
pub fn add_at(ch: &mut Character, store: &DataStore, rec: Record<'_>, improvement_source: &str, forced: Option<&str>, grade: i32) -> String {
    let (el, out) = create(ch, store, rec, improvement_source, forced, grade);
    let guid = el.get("guid");
    commit(ch, store, "metamagics", el, &out);
    guid
}

/// Oracle: rebuild a saved `<metamagic>`. Its name may carry the bonus
/// selection, "Name (Value)".
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let isrc = saved.child_text("improvementsource").unwrap_or_else(|| "Metamagic".into());
    let (file, container, item) = data_path(&isrc);
    let doc = store.doc(file).ok()?;
    let name = saved.get("name");
    let rec = super::find_saved(&doc, container, item, saved).or_else(|| {
        let base = name.rsplit_once(" (").map(|(b, _)| b)?;
        crate::data::find(&doc, container, item, base)
    })?;
    let selected = name.strip_prefix(&format!("{} (", rec.name())).and_then(|s| s.strip_suffix(')'));
    Some(element(rec, &saved.get("guid"), &isrc, selected, saved.get_i32("grade").unwrap_or(0), saved.get_bool("paidwithkarma").unwrap_or(false)))
}

/// Build an `<art>` (`Art.Create` + `Art.Save`).
pub fn art_element(rec: Record<'_>, guid: &str, improvement_source: &str, grade: i32) -> Element {
    let e = rec.el();
    let mut a = Out::new("art");
    a.put("sourceid", rec.id());
    a.put("guid", guid);
    a.put("name", rec.name());
    a.put("source", rec.source());
    a.put("page", rec.page());
    a.put("grade", grade.to_string());
    a.push(outer_copy(e.child("bonus")));
    a.put("improvementsource", improvement_source);
    a.put("notes", data_notes(e));
    a.0
}
