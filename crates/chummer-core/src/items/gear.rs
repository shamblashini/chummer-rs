//! Gear (`Gear.Create` / `Gear.Save`), the `addgear` bonus, and gear cost
//! (`Gear.TotalCost`).
//!
//! The selection dialogs that `Gear.Create` may open share the one answer
//! in [`Purchase::answer`]: the bonus selection, the name of a Custom Item,
//! the amount of a `Variable(...)` cost, or the weapon category of
//! ammunition. No gear record needs more than one of them.

use crate::bonus::{self, BonusSource, Choice, Outcome};
use crate::calc::{AttributeValues, SheetAttributes};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr::{self, AttributeSource, NoAttributes};
use crate::improvement::{bool_str, fmt_num};
use crate::xml::Element;

use super::{new_guid, Purchase};

const FILE: &str = "gear.xml";

/// Matrix attribute names as used in `{Gear X}`-style tokens, with the
/// saved field holding each.
const MATRIX: &[(&str, &str)] = &[
    ("Attack", "attack"),
    ("Sleaze", "sleaze"),
    ("Data Processing", "dataprocessing"),
    ("Firewall", "firewall"),
    ("Device Rating", "devicerating"),
    ("Program Limit", "programlimit"),
];

/// Fields the oracle does not compare for gear.
pub const IGNORE: &[&str] = &[
    // pre-5.214 saves wrote the record id as `id`; it is `sourceid` now
    "id",
    // guid of the weapon created from `<addweapon>`, generated per instance
    "weaponguid",
    // the player bonds foci and names gear; the overclocked attribute is
    // chosen by the player (5.202 also wrote it as "" or "False")
    "bonded", "gearname", "overclocked",
];

/// What `Gear.CreateCoreAsync` is called with.
struct Spec<'a> {
    rating: i32,
    qty: Option<f64>,
    /// `strForceValue`.
    forced: String,
    /// Answer to the dialog `Create` would open (see the module docs).
    answer: Option<String>,
    /// The item this gear goes into, for `Parent ...` expressions.
    parent: Option<&'a Element>,
    create_children: bool,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// What an expression on one gear can refer to.
struct Ev<'a> {
    rating: i32,
    min_rating: i32,
    parent: Option<&'a Element>,
    children: Option<&'a Element>,
    attrs: &'a dyn AttributeSource,
}

impl<'a> Ev<'a> {
    /// Context for a saved (or partly built) gear element.
    fn of(e: &'a Element, parent: Option<&'a Element>, attrs: &'a dyn AttributeSource) -> Ev<'a> {
        let rating = e.get_i32("rating").unwrap_or(0);
        let mut ev = Ev { rating, min_rating: 0, parent, children: e.child("children"), attrs };
        ev.min_rating = min_rating_value(&e.get("minrating"), &e.get("maxrating"), &ev);
        ev
    }

    /// `Gear.ProcessRatingStringAsDec`.
    fn dec(&self, expression: &str) -> f64 {
        if expression.is_empty() {
            return 0.0;
        }
        let mut s = expr::fixed_values(expression, self.rating).trim_start_matches('+').to_owned();
        if !expr::needs_evaluation(&s) {
            return expr::parse_plain(&s).unwrap_or(0.0);
        }
        let parent_rating = self.parent.and_then(|p| p.get_i32("rating")).unwrap_or(0);
        s = s.replace("{Parent Rating}", &parent_rating.to_string()).replace("Parent Rating", &parent_rating.to_string());
        let (pcost, gcost, pweight, gweight) = match self.parent.filter(|p| p.name == "gear") {
            Some(p) => {
                let w = Ev::of(p, None, self.attrs).dec(&p.get("weight"));
                (own_cost_pre(p, None), calculated_cost(p, None), w, w * qty(p))
            }
            None => (0.0, 0.0, 0.0, 0.0),
        };
        for (token, v) in [("Parent Cost", pcost), ("Gear Cost", gcost), ("Parent Weight", pweight), ("Gear Weight", gweight)] {
            s = s.replace(&format!("{{{token}}}"), &fmt_num(v)).replace(token, &fmt_num(v));
        }
        let kids: Vec<&Element> = self.children.map(|c| c.children_named("gear").collect()).unwrap_or_default();
        if s.contains("Children Cost") {
            let v: f64 = kids.iter().map(|k| calculated_cost(k, None)).sum();
            s = s.replace("{Children Cost}", &fmt_num(v)).replace("Children Cost", &fmt_num(v));
        }
        if s.contains("Children Weight") {
            let v: f64 = kids.iter().map(|k| Ev::of(k, None, self.attrs).dec(&k.get("weight")) * qty(k)).sum();
            s = s.replace("{Children Weight}", &fmt_num(v)).replace("Children Weight", &fmt_num(v));
        }
        s = s.replace("{MinRating}", &self.min_rating.to_string()).replace("MinRating", &self.min_rating.to_string());
        s = s.replace("{Rating}", &self.rating.to_string());
        for (name, field) in MATRIX {
            let token = format!("{{Gear {name}}}");
            if s.contains(&token) {
                let v = self.parent.map_or(0.0, |p| Ev::of(p, None, self.attrs).dec(&p.get(field)));
                s = s.replace(&token, &fmt_num(v));
            }
            let token = format!("{{Parent {name}}}");
            if s.contains(&token) {
                let v = self.parent.map(|p| p.get(field)).filter(|v| !v.is_empty()).unwrap_or_else(|| "0".into());
                s = s.replace(&token, &v);
            }
            let token = format!("{{Children {name}}}");
            if s.contains(&token) {
                let v: f64 = kids.iter().filter(|k| k.get_bool("equipped").unwrap_or(true)).map(|k| Ev::of(k, None, self.attrs).dec(&k.get(field))).sum();
                s = s.replace(&token, &fmt_num(v));
            }
        }
        s = s.replace("Rating", &self.rating.to_string());
        expr::evaluate_num(&expr::substitute_attributes(&s, self.attrs)).unwrap_or(0.0)
    }

    /// `Gear.ProcessRatingString`.
    fn int(&self, expression: &str) -> i32 {
        expr::standard_round(self.dec(expression))
    }
}

/// `Gear.MaxRatingValue`.
fn max_rating_value(max: &str, ev: &Ev<'_>) -> i32 {
    if max.is_empty() { i32::MAX } else { ev.int(max) }
}

/// `Gear.MinRatingValue`.
fn min_rating_value(min: &str, max: &str, ev: &Ev<'_>) -> i32 {
    let v = if min.is_empty() { 0 } else { ev.int(min) };
    let m = max_rating_value(max, ev);
    if v == 0 && m > 0 && m != i32::MAX { 1 } else { v }
}

fn qty(e: &Element) -> f64 {
    e.get_f64("qty").unwrap_or(1.0)
}

fn cost_for(e: &Element) -> f64 {
    e.get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0)
}

