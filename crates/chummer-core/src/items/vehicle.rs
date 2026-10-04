//! Vehicles and drones, vehicle mods and weapon mounts (`Vehicle`,
//! `VehicleMod`, `WeaponMount`, `WeaponMountOption`).
//!
//! - [`element`] builds a saved element from a data record
//!   (`Create` + `Save`), [`add`] puts it on the character.
//! - [`stats`] / [`stats_with`] compute the totals after mods.
//! - [`cost`] is the total nuyen cost of a saved vehicle.
//!
//! Gear and weapons inside a vehicle are created through the generic
//! [`super::add`] dispatch, so they appear once those kinds are supported.

mod cost;
mod stats;

pub use cost::{cost, own_cost, part_costs, PartCosts};
pub use stats::{is_drone, stats, stats_with, CategorySlots, VehicleRules, VehicleStats, SLOT_CATEGORIES};

use crate::bonus::{self, BonusSource, Choice};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr::{evaluate_num, needs_evaluation, parse_plain, standard_round};
use crate::improvement::bool_str;
use crate::xml::{Element, Node};

use super::{find_by_guid_mut, new_guid, Purchase};
use stats::Veh;

const FILE: &str = "vehicles.xml";

/// `Vehicle.MaxWheels`: the maximum of a mod rated in "qty".
const MAX_WHEELS: i32 = 50;

/// Fields the oracle does not compare for vehicles, vehicle mods and
/// weapon mounts. The fixtures were saved by Chummer 5.202.
pub const IGNORE: &[&str] = &[
    // vehicle: `id` is the pre-5.214 name of `sourceid`; `devicerating` was
    // saved as the computed value (= Pilot), now the data string (usually
    // empty); `vehiclename` and `physicalcmfilled` are user state.
    "id", "devicerating", "vehiclename", "physicalcmfilled",
    // mod: old saves stored the resolved maximum rating ("15" for
    // `<rating>body</rating>`, "0" for none); now the data string is saved.
    // `markup` is no longer saved.
    "maxrating", "markup",
    // weapon mount: misspelled in old saves; options were saved with `id`
    // and no guid, so the subtree cannot match the current format.
    "equuipped", "weaponmountoptions",
];

// -------------------------------------------------------------------
// Data lookup
// -------------------------------------------------------------------

/// Record by id or name in `container/item` of vehicles.xml.
fn find_rec<'a>(doc: &'a Element, container: &str, item: &'a str, key: &str) -> Option<Record<'a>> {
    crate::data::find(doc, container, item, key)
}

/// `TryGetNodeByNameOrId("/chummer/weaponmounts/weaponmount", key, "category = ...")`.
fn find_mount<'a>(doc: &'a Element, key: &str, category: Option<&str>) -> Option<Record<'a>> {
    let c = doc.child("weaponmounts")?;
    let ok = |e: &&Element| category.is_none_or(|cat| e.get("category") == cat);
    c.children_named("weaponmount")
        .filter(ok)
        .find(|e| e.get("id").eq_ignore_ascii_case(key))
        .or_else(|| c.children_named("weaponmount").filter(ok).find(|e| e.get("name") == key))
        .map(Record)
}

/// A vehicle mod record: `/chummer/mods/mod`, then weapon mount mods.
fn find_mod<'a>(doc: &'a Element, key: &str) -> Option<Record<'a>> {
    find_rec(doc, "mods", "mod", key).or_else(|| find_rec(doc, "weaponmountmods", "mod", key))
}

/// Saved `sourceid` (or legacy `id`), else the name: the key to the record.
fn saved_key(saved: &Element) -> String {
    ["sourceid", "id"].iter().filter_map(|k| saved.child_text(k)).find(|s| !s.trim().is_empty()).unwrap_or_else(|| saved.get("name"))
}

// -------------------------------------------------------------------
// Elements
// -------------------------------------------------------------------

