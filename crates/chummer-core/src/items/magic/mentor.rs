//! Mentor spirits and paragons (`MentorSpirit.Create` / `MentorSpirit.Save`),
//! including the bonuses of the two chosen `<choices>`.

use super::{inner_copy, source, Out};
use crate::bonus::{self, Outcome};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Data file for `mentortype` "MentorSpirit" or "Paragon".
pub fn data_file(mentor_type: &str) -> &'static str {
    if mentor_type == "Paragon" { "paragons.xml" } else { "mentors.xml" }
}

/// A picked choice: its bonus and the value it selected (`ExtraChoiceN`).
#[derive(Debug, Clone, Default)]
pub struct Picked {
    pub bonus: Option<Element>,
    pub extra: String,
}

/// `choices/choice[name = ...]/bonus` of a mentor record.
pub fn choice_bonus<'a>(rec: Record<'a>, choice: &str) -> Option<&'a Element> {
    rec.el().child("choices")?.children_named("choice").find(|c| c.get("name") == choice)?.child("bonus")
}

/// Names of the choices a mentor offers.
pub fn choice_names(rec: Record<'_>) -> Vec<String> {
    rec.el().child("choices").map(|c| c.children_named("choice").map(|x| x.get("name")).collect()).unwrap_or_default()
}

/// Build a `<mentorspirit>` (`MentorSpirit.Create` + `MentorSpirit.Save`).
pub fn element(rec: Record<'_>, guid: &str, mentor_type: &str, extra: &str, c1: &Picked, c2: &Picked, mentor_mask: bool) -> Element {
    let e = rec.el();
    let mut m = Out::new("mentorspirit");
    m.put("sourceid", rec.id());
    m.put("guid", guid);
    m.put("name", rec.name());
    m.put("mentortype", mentor_type);
    m.put("extra", extra);
    m.put("extrachoice1", c1.extra.clone());
    m.put("extrachoice2", c2.extra.clone());
    m.put("source", rec.source());
    m.put("page", rec.page());
    m.put("advantage", e.get("advantage"));
    m.put("disadvantage", e.get("disadvantage"));
    m.flag("mentormask", mentor_mask);
    m.push(inner_copy(e.child("bonus"), "bonus"));
    m.push(inner_copy(c1.bonus.as_ref(), "choice1"));
    m.push(inner_copy(c2.bonus.as_ref(), "choice2"));
    m.put("notes", super::data_notes(e));
    if !rec.id().is_empty() {
        m.put("id", rec.id());
    }
    m.0
}

/// Same element names and texts, ignoring attributes and layout.
fn same_tree(a: &Element, b: &Element) -> bool {
    let ka: Vec<&Element> = a.elements().collect();
    let kb: Vec<&Element> = b.elements().collect();
    a.name == b.name && ka.len() == kb.len() && (!ka.is_empty() || a.text().trim() == b.text().trim()) && ka.iter().zip(&kb).all(|(x, y)| same_tree(x, y))
}

/// The data choice whose bonus a saved `<choice1>`/`<choice2>` holds.
fn match_choice(rec: Record<'_>, saved: Option<&Element>) -> Option<Element> {
    let saved = saved.filter(|s| s.elements().next().is_some())?;
    let mut probe = saved.clone();
    probe.name = "bonus".into();
    rec.el().child("choices")?.children_named("choice").filter_map(|c| c.child("bonus")).find(|b| same_tree(b, &probe)).cloned()
}

/// Oracle: rebuild a saved `<mentorspirit>`. Older saves do not record
/// which choices were picked, so they are found by their bonus content.
pub fn rebuild(_ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let mtype = saved.child_text("mentortype").unwrap_or_else(|| "MentorSpirit".into());
    let doc = store.doc(data_file(&mtype)).ok()?;
    let rec = super::find_saved(&doc, "mentors", "mentor", saved)?;
    let c1 = Picked { bonus: match_choice(rec, saved.child("choice1")), extra: saved.get("extrachoice1") };
    let c2 = Picked { bonus: match_choice(rec, saved.child("choice2")), extra: saved.get("extrachoice2") };
    let mask = saved.get_bool("mentormask").unwrap_or(false);
    Some(element(rec, &saved.get("guid"), &mtype, &saved.get("extra"), &c1, &c2, mask))
}

/// Merge `b` into `a`.
fn merge(a: &mut Outcome, b: Outcome) {
    a.improvements.extend(b.improvements);
    a.flags.extend(b.flags);
    a.unsupported.extend(b.unsupported);
    a.added.extend(b.added);
}

