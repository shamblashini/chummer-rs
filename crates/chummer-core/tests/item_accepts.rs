//! What an owned item takes inside it (`items::place::accepts`): the
//! kinds its "Add …" commands offer and the Workspace catalog switches
//! to when it is selected, with the categories each takes.

use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::command::{Command, RecordRef, Session};
use chummer_core::engine::Engine;
use chummer_core::items::place::{self, Candidate, Dest};
use chummer_core::items::{edit, Purchase};

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn engine() -> &'static Engine {
    static E: std::sync::OnceLock<Engine> = std::sync::OnceLock::new();
    E.get_or_init(|| Engine::load().unwrap())
}

fn session() -> Session {
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Human".into(),
        metavariant: None,
        priorities: Priorities(['D', 'E', 'A', 'B', 'C']),
        talent: "Mundane".into(),
        talent_skills: Vec::new(),
        name: "Holder".into(),
    };
    Session::with_seed(chargen::create(engine(), &spec).unwrap(), 11)
}

fn guids(ch: &Character) -> Vec<String> {
    fn walk(e: &chummer_core::xml::Element, out: &mut Vec<String>) {
        if edit::is_item(e) {
            out.push(e.get("guid"));
        }
        for c in e.elements() {
            walk(c, out);
        }
    }
    let mut v = Vec::new();
    walk(&ch.doc, &mut v);
    v
}

/// Buy `name` of kind `tag`; returns the new item's guid.
fn buy(s: &mut Session, tag: &str, name: &str, rating: i32, parent: Option<&str>, answer: Option<&str>) -> String {
    let before = guids(s.ch());
    let purchase = Purchase { rating, qty: 1.0, parent: parent.map(str::to_owned), answer: answer.map(str::to_owned), cost_multiplier: 1.0, ..Default::default() };
    s.apply(engine(), Command::AddItem { tag: tag.into(), record: RecordRef { id: String::new(), name: name.into() }, purchase }).unwrap_or_else(|e| panic!("buying {name}: {}", e.reason));
    let new: Vec<String> = guids(s.ch()).into_iter().filter(|g| !before.contains(g)).collect();
    new.iter().find(|g| edit::parent(s.ch(), g).is_none_or(|p| !new.contains(&p.get("guid")))).cloned().unwrap_or_else(|| panic!("{name} was added"))
}

fn kinds(s: &Session, g: &str) -> Vec<&'static str> {
    let store = engine().store_for_character(s.ch());
    place::accepts(s.ch(), &store, g).iter().map(|a| a.tag).collect()
}

fn categories(s: &Session, g: &str, tag: &str) -> Vec<String> {
    let store = engine().store_for_character(s.ch());
    place::accepts(s.ch(), &store, g).into_iter().find(|a| a.tag == tag).map(|a| a.categories).unwrap_or_default()
}

