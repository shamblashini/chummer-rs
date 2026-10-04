//! Armor and armor mods (`Armor.Create` / `Armor.Save`,
//! `ArmorMod.Create` / `ArmorMod.Save`, `SelectArmor`, `SelectArmorMod`).
//!
//! An armor carries its mods in `<armormods>` and its gear in `<gears>`.
//! Mods listed in the data record's `<mods>` come with the armor
//! (`included`, no capacity, no cost).

use crate::bonus::{self, BonusSource, Choice};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::expr;
use crate::improvement::bool_str;
use crate::xml::Element;

use super::{apply_outcome, new_guid, Purchase};

/// Fields the oracle does not compare for armor and armor mods.
pub const IGNORE: &[&str] = &[
    // user input / per-instance state
    "armorname", "damage", "stolen",
    // lists that mix data-included and user-added children; every child is
    // checked on its own, and the included ones by unit tests
    "armormods", "gears",
    // link to a weapon created alongside; a new guid each time
    "weaponguid",
    // saves before 5.226 wrote the max rating as an int (0 = none); the
    // current Save writes the data string. Every fixture predates the
    // change. Included-mod max ratings are covered by unit tests.
    "maxrating",
];

/// `ColorTranslator.ToHtml(ColorManager.HasNotesColor)` with default settings.
pub(crate) const NOTES_COLOR: &str = "Chocolate";

/// Text of `k` in `e`, or `default` when the field is absent
/// (`TryGetStringFieldQuickly` keeps the field's initial value).
pub(crate) fn field(e: &Element, k: &str, default: &str) -> String {
    e.child_text(k).unwrap_or_else(|| default.to_owned())
}

/// A copy of a data node such as `<bonus>`, or an empty one
/// (`objWriter.WriteRaw(_nodBonus.OuterXml)` / `WriteElementString(.., "")`).
pub(crate) fn raw_node(e: &Element, k: &str) -> Element {
    e.child(k).cloned().unwrap_or_else(|| Element::new(k))
}

/// `Armor.ProcessRatingStringAsDec` / `ArmorMod.ProcessRatingStringAsDec`:
/// FixedValues, `Rating`, then evaluate.
pub fn rating_value(s: &str, rating: i32) -> f64 {
    if s.trim().is_empty() {
        return 0.0;
    }
    expr::value_to_dec(s.trim().trim_start_matches('+'), rating, &expr::NoAttributes)
}

/// `MaxRatingValue`: empty means unlimited.
fn max_rating_value(s: Option<&str>, rating: i32) -> i32 {
    match s {
        Some(m) if !m.is_empty() => expr::standard_round(rating_value(m, rating)),
        _ => i32::MAX,
    }
}

/// Cost from the record unless it is `Variable(...)`, where the buyer
/// picks the price (`Create`, Variable Cost branch). `chosen` is the price;
/// without one the minimum is used (`blnSkipSelectForms`).
pub(crate) fn resolved_cost(raw: &str, chosen: Option<&str>) -> String {
    match raw.strip_prefix("Variable(") {
        Some(rest) => chosen.map(str::to_owned).unwrap_or_else(|| {
            let inner = rest.trim_end_matches(')');
            inner.split('-').next().unwrap_or("0").trim_start_matches('+').to_owned()
        }),
        None => raw.to_owned(),
    }
}

/// The matrix fields shared by armor and weapons, in save order
/// (`devicerating` .. `canswapattributes`, read in `Create`).
pub(crate) fn matrix_fields(e: &Element) -> Vec<(&'static str, String)> {
    let mut v = vec![("devicerating", e.get("devicerating")), ("programlimit", e.get("programs")), ("overclocked", "None".to_owned())];
    let (mut a, mut s, mut d, mut f, mut swap) = (e.get("attack"), e.get("sleaze"), e.get("dataprocessing"), e.get("firewall"), false);
    let array = e.child_text("attributearray");
    if let Some(arr) = &array {
        let parts: Vec<&str> = arr.split(',').collect();
        let at = |i: usize| parts.get(i).copied().unwrap_or("").to_owned();
        (a, s, d, f, swap) = (at(0), at(1), at(2), at(3), true);
    }
    v.extend([("attack", a), ("sleaze", s), ("dataprocessing", d), ("firewall", f), ("attributearray", array.unwrap_or_default())]);
    for k in ["modattack", "modsleaze", "moddataprocessing", "modfirewall", "modattributearray"] {
        v.push((k, e.get(k)));
    }
    v.push(("canswapattributes", bool_str(swap)));
    v
}

// ---------------------------------------------------------------------------
// Armor
// ---------------------------------------------------------------------------

