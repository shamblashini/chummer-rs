//! `<vehicles>` (`Vehicle.Print`, `VehicleMod.Print`, `WeaponMount.Print`).
//!
//! Vehicle mod bonuses are not ported yet, so handling, speed, body and
//! the other ratings are the saved base values.

use super::{add, avail, bool_text, copy, copy_bool, eval, full_name, items, Ctx};
use crate::xml::Element;

const FILE: &str = "vehicles.xml";

fn children<'a>(e: &'a Element, container: &str, item: &'a str) -> Vec<&'a Element> {
    e.child(container).map(|c| c.children_named(item).collect()).unwrap_or_default()
}

fn own_cost(item: &Element) -> f64 {
    eval(&item.get("cost"), item.get_i32("rating").unwrap_or(0))
}

/// `"N"` or `"N/M"` when the off-road value differs.
fn on_off(item: &Element, on: &str, off: &str) -> String {
    let a = item.get(on);
    let b = item.get(off);
    if b.is_empty() || b == a { a } else { format!("{a}/{b}") }
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

/// `Vehicle.Print`.
pub fn vehicle(ctx: &Ctx, v: &Element) -> Element {
    let mut out = Element::new("vehicle");
    head(ctx, &mut out, v, &v.get("vehiclename"));
    let cat = v.get("category");
    add(&mut out, "isdrone", bool_text(cat.contains("Drone")));
    add(&mut out, "handling", on_off(v, "handling", "offroadhandling"));
    add(&mut out, "accel", on_off(v, "accel", "offroadaccel"));
    add(&mut out, "speed", on_off(v, "speed", "offroadspeed"));
    for f in ["pilot", "body", "armor", "seats", "sensor"] {
        add(&mut out, f, v.get_i32(f).unwrap_or(0).to_string());
    }
    add(&mut out, "avail", avail(v));
    let mods = children(v, "mods", "mod");
    let mounts = children(v, "weaponmounts", "weaponmount");
    let gears = children(v, "gears", "gear");
    let weapons = children(v, "weapons", "weapon");
    let extras: f64 = mods.iter().chain(&mounts).chain(&gears).chain(&weapons).filter(|m| !m.get_bool("included").unwrap_or(false)).map(|m| own_cost(m)).sum();
    let own = own_cost(v);
    add(&mut out, "cost", ctx.nuyen(own + extras));
    add(&mut out, "owncost", ctx.nuyen(own));
    copy(&mut out, v, "source");
    copy(&mut out, v, "page");
    let body = v.get_i32("body").unwrap_or(0);
    let base = if cat == "Drones: Anthro" { 8 } else if cat.contains("Drone") { 6 } else { 12 };
    add(&mut out, "physicalcm", (base + (body + 1) / 2).to_string());
    add(&mut out, "physicalcmfilled", v.get_i32("physicalcmfilled").unwrap_or(0).to_string());
    copy(&mut out, v, "vehiclename");
    add(&mut out, "maneuver", v.get_i32("maneuver").unwrap_or(0).to_string());
    add(&mut out, "location", ctx.location(&v.get("location")));
    matrix(&mut out, v);
    let mut list = Element::new("mods");
    for m in mods {
        list.push(vehicle_mod(ctx, m));
    }
    for m in mounts {
        list.push(weapon_mount(ctx, m));
    }
    out.push(list);
    out.push(items::gear_list(ctx, &gears));
    out.push(weapon_list(ctx, &weapons));
    ctx.notes(&mut out, v);
    out
}

/// Vehicle matrix attributes: device rating drives everything.
fn matrix(out: &mut Element, v: &Element) {
    let dr = v.get_i32("devicerating").unwrap_or(0);
    for f in ["attack", "sleaze"] {
        add(out, f, v.get_i32(f).unwrap_or(0).to_string());
    }
    for f in ["dataprocessing", "firewall", "devicerating"] {
        add(out, f, v.get_i32(f).filter(|x| *x > 0).unwrap_or(dr).to_string());
    }
    add(out, "programlimit", v.get_i32("programlimit").unwrap_or(0).to_string());
    add(out, "iscommlink", bool_text(false));
    add(out, "isprogram", bool_text(false));
    copy_bool(out, v, "active");
    copy_bool(out, v, "homenode");
    add(out, "matrixcm", (8 + (dr + 1) / 2).to_string());
    add(out, "matrixcmfilled", v.get_i32("matrixcmfilled").unwrap_or(0).to_string());
}

fn weapon_list(ctx: &Ctx, weapons: &[&Element]) -> Element {
    let mut out = Element::new("weapons");
    for w in weapons {
        out.push(items::weapon(ctx, w));
    }
    out
}

/// `VehicleMod.Print`.
fn vehicle_mod(ctx: &Ctx, m: &Element) -> Element {
    let mut out = Element::new("mod");
    head(ctx, &mut out, m, "");
    copy(&mut out, m, "limit");
    copy(&mut out, m, "slots");
    add(&mut out, "rating", m.get_i32("rating").unwrap_or(0).to_string());
    copy(&mut out, m, "ratinglabel");
    add(&mut out, "avail", avail(m));
    let c = ctx.nuyen(own_cost(m));
    add(&mut out, "cost", c.clone());
    add(&mut out, "owncost", c);
    copy(&mut out, m, "source");
    copy_bool(&mut out, m, "wirelesson");
    copy(&mut out, m, "page");
    copy_bool(&mut out, m, "included");
    out.push(weapon_list(ctx, &children(m, "weapons", "weapon")));
    let mut ware = Element::new("cyberwares");
    for w in children(m, "cyberwares", "cyberware") {
        ware.push(items::cyberware(ctx, w));
    }
    out.push(ware);
    ctx.notes(&mut out, m);
    out
}

/// `WeaponMount.Print` (also printed as `<mod>`).
fn weapon_mount(ctx: &Ctx, m: &Element) -> Element {
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
    add(&mut out, "avail", avail(m));
    let c = ctx.nuyen(own_cost(m));
    add(&mut out, "cost", c.clone());
    add(&mut out, "owncost", c);
    copy(&mut out, m, "page");
    copy(&mut out, m, "location");
    copy_bool(&mut out, m, "included");
    out.push(weapon_list(ctx, &children(m, "weapons", "weapon")));
    let mut mods = Element::new("mods");
    for x in children(m, "mods", "mod") {
        mods.push(vehicle_mod(ctx, x));
    }
    out.push(mods);
    ctx.notes(&mut out, m);
    out
}