/// Build a saved element of kind `tag` ("vehicle", "mod", "weaponmount")
/// from its data record. A mod is rated against its parent vehicle when
/// `p.parent` names one on the character.
pub fn element(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase, guid: &str) -> Result<Element, String> {
    match tag {
        "vehicle" => vehicle_element(store, rec, p, guid),
        "mod" => {
            let parent = p.parent.as_deref().and_then(|g| find_by_guid(&ch.doc, g));
            let vehicle = parent.filter(|e| e.name == "vehicle");
            let rating = vehicle.map_or(p.rating, |v| clamp_rating(v, rec, p.rating));
            let mut e = mod_element(rec, rating, p.answer.as_deref().unwrap_or(""), false, guid);
            apply_purchase(&mut e, p);
            Ok(e)
        }
        "weaponmount" => {
            let mut e = mount_element(rec, guid, false);
            apply_purchase(&mut e, p);
            Ok(e)
        }
        _ => Err(format!("{tag} is not a vehicle item")),
    }
}

/// Free and discounted purchases.
fn apply_purchase(e: &mut Element, p: &Purchase) {
    if p.free {
        e.set_child_text("cost", "0");
    }
    if p.cost_multiplier > 0.0 && p.cost_multiplier < 1.0 {
        e.set_child_text("discountedcost", bool_str(true));
    }
}

/// Lower bound of a `Variable(min-max)` cost (what Chummer uses when the
/// selection dialog is skipped); other costs unchanged.
fn resolve_variable_cost(cost: &str) -> String {
    match cost.strip_prefix("Variable(") {
        Some(rest) => {
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            inner.split_once('-').map_or(inner, |(a, _)| a).trim_start_matches('+').to_owned()
        }
        None => cost.to_owned(),
    }
}

/// `Vehicle.Create` + `Vehicle.Save`, with the data's included mods and
/// weapon mounts. Gear and weapons are added by [`add`].
pub fn vehicle_element(store: &DataStore, rec: Record<'_>, p: &Purchase, guid: &str) -> Result<Element, String> {
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let mut v = vehicle_fields(rec, guid, &resolve_variable_cost(&rec.get("cost")));
    apply_purchase(&mut v, p);
    for m in included_mods(&doc, rec.el(), &v) {
        v.child_or_insert("mods").push(m);
    }
    let mounts: Vec<Element> = rec.el().child("weaponmounts").into_iter().flat_map(|w| w.children_named("weaponmount")).filter_map(|n| mount_by_name(&doc, n)).collect();
    for m in mounts {
        v.child_or_insert("weaponmounts").push(m);
    }
    Ok(v)
}

/// The vehicle's own fields in `Vehicle.Save` order, with empty child lists.
fn vehicle_fields(rec: Record<'_>, guid: &str, cost: &str) -> Element {
    let e = rec.el();
    let num = |k: &str| e.get_i32(k).unwrap_or(0).to_string();
    let (handling, offroad_handling) = stats::split_pair(e, "handling", "");
    let (accel, offroad_accel) = stats::split_pair(e, "accel", "");
    let (speed, offroad_speed) = stats::split_pair(e, "speed", "");
    let mut v = Element::new("vehicle");
    let mut put = |k: &str, val: String| v.push(Element::with_text(k, val));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", rec.category());
    put("handling", handling.to_string());
    put("offroadhandling", offroad_handling.to_string());
    put("accel", accel.to_string());
    put("offroadaccel", offroad_accel.to_string());
    put("speed", speed.to_string());
    put("offroadspeed", offroad_speed.to_string());
    for k in ["pilot", "body", "seats", "armor", "sensor"] {
        put(k, num(k));
    }
    put("avail", e.get("avail"));
    put("cost", cost.to_owned());
    put("addslots", e.path("mods/addslots").map_or(0, |a| a.text().trim().parse().unwrap_or(0)).to_string());
    // `modslots` defaults to Body.
    put("modslots", e.get_i32("modslots").unwrap_or_else(|| e.get_i32("body").unwrap_or(0)).to_string());
    for k in ["powertrainmodslots", "protectionmodslots", "weaponmodslots", "bodymodslots", "electromagneticmodslots", "cosmeticmodslots"] {
        put(k, num(k));
    }
    put("source", rec.source());
    put("page", rec.page());
    put("parentid", String::new());
    put("stolen", bool_str(false));
    put("physicalcmfilled", "0".into());
    put("matrixcmfilled", "0".into());
    put("vehiclename", String::new());
    for c in ["mods", "weaponmounts", "gears", "weapons"] {
        v.push(Element::new(c));
    }
    let mut put = |k: &str, val: String| v.push(Element::with_text(k, val));
    put("location", String::new());
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("discountedcost", bool_str(false));
    put("dealerconnection", bool_str(false));
    put("active", bool_str(false));
    put("homenode", bool_str(false));
    put("devicerating", e.get("devicerating"));
    put("programlimit", e.get("programs"));
    put("overclocked", "None".into());
    let (attack, sleaze, dp, fw) = matrix_attributes(e);
    put("attack", attack);
    put("sleaze", sleaze);
    put("dataprocessing", dp);
    put("firewall", fw);
    put("attributearray", e.get("attributearray"));
    for k in ["modattack", "modsleaze", "moddataprocessing", "modfirewall", "modattributearray"] {
        put(k, e.get(k));
    }
    put("canswapattributes", bool_str(e.child("attributearray").is_some()));
    put("sortorder", "0".into());
    v
}

