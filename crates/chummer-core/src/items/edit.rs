//! Editing a saved item: the detail panes of `CharacterCreate.cs` /
//! `CharacterCareer.cs` (rating, quantity, equipped, wireless, custom name,
//! location, notes, Sell / Delete) and the values they show (cost,
//! availability, essence, capacity).
//!
//! Every function takes the item's guid and finds it anywhere in the
//! character (gear, ware, armor and armor mods, weapons and accessories,
//! vehicles, vehicle mods, weapon mounts, lifestyles, drugs, at any depth).
//!
//! **Rating changes.** The element builders store every rating-dependent
//! field as the data expression (`cost` = `Rating * 250`, `avail`, `ess`,
//! `capacity`, armor `armor`...) and the cost, essence and availability
//! functions evaluate it at `<rating>`. Setting `<rating>` to the clamped
//! value therefore gives the same element a fresh build at that rating
//! would (the `item_edit` tests check this against `gear::element`), and
//! keeps every bit of user state (guid, name, notes, children, location).
//! What does not follow by itself are the improvements: like
//! `nudGearRating_ValueChanged`, [`apply_rating_change`] removes the item's
//! improvements and creates them again from its data bonus at the new
//! rating, and gear children whose rating limits refer to the parent are
//! clamped again (`Gear.Rating` setter).

use crate::bonus::{self, BonusSource};
use crate::calc::Rules;
use crate::career::{self, CareerError};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::expr::{self, Availability, NoAttributes};
use crate::improvement::bool_str;
use crate::xml::{Element, Node};

use super::{armor, cyberware, drug, gear, lifestyle, vehicle, weapon, Purchase};

// ---------------------------------------------------------------------------
// Finding items
// ---------------------------------------------------------------------------

/// Element names of saved items.
const ITEM_TAGS: &[&str] = &["gear", "cyberware", "armor", "armormod", "weapon", "accessory", "vehicle", "mod", "weaponmount", "lifestyle", "drug"];

/// The item kind of a saved element: its element name, except ware from
/// bioware.xml, which is "bioware" (as in [`super::KINDS`]).
pub fn tag_of(e: &Element) -> &str {
    if e.name == "cyberware" && cyberware::is_bioware(e) {
        "bioware"
    } else {
        e.name.as_str()
    }
}

/// Whether a saved element is an item the editor handles (and has a guid).
pub fn is_item(e: &Element) -> bool {
    ITEM_TAGS.contains(&e.name.as_str()) && !e.get("guid").is_empty()
}

fn find_in<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if ITEM_TAGS.contains(&e.name.as_str()) && e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_in(c, guid))
}

fn find_in_mut<'a>(e: &'a mut Element, guid: &str) -> Option<&'a mut Element> {
    if ITEM_TAGS.contains(&e.name.as_str()) && e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements_mut().find_map(|c| find_in_mut(c, guid))
}

/// The saved item with this guid, at any depth.
pub fn find<'a>(ch: &'a Character, guid: &str) -> Option<&'a Element> {
    if guid.is_empty() {
        return None;
    }
    find_in(&ch.doc, guid)
}

fn find_mut<'a>(ch: &'a mut Character, guid: &str) -> Option<&'a mut Element> {
    if guid.is_empty() {
        return None;
    }
    find_in_mut(&mut ch.doc, guid)
}

/// The item that holds the item `guid` (a gear's `<children>`, an armor's
/// `<armormods>`, a weapon's `<underbarrel>`...). `None` at the top level.
pub fn parent<'a>(ch: &'a Character, guid: &str) -> Option<&'a Element> {
    fn walk<'a>(e: &'a Element, guid: &str, owner: Option<&'a Element>) -> Option<&'a Element> {
        let here = if ITEM_TAGS.contains(&e.name.as_str()) { Some(e) } else { owner };
        for c in e.elements() {
            if ITEM_TAGS.contains(&c.name.as_str()) && c.get("guid").eq_ignore_ascii_case(guid) {
                return here;
            }
            if let Some(p) = walk(c, guid, here) {
                return Some(p);
            }
        }
        None
    }
    walk(&ch.doc, guid, None)
}

/// Guids of an item and every item inside it.
fn subtree_guids(e: &Element, out: &mut Vec<String>) {
    if ITEM_TAGS.contains(&e.name.as_str()) {
        let g = e.get("guid");
        if !g.is_empty() {
            out.push(g);
        }
    }
    for c in e.elements() {
        subtree_guids(c, out);
    }
}

/// The data record of a saved item: by `sourceid`, else by name.
fn record_in<'a>(doc: &'a Element, container: &str, item: &'a str, saved: &Element) -> Option<Record<'a>> {
    let id = saved.child_text("sourceid").filter(|s| !s.trim().is_empty()).or_else(|| saved.child_text("id")).unwrap_or_default();
    (!id.is_empty()).then(|| data::find(doc, container, item, &id)).flatten().or_else(|| data::find(doc, container, item, &saved.get("name")))
}