#[test]
fn a_vehicle_takes_mods_weapon_mounts_and_gear() {
    let mut s = session();
    let car = buy(&mut s, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    assert_eq!(kinds(&s, &car), ["mod", "weaponmount", "gear"]);
    // A weapon mount is bought like any other kind (`AddItem`), from its
    // size record; it then takes weapons of its categories.
    let mount = buy(&mut s, "weaponmount", "Standard [SR5]", 0, Some(&car), None);
    assert_eq!(edit::parent(s.ch(), &mount).map(|p| p.get("guid")).as_deref(), Some(car.as_str()));
    assert_eq!(kinds(&s, &mount), ["weapon"]);
    let cats = categories(&s, &mount, "weapon");
    assert!(cats.iter().any(|c| c == "Light Pistols") && !cats.iter().any(|c| c == "Missile Launchers"), "{cats:?}");
    let gun = buy(&mut s, "weapon", "Ares Predator V", 0, Some(&mount), None);
    assert_eq!(edit::parent(s.ch(), &gun).map(|p| p.get("guid")).as_deref(), Some(mount.as_str()));
    // Full: takes nothing more.
    assert!(kinds(&s, &mount).is_empty());
}

#[test]
fn a_weapon_mount_bought_from_the_catalog_matches_the_inspector_command() {
    let mut a = session();
    let car = buy(&mut a, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    let mut b = session();
    let car_b = buy(&mut b, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    let m1 = buy(&mut a, "weaponmount", "Heavy [SR5]", 0, Some(&car), None);
    let before = guids(b.ch());
    let size = engine().store_for_character(b.ch()).doc("vehicles.xml").unwrap().child("weaponmounts").unwrap().children_named("weaponmount").find(|m| m.get("name") == "Heavy [SR5]").unwrap().get("id");
    b.apply(engine(), Command::AddWeaponMount { vehicle: car_b, size }).unwrap();
    let m2 = guids(b.ch()).into_iter().find(|g| !before.contains(g)).unwrap();
    let (e1, e2) = (edit::find(a.ch(), &m1).unwrap(), edit::find(b.ch(), &m2).unwrap());
    for f in ["name", "category", "slots", "cost", "avail", "weaponmountcategories"] {
        assert_eq!(e1.get(f), e2.get(f), "{f}");
    }
    let store = engine().store_for_character(a.ch());
    assert_eq!(edit::total_cost(a.ch(), &store, &m1), edit::total_cost(b.ch(), &store, &m2));
    assert_eq!(a.ch().nuyen, b.ch().nuyen);
}

#[test]
fn a_weapon_takes_accessories_and_an_underbarrel_weapon() {
    let mut s = session();
    let pistol = buy(&mut s, "weapon", "Ares Predator V", 0, None, None);
    assert_eq!(kinds(&s, &pistol), ["accessory"]);
    let rifle = buy(&mut s, "weapon", "FN HAR", 0, None, None);
    assert_eq!(kinds(&s, &rifle), ["accessory", "weapon"]);
    assert_eq!(categories(&s, &rifle, "weapon"), ["Underbarrel Weapons"]);
}

#[test]
fn armor_takes_armor_mods_and_gear() {
    let mut s = session();
    let jacket = buy(&mut s, "armor", "Armor Jacket", 0, None, None);
    assert_eq!(kinds(&s, &jacket), ["armormod", "gear"]);
}

#[test]
fn a_commlink_takes_gear_and_plain_gear_nothing() {
    let mut s = session();
    let link = buy(&mut s, "gear", "Hermes Ikon", 0, None, None);
    assert_eq!(kinds(&s, &link), ["gear"]);
    // A Matrix device: any gear (programs and the like).
    assert!(categories(&s, &link, "gear").is_empty());
    let torch = buy(&mut s, "gear", "Flashlight", 0, None, None);
    assert!(kinds(&s, &torch).is_empty(), "no capacity, no add-ons: no container");
}

#[test]
fn ware_takes_ware_by_subsystem_and_gear_by_allowgear() {
    let mut s = session();
    let arm = buy(&mut s, "cyberware", "Obvious Full Arm", 0, None, Some("Left"));
    assert_eq!(kinds(&s, &arm), ["cyberware", "gear"]);
    assert!(categories(&s, &arm, "cyberware").iter().any(|c| c == "Cyberlimb Accessory"));
    assert_eq!(categories(&s, &arm, "gear"), ["Custom", "Sensors"]);
    let eyes = buy(&mut s, "cyberware", "Cybereyes Basic System", 2, None, None);
    assert_eq!(kinds(&s, &eyes), ["cyberware", "gear"]);
    assert_eq!(categories(&s, &eyes, "gear"), ["Custom", "Sensors"]);
    // Commlink implant: room for a commlink only (`allowgear`), no ware.
    let implant = buy(&mut s, "cyberware", "Commlink", 0, None, None);
    assert_eq!(kinds(&s, &implant), ["gear"]);
    assert_eq!(categories(&s, &implant, "gear"), ["Commlinks"]);
    // The purchase rules check the same categories.
    let store = engine().store_for_character(s.ch());
    let gear = store.doc("gear.xml").unwrap();
    let rec = |name: &str| gear.child("gears").unwrap().children_named("gear").find(|g| g.get("name") == name).unwrap().clone();
    let ikon = rec("Hermes Ikon");
    let torch = rec("Flashlight");
    let at = Dest::Item(implant.clone());
    assert!(place::check(s.ch(), &store, Candidate::Record { tag: "gear", rec: &ikon, rating: 0 }, &at, false).is_ok());
    assert!(place::check(s.ch(), &store, Candidate::Record { tag: "gear", rec: &torch, rating: 0 }, &at, false).is_err());
    let ikon_in = buy(&mut s, "gear", "Hermes Ikon", 0, Some(&implant), None);
    assert_eq!(edit::parent(s.ch(), &ikon_in).map(|p| p.get("guid")).as_deref(), Some(implant.as_str()));
}

#[test]
fn a_weapon_mount_bought_in_career_costs_what_the_inspector_command_costs() {
    let load = || {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Munin_Career.chum5");
        let mut ch = Character::load(&p).unwrap();
        ch.nuyen = 100_000.0;
        Session::with_seed(ch, 3)
    };
    let expenses = |s: &Session| s.ch().doc.child("expenses").map_or(0, |e| e.elements().count());
    let (mut a, mut b) = (load(), load());
    let car_a = buy(&mut a, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    let car_b = buy(&mut b, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    let (na, ea) = (a.ch().nuyen, expenses(&a));
    buy(&mut a, "weaponmount", "Heavy [SR5]", 0, Some(&car_a), None);
    let size = engine().store_for_character(b.ch()).doc("vehicles.xml").unwrap().child("weaponmounts").unwrap().children_named("weaponmount").find(|m| m.get("name") == "Heavy [SR5]").unwrap().get("id");
    b.apply(engine(), Command::AddWeaponMount { vehicle: car_b, size }).unwrap();
    assert!(a.ch().nuyen < na, "career pays for it");
    assert_eq!(a.ch().nuyen, b.ch().nuyen);
    assert_eq!(expenses(&a), ea + 1);
    assert_eq!(expenses(&a), expenses(&b));
}

/// Ware that allows gear by name (`allowgear/gearname`), not only by
/// category: a Built-in Medkit takes the medkits and nothing else.
#[test]
fn ware_takes_gear_allowed_by_name() {
    let mut s = session();
    let arm = buy(&mut s, "cyberware", "Obvious Full Arm", 0, None, Some("Right"));
    let kit = buy(&mut s, "cyberware", "Built-in Medkit", 0, Some(&arm), None);
    let k = kinds(&s, &kit);
    let el = edit::find(s.ch(), &kit).unwrap().clone();
    assert!(k.contains(&"gear"), "{k:?} for {} ({})", el.name, el.to_xml_string().chars().take(400).collect::<String>());
    let medkit = buy(&mut s, "gear", "Medkit", 3, Some(&kit), None);
    assert_eq!(edit::parent(s.ch(), &medkit).map(|p| p.get("guid")), Some(kit.clone()));
    // The placement rules the catalog and Move check: medkits only.
    let store = engine().store_for_character(s.ch());
    let gear = store.doc("gear.xml").unwrap();
    let rec = |n: &str| chummer_core::data::find(&gear, "gears", "gear", n).unwrap().el().clone();
    let at = Dest::Item(kit.clone());
    assert!(place::check(s.ch(), &store, Candidate::Record { tag: "gear", rec: &rec("Medkit"), rating: 3 }, &at, false).is_ok());
    assert!(place::check(s.ch(), &store, Candidate::Record { tag: "gear", rec: &rec("Flashlight"), rating: 0 }, &at, false).is_err(), "a flashlight is not a medkit");
}