/// Attack, sleaze, data processing and firewall: from `attributearray`
/// when present, else the single fields.
fn matrix_attributes(e: &Element) -> (String, String, String, String) {
    match e.child_text("attributearray") {
        Some(a) => {
            let mut it = a.split(',').map(str::to_owned);
            let mut next = || it.next().unwrap_or_default();
            (next(), next(), next(), next())
        }
        None => (e.get("attack"), e.get("sleaze"), e.get("dataprocessing"), e.get("firewall")),
    }
}

/// Mods listed in a vehicle record, in both data forms:
/// `<name rating="" select="">` and `<mod><name select=""/><rating/></mod>`.
fn included_mods(doc: &Element, rec: &Element, vehicle: &Element) -> Vec<Element> {
    let Some(mods) = rec.child("mods") else { return Vec::new() };
    let mut out = Vec::new();
    for n in mods.elements() {
        let (name, rating, select) = match n.name.as_str() {
            "name" => (n.text(), n.attr("rating").and_then(|r| r.parse().ok()).unwrap_or(0), n.attr("select").unwrap_or_default().to_owned()),
            "mod" => {
                let select = n.child("name").and_then(|x| x.attr("select")).unwrap_or_default().to_owned();
                (n.get("name"), n.get_i32("rating").unwrap_or(0), select)
            }
            _ => continue,
        };
        if name.is_empty() {
            continue;
        }
        if let Some(r) = find_rec(doc, "mods", "mod", &name) {
            let rating = clamp_rating(vehicle, r, rating);
            out.push(mod_element(r, rating, &select, true, &new_guid()));
        }
    }
    out
}

/// `VehicleMod.Create` rating: 0 for unrated mods, otherwise between 1
/// and [`max_rating`].
fn clamp_rating(vehicle: &Element, rec: Record<'_>, rating: i32) -> i32 {
    if rec.get("rating").trim().is_empty() {
        return 0;
    }
    rating.max(1).min(max_rating(vehicle, rec, rating).max(1))
}

/// `VehicleMod.MaxRating` for a mod about to be added to `vehicle`.
pub fn max_rating(vehicle: &Element, rec: Record<'_>, rating: i32) -> i32 {
    let s = rec.get("rating");
    if s.trim().is_empty() {
        return 0;
    }
    let v = Veh::new(vehicle, &VehicleRules::default());
    let n = match s.to_ascii_uppercase().as_str() {
        "QTY" => MAX_WHEELS,
        "SEATS" => v.total_seats(None),
        "BODY" => v.total_body(None),
        _ if needs_evaluation(&s) => {
            let r = rating.to_string();
            let t = s.replace("{Rating}", &r).replace("Rating", &r);
            standard_round(evaluate_num(&v.process_attrs(&t, None, None)).unwrap_or(0.0))
        }
        _ => standard_round(parse_plain(&s).unwrap_or(0.0)),
    };
    let name = rec.name().to_ascii_lowercase();
    let cap = if name.starts_with("armor") {
        v.max_armor()
    } else {
        match rec.category().to_ascii_uppercase().as_str() {
            "HANDLING" => v.max_handling(),
            "SPEED" => v.max_speed(),
            "ACCELERATION" => v.max_accel(),
            "SENSOR" => v.max_sensor(),
            "PILOT" => v.max_pilot(),
            _ if name.starts_with("pilot program") => v.max_pilot(),
            _ => i32::MAX,
        }
    };
    n.min(cap)
}

