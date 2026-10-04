//! Weapons and weapon accessories (`Weapon.Create` / `Weapon.Save`,
//! `WeaponAccessory.Create` / `WeaponAccessory.Save`, `SelectWeapon`,
//! `SelectWeaponAccessory`), their derived combat values
//! (`Weapon.CalculatedDamage`, `TotalAP`, `GetTotalAccuracy`, `TotalRC`,
//! `GetDicePool`, `TotalReach`, `GetRangeStrings`) and cost.
//!
//! A weapon carries its accessories in `<accessories>` and each
//! underbarrel weapon in its own `<underbarrel>`. Accessories and
//! underbarrels listed in the data record come with the weapon
//! (`included`); weapons a record `<addweapon>`s are separate top-level
//! weapons whose `parentid` points back at it.

use std::sync::OnceLock;

use crate::bonus::{Choice, Ctx};
use crate::calc::{self, Sheet};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::expr::{self, AttributeSource};
use crate::improvement::{bool_str, fmt_num, Query};
use crate::xml::Element;

use super::armor::{field, find_saved, gear_cost, matrix_fields, raw_node, resolved_cost, saved_cost, NOTES_COLOR};
use super::{new_guid, Purchase};

/// Fields the oracle does not compare for weapons and accessories.
pub const IGNORE: &[&str] = &[
    // runtime ammunition state and user input
    "clips", "activeammoslot", "weaponname", "stolen",
    // legacy fields older versions wrote and the current Save does not
    "ammoname", "addmode", "ammoloaded", "ammoremaining", "installed",
    // saves before 5.226 wrote accessory accuracy and ammo bonus as ints
    // ("0" = none); the current Save writes the data string, empty for
    // none. Every fixture predates the change. Weapon accuracy is a plain
    // copy of the data field.
    "accuracy", "ammobonus",
    // lists that mix data-included and user-added children; every child
    // is checked on its own, and the included ones by unit tests
    "accessories", "gears",
];

/// Data file for weapons and accessories.
const FILE: &str = "weapons.xml";

// ---------------------------------------------------------------------------
// Weapon element
// ---------------------------------------------------------------------------

/// `Weapon.Create`: `weapontype` from the record, else the `type`
/// attribute of its category, else the category in lower case.
fn weapon_type(doc: &Element, rec: Record<'_>) -> String {
    if let Some(t) = rec.el().child_text("weapontype") {
        return t;
    }
    let cat = rec.category();
    doc.child("categories")
        .and_then(|c| c.children_named("category").find(|c| c.text() == cat))
        .and_then(|c| c.attr("type").map(str::to_owned))
        .unwrap_or_else(|| cat.to_lowercase())
}

/// Join `<X><mount>a</mount><mount>b</mount></X>` as `a/b`.
fn mounts(e: &Element, k: &str) -> String {
    e.child(k).map(|m| m.children_named("mount").map(Element::text).collect::<Vec<_>>().join("/")).unwrap_or_default()
}

/// `MinRatingValue` / `MaxRatingValue` of a weapon.
fn clamp_rating(e: &Element, rating: i32) -> i32 {
    let eval = |s: &str| expr::standard_round(expr::value_to_dec(s.trim(), rating, &expr::NoAttributes));
    let max = e.child_text("rating").filter(|m| !m.is_empty() && m != "0").map_or(i32::MAX, |m| eval(&m));
    let min = e.child_text("minrating").filter(|m| !m.is_empty()).map_or(0, |m| eval(&m));
    rating.min(max).max(min)
}

/// What a weapon element is built with besides its record.
#[derive(Debug, Clone, Default)]
pub struct WeaponSpec {
    pub guid: String,
    pub rating: i32,
    /// The saved cost when it is not the record's: the price chosen for a
    /// `Variable(...)` cost, or "0" for weapons paid through their parent.
    pub cost: Option<String>,
    pub parentid: String,
    /// Comes with its parent weapon (underbarrel).
    pub included: bool,
    /// `AllowAccessory` forced off by the parent weapon.
    pub no_accessories: bool,
    pub accessories: Vec<Element>,
    pub underbarrels: Vec<Element>,
}

/// Build a `<weapon>` element (`Weapon.Create` + `Weapon.Save`).
pub fn weapon_element(doc: &Element, rec: Record<'_>, spec: WeaponSpec) -> Element {
    let e = rec.el();
    let rating = clamp_rating(e, spec.rating);
    let mut ammo = e.get("ammo");
    if e.get("useskill") == "Throwing Weapons" && ammo != "1" {
        ammo = "1".into();
    }
    let flag = |k: &str, d: bool| bool_str(e.get_bool(k).unwrap_or(d));
    let int = |k: &str, d: i32| e.get_i32(k).unwrap_or(d).to_string();
    let range = e.child("range");
    let multiply = range.and_then(|r| r.attr("multiply")).and_then(|m| m.trim().parse::<f64>().ok()).unwrap_or(1.0);
    let max_rating = e.child_text("rating").filter(|m| m != "0").unwrap_or_default();
    let cost = spec.cost.clone().unwrap_or_else(|| resolved_cost(&e.get("cost"), None));
    let mut w = Element::new("weapon");
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", spec.guid.clone());
    put("name", rec.name());
    put("category", rec.category());
    put("type", e.get("type"));
    put("spec", e.get("spec"));
    put("spec2", e.get("spec2"));
    put("reach", field(e, "reach", "0"));
    put("damage", e.get("damage"));
    put("ap", field(e, "ap", "0"));
    put("mode", e.get("mode"));
    put("rc", e.get("rc"));
    put("ammo", ammo);
    put("cyberware", flag("cyberware", false));
    put("ammocategory", e.get("ammocategory"));
    put("ammoslots", int("ammoslots", 1));
    put("sizecategory", e.get("sizecategory"));
    put("firingmode", "Skill".into());
    put("minrating", e.get("minrating"));
    put("maxrating", max_rating);
    put("rating", rating.to_string());
    put("accuracy", e.get("accuracy"));
    put("activeammoslot", "1".into());
    put("conceal", field(e, "conceal", "0"));
    put("avail", e.get("avail"));
    put("cost", cost);
    put("weight", e.get("weight"));
    put("useskill", e.get("useskill"));
    put("useskillspec", e.get("useskillspec"));
    put("range", range.map(|r| r.text().trim().to_owned()).unwrap_or_default());
    put("alternaterange", e.get("alternaterange").trim().to_owned());
    put("rangemultiply", fmt_num(multiply));
    for (k, d) in [("singleshot", 1), ("shortburst", 3), ("longburst", 6), ("fullburst", 10), ("suppressive", 20)] {
        put(k, int(k, d));
    }
    for k in ["allowsingleshot", "allowshortburst", "allowlongburst", "allowfullburst", "allowsuppressive"] {
        put(k, flag(k, true));
    }
    put("source", rec.source());
    put("page", rec.page());
    put("parentid", spec.parentid.clone());
    put("allowaccessory", bool_str(e.get_bool("allowaccessory").unwrap_or(true) && !spec.no_accessories));
    put("weaponname", String::new());
    put("included", bool_str(spec.included));
    put("equipped", bool_str(true));
    put("requireammo", flag("requireammo", true));
    put("accuracy", e.get("accuracy"));
    put("mount", e.get("mount"));
    put("stolen", flag("stolen", false));
    put("extramount", e.get("extramount"));
    if !spec.accessories.is_empty() {
        let mut acc = Element::new("accessories");
        spec.accessories.into_iter().for_each(|a| acc.push(a));
        w.push(acc);
    }
    for u in spec.underbarrels {
        let mut ub = Element::new("underbarrel");
        ub.push(u);
        w.push(ub);
    }
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("location", String::new());
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("notesColor", NOTES_COLOR.into());
    put("discountedcost", bool_str(false));
    put("weaponslots", mounts(e, "accessorymounts"));
    put("doubledcostweaponslots", mounts(e, "doubledcostaccessorymounts"));
    put("active", bool_str(false));
    put("homenode", bool_str(false));
    for (k, v) in matrix_fields(e) {
        put(k, v);
    }
    put("matrixcmfilled", "0".into());
    w.push(raw_node(e, "wirelessbonus"));
    w.push(raw_node(e, "wirelessweaponbonus"));
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("wirelesson", flag("wirelesson", true));
    put("sortorder", "0".into());
    put("weapontype", weapon_type(doc, rec));
    w
}