/// `Gear.OwnCostPreMultipliers`.
fn own_cost_pre(e: &Element, parent: Option<&Element>) -> f64 {
    let v = Ev::of(e, parent, &NoAttributes).dec(&e.get("cost"));
    if e.get_bool("discountedcost").unwrap_or(false) { v * 0.9 } else { v }
}

/// `Gear.CalculatedCost`.
fn calculated_cost(e: &Element, parent: Option<&Element>) -> f64 {
    own_cost_pre(e, parent) * qty(e) / cost_for(e)
}

/// `Gear.TotalCost` with the parent item known.
fn total_cost(e: &Element, parent: Option<&Element>) -> f64 {
    let plugins: f64 = e.child("children").map(|c| c.children_named("gear").map(|k| total_cost(k, Some(e))).sum()).unwrap_or(0.0);
    let multiplier = parent.and_then(|p| p.get_i32("childcostmultiplier")).unwrap_or(1);
    own_cost_pre(e, parent) * qty(e) * f64::from(multiplier) / cost_for(e) + plugins * qty(e)
}

/// `Gear.CalculatedCost`: own cost × quantity / cost-for, without children
/// (what a parent's `Gear Cost` token sums).
pub fn own_calculated_cost(e: &Element) -> f64 {
    calculated_cost(e, None)
}

/// `Gear.TotalCost`: nuyen cost of a saved gear, its children and quantity.
pub fn cost(e: &Element) -> f64 {
    total_cost(e, None)
}

/// `Gear.OwnCostPreMultipliers`: the evaluated `<cost>` (with the
/// black-market discount), before quantity and `CostFor`.
pub fn own_cost_pre_multipliers(e: &Element, parent: Option<&Element>) -> f64 {
    own_cost_pre(e, parent)
}

/// `Gear.CostFor` (1 unless the gear is priced per N units).
pub fn cost_for_units(e: &Element) -> f64 {
    cost_for(e)
}

/// `Gear.OwnWeight`: the evaluated `<weight>`, 0 for gear included in
/// its parent.
pub fn own_weight(e: &Element, parent: Option<&Element>) -> f64 {
    if e.get_bool("includedinparent").unwrap_or(false) {
        return 0.0;
    }
    let w = e.get("weight");
    if w.trim().is_empty() { 0.0 } else { Ev::of(e, parent, &NoAttributes).dec(&w) }
}

/// `Gear.TotalWeight`: own weight plus equipped children, times quantity.
pub fn total_weight(e: &Element, parent: Option<&Element>) -> f64 {
    let kids: f64 = e
        .child("children")
        .map(|c| c.children_named("gear").filter(|k| k.get_bool("equipped").unwrap_or(true)).map(|k| total_weight(k, Some(e))).sum())
        .unwrap_or(0.0);
    (own_weight(e, parent) + kids) * qty(e)
}

/// `Gear.TotalCost` for a gear inside `parent` (another gear, armor,
/// cyberware...), whose rating and child cost multiplier apply.
pub fn cost_in(e: &Element, parent: &Element) -> f64 {
    total_cost(e, Some(parent))
}

// ---------------------------------------------------------------------------
// Building the saved element
// ---------------------------------------------------------------------------

