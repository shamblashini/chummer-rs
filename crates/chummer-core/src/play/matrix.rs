//! Devices with matrix attributes (`IHasMatrixAttributes`: gear,
//! cyberware, armor, weapons and vehicles): their attributes, matrix
//! condition monitor (`<matrixcmfilled>`) and the active commlink
//! (`Character.ActiveCommlink`, saved as `<active>` on the device).

use crate::calc::div_away_from_zero;
use crate::character::Character;
use crate::expr;
use crate::items::vehicle;
use crate::xml::Element;

/// `MatrixAttributes.MatrixAttributeStrings` with their saved elements.
pub const ATTRIBUTES: &[(&str, &str)] = &[
    ("Attack", "attack"),
    ("Sleaze", "sleaze"),
    ("Data Processing", "dataprocessing"),
    ("Firewall", "firewall"),
    ("Device Rating", "devicerating"),
    ("Program Limit", "programlimit"),
];

/// Element names that can carry matrix attributes.
const DEVICE_TAGS: &[&str] = &["gear", "cyberware", "armor", "weapon", "vehicle"];

fn field(name: &str) -> &'static str {
    let name = name.strip_prefix("Mod ").unwrap_or(name);
    ATTRIBUTES.iter().find(|(n, _)| *n == name).map_or("", |(_, f)| f)
}

/// `GetMatrixAttributeString`: the saved expression (`mod…` for `Mod X`).
/// `Mod Device Rating` and `Mod Program Limit` have none.
fn attribute_string(e: &Element, name: &str) -> String {
    let f = field(name);
    if f.is_empty() {
        return String::new();
    }
    if name.starts_with("Mod ") {
        if matches!(f, "devicerating" | "programlimit") {
            return String::new();
        }
        return e.get(&format!("mod{f}"));
    }
    e.get(f)
}

/// Equipped gear inside a device (`Children` of gear, `GearChildren` of
/// the others).
fn gear_children(e: &Element) -> Vec<&Element> {
    let c = if e.name == "gear" { "children" } else { "gears" };
    e.child(c).map(|c| c.children_named("gear").filter(|g| g.get_bool("equipped").unwrap_or(true)).collect()).unwrap_or_default()
}

fn all_gear_children(e: &Element) -> impl Iterator<Item = &Element> {
    let c = if e.name == "gear" { "children" } else { "gears" };
    e.child(c).into_iter().flat_map(|c| c.children_named("gear"))
}

fn rating(e: &Element) -> i32 {
    e.get_i32("rating").unwrap_or(0)
}

/// `ProcessRatingString` after `ProcessFixedValuesString`.
fn eval(s: &str, rating: i32) -> i32 {
    expr::value_to_int(s, rating, &expr::NoAttributes)
}

/// `Cyberware.Grade.DeviceRating` (the grades in cyberware.xml and
/// bioware.xml).
// LIKELY-BUG(LB-31): hard-coded by grade name; Chummer reads the grade's <devicerating> (bioware grades: 0) and uses this table only as a fallback. See docs/likely-bugs.md.
fn grade_device_rating(grade: &str) -> i32 {
    for (prefix, dr) in [("Alphaware", 3), ("Betaware", 4), ("Deltaware", 5), ("Gammaware", 6)] {
        if grade.starts_with(prefix) {
            return dr;
        }
    }
    2
}

/// `IsCommlink`: the device can form a persona (`canformpersona` has
/// `Self`, or a child grants `Parent`).
pub fn is_commlink(e: &Element) -> bool {
    let child_grants = all_gear_children(e).any(|g| g.get("canformpersona").contains("Parent"));
    match e.name.as_str() {
        "gear" | "armor" => e.get("canformpersona").contains("Self") || child_grants,
        "cyberware" => e.get("canformpersona").contains("Self") || (child_grants && total(e, "Device Rating") > 0),
        "vehicle" => child_grants && total(e, "Device Rating") > 0,
        _ => false,
    }
}