/// The accessories a weapon record comes with (`Weapon.Create`, "If there
/// are any Accessories that come with the Weapon, add them"). The mount is
/// the accessory record's own mount when the entry names one, else
/// `Internal`.
pub fn included_accessories(doc: &Element, rec: Record<'_>) -> Vec<Element> {
    let Some(list) = rec.el().child("accessories") else { return Vec::new() };
    list.children_named("accessory")
        .filter_map(|entry| {
            let name = entry.get("name");
            let acc = data::find(doc, "accessories", "accessory", &name).filter(|_| !name.is_empty())?;
            let (mount, extra) = if entry.child("mount").is_some() {
                let extra = if entry.child("extramount").is_some() { acc.get("extramount") } else { "None".into() };
                (acc.get("mount"), extra)
            } else {
                ("Internal".to_owned(), "None".to_owned())
            };
            let rating = entry.get_i32("rating").unwrap_or(0);
            let mut a = accessory_element(acc, &new_guid(), &mount, &extra, rating, None);
            a.set_child_text("included", bool_str(true));
            Some(a)
        })
        .collect()
}

/// The underbarrel weapons a record comes with: no cost, `included`,
/// no accessories when the parent allows none.
pub fn included_underbarrels(doc: &Element, rec: Record<'_>, parent_guid: &str) -> Vec<Element> {
    let Some(list) = rec.el().child("underbarrels") else { return Vec::new() };
    let no_acc = !rec.el().get_bool("allowaccessory").unwrap_or(true);
    list.elements()
        .filter_map(|n| data::find(doc, "weapons", "weapon", &n.text()))
        .map(|ub| {
            let guid = new_guid();
            let spec = WeaponSpec { guid: guid.clone(), cost: Some("0".into()), parentid: parent_guid.to_owned(), included: true, no_accessories: no_acc, ..Default::default() };
            weapon_tree(doc, ub, spec)
        })
        .collect()
}

/// A weapon with the accessories and underbarrels its record comes with.
pub fn weapon_tree(doc: &Element, rec: Record<'_>, mut spec: WeaponSpec) -> Element {
    spec.accessories = included_accessories(doc, rec);
    spec.underbarrels = included_underbarrels(doc, rec, &spec.guid);
    weapon_element(doc, rec, spec)
}

// ---------------------------------------------------------------------------
// Accessory element
// ---------------------------------------------------------------------------

/// Build an `<accessory>` element (`WeaponAccessory.Create` + `Save`).
/// `cost` is the price chosen for a `Variable(...)` cost.
pub fn accessory_element(rec: Record<'_>, guid: &str, mount: &str, extramount: &str, rating: i32, cost: Option<&str>) -> Element {
    let e = rec.el();
    let max_rating = e.child_text("rating");
    let max = max_rating.as_deref().filter(|m| !m.is_empty()).map_or(i32::MAX, |m| expr::standard_round(super::armor::rating_value(m, rating)));
    let rating = rating.min(max);
    let zero_empty = |k: &str| {
        let v = e.get(k);
        if matches!(v.as_str(), "0" | "+0" | "-0") { String::new() } else { v }
    };
    let mut firemode = e.get("firemode");
    if let Some(add) = e.child_text("addmode") {
        if firemode.is_empty() {
            firemode = add;
        } else if !firemode.contains(&add) {
            firemode = format!("{firemode}/{add}");
        }
    }
    let int = |k: &str| e.get_i32(k).unwrap_or(0).to_string();
    let mut a = Element::new("accessory");
    let mut put = |k: &str, v: String| a.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("mount", mount.to_owned());
    put("extramount", extramount.to_owned());
    put("addmount", e.get("addmount"));
    put("rc", e.get("rc"));
    put("maxrating", max_rating.unwrap_or_default());
    put("rating", rating.to_string());
    put("ratinglabel", field(e, "ratinglabel", "String_Rating"));
    put("rcgroup", int("rcgroup"));
    put("rcdeployable", bool_str(e.get_bool("rcdeployable").unwrap_or(false)));
    put("specialmodification", bool_str(e.get_bool("specialmodification").unwrap_or(false)));
    put("conceal", e.get("conceal"));
    if !e.get("dicepool").is_empty() {
        put("dicepool", e.get("dicepool"));
    }
    put("avail", e.get("avail"));
    put("cost", resolved_cost(&field(e, "cost", "0"), cost));
    put("weight", e.get("weight"));
    put("included", bool_str(false));
    put("equipped", bool_str(true));
    if let Some(g) = e.child("allowgear") {
        a.push(g.clone());
    }
    let mut put = |k: &str, v: String| a.push(Element::with_text(k, v));
    put("source", rec.source());
    put("page", rec.page());
    put("accuracy", zero_empty("accuracy"));
    put("ammoreplace", e.get("ammoreplace"));
    put("ammoslots", int("ammoslots"));
    put("modifyammocapacity", e.get("modifyammocapacity"));
    put("damagetype", e.get("damagetype"));
    put("damage", e.get("damage"));
    put("reach", zero_empty("reach"));
    put("damagereplace", e.get("damagereplace"));
    put("firemode", firemode);
    put("firemodereplace", e.get("firemodereplace"));
    put("ap", e.get("ap"));
    put("apreplace", e.get("apreplace"));
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("notesColor", NOTES_COLOR.into());
    put("discountedcost", bool_str(false));
    for k in ["singleshot", "shortburst", "longburst", "fullburst", "suppressive"] {
        put(k, int(k));
    }
    put("replacerange", e.get("replacerange"));
    put("rangebonus", field(e, "rangebonus", "0"));
    put("rangemodifier", field(e, "rangemodifier", "0"));
    put("extra", e.get("extra"));
    put("ammobonus", e.get("ammobonus"));
    put("wirelesson", bool_str(true));
    a.push(raw_node(e, "wirelessbonus"));
    a.push(raw_node(e, "wirelessweaponbonus"));
    let mut put = |k: &str, v: String| a.push(Element::with_text(k, v));
    put("stolen", bool_str(false));
    put("sortorder", "0".into());
    put("parentid", String::new());
    a
}

// ---------------------------------------------------------------------------
// Selection, adding
// ---------------------------------------------------------------------------

/// Mount slots an accessory can use on `weapon`: the accessory's
/// `mount` options that the weapon has and no equipped accessory holds
/// (`SelectWeaponAccessory` mount list). `None` when the accessory needs
/// no slot.
pub fn mount_options(weapon: &Element, acc: Record<'_>) -> Vec<String> {
    let wanted = acc.get("mount");
    if wanted.is_empty() {
        return vec!["None".into()];
    }
    let slots: Vec<String> = weapon.get("weaponslots").split('/').map(str::to_owned).collect();
    let used: Vec<String> = weapon
        .child("accessories")
        .map(|c| c.children_named("accessory").flat_map(|a| [a.get("mount"), a.get("extramount")]).collect())
        .unwrap_or_default();
    wanted.split('/').filter(|m| slots.iter().any(|s| s == m) && !used.iter().any(|u| u == m)).map(str::to_owned).collect()
}

/// Choices needed before adding: the mount for an accessory.
pub fn choices(tag: &str, ch: &Character, _store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    if tag != "accessory" {
        return Vec::new();
    }
    let Some(w) = p.parent.as_deref().and_then(|g| find_by_guid(&ch.doc, g)) else { return Vec::new() };
    let options = mount_options(w, rec);
    if options.len() <= 1 {
        return Vec::new();
    }
    vec![Choice { node: "mount".into(), prompt: format!("Mount for {}", rec.name()), options }]
}

/// Find any saved element by guid (immutable `items::find_by_guid_mut`).
fn find_by_guid<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_by_guid(c, guid))
}

/// Add a weapon (top level, or as an underbarrel of `Purchase.parent`) or
/// an accessory to the weapon `Purchase.parent` (mount from
/// `Purchase.answer`). Returns the new guid.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    if tag == "accessory" {
        let parent = p.parent.clone().ok_or("an accessory needs a parent weapon")?;
        let w = find_by_guid(&ch.doc, &parent).ok_or_else(|| format!("no weapon with guid {parent}"))?;
        let mount = match p.answer.clone().or_else(|| mount_options(w, rec).into_iter().next()) {
            Some(m) => m,
            None => return Err(format!("no free mount for {} on this weapon", rec.name())),
        };
        let extra = rec.el().child_text("extramount").map(|x| x.split('/').next().unwrap_or("None").to_owned()).unwrap_or_else(|| "None".into());
        return add_accessory(ch, store, rec, &parent, &mount, &extra, p.rating, p.free);
    }
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let guid = new_guid();
    let mut spec = WeaponSpec { guid: guid.clone(), rating: p.rating, ..Default::default() };
    if p.free {
        spec.cost = Some("0".into());
    }
    match p.parent.as_deref() {
        Some(parent) => {
            spec.parentid = parent.to_owned();
            let w = weapon_tree(&doc, rec, spec);
            let host = super::find_by_guid_mut(&mut ch.doc, parent).ok_or_else(|| format!("no weapon with guid {parent}"))?;
            let mut ub = Element::new("underbarrel");
            ub.push(w);
            host.push(ub);
            ch.dirty = true;
        }
        None => {
            let w = weapon_tree(&doc, rec, spec);
            ch.items_mut("weapons").push(w);
        }
    }
    add_accessory_gear(ch, store, &guid);
    for aw in rec.el().children_named("addweapon") {
        let r = aw.attr("rating").map(|s| expr::standard_round(super::armor::rating_value(s, p.rating))).unwrap_or(0);
        add_child_weapon(ch, store, &aw.text(), r, &guid);
    }
    Ok(guid)
}