/// The improvements a saved mentor spirit grants: its `<bonus>` and the
/// bonuses of `<choice1>` and `<choice2>`, all with
/// `ImprovementSource.MentorSpirit` and the mentor's guid. `forced`
/// answers selections (the mentor's `<extra>`).
pub fn mentor_outcome(ch: &Character, store: &DataStore, saved: &Element, forced: Option<&str>) -> Outcome {
    let src = source("MentorSpirit", &saved.get("guid"), &saved.get("name"), 1);
    let mut out = Outcome::default();
    for (tag, extra_tag) in [("bonus", "extra"), ("choice1", "extrachoice1"), ("choice2", "extrachoice2")] {
        let Some(node) = saved.child(tag).filter(|n| n.elements().next().is_some()) else { continue };
        let mut b = node.clone();
        b.name = "bonus".into();
        let own = saved.child_text(extra_tag).filter(|s| !s.is_empty());
        let o = bonus::apply(ch, store, &b, &src, own.as_deref().or(forced));
        if tag == "bonus" {
            out.selected = o.selected.clone();
        }
        merge(&mut out, o);
    }
    out
}

/// Apply one choice's bonus and return it with its selected value
/// (`MentorSpirit.Create`: the selection, else the choice name).
fn pick(ch: &Character, store: &DataStore, rec: Record<'_>, choice: Option<&str>, src: &bonus::BonusSource, forced: Option<&str>, out: &mut Outcome) -> Picked {
    let Some(name) = choice.filter(|c| !c.is_empty()) else { return Picked::default() };
    let Some(b) = choice_bonus(rec, name) else { return Picked { bonus: None, extra: forced.unwrap_or(name).to_owned() } };
    let o = bonus::apply(ch, store, b, src, forced);
    let extra = o.selected.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| forced.unwrap_or(name).to_owned());
    merge(out, o);
    Picked { bonus: Some(b.clone()), extra }
}

/// Create a mentor spirit (or paragon) with its two choices and apply all
/// of its bonuses. `mentor_type` is "MentorSpirit" or "Paragon". The
/// `MentorSpirit`/`Paragon` improvement that links it to the granting
/// quality is the caller's (see `selectmentorspirit`).
pub fn add_mentor(ch: &mut Character, store: &DataStore, mentor_type: &str, name: &str, choice1: Option<&str>, choice2: Option<&str>, forced: Option<&str>) -> Result<String, String> {
    let guid = super::super::new_guid();
    let el = {
        let doc = store.doc(data_file(mentor_type)).map_err(|e| e.to_string())?;
        let rec = crate::data::find(&doc, "mentors", "mentor", name).ok_or("unknown mentor")?;
        let (el, out) = create(ch, store, rec, &guid, mentor_type, choice1, choice2, forced);
        super::super::apply_outcome(ch, &out);
        crate::items::place_added(ch, store, &out.added);
        el
    };
    ch.items_mut("mentorspirits").push(el);
    Ok(guid)
}

/// Element and outcome of a new mentor spirit, not yet stored.
#[allow(clippy::too_many_arguments)]
pub fn create(ch: &Character, store: &DataStore, rec: Record<'_>, guid: &str, mentor_type: &str, choice1: Option<&str>, choice2: Option<&str>, forced: Option<&str>) -> (Element, Outcome) {
    let src = source("MentorSpirit", guid, &rec.name(), 1);
    let mut out = super::apply_bonus(ch, store, rec.el().child("bonus"), &src, forced);
    let extra = out.selected.clone().filter(|s| !s.trim().is_empty()).or(forced.map(str::to_owned)).unwrap_or_default();
    let c1 = pick(ch, store, rec, choice1, &src, forced, &mut out);
    let c2 = pick(ch, store, rec, choice2, &src, forced, &mut out);
    (element(rec, guid, mentor_type, &extra, &c1, &c2, false), out)
}

/// Pick the choices of a mentor spirit that already exists (for example
/// one added by a `selectmentorspirit` bonus): rebuilds the element and
/// replaces the improvements it granted.
pub fn set_mentor_choices(ch: &mut Character, store: &DataStore, guid: &str, choice1: Option<&str>, choice2: Option<&str>) -> Result<(), String> {
    let saved = ch.items("mentorspirits", "mentorspirit").into_iter().find(|m| m.get("guid").eq_ignore_ascii_case(guid)).cloned().ok_or("unknown mentor spirit")?;
    let mtype = saved.child_text("mentortype").unwrap_or_else(|| "MentorSpirit".into());
    let doc = store.doc(data_file(&mtype)).map_err(|e| e.to_string())?;
    let rec = super::find_saved(&doc, "mentors", "mentor", &saved).ok_or("unknown mentor")?;
    ch.improvements.remove_from_source(guid);
    let extra = saved.get("extra");
    let (el, out) = create(ch, store, rec, guid, &mtype, choice1, choice2, Some(extra.as_str()).filter(|s| !s.is_empty()));
    if let Some(m) = super::super::find_by_guid_mut(ch.items_mut("mentorspirits"), guid) {
        *m = el;
    }
    super::super::apply_outcome(ch, &out);
    crate::items::place_added(ch, store, &out.added);
    Ok(())
}
