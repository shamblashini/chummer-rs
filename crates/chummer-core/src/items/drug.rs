//! Drugs (`Drug.Save`, `DrugComponent.Save`, `Drug.GenerateImprovement`).
//!
//! In Chummer5a a `<drug>` is built from `<drugcomponent>` records of
//! drugcomponents.xml at chosen levels (`CreateCustomDrug`); the
//! ready-made drugs players buy are gear in gear.xml. The `<drugs>` list of
//! drugcomponents.xml is only browsed. [`element`] still turns such a
//! record into a loadable `<drug>`: one component at level 0 carrying the
//! record's cost, availability and `<bonus>` effects.
//!
//! No fixture contains a `<drug>`, so the item oracle cannot check this
//! module; `tests/drug.rs` checks it against the C# save layout.

use crate::bonus::Choice;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr;
use crate::improvement::{fmt_num, Improvement};
use crate::xml::Element;

use super::{new_guid, Purchase};

/// Fields the oracle does not compare for drugs.
pub const IGNORE: &[&str] = &[
    // user state
    "quantity", "stolen", "sortorder",
];

/// `Guid.Empty.ToString("D")`: the source id of a custom drug.
const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// One data `<effect>` re-written the way `DrugComponent.Save` writes it:
/// attributes, limits, qualities, infos, then the non-zero numbers.
fn effect_element(src: &Element) -> Element {
    let mut e = Element::new("effect");
    for (tag, list) in [("attribute", "attribute"), ("limit", "limit")] {
        for a in src.children_named(list) {
            let (name, value) = (a.get("name"), a.get("value"));
            if name.is_empty() || value.trim().parse::<f64>().is_err() {
                continue;
            }
            let mut x = Element::new(tag);
            x.push(Element::with_text("name", name));
            x.push(Element::with_text("value", value.trim()));
            e.push(x);
        }
    }
    for q in src.children_named("quality").filter(|q| !q.text().trim().is_empty()) {
        // `WriteRaw("<quality>" + InnerXml + "</quality>")` drops attributes.
        e.push(Element::with_text("quality", q.text()));
    }
    for i in src.children_named("info") {
        e.push(Element::with_text("info", i.text()));
    }
    for k in ["initiative", "initiativedice", "duration", "speed", "crashdamage"] {
        if let Some(n) = src.get_i32(k).filter(|n| *n != 0) {
            e.push(Element::with_text(k, n.to_string()));
        }
    }
    e
}

/// A saved `<drugcomponent>` (`DrugComponent.Save`) for a data record at
/// `level`.
pub fn component_element(rec: Record<'_>, level: i32, guid: &str) -> Element {
    let e = rec.el();
    let mut c = Element::new("drugcomponent");
    let mut put = |k: &str, v: String| c.push(Element::with_text(k, v));
    put("sourceid", rec.id().to_ascii_lowercase());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", rec.category());
    let mut effects = Element::new("effects");
    for ef in e.child("effects").map(|x| x.children_named("effect").collect::<Vec<_>>()).unwrap_or_default() {
        effects.push(effect_element(ef));
    }
    c.push(effects);
    let mut put = |k: &str, v: String| c.push(Element::with_text(k, v));
    put("availability", e.child_text("availability").unwrap_or_else(|| "0".into()));
    put("cost", e.get("cost"));
    put("level", level.to_string());
    put("limit", e.get_i32("limit").unwrap_or(1).to_string());
    for k in ["rating", "threshold"] {
        if let Some(n) = e.get_i32(k).filter(|n| *n != 0) {
            put(k, n.to_string());
        }
    }
    put("source", rec.source());
    put("page", rec.page());
    c
}

/// Levels a component offers (`<effect><level>`), lowest first.
pub fn component_levels(rec: Record<'_>) -> Vec<i32> {
    let mut v: Vec<i32> = rec.el().child("effects").map(|x| x.children_named("effect").map(|e| e.get_i32("level").unwrap_or(0)).collect()).unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------
// Drugs
// ---------------------------------------------------------------------------

/// The fields of `Drug.Save` around the components.
struct DrugHead<'a> {
    source_id: String,
    name: &'a str,
    category: &'a str,
    quantity: f64,
    availability: String,
    cost: f64,
    grade: Option<&'a str>,
    source: String,
    page: String,
}

