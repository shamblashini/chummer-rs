//! Adding gear and lifestyles, and their costs.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::items::{self, gear, lifestyle, Purchase};
use chummer_core::xml::Element;

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn find_gear<'a>(doc: &'a Element, name: &str) -> Option<&'a Element> {
    let mut all = Vec::new();
    doc.descendants("gear", &mut all);
    all.into_iter().find(|g| g.get("name") == name)
}

#[test]
fn commlink_comes_with_its_data_children() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let rec = data::find(&doc, "gears", "gear", "Meta Link").unwrap();
    let guid = items::add("gear", &mut ch, &store, rec, &Purchase::default()).unwrap();
    let g = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == guid).unwrap().clone();
    assert_eq!(g.get("sourceid"), rec.id());
    assert_eq!(g.get("devicerating"), "1");
    assert_eq!(g.get("canformpersona"), "Self");
    let functionality = g.child("children").unwrap().child("gear").unwrap();
    assert_eq!(functionality.get("name"), "Commlink Functionality");
    assert_eq!(functionality.get("parentid"), guid);
    assert_eq!(functionality.get("cost"), "0");
    let earbuds = find_gear(functionality, "Earbuds").unwrap();
    assert_eq!(earbuds.get("rating"), "1");
    assert_eq!(earbuds.get("capacity"), "[0]");
    // Only the commlink itself costs money.
    assert_eq!(gear::cost(&g), 100.0);
}

#[test]
fn gear_goes_into_its_parent() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let link = data::find(&doc, "gears", "gear", "Meta Link").unwrap();
    let parent = items::add("gear", &mut ch, &store, link, &Purchase::default()).unwrap();
    let ff = data::find(&doc, "gears", "gear", "Commlink Form Factor, Non-Standard").unwrap();
    let p = Purchase { parent: Some(parent.clone()), answer: Some("Bracelet".into()), ..Default::default() };
    let child = items::add("gear", &mut ch, &store, ff, &p).unwrap();
    let pg = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == parent).unwrap();
    let c = pg.child("children").unwrap().elements().find(|g| g.get("guid") == child).unwrap();
    assert_eq!(c.get("extra"), "Bracelet");
    // "Gear Cost * 0.2" of the 100 nuyen commlink.
    assert_eq!(gear::cost_in(c, pg), 20.0);
    assert_eq!(gear::cost(pg), 120.0);
}

#[test]
fn rating_quantity_and_bonus() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let before = ch.improvements.list.len();
    let medkit = data::find(&doc, "gears", "gear", "Medkit").unwrap();
    assert_eq!(gear::rating_range(medkit, None), Some((1, 6)));
    let guid = items::add("gear", &mut ch, &store, medkit, &Purchase { rating: 9, ..Default::default() }).unwrap();
    let g = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == guid).unwrap();
    assert_eq!(g.get("rating"), "6", "rating clamps to the maximum");
    assert_eq!(gear::cost(g), 1500.0);
    assert!(ch.improvements.list.len() > before, "the limit modifier bonus is applied");

    let tags = data::find(&doc, "gears", "gear", "Stealth Tags").unwrap();
    assert_eq!(gear::default_qty(tags), 10.0);
    let p = Purchase { qty: 30.0, ..Default::default() };
    let e = gear::element(&ch, &store, tags, &p, "x").unwrap();
    assert_eq!(e.get("costfor"), "10");
    assert_eq!(gear::cost(&e), 30.0);
}

#[test]
fn rebuild_reproduces_a_new_element() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("gear.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let rec = data::find(&doc, "gears", "gear", "Medkit").unwrap();
    let guid = items::add("gear", &mut ch, &store, rec, &Purchase { rating: 3, ..Default::default() }).unwrap();
    let saved = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == guid).unwrap();
    assert_eq!(&gear::rebuild(&ch, &store, saved).unwrap(), saved);
}

#[test]
fn addgear_bonus_creates_the_living_persona() {
    let store = DataStore::discover().unwrap();
    let qdoc = store.doc("qualities.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let techno = data::find(&qdoc, "qualities", "quality", "Technomancer").unwrap();
    let q = chummer_core::chargen::add_quality(&mut ch, &store, techno, None);
    let persona = ch.items("gears", "gear").into_iter().find(|g| g.get("name") == "Living Persona").unwrap();
    assert_eq!(persona.get("parentid"), q);
    assert_eq!(persona.get("cost"), "0");
    let link = ch.improvements.list.iter().find(|i| i.kind == "Gear" && i.source_name == q).unwrap();
    assert_eq!(link.improved_name, persona.get("guid"));
}

#[test]
fn lifestyle_costs() {
    let store = DataStore::discover().unwrap();
    let doc = store.doc("lifestyles.xml").unwrap();
    let mut ch = fixture("Barrett.chum5");
    let low = data::find(&doc, "lifestyles", "lifestyle", "Low").unwrap();
    let o = lifestyle::Options { name: "Flat".into(), months: 3, roommates: 2, ..Default::default() };
    let guid = lifestyle::add_with(&mut ch, &store, low, &o).unwrap();
    let l = ch.items("lifestyles", "lifestyle").into_iter().find(|l| l.get("guid") == guid).unwrap().clone();
    assert_eq!(l.get("name"), "Flat");
    assert_eq!(l.get("baselifestyle"), "Low");
    // 2000 + 10% per roommate.
    assert!((lifestyle::monthly_cost(&ch, &l) - 2400.0).abs() < 1e-6);
    assert!((lifestyle::total_cost(&ch, &l) - 7200.0).abs() < 1e-6);
    let o = lifestyle::Options { trust_fund: true, ..o };
    let e = lifestyle::element_with(&ch, &store, low, &o, "x").unwrap();
    assert_eq!(e.get("roommates"), "0");
    assert_eq!(lifestyle::monthly_cost(&ch, &e), 0.0);

    let gym = data::find(&doc, "qualities", "quality", "Gym").unwrap();
    lifestyle::add_quality(&mut ch, &store, &guid, gym, None, false).unwrap();
    let l = ch.items("lifestyles", "lifestyle").into_iter().find(|l| l.get("guid") == guid).unwrap();
    // The gym is an asset: its flat 300 is added before the roommates.
    assert!((lifestyle::monthly_cost(&ch, l) - 2300.0 * 1.2).abs() < 1e-6);
    assert_eq!(&lifestyle::rebuild(&ch, &store, l).unwrap().get("cost"), "2000");
}