/// Run `f` with the data record of a saved item, if it has one.
fn with_record<T>(store: &DataStore, e: &Element, f: impl FnOnce(Record<'_>) -> T) -> Option<T> {
    let (file, container, item) = match tag_of(e) {
        "gear" => {
            let doc = store.doc("gear.xml").ok()?;
            return gear::record_of(&doc, e).map(f);
        }
        "cyberware" | "bioware" => {
            let (file, container, item, _) = cyberware::data_source(cyberware::is_bioware(e));
            (file, container, item)
        }
        "armor" => ("armor.xml", "armors", "armor"),
        "armormod" => ("armor.xml", "mods", "mod"),
        "weapon" => ("weapons.xml", "weapons", "weapon"),
        "accessory" => ("weapons.xml", "accessories", "accessory"),
        "vehicle" => ("vehicles.xml", "vehicles", "vehicle"),
        "mod" => ("vehicles.xml", "mods", "mod"),
        "lifestyle" => ("lifestyles.xml", "lifestyles", "lifestyle"),
        _ => return None,
    };
    let doc = store.doc(file).ok()?;
    record_in(&doc, container, item, e).map(f)
}

// ---------------------------------------------------------------------------
// Improvements of an item
// ---------------------------------------------------------------------------

/// `ImprovementSource` of the bonuses of a kind that creates improvements.
fn bonus_kind(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "gear" => "Gear",
        "cyberware" => "Cyberware",
        "bioware" => "Bioware",
        "armor" => "Armor",
        "armormod" => "ArmorMod",
        _ => return None,
    })
}

/// The improvement source names of an item: its guid and the `Pair` /
/// `Wireless` suffixed ones (`Cyberware.Create`, `RefreshWirelessBonuses`).
fn sources(guid: &str) -> [String; 4] {
    [guid.to_owned(), format!("{guid}Pair"), format!("{guid}Wireless"), format!("{guid}WirelessPair")]
}

/// The container of an object a bonus created, by the improvement that
/// records it (as `cyberware::remove` does).
fn granted_object(kind: &str) -> Option<&'static str> {
    match kind {
        "FreeWare" => Some("cyberwares"),
        "SpecificQuality" => Some("qualities"),
        "LimitModifier" => Some("limitmodifiers"),
        "MentorSpirit" | "Paragon" => Some("mentorspirits"),
        _ => None,
    }
}

/// Objects created by the improvements with source `source`, as
/// `(container, id)`.
fn granted_by(ch: &Character, source: &str) -> Vec<(&'static str, String)> {
    ch.improvements
        .list
        .iter()
        .filter(|i| i.source_name.eq_ignore_ascii_case(source))
        .filter_map(|i| granted_object(&i.kind).map(|c| (c, i.improved_name.clone())))
        .collect()
}

