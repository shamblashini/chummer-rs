//! Career mode: the expense log against Chummer-saved careers, and
//! spend/undo round trips.

use std::path::Path;

use chummer_core::career::{self, CareerError, CareerRules, ExpenseType, InitiationOptions, KarmaExpenseType, ManualExpense, NuyenExpenseType};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::xml::Element;

const CAREERS: &[&str] = &["Draught", "Glessner", "Munin_Career", "Pañcama", "Serpent", "Soma (Career)", "Wesson"];

fn load(name: &str) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.chum5"));
    Character::load(&p).unwrap()
}

fn give_karma(ch: &mut Character, amount: f64) {
    let m = ManualExpense { amount, reason: "Run".into(), ..Default::default() };
    career::karma_gained(ch, &CareerRules::default(), &m).unwrap();
}

/// The last entry added (newest guid is not ordered; find by reason).
fn entry_by_reason(ch: &Character, reason: &str) -> career::ExpenseEntry {
    career::entries(ch).into_iter().find(|e| e.reason == reason).unwrap_or_else(|| panic!("no entry {reason:?}"))
}

#[test]
fn logs_add_up_to_saved_balances() {
    for name in CAREERS {
        let ch = load(name);
        let t = career::totals(&ch);
        assert_eq!(t.karma_logged, ch.karma, "{name} karma");
        assert!((t.nuyen_logged - ch.nuyen).abs() < 0.005, "{name} nuyen {} vs {}", t.nuyen_logged, ch.nuyen);
    }
}

#[test]
fn career_karma_and_street_cred() {
    let munin = load("Munin_Career");
    // Positive non-refund karma: 2 + 8 + 33 + 5 (the two refunds excluded).
    assert_eq!(career::career_karma(&munin), 48);
    assert_eq!(career::career_nuyen(&munin), 830.0 + 16000.0 + 10000.0);
    let rep = career::reputation(&munin, &CareerRules::default());
    assert_eq!(rep.street_cred_calculated, 4);
    assert_eq!(rep.street_cred, 4);
    let soma = load("Soma (Career)");
    assert_eq!(career::career_karma(&soma), 11);
    assert_eq!(career::reputation(&soma, &CareerRules::default()).street_cred, 1);
    let t = career::totals(&munin);
    assert_eq!(t.career_karma, 48);
    assert_eq!(t.karma_spent, 5 + 14 + 6 + 16 + 5 + 11);
}

#[test]
fn entries_load_and_save() {
    let ch = load("Munin_Career");
    let list = career::entries(&ch);
    assert_eq!(list.len(), 17);
    let ini = list.iter().find(|e| e.reason == "Initiate Grade 0 -> 1").unwrap();
    let u = ini.undo.as_ref().unwrap();
    assert_eq!(u.karma_type, KarmaExpenseType::ImproveInitiateGrade);
    assert_eq!(u.nuyen_type, NuyenExpenseType::AddCyberware, "unset type keeps its zero value");
    assert_eq!(ini.amount, -11.0);
    for e in &list {
        assert_eq!(&career::ExpenseEntry::from_xml(&e.to_xml()), e);
    }
    assert_eq!(KarmaExpenseType::parse("Bogus"), KarmaExpenseType::ManualAdd);
    assert_eq!(KarmaExpenseType::parse("2"), KarmaExpenseType::ImproveSkillGroup);
    let newest = career::entries_of(&ch, ExpenseType::Nuyen);
    assert_eq!(newest.first().unwrap().reason, "Initiate Grade 1 -> 2");
    // Old saves: " (Refund)" suffix and the 🡒 arrow are normalised.
    let mut e = Element::new("expense");
    e.push(Element::with_text("reason", "Attribute BOD 1 🡒 2 (Refund)"));
    assert_eq!(career::ExpenseEntry::from_xml(&e).reason, "Attribute BOD 1 -> 2");
}