/// `GetBaseMatrixAttribute`.
pub fn base(e: &Element, name: &str) -> i32 {
    if e.name == "vehicle" && !name.starts_with("Mod ") {
        let dr = vehicle::stats(e).device_rating - i32::from(e.get("overclocked") == "Device Rating");
        return match name {
            "Device Rating" => dr,
            "Data Processing" | "Firewall" => match e.get(field(name)) {
                s if s.trim().is_empty() => dr,
                s => eval(&s, rating(e)),
            },
            _ => eval(&e.get(field(name)), rating(e)),
        };
    }
    let mut s = attribute_string(e, name);
    if s.trim().is_empty() {
        let dr_string = attribute_string(e, "Device Rating");
        if e.name == "cyberware" {
            let grade = grade_device_rating(&e.get("grade"));
            match name {
                "Device Rating" => return grade,
                "Program Limit" | "Data Processing" | "Firewall" if dr_string.trim().is_empty() => return grade,
                "Program Limit" | "Data Processing" | "Firewall" => s = dr_string,
                _ => return 0,
            }
        } else {
            s = match name {
                "Device Rating" => if is_commlink(e) { "2".into() } else { "0".into() },
                "Program Limit" if is_commlink(e) => if dr_string.trim().is_empty() { "2".into() } else { dr_string },
                "Data Processing" | "Firewall" if !dr_string.trim().is_empty() => dr_string,
                _ => "0".into(),
            };
        }
    }
    eval(&s, rating(e))
}

/// `GetBonusMatrixAttribute`: Overclocker plus the `Mod X` of equipped
/// gear inside (and nested ware for cyberware).
pub fn bonus(e: &Element, name: &str) -> i32 {
    let mut v = i32::from(!name.is_empty() && e.get("overclocked") == name);
    if e.name == "vehicle" {
        // Vehicle mods are already in `vehicle::stats().device_rating`.
        return v;
    }
    let m = if name.starts_with("Mod ") { name.to_owned() } else { format!("Mod {name}") };
    v += gear_children(e).iter().map(|g| total(g, &m)).sum::<i32>();
    if e.name == "cyberware" {
        let kids = e.child("children").into_iter().flat_map(|c| c.children_named("cyberware"));
        v += kids.map(|c| total(c, &m)).sum::<i32>();
    }
    v
}

/// `GetTotalMatrixAttribute`.
pub fn total(e: &Element, name: &str) -> i32 {
    base(e, name) + bonus(e, name)
}

/// `TotalBonusMatrixBoxes`: `matrixcmbonus` of the device and of equipped
/// gear inside it (and vehicle mods, with their wireless bonus).
pub fn bonus_boxes(e: &Element) -> i32 {
    let mut v = if e.name == "vehicle" { 0 } else { e.get_i32("matrixcmbonus").unwrap_or(0) };
    v += gear_children(e).iter().map(|g| bonus_boxes(g)).sum::<i32>();
    if e.name == "vehicle" {
        for m in e.child("mods").into_iter().flat_map(|c| c.children_named("mod")) {
            let get = |b: &str| m.child(b).and_then(|b| b.get_i32("matrixcmbonus")).unwrap_or(0);
            v += get("bonus");
            if m.get_bool("wirelesson").unwrap_or(false) {
                v += get("wirelessbonus");
            }
        }
    }
    v
}

/// `MatrixCM`: 8 + ⌈DR / 2⌉ + bonus boxes.
pub fn condition_monitor(e: &Element) -> i32 {
    8 + div_away_from_zero(total(e, "Device Rating"), 2) + bonus_boxes(e)
}

/// `MatrixCMFilled`.
pub fn filled(e: &Element) -> i32 {
    e.get_i32("matrixcmfilled").unwrap_or(0)
}

/// Whether the editor should offer a matrix condition monitor: the
/// device has a device rating or can be a commlink.
pub fn has_matrix(e: &Element) -> bool {
    DEVICE_TAGS.contains(&e.name.as_str()) && (e.child("matrixcmfilled").is_some() || e.name == "vehicle") && (total(e, "Device Rating") > 0 || is_commlink(e))
}

/// Set a device's matrix damage (clicking its condition monitor boxes).
pub fn set_filled(ch: &mut Character, guid: &str, value: i32) -> bool {
    let Some(max) = super::find(&ch.doc, guid).map(condition_monitor) else { return false };
    super::set_filled(ch, guid, "matrixcmfilled", value, max)
}