fn remove_granted(ch: &mut Character, owned: Vec<(&'static str, String)>) {
    for (container, id) in owned {
        if container == "cyberwares" {
            cyberware::remove(ch, &id);
        } else {
            ch.remove_item(container, &id);
        }
    }
}

/// Remove the improvements with source `source` and the objects they
/// created (`ImprovementManager.RemoveImprovements`).
fn drop_source(ch: &mut Character, source: &str) {
    let owned = granted_by(ch, source);
    ch.improvements.remove_from_source(source);
    remove_granted(ch, owned);
}

/// Enable or disable the improvements with source `source`.
fn set_source_enabled(ch: &mut Character, source: &str, on: bool) {
    for i in ch.improvements.list.iter_mut().filter(|i| i.source_name.eq_ignore_ascii_case(source)) {
        i.enabled = on;
    }
}

/// Apply a bonus node for the item; returns the selected value.
#[allow(clippy::too_many_arguments)]
fn apply_bonus(ch: &mut Character, store: &DataStore, node: &Element, kind: &str, source: &str, name: &str, rating: i32, forced: Option<&str>) -> Option<String> {
    node.elements().next()?;
    let src = BonusSource { kind: kind.into(), guid: source.to_owned(), name: name.to_owned(), rating };
    let out = bonus::apply(ch, store, node, &src, forced);
    for (container, el) in out.added.iter().cloned() {
        ch.items_mut(&container).push(el);
    }
    super::apply_outcome(ch, &out);
    out.selected
}

/// The data bonus nodes of an item (saved bonus nodes lose their
/// attributes, e.g. `wirelessbonus/@mode`).
fn data_node(store: &DataStore, e: &Element, node: &str) -> Option<Element> {
    with_record(store, e, |r| r.el().child(node).cloned()).flatten().filter(|b| b.elements().next().is_some())
}

/// Foci apply their bonus only when bonded (`Gear.Create`, `bonus_applies`
/// in `items::gear`), weapon foci always.
fn gear_bonus_applies(e: &Element, bonus: &Element) -> bool {
    !matches!(e.get("category").as_str(), "Foci" | "Metamagic Foci") || e.get_bool("bonded").unwrap_or(false) || bonus.child("selectweapon").is_some()
}

/// Recreate an item's bonus and wireless bonus improvements at its current
/// rating (`nudGearRating_ValueChanged`: `RemoveImprovements` then
/// `CreateImprovements` with the extra as forced value). Pair bonuses are
/// left alone.
fn refresh_bonuses(ch: &mut Character, store: &DataStore, guid: &str) {
    let Some(e) = find(ch, guid).cloned() else { return };
    let Some(kind) = bonus_kind(tag_of(&e)) else { return };
    let rating = e.get_i32("rating").unwrap_or(0);
    let extra = e.get("extra");
    let forced = Some(extra.trim_end_matches(", Hacked")).filter(|x| !x.is_empty());
    let name = e.get("name");
    drop_source(ch, guid);
    drop_source(ch, &format!("{guid}Wireless"));
    if let Some(b) = data_node(store, &e, "bonus") {
        if e.name != "gear" || gear_bonus_applies(&e, &b) {
            apply_bonus(ch, store, &b, kind, guid, &name, rating, forced);
        }
    }
    if !is_equipped(&e) {
        set_tree_enabled(ch, guid, false);
    }
    refresh_wireless(ch, store, guid);
}

/// `RefreshWirelessBonuses`: with wireless on (and the item equipped), the
/// data `wirelessbonus` applies as `{guid}Wireless`, replacing the base
/// bonus when `@mode="replace"`; with it off, those improvements go and
/// the base ones come back.
fn refresh_wireless(ch: &mut Character, store: &DataStore, guid: &str) {
    let Some(e) = find(ch, guid).cloned() else { return };
    let Some(kind) = bonus_kind(tag_of(&e)) else { return };
    let Some(wb) = data_node(store, &e, "wirelessbonus") else { return };
    let replace = wb.attr("mode") == Some("replace");
    let source = format!("{guid}Wireless");
    drop_source(ch, &source);
    let on = e.get_bool("wirelesson").unwrap_or(false) && is_equipped(&e);
    if replace {
        set_source_enabled(ch, guid, !on && is_equipped(&e));
    }
    if on {
        let extra = e.get("extra");
        let forced = Some(extra.as_str()).filter(|x| !x.is_empty());
        let selected = apply_bonus(ch, store, &wb, kind, &source, &e.get("name"), e.get_i32("rating").unwrap_or(0), forced);
        if let (Some(s), true) = (selected.filter(|s| !s.is_empty()), extra.is_empty()) {
            if let Some(m) = find_mut(ch, guid) {
                m.set_child_text("extra", s);
            }
        }
    }
}

fn is_equipped(e: &Element) -> bool {
    e.get_bool("equipped").unwrap_or(true)
}

/// Enable or disable the improvements of an item and everything in it.
/// Enabling skips nested items that are themselves unequipped
/// (`Armor.Equipped`, `Gear.ChangeEquippedStatus`).
fn set_tree_enabled(ch: &mut Character, guid: &str, on: bool) {
    let Some(e) = find(ch, guid).cloned() else { return };
    fn walk(e: &Element, on: bool, top: bool, out: &mut Vec<String>) {
        if ITEM_TAGS.contains(&e.name.as_str()) {
            if on && !top && !is_equipped(e) {
                return;
            }
            out.push(e.get("guid"));
        }
        for c in e.elements() {
            walk(c, on, false, out);
        }
    }
    let mut guids = Vec::new();
    walk(&e, on, true, &mut guids);
    for g in guids {
        for s in sources(&g) {
            set_source_enabled(ch, &s, on);
        }
    }
}

// ---------------------------------------------------------------------------
// Rating
// ---------------------------------------------------------------------------

/// The rating an item may have, `(min, max)`, or `None` when it has no
/// rating. Per kind, as the selection dialogs compute it: gear
/// (`gear::rating_range`, `Parent Rating` from its parent), ware
/// (`cyberware::rating_range`, cyberlimb tokens from the limb it is in),
/// vehicle mods (`vehicle::max_rating`, min 1), and the record's
/// `rating`/`maxrating`/`minrating` for armor, armor mods, weapons and
/// accessories.
pub fn rating_range(ch: &Character, store: &DataStore, guid: &str) -> Option<(i32, i32)> {
    let e = find(ch, guid)?;
    let parent = parent(ch, guid);
    let rating = e.get_i32("rating").unwrap_or(0);
    let eval = |s: &str| expr::standard_round(armor::rating_value(s, rating));
    let simple = |max: &str, min: &str| -> Option<(i32, i32)> {
        let m = if max.trim().is_empty() { 0 } else { eval(max) };
        if m <= 0 {
            return None;
        }
        let lo = if min.trim().is_empty() { 1 } else { eval(min).max(0) };
        Some((lo.min(m), m))
    };
    match tag_of(e) {
        "gear" => {
            if e.get("name") == "Sensor Array" && e.get("category") == "Sensors" && parent.is_some_and(|p| p.name == "vehicle") {
                // Follows the vehicle's sensor (`Gear.Rating`).
                return None;
            }
            with_record(store, e, |r| gear::rating_range(r, parent)).flatten()
        }
        "cyberware" | "bioware" => {
            let (min, max) = with_record(store, e, |r| cyberware::rating_range(ch, store, r, parent.filter(|p| p.name == "cyberware")))?;
            (max > 0).then_some((min, max))
        }
        "armor" | "accessory" => with_record(store, e, |r| simple(&r.get("rating"), &r.get("minrating"))).flatten(),
        "armormod" => with_record(store, e, |r| simple(&r.get("maxrating"), &r.get("minrating"))).flatten(),
        "weapon" => with_record(store, e, |r| {
            let max = r.get("rating");
            if max == "0" { None } else { simple(&max, &r.get("minrating")) }
        })
        .flatten(),
        "mod" => {
            let v = parent.filter(|p| p.name == "vehicle")?;
            with_record(store, e, |r| {
                let max = vehicle::max_rating(v, r, rating);
                (max > 0).then_some((1, max))
            })
            .flatten()
        }
        _ => None,
    }
}

/// Set an item's rating, clamped to [`rating_range`], and refresh what
/// depends on it (see the module docs). Returns the rating set.
pub fn apply_rating_change(ch: &mut Character, store: &DataStore, guid: &str, rating: i32) -> Result<i32, String> {
    let (min, max) = rating_range(ch, store, guid).ok_or("this item has no rating")?;
    let new = rating.clamp(min, max);
    let e = find_mut(ch, guid).ok_or_else(|| format!("no item with guid {guid}"))?;
    let tag = e.name.clone();
    e.set_child_text("rating", new.to_string());
    ch.dirty = true;
    if tag == "gear" {
        // `Gear.Rating`: children whose limits follow the parent are
        // clamped again.
        let kids: Vec<String> = find(ch, guid)
            .and_then(|g| g.child("children"))
            .map(|c| c.children_named("gear").filter(|k| k.get("minrating").contains("Parent") || k.get("maxrating").contains("Parent")).map(|k| k.get("guid")).collect())
            .unwrap_or_default();
        for k in kids {
            let r = find(ch, &k).and_then(|x| x.get_i32("rating")).unwrap_or(0);
            let _ = apply_rating_change(ch, store, &k, r);
        }
    }
    refresh_bonuses(ch, store, guid);
    Ok(new)
}

// ---------------------------------------------------------------------------
// Other editable fields
// ---------------------------------------------------------------------------

/// Set a gear's quantity (`nudGearQty_ValueChanged`): at least 1 (or the
/// pack size `costfor`, when that is smaller than 1).
pub fn set_quantity(ch: &mut Character, guid: &str, qty: f64) -> bool {
    let Some(e) = find_mut(ch, guid).filter(|e| matches!(e.name.as_str(), "gear" | "drug")) else { return false };
    let min = e.get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0).min(1.0);
    e.set_child_text("qty", crate::improvement::fmt_num(qty.max(min)));
    ch.dirty = true;
    true
}

