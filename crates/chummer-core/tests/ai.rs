//! A.I.s: programs (creation karma, career purchases and undo), the
//! home node and its effect on derived values, kits and save round trips.

use chummer_core::bonus::{self, BonusSource};
use chummer_core::calc::{self, spell_defense};
use chummer_core::career::{self, CareerRules, KarmaExpenseType, ManualExpense};
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::{self, aiprogram, Purchase};
use chummer_core::play::{ai, matrix};
use chummer_core::requirements::{self, Check};
use chummer_core::xml::Element;

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn new_ai(engine: &Engine) -> Character {
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "A.I.".into(),
        metavariant: None,
        // Heritage C, Talent A (A.I. - 6 Depth), Attributes B, Skills D, Resources E
        priorities: Priorities(['C', 'A', 'B', 'D', 'E']),
        talent: "A.I.".into(),
        talent_skills: vec![],
        name: "Ghost".into(),
    };
    chargen::create(engine, &spec).unwrap()
}

fn program(engine: &Engine, name: &str) -> Element {
    let doc = engine.store.doc("programs.xml").unwrap();
    data::find(&doc, "programs", "program", name).unwrap().el().clone()
}

fn add_program(engine: &Engine, ch: &mut Character, name: &str) -> String {
    let rec = program(engine, name);
    items::add("aiprogram", ch, &engine.store, data::Record(&rec), &Purchase::default()).unwrap()
}

fn add_item(engine: &Engine, ch: &mut Character, tag: &str, file: &str, container: &str, item: &str, name: &str) -> String {
    let doc = engine.store.doc(file).unwrap();
    let rec = data::find(&doc, container, item, name).unwrap();
    items::add(tag, ch, &engine.store, rec, &Purchase { rating: 1, qty: 1.0, cost_multiplier: 1.0, ..Default::default() }).unwrap()
}

fn karma_for(engine: &Engine, ch: &Character, key: &str) -> i32 {
    let rules = engine.rules_for(ch);
    let sheet = engine.sheet(ch);
    let settings = engine.settings.resolve(&ch.field("settings")).unwrap();
    chargen::karma_breakdown(ch, &sheet, &rules, settings, Some(&engine.store)).into_iter().find(|(k, _)| *k == key).unwrap().1
}

#[test]
fn ai_metatype_flags_and_core_track() {
    let engine = Engine::load().unwrap();
    let ch = new_ai(&engine);
    assert!(ch.is_ai());
    // The metatype's `enabletab` is saved as Chummer's `<ai>`.
    assert!(ch.advanced_programs_enabled());
    assert_eq!(ch.field("ai"), "True");
    assert!(ch.doc.child("ainode").is_none());
    let s = engine.sheet(&ch);
    let dep = s.attr("DEP");
    assert_eq!(dep, 6);
    assert_eq!(s.physical_cm, 8 + 3, "Core track: 8 + Depth / 2");
    assert_eq!(s.stun_cm, 0, "no Matrix track without a home node");
    assert_eq!(s.cm_overflow, 0);
    assert_eq!(s.attr_values("EDG").unwrap().metatype_max, dep, "Edge maximum is Depth");
    assert_eq!(s.limit_physical, 0);
    // No home node: no Body to soak with.
    assert_eq!(calc::soak_body(&ch, &s), 0);
    let pools = spell_defense(&ch, &s);
    let manip = pools.iter().find(|(k, _)| *k == "Label_SpellDefenseManipPhysical").unwrap().1;
    assert_eq!(manip, 0);
    assert_eq!(s.matrix_cold_dice, 4, "A.I.s always roll hot-sim dice");
    assert_eq!(s.matrix_cold_initiative, s.attr("INT"));
}

