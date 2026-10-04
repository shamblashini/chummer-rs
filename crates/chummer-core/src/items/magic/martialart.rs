//! Martial arts and their techniques (`MartialArt`, `MartialArtTechnique`).

use super::{apply_bonus, commit, data_notes, find_saved, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Build a `<martialarttechnique>` (`MartialArtTechnique.Save`).
pub fn technique_element(rec: Record<'_>, guid: &str) -> Element {
    let mut t = Out::new("martialarttechnique");
    t.put("sourceid", rec.id());
    t.put("guid", guid);
    t.put("name", rec.name());
    t.put("notes", data_notes(rec.el()));
    t.put("source", rec.source());
    t.put("page", rec.page());
    t.0
}

/// Build a `<martialart>` (`MartialArt.Create` + `MartialArt.Save`) with
/// the given techniques.
pub fn element(rec: Record<'_>, guid: &str, techniques: Vec<Element>) -> Element {
    let e = rec.el();
    let mut m = Out::new("martialart");
    m.put("name", rec.name());
    m.put("sourceid", rec.id());
    m.put("guid", guid);
    m.put("source", rec.source());
    m.put("page", rec.page());
    m.put("cost", e.get_i32("cost").unwrap_or(7).to_string());
    m.flag("isquality", e.get_bool("isquality").unwrap_or(false));
    let mut list = Element::new("martialarttechniques");
    for t in techniques {
        list.push(t);
    }
    m.push(list);
    m.put("notes", data_notes(e));
    m.0
}

/// Techniques the art teaches (names from its `<techniques>`).
pub fn technique_names(rec: Record<'_>) -> Vec<String> {
    rec.el().child("techniques").map(|t| t.children_named("technique").map(|x| x.get("name")).collect()).unwrap_or_default()
}

/// A technique record from the `<techniques>` list of martialarts.xml.
fn technique_record<'a>(doc: &'a Element, name: &str) -> Option<Record<'a>> {
    crate::data::find(doc, "techniques", "technique", name)
}

/// Create one technique: element and improvements, sourced to itself
/// (`ImprovementSource.MartialArtTechnique`).
fn create_technique(ch: &Character, store: &DataStore, rec: Record<'_>) -> (Element, crate::bonus::Outcome) {
    let guid = super::super::new_guid();
    let src = source("MartialArtTechnique", &guid, &rec.name(), 1);
    (technique_element(rec, &guid), apply_bonus(ch, store, rec.el().child("bonus"), &src, None))
}

/// Add a martial art, with `technique` as its first technique if given.
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, technique: Option<&str>) -> String {
    let guid = super::super::new_guid();
    let src = source("MartialArt", &guid, &rec.name(), 1);
    let mut out = apply_bonus(ch, store, rec.el().child("bonus"), &src, None);
    let mut techniques = Vec::new();
    if let (Some(t), Ok(doc)) = (technique, store.doc("martialarts.xml")) {
        if let Some(trec) = technique_record(&doc, t) {
            let (te, tout) = create_technique(ch, store, trec);
            techniques.push(te);
            out.improvements.extend(tout.improvements);
            out.added.extend(tout.added);
        }
    }
    commit(ch, "martialarts", element(rec, &guid, techniques), &out);
    guid
}

/// Add a technique to a martial art the character has.
pub fn add_technique(ch: &mut Character, store: &DataStore, art_guid: &str, technique: &str) -> Result<String, String> {
    let doc = store.doc("martialarts.xml").map_err(|e| e.to_string())?;
    let trec = technique_record(&doc, technique).ok_or("unknown technique")?;
    let (te, out) = create_technique(ch, store, trec);
    let guid = te.get("guid");
    let art = super::super::find_by_guid_mut(ch.items_mut("martialarts"), art_guid).ok_or("unknown martial art")?;
    art.child_or_insert("martialarttechniques").push(te);
    super::super::apply_outcome(ch, &out);
    Ok(guid)
}

/// Oracle: rebuild a saved `<martialart>` and its techniques.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("martialarts.xml").ok()?;
    let rec = find_saved(&doc, "martialarts", "martialart", saved)?;
    let mut techniques = Vec::new();
    for t in saved.child("martialarttechniques").into_iter().flat_map(|l| l.children_named("martialarttechnique")) {
        let trec = find_saved(&doc, "techniques", "technique", t)?;
        techniques.push(technique_element(trec, &t.get("guid")));
    }
    Some(element(rec, &saved.get("guid"), techniques))
}