/// Whether the item kind has an equipped state in Chummer's detail pane.
pub fn can_equip(e: &Element) -> bool {
    matches!(e.name.as_str(), "gear" | "armor" | "armormod" | "weapon" | "accessory" | "mod") && e.child("equipped").is_some()
}

/// Equip or unequip an item (`chkArmorEquipped` → `Armor.Equipped`,
/// `chkGearEquipped` → `Gear.ChangeEquippedStatus`, weapons and
/// accessories): the flag, and the improvements of the item and what it
/// holds are enabled or disabled. Only equipped armor counts towards the
/// armor rating ([`crate::calc`]).
pub fn set_equipped(ch: &mut Character, store: &DataStore, guid: &str, on: bool) -> bool {
    let Some(e) = find_mut(ch, guid) else { return false };
    if !can_equip(e) {
        return false;
    }
    e.set_child_text("equipped", bool_str(on));
    ch.dirty = true;
    set_tree_enabled(ch, guid, on);
    if on {
        // A wireless bonus that replaces the base bonus keeps it off.
        refresh_wireless(ch, store, guid);
    }
    true
}

/// Turn an item's wireless on or off (`chkGearWireless`,
/// `chkCyberwareWireless`, ...). Gear, ware and armor refresh their
/// wireless bonus improvements (`RefreshWirelessBonuses`); weapons,
/// accessories and vehicles only store the flag, which their stats read.
pub fn set_wireless(ch: &mut Character, store: &DataStore, guid: &str, on: bool) -> bool {
    let Some(e) = find_mut(ch, guid) else { return false };
    if e.child("wirelesson").is_none() {
        return false;
    }
    e.set_child_text("wirelesson", bool_str(on));
    ch.dirty = true;
    refresh_wireless(ch, store, guid);
    true
}