#[test]
fn programs_save_in_chummer_order_and_cost_creation_karma() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    assert_eq!(karma_for(&engine, &ch, "programs"), 0);
    let g = add_program(&engine, &mut ch, "Browse");
    let p = ch.items("aiprograms", "aiprogram")[0].clone();
    let tags: Vec<&str> = p.elements().map(|e| e.name.as_str()).collect();
    assert_eq!(tags, ["sourceid", "guid", "name", "candelete", "isadvancedprogram", "requiresprogram", "extra", "source", "page", "notes", "notesColor"]);
    assert_eq!(p.get("guid"), g);
    assert_eq!(p.get("candelete"), "True");
    assert_eq!(p.get("isadvancedprogram"), "False");
    // No free programs in the data: each costs KarmaNewAIProgram (5).
    assert_eq!(karma_for(&engine, &ch, "programs"), 5);
    add_program(&engine, &mut ch, "Abduction");
    let adv = ch.items("aiprograms", "aiprogram").into_iter().find(|p| p.get("name") == "Abduction").unwrap().clone();
    assert_eq!(adv.get("isadvancedprogram"), "True");
    assert_eq!(karma_for(&engine, &ch, "programs"), 5 + 8);

    // A spare Advanced Program slot takes a normal program.
    ch.doc.set_child_text("aiadvancedprogramlimit", "2");
    assert_eq!(karma_for(&engine, &ch, "programs"), 0);
    ch.doc.set_child_text("aiadvancedprogramlimit", "0");

    // Programs from improvements are free and cannot be removed.
    let rec = program(&engine, "Edit");
    aiprogram::add(&mut ch, &engine.store, data::Record(&rec), None, false);
    assert_eq!(karma_for(&engine, &ch, "programs"), 13);
    let edit = ch.items("aiprograms", "aiprogram").into_iter().find(|p| p.get("name") == "Edit").unwrap().get("guid");
    assert!(aiprogram::remove(&mut ch, &edit).is_err());
    aiprogram::remove(&mut ch, &g).unwrap();
    assert_eq!(karma_for(&engine, &ch, "programs"), 8);

    // Round trip: the file keeps the programs as written.
    let again = Character::from_str(&ch.to_xml_string()).unwrap();
    assert_eq!(again.items("aiprograms", "aiprogram"), ch.items("aiprograms", "aiprogram"));
    // The oracle rebuilds a saved program from data.
    let rebuilt = items::rebuild("aiprogram", &ch, &engine.store, &adv).unwrap();
    assert_eq!(rebuilt, adv);
}

#[test]
fn advanced_programs_require_their_program() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let sheet = engine.sheet(&ch);
    let rec = program(&engine, "Authority");
    let check = |ch: &Character| requirements::unmet(&rec, &Check { ch, sheet: &sheet, ignore_quality: None }).is_empty();
    assert!(!check(&ch));
    add_program(&engine, &mut ch, "Exploit");
    assert!(check(&ch));
}

#[test]
fn karma_cost_bonuses() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let rules = engine.rules_for(&ch);
    assert_eq!(aiprogram::program_karma_cost(&ch, &rules), 5);
    assert_eq!(aiprogram::advanced_program_karma_cost(&ch, &rules), 8);
    let b = chummer_core::xml::parse(
        "<bonus><newaiprogramkarmacost>-2</newaiprogramkarmacost><newaiadvancedprogramkarmacostmultiplier>50</newaiadvancedprogramkarmacostmultiplier></bonus>",
    )
    .unwrap();
    let src = BonusSource { kind: "Custom".into(), guid: "x".into(), name: "GM".into(), rating: 1 };
    let out = bonus::apply(&ch, &engine.store, &b, &src, None);
    assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);
    items::apply_outcome(&mut ch, &out);
    assert_eq!(aiprogram::program_karma_cost(&ch, &rules), 3);
    assert_eq!(aiprogram::advanced_program_karma_cost(&ch, &rules), 4);
}

