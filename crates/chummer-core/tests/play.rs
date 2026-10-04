//! In-play tracking (`play`): Edge, ammunition, device matrix monitors,
//! the active commlink and vehicle damage. Each change survives a save
//! and reload under Chummer5a's element names.

use std::path::PathBuf;

use chummer_core::calc::{self, Rules};
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::items::{self, edit, vehicle as vcalc, Purchase};
use chummer_core::career;
use chummer_core::play::{ammo, matrix, vehicle};
use chummer_core::xml::Element;

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn reload(ch: &Character) -> Character {
    Character::from_str(&ch.to_xml_string()).unwrap()
}

fn sheet(ch: &Character) -> calc::Sheet {
    calc::compute(ch, &Rules::default(), None, None)
}

fn qty(ch: &Character, guid: &str) -> Option<f64> {
    edit::find(ch, guid).and_then(|g| g.get_f64("qty"))
}

#[test]
fn edge_boxes_and_refresh() {
    let mut ch = fixture("Barrett.chum5");
    ch.doc.remove_children("edgeused");
    let total = sheet(&ch).attr("EDG");
    assert!(total >= 2, "EDG {total}");
    assert!(career::set_edge_used(&mut ch, total, 2));
    assert!(!career::set_edge_used(&mut ch, total, 2));
    assert_eq!(reload(&ch).doc.get("edgeused"), "2");
    // Clamped to the Edge total.
    assert!(career::set_edge_used(&mut ch, total, total + 5));
    assert_eq!(ch.doc.get_i32("edgeused"), Some(total));
    assert!(career::refresh_edge(&mut ch));
    assert!(!career::refresh_edge(&mut ch));
    assert_eq!(reload(&ch).doc.get("edgeused"), "0");
}

/// A character with an Ares Predator V and 50 regular rounds for it.
fn gun_and_ammo(store: &DataStore) -> (Character, String, String) {
    let mut ch = fixture("Barrett.chum5");
    let wdoc = store.doc("weapons.xml").unwrap();
    let gdoc = store.doc("gear.xml").unwrap();
    let gun = data::find(&wdoc, "weapons", "weapon", "Ares Predator V").unwrap();
    let w = items::add("weapon", &mut ch, store, gun, &Purchase::default()).unwrap();
    let rec = data::find(&gdoc, "gears", "gear", "Ammo: Regular Ammo").unwrap();
    let a = items::add("gear", &mut ch, store, rec, &Purchase { qty: 50.0, answer: Some("Heavy Pistols".into()), ..Default::default() }).unwrap();
    (ch, w, a)
}

