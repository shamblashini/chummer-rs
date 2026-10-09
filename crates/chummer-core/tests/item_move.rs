//! Moving owned items (`Command::MoveItem`, `items::place`): into another
//! item under the purchase rules, to the top level, into a location; the
//! refusals; improvements, undo, the state hash and save/load.

use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::command::{self, Command, RecordRef, Session};
use chummer_core::engine::Engine;
use chummer_core::items::place::{self, Candidate, Dest, Misfit};
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
        name: "Mover".into(),
    };
    Session::with_seed(chargen::create(engine(), &spec).unwrap(), 7)
}

/// Every item guid in the document.
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

/// Buy `name` of kind `tag`; returns the new item's guid (the one whose
/// parent is not new too).
fn buy(s: &mut Session, tag: &str, name: &str, rating: i32, parent: Option<&str>, answer: Option<&str>) -> String {
    let before = guids(s.ch());
    let purchase = Purchase { rating, qty: 1.0, parent: parent.map(str::to_owned), answer: answer.map(str::to_owned), cost_multiplier: 1.0, ..Default::default() };
    s.apply(engine(), Command::AddItem { tag: tag.into(), record: RecordRef { id: String::new(), name: name.into() }, purchase }).unwrap_or_else(|e| panic!("buying {name}: {}", e.reason));
    let new: Vec<String> = guids(s.ch()).into_iter().filter(|g| !before.contains(g)).collect();
    new.iter().find(|g| edit::parent(s.ch(), g).is_none_or(|p| !new.contains(&p.get("guid")))).cloned().unwrap_or_else(|| panic!("{name} was added"))
}

fn mv(s: &mut Session, item: &str, to: Dest) -> Result<(), String> {
    s.apply(engine(), Command::MoveItem { item: item.into(), to }).map(|_| ()).map_err(|e| e.reason)
}

fn parent_of(s: &Session, g: &str) -> Option<String> {
    edit::parent(s.ch(), g).map(|p| p.get("guid"))
}

fn improvements_of(ch: &Character, g: &str) -> Vec<bool> {
    ch.improvements.list.iter().filter(|i| i.source_name.eq_ignore_ascii_case(g)).map(|i| i.enabled).collect()
}

#[test]
fn gear_moves_into_a_container_that_takes_it() {
    let mut s = session();
    let glasses = buy(&mut s, "gear", "Glasses", 2, None, None);
    let goggles = buy(&mut s, "gear", "Goggles", 2, None, None);
    let mag = buy(&mut s, "gear", "Vision Magnification", 0, Some(&glasses), None);
    let flash = buy(&mut s, "gear", "Flashlight", 0, None, None);
    assert_eq!(edit::capacity(s.ch(), &glasses), Some((1.0, 2.0)));
    mv(&mut s, &mag, Dest::Item(goggles.clone())).unwrap();
    assert_eq!(parent_of(&s, &mag).as_deref(), Some(goggles.as_str()));
    assert_eq!(edit::capacity(s.ch(), &glasses), Some((0.0, 2.0)));
    assert_eq!(edit::capacity(s.ch(), &goggles), Some((1.0, 2.0)));
    assert!(s.undo_label().unwrap().starts_with("Moved Vision Magnification into Goggles"), "{:?}", s.undo_label());
    // The same place again changes nothing.
    let v = s.version();
    mv(&mut s, &mag, Dest::Item(goggles.clone())).unwrap();
    assert_eq!(s.version(), v, "no change, no undo step");
    // A vision enhancement only exists in a vision device.
    assert_eq!(mv(&mut s, &mag, Dest::Top).unwrap_err(), "Vision Magnification has to be installed in another item.");
    // Glasses take vision enhancements only.
    assert_eq!(mv(&mut s, &flash, Dest::Item(glasses.clone())).unwrap_err(), "Glasses only takes Vision Enhancements, Sensors, Custom.");
    // Not into itself or what is inside it.
    assert_eq!(mv(&mut s, &goggles, Dest::Item(mag.clone())).unwrap_err(), "Goggles can't go inside itself.");
    assert_eq!(mv(&mut s, &glasses, Dest::Item(glasses.clone())).unwrap_err(), "Glasses can't go inside itself.");
    // A flashlight is no container.
    assert_eq!(mv(&mut s, &mag, Dest::Item(flash.clone())).unwrap_err(), "Vision Magnification can't be installed in Flashlight.");
    // Out of a container into one without an addon list.
    let collar = buy(&mut s, "gear", "Sensor Collar", 0, None, None);
    let medkit = buy(&mut s, "gear", "Medkit", 1, Some(&collar), None);
    mv(&mut s, &medkit, Dest::Top).unwrap();
    assert_eq!(parent_of(&s, &medkit), None);
    assert!(s.ch().items("gears", "gear").iter().any(|g| g.get("guid") == medkit));
    mv(&mut s, &medkit, Dest::Item(collar.clone())).unwrap();
    assert_eq!(parent_of(&s, &medkit).as_deref(), Some(collar.as_str()));
}