#[test]
fn new_entries_leave_saved_ones_untouched() {
    let mut ch = load("Soma (Career)");
    let before = ch.to_xml_string();
    assert!(before.contains("<amount>-6000.00</amount>"));
    give_karma(&mut ch, 3.0);
    let after = ch.to_xml_string();
    assert!(after.contains("<amount>-6000.00</amount>"), "decimal scale kept");
    assert_eq!(ch.karma, 14);
    assert_eq!(career::entries(&ch).len(), 17);
    // Dated now, so it sorts last in the file.
    assert_eq!(career::entries(&ch).last().unwrap().reason, "Run");
    let again = Character::from_str(&after).unwrap();
    assert_eq!(career::totals(&again).karma_logged, 14);
}

#[test]
fn manual_entries_and_exchange() {
    let mut ch = load("Wesson");
    let cr = CareerRules::default();
    let (k0, n0) = (ch.karma, ch.nuyen);
    let m = ManualExpense { amount: 2.0, reason: "Working for the man".into(), exchange: true, ..Default::default() };
    career::karma_spent(&mut ch, &cr, &m).unwrap();
    assert_eq!(ch.karma, k0 - 2);
    assert_eq!(ch.nuyen, n0 + 4000.0);
    let t = career::totals(&ch);
    assert_eq!(t.karma_logged, ch.karma);
    assert!((t.nuyen_logged - ch.nuyen).abs() < 0.005);
    let err = career::karma_spent(&mut ch, &cr, &ManualExpense { amount: 99.0, ..Default::default() }).unwrap_err();
    assert_eq!(err, CareerError::NotEnoughKarma { need: 99, have: 0 });

    let g = career::nuyen_gained(&mut ch, &cr, &ManualExpense { amount: 500.0, reason: "Pay".into(), date: Some("2020-01-01T00:00:00".into()), ..Default::default() }).unwrap();
    assert_eq!(career::find_entry(&ch, &g).unwrap().date, "2020-01-01T00:00:00");
    assert!(career::edit_entry(&mut ch, &g, &career::EntryEdit { amount: Some(800.0), reason: Some("Bonus".into()), date: None }));
    assert_eq!(ch.nuyen, n0 + 4000.0 + 800.0);
    assert_eq!(career::find_entry(&ch, &g).unwrap().reason, "Bonus");
    let engine = Engine::load().unwrap();
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(ch.nuyen, n0 + 4000.0);
    assert!(career::find_entry(&ch, &g).is_none());
}

#[test]
fn attribute_upgrade_and_undo() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    give_karma(&mut ch, 40.0);
    let rules = engine.rules_for(&ch);
    let v = chummer_core::calc::attribute_values(&ch, "BOD", &rules);
    let cost = career::attribute_upgrade_karma_cost(&engine, &ch, "BOD").unwrap();
    assert_eq!(cost, (v.value + 1) * 5);
    let karma_before = ch.attribute("BOD").unwrap().karma;
    let g = career::improve_attribute(&mut ch, &engine, "BOD").unwrap();
    assert_eq!(ch.karma, 40 - cost);
    assert_eq!(ch.attribute("BOD").unwrap().karma, karma_before + 1);
    let e = career::find_entry(&ch, &g).unwrap();
    assert_eq!(e.reason, format!("Attribute BOD {} -> {}", v.value, v.value + 1));
    assert_eq!(e.undo.unwrap().object_id, "BOD");
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(ch.karma, 40);
    assert_eq!(ch.attribute("BOD").unwrap().karma, karma_before);
    // Not enough karma.
    ch.karma = 0;
    assert!(matches!(career::improve_attribute(&mut ch, &engine, "BOD"), Err(CareerError::NotEnoughKarma { .. })));
}