/// Add an accessory to the weapon `parent` at the given mount
/// (`SelectWeaponAccessory` result). Returns the new guid.
#[allow(clippy::too_many_arguments)]
pub fn add_accessory(ch: &mut Character, store: &DataStore, rec: Record<'_>, parent: &str, mount: &str, extramount: &str, rating: i32, free: bool) -> Result<String, String> {
    let guid = new_guid();
    let mut a = accessory_element(rec, &guid, mount, extramount, rating, None);
    a.set_child_text("parentid", parent);
    if free {
        a.set_child_text("cost", "0");
    }
    let w = super::find_by_guid_mut(&mut ch.doc, parent).filter(|w| w.name == "weapon").ok_or_else(|| format!("no weapon with guid {parent}"))?;
    w.child_or_insert("accessories").push(a);
    ch.dirty = true;
    add_gear_to(ch, store, rec.el(), &guid);
    Ok(guid)
}

/// Gear from the accessory records of a new weapon's accessories.
fn add_accessory_gear(ch: &mut Character, store: &DataStore, weapon_guid: &str) {
    let Ok(doc) = store.doc(FILE) else { return };
    let accs: Vec<(String, String)> = find_by_guid(&ch.doc, weapon_guid)
        .and_then(|w| w.child("accessories"))
        .map(|c| c.children_named("accessory").map(|a| (a.get("guid"), a.get("name"))).collect())
        .unwrap_or_default();
    for (g, name) in accs {
        if let Some(rec) = data::find(&doc, "accessories", "accessory", &name) {
            add_gear_to(ch, store, rec.el(), &g);
        }
    }
}

/// `<gears><usegear>` of a record, added through `items::gear` with
/// `Purchase.parent`; skipped while that kind cannot add gear.
fn add_gear_to(ch: &mut Character, store: &DataStore, rec: &Element, parent: &str) {
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
            free: true,
            cost_multiplier: 1.0,
            ..Default::default()
        };
        let _ = super::gear::add(ch, store, g, &p);
    }
}

/// A weapon granted by another item (`addweapon` in armor, mods, gear,
/// cyberware, weapons): top level, `parentid` = the item, no cost.
/// Returns the new guid, or `None` when the record does not exist.
pub fn add_child_weapon(ch: &mut Character, store: &DataStore, name: &str, rating: i32, parent: &str) -> Option<String> {
    let doc = store.doc(FILE).ok()?;
    let rec = data::find(&doc, "weapons", "weapon", name.trim())?;
    let guid = new_guid();
    let spec = WeaponSpec { guid: guid.clone(), rating, cost: Some("0".into()), parentid: parent.to_owned(), ..Default::default() };
    let w = weapon_tree(&doc, rec, spec);
    ch.items_mut("weapons").push(w);
    Some(guid)
}

// ---------------------------------------------------------------------------
// Bonus hooks
// ---------------------------------------------------------------------------

/// `AddImprovementCollection.naturalweapon`: a melee weapon built from the
/// bonus node, owned by the bonus source.
pub fn bonus_naturalweapon(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let guid = new_guid();
    let w = natural_weapon_element(node, &guid, &ctx.src.name, &ctx.src.guid);
    ctx.out.added.push(("weapons".into(), w));
    let i = ctx.imp("Weapon", &guid);
    ctx.push(i);
    true
}

/// The `<weapon>` a `naturalweapon` bonus creates (a `Weapon` with only
/// the listed fields set; everything else keeps its default).
pub fn natural_weapon_element(node: &Element, guid: &str, friendly_name: &str, parent: &str) -> Element {
    let or = |k: &str, d: &str| node.child_text(k).unwrap_or_else(|| d.to_owned());
    let mut w = Element::new("weapon");
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    // `SourceIDString` of `Guid.Empty`.
    put("sourceid", "00000000-0000-0000-0000-000000000000".into());
    put("guid", guid.to_owned());
    put("name", or("name", friendly_name));
    put("category", "Critter Powers".into());
    put("type", "Melee".into());
    put("spec", String::new());
    put("spec2", String::new());
    put("reach", or("reach", "0"));
    put("damage", or("damage", "({STR})S"));
    put("ap", or("ap", "0"));
    put("mode", "0".into());
    put("rc", "0".into());
    put("ammo", "0".into());
    put("cyberware", bool_str(false));
    put("ammocategory", String::new());
    put("ammoslots", "1".into());
    put("sizecategory", String::new());
    put("firingmode", "Skill".into());
    put("minrating", String::new());
    put("maxrating", String::new());
    put("rating", "0".into());
    put("accuracy", or("accuracy", "Physical"));
    put("activeammoslot", "1".into());
    put("conceal", "0".into());
    put("avail", "0".into());
    put("cost", "0".into());
    put("weight", String::new());
    put("useskill", or("useskill", ""));
    put("useskillspec", String::new());
    put("range", String::new());
    put("alternaterange", String::new());
    put("rangemultiply", "1".into());
    for (k, d) in [("singleshot", "1"), ("shortburst", "3"), ("longburst", "6"), ("fullburst", "10"), ("suppressive", "20")] {
        put(k, d.into());
    }
    for k in ["allowsingleshot", "allowshortburst", "allowlongburst", "allowfullburst", "allowsuppressive"] {
        put(k, bool_str(true));
    }
    put("source", or("source", "SR5"));
    put("page", or("page", "0"));
    put("parentid", parent.to_owned());
    put("allowaccessory", bool_str(true));
    put("weaponname", String::new());
    put("included", bool_str(false));
    put("equipped", bool_str(true));
    put("requireammo", bool_str(true));
    put("accuracy", or("accuracy", "Physical"));
    put("mount", String::new());
    put("stolen", bool_str(false));
    put("extramount", String::new());
    for (k, v) in [("location", ""), ("notes", ""), ("notesColor", NOTES_COLOR), ("discountedcost", "False"), ("weaponslots", ""), ("doubledcostweaponslots", ""), ("active", "False"), ("homenode", "False")] {
        put(k, v.into());
    }
    for (k, v) in matrix_fields(&Element::new("x")) {
        put(k, v);
    }
    put("matrixcmfilled", "0".into());
    w.push(Element::new("wirelessbonus"));
    w.push(Element::new("wirelessweaponbonus"));
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("wirelesson", bool_str(true));
    put("sortorder", "0".into());
    put("weapontype", String::new());
    w
}

/// `AddImprovementCollection.addweapon`: a weapon from weapons.xml (free
/// unless `<fullcost>`), owned by the bonus source, plus the weapons it
/// `addweapon`s itself.
pub fn bonus_addweapon(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc(FILE) else { return false };
    let name = node.child_text("name").unwrap_or_default();
    let Some(rec) = data::find(&doc, "weapons", "weapon", name.trim()) else { return false };
    let guid = new_guid();
    let cost = node.child("fullcost").is_none().then(|| "0".to_owned());
    let spec = WeaponSpec { guid: guid.clone(), cost, parentid: ctx.src.guid.clone(), ..Default::default() };
    let w = weapon_tree(&doc, rec, spec);
    for aw in rec.el().children_named("addweapon") {
        if let Some(sub) = data::find(&doc, "weapons", "weapon", aw.text().trim()) {
            let spec = WeaponSpec { guid: new_guid(), cost: Some("0".into()), parentid: guid.clone(), ..Default::default() };
            ctx.out.added.push(("weapons".into(), weapon_tree(&doc, sub, spec)));
        }
    }
    ctx.out.added.push(("weapons".into(), w));
    let i = ctx.imp("Weapon", &guid);
    ctx.push(i);
    true
}

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