/// The field holding the player's name for an item (`txtGearName`...;
/// for lifestyles the name itself).
pub fn custom_name_field(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "gear" => "gearname",
        "weapon" => "weaponname",
        "armor" => "armorname",
        "vehicle" => "vehiclename",
        "lifestyle" => "name",
        _ => return None,
    })
}

/// Whether the item has a location (Chummer's gear/armor/weapon/vehicle
/// locations; a ware's `location` is its side and is not editable).
pub fn has_location(ch: &Character, guid: &str) -> bool {
    find(ch, guid).is_some_and(|e| matches!(e.name.as_str(), "gear" | "armor" | "weapon" | "vehicle") && e.child("location").is_some()) && parent(ch, guid).is_none()
}

/// The locations container for a top-level item kind (`gear` → `gearlocations`).
fn locations_container(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "gear" => "gearlocations",
        "armor" => "armorlocations",
        "weapon" => "weaponlocations",
        "vehicle" => "vehiclelocations",
        _ => return None,
    })
}

/// `(guid, name)` of the locations an item can be put in.
pub fn locations(ch: &Character, guid: &str) -> Vec<(String, String)> {
    let Some(c) = find(ch, guid).and_then(|e| locations_container(&e.name)) else { return Vec::new() };
    ch.doc.child(c).map(|c| c.children_named("location").map(|l| (l.get("guid"), l.get("name"))).collect()).unwrap_or_default()
}

/// Add a location for the item's kind, as `Location.Save` writes it, and
/// return its guid.
pub fn add_location(ch: &mut Character, guid: &str, name: &str) -> Option<String> {
    let c = find(ch, guid).and_then(|e| locations_container(&e.name))?;
    let id = super::new_guid();
    let list = ch.doc.child_or_insert(c);
    let order = list.children_named("location").count();
    let mut l = Element::new("location");
    for (k, v) in [("guid", id.as_str()), ("name", name), ("notes", ""), ("notesColor", "Chocolate"), ("sortorder", &order.to_string())] {
        l.push(Element::with_text(k, v));
    }
    list.push(l);
    ch.dirty = true;
    Some(id)
}

/// Set a plain text field of an item (custom name, location, notes).
pub fn set_text(ch: &mut Character, guid: &str, field: &str, value: &str) -> bool {
    let Some(e) = find_mut(ch, guid) else { return false };
    e.set_child_text(field, value);
    ch.dirty = true;
    true
}

// ---------------------------------------------------------------------------
// Computed values
// ---------------------------------------------------------------------------

/// Total nuyen cost of an item with what it holds, by the kind's cost
/// function (`TotalCost`). Vehicle mods and weapon mounts use
/// `chargen::item_cost`: their `Veh`-based cost is private to
/// `items::vehicle`.
pub fn total_cost(ch: &Character, store: &DataStore, guid: &str) -> f64 {
    let Some(e) = find(ch, guid) else { return 0.0 };
    let parent = parent(ch, guid);
    match tag_of(e) {
        "gear" => parent.map_or_else(|| gear::cost(e), |p| gear::cost_in(e, p)),
        "cyberware" | "bioware" => cyberware::cost(ch, store, e),
        "armor" | "armormod" => armor::cost(e),
        "weapon" => weapon::cost(e),
        "accessory" => {
            let gear: f64 = e.child("gears").map(|g| g.children_named("gear").map(|k| gear::cost_in(k, e)).sum()).unwrap_or(0.0);
            parent.filter(|p| p.name == "weapon").map_or_else(|| crate::chargen::item_cost(e), |w| weapon::accessory_cost(e, w) + gear)
        }
        "vehicle" => vehicle::cost(e),
        "lifestyle" => lifestyle::total_cost(ch, e),
        "drug" => drug::cost_with(Some(store), e),
        _ => crate::chargen::item_cost(e),
    }
}

/// What selling the item pays at `fraction` of its total cost.
pub fn sale_value(ch: &Character, store: &DataStore, guid: &str, fraction: f64) -> f64 {
    total_cost(ch, store, guid) * fraction
}

