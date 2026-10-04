//! Vehicles, drones, vehicle mods and weapon mounts: derived values on the
//! fixtures, costs, and adding items from the data.
//!
//! Run with VEHICLE_DUMP=1 to print the values of every fixture vehicle.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::items::{self, vehicle, Purchase};
use chummer_core::xml::Element;

fn fixture(name: &str) -> Character {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    Character::load(&p).unwrap()
}

fn vehicle_named<'a>(ch: &'a Character, name: &str) -> &'a Element {
    ch.items("vehicles", "vehicle").into_iter().find(|v| v.get("name") == name).unwrap_or_else(|| panic!("no vehicle {name}"))
}

#[test]
fn dump_fixture_vehicles() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let dump = std::env::var("VEHICLE_DUMP").is_ok();
    for f in files {
        let ch = Character::load(&f).unwrap();
        for v in ch.items("vehicles", "vehicle") {
            // Every fixture vehicle evaluates without panicking and to sane values.
            let s = vehicle::stats(v);
            let c = vehicle::cost(v);
            assert!(s.body >= 0 && s.seats >= 0 && s.slots >= 4, "{}: {s:?}", v.get("name"));
            assert!(c >= vehicle::own_cost(v), "{}: total {c} below own cost", v.get("name"));
            if dump {
                let cats: Vec<String> = s.categories.iter().map(|c| format!("{}={}/{}", &c.category[..3], c.used, c.total)).collect();
                eprintln!(
                    "{:<22} {:<40} H {} S {} A {} B {} Ar {} P {} Se {} St {} DR {} slots {}/{} drone {}/{} [{}] cost {}",
                    f.file_name().unwrap().to_string_lossy(),
                    v.get("name"),
                    s.handling_text,
                    s.speed_text,
                    s.accel_text,
                    s.body,
                    s.armor,
                    s.pilot,
                    s.sensor,
                    s.seats,
                    s.device_rating,
                    s.slots_used,
                    s.slots,
                    s.drone_mod_slots_used,
                    s.drone_mod_slots,
                    cats.join(" "),
                    c
                );
            }
        }
    }
}

/// Saeder-Krupp LT-21 in Serpent: Armor (Concealed) 5 saved with the old
/// override bonus `<armor>Rating</armor>`, three Increased Seating
/// (`+Seats * 0.5`), Chameleon Coating (`Body * 1000`).
#[test]
fn lt21_totals() {
    let ch = fixture("Serpent.chum5");
    let v = vehicle_named(&ch, "Saeder-Krupp LT-21 (Delivery Van)");
    let s = vehicle::stats(v);
    assert!(!s.is_drone);
    // Override 5 loses against the base 7.
    assert_eq!(s.armor, 7);
    // Each Increased Seating adds round_up(2 * 0.5) on the base 2.
    assert_eq!(s.seats, 5);
    assert_eq!(s.body, 15);
    assert_eq!((s.handling, s.offroad_handling, s.handling_text.as_str()), (2, 1, "2/1"));
    assert_eq!(s.slots, 15);
    let body = s.categories.iter().find(|c| c.category == "Body").unwrap();
    // Chameleon 2 + Entry 1 + 3 x Seating 2.
    assert!(body.used >= 9, "{body:?}");
    let protection = s.categories.iter().find(|c| c.category == "Protection").unwrap();
    assert_eq!((protection.used, protection.total), (15, 15));
    // 31000 + Chameleon 15000 + Entry 2500 + Armor 15000 + Seating 3 x 2000 + ...
    assert!(vehicle::cost(v) >= 31000.0 + 15000.0 + 2500.0 + 15000.0 + 6000.0);
    assert_eq!(vehicle::own_cost(v), 31000.0);
}

#[test]
fn drones_are_detected() {
    let ch = fixture("Serpent.chum5");
    let lynx = vehicle_named(&ch, "Steel Lynx Combat Drone (Large)");
    assert!(vehicle::is_drone(lynx));
    let s = vehicle::stats(lynx);
    assert!(s.is_drone);
    // Drone maximums: twice the base.
    assert_eq!(s.max_handling, (lynx.get_i32("handling").unwrap() * 2).max(1));
    let van = vehicle_named(&ch, "Saeder-Krupp LT-21 (Delivery Van)");
    assert!(!vehicle::is_drone(van));
    assert_eq!(vehicle::stats(van).max_handling, i32::MAX);
}

#[test]
fn drone_rules_lift_armor_cap() {
    let ch = fixture("Serpent.chum5");
    let lynx = vehicle_named(&ch, "Steel Lynx Combat Drone (Large)");
    let std = vehicle::stats(lynx);
    let rules = vehicle::VehicleRules { drone_mods: true, ..Default::default() };
    let r5 = vehicle::stats_with(lynx, &rules);
    assert_eq!(std.max_armor, lynx.get_i32("body").unwrap() + lynx.get_i32("armor").unwrap());
    assert_eq!(r5.max_armor, i32::MAX);
}