/// Build an `<armor>` element with an empty `<armormods>` and no gear
/// (`Armor.Create` + `Armor.Save`). `cost` overrides a `Variable(...)` cost.
pub fn armor_element(rec: Record<'_>, guid: &str, rating: i32, extra: &str, cost: Option<&str>) -> Element {
    let e = rec.el();
    let max_rating = e.child_text("rating");
    let rating = rating.min(max_rating_value(max_rating.as_deref(), rating));
    let mut over = e.get("armoroverride");
    if over == "0" {
        over.clear();
    }
    let encumbrance = e.get_bool("encumbrance").or_else(|| e.get_bool("emcumbrance")).unwrap_or(true);
    let mut a = Element::new("armor");
    let mut put = |k: &str, v: String| a.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", rec.category());
    put("armor", field(e, "armor", "0"));
    put("armoroverride", over);
    put("armorcapacity", field(e, "armorcapacity", "0"));
    put("avail", e.get("avail"));
    put("cost", resolved_cost(&e.get("cost"), cost));
    put("weight", e.get("weight"));
    put("source", rec.source());
    put("page", rec.page());
    put("armorname", String::new());
    put("equipped", bool_str(true));
    put("active", bool_str(false));
    put("homenode", bool_str(false));
    for (k, v) in matrix_fields(e) {
        put(k, v);
    }
    put("matrixcmfilled", "0".into());
    put("matrixcmbonus", e.get_i32("matrixcmbonus").unwrap_or(0).to_string());
    put("wirelesson", bool_str(false));
    put("canformpersona", e.get("canformpersona"));
    put("extra", extra.to_owned());
    put("damage", "0".into());
    put("rating", rating.to_string());
    put("maxrating", max_rating.unwrap_or_default());
    put("ratinglabel", field(e, "ratinglabel", "String_Rating"));
    put("stolen", bool_str(false));
    put("encumbrance", bool_str(encumbrance));
    a.push(Element::new("armormods"));
    a.push(raw_node(e, "bonus"));
    a.push(raw_node(e, "wirelessbonus"));
    let mut put = |k: &str, v: String| a.push(Element::with_text(k, v));
    put("location", String::new());
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("notesColor", NOTES_COLOR.into());
    put("discountedcost", bool_str(false));
    put("sortorder", "0".into());
    a
}

/// One `<mods><name rating=".." select=".." maxrating="..">` entry of an
/// armor record.
#[derive(Debug, Clone, Default)]
pub struct IncludedMod {
    pub name: String,
    pub rating: i32,
    pub select: String,
    pub maxrating: Option<String>,
}

/// The mods an armor record comes with (`Armor.Create`, "Add any Armor
/// Mods that come with the Armor").
pub fn included_mods(rec: Record<'_>) -> Vec<IncludedMod> {
    let Some(mods) = rec.el().child("mods") else { return Vec::new() };
    mods.children_named("name")
        .map(|n| IncludedMod {
            name: n.text(),
            // int.TryParse into the shared rating: absent means 0.
            rating: n.attr("rating").and_then(|r| r.trim().parse().ok()).unwrap_or(0),
            select: n.attr("select").unwrap_or_default().to_owned(),
            maxrating: n.attr("maxrating").filter(|m| !m.is_empty()).map(str::to_owned),
        })
        .collect()
}

/// Make a mod element "included in armor": no capacity, no cost, and a
/// max rating fixed at the current rating unless the data gives one (in
/// which case the data rating bypasses the clamp).
fn mark_included(m: &mut Element, inc: Option<&IncludedMod>) {
    m.set_child_text("included", bool_str(true));
    m.set_child_text("armorcapacity", "[0]");
    m.set_child_text("cost", "0");
    match inc.and_then(|i| i.maxrating.as_ref().map(|mx| (mx, i.rating))) {
        Some((mx, r)) => {
            m.set_child_text("maxrating", mx.clone());
            m.set_child_text("rating", r.to_string());
        }
        None => {
            let r = m.get("rating");
            m.set_child_text("maxrating", r);
        }
    }
}