/// `VehicleMod.Create` + `VehicleMod.Save`.
pub fn mod_element(rec: Record<'_>, rating: i32, extra: &str, included: bool, guid: &str) -> Element {
    let e = rec.el();
    let max = e.get("rating");
    let label = match max.to_ascii_uppercase().as_str() {
        "QTY" => "Label_Qty",
        "SEATS" => "Label_Seats",
        _ => "String_Rating",
    };
    let subsystems: Vec<String> = e.child("subsystems").map(|s| s.children_named("subsystem").map(Element::text).collect()).unwrap_or_default();
    let mut m = Element::new("mod");
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", rec.category());
    put("limit", e.get("limit"));
    put("slots", e.child_text("slots").unwrap_or_else(|| "0".into()));
    put("capacity", e.get("capacity"));
    put("rating", rating.to_string());
    put("maxrating", max);
    put("ratinglabel", e.child_text("ratinglabel").unwrap_or_else(|| label.into()));
    put("conditionmonitor", e.get_i32("conditionmonitor").unwrap_or(0).to_string());
    put("avail", e.get("avail"));
    put("cost", resolve_variable_cost(&e.get("cost")));
    put("extra", extra.to_owned());
    put("source", rec.source());
    put("page", rec.page());
    put("included", bool_str(included));
    put("equipped", bool_str(true));
    put("wirelesson", bool_str(false));
    put("subsystems", subsystems.join(","));
    put("weaponmountcategories", e.get("weaponmountcategories"));
    put("ammobonus", decimal(e, "ammobonus"));
    put("ammobonuspercent", decimal(e, "ammobonuspercent"));
    put("ammoreplace", e.get("ammoreplace"));
    m.push(Element::new("weapons"));
    for b in ["bonus", "wirelessbonus"] {
        if let Some(x) = e.child(b) {
            m.push(x.clone());
        }
    }
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("discountedcost", bool_str(false));
    put("useownattributesforweapon", bool_str(e.get_bool("useownattributesforweapon").unwrap_or(false)));
    put("sortorder", "0".into());
    put("stolen", bool_str(false));
    m
}

/// A decimal data field as `decimal.ToString` writes it (0 when absent).
fn decimal(e: &Element, k: &str) -> String {
    crate::improvement::fmt_num(e.get_f64(k).unwrap_or(0.0))
}

/// `WeaponMount.Create` + `WeaponMount.Save`, without options or weapons.
pub fn mount_element(rec: Record<'_>, guid: &str, included: bool) -> Element {
    let e = rec.el();
    let mut m = Element::new("weaponmount");
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", rec.category());
    put("limit", e.get("limit"));
    put("slots", e.get_i32("slots").unwrap_or(0).to_string());
    put("avail", e.get("avail"));
    put("cost", resolve_variable_cost(&e.get("cost")));
    put("freecost", bool_str(false));
    put("extra", String::new());
    put("source", rec.source());
    put("page", rec.page());
    put("included", bool_str(included));
    put("equipped", bool_str(true));
    put("weaponmountcategories", e.get("weaponcategories"));
    put("weaponfilter", e.get("weaponfilter"));
    put("weaponcapacity", e.get_i32("weaponcapacity").unwrap_or(1).to_string());
    for c in ["weapons", "weaponmountoptions", "mods"] {
        m.push(Element::new(c));
    }
    let mut put = |k: &str, v: String| m.push(Element::with_text(k, v));
    put("notes", e.child_text("altnotes").unwrap_or_else(|| e.get("notes")));
    put("discountedcost", bool_str(false));
    put("sortorder", "0".into());
    put("stolen", bool_str(false));
    m
}