#[test]
fn skill_spec_and_knowledge_round_trips() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    give_karma(&mut ch, 60.0);
    let sheet = engine.sheet(&ch);
    let sk = sheet.skills.iter().find(|s| s.group.is_empty() && s.rating > 0 && s.rating == s.base + s.karma && !s.disabled).unwrap().clone();
    let cost = career::skill_upgrade_karma_cost(&engine, &ch, &sk.guid).unwrap();
    assert_eq!(cost, (sk.rating + 1) * 2);
    let g = career::improve_skill(&mut ch, &engine, &sk.guid).unwrap();
    let e = career::find_entry(&ch, &g).unwrap();
    assert_eq!(e.reason, format!("Active Skill {} {} -> {}", sk.name, sk.rating, sk.rating + 1));
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::ImproveSkill);
    assert_eq!(engine.sheet(&ch).skills.iter().find(|s| s.guid == sk.guid).unwrap().rating, sk.rating + 1);
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(engine.sheet(&ch).skills.iter().find(|s| s.guid == sk.guid).unwrap().rating, sk.rating);
    assert_eq!(ch.karma, 60);

    // Specialization: 7 karma, undone by its own guid.
    assert_eq!(career::specialization_karma_cost(&engine, &ch, &sk.guid), Some(7));
    let specs = ch.skills.iter().find(|s| s.guid == sk.guid).unwrap().specs.len();
    let spec = career::buy_specialization(&mut ch, &engine, &sk.guid, "Testing").unwrap();
    assert_eq!(ch.karma, 53);
    assert_eq!(ch.skills.iter().find(|s| s.guid == sk.guid).unwrap().specs.len(), specs + 1);
    let eg = career::entries(&ch).into_iter().find(|e| e.undo.as_ref().is_some_and(|u| u.object_id == spec)).unwrap().guid;
    career::undo_expense(&mut ch, &engine, &eg).unwrap();
    assert_eq!(ch.skills.iter().find(|s| s.guid == sk.guid).unwrap().specs.len(), specs);
    assert_eq!(ch.karma, 60);

    // A new knowledge skill costs KarmaNewKnowledgeSkill (1) and undo deletes it.
    let n = ch.knowledge_skills.len();
    let kg = career::learn_knowledge_skill(&mut ch, &engine, "Seattle Gangs", "Street").unwrap();
    assert_eq!(ch.karma, 59);
    let e = entry_by_reason(&ch, "Knowledge Skill Seattle Gangs 0 -> 1");
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::AddSkill);
    assert_eq!(career::skill_upgrade_karma_cost(&engine, &ch, &kg), Some(2));
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.knowledge_skills.len(), n);
    assert_eq!(ch.karma, 60);
}

#[test]
fn skill_group_reproduces_munin() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    // Munin's log: "Skill Group Engineering 0 -> 1", -5, an old AddSkill
    // undo naming the group.
    let e = entry_by_reason(&ch, "Skill Group Engineering 0 -> 1");
    assert_eq!(ch.skill_groups.iter().find(|g| g.name == "Engineering").unwrap().karma, 1);
    career::undo_expense(&mut ch, &engine, &e.guid).unwrap();
    assert_eq!(ch.karma, 5);
    assert_eq!(ch.skill_groups.iter().find(|g| g.name == "Engineering").unwrap().karma, 0);
    assert_eq!(career::skill_group_upgrade_karma_cost(&engine, &ch, "Engineering"), Some(5));
    let g = career::improve_skill_group(&mut ch, &engine, "Engineering").unwrap();
    assert_eq!(ch.karma, 0);
    assert_eq!(career::find_entry(&ch, &g).unwrap().reason, "Skill Group Engineering 0 -> 1");
    assert_eq!(ch.skill_groups.iter().find(|g| g.name == "Engineering").unwrap().karma, 1);
}

#[test]
fn quality_costs_reproduce_munin() {
    let engine = Engine::load().unwrap();
    let ch = load("Munin_Career");
    let doc = engine.store.doc("qualities.xml").unwrap();
    for (name, cost) in [("Apt Pupil", 5), ("Privileged Family Name", 14), ("Hawk Eye", 6), ("Spirit Whisperer", 16)] {
        let rec = data::find(&doc, "qualities", "quality", name).unwrap();
        assert_eq!(career::quality_karma_cost(&engine, &ch, rec), cost, "{name}");
        let logged = entry_by_reason(&ch, &format!("Gained Positive Quality {name}"));
        assert_eq!(logged.amount, -f64::from(cost));
    }
}