/// `Save` writes bonus nodes as `<x>` + InnerXml, so their attributes are
/// lost. Nodes with child elements but no text (`<selecttext />`) are kept,
/// as 5.202 saves do, so the bonus can be applied again.
fn bonus_node(rec: &Element, name: &str) -> Element {
    match rec.child(name) {
        Some(b) if b.elements().next().is_some() || !b.text().trim().is_empty() => Element { name: name.into(), attrs: Vec::new(), children: b.children.clone() },
        _ => Element::new(name),
    }
}

/// A gear record by `<name>` and optional `<category>` (`CreateChild`).
fn find_named<'a>(doc: &'a Element, name: &str, category: &str) -> Option<Record<'a>> {
    doc.child("gears")?
        .children_named("gear")
        .find(|g| (name.is_empty() || g.get("name") == name) && (category.is_empty() || g.get("category") == category))
        .map(Record)
}

/// The data record of a saved gear: `sourceid` (or legacy `id`), else name
/// and category.
pub fn record_of<'a>(doc: &'a Element, saved: &Element) -> Option<Record<'a>> {
    let gears = doc.child("gears")?;
    let id = saved.child_text("sourceid").filter(|s| !s.is_empty()).or_else(|| saved.child_text("id")).unwrap_or_default();
    if !id.is_empty() {
        if let Some(g) = gears.children_named("gear").find(|g| g.get("id").eq_ignore_ascii_case(&id)) {
            return Some(Record(g));
        }
    }
    find_named(doc, &saved.get("name"), &saved.get("category")).or_else(|| find_named(doc, &saved.get("name"), ""))
}

/// Attribute values for `{LOG}`-style tokens.
fn attribute_values(ch: &Character, store: &DataStore) -> Vec<AttributeValues> {
    let rules = crate::calc::Rules::default();
    ch.attributes.iter().map(|a| crate::calc::attribute_values_with(ch, &a.name, &rules, Some(store))).collect()
}

/// The extra `Create` stores: the weapon category of ammunition, the bonus
/// selection (answered by the forced value when there is no answer), or
/// the forced value when the gear has no bonus.
fn initial_extra(d: &Element, s: &Spec<'_>) -> String {
    let answer = s.answer.clone().filter(|a| !a.is_empty());
    if ammo_needs_category(d) {
        return answer.unwrap_or_default();
    }
    match d.child("bonus") {
        Some(b) if bonus_applies(d, b) => answer.or_else(|| asks_selection(b).then(|| s.forced.clone())).unwrap_or_default(),
        Some(_) => String::new(),
        None => s.forced.clone(),
    }
}

/// Whether a bonus selects a value (which the forced value answers).
fn asks_selection(b: &Element) -> bool {
    b.elements().any(|n| n.name.starts_with("select") || asks_selection(n))
}

/// `Gear.Create` + `Gear.Save`.
fn build(doc: &Element, attrs: &dyn AttributeSource, rec: Record<'_>, s: &Spec<'_>, guid: &str) -> Element {
    let d = rec.el();
    let cost_for = d.get_f64("costfor").unwrap_or(1.0);
    let mut max_rating = d.get("rating");
    if max_rating == "0" {
        max_rating = String::new();
    }
    let min_rating = d.get("minrating");
    let mut ev = Ev { rating: s.rating, min_rating: 0, parent: s.parent, children: None, attrs };
    ev.min_rating = min_rating_value(&min_rating, &max_rating, &ev);
    let defer = (max_rating.contains("Parent") || min_rating.contains("Parent")) && s.parent.is_none();
    let rating = if defer { s.rating } else { s.rating.min(max_rating_value(&max_rating, &ev)).max(ev.min_rating) };

    let mut name = rec.name();
    if name == "Custom Item" {
        if !s.forced.is_empty() {
            name = s.forced.clone();
        } else if let Some(a) = s.answer.as_ref().filter(|a| !a.is_empty()) {
            name = a.clone();
        }
    }
    let mut cost = d.get("cost");
    if cost.starts_with("Variable(") && s.forced.is_empty() {
        cost = variable_cost(&cost, s.answer.as_deref());
    }
    let (mut attack, mut sleaze, mut dp, mut fw) = (d.get("attack"), d.get("sleaze"), d.get("dataprocessing"), d.get("firewall"));
    let array = d.get("attributearray");
    if d.child("attributearray").is_some() {
        let p: Vec<String> = array.split(',').map(str::to_owned).chain(std::iter::repeat(String::new())).take(4).collect();
        (attack, sleaze, dp, fw) = (p[0].clone(), p[1].clone(), p[2].clone(), p[3].clone());
    }

    let mut g = Element::new("gear");
    let mut put = |k: &str, v: String| g.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", name);
    put("category", rec.category());
    put("capacity", d.get("capacity"));
    put("armorcapacity", d.get("armorcapacity"));
    put("minrating", min_rating);
    put("maxrating", max_rating);
    put("rating", rating.to_string());
    put("qty", fmt_num(s.qty.unwrap_or(cost_for)));
    put("avail", d.get("avail"));
    if cost_for > 1.0 {
        put("costfor", fmt_num(cost_for));
    }
    put("cost", cost);
    put("weight", d.get("weight"));
    put("extra", initial_extra(d, s));
    put("bonded", bool_str(false));
    put("equipped", bool_str(true));
    put("wirelesson", bool_str(false));
    put("stolen", bool_str(d.get_bool("stolen").unwrap_or(false)));
    for b in ["bonus", "wirelessbonus", "weaponbonus", "flechetteweaponbonus"] {
        g.push(bonus_node(d, b));
    }
    let mut put = |k: &str, v: String| g.push(Element::with_text(k, v));
    put("source", rec.source());
    put("page", rec.page());
    put("isflechetteammo", bool_str(d.get_bool("isflechetteammo").unwrap_or(false)));
    put("ammoforweapontype", d.get("ammoforweapontype"));
    put("canformpersona", d.get("canformpersona"));
    put("devicerating", d.get("devicerating"));
    put("gearname", String::new());
    put("forcedvalue", s.forced.clone());
    put("matrixcmfilled", "0".into());
    put("matrixcmbonus", d.get_i32("matrixcmbonus").unwrap_or(0).to_string());
    put("parentid", String::new());
    put("allowrename", bool_str(d.get_bool("allowrename").unwrap_or(false)));
    if let Some(m) = d.get_i32("childcostmultiplier").filter(|m| *m != 1) {
        put("childcostmultiplier", m.to_string());
    }
    if let Some(m) = d.get_i32("childavailmodifier").filter(|m| *m != 0) {
        put("childavailmodifier", m.to_string());
    }
    g.push(Element::new("children"));
    let mut put = |k: &str, v: String| g.push(Element::with_text(k, v));
    put("location", String::new());
    put("notes", d.child_text("altnotes").unwrap_or_else(|| d.get("notes")));
    put("notesColor", "Chocolate".into());
    put("discountedcost", bool_str(false));
    put("programlimit", d.get("programs"));
    put("overclocked", "None".into());
    put("attack", attack);
    put("sleaze", sleaze);
    put("dataprocessing", dp);
    put("firewall", fw);
    put("attributearray", array);
    for m in ["modattack", "modsleaze", "moddataprocessing", "modfirewall", "modattributearray"] {
        put(m, d.get(m));
    }
    put("canswapattributes", bool_str(d.child("attributearray").is_some()));
    put("active", bool_str(false));
    put("homenode", bool_str(false));
    put("sortorder", "0".into());

    if s.create_children {
        create_children(doc, attrs, d, &mut g);
    }
    g
}