/// `WeaponMountOption.Create` + `Save`.
pub fn mount_option_element(rec: Record<'_>, included: bool) -> Element {
    let e = rec.el();
    let mut o = Element::new("weaponmountoption");
    let mut put = |k: &str, v: String| o.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", new_guid());
    put("name", rec.name());
    put("category", rec.category());
    put("slots", e.get_i32("slots").unwrap_or(0).to_string());
    put("avail", e.get("avail"));
    put("cost", e.child_text("cost").unwrap_or_else(|| "0".into()));
    put("includedinparent", bool_str(included));
    o
}

/// The option categories of a weapon mount, in `CreateByName` order.
const OPTION_CATEGORIES: &[(&str, &str)] = &[("flexibility", "Flexibility"), ("control", "Control"), ("visibility", "Visibility")];

/// `WeaponMount.CreateByName`: a mount listed in a vehicle record
/// (`<size>`, `<flexibility>`, `<control>`, `<visibility>`, `<mods>`).
fn mount_by_name(doc: &Element, node: &Element) -> Option<Element> {
    let size = node.get("size");
    if size.is_empty() {
        return None;
    }
    let rec = find_mount(doc, &size, Some("Size"))?;
    let mut m = mount_element(rec, &new_guid(), true);
    for (key, cat) in OPTION_CATEGORIES {
        let v = node.get(key);
        if let Some(o) = (!v.is_empty()).then(|| find_mount(doc, &v, Some(cat))).flatten() {
            m.child_or_insert("weaponmountoptions").push(mount_option_element(o, true));
        }
    }
    for n in node.child("mods").into_iter().flat_map(|x| x.children_named("mod")) {
        if let Some(r) = find_rec(doc, "weaponmountmods", "mod", &n.text()) {
            m.child_or_insert("mods").push(mod_element(r, 0, "", true, &new_guid()));
        }
    }
    Some(m)
}

// -------------------------------------------------------------------
// Adding to a character
// -------------------------------------------------------------------

/// Selections needed before adding: the mod's bonus selections
/// (`selecttext` and the like). Vehicles and mounts need none.
pub fn choices(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    if tag != "mod" {
        return Vec::new();
    }
    let src = BonusSource { kind: "VehicleMod".into(), guid: String::new(), name: rec.name(), rating: 1 };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Add a vehicle (with its gear and weapons), a mod (`p.parent` = vehicle
/// or weapon mount guid) or a weapon mount (`p.parent` = vehicle guid).
/// Mod bonuses change vehicle values only (see [`stats`]); they create no
/// character improvements. In career mode the cost is paid in nuyen.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    match tag {
        "vehicle" => add_vehicle(ch, store, rec, p),
        "mod" => add_mod(ch, store, rec, p),
        "weaponmount" => add_weapon_mount(ch, store, p.parent.as_deref().unwrap_or(""), &rec.id(), &[], p),
        _ => Err(format!("{tag} is not a vehicle item")),
    }
}

/// Add a vehicle from `vehicles.xml`, then its data gear and weapons.
fn add_vehicle(ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    let guid = new_guid();
    let v = vehicle_element(store, rec, p, &guid)?;
    ch.items_mut("vehicles").push(v);
    add_data_gear(ch, store, rec.el(), &guid);
    add_data_weapons(ch, store, rec.el(), &guid);
    let c = vehicle_of(ch, &guid).map_or(0.0, cost);
    pay(ch, p, c);
    Ok(guid)
}

/// Deduct a purchase in career mode. In creation the budget is computed
/// from the items.
fn pay(ch: &mut Character, p: &Purchase, cost: f64) {
    if ch.created && !p.free {
        let m = if p.cost_multiplier > 0.0 { p.cost_multiplier } else { 1.0 };
        ch.nuyen -= cost * m;
        ch.dirty = true;
    }
}

