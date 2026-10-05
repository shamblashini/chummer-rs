//! A.I.s and their home node (`Character.IsAI`, `Character.HomeNode`).
//!
//! An A.I. has no body: its Physical track is a Core track (8 + ⌈Depth / 2⌉)
//! unless it lives in a vehicle or drone, whose damage track it then uses,
//! and its Stun track is the home node's Matrix track. Chummer redirects
//! `PhysicalCMFilled` and `StunCMFilled` to the home node but saves the
//! character's own values, so the setters here write to the device.

use super::matrix;
use crate::character::Character;
use crate::items::vehicle;
use crate::xml::Element;

/// Values the home node gives the A.I.'s derived stats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HomeNode {
    /// `GetTotalMatrixAttribute("Data Processing")`, or the vehicle's
    /// Pilot when higher (initiative and Social limit).
    pub dp_or_pilot: i32,
    pub data_processing: i32,
    /// `MatrixCM` of the device.
    pub matrix_cm: i32,
    /// Vehicle values (`TotalBody`, `Handling`, `CalculatedSensor`,
    /// `PhysicalCM`), when the home node is a vehicle or drone.
    pub vehicle: Option<HomeVehicle>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HomeVehicle {
    pub body: i32,
    pub handling: i32,
    pub sensor: i32,
    pub physical_cm: i32,
}

/// The A.I.'s home node values, or `None` when the character is no A.I.
/// or has no home node.
pub fn home_node(ch: &Character) -> Option<HomeNode> {
    if !ch.is_ai() {
        return None;
    }
    matrix::home_node(ch).map(node_values)
}

fn node_values(e: &Element) -> HomeNode {
    let dp = matrix::total(e, "Data Processing");
    let vehicle = (e.name == "vehicle").then(|| {
        let s = vehicle::stats(e);
        (s.pilot, HomeVehicle { body: s.body, handling: e.get_i32("handling").unwrap_or(0), sensor: s.sensor, physical_cm: super::vehicle::condition_monitor(e, &Default::default()) })
    });
    HomeNode {
        dp_or_pilot: vehicle.map_or(dp, |(pilot, _)| dp.max(pilot)),
        data_processing: dp,
        matrix_cm: matrix::condition_monitor(e),
        vehicle: vehicle.map(|(_, v)| v),
    }
}

/// `Character.PhysicalCMFilled`: a vehicle home node's damage.
pub fn physical_filled(ch: &Character) -> i32 {
    match matrix::home_node_vehicle(ch) {
        Some(v) => super::vehicle::filled(v),
        None => ch.physical_cm_filled,
    }
}

/// `Character.StunCMFilled`: an A.I.'s home node Matrix damage.
pub fn stun_filled(ch: &Character) -> i32 {
    match matrix::home_node(ch).filter(|_| ch.is_ai()) {
        Some(e) => matrix::filled(e),
        None => ch.stun_cm_filled,
    }
}

/// Set the Physical damage shown on the character's condition monitor.
pub fn set_physical_filled(ch: &mut Character, value: i32) -> bool {
    if let Some(guid) = matrix::home_node_vehicle(ch).map(|v| v.get("guid")) {
        return super::vehicle::set_filled(ch, &guid, value, &Default::default());
    }
    let changed = ch.physical_cm_filled != value;
    ch.physical_cm_filled = value;
    changed
}

/// Set the Stun damage shown on the character's condition monitor.
pub fn set_stun_filled(ch: &mut Character, value: i32) -> bool {
    if let Some(guid) = matrix::home_node(ch).filter(|_| ch.is_ai()).map(|e| e.get("guid")) {
        return matrix::set_filled(ch, &guid, value);
    }
    let changed = ch.stun_cm_filled != value;
    ch.stun_cm_filled = value;
    changed
}
