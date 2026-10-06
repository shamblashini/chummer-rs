//! Career actions: burning Edge, the second-MAG limits, focus binding and
//! technique undo, spirit fettering, martial arts, metamagics, critter
//! powers, groups, quickening, Edge and street cred.

use std::path::Path;

use chummer_core::calc::{self, Rules};
use chummer_core::career::{self, CareerError, CareerRules, InitiationOptions, KarmaExpenseType, ManualExpense};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::magic::{martialart, spirit};

fn load(name: &str) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.chum5"));
    Character::load(&p).unwrap()
}

fn give_karma(ch: &mut Character, amount: f64) {
    let m = ManualExpense { amount, reason: "Run".into(), ..Default::default() };
    career::karma_gained(ch, &CareerRules::default(), &m).unwrap();
}

fn entry_by_reason(ch: &Character, reason: &str) -> career::ExpenseEntry {
    career::entries(ch).into_iter().find(|e| e.reason == reason).unwrap_or_else(|| panic!("no entry {reason:?}"))
}

fn rules(engine: &Engine, ch: &Character) -> Rules {
    CareerRules::for_character(engine, ch).rules
}

/// A saved copy of the character, loaded back.
fn reload(ch: &mut Character) -> Character {
    let dir = std::env::temp_dir().join(format!("chummer-career-actions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{}.chum5", chummer_core::items::new_guid()));
    ch.save(&p).unwrap();
    let back = Character::load(&p).unwrap();
    let _ = std::fs::remove_file(&p);
    back
}

#[test]
fn burning_edge_takes_karma_then_base_then_the_minimum() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    let r = rules(&engine, &ch);
    {
        let a = ch.attribute_mut("EDG").unwrap();
        a.base = 1;
        a.karma = 1;
    }
    let entries = career::entries(&ch).len();
    let karma = ch.karma;
    let start = calc::attribute_values(&ch, "EDG", &r);
    career::burn_edge(&mut ch, &engine).unwrap();
    assert_eq!(ch.attribute("EDG").unwrap().karma, 0);
    career::burn_edge(&mut ch, &engine).unwrap();
    assert_eq!(ch.attribute("EDG").unwrap().base, 0);
    // Now the metatype minimum burns.
    let min = start.metatype_min;
    assert!(min >= 1);
    for burned in 1..=min {
        career::burn_edge(&mut ch, &engine).unwrap();
        let imps: Vec<_> = ch.improvements.list.iter().filter(|i| i.source == "BurnedEdge").collect();
        assert_eq!(imps.len(), 1, "one BurnedEdge improvement, replaced each time");
        assert_eq!((imps[0].improved_name.as_str(), imps[0].kind.as_str(), imps[0].min, imps[0].rating), ("EDG", "Attribute", f64::from(-burned), 1));
        assert_eq!(calc::attribute_values(&ch, "EDG", &r).value, min - burned);
    }
    assert!(matches!(career::burn_edge(&mut ch, &engine), Err(CareerError::Refused(_))));
    // No expense and no refund.
    assert_eq!(career::entries(&ch).len(), entries);
    assert_eq!(ch.karma, karma);
    // The improvement is saved as Chummer saves it.
    let back = reload(&mut ch);
    assert!(back.improvements.list.iter().any(|i| i.source == "BurnedEdge" && i.min == f64::from(-min)));
    assert_eq!(calc::attribute_values(&back, "EDG", &r).value, 0);
}

#[test]
fn spending_and_regaining_edge() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    let edg = calc::attribute_values(&ch, "EDG", &rules(&engine, &ch)).total;
    ch.doc.set_child_text("edgeused", "0");
    assert!(career::regain_edge(&mut ch).is_err());
    for n in 1..=edg {
        assert_eq!(career::spend_edge(&mut ch, &engine).unwrap(), n);
    }
    assert!(career::spend_edge(&mut ch, &engine).is_err());
    assert_eq!(career::regain_edge(&mut ch).unwrap(), edg - 1);
}

