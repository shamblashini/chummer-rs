//! Core helpers behind the GUI's magic, lifestyle and drug editors.

use std::path::Path;

use chummer_core::career::{self, CareerRules, KarmaExpenseType, ManualExpense};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::lifestyle::{self, Options};
use chummer_core::items::magic::{account, martialart, mentor, power, spell};
use chummer_core::items::{self, Purchase};

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

#[test]
fn power_set_rating_reapplies_bonus() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Skink");
    let doc = engine.store.doc("powers.xml").unwrap();
    let rec = data::find(&doc, "powers", "power", "Improved Reflexes").unwrap();
    let guid = items::add("power", &mut ch, &engine.store, rec, &Purchase { rating: 1, ..Default::default() }).unwrap();
    let before = ch.improvements.list.iter().filter(|i| i.source_name == guid).count();
    assert!(before > 0);
    power::set_rating(&mut ch, &engine.store, &guid, 3).unwrap();
    let p = ch.items("powers", "power").into_iter().find(|p| p.get("guid") == guid).unwrap();
    assert_eq!(p.get("rating"), "3");
    let after: Vec<_> = ch.improvements.list.iter().filter(|i| i.source_name == guid).collect();
    assert_eq!(after.len(), before, "improvements are replaced, not duplicated");
    // Same improvements as a power bought at 3 levels outright.
    let fresh = items::add("power", &mut ch, &engine.store, rec, &Purchase { rating: 3, ..Default::default() }).unwrap();
    let key = |g: &str| {
        let mut v: Vec<(String, String, String)> = ch.improvements.list.iter().filter(|i| i.source_name == g).map(|i| (i.kind.clone(), i.improved_name.clone(), format!("{} {} {}", i.val, i.aug, i.rating))).collect();
        v.sort();
        v
    };
    assert_eq!(key(&guid), key(&fresh));
}

#[test]
fn career_spell_with_options_is_charged_by_category() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("spells.xml").unwrap();
    let rec = data::find(&doc, "spells", "spell", "Stunbolt").unwrap();
    let karma = ch.karma;
    let o = spell::SpellOptions { alchemical: true, limited: true, ..Default::default() };
    let guid = career::learn_spell_with(&mut ch, &engine, &engine.store, rec, None, &o).unwrap();
    let s = ch.items("spells", "spell").into_iter().find(|s| s.get("guid") == guid).unwrap();
    assert_eq!(s.get("alchemical"), "True");
    assert_eq!(s.get("limited"), "True");
    let cost = career::spell_karma_cost(&engine, &ch, "Preparations");
    assert_eq!(ch.karma, karma - cost);
    // A free spell costs nothing.
    let karma = ch.karma;
    let rec = data::find(&doc, "spells", "spell", "Manabolt").unwrap();
    career::learn_spell_with(&mut ch, &engine, &engine.store, rec, None, &spell::SpellOptions { free_bonus: true, ..Default::default() }).unwrap();
    assert_eq!(ch.karma, karma);
}

#[test]
fn techniques_after_the_first_cost_karma() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    give_karma(&mut ch, 50.0);
    let doc = engine.store.doc("martialarts.xml").unwrap();
    let art = data::find(&doc, "martialarts", "martialart", "Aikido").unwrap();
    let names = martialart::technique_names(art);
    assert!(names.len() >= 2);
    let guid = martialart::add(&mut ch, &engine.store, art, None);
    assert_eq!(career::technique_karma_cost(&engine, &ch, &guid), 0);
    let karma = ch.karma;
    career::learn_technique(&mut ch, &engine, &engine.store, &guid, &names[0]).unwrap();
    assert_eq!(ch.karma, karma);
    let cost = career::technique_karma_cost(&engine, &ch, &guid);
    assert_eq!(cost, 5);
    career::learn_technique(&mut ch, &engine, &engine.store, &guid, &names[1]).unwrap();
    assert_eq!(ch.karma, karma - 5);
    assert_eq!(entry_by_reason(&ch, &format!("Learned Technique {}", names[1])).undo.unwrap().karma_type, KarmaExpenseType::AddMartialArtTechnique);
    let a = ch.items("martialarts", "martialart").into_iter().find(|a| a.get("guid") == guid).unwrap();
    assert_eq!(a.child("martialarttechniques").unwrap().children_named("martialarttechnique").count(), 2);
}