/// Whether a weapon with this parent was given for free: everything that
/// grants weapons (armor, gear, ware, other weapons, bonuses) zeroes the
/// cost, except vehicles and their mounts, which only hold them.
fn parent_pays(ch: &Character, parentid: &str) -> bool {
    if parentid.is_empty() {
        return false;
    }
    !find_by_guid(&ch.doc, parentid).is_some_and(|p| matches!(p.name.as_str(), "vehicle" | "weaponmount" | "mod"))
}

/// Oracle: rebuild a saved `<weapon>` or `<accessory>` from its data
/// record plus the choices stored in it (rating, mount, variable cost,
/// whether it came with its parent, and its parent).
pub fn rebuild(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    let rating = saved.get_i32("rating").unwrap_or(0);
    if tag == "accessory" {
        let rec = find_saved(&doc, "accessories", "accessory", saved)?;
        let mut a = accessory_element(rec, &saved.get("guid"), &saved.get("mount"), &saved.get("extramount"), rating, saved_cost(rec, saved).as_deref());
        a.set_child_text("included", bool_str(saved.get_bool("included").unwrap_or(false)));
        return Some(a);
    }
    let Some(rec) = find_saved(&doc, "weapons", "weapon", saved) else {
        return rebuild_natural(ch, store, saved);
    };
    let included = saved.get_bool("included").unwrap_or(false);
    let parentid = saved.get("parentid");
    let cost = if included || parent_pays(ch, &parentid) { Some("0".to_owned()) } else { saved_cost(rec, saved) };
    let no_acc = included && find_by_guid(&ch.doc, &parentid).is_some_and(|p| p.get_bool("allowaccessory") == Some(false));
    let spec = WeaponSpec { guid: saved.get("guid"), rating, cost, parentid, included, no_accessories: no_acc, ..Default::default() };
    Some(weapon_tree(&doc, rec, spec))
}

/// A saved natural weapon: rebuilt from the `naturalweapon` bonus that
/// made it. The owner is the item whose `Weapon` improvement names the
/// weapon; metatype bonuses are looked up in the data by weapon name.
fn rebuild_natural(ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let guid = saved.get("guid");
    let name = saved.get("name");
    let imp = ch.improvements.list.iter().find(|i| i.kind == "Weapon" && i.improved_name.eq_ignore_ascii_case(&guid))?;
    let matches = |n: &&Element| n.child_text("name").is_none_or(|nm| nm == name);
    if let Some(owner) = find_by_guid(&ch.doc, &imp.source_name) {
        let mut nodes = Vec::new();
        owner.descendants("naturalweapon", &mut nodes);
        let node = nodes.into_iter().find(matches)?;
        return Some(natural_weapon_element(node, &guid, &owner.get("name"), &imp.source_name));
    }
    for file in ["metatypes.xml", "critters.xml", "critterpowers.xml", "qualities.xml"] {
        let Ok(doc) = store.doc(file) else { continue };
        let mut nodes = Vec::new();
        doc.descendants("naturalweapon", &mut nodes);
        if let Some(node) = nodes.iter().find(|n| n.child_text("name").is_some_and(|nm| nm == name)) {
            return Some(natural_weapon_element(node, &guid, &imp.source_name, &imp.source_name));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// Shared data store for lookups that only have a saved element at hand
/// (accessory cost multipliers, ranges).
fn shared_store() -> Option<&'static DataStore> {
    static STORE: OnceLock<Option<DataStore>> = OnceLock::new();
    STORE.get_or_init(DataStore::discover).as_ref()
}

/// `Weapon.ProcessRatingStringAsDec` for costs: FixedValues, `Rating`,
/// evaluate.
fn cost_value(s: &str, rating: i32) -> f64 {
    super::armor::rating_value(s, rating)
}

/// Doubles a cost once per doubled-cost slot the item occupies.
fn doubled(cost: f64, mount: &str, extramount: &str, slots: &str) -> f64 {
    let mut c = cost;
    let break_after_first = mount.is_empty() || extramount.is_empty();
    let mut found = false;
    for s in slots.split('/').filter(|s| !s.is_empty()) {
        if s == mount || s == extramount {
            c *= 2.0;
            if break_after_first || found {
                break;
            }
            found = true;
        }
    }
    c
}

/// `Weapon.OwnCost`.
pub fn own_cost(w: &Element, parent: Option<&Element>) -> f64 {
    if w.get("category") == "Gear" {
        return 0.0;
    }
    let mut c = cost_value(&w.get("cost"), w.get_i32("rating").unwrap_or(0));
    if w.get_bool("cyberware").unwrap_or(false) && c == 0.0 {
        return 0.0;
    }
    if w.get_bool("discountedcost").unwrap_or(false) {
        c *= 0.9;
    }
    match parent {
        Some(p) => doubled(c, &w.get("mount"), &w.get("extramount"), &p.get("doubledcostweaponslots")),
        None => c,
    }
}

/// `Weapon.AccessoryMultiplier`: sum of non-1 `accessorycostmultiplier`s
/// of equipped accessories, or 1.
fn accessory_multiplier(w: &Element) -> f64 {
    let doc = shared_store().and_then(|s| s.doc(FILE).ok());
    let mut m = 0;
    for a in accessories(w).filter(|a| equipped(a)) {
        let mult = doc.as_ref().and_then(|d| find_saved(d, "accessories", "accessory", a)).and_then(|r| r.el().get_i32("accessorycostmultiplier")).unwrap_or(1);
        if mult != 1 {
            m += mult;
        }
    }
    if m == 0 { 1.0 } else { f64::from(m) }
}

/// `WeaponAccessory.OwnCost`: `Weapon Cost` / `Weapon Total Cost` /
/// `Parent Rating` resolve against the weapon.
pub fn accessory_cost(a: &Element, w: &Element) -> f64 {
    if a.get_bool("included").unwrap_or(false) {
        return 0.0;
    }
    let wcost = own_cost(w, None);
    let s = a
        .get("cost")
        .replace("{Weapon Total Cost}", &fmt_num(multipliable_cost(w, a)))
        .replace("Weapon Total Cost", &fmt_num(multipliable_cost(w, a)))
        .replace("{Weapon Cost}", &fmt_num(wcost))
        .replace("Weapon Cost", &fmt_num(wcost))
        .replace("{Parent Rating}", &w.get("rating"))
        .replace("Parent Rating", &w.get("rating"))
        .replace("{Weapon Rating}", &w.get("rating"))
        .replace("Weapon Rating", &w.get("rating"));
    let mut c = cost_value(&s, a.get_i32("rating").unwrap_or(0));
    if a.get_bool("discountedcost").unwrap_or(false) {
        c *= 0.9;
    }
    c *= accessory_multiplier(w);
    doubled(c, &a.get("mount"), &a.get("extramount"), &w.get("doubledcostweaponslots"))
}

/// `Weapon.MultipliableCost`: the weapon's own cost plus accessories that
/// are not the asking one and do not price off the weapon themselves.
fn multipliable_cost(w: &Element, asking: &Element) -> f64 {
    let mut c = own_cost(w, None);
    for a in accessories(w) {
        if a.get("guid") != asking.get("guid") && !a.get("cost").contains("Weapon") && !a.get_bool("included").unwrap_or(false) {
            c += cost_value(&a.get("cost"), a.get_i32("rating").unwrap_or(0));
        }
    }
    c
}

/// `Weapon.TotalCost`: own cost plus accessories (with their gear) and
/// underbarrel weapons.
pub fn cost(w: &Element) -> f64 {
    total_cost(w, None)
}

fn total_cost(w: &Element, parent: Option<&Element>) -> f64 {
    let acc: f64 = accessories(w)
        .map(|a| accessory_cost(a, w) + a.child("gears").map(|g| g.children_named("gear").map(gear_cost).sum::<f64>()).unwrap_or(0.0))
        .sum();
    let ub: f64 = underbarrels(w).map(|u| total_cost(u, Some(w))).sum();
    own_cost(w, parent) + acc + ub
}

fn accessories(w: &Element) -> impl Iterator<Item = &Element> {
    w.child("accessories").into_iter().flat_map(|c| c.children_named("accessory"))
}

fn underbarrels(w: &Element) -> impl Iterator<Item = &Element> {
    w.children_named("underbarrel").flat_map(|u| u.children_named("weapon"))
}

fn equipped(e: &Element) -> bool {
    e.get_bool("equipped").unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Derived values
// ---------------------------------------------------------------------------

/// House rules the weapon calculations read (`CharacterSettings`).
#[derive(Debug, Clone)]
pub struct WeaponRules {
    pub restrict_recoil: bool,
    pub more_lethal_gameplay: bool,
    pub unarmed_improvements_apply_to_weapons: bool,
}

impl Default for WeaponRules {
    fn default() -> Self {
        WeaponRules { restrict_recoil: true, more_lethal_gameplay: false, unarmed_improvements_apply_to_weapons: false }
    }
}

impl WeaponRules {
    pub fn from_settings(s: &crate::settings::CharacterSettings) -> Self {
        WeaponRules {
            restrict_recoil: s.flag("restrictrecoil"),
            more_lethal_gameplay: s.flag("morelethalgameplay"),
            unarmed_improvements_apply_to_weapons: s.flag("unarmedimprovementsapplytoweapons"),
        }
    }
}

/// A weapon's range bands, as `"min-max"` metres (empty when the band
/// does not apply).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranges {
    pub short: String,
    pub medium: String,
    pub long: String,
    pub extreme: String,
    pub alt_short: String,
    pub alt_medium: String,
    pub alt_long: String,
    pub alt_extreme: String,
}

/// Final combat values of a weapon.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeaponStats {
    /// `CalculatedDamage`, e.g. `"7P"`, `"10S(e)"`.
    pub damage: String,
    /// `TotalAP`: `"-"`, `"+1"`, `"-2"`, or a special like `"-half"`.
    pub ap: String,
    /// `GetTotalAccuracy`.
    pub accuracy: i32,
    /// `TotalRC`: `"3"` or `"3 (5)"` with deployable recoil compensation.
    pub rc: String,
    /// `GetDicePool`.
    pub dice_pool: i32,
    /// `TotalReach`.
    pub reach: i32,
    /// The active skill used (`Weapon.Skill`).
    pub skill: String,
    /// `GetRangeStrings`; empty for melee weapons.
    pub ranges: Ranges,
}

/// What every calculation needs.
struct W<'a> {
    ch: &'a Character,
    sheet: &'a Sheet,
    w: &'a Element,
    rules: &'a WeaponRules,
    rating: i32,
    /// Parent weapon of an underbarrel.
    parent: Option<&'a Element>,
}