#[test]
fn quality_add_buy_off_and_undo() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    give_karma(&mut ch, 30.0);
    let doc = engine.store.doc("qualities.xml").unwrap();
    let count = |ch: &Character| ch.items("qualities", "quality").len();
    let n = count(&ch);

    // Undo Munin's own Hawk Eye purchase: quality gone, 6 karma back.
    let hawk = entry_by_reason(&ch, "Gained Positive Quality Hawk Eye");
    career::undo_expense(&mut ch, &engine, &hawk.guid).unwrap();
    assert_eq!(count(&ch), n - 1);
    assert_eq!(ch.karma, 36);

    // Buy it again.
    let rec = data::find(&doc, "qualities", "quality", "Hawk Eye").unwrap();
    let q = career::add_quality(&mut ch, &engine, rec, None).unwrap();
    assert_eq!(ch.karma, 30);
    assert_eq!(count(&ch), n);
    let e = entry_by_reason(&ch, "Gained Positive Quality Hawk Eye");
    assert_eq!(e.undo.as_ref().unwrap().object_id, q);

    // Buy off SINner (National), karma -5: costs 10.
    let sinner = ch.items("qualities", "quality").into_iter().find(|q| q.get("name") == "SINner (National)").unwrap().get("guid");
    let g = career::remove_quality(&mut ch, &engine, &sinner).unwrap().unwrap();
    assert_eq!(ch.karma, 20);
    assert_eq!(count(&ch), n - 1);
    let e = career::find_entry(&ch, &g).unwrap();
    assert_eq!(e.reason, "Removed Negative Quality SINner (National)");
    assert_eq!(e.undo.as_ref().unwrap().karma_type, KarmaExpenseType::RemoveQuality);
    // Undo puts it back from data.
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(ch.karma, 30);
    assert!(ch.items("qualities", "quality").iter().any(|q| q.get("name") == "SINner (National)"));

    // Metatype qualities stay.
    let meta = ch.items("qualities", "quality").into_iter().find(|q| q.get("qualitysource") == "Metatype").unwrap().get("guid");
    assert!(matches!(career::remove_quality(&mut ch, &engine, &meta), Err(CareerError::Refused(_))));
}

#[test]
fn initiation_reproduces_munin() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    let cr = CareerRules::for_character(&engine, &ch);
    let opts = InitiationOptions { group: false, ordeal: true, schooling: true };
    // (10 + 1 × 3) × 0.8 = 10.4, rounded up: Munin paid 11.
    assert_eq!(career::grade_karma_cost(&cr, 1, false, opts), 11);
    assert_eq!(career::grade_karma_cost(&cr, 1, false, InitiationOptions::default()), 13);
    assert_eq!(career::grade_karma_cost(&cr, 1, true, InitiationOptions { group: false, ordeal: true, schooling: false }), 11);

    // Undo the schooling nuyen, then the grade itself.
    let (karma0, nuyen0) = (ch.karma, ch.nuyen);
    let school = entry_by_reason(&ch, "Initiate Grade 1 -> 2");
    career::undo_expense(&mut ch, &engine, &school.guid).unwrap();
    assert_eq!(ch.nuyen, nuyen0 + 10_000.0);
    let grade = entry_by_reason(&ch, "Initiate Grade 0 -> 1");
    career::undo_expense(&mut ch, &engine, &grade.guid).unwrap();
    assert_eq!(ch.karma, karma0 + 11);
    assert_eq!(career::grade_count(&ch, false), 0);
    assert_eq!(ch.field("initiategrade"), "0");
    assert!(!ch.improvements.list.iter().any(|i| i.source == "Initiation"));
    assert!(ch.items("metamagics", "metamagic").is_empty(), "grade-1 metamagic removed");

    // Join again: same price, schooling nuyen, MAG maximum +1.
    let g = career::add_initiation_grade(&mut ch, &engine, opts).unwrap();
    assert_eq!(ch.karma, karma0);
    assert_eq!(ch.nuyen, nuyen0);
    assert_eq!(career::grade_count(&ch, false), 1);
    let mag: Vec<_> = ch.improvements.list.iter().filter(|i| i.source == "Initiation").collect();
    assert_eq!(mag.len(), 2);
    assert!(mag.iter().all(|i| i.max == 1.0 && i.rating == 1));
    let t = career::totals(&ch);
    assert_eq!(t.karma_logged, ch.karma);
    assert!((t.nuyen_logged - ch.nuyen).abs() < 0.005);
    let saved = Character::from_str(&ch.to_xml_string()).unwrap();
    assert!(saved.items("initiationgrades", "initiationgrade").iter().any(|e| e.get("guid") == g && e.get("ordeal") == "True"));
}

