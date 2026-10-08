//! Editing saved items (`items::edit`): rating changes match a fresh build,
//! equipped armor counts towards the armor rating, removal and selling
//! clean up improvements, vehicle weapons land in their mount.

use std::path::PathBuf;

use chummer_core::calc::{self, Rules};
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::expr::{Availability, NoAttributes};
use chummer_core::items::{self, cyberware, edit, gear, Purchase};

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn avail(e: &chummer_core::xml::Element) -> String {
    Availability::parse(&e.get("avail"), e.get_i32("rating").unwrap_or(0), e.get_i32("minrating").unwrap_or(0), &NoAttributes).to_string()
}

fn from_source(ch: &Character, guid: &str) -> Vec<f64> {
    ch.improvements.list.iter().filter(|i| i.source_name.eq_ignore_ascii_case(guid)).map(|i| i.val).collect()
}

#[test]
fn gear_rating_change_matches_a_fresh_build() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    // "Rating * 150" nuyen, "Rating * 2" availability, bonus values "Rating".
    let rec = data::find(&doc, "gears", "gear", "Chemsuit").unwrap();
    let guid = items::add("gear", &mut ch, &store, rec, &Purchase { rating: 2, ..Default::default() }).unwrap();
    edit::set_text(&mut ch, &guid, "notes", "my suit");
    let imps_before = ch.improvements.list.len();
    assert_eq!(edit::rating_range(&ch, &store, &guid), Some((1, 6)));

    assert_eq!(edit::apply_rating_change(&mut ch, &store, &guid, 5), Ok(5));
    let edited = edit::find(&ch, &guid).unwrap().clone();
    let fresh = gear::element(&ch, &store, rec, &Purchase { rating: 5, ..Default::default() }, &guid).unwrap();
    assert_eq!(edited.get("rating"), fresh.get("rating"));
    assert_eq!(gear::cost(&edited), gear::cost(&fresh));
    assert_eq!(gear::cost(&edited), 750.0);
    assert_eq!(avail(&edited), avail(&fresh));
    assert_eq!(edit::availability(&ch, &store, &guid), avail(&fresh));
    assert_eq!(edit::total_cost(&ch, &store, &guid), 750.0);
    // User state is kept.
    assert_eq!(edited.get("notes"), "my suit");
    // The improvements are made again at the new rating, not added twice.
    assert_eq!(ch.improvements.list.len(), imps_before);
    let vals = from_source(&ch, &guid);
    assert!(!vals.is_empty());
    assert!(vals.iter().all(|v| *v == 5.0), "{vals:?}");

    // Out of range clamps to the maximum.
    assert_eq!(edit::apply_rating_change(&mut ch, &store, &guid, 99), Ok(6));
    assert_eq!(edit::find(&ch, &guid).unwrap().get("rating"), "6");
}

#[test]
fn ware_rating_change_matches_a_fresh_build() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("cyberware.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let rec = data::find(&doc, "cyberwares", "cyberware", "Muscle Replacement").unwrap();
    let guid = items::add("cyberware", &mut ch, &store, rec, &Purchase { rating: 1, ..Default::default() }).unwrap();
    let imps_before = ch.improvements.list.len();
    assert_eq!(edit::apply_rating_change(&mut ch, &store, &guid, 3), Ok(3));
    let edited = edit::find(&ch, &guid).unwrap().clone();
    let (fresh, _) = cyberware::element("cyberware", &ch, &store, rec, &Purchase { rating: 3, ..Default::default() }, &guid);
    let rules = Rules::default();
    assert_eq!(cyberware::cost(&ch, &store, &edited), cyberware::cost(&ch, &store, &fresh));
    assert_eq!(cyberware::cost(&ch, &store, &edited), 75000.0);
    assert_eq!(edit::essence(&ch, &store, &rules, &guid), Some(cyberware::essence(&ch, &store, &rules, &fresh)));
    assert_eq!(ch.improvements.list.len(), imps_before);
    let aug: Vec<f64> = ch.improvements.list.iter().filter(|i| i.source_name == guid && i.kind == "Attribute").map(|i| i.aug).collect();
    assert_eq!(aug, vec![3.0, 3.0], "the AGI/STR bonus follows the rating");
}

#[test]
fn equipped_armor_counts_towards_armor() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("armor.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    for a in ch.items("armors", "armor").into_iter().map(|a| a.get("guid")).collect::<Vec<_>>() {
        edit::set_equipped(&mut ch, &store, &a, false);
    }
    let rules = Rules::default();
    let bare = calc::compute(&ch, &rules, Some(&store), None).armor;
    let rec = data::find(&doc, "armors", "armor", "Armor Jacket").unwrap();
    let guid = items::add("armor", &mut ch, &store, rec, &Purchase::default()).unwrap();
    let worn = calc::compute(&ch, &rules, Some(&store), None).armor;
    assert_eq!(worn, bare + 12);
    assert!(edit::set_equipped(&mut ch, &store, &guid, false));
    assert_eq!(edit::find(&ch, &guid).unwrap().get("equipped"), "False");
    assert_eq!(calc::compute(&ch, &rules, Some(&store), None).armor, bare);
    edit::set_equipped(&mut ch, &store, &guid, true);
    assert_eq!(calc::compute(&ch, &rules, Some(&store), None).armor, worn);
}