/// Attribute tokens with a weapon's STR/AGI overrides
/// (`Weapon.ProcessAttributesInXPath`).
struct WeaponAttrs<'a> {
    sheet: &'a Sheet,
    str_bonus: i32,
    zero_limb: bool,
}

impl AttributeSource for WeaponAttrs<'_> {
    fn attribute_token(&self, token: &str) -> Option<i32> {
        let base = calc::SheetAttributes(&self.sheet.attributes);
        let is_limb = ["STR", "AGI"].iter().any(|a| token.strip_prefix(a).is_some_and(|s| matches!(s, "" | "Unaug" | "Base")));
        if is_limb && self.zero_limb {
            return Some(0);
        }
        let v = base.attribute_token(token)?;
        Some(if token == "STR" { v + self.str_bonus } else { v })
    }
}

/// `Weapon.GetSkillDictionaryKey`.
fn skill_key_for_category(cat: &str, spec: &str) -> String {
    match cat {
        "Bows" | "Crossbows" => "Archery".into(),
        "Assault Rifles" | "Carbines" | "Machine Pistols" | "Submachine Guns" => "Automatics".into(),
        "Blades" => "Blades".into(),
        "Clubs" | "Improvised Weapons" => "Clubs".into(),
        "Exotic Melee Weapons" => format!("Exotic Melee Weapon ({spec})"),
        "Exotic Ranged Weapons" | "Special Weapons" => format!("Exotic Ranged Weapon ({spec})"),
        "Flamethrowers" => "Exotic Ranged Weapon (Flamethrowers)".into(),
        "Laser Weapons" => "Exotic Ranged Weapon (Laser Weapons)".into(),
        "Assault Cannons" | "Grenade Launchers" | "Missile Launchers" | "Light Machine Guns" | "Medium Machine Guns" | "Heavy Machine Guns" => "Heavy Weapons".into(),
        "Shotguns" | "Sniper Rifles" | "Sporting Rifles" => "Longarms".into(),
        "Throwing Weapons" => "Throwing Weapons".into(),
        "Unarmed" => "Unarmed Combat".into(),
        _ => "Pistols".into(),
    }
}

/// Exotic skills carry the weapon in parentheses.
fn is_exotic(skill: &str) -> bool {
    skill.starts_with("Exotic Melee Weapon") || skill.starts_with("Exotic Ranged Weapon")
}

impl<'a> W<'a> {
    fn get(&self, k: &str) -> String {
        self.w.get(k)
    }

    fn imps(&self) -> &'a crate::improvement::Improvements {
        &self.ch.improvements
    }

    fn cyberware(&self) -> bool {
        self.w.get_bool("cyberware").unwrap_or(false)
    }

    /// `Weapon.Skill`: the skill key (`DictionaryKey`).
    fn skill(&self) -> String {
        let mut cat = self.get("category");
        if cat == "Special Weapons" && !self.get("range").is_empty() {
            cat = self.get("range");
        }
        // `UseSkillSpec` falls back to the weapon name.
        let spec = Some(self.get("useskillspec")).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| self.get("name"));
        let useskill = self.get("useskill");
        if useskill.is_empty() {
            return skill_key_for_category(&cat, &spec);
        }
        if is_exotic(&useskill) && !useskill.contains('(') {
            return format!("{useskill} ({spec})");
        }
        useskill
    }

    fn is_throwing(&self) -> bool {
        self.get("category") == "Throwing Weapons" || self.skill() == "Throwing Weapons"
    }

    /// Attribute values for `{STR}` and friends; `for_range` adds
    /// `ThrowRangeSTR` for throwing weapons.
    fn attrs(&self, for_range: bool) -> WeaponAttrs<'a> {
        let mut str_bonus = 0.0;
        if self.is_throwing() {
            str_bonus = self.imps().val("ThrowSTR", None);
            if for_range {
                str_bonus += self.imps().val("ThrowRangeSTR", None);
            }
        }
        // Cyberware weapons without a parent use no character STR/AGI.
        let zero_limb = self.cyberware() && self.get("parentid").is_empty();
        WeaponAttrs { sheet: self.sheet, str_bonus: expr::standard_round(str_bonus), zero_limb }
    }

    /// `Weapon.ProcessRatingStringAsDec`: `None` when the expression does
    /// not evaluate.
    fn value(&self, s: &str, for_range: bool) -> Option<f64> {
        if s.is_empty() {
            return Some(0.0);
        }
        let s = expr::fixed_values(s, self.rating);
        let s = s.trim_start_matches('+');
        if !expr::needs_evaluation(s) {
            return expr::parse_plain(s);
        }
        let limit = self.sheet.limit_physical.to_string();
        let parent_rating = self.parent.map_or_else(|| "1".to_owned(), |p| p.get("rating"));
        let r = self.rating.to_string();
        let s = s
            .replace("{Physical}", &limit)
            .replace("Physical", &limit)
            .replace("{Missile}", &limit)
            .replace("Missile", &limit)
            .replace("{Parent Rating}", &parent_rating)
            .replace("Parent Rating", &parent_rating)
            .replace("{Weapon Rating}", &r)
            .replace("Weapon Rating", &r)
            .replace("{Rating}", &r)
            .replace("Rating", &r);
        let s = expr::substitute_attributes(&s, &self.attrs(for_range));
        expr::evaluate_num(&s).ok()
    }

    fn int(&self, s: &str) -> i32 {
        expr::standard_round(self.value(s, false).unwrap_or(0.0))
    }

    fn wireless_on(&self) -> bool {
        // Weapon.Load keeps the field default (on) when the save has none.
        self.w.get_bool("wirelesson").unwrap_or(true)
    }

    /// Equipped accessories.
    fn accessories(&self) -> Vec<&'a Element> {
        accessories(self.w).filter(|a| equipped(a)).collect()
    }

    /// The weapon's wireless weapon bonus, when wireless is on.
    fn wireless_bonus(&self) -> Option<&'a Element> {
        self.w.child("wirelessweaponbonus").filter(|b| self.wireless_on() && b.elements().next().is_some())
    }

    /// An accessory's wireless weapon bonus, when it and the weapon are on.
    fn acc_wireless(&self, a: &'a Element) -> Option<&'a Element> {
        a.child("wirelessweaponbonus").filter(|b| self.wireless_on() && a.get_bool("wirelesson").unwrap_or(false) && b.elements().next().is_some())
    }

    /// `Weapon.AmmoLoaded`: the gear in the active clip, with the bonus
    /// that applies (flechette bonus for `(f)` weapons).
    fn ammo_bonus(&self) -> Option<&'a Element> {
        let slot = self.w.get_i32("activeammoslot").unwrap_or(1).max(1) as usize;
        let clip = self.w.child("clips")?.children_named("clip").nth(slot - 1)?;
        let gear = find_by_guid(&self.ch.doc, &clip.get("id")).filter(|_| !clip.get("id").starts_with("00000000"))?;
        let flechette = self.get("damage").contains("(f)") && self.get("ammocategory") != "Gear";
        let fb = gear.child("flechetteweaponbonus").filter(|b| b.elements().next().is_some());
        if flechette && fb.is_some() {
            return fb;
        }
        gear.child("weaponbonus").filter(|b| b.elements().next().is_some())
    }

    /// `Weapon.HasWirelessSmartgun`.
    fn has_wireless_smartgun(&self) -> bool {
        self.accessories().iter().any(|a| a.get("name").starts_with("Smartgun") && a.get_bool("wirelesson").unwrap_or(false))
    }

    /// `GetWeaponCategoryImprovements`: by category, by skill when
    /// different, and by the category without `Cyberware `.
    fn category_improvements(&self, kind: &str) -> f64 {
        let mut cat = self.get("category");
        if cat == "Unarmed" {
            cat = "Unarmed Combat".into();
        }
        let mut v = self.imps().val(kind, Some(&cat));
        let skill = self.skill();
        if !skill.is_empty() && skill != cat {
            v += self.imps().val(kind, Some(&skill));
        }
        if let Some(rest) = cat.strip_prefix("Cyberware ") {
            v += self.imps().val(kind, Some(rest));
        }
        v
    }

    fn unarmed_applies(&self) -> bool {
        self.get("name") == "Unarmed Attack" || (self.skill() == "Unarmed Combat" && self.rules.unarmed_improvements_apply_to_weapons)
    }
}