#[test]
fn reload_fire_and_unload() {
    let store = DataStore::discover().unwrap();
    let (mut ch, w, a) = gun_and_ammo(&store);
    assert_eq!(qty(&ch, &a), Some(50.0));
    let wel = edit::find(&ch, &w).unwrap().clone();
    assert_eq!(ammo::reload_counts(&ch, &wel), vec!["15".to_owned()]);
    assert_eq!(ammo::clips(&wel).len(), 1);
    assert_eq!(ammo::remaining(&wel), 0);
    let choices = ammo::reloadable(&ch, Some(&store), &w);
    assert_eq!(choices.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec![a.as_str()]);

    // Reloading splits 15 rounds off the stack.
    ammo::reload(&mut ch, &w, Some(&a), 15).unwrap();
    assert_eq!(qty(&ch, &a), Some(35.0));
    let wel = edit::find(&ch, &w).unwrap().clone();
    assert_eq!(ammo::remaining(&wel), 15);
    let loaded = ammo::loaded(&ch, &wel).unwrap().get("guid");
    assert_ne!(loaded, a);
    assert_eq!(qty(&ch, &loaded), Some(15.0));
    // Loaded rounds are not offered again.
    assert_eq!(ammo::reloadable(&ch, Some(&store), &w).len(), 1);

    // Semi-automatic: single shots and short bursts, no full auto.
    assert!(ammo::allows(&ch, &wel, ammo::FireMode::SingleShot));
    assert!(ammo::allows(&ch, &wel, ammo::FireMode::ShortBurst));
    assert!(!ammo::allows(&ch, &wel, ammo::FireMode::FullBurst));
    assert_eq!(ammo::fire(&mut ch, &w, ammo::FireMode::ShortBurst), ammo::Fired::Fired(3));
    assert_eq!(ammo::fire(&mut ch, &w, ammo::FireMode::SingleShot), ammo::Fired::Fired(1));
    assert_eq!(qty(&ch, &loaded), Some(11.0));

    // Saved as Chummer does: <clips><clip><count/><location/><id/>.
    let mut back = reload(&ch);
    let wel = edit::find(&back, &w).unwrap();
    let clip = wel.child("clips").unwrap().child("clip").unwrap();
    let names: Vec<&str> = clip.elements().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["count", "location", "id"]);
    assert_eq!(clip.get("count"), "11");
    assert_eq!(clip.get("location"), "loaded");
    assert_eq!(clip.get("id"), loaded);
    let pos = |n: &str| wel.elements().position(|e| e.name == n).unwrap();
    assert_eq!(pos("clips"), pos("activeammoslot") + 1);
    assert_eq!(ammo::remaining(wel), 11);

    // Not enough for a long burst: Chummer asks first; nothing is used yet.
    ammo::set_remaining(&mut back, &w, 4);
    assert_eq!(ammo::fire(&mut back, &w, ammo::FireMode::LongBurst), ammo::Fired::Confirm("Not enough Ammunition. Treat as shortened Long Burst?"));
    assert_eq!(ammo::remaining(edit::find(&back, &w).unwrap()), 4);
    // Firing the last rounds deletes the loaded gear.
    ammo::set_remaining(&mut back, &w, 0);
    assert!(edit::find(&back, &loaded).is_none());
    assert!(edit::find(&back, &w).unwrap().child("clips").is_none());
    assert_eq!(ammo::fire(&mut back, &w, ammo::FireMode::SingleShot), ammo::Fired::OutOfAmmo);

    // Unloading puts the rounds back on the identical stack.
    assert!(ammo::unload(&mut ch, &w));
    assert!(edit::find(&ch, &loaded).is_none());
    assert_eq!(qty(&ch, &a), Some(46.0));
    assert_eq!(ammo::remaining(edit::find(&ch, &w).unwrap()), 0);
}

#[test]
fn topping_up_and_switching_clips() {
    let store = DataStore::discover().unwrap();
    let (mut ch, w, a) = gun_and_ammo(&store);
    ammo::reload(&mut ch, &w, Some(&a), 15).unwrap();
    ammo::fire(&mut ch, &w, ammo::FireMode::ShortBurst);
    let loaded = ammo::loaded(&ch, edit::find(&ch, &w).unwrap()).unwrap().get("guid");
    // Same ammunition: the loaded gear is topped up from the stack.
    ammo::reload(&mut ch, &w, Some(&a), 15).unwrap();
    assert_eq!(qty(&ch, &loaded), Some(15.0));
    assert_eq!(qty(&ch, &a), Some(32.0));
    assert_eq!(ammo::remaining(edit::find(&ch, &w).unwrap()), 15);

    // A second slot (as an accessory would add) keeps its position.
    items::find_by_guid_mut(&mut ch.doc, &w).unwrap().set_child_text("ammoslots", "2");
    assert!(ammo::set_active_slot(&mut ch, &w, 2));
    ammo::reload(&mut ch, &w, Some(&a), 10).unwrap();
    let back = reload(&ch);
    let wel = edit::find(&back, &w).unwrap();
    let cs = ammo::clips(wel);
    assert_eq!(cs.iter().map(|c| c.count).collect::<Vec<_>>(), vec![15, 10]);
    assert_eq!(ammo::active_slot(wel), 2);
    assert_eq!(qty(&back, &a), Some(22.0));
}

#[test]
fn charges_for_weapons_without_ammunition() {
    let store = DataStore::discover().unwrap();
    let (mut ch, w, _) = gun_and_ammo(&store);
    items::find_by_guid_mut(&mut ch.doc, &w).unwrap().set_child_text("requireammo", "False");
    assert!(ammo::reloadable(&ch, Some(&store), &w).is_empty());
    assert!(ammo::set_charges(&mut ch, &w, 40));
    assert_eq!(ammo::remaining(edit::find(&ch, &w).unwrap()), 15);
    assert_eq!(ammo::fire(&mut ch, &w, ammo::FireMode::SingleShot), ammo::Fired::Fired(1));
    let back = reload(&ch);
    let wel = edit::find(&back, &w).unwrap();
    assert_eq!(ammo::remaining(wel), 14);
    assert_eq!(wel.child("clips").unwrap().child("clip").unwrap().get("id"), ammo::EMPTY_GUID);
}