#[test]
fn capacity_counts_what_is_already_inside_at_its_rating() {
    let mut s = session();
    let glasses = buy(&mut s, "gear", "Glasses", 1, None, None);
    let goggles = buy(&mut s, "gear", "Goggles", 1, None, None);
    buy(&mut s, "gear", "Vision Magnification", 0, Some(&glasses), None);
    let flare = buy(&mut s, "gear", "Flare Compensation", 0, Some(&goggles), None);
    let e = mv(&mut s, &flare, Dest::Item(glasses.clone())).unwrap_err();
    assert_eq!(e, "Glasses is full (1/1 capacity used; Flare Compensation needs 1).");
    // The catalog's check for a record gives the same answer.
    let doc = engine().store.doc("gear.xml").unwrap();
    let rec = chummer_core::data::find(&doc, "gears", "gear", "Thermographic Vision").unwrap();
    let r = place::check(s.ch(), &engine().store, Candidate::Record { tag: "gear", rec: rec.el(), rating: 0 }, &Dest::Item(glasses.clone()), true);
    assert_eq!(r, Err(Misfit::Full { used: 1.0, total: 1.0, need: 1.0 }));
    // Without "Enforce capacity" it fits.
    assert!(place::check(s.ch(), &engine().store, Candidate::Owned(&flare), &Dest::Item(glasses.clone()), false).is_ok());
    // A higher rating has room (capacity = Rating).
    s.apply(engine(), Command::SetItemRating { guid: glasses.clone(), rating: 2 }).unwrap();
    mv(&mut s, &flare, Dest::Item(glasses.clone())).unwrap();
    assert_eq!(edit::capacity(s.ch(), &glasses), Some((2.0, 2.0)));
}

#[test]
fn gear_moves_between_locations() {
    let mut s = session();
    let flash = buy(&mut s, "gear", "Flashlight", 0, None, None);
    let medkit = buy(&mut s, "gear", "Medkit", 3, None, None);
    s.apply(engine(), Command::AddItemLocation { guid: flash.clone(), name: "Car".into() }).unwrap();
    let car = edit::locations(s.ch(), &flash).into_iter().find(|(_, n)| n == "Car").unwrap().0;
    mv(&mut s, &medkit, Dest::Location(car.clone())).unwrap();
    assert_eq!(edit::find(s.ch(), &medkit).unwrap().get("location"), car);
    assert_eq!(place::current(s.ch(), &medkit), Some(Dest::Location(car.clone())));
    assert!(s.undo_label().unwrap().contains("to Car"), "{:?}", s.undo_label());
    mv(&mut s, &medkit, Dest::Top).unwrap();
    assert_eq!(edit::find(s.ch(), &medkit).unwrap().get("location"), "");
    // Into a container from a location: the location goes.
    let collar = buy(&mut s, "gear", "Sensor Collar", 0, None, None);
    mv(&mut s, &medkit, Dest::Location(car.clone())).unwrap();
    mv(&mut s, &medkit, Dest::Item(collar.clone())).unwrap();
    assert_eq!(edit::find(s.ch(), &medkit).unwrap().get("location"), "");
    // A nested item straight into a location.
    mv(&mut s, &medkit, Dest::Location(car.clone())).unwrap();
    assert_eq!(parent_of(&s, &medkit), None);
    assert_eq!(place::current(s.ch(), &medkit), Some(Dest::Location(car)));
    assert!(mv(&mut s, &medkit, Dest::Location("no-such-location".into())).unwrap_err().contains("can't be put in a location"));
    // What needs a parent has no place in a location.
    let glasses = buy(&mut s, "gear", "Glasses", 2, None, None);
    let mag = buy(&mut s, "gear", "Vision Magnification", 0, Some(&glasses), None);
    let loc = edit::locations(s.ch(), &flash)[0].0.clone();
    assert!(mv(&mut s, &mag, Dest::Location(loc)).unwrap_err().contains("has to be installed in another item"));
}