/// `Variable(min-max)` cost: the chosen amount, else the minimum.
fn variable_cost(cost: &str, answer: Option<&str>) -> String {
    let inner = cost.trim_start_matches("Variable(").trim_end_matches(')');
    let first = inner.split('-').next().unwrap_or("").to_owned();
    match answer.and_then(expr::parse_plain) {
        Some(v) if variable_range(cost).is_some() => fmt_num(v),
        _ => first,
    }
}

/// The range a `Variable(...)` cost asks for, if it asks at all.
fn variable_range(cost: &str) -> Option<(f64, f64)> {
    let inner = cost.strip_prefix("Variable(")?.trim_end_matches(')');
    let (min, max) = match inner.split_once('-') {
        Some((a, b)) => (expr::parse_plain(a).unwrap_or(0.0), expr::parse_plain(b).unwrap_or(f64::MAX)),
        None => (expr::parse_plain(&inner.replace('+', "")).unwrap_or(0.0), f64::MAX),
    };
    (min != 0.0 || max != f64::MAX).then_some((min, max.min(1_000_000.0)))
}

/// Ammunition asks for the weapon category it is for, unless `@noextra`.
fn ammo_needs_category(d: &Element) -> bool {
    d.child("ammoforweapontype").is_some_and(|a| !a.text().is_empty() && a.attr("noextra") != Some("True"))
}

/// Foci apply their bonus only when bonded, except weapon foci.
fn bonus_applies(d: &Element, bonus: &Element) -> bool {
    !matches!(d.get("category").as_str(), "Foci" | "Metamagic Foci") || bonus.child("selectweapon").is_some()
}

/// `Gear.CreateChildren`: the `<gears><usegear>` the data adds.
/// (`<choosegear>` lists need a dialog and are not created.)
fn create_children(doc: &Element, attrs: &dyn AttributeSource, node: &Element, parent: &mut Element) {
    let Some(gears) = node.child("gears") else { return };
    for u in gears.children_named("usegear") {
        if let Some(child) = create_child(doc, attrs, u, parent) {
            parent.child_or_insert("children").push(child);
        }
    }
}

/// `Gear.CreateChild`.
fn create_child(doc: &Element, attrs: &dyn AttributeSource, u: &Element, parent: &Element) -> Option<Element> {
    let rec = find_named(doc, &u.get("name"), &u.get("category"))?;
    let name_el = u.child("name");
    let attr = |k: &str| name_el.and_then(|n| n.attr(k)).unwrap_or("");
    let spec = Spec {
        rating: u.get_i32("rating").unwrap_or(0),
        qty: Some(expr::parse_plain(attr("qty")).unwrap_or(1.0)),
        forced: attr("select").to_owned(),
        answer: None,
        parent: Some(parent),
        create_children: attr("createchildren") != "False",
    };
    let mut c = build(doc, attrs, rec, &spec, &new_guid());
    c.set_child_text("cost", "0");
    c.set_child_text("parentid", parent.get("guid"));
    for k in ["source", "page"] {
        if let Some(v) = u.child_text(k).filter(|v| !v.is_empty()) {
            c.set_child_text(k, v);
        }
    }
    if let Some(v) = u.child_text("capacity") {
        c.set_child_text("capacity", v);
    }
    create_children(doc, attrs, u, &mut c);
    Some(c)
}