fn add_gear(ch: &mut Character, store: &DataStore, name: &str) -> String {
    let gdoc = store.doc("gear.xml").unwrap();
    let rec = data::find(&gdoc, "gears", "gear", name).unwrap();
    items::add("gear", ch, store, rec, &Purchase::default()).unwrap()
}

#[test]
fn commlink_matrix_monitor_and_active_commlink() {
    let store = DataStore::discover().unwrap();
    let mut ch = fixture("Barrett.chum5");
    let ikon = add_gear(&mut ch, &store, "Hermes Ikon");
    let avalon = add_gear(&mut ch, &store, "Transys Avalon");
    let e = edit::find(&ch, &ikon).unwrap().clone();
    assert!(matrix::is_commlink(&e));
    assert!(matrix::has_matrix(&e));
    assert_eq!(matrix::total(&e, "Device Rating"), 5);
    assert_eq!(matrix::total(&e, "Data Processing"), 5);
    assert_eq!(matrix::total(&e, "Attack"), 0);
    // 8 + ⌈5 / 2⌉.
    assert_eq!(matrix::condition_monitor(&e), 11);

    assert!(matrix::set_filled(&mut ch, &ikon, 4));
    assert!(matrix::set_filled(&mut ch, &avalon, 99));
    let back = reload(&ch);
    assert_eq!(matrix::filled(edit::find(&back, &ikon).unwrap()), 4);
    // Clamped to the Avalon's 8 + 3 boxes.
    assert_eq!(matrix::filled(edit::find(&back, &avalon).unwrap()), 11);

    let active = |ch: &Character| {
        let mut v = Vec::new();
        ch.doc.descendants("active", &mut v);
        v.iter().filter(|e| e.text() == "True").count()
    };
    matrix::set_active(&mut ch, &ikon, true);
    matrix::set_active(&mut ch, &avalon, true);
    assert_eq!(active(&ch), 1);
    let back = reload(&ch);
    assert_eq!(matrix::active_commlink(&back).map(|c| c.get("guid")), Some(avalon.clone()));
    // Matrix initiative uses the active commlink's Data Processing.
    let s = sheet(&back);
    let int = s.attr("INT");
    assert_eq!(s.matrix_cold_initiative - s.wound_modifier, int + 6 + back.improvements.val_int("MatrixInitiative", None));
    matrix::set_active(&mut ch, &avalon, false);
    assert!(matrix::active_commlink(&ch).is_none());
    // Only devices that can form a persona can be active.
    let other = add_gear(&mut ch, &store, "Ammo: Regular Ammo");
    assert!(!matrix::set_active(&mut ch, &other, true));
}

fn vehicle_rec<'a>(doc: &'a Element, pred: impl Fn(&Element) -> bool) -> data::Record<'a> {
    data::records(doc, "vehicles", "vehicle").into_iter().find(|r| pred(r.el())).unwrap()
}

#[test]
fn vehicle_and_drone_damage() {
    let store = DataStore::discover().unwrap();
    let vdoc = store.doc("vehicles.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let car = items::add("vehicle", &mut ch, &store, data::find(&vdoc, "vehicles", "vehicle", "GMC Bulldog Step-Van (Van)").unwrap(), &Purchase::default()).unwrap();
    let drone = items::add("vehicle", &mut ch, &store, vehicle_rec(&vdoc, |e| e.get("category") == "Drones: Small"), &Purchase::default()).unwrap();
    let rules = vcalc::VehicleRules::default();
    for (g, base) in [(&car, 12), (&drone, 6)] {
        let v = edit::find(&ch, g).unwrap();
        let body = vcalc::stats(v).body;
        assert_eq!(vehicle::condition_monitor(v, &rules), base + (body + 1) / 2, "{}", v.get("name"));
        assert_eq!(matrix::condition_monitor(v), 8 + (vcalc::stats(v).device_rating + 1) / 2);
    }
    assert!(vehicle::set_filled(&mut ch, &car, 3, &rules));
    assert!(!vehicle::set_filled(&mut ch, &car, 3, &rules));
    assert!(vehicle::set_filled(&mut ch, &drone, 100, &rules));
    assert!(matrix::set_filled(&mut ch, &drone, 2));
    let back = reload(&ch);
    assert_eq!(edit::find(&back, &car).unwrap().get("physicalcmfilled"), "3");
    let d = edit::find(&back, &drone).unwrap();
    assert_eq!(vehicle::filled(d), vehicle::condition_monitor(d, &rules));
    assert_eq!(d.get("matrixcmfilled"), "2");
}