#[test]
fn ware_follows_the_purchase_rules_of_limbs() {
    let mut s = session();
    let rig = buy(&mut s, "cyberware", "Control Rig", 1, None, None);
    let jack = buy(&mut s, "cyberware", "Datajack", 0, None, None);
    assert_eq!(mv(&mut s, &jack, Dest::Item(rig.clone())).unwrap_err(), "Datajack can't be installed in Control Rig.");
    let left = buy(&mut s, "cyberware", "Obvious Full Arm", 0, None, Some("Left"));
    let right = buy(&mut s, "cyberware", "Obvious Full Arm", 0, None, Some("Right"));
    // A datajack has no [capacity]: it does not go into an arm either.
    assert_eq!(mv(&mut s, &jack, Dest::Item(left.clone())).unwrap_err(), "Datajack can't be installed in Obvious Full Arm.");
    // Ware with a [capacity] goes into a limb; its essence then counts
    // as the limb's (none of its own).
    let smug = buy(&mut s, "cyberware", "Smuggling Compartment", 0, None, None);
    let ess_before = engine().sheet(s.ch()).essence;
    mv(&mut s, &smug, Dest::Item(left.clone())).unwrap();
    let ess_inside = engine().sheet(s.ch()).essence;
    assert!(ess_inside > ess_before + 0.19, "the compartment's 0.2 essence is part of the arm now: {ess_before} → {ess_inside}");
    assert_eq!(edit::capacity(s.ch(), &left), Some((2.0, 15.0)));
    // Between limbs; out again.
    mv(&mut s, &smug, Dest::Item(right.clone())).unwrap();
    assert_eq!(parent_of(&s, &smug).as_deref(), Some(right.as_str()));
    mv(&mut s, &smug, Dest::Top).unwrap();
    assert!((engine().sheet(s.ch()).essence - ess_before).abs() < 1e-9, "back to its own essence");
    // A holster only exists inside a limb.
    let holster = buy(&mut s, "cyberware", "Cyber Holster", 0, Some(&left), None);
    assert_eq!(mv(&mut s, &holster, Dest::Top).unwrap_err(), "Cyber Holster has to be installed in another item.");
    mv(&mut s, &holster, Dest::Item(right.clone())).unwrap();
    // A gyromount needs a full arm: not into the control rig or eyes.
    let eyes = buy(&mut s, "cyberware", "Cybereyes Basic System", 2, None, None);
    let vm = buy(&mut s, "cyberware", "Vision Magnification", 0, Some(&eyes), None);
    assert_eq!(mv(&mut s, &vm, Dest::Item(left.clone())).unwrap_err(), "Obvious Full Arm only takes Bodyware, Cosmetic Enhancement, Cyberlimb, Cyberlimb Enhancement, Cyberlimb Accessory, Cyber Implant Weapon, Headware, Nanocybernetics.");
    // The capacity of the right arm: 15, holster 5; a gyromount 8 fits,
    // a second does not.
    let g1 = buy(&mut s, "cyberware", "Cyberarm Gyromount", 0, Some(&left), None);
    let g2 = buy(&mut s, "cyberware", "Cyberarm Gyromount", 0, Some(&left), Some("x"));
    mv(&mut s, &g1, Dest::Item(right.clone())).unwrap();
    assert_eq!(mv(&mut s, &g2, Dest::Item(right.clone())).unwrap_err(), "Obvious Full Arm is full (13/15 capacity used; Cyberarm Gyromount needs 8).");
}