/// `Vehicle.Create` gear: `<gears><gear><name/><rating/></gear>` or
/// `<gear rating="" select="">Name</gear>`, added through the gear kind.
fn add_data_gear(ch: &mut Character, store: &DataStore, rec: &Element, vguid: &str) {
    let Ok(doc) = store.doc("gear.xml") else { return };
    for n in rec.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
        let (name, rating, select, qty) = match n.child("name") {
            Some(x) => (x.text(), n.get_i32("rating").unwrap_or(0), x.attr("select").or(n.attr("select")).unwrap_or_default().to_owned(), n.get_f64("qty").unwrap_or(1.0)),
            None => (n.text(), n.attr("rating").and_then(|r| r.parse().ok()).unwrap_or(0), n.attr("select").unwrap_or_default().to_owned(), n.attr("qty").and_then(|q| q.parse().ok()).unwrap_or(1.0)),
        };
        let Some(g) = crate::data::find(&doc, "gears", "gear", name.trim()) else { continue };
        let p = Purchase { rating, qty, answer: (!select.is_empty()).then_some(select), parent: Some(vguid.to_owned()), free: true, cost_multiplier: 1.0, ..Default::default() };
        // Gear support lands separately; until then this returns Err.
        if let Ok(id) = super::add("gear", ch, store, g, &p) {
            // `Gear.CreateFromNode` sets `Cost = "0"` for data-included gear.
            place(ch, &id, vguid, "gears", true);
        }
    }
}

/// `Vehicle.Create` weapons: each goes into the first weapon mount that has
/// room and allows it, else a "Weapon Mount" mod.
fn add_data_weapons(ch: &mut Character, store: &DataStore, rec: &Element, vguid: &str) {
    let Ok(doc) = store.doc("weapons.xml") else { return };
    let allowed: Vec<String> = rec.child("weaponmounts").into_iter().flat_map(|w| w.children_named("weaponmount")).map(|w| w.get("allowedweapons")).collect();
    for n in rec.child("weapons").into_iter().flat_map(|w| w.children_named("weapon")) {
        let Some(w) = crate::data::find(&doc, "weapons", "weapon", &n.get("name")) else { continue };
        let size = Some(w.get("sizecategory")).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| w.category());
        let Some(target) = weapon_target(ch, vguid, &w.name(), &size, &allowed) else { continue };
        let p = Purchase { parent: Some(target.clone()), free: true, cost_multiplier: 1.0, ..Default::default() };
        // Weapon support lands separately; until then this returns Err.
        if let Ok(id) = super::add("weapon", ch, store, w, &p) {
            place(ch, &id, &target, "weapons", true);
            if let Some(e) = find_by_guid_mut(&mut ch.doc, &id) {
                e.set_child_text("parentid", vguid);
            }
        }
    }
}

/// Guid of the mount or mod a data weapon goes into (`Vehicle.Create`).
fn weapon_target(ch: &Character, vguid: &str, name: &str, size: &str, allowed: &[String]) -> Option<String> {
    let v = find_by_guid(&ch.doc, vguid)?;
    let kids = |e: &Element, c: &str| e.child(c).map_or(0, |x| x.elements().count());
    let mounts = v.child("weaponmounts").into_iter().flat_map(|w| w.children_named("weaponmount"));
    for (i, m) in mounts.enumerate() {
        let cats = m.get("weaponmountcategories");
        let full = kids(m, "weapons") as i32 >= m.get_i32("weaponcapacity").unwrap_or(1);
        let names = allowed.get(i).map(String::as_str).unwrap_or("");
        if !full && (cats.contains(size) || (!names.is_empty() && names.contains(name)) || cats.is_empty()) {
            return Some(m.get("guid"));
        }
    }
    let mods: Vec<&Element> = v.child("mods").into_iter().flat_map(|w| w.children_named("mod")).collect();
    let fits = |m: &&&Element| {
        let cats = m.get("weaponmountcategories");
        m.get("name").contains("Weapon Mount") || !cats.is_empty() && cats.contains(size)
    };
    mods.iter().filter(fits).find(|m| kids(m, "weapons") == 0).or_else(|| mods.iter().find(fits)).map(|m| m.get("guid"))
}