#[test]
fn burning_street_cred() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    let before = career::reputation_for(&engine, &ch);
    assert!(before.street_cred >= 2);
    career::burn_street_cred(&mut ch, &engine).unwrap();
    let after = career::reputation_for(&engine, &ch);
    assert_eq!(after.burnt_street_cred, before.burnt_street_cred + 2);
    assert_eq!(after.street_cred, before.street_cred - 2);
}

/// Turn the second-MAG house rule on for every preset.
fn second_mag(engine: &mut Engine, on: bool) {
    for p in &mut engine.settings.presets {
        p.raw.set_child_text("mysadeptsecondmagattribute", if on { "True" } else { "False" });
    }
}

#[test]
fn mystic_adept_second_mag_limits_initiation() {
    let mut engine = Engine::load().unwrap();
    let mut ch = load("Soma (Career)");
    assert!(ch.is_adept() && ch.is_magician());
    give_karma(&mut ch, 200.0);
    let r = rules(&engine, &ch);
    // MAG has room; MAGAdept is as low as it goes, and initiation (with
    // the house rule off) catches up with it.
    ch.attribute_mut("MAG").unwrap().karma += 6;
    let a = ch.attribute_mut("MAGAdept").unwrap();
    a.base = 0;
    a.karma = 0;
    let adept = calc::attribute_values(&ch, "MAGAdept", &r).total;
    while career::grade_count(&ch, false) < adept {
        career::add_initiation_grade(&mut ch, &engine, InitiationOptions::default()).unwrap();
    }
    let grade = career::grade_count(&ch, false);
    assert_eq!(calc::attribute_values(&ch, "MAGAdept", &r).total, grade);
    assert!(calc::attribute_values(&ch, "MAG", &r).total > grade);
    second_mag(&mut engine, true);
    assert!(matches!(career::add_initiation_grade(&mut ch, &engine, InitiationOptions::default()), Err(CareerError::AtMaximum(_))));
    second_mag(&mut engine, false);
    career::add_initiation_grade(&mut ch, &engine, InitiationOptions::default()).unwrap();
    assert_eq!(career::grade_count(&ch, false), grade + 1);
}

/// An unbound focus of a career character with room to bind it.
fn focus_character(engine: &Engine) -> (Character, String) {
    let mut ch = load("Gangerbean");
    ch.created = true;
    ch.improvements.career = true;
    give_karma(&mut ch, 50.0);
    ch.attribute_mut("MAG").unwrap().karma = 4;
    let _ = engine;
    let gear = ch.items("gears", "gear").into_iter().find(|g| g.get("category") == "Foci" && !ch.items("foci", "focus").iter().any(|f| f.get("gearid") == g.get("guid"))).unwrap().get("guid");
    (ch, gear)
}

#[test]
fn focus_binding_limits_and_undo() {
    let engine = Engine::load().unwrap();
    let (mut ch, gear) = focus_character(&engine);
    // MAG 1: one focus is bound already, so a second one is refused.
    ch.attribute_mut("MAG").unwrap().karma = 0;
    let karma = ch.karma;
    assert!(matches!(career::bind_focus(&mut ch, &engine, &gear), Err(CareerError::Refused(_))));
    assert_eq!(ch.karma, karma);
    ch.attribute_mut("MAG").unwrap().karma = 4;
    let foci = ch.items("foci", "focus").len();
    let imps = ch.improvements.list.len();
    let focus = career::bind_focus(&mut ch, &engine, &gear).unwrap();
    let g = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == gear).unwrap().clone();
    assert_eq!(g.get("bonded"), "True");
    let e = career::entries(&ch).into_iter().find(|e| e.undo.as_ref().is_some_and(|u| u.object_id == focus)).unwrap();
    assert!(e.reason.starts_with(&format!("Bound {}", g.get("name"))), "{}", e.reason);
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::BindFocus);
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, karma);
    assert_eq!(ch.items("foci", "focus").len(), foci);
    let g = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == gear).unwrap();
    assert_eq!(g.get("bonded"), "False");
    assert_eq!(ch.improvements.list.len(), imps);
    assert!(career::find_entry(&ch, &e.guid).is_none());
}