#[test]
fn binding_a_focus_in_career() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Gangerbean");
    ch.created = true;
    ch.improvements.career = true;
    give_karma(&mut ch, 50.0);
    let gear = ch.items("gears", "gear").into_iter().find(|g| g.get("category") == "Foci" && !ch.items("foci", "focus").iter().any(|f| f.get("gearid") == g.get("guid"))).cloned();
    let gear = gear.expect("an unbound focus");
    let guid = gear.get("guid");
    let cost = career::focus_karma_cost(&engine, &ch, &gear);
    let karma = ch.karma;
    career::bind_focus(&mut ch, &engine, &guid).unwrap();
    account::set_focus_bonded(&mut ch, &engine.store, &guid, true);
    assert_eq!(ch.karma, karma - cost);
    assert!(ch.items("foci", "focus").iter().any(|f| f.get("gearid") == guid));
    let g = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == guid).unwrap();
    assert_eq!(g.get("bonded"), "True");
    // Binding twice is refused.
    assert!(career::bind_focus(&mut ch, &engine, &guid).is_err());
    assert!(account::unbind_focus(&mut ch, &guid));
    account::set_focus_bonded(&mut ch, &engine.store, &guid, false);
    assert!(!ch.improvements.list.iter().any(|i| i.source_name == guid));
}

#[test]
fn mentor_for_a_quality_that_grants_one() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Fuzzy-chargen");
    assert!(ch.items("mentorspirits", "mentorspirit").is_empty());
    let doc = engine.store.doc("qualities.xml").unwrap();
    let rec = data::find(&doc, "qualities", "quality", "Mentor Spirit").unwrap();
    let qguid = items::add("quality", &mut ch, &engine.store, rec, &Purchase::default()).unwrap();
    let pending = mentor::pending_mentor_qualities(&ch, &engine.store);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, qguid);
    assert_eq!(pending[0].2, "MentorSpirit");
    let mguid = mentor::add_mentor_for_quality(&mut ch, &engine.store, &qguid, "MentorSpirit", "Bear", None, None).unwrap();
    assert!(mentor::pending_mentor_qualities(&ch, &engine.store).is_empty());
    let mdoc = engine.store.doc("mentors.xml").unwrap();
    let bear = data::find(&mdoc, "mentors", "mentor", "Bear").unwrap();
    let c = mentor::choice_names(bear);
    mentor::set_mentor_choices(&mut ch, &engine.store, &mguid, Some(&c[0]), None).unwrap();
    let m = ch.items("mentorspirits", "mentorspirit").into_iter().find(|m| m.get("guid") == mguid).unwrap();
    assert_eq!(m.get("extrachoice1"), c[0]);
}

#[test]
fn lifestyle_edit_limits_and_quality_removal() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Glessner");
    let l = ch.items("lifestyles", "lifestyle")[0].clone();
    let guid = l.get("guid");
    let mut o = Options::from_saved(&l);
    o.months = 4;
    o.roommates = 1;
    o.style = "Advanced".into();
    o.comforts = 99;
    assert!(lifestyle::update(&mut ch, &guid, &o));
    let l = ch.items("lifestyles", "lifestyle")[0].clone();
    assert_eq!(l.get("months"), "4");
    assert_eq!(l.get("roommates"), "1");
    let (_, max_comforts, _) = lifestyle::point_limits(&l);
    assert!(l.get_i32("comforts").unwrap() <= max_comforts);
    assert!((lifestyle::total_cost(&ch, &l) - 4.0 * lifestyle::monthly_cost(&ch, &l)).abs() < 1e-6);
    // Add then remove a quality.
    let doc = engine.store.doc("lifestyles.xml").unwrap();
    let rec = data::find(&doc, "qualities", "quality", "Armory").unwrap();
    let before = lifestyle::monthly_cost(&ch, &l);
    let q = lifestyle::add_quality(&mut ch, &engine.store, &guid, rec, None, false).unwrap();
    let l2 = ch.items("lifestyles", "lifestyle")[0].clone();
    assert!(lifestyle::monthly_cost(&ch, &l2) > before);
    assert!(lifestyle::remove_quality(&mut ch, &guid, &q));
    let l3 = ch.items("lifestyles", "lifestyle")[0].clone();
    assert!((lifestyle::monthly_cost(&ch, &l3) - before).abs() < 1e-6);
    assert!(!lifestyle::remove_quality(&mut ch, &guid, &q));
}