#[test]
fn ware_moved_into_a_limb_takes_its_grade_and_side() {
    let mut s = session();
    let p = Purchase { qty: 1.0, grade: Some("Alphaware".into()), answer: Some("Right".into()), cost_multiplier: 1.0, ..Default::default() };
    s.apply(engine(), Command::AddItem { tag: "cyberware".into(), record: RecordRef { id: String::new(), name: "Obvious Full Arm".into() }, purchase: p }).unwrap();
    let arm = s.ch().items("cyberwares", "cyberware").iter().find(|w| w.get("name") == "Obvious Full Arm").unwrap().get("guid");
    let smug = buy(&mut s, "cyberware", "Smuggling Compartment", 0, None, None);
    assert_eq!(edit::find(s.ch(), &smug).unwrap().get("grade"), "Standard");
    let cost_before = edit::total_cost(s.ch(), &engine().store, &arm);
    mv(&mut s, &smug, Dest::Item(arm.clone())).unwrap();
    assert_eq!(edit::find(s.ch(), &smug).unwrap().get("grade"), "Alphaware", "bought into the arm it would be Alphaware");
    assert!(edit::total_cost(s.ch(), &engine().store, &arm) > cost_before, "the arm's cost now holds the compartment");
}

#[test]
fn armor_mods_move_between_armors_and_keep_their_improvements() {
    let mut s = session();
    let jacket = buy(&mut s, "armor", "Armor Jacket", 0, None, None);
    let coat = buy(&mut s, "armor", "Lined Coat", 0, None, None);
    let fire = buy(&mut s, "armormod", "Fire Resistance", 2, Some(&jacket), None);
    let imps = improvements_of(s.ch(), &fire);
    assert!(!imps.is_empty() && imps.iter().all(|on| *on), "{imps:?}");
    let sheet = engine().sheet(s.ch());
    mv(&mut s, &fire, Dest::Item(coat.clone())).unwrap();
    assert_eq!(parent_of(&s, &fire).as_deref(), Some(coat.as_str()));
    assert_eq!(improvements_of(s.ch(), &fire), imps, "the bonus stays, keyed by the mod's guid");
    assert_eq!(engine().sheet(s.ch()).armor, sheet.armor);
    assert_eq!(edit::capacity(s.ch(), &coat), Some((2.0, 9.0)), "FixedValues([1],[2],…) at rating 2");
    assert_eq!(mv(&mut s, &fire, Dest::Top).unwrap_err(), "Fire Resistance has to be installed in another item.");
    // Into unequipped armor: its improvements go off, as unequipping
    // the armor does; back: on again.
    s.apply(engine(), Command::SetItemEquipped { guid: jacket.clone(), on: false }).unwrap();
    mv(&mut s, &fire, Dest::Item(jacket.clone())).unwrap();
    assert!(improvements_of(s.ch(), &fire).iter().all(|on| !*on), "off in unequipped armor");
    mv(&mut s, &fire, Dest::Item(coat.clone())).unwrap();
    assert_eq!(improvements_of(s.ch(), &fire), imps, "on again");
    // Armor is no armor mod.
    assert_eq!(mv(&mut s, &jacket, Dest::Item(coat.clone())).unwrap_err(), "Armor Jacket can't be installed in Lined Coat.");
}