/// Find a saved item by guid.
fn find_by_guid<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_by_guid(c, guid))
}

/// The item a nested gear sits in (a gear's `<children>`, an armor's or a
/// cyberware's `<gears>`, ...). `None` for top-level gear.
fn owner_of<'a>(e: &'a Element, guid: &str, top: bool) -> Option<&'a Element> {
    for list in e.elements() {
        if !top && list.elements().any(|g| g.name == "gear" && g.get("guid").eq_ignore_ascii_case(guid)) {
            return Some(e);
        }
        for item in list.elements() {
            if let Some(o) = owner_of(item, guid, false) {
                return Some(o);
            }
        }
    }
    None
}

/// The spec a purchase asks for.
fn spec_of<'a>(p: &Purchase, rec: Record<'_>, parent: Option<&'a Element>) -> Spec<'a> {
    Spec {
        rating: p.rating,
        qty: Some(if p.qty > 0.0 { p.qty } else { default_qty(rec) }),
        forced: String::new(),
        answer: p.answer.clone(),
        parent,
        create_children: true,
    }
}

/// Apply the buyer's cost choices (`PickGear`: black market, free).
fn apply_cost_choices(g: &mut Element, p: &Purchase) {
    if p.free {
        g.set_child_text("cost", "0");
    } else if (p.cost_multiplier - 0.9).abs() < 1e-9 {
        g.set_child_text("discountedcost", "True");
    } else if p.cost_multiplier > 0.0 && (p.cost_multiplier - 1.0).abs() > 1e-9 {
        let c = g.get("cost");
        g.set_child_text("cost", format!("({c}) * {}", fmt_num(p.cost_multiplier)));
    }
}

/// Build the saved `<gear>` for a purchase. `p.parent` names the gear (or
/// armor, cyberware...) it goes into.
pub fn element(ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase, guid: &str) -> Result<Element, String> {
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let parent = match &p.parent {
        Some(pg) => Some(find_by_guid(&ch.doc, pg).ok_or_else(|| format!("no item with guid {pg}"))?),
        None => None,
    };
    let attrs = attribute_values(ch, store);
    let mut g = build(&doc, &SheetAttributes(&attrs), rec, &spec_of(p, rec, parent), guid);
    apply_cost_choices(&mut g, p);
    Ok(g)
}

/// Quantity the selection dialog starts at: the record's `costfor`.
pub fn default_qty(rec: Record<'_>) -> f64 {
    rec.el().get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0)
}

/// Rating range the selection dialog offers (`SelectGear.UpdateGearInfo`),
/// or `None` for gear without a rating.
pub fn rating_range(rec: Record<'_>, parent: Option<&Element>) -> Option<(i32, i32)> {
    let d = rec.el();
    let ev = Ev { rating: 0, min_rating: 0, parent, children: None, attrs: &NoAttributes };
    let max = ev.int(&d.get("rating"));
    if max <= 0 {
        return None;
    }
    let ev = Ev { rating: max, ..ev };
    let min = match d.child_text("minrating") {
        Some(m) if !m.is_empty() => ev.int(&m).min(max),
        _ => 1,
    };
    Some((min, max))
}

/// Selections needed before adding a gear: the bonus selection, a Custom
/// Item name, a variable cost, or an ammunition weapon category.
pub fn choices(ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    let d = rec.el();
    let mut v = Vec::new();
    if rec.name() == "Custom Item" {
        v.push(Choice { node: "customitem".into(), prompt: "Name of the custom item".into(), options: Vec::new() });
    }
    if let Some((min, max)) = variable_range(&d.get("cost")) {
        let prompt = format!("Cost of {} ({}-{})", rec.name(), fmt_num(min), fmt_num(max));
        v.push(Choice { node: "variablecost".into(), prompt, options: Vec::new() });
    }
    if ammo_needs_category(d) {
        let options = weapon_categories(store, &d.get("ammoforweapontype"));
        v.push(Choice { node: "ammoforweapontype".into(), prompt: format!("Weapon category for {}", rec.name()), options });
    }
    if let Some(b) = d.child("bonus").filter(|b| bonus_applies(d, b)) {
        let src = BonusSource { kind: "Gear".into(), guid: String::new(), name: rec.name(), rating: p.rating };
        v.extend(bonus::choices(ch, store, b, &src));
    }
    v
}

/// `SelectWeaponCategory` with a weapon type: categories of that type.
fn weapon_categories(store: &DataStore, kind: &str) -> Vec<String> {
    let Ok(doc) = store.doc("weapons.xml") else { return Vec::new() };
    doc.child("categories")
        .map(|c| c.children_named("category").filter(|e| e.attr("type") == Some(kind) || e.text() == "Exotic Ranged Weapons").map(Element::text).collect())
        .unwrap_or_default()
}

