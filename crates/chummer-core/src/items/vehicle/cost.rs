//! Vehicle costs: `Vehicle.TotalCost`, `VehicleMod.TotalCost`,
//! `WeaponMount.TotalCost` and `WeaponMountOption.TotalCost`.

use crate::character::Character;
use crate::data::DataStore;
use crate::expr::{evaluate_num, needs_evaluation, parse_plain};
use crate::items::cyberware::{self, VehicleAttributes};
use crate::items::weapon;
use crate::xml::Element;

use super::stats::{mount_mods, ModAt, Veh, VehicleRules};

/// Total cost of a saved vehicle: its own cost plus mods, weapon mounts
/// (with their weapons and mods) and gear (`Vehicle.TotalCost`). Weapons
/// directly under the vehicle are not counted, as in Chummer: they sit in a
/// mount or a mod, whose cost includes them.
pub fn cost(vehicle: &Element) -> f64 {
    total(&Veh::new(vehicle, &VehicleRules::default()))
}

/// [`cost`] with the owner known, so ware in drone arms and legs is priced
/// with its grade and the vehicle's Body and Pilot as its attribute limits.
pub fn cost_with(ch: &Character, store: &DataStore, vehicle: &Element) -> f64 {
    let mut v = Veh::new(vehicle, &VehicleRules::default());
    v.pricing = Some((ch, store));
    total(&v)
}

/// `Vehicle.TotalCost`.
fn total(v: &Veh<'_>) -> f64 {
    let vehicle = v.e;
    let mods: f64 = (0..v.mods.len()).map(|i| v.mod_total_cost(ModAt::Vehicle(i))).sum();
    let mounts: f64 = (0..v.mounts.len()).map(|w| v.mount_total_cost(w)).sum();
    let gear: f64 = children(vehicle, "gears").map(super::super::gear::cost).sum();
    v.own_cost() + mods + mounts + gear
}

/// Cost of the vehicle itself, without anything on it (`Vehicle.OwnCost`).
pub fn own_cost(vehicle: &Element) -> f64 {
    Veh::new(vehicle, &VehicleRules::default()).own_cost()
}


fn children<'a>(e: &'a Element, container: &str) -> impl Iterator<Item = &'a Element> {
    e.child(container).into_iter().flat_map(|c| c.elements())
}

/// A decimal as Chummer writes it into an expression.
pub(super) fn fmt_dec(v: f64) -> String {
    crate::improvement::fmt_num(v)
}

fn flag(e: &Element, k: &str) -> bool {
    e.get_bool(k).unwrap_or(false)
}

/// The 10% discounts: black market (`discountedcost`) and, for vehicles,
/// Dealer Connection (`dealerconnection`).
fn discounted(e: &Element, v: f64) -> f64 {
    let mut v = v;
    if flag(e, "discountedcost") {
        v *= 0.9;
    }
    if flag(e, "dealerconnection") {
        v *= 0.9;
    }
    v
}

impl Veh<'_> {
    /// `Vehicle.OwnCost`.
    pub fn own_cost(&self) -> f64 {
        let s = self.e.get("cost");
        let c = if needs_evaluation(&s) {
            evaluate_num(&self.process_attrs(&s, None, None)).unwrap_or(0.0)
        } else {
            parse_plain(&s).unwrap_or(0.0)
        };
        discounted(self.e, c)
    }

    fn mod_at(&self, at: ModAt) -> Option<&Element> {
        match at {
            ModAt::Vehicle(i) => self.mods.get(i).map(|m| m.e),
            ModAt::Mount(w, i) => mount_mods(self.mounts.get(w)?).nth(i),
        }
    }

    /// `VehicleMod.OwnCost`.
    pub fn mod_own_cost(&self, at: ModAt) -> f64 {
        let Some(m) = self.mod_at(at) else { return 0.0 };
        let s = m.get("cost");
        if s.trim().is_empty() {
            return 0.0;
        }
        let c = self.mod_value(at, &s);
        if flag(m, "discountedcost") { c * 0.9 } else { c }
    }

    /// `VehicleMod.TotalCost`: own cost unless included, plus weapons and
    /// cyberware on the mod.
    pub fn mod_total_cost(&self, at: ModAt) -> f64 {
        let Some(m) = self.mod_at(at) else { return 0.0 };
        let own = if flag(m, "included") { 0.0 } else { self.mod_own_cost(at) };
        own + children(m, "weapons").map(weapon::cost).sum::<f64>() + children(m, "cyberwares").map(|c| self.ware_cost(c)).sum::<f64>()
    }

    /// `Cyberware.TotalCost` of ware in a mod. Without the owner, the saved
    /// cost expression at the saved rating stands in.
    fn ware_cost(&self, c: &Element) -> f64 {
        match self.pricing {
            Some((ch, store)) => {
                let limits = VehicleAttributes { body: self.total_body(None), pilot: self.pilot(None), max_pilot: self.max_pilot() };
                cyberware::cost_in_vehicle(ch, store, c, limits)
            }
            None => crate::chargen::item_cost(c),
        }
    }

    /// `WeaponMount.OwnCost`.
    pub fn mount_own_cost(&self, w: usize) -> f64 {
        let Some(m) = self.mounts.get(w) else { return 0.0 };
        if flag(m, "freecost") {
            return 0.0;
        }
        let raw = m.get("cost");
        let s = raw.trim_start_matches('+');
        let c = if needs_evaluation(s) {
            evaluate_num(&self.process_attrs(s, None, Some(w))).unwrap_or(0.0)
        } else {
            parse_plain(s).unwrap_or(0.0)
        };
        if flag(m, "discountedcost") { c * 0.9 } else { c }
    }

    /// `WeaponMountOption.Cost`.
    fn option_cost(&self, w: usize, o: &Element) -> f64 {
        if flag(o, "includedinparent") {
            return 0.0;
        }
        let raw = o.get("cost");
        let s = raw.trim_start_matches('+');
        if !needs_evaluation(s) {
            return parse_plain(s).unwrap_or(0.0);
        }
        let mut s = s.to_owned();
        if s.contains("Parent Cost") {
            let c = fmt_dec(self.mount_own_cost(w));
            s = s.replace("{Parent Cost}", &c).replace("Parent Cost", &c);
        }
        if s.contains("Parent Slots") {
            let c = self.mount_slots(w).to_string();
            s = s.replace("{Parent Slots}", &c).replace("Parent Slots", &c);
        }
        evaluate_num(&self.process_attrs(&s, None, Some(w))).unwrap_or(0.0)
    }

    /// `WeaponMount.TotalCost`.
    pub fn mount_total_cost(&self, w: usize) -> f64 {
        let Some(m) = self.mounts.get(w) else { return 0.0 };
        let weapons: f64 = children(m, "weapons").map(weapon::cost).sum();
        let mods: f64 = (0..mount_mods(m).count()).map(|i| self.mod_total_cost(ModAt::Mount(w, i))).sum();
        if flag(m, "included") || flag(m, "freecost") {
            return weapons + mods;
        }
        let mut options: f64 = children(m, "weaponmountoptions").map(|o| self.option_cost(w, o)).sum();
        if flag(m, "discountedcost") {
            options *= 0.9;
        }
        self.mount_own_cost(w) + options + weapons + mods
    }
}