#[test]
fn selectaiprogram_grants_an_undeletable_program() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let b = chummer_core::xml::parse("<bonus><selectaiprogram /></bonus>").unwrap();
    let src = BonusSource { kind: "Quality".into(), guid: "q".into(), name: "Test".into(), rating: 1 };
    let out = bonus::apply(&ch, &engine.store, &b, &src, Some("Sneak"));
    assert_eq!(out.selected.as_deref(), Some("Sneak"));
    items::place_added(&mut ch, &engine.store, &out.added);
    let p = ch.items("aiprograms", "aiprogram");
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].get("candelete"), "False");
}

#[test]
fn career_programs_cost_karma_and_undo_refunds() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let rules = engine.rules_for(&ch);
    let sheet = engine.sheet(&ch);
    let settings = engine.settings.resolve(STANDARD).unwrap();
    let b = chargen::budget(&ch, &sheet, &rules, settings);
    chargen::finalize(&mut ch, &b, settings);
    let start = ch.karma;
    career::karma_gained(&mut ch, &CareerRules::default(), &ManualExpense { amount: 20.0, reason: "Run".into(), ..Default::default() }).unwrap();
    let rec = program(&engine, "Browse");
    let g = career::learn_ai_program(&mut ch, &engine, data::Record(&rec), &Purchase::default()).unwrap();
    assert_eq!(ch.karma, start + 20 - 5);
    let e = career::entries(&ch).into_iter().find(|e| e.reason == "Learned AI Program Browse").unwrap();
    let u = e.undo.clone().unwrap();
    assert_eq!(u.karma_type, KarmaExpenseType::AddAIProgram);
    assert_eq!(u.object_id, g);
    let rec = program(&engine, "Abduction");
    career::learn_ai_program(&mut ch, &engine, data::Record(&rec), &Purchase::default()).unwrap();
    assert_eq!(ch.karma, start + 20 - 13);
    let adv = career::entries(&ch).into_iter().find(|e| e.reason == "Learned AI Program Abduction").unwrap();
    assert_eq!(adv.undo.unwrap().karma_type, KarmaExpenseType::AddAIAdvancedProgram);
    // No creation karma in career mode.
    assert_eq!(karma_for(&engine, &ch, "programs"), 0);
    // Undo refunds the karma and removes the program (Chummer keeps it),
    // unless a program the character keeps requires it.
    let rec = program(&engine, "Clearsight Autosoft");
    let cs = career::learn_ai_program(&mut ch, &engine, data::Record(&rec), &Purchase::default()).unwrap();
    let cs_entry = career::entries(&ch).into_iter().find(|e| e.undo.as_ref().is_some_and(|u| u.object_id == cs)).unwrap();
    let karma = ch.karma;
    assert!(career::undo_expense(&mut ch, &engine, &cs_entry.guid).is_err(), "Abduction requires Clearsight Autosoft");
    assert_eq!(ch.karma, karma);
    assert!(ch.items("aiprograms", "aiprogram").iter().any(|p| p.get("guid") == cs));
    ch.karma += 5;
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, start + 20 - 8);
    let names: Vec<String> = ch.items("aiprograms", "aiprogram").iter().map(|p| p.get("name")).collect();
    assert_eq!(names, ["Abduction", "Clearsight Autosoft"]);
    // Not enough karma: nothing is added.
    ch.karma = 4;
    let rec = program(&engine, "Edit");
    assert!(career::learn_ai_program(&mut ch, &engine, data::Record(&rec), &Purchase::default()).is_err());
    assert_eq!(ch.items("aiprograms", "aiprogram").len(), 2);
}