/// `Drug.Save`.
fn drug_element(h: &DrugHead<'_>, components: Vec<Element>, guid: &str) -> Element {
    let mut d = Element::new("drug");
    let mut put = |k: &str, v: String| d.push(Element::with_text(k, v));
    put("sourceid", h.source_id.clone());
    put("guid", guid.to_owned());
    put("name", h.name.to_owned());
    put("category", h.category.to_owned());
    put("quantity", fmt_num(h.quantity));
    let mut comps = Element::new("drugcomponents");
    for c in components {
        comps.push(c);
    }
    d.push(comps);
    let mut put = |k: &str, v: String| d.push(Element::with_text(k, v));
    put("availability", h.availability.clone());
    if h.cost != 0.0 {
        put("cost", fmt_num(h.cost));
    }
    if let Some(g) = h.grade {
        put("grade", g.to_owned());
    }
    put("sortorder", "0".into());
    put("stolen", "False".into());
    put("source", h.source.clone());
    put("page", h.page.clone());
    put("notes", String::new());
    put("notesColor", "#003FFF".into());
    d
}

/// A custom drug from `(component name or id, level)` pairs
/// (`CreateCustomDrug`): category "Custom Drug", quantity 1. The grade is a
/// drugcomponents.xml grade name ("Standard", "Street Cooked", ...).
pub fn custom_drug(store: &DataStore, name: &str, grade: &str, components: &[(&str, i32)], guid: &str) -> Result<Element, String> {
    let doc = store.doc("drugcomponents.xml").map_err(|e| e.to_string())?;
    let mut comps = Vec::new();
    for (key, level) in components {
        let rec = crate::data::find(&doc, "drugcomponents", "drugcomponent", key).ok_or_else(|| format!("no drug component {key}"))?;
        if !component_levels(rec).contains(level) {
            return Err(format!("{} has no level {level}", rec.name()));
        }
        comps.push(component_element(rec, *level, &new_guid()));
    }
    let foundations = comps.iter().filter(|c| c.get("category") == "Foundation").count();
    if foundations != 1 {
        return Err("a custom drug needs exactly one foundation".into());
    }
    let grade = crate::data::records(&doc, "grades", "grade").into_iter().find(|g| g.name() == grade).map_or_else(|| "Standard".to_owned(), |g| g.name());
    let head = DrugHead {
        source_id: EMPTY_GUID.into(),
        name,
        category: "Custom Drug",
        quantity: 1.0,
        availability: "0".into(),
        cost: 0.0,
        grade: Some(&grade),
        source: String::new(),
        page: String::new(),
    };
    Ok(drug_element(&head, comps, guid))
}

/// The `<effect>` of a ready-made record: its `<bonus>` attributes, limits,
/// qualities and initiative dice (other bonus nodes have no drug effect).
fn bonus_effect(rec: Record<'_>) -> Element {
    let mut src = Element::new("effect");
    src.push(Element::with_text("level", "0"));
    if let Some(b) = rec.el().child("bonus") {
        for n in b.elements() {
            match n.name.as_str() {
                "attribute" | "limit" | "quality" | "initiativedice" | "initiative" => src.push(n.clone()),
                _ => {}
            }
        }
    }
    effect_element(&src)
}

/// A `<drug>` from a ready-made record of drugcomponents.xml `<drugs>`.
pub fn element(rec: Record<'_>, p: &Purchase, guid: &str) -> Element {
    let e = rec.el();
    let mut c = Element::new("drugcomponent");
    let mut put = |k: &str, v: String| c.push(Element::with_text(k, v));
    put("sourceid", rec.id().to_ascii_lowercase());
    put("guid", new_guid());
    put("name", rec.name());
    put("category", "Foundation".into());
    let mut effects = Element::new("effects");
    effects.push(bonus_effect(rec));
    c.push(effects);
    let mut put = |k: &str, v: String| c.push(Element::with_text(k, v));
    put("availability", "0".into());
    put("cost", if p.free { "0".into() } else { e.get("cost") });
    put("level", "0".into());
    put("limit", "1".into());
    put("source", rec.source());
    put("page", rec.page());
    let head = DrugHead {
        source_id: rec.id().to_ascii_lowercase(),
        name: &rec.name(),
        category: &rec.category(),
        quantity: p.qty(),
        availability: e.get("avail"),
        cost: 0.0,
        grade: Some("Standard"),
        source: rec.source(),
        page: rec.page(),
    };
    drug_element(&head, vec![c], guid)
}