/// Run the data bonuses of a built gear and its auto-created children
/// (`Gear.Create` with `blnAddImprovements`). The top gear's bonus uses
/// `rating`, the purchase rating before clamping, as `Create` does.
fn apply_bonuses(ch: &Character, store: &DataStore, doc: &Element, g: &mut Element, rating: i32, forced: Option<&str>) -> Outcome {
    let mut out = Outcome::default();
    if let Some(rec) = record_of(doc, g) {
        let d = rec.el();
        if let Some(b) = d.child("bonus").filter(|b| bonus_applies(d, b)) {
            let src = BonusSource { kind: "Gear".into(), guid: g.get("guid"), name: g.get("name"), rating };
            let answer = forced.map(str::to_owned).or_else(|| Some(g.get("extra")).filter(|e| !e.is_empty()));
            let o = bonus::apply(ch, store, b, &src, answer.as_deref());
            if let Some(sel) = o.selected.clone().filter(|s| !s.is_empty()) {
                g.set_child_text("extra", sel);
            }
            merge(&mut out, o);
        }
    }
    if let Some(kids) = g.child_mut("children") {
        for k in kids.elements_mut() {
            let r = k.get_i32("rating").unwrap_or(0);
            let f = k.get("forcedvalue");
            let o = apply_bonuses(ch, store, doc, k, r, Some(f.as_str()).filter(|f| !f.is_empty()));
            merge(&mut out, o);
        }
    }
    out
}

fn merge(into: &mut Outcome, o: Outcome) {
    into.improvements.extend(o.improvements);
    into.added.extend(o.added);
    into.flags.extend(o.flags);
    into.unsupported.extend(o.unsupported);
}

/// Store a bonus outcome: improvements, flags and the objects it created.
fn store_outcome(ch: &mut Character, store: &DataStore, out: &Outcome) {
    crate::items::place_added(ch, store, &out.added);
    super::apply_outcome(ch, out);
}

/// Add a gear to the character, at the top level or inside `p.parent`.
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    let guid = new_guid();
    let mut g = element(ch, store, rec, p, &guid)?;
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let out = apply_bonuses(ch, store, &doc, &mut g, p.rating, p.answer.as_deref());
    match &p.parent {
        Some(pg) => {
            let parent = super::find_by_guid_mut(&mut ch.doc, pg).ok_or_else(|| format!("no item with guid {pg}"))?;
            let list = if parent.name == "gear" { "children" } else { "gears" };
            parent.child_or_insert(list).push(g);
        }
        None => ch.items_mut("gears").push(g),
    }
    store_outcome(ch, store, &out);
    Ok(guid)
}

/// How a data node creates nested gear.
#[derive(Clone, Copy, PartialEq)]
enum Maker {
    /// `Gear.CreateChild` (gear and weapon accessory `<usegear>`).
    Child,
    /// `Gear.CreateFromNode` (armor, armor mod, cyberware, vehicle).
    FromNode,
}

/// The data file, container and item of an item that can hold gear.
fn owner_data(owner: &Element) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match owner.name.as_str() {
        "armor" => ("armor.xml", "armors", "armor"),
        "armormod" => ("armor.xml", "mods", "mod"),
        "cyberware" if owner.get("improvementsource") == "Bioware" => ("bioware.xml", "biowares", "bioware"),
        "cyberware" => ("cyberware.xml", "cyberwares", "cyberware"),
        "vehicle" => ("vehicles.xml", "vehicles", "vehicle"),
        "accessory" => ("weapons.xml", "accessories", "accessory"),
        _ => return None,
    })
}

/// Name a gear-creating node refers to (`<name>`, `<id>` or its text).
fn node_target(n: &Element) -> String {
    n.child_text("name").or_else(|| n.child_text("id")).unwrap_or_else(|| n.text())
}

/// The child node of `node/gears` that created `saved`.
fn find_maker_node(node: &Element, maker: Maker, saved: &Element) -> Option<Element> {
    let item = if maker == Maker::Child { "usegear" } else { "gear" };
    let gears = node.child("gears")?;
    let (name, category, id) = (saved.get("name"), saved.get("category"), saved.get("sourceid") + &saved.get("id"));
    gears
        .children_named(item)
        .find(|u| {
            let t = node_target(u);
            (t == name || (!id.is_empty() && t.eq_ignore_ascii_case(&id))) && (u.get("category").is_empty() || u.get("category") == category)
        })
        .cloned()
}