#[test]
fn undo_of_a_legacy_focus_entry_by_gear_id() {
    let engine = Engine::load().unwrap();
    let (mut ch, gear) = focus_character(&engine);
    let karma = ch.karma;
    let focus = career::bind_focus(&mut ch, &engine, &gear).unwrap();
    let e = career::entries(&ch).into_iter().find(|e| e.undo.as_ref().is_some_and(|u| u.object_id == focus)).unwrap();
    // Old Chummer versions saved the gear's guid in the undo entry.
    let mut legacy = e.clone();
    legacy.undo.as_mut().unwrap().object_id = gear.clone();
    career::remove_entry(&mut ch, &e.guid);
    career::push_entry(&mut ch, &legacy);
    career::undo_expense(&mut ch, &engine, &legacy.guid).unwrap();
    assert_eq!(ch.karma, karma);
    assert!(!ch.items("foci", "focus").iter().any(|f| f.get("gearid") == gear));
}

#[test]
fn technique_undo_removes_the_technique() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("martialarts.xml").unwrap();
    let art = data::find(&doc, "martialarts", "martialart", "Aikido").unwrap();
    let names = martialart::technique_names(art);
    let guid = martialart::add(&mut ch, &engine.store, art, Some(&names[0]));
    let karma = ch.karma;
    let t = career::learn_technique(&mut ch, &engine, &engine.store, &guid, &names[1]).unwrap();
    assert_eq!(ch.karma, karma - 5);
    let e = entry_by_reason(&ch, &format!("Learned Technique {}", names[1]));
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, karma);
    let a = ch.items("martialarts", "martialart").into_iter().find(|a| a.get("guid") == guid).unwrap();
    let left: Vec<String> = a.child("martialarttechniques").unwrap().children_named("martialarttechnique").map(|t| t.get("name")).collect();
    assert_eq!(left, vec![names[0].clone()]);
    assert!(!ch.improvements.list.iter().any(|i| i.source_name == t));
}

#[test]
fn martial_art_is_logged_and_undone() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("martialarts.xml").unwrap();
    let art = data::find(&doc, "martialarts", "martialart", "Aikido").unwrap();
    let cost = art.el().get_i32("cost").unwrap_or(7);
    let karma = ch.karma;
    let guid = career::learn_martial_art(&mut ch, &engine, art, None).unwrap();
    assert_eq!(ch.karma, karma - cost);
    let e = entry_by_reason(&ch, "Learned Martial Art Aikido");
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::AddMartialArt);
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, karma);
    assert!(!ch.items("martialarts", "martialart").iter().any(|a| a.get("guid") == guid));
}

/// A spirit of the character's own kind (added from traditions.xml).
fn add_spirit(ch: &mut Character, engine: &Engine, force: i32, services: i32) -> String {
    let doc = engine.store.doc("traditions.xml").unwrap();
    let rec = data::find(&doc, "spirits", "spirit", "Spirit of Air").unwrap();
    spirit::add(ch, rec, force, services, true)
}