/// Make sure the item `id` sits in `parent`'s `container`, moving it there
/// if the kind's `add` put it elsewhere. `zero_cost`: data-included items
/// cost nothing (`Cost = "0"`).
fn place(ch: &mut Character, id: &str, parent: &str, container: &str, zero_cost: bool) {
    let inside = find_by_guid(&ch.doc, parent).and_then(|p| p.child(container)).is_some_and(|c| c.elements().any(|e| e.get("guid").eq_ignore_ascii_case(id)));
    if !inside {
        if let Some(item) = take_by_guid(&mut ch.doc, id) {
            if let Some(p) = find_by_guid_mut(&mut ch.doc, parent) {
                p.child_or_insert(container).push(item);
            }
        }
    }
    if let Some(e) = find_by_guid_mut(&mut ch.doc, id) {
        if container == "gears" {
            e.set_child_text("parentid", parent);
        }
        if zero_cost {
            e.set_child_text("cost", "0");
        }
    }
    ch.dirty = true;
}

/// Find a saved item anywhere below `e` by guid.
fn find_by_guid<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_by_guid(c, guid))
}

/// Remove and return the item with this guid from anywhere below `e`.
fn take_by_guid(e: &mut Element, guid: &str) -> Option<Element> {
    let pos = e.children.iter().position(|n| matches!(n, Node::Element(c) if c.get("guid").eq_ignore_ascii_case(guid)));
    if let Some(i) = pos {
        if let Node::Element(c) = e.children.remove(i) {
            return Some(c);
        }
    }
    e.elements_mut().find_map(|c| take_by_guid(c, guid))
}

/// Add a vehicle mod to the vehicle or weapon mount `p.parent`
/// (`SelectVehicleMod` → `VehicleMod.Create`).
fn add_mod(ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    let parent = p.parent.clone().ok_or("a vehicle mod needs a parent vehicle")?;
    let target = find_by_guid(&ch.doc, &parent).ok_or("parent not found")?;
    if target.name != "vehicle" && target.name != "weaponmount" {
        return Err(format!("cannot add a vehicle mod to a {}", target.name));
    }
    let guid = new_guid();
    let el = element("mod", ch, store, rec, p, &guid)?;
    find_by_guid_mut(&mut ch.doc, &parent).ok_or("parent not found")?.child_or_insert("mods").push(el);
    ch.dirty = true;
    let c = vehicle_of(ch, &parent).map_or(0.0, |v| cost_of_item(v, &guid));
    pay(ch, p, c);
    Ok(guid)
}

/// Add a weapon mount to a vehicle (`CreateWeaponMount` form): `size` is
/// the id or name of a "Size" mount, `options` ids or names of
/// flexibility/control/visibility options.
pub fn add_weapon_mount(ch: &mut Character, store: &DataStore, vehicle: &str, size: &str, options: &[&str], p: &Purchase) -> Result<String, String> {
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let rec = find_mount(&doc, size, Some("Size")).ok_or_else(|| format!("no weapon mount {size}"))?;
    if find_by_guid(&ch.doc, vehicle).is_none_or(|v| v.name != "vehicle") {
        return Err("weapon mount parent vehicle not found".into());
    }
    let guid = new_guid();
    let mut m = element("weaponmount", ch, store, rec, p, &guid)?;
    for o in options {
        let r = OPTION_CATEGORIES.iter().find_map(|(_, cat)| find_mount(&doc, o, Some(cat))).ok_or_else(|| format!("no weapon mount option {o}"))?;
        m.child_or_insert("weaponmountoptions").push(mount_option_element(r, false));
    }
    find_by_guid_mut(&mut ch.doc, vehicle).ok_or("vehicle not found")?.child_or_insert("weaponmounts").push(m);
    ch.dirty = true;
    let c = vehicle_of(ch, vehicle).map_or(0.0, |v| cost_of_item(v, &guid));
    pay(ch, p, c);
    Ok(guid)
}

/// The vehicle that is or contains the item `guid`.
fn vehicle_of<'a>(ch: &'a Character, guid: &str) -> Option<&'a Element> {
    ch.items("vehicles", "vehicle").into_iter().find(|v| find_by_guid(v, guid).is_some())
}

/// Cost of a mod or weapon mount on `vehicle`, by guid.
fn cost_of_item(vehicle: &Element, guid: &str) -> f64 {
    let v = Veh::new(vehicle, &VehicleRules::default());
    if let Some(i) = v.mods.iter().position(|m| m.e.get("guid") == guid) {
        return v.mod_total_cost(stats::ModAt::Vehicle(i));
    }
    for (w, m) in v.mounts.iter().enumerate() {
        if m.get("guid") == guid {
            return v.mount_total_cost(w);
        }
        if let Some(i) = stats::mount_mods(m).position(|x| x.get("guid") == guid) {
            return v.mod_total_cost(stats::ModAt::Mount(w, i));
        }
    }
    0.0
}