/// A mod element for an included mod name that has no record
/// (`Armor.Create`: the "Features" placeholder named after the armor).
fn feature_mod(armor: Record<'_>, inc: &IncludedMod, guid: &str) -> Element {
    let mut m = Element::new("armormod");
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("guid", guid.to_owned());
    put("sourceid", String::new());
    put("name", armor.name());
    put("category", "Features".into());
    put("armor", "0".into());
    put("armorcapacity", "[0]".into());
    put("gearcapacity", String::new());
    put("maxrating", inc.maxrating.clone().unwrap_or_default());
    put("rating", "0".into());
    put("ratinglabel", "String_Rating".into());
    put("avail", "0".into());
    put("cost", "0".into());
    put("weight", String::new());
    m.push(Element::new("bonus"));
    m.push(Element::new("wirelessbonus"));
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("wirelesson", bool_str(false));
    put("source", armor.source());
    put("page", armor.page());
    put("included", bool_str(true));
    put("equipped", bool_str(true));
    put("extra", inc.select.clone());
    put("encumbrance", bool_str(true));
    put("stolen", bool_str(false));
    put("notes", String::new());
    put("notesColor", NOTES_COLOR.into());
    put("discountedcost", bool_str(false));
    put("sortorder", "0".into());
    m
}

/// Build the included mod elements for an armor record (no bonuses).
pub fn included_mod_elements(doc: &Element, armor: Record<'_>) -> Vec<Element> {
    included_mods(armor)
        .iter()
        .map(|inc| match data::find(doc, "mods", "mod", &inc.name) {
            Some(rec) => {
                let mut m = armormod_element(rec, &new_guid(), inc.rating, &inc.select, None);
                mark_included(&mut m, Some(inc));
                m
            }
            None => feature_mod(armor, inc, &new_guid()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Armor mods
// ---------------------------------------------------------------------------

/// Build an `<armormod>` element (`ArmorMod.Create` + `ArmorMod.Save`).
pub fn armormod_element(rec: Record<'_>, guid: &str, rating: i32, extra: &str, cost: Option<&str>) -> Element {
    let e = rec.el();
    let max_rating = e.child_text("maxrating");
    let rating = rating.min(max_rating_value(max_rating.as_deref(), rating));
    let mut m = Element::new("armormod");
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("guid", guid.to_owned());
    put("sourceid", rec.id());
    put("name", rec.name());
    put("category", rec.category());
    put("armor", e.get_i32("armor").unwrap_or(0).to_string());
    put("armorcapacity", field(e, "armorcapacity", "[0]"));
    put("gearcapacity", e.get("gearcapacity"));
    put("maxrating", max_rating.unwrap_or_default());
    put("rating", rating.to_string());
    put("ratinglabel", field(e, "ratinglabel", "String_Rating"));
    put("avail", e.get("avail"));
    put("cost", resolved_cost(&e.get("cost"), cost));
    put("weight", e.get("weight"));
    m.push(raw_node(e, "bonus"));
    m.push(raw_node(e, "wirelessbonus"));
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("wirelesson", bool_str(false));
    put("source", rec.source());
    put("page", rec.page());
    put("included", bool_str(false));
    put("equipped", bool_str(true));
    put("extra", extra.to_owned());
    put("encumbrance", bool_str(e.get_bool("encumbrance").unwrap_or(true)));
    put("stolen", bool_str(false));
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("notesColor", NOTES_COLOR.into());
    put("discountedcost", bool_str(false));
    put("sortorder", "0".into());
    m
}

// ---------------------------------------------------------------------------
// Selection, adding
// ---------------------------------------------------------------------------

/// Choices needed before adding an armor or a mod: its bonus selection.
pub fn choices(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    let kind = if tag == "armormod" { "ArmorMod" } else { "Armor" };
    let src = BonusSource { kind: kind.into(), guid: String::new(), name: rec.name(), rating: p.rating };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Apply a record's bonus for a new item (`ImprovementManager.CreateImprovements`);
/// returns the selected value.
fn apply_bonus(ch: &mut Character, store: &DataStore, rec: Record<'_>, kind: &str, guid: &str, rating: i32, answer: Option<&str>) -> Option<String> {
    let b = rec.el().child("bonus").filter(|b| b.elements().next().is_some())?;
    let src = BonusSource { kind: kind.into(), guid: guid.to_owned(), name: rec.name(), rating };
    let out = bonus::apply(ch, store, b, &src, answer);
    for (container, el) in out.added.iter().cloned() {
        ch.items_mut(&container).push(el);
    }
    apply_outcome(ch, &out);
    out.selected
}

/// Add an armor (with its mods, gear and weapons) or a mod to an existing
/// armor (`Purchase.parent`). Returns the new guid.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    if tag == "armormod" {
        let parent = p.parent.clone().ok_or("an armor mod needs a parent armor")?;
        return add_mod(ch, store, rec, &parent, p);
    }
    let doc = store.doc("armor.xml").map_err(|e| e.to_string())?;
    let guid = new_guid();
    let selected = apply_bonus(ch, store, rec, "Armor", &guid, p.rating, p.answer.as_deref());
    let extra = selected.or_else(|| p.answer.clone()).unwrap_or_default();
    let mut a = armor_element(rec, &guid, p.rating, &extra, None);
    let mods: Vec<Element> = included_mods(rec).iter().map(|inc| add_included_mod(ch, store, &doc, rec, inc)).collect();
    if let Some(c) = a.child_mut("armormods") {
        mods.into_iter().for_each(|m| c.push(m));
    }
    if p.free {
        a.set_child_text("cost", "0");
    }
    ch.items_mut("armors").push(a);
    add_gear(ch, store, rec.el(), &guid);
    add_weapons(ch, store, rec.el(), &guid);
    Ok(guid)
}

/// One included mod of a new armor, with its bonus applied.
fn add_included_mod(ch: &mut Character, store: &DataStore, doc: &Element, armor: Record<'_>, inc: &IncludedMod) -> Element {
    let Some(rec) = data::find(doc, "mods", "mod", &inc.name) else {
        return feature_mod(armor, inc, &new_guid());
    };
    let guid = new_guid();
    let answer = Some(inc.select.as_str()).filter(|s| !s.is_empty());
    let selected = apply_bonus(ch, store, rec, "ArmorMod", &guid, inc.rating, answer);
    let extra = selected.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| inc.select.clone());
    let mut m = armormod_element(rec, &guid, inc.rating, &extra, None);
    mark_included(&mut m, Some(inc));
    m
}

/// Add a mod to the armor with guid `parent` (`SelectArmorMod` result).
fn add_mod(ch: &mut Character, store: &DataStore, rec: Record<'_>, parent: &str, p: &Purchase) -> Result<String, String> {
    if super::find_by_guid_mut(&mut ch.doc, parent).is_none_or(|a| a.name != "armor") {
        return Err(format!("no armor with guid {parent}"));
    }
    let guid = new_guid();
    let selected = apply_bonus(ch, store, rec, "ArmorMod", &guid, p.rating, p.answer.as_deref());
    let extra = selected.or_else(|| p.answer.clone()).unwrap_or_default();
    let mut m = armormod_element(rec, &guid, p.rating, &extra, None);
    if p.free {
        m.set_child_text("cost", "0");
    }
    let armor = super::find_by_guid_mut(&mut ch.doc, parent).ok_or("armor vanished")?;
    armor.child_or_insert("armormods").push(m);
    ch.dirty = true;
    add_gear(ch, store, rec.el(), &guid);
    add_weapons(ch, store, rec.el(), &guid);
    Ok(guid)
}

/// Gear that comes with an armor or mod (`<gears><usegear>`). Gear is
/// built by `items::gear` with `Purchase.parent`; while that kind cannot
/// add gear yet, the entries are skipped.
fn add_gear(ch: &mut Character, store: &DataStore, rec: &Element, parent: &str) {
    let Some(gears) = rec.child("gears") else { return };
    let Ok(doc) = store.doc("gear.xml") else { return };
    for ug in gears.children_named("usegear") {
        let name_node = ug.child("name");
        let name = name_node.map_or_else(|| ug.text(), Element::text);
        let Some(g) = data::find(&doc, "gears", "gear", name.trim()) else { continue };
        let p = Purchase {
            rating: ug.attr("rating").and_then(|r| r.trim().parse().ok()).or_else(|| ug.get_i32("rating")).unwrap_or(0),
            qty: name_node.and_then(|n| n.attr("qty")).and_then(|q| q.parse().ok()).unwrap_or(1.0),
            parent: Some(parent.to_owned()),
            answer: name_node.and_then(|n| n.attr("select")).or_else(|| ug.attr("select")).map(str::to_owned),
            cost_multiplier: 1.0,
            ..Default::default()
        };
        let _ = super::gear::add(ch, store, g, &p);
    }
}

/// Weapons an armor or mod adds (`<addweapon>`): top-level weapons with
/// `parentid` = the item and no cost; the last one is linked by `weaponguid`.
fn add_weapons(ch: &mut Character, store: &DataStore, rec: &Element, parent: &str) {
    let rating = super::find_by_guid_mut(&mut ch.doc, parent).and_then(|e| e.get_i32("rating")).unwrap_or(0);
    let mut last = None;
    for aw in rec.children_named("addweapon") {
        let r = aw.attr("rating").map(|s| expr::standard_round(rating_value(&s.replace("{Rating}", &rating.to_string()), rating))).unwrap_or(0);
        if let Some(g) = super::weapon::add_child_weapon(ch, store, &aw.text(), r, parent) {
            last = Some(g);
        }
    }
    if let (Some(w), Some(item)) = (last, super::find_by_guid_mut(&mut ch.doc, parent)) {
        item.set_child_text("weaponguid", w);
    }
}

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

/// The saved armor that holds the armor mod `guid`.
fn parent_armor<'a>(doc: &'a Element, guid: &str) -> Option<&'a Element> {
    let mut out = Vec::new();
    doc.descendants("armor", &mut out);
    out.into_iter().find(|a| a.child("armormods").is_some_and(|ms| ms.children_named("armormod").any(|m| m.get("guid").eq_ignore_ascii_case(guid))))
}