#[test]
fn fettering_a_spirit_costs_force_times_three() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let r = rules(&engine, &ch);
    let mag = calc::attribute_values(&ch, "MAG", &r).total;
    let guid = add_spirit(&mut ch, &engine, 4, 2);
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == guid).unwrap().clone();
    assert_eq!(career::fettering_karma_cost(&engine, &ch, &s), 12);
    let karma = ch.karma;
    let entry = career::set_spirit_fettered(&mut ch, &engine, &guid, true).unwrap().unwrap();
    assert_eq!(ch.karma, karma - 12);
    let e = career::find_entry(&ch, &entry).unwrap();
    assert_eq!(e.reason, "Fettered a Spirit Spirit of Air");
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::SpiritFettering);
    assert_eq!(e.undo.as_ref().unwrap().object_id, guid);
    // A fettered spirit costs a point of MAG.
    let imp = ch.improvements.list.iter().find(|i| i.source == "SpiritFettering").unwrap();
    assert_eq!((imp.improved_name.as_str(), imp.kind.as_str(), imp.aug, imp.rating), ("MAG", "Attribute", -1.0, 1));
    assert_eq!(calc::attribute_values(&ch, "MAG", &r).total, mag - 1);
    // Only one fettered spirit.
    let other = add_spirit(&mut ch, &engine, 2, 1);
    assert!(matches!(career::set_spirit_fettered(&mut ch, &engine, &other, true), Err(CareerError::Refused(_))));
    // Releasing refunds nothing and gives MAG back.
    let karma = ch.karma;
    assert_eq!(career::set_spirit_fettered(&mut ch, &engine, &guid, false).unwrap(), None);
    assert_eq!(ch.karma, karma);
    assert!(!ch.improvements.list.iter().any(|i| i.source == "SpiritFettering"));
    assert_eq!(calc::attribute_values(&ch, "MAG", &r).total, mag);
}

/// Undo refunds the karma and releases the spirit, giving the point of
/// Magic back (LB-01; Chummer leaves the spirit fettered).
#[test]
fn fettering_undo_releases_the_spirit() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let r = rules(&engine, &ch);
    let mag = calc::attribute_values(&ch, "MAG", &r).total;
    let guid = add_spirit(&mut ch, &engine, 3, 1);
    let karma = ch.karma;
    let entry = career::set_spirit_fettered(&mut ch, &engine, &guid, true).unwrap().unwrap();
    career::undo_expense(&mut ch, &engine, &entry).unwrap();
    assert_eq!(ch.karma, karma);
    assert!(career::find_entry(&ch, &entry).is_none());
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == guid).unwrap();
    assert_eq!(s.get("fettered"), "False");
    assert!(!ch.improvements.list.iter().any(|i| i.source == "SpiritFettering"));
    assert_eq!(calc::attribute_values(&ch, "MAG", &r).total, mag);
    // The spirit can be fettered again.
    assert!(career::set_spirit_fettered(&mut ch, &engine, &guid, true).unwrap().is_some());
}

/// Undo after the spirit was deleted still drops a stale MAG -1, but
/// leaves the penalty of another fettered spirit alone.
#[test]
fn fettering_undo_of_a_deleted_spirit() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 80.0);
    let first = add_spirit(&mut ch, &engine, 2, 1);
    let entry = career::set_spirit_fettered(&mut ch, &engine, &first, true).unwrap().unwrap();
    career::set_spirit_fettered(&mut ch, &engine, &first, false).unwrap();
    let second = add_spirit(&mut ch, &engine, 2, 1);
    career::set_spirit_fettered(&mut ch, &engine, &second, true).unwrap().unwrap();
    ch.remove_item("spirits", &first);
    career::undo_expense(&mut ch, &engine, &entry).unwrap();
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == second).unwrap();
    assert_eq!(s.get("fettered"), "True");
    assert_eq!(ch.improvements.list.iter().filter(|i| i.source == "SpiritFettering").count(), 1);
}