#[test]
fn accessories_need_a_free_mount() {
    let mut s = session();
    let a = buy(&mut s, "weapon", "Colt America L36", 0, None, None);
    let b = buy(&mut s, "weapon", "Ares Alpha", 0, None, None);
    let laser = buy(&mut s, "accessory", "Laser Sight", 0, Some(&a), None);
    let mount = edit::find(s.ch(), &laser).unwrap().get("mount");
    assert!(mount == "Top" || mount == "Under", "{mount}");
    mv(&mut s, &laser, Dest::Item(b.clone())).unwrap();
    assert_eq!(parent_of(&s, &laser).as_deref(), Some(b.as_str()));
    assert!(!edit::find(s.ch(), &laser).unwrap().get("mount").is_empty());
    // A scope on the pistol's top: no mount left there for the laser
    // (the L36 has a barrel and a top mount).
    buy(&mut s, "accessory", "Imaging Scope", 0, Some(&a), None);
    assert_eq!(mv(&mut s, &laser, Dest::Item(a.clone())).unwrap_err(), "Colt America L36 has no free mount for Laser Sight.");
    assert_eq!(mv(&mut s, &laser, Dest::Top).unwrap_err(), "Laser Sight has to be installed in another item.");
    // A rifle is no underbarrel weapon, and the pistol has no under mount.
    assert!(mv(&mut s, &b, Dest::Item(a.clone())).is_err());
}

#[test]
fn vehicle_mods_move_between_vehicles_but_not_to_other_lists() {
    let mut s = session();
    let car = buy(&mut s, "vehicle", "Ford Americar (Sedan)", 0, None, None);
    let van = buy(&mut s, "vehicle", "Ford Americar (2050)", 0, None, None);
    let susp = buy(&mut s, "mod", "Off-Road Suspension", 0, Some(&car), None);
    let used = |s: &Session, v: &str| edit::capacity(s.ch(), v).unwrap().0;
    let (car_used, van_used) = (used(&s, &car), used(&s, &van));
    mv(&mut s, &susp, Dest::Item(van.clone())).unwrap();
    assert_eq!(used(&s, &car), car_used - 2.0);
    assert_eq!(used(&s, &van), van_used + 2.0);
    assert_eq!(mv(&mut s, &susp, Dest::Top).unwrap_err(), "Off-Road Suspension has to be installed in another item.");
    // Gear of a vehicle stays with vehicles.
    let medkit = buy(&mut s, "gear", "Medkit", 1, Some(&car), None);
    mv(&mut s, &medkit, Dest::Item(van.clone())).unwrap();
    assert_eq!(mv(&mut s, &medkit, Dest::Top).unwrap_err(), "Medkit can only be moved within its own list.");
    let glasses = buy(&mut s, "gear", "Glasses", 2, None, None);
    assert_eq!(mv(&mut s, &glasses, Dest::Item(van.clone())).unwrap_err(), "Glasses can only be moved within its own list.");
}

#[test]
fn included_items_stay_with_their_parent() {
    let mut s = session();
    let w = buy(&mut s, "weapon", "Ares Predator V", 0, None, None);
    let other = buy(&mut s, "weapon", "Ares Alpha", 0, None, None);
    let included: Vec<String> = edit::children(s.ch(), &w).into_iter().map(|(g, _, _)| g).filter(|g| edit::is_included(s.ch(), g)).collect();
    assert!(!included.is_empty(), "the Predator comes with a smartgun system");
    let e = mv(&mut s, &included[0], Dest::Item(other)).unwrap_err();
    assert!(e.contains("comes with its parent item"), "{e}");
}