// ---------------------------------------------------------------------------
// Effects and improvements
// ---------------------------------------------------------------------------

/// The combined active effects of a saved drug (`Drug.Attributes`,
/// `Limits`, `Initiative`, `InitiativeDice`, `Qualities`, `Speed`,
/// `CrashDamage`): each component contributes its effect at its level.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effects {
    pub attributes: Vec<(String, f64)>,
    pub limits: Vec<(String, i32)>,
    pub initiative: i32,
    pub initiative_dice: i32,
    pub qualities: Vec<String>,
    pub speed: i32,
    pub crash_damage: i32,
}

fn add_to(list: &mut Vec<(String, f64)>, k: String, v: f64) {
    match list.iter_mut().find(|(n, _)| *n == k) {
        Some(slot) => slot.1 += v,
        None => list.push((k, v)),
    }
}

/// The effect of `component` at its level (`DrugComponent.ActiveDrugEffect`).
fn active_effect(component: &Element) -> Option<&Element> {
    let level = component.get_i32("level").unwrap_or(0);
    component.child("effects")?.children_named("effect").find(|e| e.get_i32("level").unwrap_or(0) == level)
}

/// Aggregate the active effects of a saved `<drug>`. Speed starts at
/// Chummer's base of 9.
pub fn effects(drug: &Element) -> Effects {
    let mut out = Effects { speed: 9, ..Effects::default() };
    let mut limits: Vec<(String, f64)> = Vec::new();
    for c in drug.child("drugcomponents").map(|x| x.children_named("drugcomponent").collect::<Vec<_>>()).unwrap_or_default() {
        let Some(e) = active_effect(c) else { continue };
        for a in e.children_named("attribute") {
            add_to(&mut out.attributes, a.get("name"), a.get_f64("value").unwrap_or(0.0));
        }
        for l in e.children_named("limit") {
            add_to(&mut limits, l.get("name"), l.get_f64("value").unwrap_or(0.0));
        }
        for q in e.children_named("quality") {
            if !out.qualities.contains(&q.text()) {
                out.qualities.push(q.text());
            }
        }
        out.initiative += e.get_i32("initiative").unwrap_or(0);
        out.initiative_dice += e.get_i32("initiativedice").unwrap_or(0);
        out.speed += e.get_i32("speed").unwrap_or(0);
        out.crash_damage += e.get_i32("crashdamage").unwrap_or(0);
    }
    out.limits = limits.into_iter().map(|(k, v)| (k, v as i32)).collect();
    out
}

/// Nuyen cost of one dose with the stock game data (`cost_with`).
pub fn cost(drug: &Element) -> f64 {
    cost_with(None, drug)
}

/// Multiplier of a drug grade: the grade's `<cost>` in drugcomponents.xml
/// of `store` (else of the stock data), 1 for an unknown grade.
pub fn grade_multiplier(store: Option<&DataStore>, grade: &str) -> f64 {
    store
        .or_else(|| crate::data::shared_store())
        .and_then(|st| st.doc("drugcomponents.xml").ok())
        .and_then(|doc| crate::data::records(&doc, "grades", "grade").into_iter().find(|g| g.name() == grade).and_then(|g| g.el().get_f64("cost")))
        .unwrap_or(1.0)
}