/// A fettered spirit gains Banishing Resistance (SG p. 192, LB-32; Chummer
/// adds nothing). The print lists the critter's powers with it; a spirit
/// that is not fettered prints no powers, as in Chummer.
#[test]
fn fettered_spirits_gain_banishing_resistance() {
    let engine = Engine::load().unwrap();
    let lang = chummer_core::lang::Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let guid = add_spirit(&mut ch, &engine, 3, 1);
    let printed = |ch: &Character| {
        let root = chummer_core::print::print_xml(ch, &engine, &lang);
        let mut v = Vec::new();
        root.descendants("spirit", &mut v);
        v.into_iter().find(|s| s.get("guid") == guid).unwrap().clone()
    };
    assert!(printed(&ch).child("powers").is_none());
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == guid).unwrap().clone();
    assert!(!spirit::gains_banishing_resistance(&s));
    career::set_spirit_fettered(&mut ch, &engine, &guid, true).unwrap();
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == guid).unwrap().clone();
    assert!(spirit::gains_banishing_resistance(&s));
    let p = printed(&ch);
    let names: Vec<String> = p.child("powers").unwrap().children_named("critterpower").map(|c| c.get("name_english")).collect();
    assert!(names.contains(&"Banishing Resistance".to_owned()), "{names:?}");
    assert!(names.contains(&"Accident".to_owned()), "the critter's own powers too: {names:?}");
    let br = p.child("powers").unwrap().children_named("critterpower").find(|c| c.get("name_english") == "Banishing Resistance").unwrap();
    assert_eq!((br.get("source"), br.get("page")), ("SG".to_owned(), "194".to_owned()));
    career::set_spirit_fettered(&mut ch, &engine, &guid, false).unwrap();
    assert!(printed(&ch).child("powers").is_none());
}

#[test]
fn sprites_need_sprite_pet_to_be_fettered() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("streams.xml").unwrap();
    let rec = data::find(&doc, "spirits", "spirit", "Courier Sprite").unwrap();
    let guid = spirit::add(&mut ch, rec, 3, 1, true);
    assert!(matches!(career::set_spirit_fettered(&mut ch, &engine, &guid, true), Err(CareerError::Refused(_))));
    ch.improvements.list.push(chummer_core::improvement::Improvement { kind: "AllowSpriteFettering".into(), source: "Custom".into(), enabled: true, rating: 1, ..Default::default() });
    let karma = ch.karma;
    career::set_spirit_fettered(&mut ch, &engine, &guid, true).unwrap();
    // Sprites cost their force, and no MAG.
    assert_eq!(ch.karma, karma - 3);
    assert!(!ch.improvements.list.iter().any(|i| i.source == "SpiritFettering"));
}

#[test]
fn second_metamagic_at_a_grade_costs_karma() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 200.0);
    ch.attribute_mut("MAG").unwrap().karma += 2;
    // A new grade with no metamagic yet.
    career::add_initiation_grade(&mut ch, &engine, InitiationOptions::default()).unwrap();
    let grade = career::grade_count(&ch, false);
    let doc = engine.store.doc("metamagic.xml").unwrap();
    let first = data::find(&doc, "metamagics", "metamagic", "Centering").unwrap();
    let second = data::find(&doc, "metamagics", "metamagic", "Masking").unwrap();
    assert_eq!(career::metamagic_karma_cost(&engine, &ch, grade), 0);
    let karma = ch.karma;
    let entries = career::entries(&ch).len();
    let m1 = career::learn_metamagic(&mut ch, &engine, first, None, grade).unwrap();
    assert_eq!(ch.karma, karma);
    assert_eq!(career::entries(&ch).len(), entries, "the free one is not logged");
    assert_eq!(career::metamagic_karma_cost(&engine, &ch, grade), 15);
    let m2 = career::learn_metamagic(&mut ch, &engine, second, None, grade).unwrap();
    assert_eq!(ch.karma, karma - 15);
    let e = entry_by_reason(&ch, "Metamagic Masking");
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::AddMetamagic);
    let saved = ch.items("metamagics", "metamagic").into_iter().find(|m| m.get("guid") == m2).unwrap();
    assert_eq!(saved.get_i32("grade"), Some(grade));
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, karma);
    let left: Vec<String> = ch.items("metamagics", "metamagic").iter().map(|m| m.get("guid")).collect();
    assert!(left.contains(&m1) && !left.contains(&m2));
    assert!(career::learn_metamagic(&mut ch, &engine, second, None, grade + 5).is_err());
}

