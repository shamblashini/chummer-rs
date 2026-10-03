//! `<required>` / `<forbidden>` checks on data records, a port of the
//! common cases of `SelectionShared.RequirementsMet`.
//!
//! `<required>` holds `<allof>` (every child must hold) and `<oneof>` (at
//! least one must hold). `<forbidden><oneof>` fails if any child holds.
//! A `<group>` inside a `<oneof>` holds when all of its children hold.

use crate::calc::Sheet;
use crate::character::Character;
use crate::xml::Element;

pub struct Check<'a> {
    pub ch: &'a Character,
    pub sheet: &'a Sheet,
    /// Name of the record being checked, to skip itself in counts.
    pub ignore_quality: Option<&'a str>,
}

/// Why a record cannot be taken. Empty means allowed.
pub fn unmet(record: &Element, c: &Check<'_>) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(req) = record.child("required") {
        if let Some(all) = req.child("allof") {
            for n in all.elements() {
                if !holds(n, c).unwrap_or(true) {
                    why.push(format!("requires {}", describe(n)));
                }
            }
        }
        for one in req.children_named("oneof") {
            let opts: Vec<&Element> = one.elements().collect();
            if !opts.is_empty() && !opts.iter().any(|n| holds(n, c).unwrap_or(true)) {
                why.push(format!("requires one of: {}", opts.iter().map(|n| describe(n)).collect::<Vec<_>>().join(", ")));
            }
        }
    }
    if let Some(forb) = record.child("forbidden") {
        for one in forb.children_named("oneof").chain(forb.children_named("allof")) {
            for n in one.elements() {
                if holds(n, c).unwrap_or(false) {
                    why.push(format!("not allowed with {}", describe(n)));
                }
            }
        }
    }
    why
}

/// How many more times a quality can be taken (`<limit>`), `None` = no limit.
pub fn remaining_quality_slots(record: &Element, ch: &Character) -> Option<usize> {
    let limit = record.get("limit");
    if limit.eq_ignore_ascii_case("false") {
        return None;
    }
    let max: usize = limit.trim().parse().unwrap_or(1);
    let name = record.get("name");
    let have = ch.items("qualities", "quality").into_iter().filter(|q| q.get("name") == name).count();
    Some(max.saturating_sub(have))
}

fn describe(n: &Element) -> String {
    let t = n.text();
    match n.name.as_str() {
        "skill" => format!("{} {}", n.get("name"), n.get("val")),
        "magenabled" => "Magic".into(),
        "resenabled" => "Resonance".into(),
        "depenabled" => "Depth".into(),
        "ess" => format!("Essence {t}"),
        "group" => n.elements().map(describe).collect::<Vec<_>>().join(" and "),
        "attribute" | "attributetotal" => format!("{} {}", n.get("name"), n.get("total")),
        _ if !t.trim().is_empty() => t,
        other => other.to_owned(),
    }
}

fn has_named(ch: &Character, container: &str, item: &str, name: &str) -> bool {
    let mut all = Vec::new();
    if let Some(c) = ch.doc.child(container) {
        c.descendants(item, &mut all);
        all.extend(c.children_named(item));
    }
    all.iter().any(|e| e.get("name") == name)
}

/// `Some(true/false)` when the condition is understood, `None` when not
/// (callers treat unknown conditions as satisfied for `<required>` and
/// unsatisfied for `<forbidden>`, so the UI never blocks on them).
fn holds(n: &Element, c: &Check<'_>) -> Option<bool> {
    let ch = c.ch;
    let t = n.text();
    let t = t.trim();
    Some(match n.name.as_str() {
        "quality" | "characterquality" => {
            if c.ignore_quality == Some(t) {
                false
            } else {
                ch.items("qualities", "quality").iter().any(|q| q.get("name") == t)
            }
        }
        "magenabled" => ch.mag_enabled(),
        "resenabled" => ch.res_enabled(),
        "depenabled" => ch.dep_enabled(),
        "metatype" => ch.field("metatype") == t,
        "metavariant" => ch.field("metavariant") == t,
        "metatypecategory" => ch.field("metatypecategory") == t,
        "gameplayoption" | "setting" => ch.field("gameplayoption") == t || ch.field("settings").trim_end_matches(".xml") == t,
        "power" => has_named(ch, "powers", "power", t),
        "spell" => has_named(ch, "spells", "spell", t),
        "critterpower" => has_named(ch, "critterpowers", "critterpower", t),
        "metamagic" => has_named(ch, "metamagics", "metamagic", t),
        "martialart" => has_named(ch, "martialarts", "martialart", t),
        "bioware" | "cyberware" => has_named(ch, "cyberwares", "cyberware", t),
        "cyberwarecontains" | "biowarecontains" => {
            let mut all = Vec::new();
            if let Some(cw) = ch.doc.child("cyberwares") {
                cw.descendants("cyberware", &mut all);
            }
            all.iter().any(|e| e.get("name").contains(t))
        }
        "tradition" => ch.doc.child("tradition").is_some_and(|tr| tr.get("name") == t),
        "initiategrade" => ch.doc.get_i32("initiategrade").unwrap_or(0) >= t.parse().unwrap_or(0),
        "submersiongrade" => ch.doc.get_i32("submersiongrade").unwrap_or(0) >= t.parse().unwrap_or(0),
        "careeronly" => ch.created,
        "chargenonly" => !ch.created,
        "ess" if n.attr("grade").is_none() => {
            let v: f64 = t.trim_start_matches(['-', '+']).parse().ok()?;
            if t.starts_with('-') {
                c.sheet.essence < v
            } else {
                c.sheet.essence >= v
            }
        }
        "skill" => {
            let name = n.get("name");
            let val = n.get_i32("val").unwrap_or(0);
            let spec = n.get("spec");
            let pool = c.sheet.skills.iter().chain(c.sheet.knowledge_skills.iter());
            pool.into_iter().any(|s| s.name == name && s.rating >= val && (spec.is_empty() || s.specs.contains(&spec)))
        }
        "attribute" | "attributetotal" => {
            let total = n.get_i32("total").unwrap_or(0);
            c.sheet.attr(&n.get("name")) >= total
        }
        "group" => {
            let mut all = true;
            for k in n.elements() {
                all &= holds(k, c).unwrap_or(true);
            }
            all
        }
        _ => return None,
    })
}