/// Non-zero modifier text (`!= "0" && != "+0" && != "-0"`).
fn nonzero(s: &str) -> Option<&str> {
    (!s.is_empty() && !matches!(s, "0" | "+0" | "-0")).then_some(s)
}

/// Replace `{Rating}` / `Rating` with an item's rating.
fn with_rating(s: &str, rating: &str) -> String {
    s.replace("{Rating}", rating).replace("Rating", rating)
}

// --- Damage -----------------------------------------------------------------

/// Split the damage type and extras off a damage code
/// (`CalculatedDamage`: "P or S", P, S, (M); (e), (f), (fire); splash).
fn split_damage(mut d: String) -> (String, String, String) {
    let mut kind = String::new();
    if d.contains("P or S") {
        kind = "P or S".into();
        d = d.replace("P or S", "");
    } else if d.contains('P') {
        kind = "P".into();
        d = d.replace('P', "");
    } else if d.contains('S') {
        kind = "S".into();
        d = d.replace('S', "");
    } else if d.contains("(M)") {
        kind = "M".into();
        d = d.replace("(M)", "");
    }
    let mut extra = String::new();
    for x in ["(e)", "(f)", "(fire)"] {
        if d.contains(x) {
            extra = x.into();
            d = d.replace(x, "");
            break;
        }
    }
    (d, kind, extra)
}

/// Evaluate `min(a,b,..)` in a damage code.
fn eval_min(d: &str) -> String {
    let Some(start) = d.find("min(") else { return d.to_owned() };
    let Some(end) = d[start..].find(')').map(|e| e + start) else { return d.to_owned() };
    let inner = &d[start + 4..end];
    let m = inner.split(',').filter_map(|v| v.trim().parse::<i32>().ok()).min().unwrap_or(i32::MAX);
    format!("{}{}{}", &d[..start], m, &d[end + 1..])
}

/// `Weapon.Load` legacy catch: saves before 5.214.98 wrote attribute
/// damage without braces (`(STR+2)P`); Chummer then uses the data value.
pub fn legacy_damage(w: &Element) -> Option<String> {
    let d = w.get("damage");
    if d.is_empty() || d.contains('{') || !expr::ATTRIBUTE_NAMES.iter().any(|a| d.contains(a)) {
        return None;
    }
    let doc = shared_store()?.doc(FILE).ok()?;
    find_saved(&doc, "weapons", "weapon", w).map(|r| r.get("damage"))
}

/// Damage code being modified by wireless bonuses, accessories and ammo.
struct DamageMods {
    d: String,
    kind: String,
    extra: String,
    bonus: String,
    replaced: bool,
}

impl DamageMods {
    /// Apply one modifier node. Bonus nodes change the damage type when
    /// they have a `damagetype` child; accessories when it is non-empty.
    /// The weapon's own wireless bonus is appended without a `+`, as in
    /// Chummer.
    fn apply(&mut self, b: &Element, first: bool, bonus_node: bool) {
        let dt = b.child_text("damagetype");
        if dt.as_deref().is_some_and(|t| bonus_node || !t.is_empty()) {
            self.kind.clear();
            self.extra = dt.unwrap_or_default();
        }
        let dmg = b.get("damage");
        let add = if bonus_node { nonzero(&dmg) } else { Some(dmg.as_str()).filter(|s| !s.is_empty()) };
        if let Some(x) = add {
            self.bonus.push_str(&format!("{}({})", if first { "" } else { "+" }, x.trim_start_matches('+')));
        }
        if let Some(r) = b.child_text("damagereplace").filter(|r| !r.is_empty()) {
            self.replaced = true;
            self.d = r;
        }
    }
}

/// `Weapon.CalculatedDamage` (English, ammunition included).
fn damage(c: &W<'_>) -> String {
    let raw = legacy_damage(c.w).unwrap_or_else(|| c.get("damage"));
    let d = expr::substitute_attributes(&raw, &c.attrs(false)).replace("{Rating}", &c.rating.to_string());
    let (mut d, mut kind, mut extra) = split_damage(eval_min(&d));
    if d.contains("/m)") || d.contains(" Radius)") {
        if let (Some(a), Some(b)) = (d.find('('), d.find(')')) {
            let splash = d[a..=b].to_owned();
            extra = format!("{extra} {splash}");
            d = d.replace(&splash, "").trim().to_owned();
        }
    }
    let mut improve = c.category_improvements("WeaponCategoryDV");
    if c.get("name") == "Unarmed Attack" {
        if kind == "S" && c.imps().has("UnarmedDVPhysical") {
            kind = "P".into();
        }
        improve += c.imps().val("UnarmedDV", None);
    } else if c.skill() == "Unarmed Combat" && c.rules.unarmed_improvements_apply_to_weapons {
        improve += c.imps().val("UnarmedDV", None);
    }
    if c.rules.more_lethal_gameplay {
        improve += 2.0;
    }
    let mut acc = DamageMods { d, kind, extra, bonus: String::new(), replaced: false };
    if let Some(b) = c.wireless_bonus() {
        acc.apply(b, true, true);
    }
    for a in c.accessories() {
        acc.apply(a, false, false);
        if let Some(b) = c.acc_wireless(a) {
            acc.apply(b, false, true);
        }
    }
    if let Some(b) = c.ammo_bonus() {
        acc.apply(b, false, true);
    }
    let DamageMods { mut d, kind, extra, bonus, replaced } = acc;
    d.push_str(&bonus);
    let result = if replaced {
        let original = d.clone();
        let (d2, k2, e2) = split_damage(d);
        let (k, e) = (if k2.is_empty() { kind } else { k2 }, if e2.is_empty() { extra } else { e2 });
        finish_damage(c, &d2, &k, &e, improve).unwrap_or(original)
    } else {
        finish_damage(c, &d, &kind, &extra, improve).unwrap_or_else(|| "NaN".into())
    };
    if result.starts_with("NaN") { raw } else { result }
}

/// Evaluate the numeric part of a damage code and reattach type/extras.
fn finish_damage(c: &W<'_>, d: &str, kind: &str, extra: &str, improve: f64) -> Option<String> {
    if d.trim().is_empty() {
        return Some(format!("{kind}{extra}"));
    }
    if d.contains("//") {
        return Some(format!("{}{kind}{extra}", d.replace("//", "/")));
    }
    let v = c.value(d, false)?;
    let mut n = expr::standard_round(v + improve);
    if c.get("name") == "Unarmed Attack (Smashing Blow)" {
        n *= 2;
    }
    Some(format!("{n}{kind}{extra}"))
}

// --- AP ---------------------------------------------------------------------