#[test]
fn undo_restores_exactly_and_saves_round_trip() {
    let mut s = session();
    let glasses = buy(&mut s, "gear", "Glasses", 2, None, None);
    let mag = buy(&mut s, "gear", "Vision Magnification", 0, None, None);
    let jacket = buy(&mut s, "armor", "Armor Jacket", 0, None, None);
    let coat = buy(&mut s, "armor", "Lined Coat", 0, None, None);
    let fire = buy(&mut s, "armormod", "Fire Resistance", 1, Some(&jacket), None);
    let h0 = s.state_hash();
    let xml0 = s.ch().to_xml_string();
    mv(&mut s, &mag, Dest::Item(glasses.clone())).unwrap();
    let h1 = s.state_hash();
    mv(&mut s, &fire, Dest::Item(coat.clone())).unwrap();
    let h2 = s.state_hash();
    assert!(h0 != h1 && h1 != h2);
    // Save and load: the same state, the items where they were put.
    let xml = s.ch().to_xml_string();
    let back = Character::from_str(&xml).unwrap();
    assert_eq!(command::state_hash(&back), h2);
    assert_eq!(edit::parent(&back, &mag).map(|p| p.get("guid")).as_deref(), Some(glasses.as_str()));
    assert_eq!(improvements_of(&back, &fire), improvements_of(s.ch(), &fire));
    assert_eq!(command::state_hash(&command::restore(&command::snapshot(s.ch())).unwrap()), h2);
    s.undo().unwrap();
    assert_eq!(s.state_hash(), h1);
    s.undo().unwrap();
    assert_eq!(s.state_hash(), h0);
    assert_eq!(s.ch().to_xml_string(), xml0, "undo restores the document exactly");
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(s.state_hash(), h2);
    // Replaying the envelopes on the start state gives the same state.
    let mut again = Session::with_seed(Character::from_str(&xml0).unwrap(), 7);
    for env in s.envelopes().into_iter().skip(5) {
        again.apply_envelope(engine(), env).unwrap();
    }
    assert_eq!(again.state_hash(), h2);
}

#[test]
fn the_catalog_check_for_records_uses_the_same_rules() {
    let mut s = session();
    let left = buy(&mut s, "cyberware", "Obvious Full Arm", 0, None, Some("Left"));
    let rig = buy(&mut s, "cyberware", "Control Rig", 1, None, None);
    let doc = engine().store.doc("cyberware.xml").unwrap();
    let rec = |n: &str| chummer_core::data::find(&doc, "cyberwares", "cyberware", n).unwrap();
    let check = |n: &str, to: &str| place::check(s.ch(), &engine().store, Candidate::Record { tag: "cyberware", rec: rec(n).el(), rating: 0 }, &Dest::Item(to.into()), true);
    assert_eq!(check("Cyberarm Gyromount", &left), Ok(()));
    assert_eq!(check("Cyberarm Gyromount", &rig), Err(Misfit::Kind));
    assert_eq!(check("Datajack", &left), Err(Misfit::Kind));
    assert_eq!(Misfit::Kind.message("Datajack", "Control Rig"), "Datajack can't be installed in Control Rig.");
    // A gyromount needs a full arm (parentdetails), even with room.
    let eyes = buy(&mut s, "cyberware", "Cybereyes Basic System", 4, None, None);
    let check = |n: &str, to: &str| place::check(s.ch(), &engine().store, Candidate::Record { tag: "cyberware", rec: rec(n).el(), rating: 0 }, &Dest::Item(to.into()), true);
    assert!(check("Cyberarm Gyromount", &eyes).is_err());
    assert_eq!(check("Vision Magnification", &eyes), Ok(()));
    // Records that need a parent are refused at the top level.
    assert_eq!(place::check(s.ch(), &engine().store, Candidate::Record { tag: "cyberware", rec: rec("Cyber Holster").el(), rating: 0 }, &Dest::Top, true), Err(Misfit::NeedsParent));
}

