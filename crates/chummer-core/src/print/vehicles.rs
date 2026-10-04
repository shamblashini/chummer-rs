//! `<vehicles>` (`Vehicle.Print`, `VehicleMod.Print`, `WeaponMount.Print`).
//!
//! Handling, speed, body and the other ratings are the totals after mods
//! (`items::vehicle::stats_with`); costs come from `items::vehicle`.

use super::items::{self, GearParent, WeaponPlace};
use super::{add, bool_text, copy, copy_bool, full_name, own_avail, total_avail, Ctx};
use crate::expr::Availability;
use crate::items::vehicle as vcalc;
use crate::xml::Element;

const FILE: &str = "vehicles.xml";

fn children<'a>(e: &'a Element, container: &str, item: &'a str) -> Vec<&'a Element> {
    e.child(container).map(|c| c.children_named(item).collect()).unwrap_or_default()
}

fn included(e: &Element) -> bool {
    e.get_bool("included").unwrap_or(false)
}

/// `guid`, `sourceid`, names, category.
fn head(ctx: &Ctx, out: &mut Element, item: &Element, custom: &str) {
    copy(out, item, "guid");
    add(out, "sourceid", [item.get("sourceid"), item.get("id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default());
    let name = ctx.tr_name(FILE, item);
    add(out, "name", name.clone());
    add(out, "name_english", item.get("name"));
    let rating = item.get_i32("rating").unwrap_or(0);
    add(out, "fullname", full_name(ctx, &name, None, rating, &item.get("extra"), custom));
    add(out, "fullname_english", full_name(ctx, &item.get("name"), None, rating, &item.get("extra"), custom));
    let cat = item.get("category");
    add(out, "category", ctx.tr_category(FILE, &cat));
    add(out, "category_english", cat);
}

/// `VehicleMod.TotalAvailTuple`: own plus its weapons.
fn mod_avail(ctx: &Ctx, m: &Element) -> Availability {
    let kids: Vec<Availability> = children(m, "weapons", "weapon").into_iter().map(own_avail).collect();
    total_avail(ctx, m, own_avail(m), &kids)
}

/// `WeaponMount.TotalAvailTuple`: own plus its weapons and mods.
fn mount_avail(ctx: &Ctx, m: &Element) -> Availability {
    let mut kids: Vec<Availability> = children(m, "weapons", "weapon").into_iter().map(own_avail).collect();
    kids.extend(children(m, "mods", "mod").into_iter().filter(|x| !included(x)).map(|x| mod_avail(ctx, x)));
    total_avail(ctx, m, own_avail(m), &kids)
}

/// `Vehicle.TotalAvailTuple`: own plus mods, weapon mounts and gear.
fn vehicle_avail(ctx: &Ctx, v: &Element) -> Availability {
    let mut kids: Vec<Availability> = children(v, "mods", "mod").into_iter().filter(|m| !included(m)).map(|m| mod_avail(ctx, m)).collect();
    kids.extend(children(v, "weaponmounts", "weaponmount").into_iter().filter(|m| !included(m)).map(|m| mount_avail(ctx, m)));
    kids.extend(children(v, "gears", "gear").into_iter().map(|g| items::gear_avail(ctx, g)));
    total_avail(ctx, v, own_avail(v), &kids)
}

/// `Vehicle.PhysicalCM`: base boxes (8 Anthro drones, 6 drones, 12) plus
/// half the total Body, plus the mods' `conditionmonitor`.
pub fn physical_cm(v: &Element, st: &vcalc::VehicleStats) -> i32 {
    let base = if !st.is_drone { 12 } else if v.get("category") == "Drones: Anthro" { 8 } else { 6 };
    let mods: i32 = children(v, "mods", "mod").into_iter().map(|m| m.get_i32("conditionmonitor").unwrap_or(0)).sum();
    base + crate::calc::div_away_from_zero(st.body, 2) + mods
}

/// `Vehicle.Print`.
pub fn vehicle(ctx: &Ctx, v: &Element) -> Element {
    let mut out = Element::new("vehicle");
    head(ctx, &mut out, v, &v.get("vehiclename"));
    let st = vcalc::stats_with(v, &ctx.vehicle_rules);
    add(&mut out, "isdrone", bool_text(st.is_drone));
    add(&mut out, "handling", st.handling_text.clone());
    add(&mut out, "accel", st.accel_text.clone());
    add(&mut out, "speed", st.speed_text.clone());
    add(&mut out, "pilot", st.pilot.to_string());
    add(&mut out, "body", st.body.to_string());
    add(&mut out, "armor", st.armor.to_string());
    add(&mut out, "seats", st.seats.to_string());
    add(&mut out, "sensor", st.sensor.to_string());
    ctx.add_avail(&mut out, vehicle_avail(ctx, v), false);
    add(&mut out, "cost", ctx.nuyen(vcalc::cost(v)));
    add(&mut out, "owncost", ctx.nuyen(vcalc::own_cost(v)));
    copy(&mut out, v, "source");
    copy(&mut out, v, "page");
    add(&mut out, "physicalcm", physical_cm(v, &st).to_string());
    add(&mut out, "physicalcmfilled", v.get_i32("physicalcmfilled").unwrap_or(0).to_string());
    copy(&mut out, v, "vehiclename");
    add(&mut out, "maneuver", v.get_i32("maneuver").unwrap_or(0).to_string());
    add(&mut out, "location", ctx.location(&v.get("location")));
    matrix(&mut out, v, st.device_rating);
    let costs = vcalc::part_costs(v);
    let mut list = Element::new("mods");
    for (i, m) in children(v, "mods", "mod").into_iter().enumerate() {
        list.push(vehicle_mod(ctx, m, v, None, costs.mods.get(i).copied().unwrap_or_default()));
    }
    for (w, m) in children(v, "weaponmounts", "weaponmount").into_iter().enumerate() {
        let own_total = costs.mounts.get(w).copied().unwrap_or_default();
        list.push(weapon_mount(ctx, m, v, own_total, costs.mount_mods.get(w).map(Vec::as_slice).unwrap_or_default()));
    }
    out.push(list);
    out.push(items::gear_list(ctx, &children(v, "gears", "gear"), GearParent::Other(v)));
    out.push(weapon_list(ctx, &children(v, "weapons", "weapon"), WeaponPlace { vehicle: Some(v), mount: None }));
    ctx.notes(&mut out, v);
    out
}

/// Vehicle matrix attributes: device rating drives everything.
fn matrix(out: &mut Element, v: &Element, dr: i32) {
    for f in ["attack", "sleaze"] {
        add(out, f, v.get_i32(f).unwrap_or(0).to_string());
    }
    for f in ["dataprocessing", "firewall"] {
        add(out, f, v.get_i32(f).filter(|x| *x > 0).unwrap_or(dr).to_string());
    }
    add(out, "devicerating", dr.to_string());
    add(out, "programlimit", v.get_i32("programlimit").unwrap_or(0).to_string());
    add(out, "iscommlink", bool_text(false));
    add(out, "isprogram", bool_text(false));
    copy_bool(out, v, "active");
    copy_bool(out, v, "homenode");
    add(out, "matrixcm", (8 + (dr + 1) / 2).to_string());
    add(out, "matrixcmfilled", v.get_i32("matrixcmfilled").unwrap_or(0).to_string());
}

fn weapon_list(ctx: &Ctx, weapons: &[&Element], place: WeaponPlace) -> Element {
    let mut out = Element::new("weapons");
    for w in weapons {
        out.push(items::weapon(ctx, w, place));
    }
    out
}

/// `VehicleMod.Print`.
fn vehicle_mod(ctx: &Ctx, m: &Element, v: &Element, mount: Option<&Element>, (own, total): (f64, f64)) -> Element {
    let mut out = Element::new("mod");
    head(ctx, &mut out, m, "");
    copy(&mut out, m, "limit");
    copy(&mut out, m, "slots");
    add(&mut out, "rating", m.get_i32("rating").unwrap_or(0).to_string());
    copy(&mut out, m, "ratinglabel");
    ctx.add_avail(&mut out, mod_avail(ctx, m), false);
    add(&mut out, "cost", ctx.nuyen(total));
    add(&mut out, "owncost", ctx.nuyen(own));
    copy(&mut out, m, "source");
    copy_bool(&mut out, m, "wirelesson");
    copy(&mut out, m, "page");
    copy_bool(&mut out, m, "included");
    out.push(weapon_list(ctx, &children(m, "weapons", "weapon"), WeaponPlace { vehicle: Some(v), mount }));
    let mut ware = Element::new("cyberwares");
    for w in children(m, "cyberwares", "cyberware") {
        ware.push(items::cyberware(ctx, w));
    }
    out.push(ware);
    ctx.notes(&mut out, m);
    out
}

/// `WeaponMount.Print` (also printed as `<mod>`).
fn weapon_mount(ctx: &Ctx, m: &Element, v: &Element, (own, total): (f64, f64), mod_costs: &[(f64, f64)]) -> Element {
    let mut out = Element::new("mod");
    copy(&mut out, m, "guid");
    add(&mut out, "sourceid", m.get("sourceid"));
    copy(&mut out, m, "source");
    let mut named = Element::new("x");
    head(ctx, &mut named, m, "");
    for n in named.children.drain(..).skip(2) {
        out.children.push(n);
    }
    copy(&mut out, m, "limit");
    add(&mut out, "slots", m.get_i32("slots").unwrap_or(0).to_string());
    ctx.add_avail(&mut out, mount_avail(ctx, m), false);
    add(&mut out, "cost", ctx.nuyen(total));
    add(&mut out, "owncost", ctx.nuyen(own));
    copy(&mut out, m, "page");
    copy(&mut out, m, "location");
    copy_bool(&mut out, m, "included");
    out.push(weapon_list(ctx, &children(m, "weapons", "weapon"), WeaponPlace { vehicle: Some(v), mount: Some(m) }));
    let mut mods = Element::new("mods");
    for (i, x) in children(m, "mods", "mod").into_iter().enumerate() {
        mods.push(vehicle_mod(ctx, x, v, Some(m), mod_costs.get(i).copied().unwrap_or_default()));
    }
    out.push(mods);
    ctx.notes(&mut out, m);
    out
}