#[test]
fn commlink_home_node() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let base = engine.sheet(&ch);
    let link = add_item(&engine, &mut ch, "gear", "gear.xml", "gears", "gear", "Meta Link");
    let deck = add_item(&engine, &mut ch, "gear", "gear.xml", "gears", "gear", "Microtrónica Azteca 200");
    let dep = base.attr("DEP");
    let find = |ch: &Character, g: &str| ch.items("gears", "gear").into_iter().find(|e| e.get("guid") == g).unwrap().clone();
    // Depth 6 is above both device ratings: the Program Limit must be 2.
    assert!(!matrix::can_be_home_node(&find(&ch, &link), dep));
    assert!(matrix::can_be_home_node(&find(&ch, &deck), dep));

    assert!(matrix::set_home_node(&mut ch, &deck, true));
    let d = find(&ch, &deck);
    let s = engine.sheet(&ch);
    assert_eq!(s.stun_cm, matrix::condition_monitor(&d), "Stun track is the Matrix track");
    assert_eq!(s.physical_cm, base.physical_cm, "still a Core track");
    let dp = matrix::total(&d, "Data Processing");
    assert_eq!(s.matrix_cold_initiative, s.attr("INT") + dp);
    assert_eq!(s.limit_mental, base.limit_mental.max(dp));
    // Matrix damage goes to the deck; it gives no wound penalty.
    assert!(ai::set_stun_filled(&mut ch, 3));
    assert_eq!(matrix::filled(&find(&ch, &deck)), 3);
    assert_eq!(ch.stun_cm_filled, 0);
    assert_eq!(engine.sheet(&ch).wound_modifier, 0);

    // Only one home node at a time; saved as Chummer's `<homenode>`.
    assert!(matrix::set_home_node(&mut ch, &link, true));
    assert_eq!(find(&ch, &deck).get("homenode"), "False");
    let again = Character::from_str(&ch.to_xml_string()).unwrap();
    assert_eq!(matrix::home_node(&again).map(|e| e.get("guid")), Some(link.clone()));
    assert!(matrix::set_home_node(&mut ch, &link, false));
    assert!(matrix::home_node(&ch).is_none());
}

#[test]
fn vehicle_home_node() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let v = add_item(&engine, &mut ch, "vehicle", "vehicles.xml", "vehicles", "vehicle", "GM-Nissan Doberman (Medium)");
    matrix::set_home_node(&mut ch, &v, true);
    let veh = ch.items("vehicles", "vehicle")[0].clone();
    let st = chummer_core::items::vehicle::stats(&veh);
    let s = engine.sheet(&ch);
    assert_eq!(s.physical_cm, chummer_core::play::vehicle::condition_monitor(&veh, &Default::default()), "the drone's damage track");
    assert_eq!(s.limit_physical, 5, "the drone's Handling");
    assert_eq!(calc::soak_body(&ch, &s), st.body);
    let pools = spell_defense(&ch, &s);
    let get = |k: &str| pools.iter().find(|(n, _)| *n == k).unwrap().1;
    assert_eq!(get("Label_SpellDefenseManipPhysical"), 2 * st.body);
    assert_eq!(get("Label_SpellDefenseDirectSoakPhysical"), st.body);
    assert_eq!(s.matrix_cold_initiative, s.attr("INT") + matrix::total(&veh, "Data Processing").max(st.pilot));
    // Physical damage goes to the drone, not the character's own track.
    assert!(ai::set_physical_filled(&mut ch, 2));
    assert_eq!(chummer_core::play::vehicle::filled(ch.items("vehicles", "vehicle")[0]), 2);
    assert_eq!(ch.physical_cm_filled, 0);
    assert_eq!(ai::physical_filled(&ch), 2);
}

#[test]
fn kits_add_programs() {
    let engine = Engine::load().unwrap();
    let mut ch = new_ai(&engine);
    let kit = chummer_core::xml::parse("<pack><name>Test</name><programs><program><name>Browse</name></program><program><name>No Such Program</name></program></programs></pack>").unwrap();
    let r = chummer_core::gm::packs::apply(&mut ch, &engine.store, engine.settings.resolve(STANDARD), &kit);
    assert!(r.added.contains(&"Program: Browse".to_owned()), "{r:?}");
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(ch.items("aiprograms", "aiprogram").len(), 2);
}