/// Nuyen cost of one dose (`Drug.Cost`): each active component's cost at
/// its level (`DrugComponent.CostPerLevel`), times the grade multiplier.
// chummer-rs deviates from Chummer (LB-05): Chummer reads the grade's
// <cost> in CreateCustomDrug and never applies it. CF p. 190 prices the
// grades (Street Cooked half, Pharmaceutical x2, Designer x6), so the sum
// is multiplied by it.
pub fn cost_with(store: Option<&DataStore>, drug: &Element) -> f64 {
    let attrs = expr::NoAttributes;
    let sum: f64 = drug
        .child("drugcomponents")
        .map(|x| {
            x.children_named("drugcomponent")
                .filter(|c| active_effect(c).is_some())
                .map(|c| {
                    let level = c.get_i32("level").unwrap_or(0);
                    let s = c.get("cost").replace("{Level}", &level.to_string()).replace("Level", &level.to_string());
                    expr::value_to_dec(&s, level, &attrs)
                })
                .sum()
        })
        .unwrap_or(0.0);
    let grade = drug.get("grade");
    if grade.is_empty() { sum } else { sum * grade_multiplier(store, &grade) }
}

/// `"+#,0;-#,0;0"`.
fn signed(v: f64) -> String {
    if v > 0.0 {
        format!("+{}", fmt_num(v))
    } else {
        fmt_num(v)
    }
}

/// `Drug.GenerateImprovement`: custom, disabled improvements in the drug's
/// improvement group, which the player switches on while the drug is
/// active. Qualities granted by the drug need quality creation and are
/// returned by name instead.
pub fn generate_improvements(drug: &Element) -> (Vec<Improvement>, Vec<String>) {
    let name = drug.get("name");
    let guid = drug.get("guid");
    let fx = effects(drug);
    let base = |kind: &str, label: String| Improvement {
        kind: kind.into(),
        source: "Drug".into(),
        source_name: guid.clone(),
        custom_name: format!("{name} - {label}"),
        custom_group: name.clone(),
        custom: true,
        enabled: false,
        rating: 1,
        ..Default::default()
    };
    let mut v = Vec::new();
    for (a, val) in fx.attributes.iter().filter(|(_, x)| *x != 0.0) {
        v.push(Improvement { improved_name: a.clone(), aug: *val, ..base("Attribute", format!("{a} {}", signed(*val))) });
    }
    for (l, val) in fx.limits.iter().filter(|(_, x)| *x != 0) {
        let kind = match l.to_ascii_uppercase().as_str() {
            "PHYSICAL" => "PhysicalLimit",
            "MENTAL" => "MentalLimit",
            "SOCIAL" => "SocialLimit",
            _ => continue,
        };
        v.push(Improvement { val: f64::from(*val), ..base(kind, format!("{l} {}", signed(f64::from(*val)))) });
    }
    if fx.initiative != 0 {
        v.push(Improvement { val: f64::from(fx.initiative), ..base("Initiative", format!("Initiative {}", signed(f64::from(fx.initiative)))) });
    }
    if fx.initiative_dice != 0 {
        v.push(Improvement { val: f64::from(fx.initiative_dice), ..base("InitiativeDice", format!("Initiative Dice {}", signed(f64::from(fx.initiative_dice)))) });
    }
    (v, fx.qualities)
}

// ---------------------------------------------------------------------------
// Module contract
// ---------------------------------------------------------------------------

/// Ready-made drug records ask nothing.
pub fn choices(_ch: &Character, _store: &DataStore, _rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    Vec::new()
}

/// Add a ready-made drug and its (disabled) effect improvements. Qualities
/// the drug grants are not created (see [`generate_improvements`]).
pub fn add(ch: &mut Character, _store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    let guid = new_guid();
    let d = element(rec, p, &guid);
    add_element(ch, d);
    Ok(guid)
}

/// Add a built `<drug>` (e.g. from [`custom_drug`]) with its improvements.
pub fn add_element(ch: &mut Character, d: Element) {
    let (imps, _qualities) = generate_improvements(&d);
    ch.items_mut("drugs").push(d);
    ch.improvements.list.extend(imps);
    ch.dirty = true;
}

/// Oracle: a saved drug cannot be traced back to one record (custom drugs
/// have no source id), so only ready-made ones would rebuild; no fixture
/// has any.
pub fn rebuild(_ch: &Character, _store: &DataStore, _saved: &Element) -> Option<Element> {
    None
}