/// Find a record by saved `sourceid`, else by name (old saves have no
/// `sourceid` on mods).
pub(crate) fn find_saved<'a>(doc: &'a Element, container: &str, item: &'a str, saved: &Element) -> Option<Record<'a>> {
    let id = saved.get("sourceid");
    (!id.is_empty()).then(|| data::find(doc, container, item, &id)).flatten().or_else(|| data::find(doc, container, item, &saved.get("name")))
}

/// Saved cost when the record's cost is chosen by the buyer.
pub(crate) fn saved_cost(rec: Record<'_>, saved: &Element) -> Option<String> {
    rec.get("cost").starts_with("Variable(").then(|| saved.get("cost"))
}

/// Oracle: rebuild a saved `<armor>` or `<armormod>` from its data record
/// plus the choices stored in it (rating, extra, cost of variable-cost
/// items, and whether a mod came with its armor).
pub fn rebuild(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("armor.xml").ok()?;
    let rating = saved.get_i32("rating").unwrap_or(0);
    let extra = saved.get("extra");
    if tag == "armor" {
        let rec = find_saved(&doc, "armors", "armor", saved)?;
        let mut a = armor_element(rec, &saved.get("guid"), rating, &extra, saved_cost(rec, saved).as_deref());
        if let Some(c) = a.child_mut("armormods") {
            included_mod_elements(&doc, rec).into_iter().for_each(|m| c.push(m));
        }
        return Some(a);
    }
    let Some(rec) = find_saved(&doc, "mods", "mod", saved) else {
        return rebuild_feature_mod(ch, &doc, saved);
    };
    let mut m = armormod_element(rec, &saved.get("guid"), rating, &extra, saved_cost(rec, saved).as_deref());
    if saved.get_bool("included").unwrap_or(false) {
        let inc = included_entry(ch, &doc, saved);
        mark_included(&mut m, inc.as_ref());
    }
    Some(m)
}