/// Every device that can be the active commlink, in document order.
pub fn commlinks(ch: &Character) -> Vec<&Element> {
    fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
        for c in e.elements().filter(|c| c.name != "improvements") {
            if DEVICE_TAGS.contains(&c.name.as_str()) && !c.get("guid").is_empty() && c.child("active").is_some() && is_commlink(c) {
                out.push(c);
            }
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(&ch.doc, &mut out);
    out
}

/// `Character.ActiveCommlink`: the device saved with `<active>True`
/// (the last one, as each load overrides the one before).
pub fn active_commlink(ch: &Character) -> Option<&Element> {
    fn walk<'a>(e: &'a Element, out: &mut Option<&'a Element>) {
        for c in e.elements().filter(|c| c.name != "improvements") {
            if DEVICE_TAGS.contains(&c.name.as_str()) && c.get_bool("active").unwrap_or(false) {
                *out = Some(c);
            }
            walk(c, out);
        }
    }
    let mut out = None;
    walk(&ch.doc, &mut out);
    out
}

/// Data Processing of the active commlink, for matrix initiative.
pub fn active_commlink_dp(ch: &Character) -> i32 {
    active_commlink(ch).map_or(0, |c| total(c, "Data Processing"))
}

/// `SetActiveCommlink`: make `guid` the active commlink (only a device
/// that `is_commlink`), or clear it. Only one device is active at a time.
pub fn set_active(ch: &mut Character, guid: &str, on: bool) -> bool {
    let Some(e) = super::find(&ch.doc, guid) else { return false };
    if on && !is_commlink(e) {
        return false;
    }
    let was = e.get_bool("active").unwrap_or(false);
    if was == on {
        return false;
    }
    fn clear(e: &mut Element) {
        for c in e.elements_mut().filter(|c| c.name != "improvements") {
            if DEVICE_TAGS.contains(&c.name.as_str()) && c.get_bool("active").unwrap_or(false) {
                c.set_child_text("active", "False");
            }
            clear(c);
        }
    }
    if on {
        clear(&mut ch.doc);
    }
    if let Some(e) = super::find_mut(&mut ch.doc, guid) {
        e.set_child_text("active", crate::improvement::bool_str(on));
    }
    ch.dirty = true;
    true
}


/// `Character.HomeNode`: the device saved with `<homenode>True` (the last
/// one, as each load overrides the one before). Any character can have
/// one saved; only an A.I.'s home node changes its values.
pub fn home_node(ch: &Character) -> Option<&Element> {
    fn walk<'a>(e: &'a Element, out: &mut Option<&'a Element>) {
        for c in e.elements().filter(|c| c.name != "improvements") {
            if DEVICE_TAGS.contains(&c.name.as_str()) && c.get_bool("homenode").unwrap_or(false) {
                *out = Some(c);
            }
            walk(c, out);
        }
    }
    let mut out = None;
    walk(&ch.doc, &mut out);
    out
}

/// The A.I.'s home node when it is a vehicle or drone.
pub fn home_node_vehicle(ch: &Character) -> Option<&Element> {
    home_node(ch).filter(|e| e.name == "vehicle")
}

/// Whether the "Home Node" checkbox is enabled for a device
/// (`chkGearHomeNode` in `CharacterCareer.cs`): it must be a commlink
/// with a Program Limit of at least 1, or 2 when the A.I.'s Depth is
/// above the device's rating.
pub fn can_be_home_node(e: &Element, depth: i32) -> bool {
    let dr = total(e, "Device Rating");
    is_commlink(e) && total(e, "Program Limit") >= if depth > dr { 2 } else { 1 }
}

/// `SetHomeNode`: make `guid` the home node, or clear it. Only one device
/// is the home node at a time.
pub fn set_home_node(ch: &mut Character, guid: &str, on: bool) -> bool {
    let Some(e) = super::find(&ch.doc, guid) else { return false };
    if !DEVICE_TAGS.contains(&e.name.as_str()) || e.get_bool("homenode").unwrap_or(false) == on {
        return false;
    }
    fn clear(e: &mut Element) {
        for c in e.elements_mut().filter(|c| c.name != "improvements") {
            if DEVICE_TAGS.contains(&c.name.as_str()) && c.get_bool("homenode").unwrap_or(false) {
                c.set_child_text("homenode", "False");
            }
            clear(c);
        }
    }
    if on {
        clear(&mut ch.doc);
    }
    if let Some(e) = super::find_mut(&mut ch.doc, guid) {
        e.set_child_text("homenode", crate::improvement::bool_str(on));
    }
    ch.dirty = true;
    true
}
