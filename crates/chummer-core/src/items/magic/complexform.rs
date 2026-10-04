//! Complex forms (`ComplexForm.Create` / `ComplexForm.Save`).

use super::{apply_bonus, commit, data_notes, find_saved, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Build a `<complexform>` (`ComplexForm.Create` + `ComplexForm.Save`).
/// `grade` is the submersion grade it was learned at, -1 when granted.
pub fn element(rec: Record<'_>, guid: &str, extra: &str, grade: i32) -> Element {
    let e = rec.el();
    let mut c = Out::new("complexform");
    c.put("sourceid", rec.id());
    c.put("guid", guid);
    c.put("name", rec.name());
    for k in ["useskill", "target", "duration", "fv"] {
        c.put(k, e.get(k));
    }
    c.put("extra", extra);
    c.put("source", rec.source());
    c.put("page", rec.page());
    c.put("notes", data_notes(e));
    c.put("grade", grade.to_string());
    c.0
}

/// Add a complex form. `extra` is its selection (e.g. the Matrix
/// attribute of "Infusion of [Matrix Attribute]").
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, extra: Option<&str>) -> String {
    let guid = super::super::new_guid();
    let src = source("ComplexForm", &guid, &rec.name(), 1);
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, extra);
    let extra = out.selected.clone().or(extra.map(str::to_owned)).unwrap_or_default();
    commit(ch, store, "complexforms", element(rec, &guid, &extra, 0), &out);
    guid
}

/// Oracle: rebuild a saved `<complexform>`.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("complexforms.xml").ok()?;
    let rec = find_saved(&doc, "complexforms", "complexform", saved)?;
    Some(element(rec, &saved.get("guid"), &saved.get("extra"), saved.get_i32("grade").unwrap_or(0)))
}
