//! Vehicle and drone damage: `Vehicle.PhysicalCM` / `<physicalcmfilled>`.
//! The matrix track of a vehicle is in [`super::matrix`].

use crate::calc::div_away_from_zero;
use crate::character::Character;
use crate::items::vehicle::{self, VehicleRules};
use crate::xml::Element;

/// `Vehicle.BasePhysicalBoxes`: 12 for vehicles, 6 for drones, 8 for
/// anthro drones (Rigger 5.0 p. 145).
pub fn base_boxes(v: &Element) -> i32 {
    if vehicle::is_drone(v) {
        if v.get("category") == "Drones: Anthro" { 8 } else { 6 }
    } else {
        12
    }
}

/// `Vehicle.PhysicalCM`: base + ⌈Body / 2⌉ + the mods' `conditionmonitor`.
pub fn condition_monitor(v: &Element, rules: &VehicleRules) -> i32 {
    let body = vehicle::stats_with(v, rules).body;
    let mods: i32 = v.child("mods").into_iter().flat_map(|m| m.children_named("mod")).map(|m| m.get_i32("conditionmonitor").unwrap_or(0)).sum();
    base_boxes(v) + div_away_from_zero(body, 2) + mods
}

/// `Vehicle.PhysicalCMFilled`.
pub fn filled(v: &Element) -> i32 {
    v.get_i32("physicalcmfilled").unwrap_or(0)
}

/// Set a vehicle's physical damage.
pub fn set_filled(ch: &mut Character, guid: &str, value: i32, rules: &VehicleRules) -> bool {
    let Some(max) = super::find(&ch.doc, guid).filter(|v| v.name == "vehicle").map(|v| condition_monitor(v, rules)) else { return false };
    super::set_filled(ch, guid, "physicalcmfilled", value, max)
}