/// `Weapon.TotalAP` (English, ammunition included).
fn ap(c: &W<'_>) -> String {
    let mut ap = c.get("ap");
    let mut bonus = String::new();
    let mut take = |b: &Element, rating: &str, first: bool, ap: &mut String| {
        if let Some(r) = b.child_text("apreplace").filter(|r| !r.is_empty()) {
            *ap = with_rating(&r, rating);
        }
        if let Some(x) = nonzero(&b.get("ap")) {
            bonus.push_str(&format!("{}({})", if first { "" } else { "+" }, with_rating(x, rating).trim_start_matches('+')));
        }
    };
    if let Some(b) = c.wireless_bonus() {
        take(b, &c.rating.to_string(), true, &mut ap);
    }
    for a in c.accessories() {
        if a.get("damagetype").contains("(f)") && c.get("damage").contains("(f)") {
            continue;
        }
        let r = a.get("rating");
        take(a, &r, false, &mut ap);
        if let Some(b) = c.acc_wireless(a) {
            take(b, &r, false, &mut ap);
        }
    }
    if let Some(b) = c.ammo_bonus() {
        take(b, "0", false, &mut ap);
    }
    let mut improve = 0;
    if c.unarmed_applies() {
        improve += c.imps().val_int("UnarmedAP", None);
    }
    improve += expr::standard_round(c.category_improvements("WeaponCategoryAP"));
    if ap == "-" {
        ap = "0".into();
    }
    ap.push_str(&bonus);
    if ap.contains("//") {
        return ap.replace("//", "/");
    }
    let Some(v) = c.value(&ap, false) else { return ap };
    let n = expr::standard_round(v) + improve;
    match n {
        0 => "-".into(),
        n if n > 0 => format!("+{n}"),
        n => n.to_string(),
    }
}

// --- Accuracy ---------------------------------------------------------------

/// Smartgun systems and sights do not stack: the best one counts.
fn non_stacking(a: &Element) -> bool {
    let n = a.get("name");
    n.starts_with("Smartgun") || n.contains("Sight")
}

/// `Weapon.GetTotalAccuracy` (ammunition included).
fn accuracy(c: &W<'_>) -> i32 {
    let mut acc = c.get("accuracy");
    let mut bonus = String::new();
    if let Some(b) = c.wireless_bonus() {
        if let Some(r) = b.child_text("accuracyreplace").filter(|r| !r.is_empty()) {
            acc = r;
        }
        if let Some(x) = nonzero(&b.get("accuracy")) {
            bonus.push_str(&format!("+({})", x.trim_start_matches('+')));
        }
    }
    let mut best_of: Vec<String> = Vec::new();
    for a in c.accessories() {
        let r = a.get("rating");
        let own = nonzero(&a.get("accuracy")).map(|x| with_rating(x, &r));
        if let Some(x) = &own {
            if non_stacking(a) { best_of.push(x.clone()) } else { bonus.push_str(&format!("+({})", x.trim_start_matches('+'))) }
        }
        if let Some(b) = c.acc_wireless(a) {
            if let Some(rep) = b.child_text("accuracyreplace").filter(|r| !r.is_empty()) {
                acc = with_rating(&rep, &r);
            }
            if let Some(x) = nonzero(&b.get("accuracy")).map(|x| with_rating(x, &r)) {
                if !non_stacking(a) {
                    bonus.push_str(&format!("+({})", x.trim_start_matches('+')));
                } else if own.is_some() {
                    let last = best_of.pop().unwrap_or_default();
                    best_of.push(format!("({last}) + ({})", x.trim_start_matches('+')));
                } else {
                    best_of.push(x);
                }
            }
        }
    }
    // Included underbarrels of the same type share the parent's built-in smartgun.
    if c.w.get_bool("included").unwrap_or(false) {
        if let Some(p) = c.parent.filter(|p| p.get("type") == c.get("type")) {
            for a in accessories(p).filter(|a| equipped(a) && a.get("name").starts_with("Smartgun") && a.get_bool("included").unwrap_or(false)) {
                if !a.get("accuracy").is_empty() {
                    best_of.push(with_rating(&a.get("accuracy"), &a.get("rating")));
                }
            }
        }
    }
    if let Some(best) = best_of.iter().map(|s| (s, c.value(s, false).unwrap_or(0.0))).fold(None, |m: Option<(&String, f64)>, x| match m {
        Some(b) if b.1 >= x.1 => Some(b),
        _ => Some(x),
    }) {
        bonus.push_str(&format!("+({})", best.0));
    }
    if let Some(b) = c.ammo_bonus() {
        if let Some(r) = b.child_text("accuracyreplace").filter(|r| !r.is_empty()) {
            acc = r;
        }
        if let Some(x) = nonzero(&b.get("accuracy")) {
            bonus.push_str(&format!("+({})", x.trim_start_matches('+')));
        }
    }
    if !bonus.is_empty() {
        acc = format!("({acc}){bonus}");
    }
    let mut n = c.int(&acc);
    let name = c.get("name");
    let mut improve = c.imps().sum(Query::named("WeaponSkillAccuracy", &name).with_non_improved(), crate::improvement::Field::Val);
    let skill = c.skill();
    if !skill.is_empty() {
        improve += c.imps().val("WeaponSkillAccuracy", Some(&skill));
    }
    let upper = name.to_uppercase();
    for i in c.imps().of_kind("WeaponAccuracy") {
        if let Some(part) = i.improved_name.strip_prefix("[contains]") {
            if upper.contains(&part.to_uppercase()) {
                improve += i.val;
            }
        }
    }
    n += expr::standard_round(improve);
    n
}

// --- Recoil -----------------------------------------------------------------

/// `Weapon.TotalRC`: `"base"` or `"base (full)"` when deployable recoil
/// compensation adds more.
fn rc(c: &W<'_>) -> String {
    let raw = c.get("rc").replace("{Rating}", &c.rating.to_string());
    let (base_s, full_s) = match raw.find('(') {
        Some(0) => ("0".to_owned(), raw.clone()),
        Some(p) => (raw[..p].to_owned(), raw[p..].to_owned()),
        None => (raw.clone(), raw.clone()),
    };
    let parse = |s: &str| s.trim().trim_start_matches('+').parse::<i32>().unwrap_or(0);
    let mut base = parse(&base_s);
    let mut full = parse(full_s.trim_matches(|ch| ch == '(' || ch == ')'));
    let rc_of = |b: &Element| b.child_text("rc").and_then(|s| s.trim().parse::<i32>().ok());
    if let Some(v) = c.wireless_bonus().and_then(rc_of) {
        base += v;
        full += v;
    }
    let (mut groups, mut deploy): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for a in c.accessories() {
        let a_rc = a.get("rc");
        if !a_rc.is_empty() {
            let group = a.get_i32("rcgroup").unwrap_or(0);
            let deployable = a.get_bool("rcdeployable").unwrap_or(false);
            if c.rules.restrict_recoil && group != 0 {
                let v = super::armor::rating_value(&a_rc, a.get_i32("rating").unwrap_or(0));
                let list = if deployable { &mut deploy } else { &mut groups };
                if list.len() < group as usize {
                    list.resize(group as usize, 0.0);
                }
                let slot = &mut list[group as usize - 1];
                *slot = slot.max(v);
            } else if let Ok(v) = a_rc.trim().parse::<i32>() {
                full += v;
                if !deployable {
                    base += v;
                }
            }
        }
        if let Some(v) = c.acc_wireless(a).and_then(rc_of) {
            base += v;
            full += v;
        }
    }
    if let Some(v) = c.ammo_bonus().and_then(rc_of) {
        base += v;
        full += v;
    }
    for v in groups {
        let n = expr::standard_round(v);
        base += n;
        full += n;
    }
    for v in deploy {
        full += expr::standard_round(v);
    }
    let mut strength = if c.cyberware() && c.get("parentid").is_empty() { 0 } else { c.sheet.attr("STR") };
    if c.is_throwing() {
        strength += c.imps().val_int("ThrowSTR", None);
    }
    let from_str = calc::div_away_from_zero(strength, 3) + 1;
    base += from_str;
    full += from_str;
    if full > base { format!("{base} ({full})") } else { base.to_string() }
}

// --- Dice pool --------------------------------------------------------------