/// The data node that created a nested gear, and how. Found through the
/// data of the items it sits in.
fn maker_of(ch: &Character, store: &DataStore, doc: &Element, saved: &Element) -> Option<(Element, Maker)> {
    let owner = owner_of(&ch.doc, &saved.get("guid"), true)?;
    // Only data-created gear has a parent id. (Older saves gave gear in
    // armor and vehicles a parent id other than the owner's guid.)
    if saved.get("parentid").is_empty() {
        return None;
    }
    if owner.name == "gear" {
        if let Some(u) = record_of(doc, owner).and_then(|r| find_maker_node(r.el(), Maker::Child, saved)) {
            return Some((u, Maker::Child));
        }
        let (node, maker) = maker_of(ch, store, doc, owner)?;
        return find_maker_node(&node, maker, saved).map(|u| (u, maker));
    }
    let (file, container, item) = owner_data(owner)?;
    let data = store.doc(file).ok()?;
    let key = owner.child_text("sourceid").filter(|s| !s.is_empty()).unwrap_or_else(|| owner.get("name"));
    let rec = crate::data::find(&data, container, item, &key)?;
    let maker = if owner.name == "accessory" { Maker::Child } else { Maker::FromNode };
    let mut node = rec.el().clone();
    // Vehicles list `<gears><gear>`; the others `<gears><usegear>`.
    if owner.name != "vehicle" && maker == Maker::FromNode {
        if let Some(g) = node.child_mut("gears") {
            for u in g.elements_mut() {
                if u.name == "usegear" {
                    u.name = "gear".into();
                }
            }
        }
    }
    find_maker_node(&node, maker, saved).map(|u| (u, maker))
}

/// `Gear.CreateFromNode`: a gear that comes with an armor, armor mod,
/// cyberware or vehicle, from that item's `<gears>` entry. It is free, takes
/// no capacity unless `@consumecapacity`, and has `parentid` = `parent`.
pub fn from_node(ch: &Character, store: &DataStore, node: &Element, parent: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    let attrs = attribute_values(ch, store);
    from_node_in(&doc, &SheetAttributes(&attrs), node, parent)
}

fn from_node_in(doc: &Element, attrs: &dyn AttributeSource, node: &Element, parent: &Element) -> Option<Element> {
    let key = node.child_text("id").or_else(|| node.child_text("name")).unwrap_or_else(|| node.text());
    let rec = crate::data::find(doc, "gears", "gear", &key)?;
    let attr = |k: &str| node.attr(k).unwrap_or("");
    let rating = expr::trunc_int(expr::parse_plain(attr("rating")).or_else(|| node.get_f64("rating")).unwrap_or(0.0));
    let spec = Spec { rating, qty: Some(expr::parse_plain(attr("qty")).unwrap_or(1.0)), forced: attr("select").to_owned(), answer: None, parent: Some(parent), create_children: true };
    let mut g = build(doc, attrs, rec, &spec, &new_guid());
    if let Some(c) = node.child_text("capacity") {
        g.set_child_text("capacity", c);
    }
    if attr("consumecapacity") != "True" {
        for k in ["capacity", "armorcapacity"] {
            let old = g.get(k);
            let v = match old.find("/[") {
                Some(i) => format!("{}/[0]", &old[..i]),
                None => "[0]".into(),
            };
            g.set_child_text(k, v);
        }
    }
    g.set_child_text("cost", "0");
    if !attr("maxrating").is_empty() {
        g.set_child_text("maxrating", attr("maxrating"));
    }
    if parent.name == "vehicle" && rec.name() == "Sensor Array" && rec.category() == "Sensors" {
        // `MaxRatingValue`: a vehicle's sensor array follows its sensor rating.
        if let Some(sensor) = parent.get_i32("sensor") {
            g.set_child_text("rating", sensor.to_string());
        }
    }
    g.set_child_text("parentid", parent.get("guid"));
    if let Some(inner) = node.child("gears") {
        for n in inner.children_named("gear") {
            if let Some(c) = from_node_in(doc, attrs, n, &g) {
                g.child_or_insert("children").push(c);
            }
        }
    }
    Some(g)
}

/// `Gear.CreateChild`: a gear from a `<usegear>` of a gear or weapon
/// accessory record, free, with `parentid` = `parent`.
pub fn from_usegear(ch: &Character, store: &DataStore, usegear: &Element, parent: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    let attrs = attribute_values(ch, store);
    create_child(&doc, &SheetAttributes(&attrs), usegear, parent)
}