#[test]
fn parentdetails_filters() {
    use chummer_core::xml::parse;
    let arm = parse("<cyberware><name>Obvious Full Arm</name><category>Cyberlimb</category></cyberware>").unwrap();
    let leg = parse("<cyberware><name>Obvious Lower Leg</name><category>Cyberlimb</category></cyberware>").unwrap();
    let op = parse(r#"<parentdetails><OR><name operation="contains">Full Arm</name><name operation="contains">Lower Arm</name></OR></parentdetails>"#).unwrap();
    assert!(place::filter_matches(Some(&arm), &op, false));
    assert!(!place::filter_matches(Some(&leg), &op, false));
    let none = parse("<parentdetails><NONE /></parentdetails>").unwrap();
    assert!(place::filter_matches(None, &none, false));
    assert!(!place::filter_matches(Some(&arm), &none, false));
    let not = parse(r#"<parentdetails><category NOT="">Cyberlimb</category></parentdetails>"#).unwrap();
    assert!(!place::filter_matches(Some(&arm), &not, false));
    let and = parse("<parentdetails><name>Cyberdeck</name><category>Headware</category></parentdetails>").unwrap();
    let deck = parse("<cyberware><name>Cyberdeck</name><category>Headware</category></cyberware>").unwrap();
    assert!(place::filter_matches(Some(&deck), &and, false));
    assert!(!place::filter_matches(Some(&arm), &and, false));
}

#[test]
fn underbarrel_weapons_move_out_and_into_another_weapon() {
    let mut s = session();
    let ak = buy(&mut s, "weapon", "AK-97", 0, None, None);
    let colt = buy(&mut s, "weapon", "Colt M23", 0, None, None);
    let gl = buy(&mut s, "weapon", "Underbarrel Grenade Launcher", 0, Some(&ak), None);
    assert_eq!(parent_of(&s, &gl).as_deref(), Some(ak.as_str()));
    let can_take = |s: &Session, w: &str| edit::child_kinds(s.ch(), w).iter().any(|k| k.tag == "weapon");
    assert!(!can_take(&s, &ak), "one underbarrel per weapon");
    mv(&mut s, &gl, Dest::Top).unwrap();
    let host = edit::find(s.ch(), &ak).unwrap();
    assert!(host.child("underbarrel").is_none(), "no empty <underbarrel> is left behind");
    assert!(can_take(&s, &ak), "the rifle takes an underbarrel again");
    assert_eq!(edit::find(s.ch(), &gl).unwrap().get("parentid"), "", "a top-level weapon is no longer granted");
    assert!(!edit::is_included(s.ch(), &gl));
    mv(&mut s, &gl, Dest::Item(colt.clone())).unwrap();
    let host = edit::find(s.ch(), &colt).unwrap();
    assert_eq!(host.children_named("underbarrel").count(), 1);
    assert_eq!(parent_of(&s, &gl).as_deref(), Some(colt.as_str()));
    // A rifle is no underbarrel weapon.
    assert!(mv(&mut s, &ak, Dest::Item(colt.clone())).is_err());
    mv(&mut s, &gl, Dest::Item(ak.clone())).unwrap();
    assert!(edit::find(s.ch(), &colt).unwrap().child("underbarrel").is_none());
    assert_eq!(edit::find(s.ch(), &ak).unwrap().children_named("underbarrel").count(), 1);
}

#[test]
fn buying_into_a_location_is_one_command() {
    let mut s = session();
    let flash = buy(&mut s, "gear", "Flashlight", 0, None, None);
    s.apply(engine(), Command::AddItemLocation { guid: flash.clone(), name: "Car".into() }).unwrap();
    let car = edit::locations(s.ch(), &flash)[0].0.clone();
    let h0 = s.state_hash();
    let v = s.version();
    let purchase = Purchase { qty: 1.0, cost_multiplier: 1.0, location: Some(car.clone()), ..Default::default() };
    s.apply(engine(), Command::AddItem { tag: "gear".into(), record: RecordRef { id: String::new(), name: "Medkit".into() }, purchase }).unwrap();
    assert_eq!(s.version(), v + 1, "one command");
    let medkit = s.ch().items("gears", "gear").iter().find(|g| g.get("name") == "Medkit").unwrap().get("guid");
    assert_eq!(place::current(s.ch(), &medkit), Some(Dest::Location(car)));
    s.undo().unwrap();
    assert_eq!(s.state_hash(), h0, "one undo takes it back");
    // A location that is not there: refused, nothing changes.
    let purchase = Purchase { qty: 1.0, cost_multiplier: 1.0, location: Some("gone".into()), ..Default::default() };
    let e = s.apply(engine(), Command::AddItem { tag: "gear".into(), record: RecordRef { id: String::new(), name: "Medkit".into() }, purchase }).unwrap_err();
    assert!(e.reason.contains("can't be put in that location"), "{}", e.reason);
    assert_eq!(s.state_hash(), h0);
}