/// `Weapon.GetDicePool` for a character's own weapon (`FiringMode.Skill`).
fn dice_pool(c: &W<'_>) -> i32 {
    let key = c.skill();
    let skill = c.sheet.skills.iter().find(|s| s.name == key);
    let mut pool = skill.map_or(0, |s| s.pool);
    let mut modifier = 0.0;
    if let Some(sk) = skill {
        if c.wireless_on() && c.has_wireless_smartgun() {
            modifier += c.imps().val("Smartlink", None);
        }
        modifier += c.imps().val("WeaponCategoryDice", Some(&c.get("category")));
        // Chummer adds all weapon-specific improvements here.
        let guid = c.get("guid");
        for k in ["WeaponSpecificDice", "WeaponSpecificDV", "WeaponSpecificAP", "WeaponSpecificAccuracy", "WeaponSpecificRange"] {
            modifier += c.imps().val(k, Some(&guid));
        }
        pool += spec_bonus(c, sk);
    }
    let mut extra = String::new();
    if let Some(b) = c.wireless_bonus() {
        if let Some(x) = nonzero(&b.get("pool")) {
            extra.push_str(&format!("({})", x.trim_start_matches('+')));
        }
        if c.has_wireless_smartgun() {
            if let Some(x) = nonzero(&b.get("smartlinkpool")) {
                extra.push_str(&format!("+({})", x.trim_start_matches('+')));
            }
        }
    }
    for a in c.accessories() {
        let r = a.get("rating");
        if let Some(b) = c.acc_wireless(a) {
            if let Some(x) = nonzero(&b.get("pool")) {
                extra.push_str(&format!("+({})", with_rating(x.trim_start_matches('+'), &r)));
            }
            if c.has_wireless_smartgun() {
                if let Some(x) = nonzero(&b.get("smartlinkpool")) {
                    extra.push_str(&format!("+({})", with_rating(x.trim_start_matches('+'), &r)));
                }
            }
        }
        modifier += super::armor::rating_value(&a.get("dicepool"), a.get_i32("rating").unwrap_or(0));
    }
    if let Some(b) = c.ammo_bonus() {
        if let Some(x) = nonzero(&b.get("pool")) {
            extra.push_str(&format!("+({})", x.trim_start_matches('+')));
        }
        if c.wireless_on() && c.has_wireless_smartgun() {
            if let Some(x) = nonzero(&b.get("smartlinkpool")) {
                extra.push_str(&format!("+({})", x.trim_start_matches('+')));
            }
        }
    }
    let extra = c.value(extra.trim_start_matches('+'), false).unwrap_or(0.0);
    pool + expr::standard_round(modifier + extra)
}

/// Specialization bonus: the weapon's name, category, or its data
/// `spec`/`spec2` (`objSkill.GetSpecialization(..)`).
fn spec_bonus(c: &W<'_>, skill: &calc::SkillValues) -> i32 {
    if skill.specs.is_empty() || is_exotic(&skill.name) {
        return 0;
    }
    let candidates = [c.get("name"), c.get("category"), c.get("spec"), c.get("spec2")];
    if candidates.iter().any(|n| !n.is_empty() && skill.specs.iter().any(|s| s == n)) { skill.spec_bonus } else { 0 }
}

// --- Reach ------------------------------------------------------------------

/// `Weapon.TotalReach`.
fn reach(c: &W<'_>) -> i32 {
    let mut r = c.value(&c.get("reach"), false).unwrap_or(0.0);
    for a in c.accessories() {
        r += super::armor::rating_value(&a.get("reach"), a.get_i32("rating").unwrap_or(0));
    }
    if c.get("type") == "Melee" {
        r += c.imps().sum(Query::named("Reach", &c.get("name")).with_non_improved(), crate::improvement::Field::Val);
        let mut cat = c.get("category");
        if cat == "Unarmed" {
            cat = "Unarmed Combat".into();
        }
        r += c.imps().val("WeaponCategoryReach", Some(&cat));
    }
    if c.unarmed_applies() {
        r += c.imps().val("UnarmedReach", None);
    }
    expr::standard_round(r)
}

// --- Ranges -----------------------------------------------------------------

/// `Weapon.GetRange`: one band of the weapon's (or alternate) range
/// category, `-1` when absent.
fn range_band(c: &W<'_>, doc: &Element, band: &str, alternate: bool) -> f64 {
    let mut cat = if alternate {
        let alt = c.get("alternaterange");
        if alt.trim().is_empty() {
            return -1.0;
        }
        alt
    } else if !c.get("range").is_empty() {
        c.get("range")
    } else {
        c.get("category")
    };
    if let Some(r) = c.wireless_bonus().and_then(|b| b.child_text("userange")) {
        cat = r;
    }
    for a in c.accessories() {
        if !a.get("replacerange").is_empty() {
            cat = a.get("replacerange");
        }
    }
    if let Some(r) = c.ammo_bonus().and_then(|b| b.child_text("userange")) {
        cat = r;
    }
    let Some(node) = data::find(doc, "ranges", "range", &cat).and_then(|r| r.el().child_text(band)) else { return -1.0 };
    let Some(mut v) = c.value(&node, true) else { return -1.0 };
    if c.is_throwing() {
        v += c.imps().val("ThrowRange", None);
    }
    v * c.w.get_f64("rangemultiply").unwrap_or(1.0)
}

/// `Weapon.GetRangeBonus`: percent bonus from wireless, accessories, ammo.
fn range_bonus(c: &W<'_>) -> f64 {
    let mut s = String::new();
    if let Some(x) = c.wireless_bonus().map(|b| b.get("rangebonus")).as_deref().and_then(nonzero) {
        s.push_str(&format!("({})", x.trim_start_matches('+')));
    }
    for a in c.accessories() {
        let r = a.get("rating");
        if let Some(x) = nonzero(&a.get("rangebonus")) {
            s.push_str(&format!("+({})", with_rating(x.trim_start_matches('+'), &r)));
        }
        if let Some(x) = c.acc_wireless(a).map(|b| b.get("rangebonus")).as_deref().and_then(nonzero) {
            s.push_str(&format!("+({})", with_rating(x.trim_start_matches('+'), &r)));
        }
    }
    if let Some(x) = c.ammo_bonus().map(|b| b.get("rangebonus")).as_deref().and_then(nonzero) {
        s.push_str(&format!("+({})", x.trim_start_matches('+')));
    }
    c.value(s.trim_start_matches('+'), true).unwrap_or(0.0)
}

/// `Weapon.GetRangeStrings`.
fn ranges(c: &W<'_>, doc: &Element) -> Ranges {
    let m = 1.0 + range_bonus(c) / 100.0;
    let band = |b: &str, alt: bool| expr::standard_round(range_band(c, doc, b, alt) * m);
    let fmt = |lo: i32, hi: i32, first: bool| {
        if lo < 0 || hi < 0 {
            String::new()
        } else {
            format!("{}-{hi}", if first { lo } else { lo + 1 })
        }
    };
    let set = |alt: bool| {
        let [mn, s, md, l, x] = ["min", "short", "medium", "long", "extreme"].map(|b| band(b, alt));
        [fmt(mn, s, true), fmt(s, md, false), fmt(md, l, false), fmt(l, x, false)]
    };
    let [short, medium, long, extreme] = set(false);
    let [alt_short, alt_medium, alt_long, alt_extreme] = set(true);
    Ranges { short, medium, long, extreme, alt_short, alt_medium, alt_long, alt_extreme }
}

/// Final combat values of `weapon` with default house rules.
pub fn stats(ch: &Character, sheet: &Sheet, weapon: &Element) -> WeaponStats {
    stats_with(ch, sheet, shared_store(), weapon, &WeaponRules::default())
}

/// Final combat values of `weapon`. `store` supplies ranges.xml; without
/// it the ranges stay empty.
pub fn stats_with(ch: &Character, sheet: &Sheet, store: Option<&DataStore>, weapon: &Element, rules: &WeaponRules) -> WeaponStats {
    let parent = {
        let pid = weapon.get("parentid");
        (!pid.is_empty() && weapon.get_bool("included").unwrap_or(false)).then(|| find_by_guid(&ch.doc, &pid)).flatten().filter(|p| p.name == "weapon")
    };
    let c = W { ch, sheet, w: weapon, rules, rating: weapon.get_i32("rating").unwrap_or(0), parent };
    let ranges = if weapon.get("type") == "Melee" {
        Ranges::default()
    } else {
        store.and_then(|s| s.doc("ranges.xml").ok()).map(|d| ranges(&c, &d)).unwrap_or_default()
    };
    WeaponStats { damage: damage(&c), ap: ap(&c), accuracy: accuracy(&c), rc: rc(&c), dice_pool: dice_pool(&c), reach: reach(&c), skill: c.skill(), ranges }
}