/// The mount, its options and the data mods of the F-B Bumblebee.
#[test]
fn bumblebee_from_data() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("vehicles.xml").unwrap();
    let rec = data::find(&doc, "vehicles", "vehicle", "F-B Bumblebee").unwrap();
    let v = vehicle::vehicle_element(&store, rec, &Purchase::default(), "g").unwrap();
    assert_eq!(v.get("sourceid"), "73b7729b-89c1-44fb-950c-f8391376a6b8");
    assert_eq!(v.get("handling"), "3");
    assert_eq!(v.get("offroadhandling"), "3");
    assert_eq!(v.get("modslots"), "4", "defaults to Body");
    let mods: Vec<&Element> = v.child("mods").unwrap().elements().collect();
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].get("name"), "Rigger Interface");
    assert_eq!(mods[0].get("included"), "True");
    let mounts: Vec<&Element> = v.child("weaponmounts").unwrap().elements().collect();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].get("name"), "Heavy [SR5]");
    assert_eq!(mounts[0].get("included"), "True");
    let opts: Vec<(String, String)> = mounts[0].child("weaponmountoptions").unwrap().elements().map(|o| (o.get("category"), o.get("name"))).collect();
    assert_eq!(opts.len(), 3, "{opts:?}");
    assert_eq!(opts.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["Flexibility", "Control", "Visibility"]);
    // Included mods and mounts cost nothing and use no slots.
    assert_eq!(vehicle::cost(&v), 24000.0);
    let s = vehicle::stats(&v);
    assert_eq!(s.drone_mod_slots_used, 0);
    assert_eq!(s.slots_used, 0);
}

/// Split handling ("4/2") goes to both fields.
#[test]
fn split_handling() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("vehicles.xml").unwrap();
    let rec = data::find(&doc, "vehicles", "vehicle", "Cocotaxi").unwrap();
    let v = vehicle::vehicle_element(&store, rec, &Purchase::default(), "g").unwrap();
    assert_eq!((v.get("handling").as_str(), v.get("offroadhandling").as_str()), ("4", "2"));
    assert_eq!(vehicle::stats(&v).handling_text, "4/2");
}

fn new_character() -> Character {
    Character::from_str("<character><created>False</created><nuyen>0</nuyen></character>").unwrap()
}

/// Add a vehicle, a rated mod and a weapon mount; the mod changes the totals.
#[test]
fn add_vehicle_mod_and_mount() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("vehicles.xml").unwrap();
    let mut ch = new_character();
    let rec = data::find(&doc, "vehicles", "vehicle", "Ford Americar (Sedan)").unwrap();
    let vg = items::add("vehicle", &mut ch, &store, rec, &Purchase::default()).unwrap();
    let before = vehicle::stats(vehicle_named(&ch, "Ford Americar (Sedan)"));

    let armor = data::find(&doc, "mods", "mod", "Armor (Concealed)").unwrap();
    let p = Purchase { rating: 99, parent: Some(vg.clone()), ..Default::default() };
    let mg = items::add("mod", &mut ch, &store, armor, &p).unwrap();
    let v = vehicle_named(&ch, "Ford Americar (Sedan)");
    let m = v.child("mods").unwrap().elements().find(|m| m.get("guid") == mg).unwrap();
    // Rated to Body at most (`<rating>body</rating>`).
    assert_eq!(m.get_i32("rating"), v.get_i32("body"));
    let after = vehicle::stats(v);
    let body = v.get_i32("body").unwrap();
    assert_eq!(after.armor, (v.get_i32("armor").unwrap() + body).min(after.max_armor));
    assert!(after.armor > before.armor);
    assert!(vehicle::cost(v) > vehicle::own_cost(v));

    let mount = vehicle::add_weapon_mount(&mut ch, &store, &vg, "Standard [SR5]", &["Flexible [SR5]", "Remote [SR5]", "External [SR5]"], &Purchase::default()).unwrap();
    let v = vehicle_named(&ch, "Ford Americar (Sedan)");
    let w = v.child("weaponmounts").unwrap().elements().find(|m| m.get("guid") == mount).unwrap();
    assert_eq!(w.child("weaponmountoptions").unwrap().elements().count(), 3);
    let s = vehicle::stats(v);
    let weapons = s.categories.iter().find(|c| c.category == "Weapons").unwrap();
    assert!(weapons.used >= 3, "{weapons:?}");
}

/// Mods are never added to things that are not vehicles or mounts.
#[test]
fn mod_needs_vehicle_parent() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("vehicles.xml").unwrap();
    let mut ch = new_character();
    let armor = data::find(&doc, "mods", "mod", "Armor (Concealed)").unwrap();
    assert!(items::add("mod", &mut ch, &store, armor, &Purchase::default()).is_err());
    let p = Purchase { parent: Some("nope".into()), ..Default::default() };
    assert!(items::add("mod", &mut ch, &store, armor, &p).is_err());
}

/// Adding never deducts nuyen itself; career purchases go through the
/// ledger (career::pay_for_item) at the vehicle's cost.
#[test]
fn career_purchase_costs_nuyen() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("vehicles.xml").unwrap();
    let mut ch = Character::from_str("<character><created>True</created><nuyen>50000</nuyen></character>").unwrap();
    let rec = data::find(&doc, "vehicles", "vehicle", "Ford Americar (Sedan)").unwrap();
    let g = items::add("vehicle", &mut ch, &store, rec, &Purchase::default()).unwrap();
    assert_eq!(ch.nuyen, 50000.0);
    let cost = items::edit::total_cost(&ch, &store, &g);
    assert_eq!(cost, 16000.0);
    chummer_core::career::pay_for_item(&mut ch, "vehicle", None, &g, cost).unwrap();
    assert_eq!(ch.nuyen, 50000.0 - 16000.0);
}