/// Oracle: rebuild a saved `<gear>` from its data record and the saved
/// choices (rating, quantity, extra, forced value).
pub fn rebuild(ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    let rec = record_of(&doc, saved)?;
    let guid = saved.get("guid");
    let owner = owner_of(&ch.doc, &guid, true);
    let attrs = attribute_values(ch, store);
    let attrs = SheetAttributes(&attrs);
    let made = maker_of(ch, store, &doc, saved).zip(owner).and_then(|((node, maker), o)| match maker {
        Maker::Child => create_child(&doc, &attrs, &node, o),
        Maker::FromNode => from_node_in(&doc, &attrs, &node, o),
    });
    let mut g = match made {
        Some(mut c) => {
            c.set_child_text("guid", guid.clone());
            c
        }
        _ => {
            let spec = Spec {
                rating: saved.get_i32("rating").unwrap_or(0),
                qty: None,
                forced: saved.get("forcedvalue"),
                answer: Some(saved.get("extra")),
                parent: owner,
                create_children: true,
            };
            let mut g = build(&doc, &attrs, rec, &spec, &guid);
            // Created by an item's data or a bonus (`addgear`): free.
            if !saved.get("parentid").is_empty() {
                g.set_child_text("cost", "0");
                g.set_child_text("parentid", saved.get("parentid"));
            }
            g
        }
    };
    g.set_child_text("qty", saved.get("qty"));
    legacy_ratings(saved, &mut g, owner, &attrs);
    if rec.el().get("cost").starts_with("Variable(") || saved.get("discountedcost") == "True" {
        g.set_child_text("cost", saved.get("cost"));
    }
    if rec.name() == "Custom Item" {
        g.set_child_text("name", saved.get("name"));
    }
    // A focus gets its extra when bonded, which is the player's doing.
    if matches!(rec.category().as_str(), "Foci" | "Metamagic Foci") {
        g.set_child_text("extra", saved.get("extra"));
    }
    // The player may swap a cyberdeck's attribute array.
    if saved.get_bool("canswapattributes").unwrap_or(false) {
        for f in ["attack", "sleaze", "dataprocessing", "firewall"] {
            g.set_child_text(f, saved.get(f));
        }
    }
    // Children the player added (not created by the data) come back as saved.
    if let Some(kids) = saved.child("children") {
        let list = g.child_or_insert("children");
        for k in kids.children_named("gear").filter(|k| k.get("parentid").is_empty()) {
            list.push(k.clone());
        }
    }
    Some(g)
}

/// 5.202 saves stored the rating limits evaluated to integers, 0 for none,
/// where current Chummer keeps the data expression. Compare like with like.
fn legacy_ratings(saved: &Element, g: &mut Element, parent: Option<&Element>, attrs: &dyn AttributeSource) {
    let ev = Ev::of(g, parent, attrs);
    let (min, max) = (g.get("minrating"), g.get("maxrating"));
    let max = match parent.filter(|p| p.name == "vehicle" && g.get("name") == "Sensor Array" && g.get("category") == "Sensors") {
        Some(v) => v.get_i32("sensor").unwrap_or(0),
        None if max.is_empty() => 0,
        None => max_rating_value(&max, &ev),
    };
    let values = [("minrating", if min.is_empty() { 0 } else { ev.int(&min) }), ("maxrating", max)];
    for (field, value) in values {
        let old = saved.get(field);
        if expr::parse_plain(&old).is_some() && !expr::needs_evaluation(&old) && old != g.get(field) {
            g.set_child_text(field, value.to_string());
        }
    }
}

/// `AddImprovementCollection.addgear`: create the gear (and the
/// `<children>` listed with it), each with a `Gear` improvement.
pub fn bonus_addgear(ctx: &mut crate::bonus::Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc(FILE) else { return false };
    let Some(mut g) = addgear_one(ctx, &doc, node, None) else { return false };
    if let Some(kids) = node.child("children") {
        for k in kids.elements() {
            let Some(c) = addgear_one(ctx, &doc, k, Some(&g)) else { return false };
            g.child_or_insert("children").push(c);
        }
    }
    ctx.out.added.push(("gears".into(), g));
    true
}

/// `addgear`'s local `Purchase`.
fn addgear_one(ctx: &mut crate::bonus::Ctx<'_>, doc: &Element, node: &Element, parent: Option<&Element>) -> Option<Element> {
    let rec = find_named(doc, &node.get("name"), &node.get("category"))?;
    let rating = node.child_text("rating").map_or(0, |r| ctx.int(&r));
    let qty = node.child_text("quantity").map_or(1.0, |q| ctx.dec(&q));
    let guid = new_guid();
    let forced = ctx.selected.clone().unwrap_or_default();
    let spec = Spec { rating, qty: Some(qty), forced: forced.clone(), answer: None, parent, create_children: true };
    let mut g = build(doc, &SheetAttributes(&ctx.attrs), rec, &spec, &guid);
    if node.child("fullcost").is_none() {
        g.set_child_text("cost", "0");
    }
    g.set_child_text("parentid", ctx.src.guid.clone());
    let out = apply_bonuses(ctx.ch, ctx.store, doc, &mut g, rating, Some(forced.as_str()).filter(|f| !f.is_empty()));
    merge(&mut ctx.out, out);
    let i = ctx.imp("Gear", &guid);
    ctx.push(i);
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gear(cost: &str, qty: &str, extra: &[(&str, &str)]) -> Element {
        let mut g = Element::new("gear");
        g.push(Element::with_text("cost", cost));
        g.push(Element::with_text("qty", qty));
        for (k, v) in extra {
            g.push(Element::with_text(*k, *v));
        }
        g
    }

    #[test]
    fn total_cost_counts_costfor_children_and_qty() {
        let mut parent = gear("Rating * 100", "2", &[("rating", "3")]);
        let mut kids = Element::new("children");
        kids.push(gear("Parent Rating * 10", "1", &[]));
        parent.push(kids);
        assert_eq!(cost(&parent), 300.0 * 2.0 + 30.0 * 2.0);
        assert_eq!(cost(&gear("45", "20", &[("costfor", "10")])), 90.0);
        assert_eq!(cost(&gear("100", "1", &[("discountedcost", "True")])), 90.0);
    }
}