/// The data `<mods><name>` entry of the parent armor for a saved mod.
fn included_entry(ch: &Character, doc: &Element, saved: &Element) -> Option<IncludedMod> {
    let armor = parent_armor(&ch.doc, &saved.get("guid"))?;
    let rec = find_saved(doc, "armors", "armor", armor)?;
    included_mods(rec).into_iter().find(|i| i.name == saved.get("name"))
}

/// A saved "Features" placeholder mod (named after its armor).
fn rebuild_feature_mod(ch: &Character, doc: &Element, saved: &Element) -> Option<Element> {
    let armor = parent_armor(&ch.doc, &saved.get("guid"))?;
    let rec = find_saved(doc, "armors", "armor", armor)?;
    let inc = included_mods(rec).into_iter().find(|i| data::find(doc, "mods", "mod", &i.name).is_none())?;
    Some(feature_mod(rec, &inc, &saved.get("guid")))
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// `OwnCost` of an armor or mod: cost at its rating, 10% off when
/// discounted.
fn own_cost(e: &Element) -> f64 {
    let c = rating_value(&e.get("cost"), e.get_i32("rating").unwrap_or(0));
    if e.get_bool("discountedcost").unwrap_or(false) { c * 0.9 } else { c }
}

/// Saved gear cost × quantity, with its children (stand-in for
/// `Gear.TotalCost` until `items::gear` provides one).
pub(crate) fn gear_cost(g: &Element) -> f64 {
    let qty = g.get_f64("qty").unwrap_or(1.0);
    let own = rating_value(&g.get("cost"), g.get_i32("rating").unwrap_or(0));
    let kids: f64 = g.child("children").map(|c| c.children_named("gear").map(gear_cost).sum()).unwrap_or(0.0);
    own * qty + kids
}

/// `Armor.TotalCost` / `ArmorMod.TotalCost`: own cost plus mods and gear.
pub fn cost(e: &Element) -> f64 {
    let mods: f64 = e.child("armormods").map(|c| c.children_named("armormod").map(cost).sum()).unwrap_or(0.0);
    let gear: f64 = e.child("gears").map(|c| c.children_named("gear").map(gear_cost).sum()).unwrap_or(0.0);
    own_cost(e) + mods + gear
}