// -------------------------------------------------------------------
// Oracle
// -------------------------------------------------------------------

/// Oracle: rebuild a saved vehicle, mod or weapon mount from its data
/// record and the choices stored in it (rating, extra, included, a chosen
/// variable cost). Nested items are rebuilt through [`super::rebuild`],
/// or copied while their kind is unsupported.
pub fn rebuild(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    match tag {
        "vehicle" => rebuild_vehicle(ch, store, &doc, saved),
        "mod" => rebuild_mod(ch, store, &doc, saved),
        "weaponmount" => rebuild_mount(ch, store, &doc, saved),
        _ => None,
    }
}

/// The data cost, or the saved one when the data cost was a choice.
fn chosen_cost(rec: Record<'_>, saved: &Element) -> String {
    let c = rec.get("cost");
    if c.starts_with("Variable(") { saved.get("cost") } else { resolve_variable_cost(&c) }
}

/// Rebuild the saved children `container/tag` of `saved` into `out`.
fn rebuild_children(out: &mut Element, container: &str, tag: &str, ch: &Character, store: &DataStore, saved: &Element) {
    let kids: Vec<Element> = saved.child(container).into_iter().flat_map(|x| x.children_named(tag)).map(|s| rebuild_child(tag, ch, store, s)).collect();
    let dst = out.child_or_insert(container);
    for k in kids {
        dst.push(k);
    }
}

/// A nested item rebuilt by its kind, or a copy while unsupported.
fn rebuild_child(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Element {
    super::rebuild(tag, ch, store, saved).unwrap_or_else(|| saved.clone())
}

fn rebuild_vehicle(ch: &Character, store: &DataStore, doc: &Element, saved: &Element) -> Option<Element> {
    let rec = find_rec(doc, "vehicles", "vehicle", &saved_key(saved))?;
    let mut v = vehicle_fields(rec, &saved.get("guid"), &chosen_cost(rec, saved));
    for (c, kid) in [("mods", "mod"), ("weaponmounts", "weaponmount"), ("gears", "gear"), ("weapons", "weapon")] {
        rebuild_children(&mut v, c, kid, ch, store, saved);
    }
    Some(v)
}

fn rebuild_mod(ch: &Character, store: &DataStore, doc: &Element, saved: &Element) -> Option<Element> {
    let rec = find_mod(doc, &saved_key(saved))?;
    let rating = saved.get_i32("rating").unwrap_or(0);
    let mut m = mod_element(rec, rating, &saved.get("extra"), saved.get_bool("included").unwrap_or(false), &saved.get("guid"));
    m.set_child_text("cost", chosen_cost(rec, saved));
    rebuild_children(&mut m, "weapons", "weapon", ch, store, saved);
    if saved.child("cyberwares").is_some() {
        rebuild_children(&mut m, "cyberwares", "cyberware", ch, store, saved);
    }
    Some(m)
}

fn rebuild_mount(ch: &Character, store: &DataStore, doc: &Element, saved: &Element) -> Option<Element> {
    let rec = find_mount(doc, &saved_key(saved), Some("Size"))?;
    let mut m = mount_element(rec, &saved.get("guid"), saved.get_bool("included").unwrap_or(false));
    m.set_child_text("cost", chosen_cost(rec, saved));
    for o in saved.child("weaponmountoptions").into_iter().flat_map(|x| x.children_named("weaponmountoption")) {
        let cat = o.get("category");
        if let Some(r) = find_mount(doc, &saved_key(o), (!cat.is_empty()).then_some(cat.as_str())) {
            m.child_or_insert("weaponmountoptions").push(mount_option_element(r, o.get_bool("includedinparent").unwrap_or(false)));
        }
    }
    rebuild_children(&mut m, "weapons", "weapon", ch, store, saved);
    rebuild_children(&mut m, "mods", "mod", ch, store, saved);
    Some(m)
}