/// Availability as shown (`TotalAvail`, own part): ware with its grade
/// modifier (`cyberware::availability`), else the saved expression at the
/// item's rating. Expressions that need the parent item stay as written.
pub fn availability(ch: &Character, store: &DataStore, guid: &str) -> String {
    let Some(e) = find(ch, guid) else { return String::new() };
    let v = e.get("avail");
    if v.trim().is_empty() {
        return String::new();
    }
    if e.name == "cyberware" {
        return cyberware::availability(ch, store, e).to_string();
    }
    if v.contains('{') || v.contains("Parent") || v.contains("Gear") {
        return v;
    }
    let rating = e.get_i32("rating").unwrap_or(0);
    let min = e.get_i32("minrating").unwrap_or(0);
    Availability::parse(&v, rating, min, &NoAttributes).to_string()
}

/// Essence cost of ware (`Cyberware.CalculatedESS`), `None` for other kinds.
pub fn essence(ch: &Character, store: &DataStore, rules: &Rules, guid: &str) -> Option<f64> {
    let e = find(ch, guid).filter(|e| e.name == "cyberware")?;
    Some(cyberware::essence(ch, store, rules, e))
}

/// `(provides, consumes)` of a capacity string at `rating`: `n` provides
/// n, `[m]` consumes m of the parent, `n/[m]` both; `[*]` consumes nothing.
pub fn parse_capacity(s: &str, rating: i32) -> (f64, f64) {
    let s = s.trim();
    let val = |t: &str| {
        let t = t.trim().trim_start_matches('[').trim_end_matches(']');
        if t.is_empty() || t == "*" { 0.0 } else { armor::rating_value(t, rating) }
    };
    if let Some((a, b)) = s.split_once("/[") {
        return (val(a), val(b));
    }
    if s.starts_with('[') { (0.0, val(s)) } else { (val(s), 0.0) }
}

/// Capacity used and total (`CapacityRemaining`, simplified: the item's
/// own capacity at its rating against what its children consume), or
/// vehicle mod slots. `None` when the item provides no capacity.
pub fn capacity(ch: &Character, guid: &str) -> Option<(f64, f64)> {
    let e = find(ch, guid)?;
    let consumed = |list: &str, item: &str, field: &str| -> f64 {
        e.child(list).map(|c| c.children_named(item).map(|k| parse_capacity(&k.get(field), k.get_i32("rating").unwrap_or(0)).1).sum()).unwrap_or(0.0)
    };
    let rating = e.get_i32("rating").unwrap_or(0);
    let (total, used) = match e.name.as_str() {
        "gear" => (parse_capacity(&e.get("capacity"), rating).0, consumed("children", "gear", "capacity")),
        "cyberware" => (parse_capacity(&e.get("capacity"), rating).0, consumed("children", "cyberware", "capacity") + consumed("gears", "gear", "capacity")),
        "armor" => (parse_capacity(&e.get("armorcapacity"), rating).0, consumed("armormods", "armormod", "armorcapacity") + consumed("gears", "gear", "armorcapacity")),
        "armormod" => (parse_capacity(&e.get("gearcapacity"), rating).0, consumed("gears", "gear", "armorcapacity")),
        "vehicle" => {
            let st = vehicle::stats(e);
            return Some(if st.is_drone { (f64::from(st.drone_mod_slots_used), f64::from(st.drone_mod_slots)) } else { (f64::from(st.slots_used), f64::from(st.slots)) });
        }
        _ => return None,
    };
    (total > 0.0).then_some((used, total))
}

/// Accessory mounts of a weapon no accessory uses (the mount list of
/// `SelectWeaponAccessory`, see `weapon::mount_options`).
pub fn free_mounts(ch: &Character, guid: &str) -> Vec<String> {
    let Some(w) = find(ch, guid).filter(|e| e.name == "weapon") else { return Vec::new() };
    let used: Vec<String> = w
        .child("accessories")
        .map(|c| c.children_named("accessory").flat_map(|a| [a.get("mount"), a.get("extramount")]).collect())
        .unwrap_or_default();
    w.get("weaponslots").split('/').filter(|s| !s.is_empty() && !used.iter().any(|u| u == s)).map(str::to_owned).collect()
}