#[test]
fn spells_cost_karma_and_undo_removes() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    give_karma(&mut ch, 10.0);
    assert_eq!(career::spell_karma_cost(&engine, &ch, "Spells"), 5);
    assert_eq!(career::complex_form_karma_cost(&engine, &ch), 4);
    let mut s = Element::new("spell");
    s.push(Element::with_text("guid", "11111111-2222-4333-8444-555555555555"));
    s.push(Element::with_text("name", "Armor"));
    s.push(Element::with_text("category", "Manipulation"));
    s.push(Element::with_text("alchemical", "False"));
    s.push(Element::with_text("freebonus", "False"));
    ch.items_mut("spells").push(s);
    let n = ch.items("spells", "spell").len();
    let g = career::pay_for_spell(&mut ch, &engine, "11111111-2222-4333-8444-555555555555").unwrap().unwrap();
    assert_eq!(ch.karma, 5);
    assert_eq!(career::find_entry(&ch, &g).unwrap().reason, "Learned Spell Armor");
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(ch.items("spells", "spell").len(), n - 1);
    assert_eq!(ch.karma, 10);
}

#[test]
fn nuyen_purchases_undo_and_sale() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Soma (Career)");
    let gear = |ch: &Character, guid: &str| ch.items("gears", "gear").into_iter().any(|g| g.get("guid") == guid);
    // Undo "Purchased Gear Psyche" (3 doses, 714¥): the gear goes, nuyen back.
    let n0 = ch.nuyen;
    let psyche = career::entries(&ch).into_iter().find(|e| e.amount == -714.0).unwrap();
    assert_eq!(psyche.undo.as_ref().unwrap().qty, 3.0);
    career::undo_expense(&mut ch, &engine, &psyche.guid).unwrap();
    assert!(!gear(&ch, "b3591ce4-6700-4366-b094-16cba5b30f81"));
    assert_eq!(ch.nuyen, n0 + 714.0);

    // Generic spend with not enough nuyen.
    let err = career::spend_nuyen(&mut ch, 1e9, "Purchased Gear Yacht", NuyenExpenseType::AddGear, "", 1.0).unwrap_err();
    assert!(matches!(err, CareerError::NotEnoughNuyen { .. }));
    let g = career::spend_nuyen(&mut ch, 100.0, "Purchased Gear Thing", NuyenExpenseType::AddGear, "", 1.0).unwrap();
    assert_eq!(ch.nuyen, n0 + 614.0);
    career::undo_expense(&mut ch, &engine, &g).unwrap();
    assert_eq!(ch.nuyen, n0 + 714.0);

    // Sell the reagents at half price.
    let before = ch.nuyen;
    let got = career::sell_item(&mut ch, "1ee6f3a8-cf36-4dc2-a1a4-e9ed9d248e05", 0.5).unwrap();
    assert!(got > 0.0);
    assert!(!gear(&ch, "1ee6f3a8-cf36-4dc2-a1a4-e9ed9d248e05"));
    assert_eq!(ch.nuyen, before + got);
    let sale = entry_by_reason(&ch, "Sold Gear Reagents, per dram");
    assert!(sale.undo.is_none());
    let t = career::totals(&ch);
    assert!((t.nuyen_logged - ch.nuyen).abs() < 0.005);
}

#[test]
fn creation_mode_is_refused() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    ch.created = false;
    assert_eq!(career::improve_attribute(&mut ch, &engine, "BOD"), Err(CareerError::NotCareer));
}