#[test]
fn unequipping_disables_the_item_improvements() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let rec = data::find(&doc, "gears", "gear", "Chemsuit").unwrap();
    let guid = items::add("gear", &mut ch, &store, rec, &Purchase { rating: 3, ..Default::default() }).unwrap();
    let active = |ch: &Character| ch.improvements.active().filter(|i| i.source_name == guid).count();
    let n = active(&ch);
    assert!(n > 0);
    edit::set_equipped(&mut ch, &store, &guid, false);
    assert_eq!(active(&ch), 0);
    edit::set_equipped(&mut ch, &store, &guid, true);
    assert_eq!(active(&ch), n);
}

#[test]
fn removing_drops_nested_improvements() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let link = data::find(&doc, "gears", "gear", "Meta Link").unwrap();
    let parent = items::add("gear", &mut ch, &store, link, &Purchase::default()).unwrap();
    assert!(edit::child_kinds(&ch, &parent).iter().any(|k| k.tag == "gear"));
    let suit = data::find(&doc, "gears", "gear", "Chemsuit").unwrap();
    let child = items::add("gear", &mut ch, &store, suit, &Purchase { rating: 2, parent: Some(parent.clone()), ..Default::default() }).unwrap();
    assert_eq!(edit::parent(&ch, &child).map(|p| p.get("guid")), Some(parent.clone()));
    assert!(edit::children(&ch, &parent).iter().any(|(g, _, _)| *g == child));
    assert!(!from_source(&ch, &child).is_empty());
    assert!(edit::remove(&mut ch, &parent));
    assert!(edit::find(&ch, &child).is_none());
    assert!(from_source(&ch, &child).is_empty());
}

#[test]
fn selling_pays_the_sale_value() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    ch.created = true;
    ch.improvements.career = true;
    let rec = data::find(&doc, "gears", "gear", "Chemsuit").unwrap();
    let guid = items::add("gear", &mut ch, &store, rec, &Purchase { rating: 4, ..Default::default() }).unwrap();
    let before = ch.nuyen;
    let expect = edit::sale_value(&ch, &store, &guid, 0.5);
    assert_eq!(expect, 300.0);
    assert_eq!(edit::sell(&mut ch, &store, &guid, 0.5).unwrap(), expect);
    assert_eq!(ch.nuyen, before + expect);
    assert!(edit::find(&ch, &guid).is_none());
    assert!(from_source(&ch, &guid).is_empty());
}

#[test]
fn vehicle_weapons_go_into_the_mount() {
    let store = DataStore::discover().unwrap();
    let vdoc = store.doc("vehicles.xml").unwrap();
    let wdoc = store.doc("weapons.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let rec = data::find(&vdoc, "vehicles", "vehicle", "GMC Bulldog Step-Van (Van)").unwrap();
    let v = items::add("vehicle", &mut ch, &store, rec, &Purchase::default()).unwrap();
    let (size, _) = edit::weapon_mount_sizes(&store).into_iter().next().unwrap();
    let mount = edit::add_weapon_mount(&mut ch, &store, &v, &size).unwrap();
    assert!(edit::child_kinds(&ch, &mount).iter().any(|k| k.tag == "weapon"));
    let gun = data::find(&wdoc, "weapons", "weapon", "Ares Predator V").unwrap();
    let w = items::add("weapon", &mut ch, &store, gun, &Purchase { parent: Some(mount.clone()), ..Default::default() }).unwrap();
    edit::settle_new_item(&mut ch, &w);
    let m = edit::find(&ch, &mount).unwrap();
    assert!(m.child("underbarrel").is_none());
    assert!(m.child("weapons").unwrap().elements().any(|e| e.get("guid") == w));
    assert_eq!(edit::parent(&ch, &w).map(|p| p.name.clone()), Some("weaponmount".into()));
}

#[test]
fn capacity_strings() {
    assert_eq!(edit::parse_capacity("6", 0), (6.0, 0.0));
    assert_eq!(edit::parse_capacity("[2]", 0), (0.0, 2.0));
    assert_eq!(edit::parse_capacity("Rating/[1]", 3), (3.0, 1.0));
    assert_eq!(edit::parse_capacity("[*]", 0), (0.0, 0.0));
    assert_eq!(edit::custom_name_field("gear"), Some("gearname"));
}

#[test]
fn addon_categories_come_from_the_data_record() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let glasses = data::find(&doc, "gears", "gear", "Glasses").unwrap();
    let g = items::add("gear", &mut ch, &store, glasses, &Purchase { rating: 2, ..Default::default() }).unwrap();
    assert_eq!(edit::addon_categories(&ch, &store, &g), ["Vision Enhancements", "Sensors", "Custom"]);
    // A commlink takes any gear (no list).
    let link = data::find(&doc, "gears", "gear", "Meta Link").unwrap();
    let l = items::add("gear", &mut ch, &store, link, &Purchase::default()).unwrap();
    assert!(edit::addon_categories(&ch, &store, &l).is_empty());
    assert!(edit::addon_categories(&ch, &store, "no such item").is_empty());
}