/// Data-included items (`IncludedInParent`): Chummer does not let them be
/// sold or deleted on their own.
pub fn is_included(ch: &Character, guid: &str) -> bool {
    let Some(e) = find(ch, guid) else { return false };
    if e.get_bool("included").unwrap_or(false) || e.get_bool("includedinparent").unwrap_or(false) {
        return true;
    }
    let granted = !e.get("parentid").is_empty();
    match e.name.as_str() {
        "gear" | "cyberware" => granted,
        // Weapons granted by an armor, gear or ware sit at the top level.
        "weapon" => granted && parent(ch, guid).is_none(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Children
// ---------------------------------------------------------------------------

/// A kind of item that can be added inside another one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildKind {
    /// Kind tag for the selection dialog (see [`super::KINDS`]).
    pub tag: &'static str,
    pub label: &'static str,
}

/// What can be added into an item (`CharacterShared` context menus:
/// "Add Gear" / "Add as Plugin", "Add Cyberware" into ware with
/// `allowsubsystems` or capacity, "Add Armor Mod", "Add Accessory",
/// "Add Underbarrel Weapon", "Add Vehicle Mod", "Add Weapon" to weapon
/// mounts). Weapon mounts are added with [`add_weapon_mount`].
pub fn child_kinds(ch: &Character, guid: &str) -> Vec<ChildKind> {
    let Some(e) = find(ch, guid) else { return Vec::new() };
    let k = |tag, label| ChildKind { tag, label };
    let mut v = Vec::new();
    match tag_of(e) {
        "gear" => v.push(k("gear", "Gear (plugin)")),
        tag @ ("cyberware" | "bioware") => {
            let room = !e.get("subsystems").is_empty() || parse_capacity(&e.get("capacity"), e.get_i32("rating").unwrap_or(0)).0 > 0.0;
            if room {
                v.push(if tag == "bioware" { k("bioware", "Bioware") } else { k("cyberware", "Cyberware") });
            }
            if e.child("allowgear").is_some() {
                v.push(k("gear", "Gear"));
            }
        }
        "armor" => {
            v.push(k("armormod", "Armor mod"));
            v.push(k("gear", "Gear"));
        }
        "armormod" if !e.get("gearcapacity").is_empty() => v.push(k("gear", "Gear")),
        "weapon" => {
            if e.get_bool("allowaccessory").unwrap_or(true) {
                v.push(k("accessory", "Accessory"));
            }
            let is_underbarrel = parent(ch, guid).is_some_and(|p| p.name == "weapon");
            if e.child("underbarrel").is_none() && !is_underbarrel && e.get_bool("allowaccessory").unwrap_or(true) && e.get("weaponslots").contains("Under") {
                v.push(k("weapon", "Underbarrel weapon"));
            }
        }
        "accessory" if e.child("allowgear").is_some() => v.push(k("gear", "Gear")),
        "vehicle" => {
            v.push(k("mod", "Vehicle mod"));
            v.push(k("gear", "Gear"));
        }
        "weaponmount" => {
            let n = e.child("weapons").map_or(0, |w| w.elements().count()) as i32;
            if n < e.get_i32("weaponcapacity").unwrap_or(1) {
                v.push(k("weapon", "Weapon"));
            }
        }
        "mod" if !e.get("weaponmountcategories").is_empty() || e.get("name").contains("Weapon Mount") => v.push(k("weapon", "Weapon")),
        _ => {}
    }
    v
}

/// Gear categories that may go into an item: the `addoncategory` list of
/// its data record (`SelectGear`'s allowed categories when adding into
/// gear or armor). Empty when any category may.
pub fn addon_categories(ch: &Character, store: &DataStore, guid: &str) -> Vec<String> {
    let Some(e) = find(ch, guid) else { return Vec::new() };
    if !matches!(tag_of(e), "gear" | "armor" | "armormod") {
        return Vec::new();
    }
    with_record(store, e, |r| r.el().children_named("addoncategory").map(Element::text).filter(|c| !c.is_empty()).collect()).unwrap_or_default()
}

/// The items directly inside an item: `(guid, tag, name)`.
pub fn children(ch: &Character, guid: &str) -> Vec<(String, String, String)> {
    let Some(e) = find(ch, guid) else { return Vec::new() };
    let mut v = Vec::new();
    fn walk(e: &Element, v: &mut Vec<(String, String, String)>) {
        for c in e.elements() {
            if ITEM_TAGS.contains(&c.name.as_str()) {
                v.push((c.get("guid"), tag_of(c).to_owned(), c.get("name")));
            } else if c.child("guid").is_none() && !matches!(c.name.as_str(), "bonus" | "wirelessbonus" | "pairbonus" | "wirelesspairbonus") {
                walk(c, v);
            }
        }
    }
    walk(e, &mut v);
    v
}

/// Put an item the generic [`super::add`] just created where it belongs:
/// `weapon::add` wraps a weapon with a parent in `<underbarrel>`, which is
/// right for weapons but not for a vehicle weapon mount or mod, whose
/// weapons go in `<weapons>` (as `vehicle::add_data_weapons` does). Call
/// after every successful add; it does nothing for other items.
pub fn settle_new_item(ch: &mut Character, guid: &str) {
    let Some(host) = parent(ch, guid).filter(|p| matches!(p.name.as_str(), "weaponmount" | "mod")).map(|p| p.get("guid")) else { return };
    let Some(h) = find_mut(ch, &host) else { return };
    let mut moved = None;
    for n in h.children.iter_mut() {
        if let Node::Element(ub) = n {
            if ub.name == "underbarrel" {
                if let Some(i) = ub.children.iter().position(|c| matches!(c, Node::Element(w) if w.get("guid").eq_ignore_ascii_case(guid))) {
                    if let Node::Element(w) = ub.children.remove(i) {
                        moved = Some(w);
                    }
                    break;
                }
            }
        }
    }
    if let Some(w) = moved {
        h.children.retain(|n| !matches!(n, Node::Element(u) if u.name == "underbarrel" && u.elements().next().is_none()));
        h.child_or_insert("weapons").push(w);
        ch.dirty = true;
    }
}

/// Weapon mount sizes (`weaponmounts/weaponmount` with category "Size"):
/// `(id, name)`.
pub fn weapon_mount_sizes(store: &DataStore) -> Vec<(String, String)> {
    let Ok(doc) = store.doc("vehicles.xml") else { return Vec::new() };
    doc.child("weaponmounts")
        .map(|c| c.children_named("weaponmount").filter(|m| m.get("category") == "Size" && m.child("hide").is_none()).map(|m| (m.get("id"), m.get("name"))).collect())
        .unwrap_or_default()
}

/// Add a weapon mount of `size` to a vehicle (`CreateWeaponMount`). In
/// career mode it is paid and logged like other purchases
/// (`career::pay_for_item`) instead of the plain nuyen deduction
/// `vehicle::add_weapon_mount` does; if it cannot be paid it is taken back.
pub fn add_weapon_mount(ch: &mut Character, store: &DataStore, vehicle_guid: &str, size: &str) -> Result<String, String> {
    let nuyen = ch.nuyen;
    let p = Purchase { cost_multiplier: 1.0, ..Default::default() };
    let guid = vehicle::add_weapon_mount(ch, store, vehicle_guid, size, &[], &p)?;
    if ch.created {
        ch.nuyen = nuyen;
        let cost = find(ch, &guid).map_or(0.0, crate::chargen::item_cost);
        if cost > 0.0 {
            if let Err(e) = career::pay_for_item(ch, "weaponmount", Some("vehicle"), &guid, cost) {
                ch.remove_item_anywhere(&guid);
                return Err(e.to_string());
            }
        }
    }
    Ok(guid)
}

// ---------------------------------------------------------------------------
// Removing and selling
// ---------------------------------------------------------------------------

/// Top-level weapons another item created (`addweapon`, natural weapons):
/// their `parentid` names an item of `guids`.
fn granted_weapons(ch: &Character, guids: &[String]) -> Vec<String> {
    ch.items("weapons", "weapon")
        .into_iter()
        .filter(|w| {
            let p = w.get("parentid");
            !p.is_empty() && guids.iter().any(|g| g.eq_ignore_ascii_case(&p))
        })
        .map(|w| w.get("guid"))
        .collect()
}

/// Delete an item in creation mode (`cmdDelete*`): ware through
/// `cyberware::remove`, anything else with every improvement it and the
/// items inside it made (with the objects they created) and the weapons
/// it added.
pub fn remove(ch: &mut Character, guid: &str) -> bool {
    let Some(e) = find(ch, guid).cloned() else { return false };
    let mut guids = Vec::new();
    subtree_guids(&e, &mut guids);
    let weapons = granted_weapons(ch, &guids);
    for g in &guids {
        for s in sources(g) {
            drop_source(ch, &s);
        }
    }
    let removed = if e.name == "cyberware" { cyberware::remove(ch, guid) } else { ch.remove_item_anywhere(guid) };
    for w in weapons {
        remove(ch, &w);
    }
    removed
}

/// Sell an item in career mode (`ICanSell.Sell` via [`career::sell_item`]
/// at `fraction` of its cost), then clean up what `sell_item` leaves: the
/// `Pair`/`Wireless` improvements, objects the item's bonuses created and
/// weapons it added. Returns the nuyen received.
pub fn sell(ch: &mut Character, store: &DataStore, guid: &str, fraction: f64) -> Result<f64, CareerError> {
    let value = sale_value(ch, store, guid, fraction);
    let e = find(ch, guid).cloned().ok_or_else(|| CareerError::NotFound(format!("item {guid}")))?;
    let mut guids = Vec::new();
    subtree_guids(&e, &mut guids);
    let weapons = granted_weapons(ch, &guids);
    // `sell_item` drops the improvements of these guids, but not the
    // objects they created: note those first.
    let owned: Vec<(&'static str, String)> = guids.iter().flat_map(|g| granted_by(ch, g)).collect();
    let amount = career::sell_item_valued(ch, guid, value)?;
    remove_granted(ch, owned);
    for g in &guids {
        for s in sources(g) {
            drop_source(ch, &s);
        }
    }
    for w in weapons {
        remove(ch, &w);
    }
    Ok(amount)
}