#[test]
fn critter_powers_are_logged_even_when_free() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("critterpowers.xml").unwrap();
    let rec = data::records(&doc, "powers", "power").into_iter().find(|p| p.el().get_i32("karma").unwrap_or(0) > 0 && p.el().child("bonus").is_none()).unwrap();
    let cost = rec.el().get_i32("karma").unwrap();
    let karma = ch.karma;
    let guid = career::learn_critter_power(&mut ch, &engine, rec, 0, None).unwrap();
    assert_eq!(ch.karma, karma - cost);
    let e = entry_by_reason(&ch, &format!("Purchased Critter Power {}", rec.name()));
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::AddCritterPower);
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, karma);
    assert!(!ch.items("critterpowers", "critterpower").iter().any(|p| p.get("guid") == guid));
    let free = data::records(&doc, "powers", "power").into_iter().find(|p| p.el().get_i32("karma").unwrap_or(0) == 0 && p.el().child("bonus").is_none()).unwrap();
    career::learn_critter_power(&mut ch, &engine, free, 0, None).unwrap();
    assert_eq!(entry_by_reason(&ch, &format!("Purchased Critter Power {}", free.name())).amount, 0.0);
}

#[test]
fn joining_and_leaving_a_group() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    assert!(ch.mag_enabled());
    give_karma(&mut ch, 20.0);
    ch.doc.set_child_text("groupmember", "False");
    let karma = ch.karma;
    let joined = career::set_group_member(&mut ch, &engine, true).unwrap().unwrap();
    assert!(ch.flag("groupmember"));
    assert_eq!(ch.karma, karma - 5);
    assert_eq!(career::find_entry(&ch, &joined).unwrap().reason, "Joined a Group");
    career::set_group_member(&mut ch, &engine, false).unwrap().unwrap();
    assert!(!ch.flag("groupmember"));
    assert_eq!(ch.karma, karma - 6);
    let left = entry_by_reason(&ch, "Left a Group");
    assert_eq!(left.undo.as_ref().unwrap().karma_type, KarmaExpenseType::LeaveGroup);
    // Undoing the leave puts the character back in the group.
    career::undo_expense(&mut ch, &engine, &left.guid).unwrap();
    assert!(ch.flag("groupmember"));
    assert_eq!(ch.karma, karma - 5);
    career::undo_expense(&mut ch, &engine, &joined).unwrap();
    assert!(!ch.flag("groupmember"));
    assert_eq!(ch.karma, karma);
}

#[test]
fn quickening_a_spell() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 20.0);
    let spell = ch.items("spells", "spell").first().map(|s| (s.get("guid"), s.get("name"), s.get("extra"))).unwrap();
    assert!(career::quicken_spell(&mut ch, &spell.0, 0).is_err());
    let karma = ch.karma;
    let guid = career::quicken_spell(&mut ch, &spell.0, 4).unwrap();
    assert_eq!(ch.karma, karma - 4);
    let e = career::find_entry(&ch, &guid).unwrap();
    assert!(e.reason.starts_with(&format!("Quickened {}", spell.1)));
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::QuickeningMetamagic);
    career::undo_expense(&mut ch, &engine, &guid).unwrap();
    assert_eq!(ch.karma, karma);
}

#[test]
fn fettering_in_creation_sets_the_mag_penalty() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Gangerbean");
    let guid = add_spirit(&mut ch, &engine, 3, 1);
    assert!(spirit::set_state(&mut ch, &guid, 3, 1, true, true));
    assert!(ch.improvements.list.iter().any(|i| i.source == "SpiritFettering"));
    assert!(spirit::set_state(&mut ch, &guid, 3, 1, true, false));
    assert!(!ch.improvements.list.iter().any(|i| i.source == "SpiritFettering"));
}
