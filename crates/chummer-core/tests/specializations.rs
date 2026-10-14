//! Skill specializations: free text in creation and career, the
//! suggestions, the weapon-pool bonus (`Weapon.GetDicePool`'s
//! `GetSpecialization` chain) and the save round trip.

use std::path::Path;

use chummer_core::career::{self, CareerRules, ManualExpense};
use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, RecordRef};
use chummer_core::engine::Engine;
use chummer_core::items::{weapon, Purchase};
use chummer_core::skills::{self, SpecSource};

fn load(name: &str) -> Character {
    Character::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.chum5"))).unwrap()
}

fn s(v: &str) -> String {
    v.to_owned()
}

fn run(ch: &mut Character, engine: &Engine, cmd: Command) -> Result<command::Applied, command::Rejected> {
    command::apply(ch, engine, &Envelope::new(cmd, 7, 3_000_000_000_000, ""))
}

fn guid(engine: &Engine, ch: &Character, name: &str) -> String {
    engine.sheet(ch).skills.iter().find(|x| x.name == name).unwrap_or_else(|| panic!("no skill {name}")).guid.clone()
}

fn specs(ch: &Character, guid: &str) -> Vec<String> {
    ch.skills.iter().find(|x| x.guid == guid).unwrap().specs.iter().map(|x| x.name.clone()).collect()
}

fn add_weapon(ch: &mut Character, engine: &Engine, name: &str) {
    let purchase = Purchase { qty: 1.0, cost_multiplier: 1.0, free: true, ..Default::default() };
    run(ch, engine, Command::AddItem { tag: s("weapon"), record: RecordRef { id: String::new(), name: s(name) }, purchase }).unwrap();
}

fn pool(engine: &Engine, ch: &Character, weapon_name: &str) -> (i32, Option<String>) {
    let sheet = engine.sheet(ch);
    let w = ch.items("weapons", "weapon").into_iter().find(|w| w.get("name") == weapon_name).unwrap_or_else(|| panic!("no weapon {weapon_name}"));
    let st = weapon::stats(ch, &sheet, w);
    (st.dice_pool, st.spec)
}

#[test]
fn creation_takes_any_text_and_costs_a_skill_point() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 3 }).unwrap();
    let settings = engine.settings().resolve_for(&ch).unwrap().clone();
    let points = |ch: &Character| {
        let sheet = engine.sheet(ch);
        chummer_core::chargen::budget(ch, &sheet, &engine.rules_for(ch), &settings).skill_points.1
    };
    let before = points(&ch);
    // Not in the data's list: any text is a specialization.
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("  Kicks ") }).unwrap();
    assert_eq!(specs(&ch, &unarmed), ["Kicks"], "trimmed");
    assert_eq!(points(&ch), before + 1, "one skill point (priority/sum-to-ten build)");
    let v = engine.sheet(&ch).skills.into_iter().find(|x| x.guid == unarmed).unwrap();
    assert_eq!(v.spec_bonus, 2, "the skill's specialized pool is pool+2");
    // Retyping replaces it (Chummer's one creation spec), not a second one.
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Knucks") }).unwrap();
    assert_eq!(specs(&ch, &unarmed), ["Knucks"]);
    assert_eq!(points(&ch), before + 1);
    // The same text again changes nothing.
    let same = run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Knucks") }).unwrap();
    assert!(!same.changed);
    // Empty text removes it.
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: String::new() }).unwrap();
    assert!(specs(&ch, &unarmed).is_empty());
    assert_eq!(points(&ch), before);
}

#[test]
fn creation_refusals() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 0 }).unwrap();
    run(&mut ch, &engine, Command::SetSkillKarma { skill: unarmed.clone(), value: 0 }).unwrap();
    let e = run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Kicks") }).unwrap_err();
    assert!(e.reason.contains("needs a rating"), "{}", e.reason);
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 2 }).unwrap();
    assert!(run(&mut ch, &engine, Command::AddSpecialization { skill: unarmed.clone(), name: s("   ") }).is_err(), "empty");
    run(&mut ch, &engine, Command::AddSpecialization { skill: unarmed.clone(), name: s("Kicks") }).unwrap();
    let e = run(&mut ch, &engine, Command::AddSpecialization { skill: unarmed.clone(), name: s("kicks") }).unwrap_err();
    assert!(e.reason.contains("already has"), "{}", e.reason);
    assert_eq!(specs(&ch, &unarmed), ["Kicks"]);
    // Career has no creation box.
    let mut career = load("Munin_Career");
    let g = guid(&engine, &career, "Unarmed Combat");
    assert!(run(&mut career, &engine, Command::SetSpecialization { skill: g, name: s("Kicks") }).is_err());
}

#[test]
fn karma_build_pays_seven_karma() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 2 }).unwrap();
    ch.skills.iter_mut().find(|x| x.guid == unarmed).unwrap().buy_with_karma = true;
    let cost = |ch: &Character| engine.sheet(ch).skills.into_iter().find(|x| x.guid == unarmed).unwrap().karma_cost;
    let before = cost(&ch);
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Kicks") }).unwrap();
    assert_eq!(cost(&ch), before + 7, "Skill.CurrentKarmaCost: KarmaSpecialization per spec when bought with karma");
}

#[test]
fn career_buys_any_text_for_karma_and_undoes() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    career::karma_gained(&mut ch, &CareerRules::default(), &ManualExpense { amount: 30.0, reason: s("Run"), ..Default::default() }).unwrap();
    let pistols = guid(&engine, &ch, "Pistols");
    let karma = ch.karma;
    assert_eq!(career::specialization_karma_cost(&engine, &ch, &pistols), Some(7));
    run(&mut ch, &engine, Command::BuySpecialization { skill: pistols.clone(), name: s("Ares Predator V") }).unwrap();
    assert_eq!(ch.karma, karma - 7);
    assert!(specs(&ch, &pistols).contains(&s("Ares Predator V")));
    let entry = career::entries(&ch).into_iter().find(|e| e.reason == "Learned Specialization Pistols (Ares Predator V)").expect("ledger entry");
    // Duplicates and empty names are refused and cost nothing.
    assert!(run(&mut ch, &engine, Command::BuySpecialization { skill: pistols.clone(), name: s("ares predator v") }).is_err());
    assert!(run(&mut ch, &engine, Command::BuySpecialization { skill: pistols.clone(), name: s("") }).is_err());
    assert_eq!(ch.karma, karma - 7);
    // Career removal is the ledger's undo, as in Chummer.
    run(&mut ch, &engine, Command::UndoExpense { entry: entry.guid }).unwrap();
    assert_eq!(ch.karma, karma);
    assert!(!specs(&ch, &pistols).contains(&s("Ares Predator V")));
}

#[test]
fn weapon_pool_uses_a_spec_naming_the_weapon_exactly() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let pistols = guid(&engine, &ch, "Pistols");
    run(&mut ch, &engine, Command::SetSkillBase { skill: pistols.clone(), value: 3 }).unwrap();
    add_weapon(&mut ch, &engine, "Ares Predator V");
    let (base, spec) = pool(&engine, &ch, "Ares Predator V");
    assert_eq!(spec, None);
    // Not the weapon's name, category or data spec: no bonus.
    for other in ["Ares Predator", "ares predator v", "Revolvers", "Holdouts"] {
        run(&mut ch, &engine, Command::SetSpecialization { skill: pistols.clone(), name: s(other) }).unwrap();
        assert_eq!(pool(&engine, &ch, "Ares Predator V"), (base, None), "{other}");
    }
    // The weapon's name (custom text), its category, its data spec.
    for hit in ["Ares Predator V", "Heavy Pistols", "Semi-Automatics"] {
        run(&mut ch, &engine, Command::SetSpecialization { skill: pistols.clone(), name: s(hit) }).unwrap();
        assert_eq!(pool(&engine, &ch, "Ares Predator V"), (base + 2, Some(s(hit))), "{hit}");
    }
    // An expertise adds 3.
    ch.skills.iter_mut().find(|x| x.guid == pistols).unwrap().specs[0].expertise = true;
    assert_eq!(pool(&engine, &ch, "Ares Predator V").0, base + 3);
}

#[test]
fn unarmed_knucks_matches_the_knucks_and_kicks_matches_nothing() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 3 }).unwrap();
    add_weapon(&mut ch, &engine, "Knucks");
    let (knucks, _) = pool(&engine, &ch, "Knucks");
    let (strike, _) = pool(&engine, &ch, "Unarmed Attack");
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Kicks") }).unwrap();
    assert_eq!(pool(&engine, &ch, "Knucks"), (knucks, None), "no weapon is a kick");
    assert_eq!(pool(&engine, &ch, "Unarmed Attack"), (strike, None));
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Knucks") }).unwrap();
    assert_eq!(pool(&engine, &ch, "Knucks"), (knucks + 2, Some(s("Knucks"))), "the weapon's name");
    assert_eq!(pool(&engine, &ch, "Unarmed Attack"), (strike, None));
}

#[test]
fn suggestions_expand_martial_arts_and_list_weapons() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    let names = |ch: &Character, g: &str, src: SpecSource| skills::spec_suggestions(&engine, ch, g).into_iter().filter(|x| x.source == src).map(|x| x.name).collect::<Vec<_>>();
    assert_eq!(names(&ch, &unarmed, SpecSource::Data), ["Blocking", "Cyber Implants", "Subduing Combat"], "no [Martial Art] placeholder");
    assert!(names(&ch, &unarmed, SpecSource::MartialArt).is_empty());
    let purchase = Purchase { qty: 1.0, cost_multiplier: 1.0, ..Default::default() };
    run(&mut ch, &engine, Command::AddItem { tag: s("martialart"), record: RecordRef { id: String::new(), name: s("Krav Maga") }, purchase }).unwrap();
    // Chummer: the art's `addskillspecializationoption`; the [Martial Art]
    // expansion finds the same name and leaves it there.
    assert_eq!(names(&ch, &unarmed, SpecSource::Granted), ["Krav Maga"]);
    assert!(names(&ch, &unarmed, SpecSource::MartialArt).is_empty());
    // The character's own weapons for the skill, then the data's.
    assert_eq!(names(&ch, &unarmed, SpecSource::OwnWeapon), ["Unarmed Attack"]);
    let data_weapons = names(&ch, &unarmed, SpecSource::Weapon);
    // weapons.xml: spec Cyber Implants, hidden records left out.
    assert_eq!(data_weapons, ["Forearm Snap-Blades", "Hand Razors"]);
    // Taken ones are left out.
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 2 }).unwrap();
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Blocking") }).unwrap();
    assert!(!names(&ch, &unarmed, SpecSource::Data).contains(&s("Blocking")));
    // Pistols: SelectSpec's weapons.xml list (spec or spec2 in the data's specs).
    let pistols = guid(&engine, &ch, "Pistols");
    let w = names(&ch, &pistols, SpecSource::Weapon);
    assert!(w.contains(&s("Ares Predator V")), "Semi-Automatics");
    assert!(names(&ch, &pistols, SpecSource::OwnWeapon).contains(&s("Defiance EX Shocker")));
    // Not a combat skill: data specs only.
    let perception = guid(&engine, &ch, "Perception");
    assert!(skills::spec_suggestions(&engine, &ch, &perception).iter().all(|x| x.source == SpecSource::Data));
}

#[test]
fn custom_specs_round_trip_through_chum5_as_chummer_writes_them() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let unarmed = guid(&engine, &ch, "Unarmed Combat");
    run(&mut ch, &engine, Command::SetSkillBase { skill: unarmed.clone(), value: 2 }).unwrap();
    run(&mut ch, &engine, Command::SetSpecialization { skill: unarmed.clone(), name: s("Kicks & <Knees>") }).unwrap();
    let xml = ch.to_xml_string();
    // `SkillSpecialization.Save`: guid, name, free, expertise.
    let sp = ch.skills.iter().find(|x| x.guid == unarmed).unwrap().specs[0].clone();
    let want = format!("<spec><guid>{}</guid><name>Kicks &amp; &lt;Knees&gt;</name><free>False</free><expertise>False</expertise></spec>", sp.guid);
    assert!(xml.split_whitespace().collect::<String>().contains(&want.split_whitespace().collect::<String>()), "{want}");
    let back = Character::from_str(&xml).unwrap();
    assert_eq!(specs(&back, &unarmed), ["Kicks & <Knees>"]);
    assert_eq!(back.to_xml_string(), xml);
}

#[test]
fn disabled_specialization_effects_take_the_bonus_away() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin");
    let pistols = guid(&engine, &ch, "Pistols");
    run(&mut ch, &engine, Command::SetSkillBase { skill: pistols.clone(), value: 4 }).unwrap();
    add_weapon(&mut ch, &engine, "Ares Predator V");
    run(&mut ch, &engine, Command::SetSpecialization { skill: pistols.clone(), name: s("Ares Predator V") }).unwrap();
    let (with_spec, _) = pool(&engine, &ch, "Ares Predator V");
    // Loss of Confidence's `<disablespecializationeffects />` is empty, which
    // Chummer's selectskill reads as False (LB-47): only the -2 applies.
    let purchase = Purchase { qty: 1.0, cost_multiplier: 1.0, answer: Some(s("Pistols")), ..Default::default() };
    run(&mut ch, &engine, Command::AddItem { tag: s("quality"), record: RecordRef { id: String::new(), name: s("Loss of Confidence") }, purchase }).unwrap();
    assert_eq!(pool(&engine, &ch, "Ares Predator V"), (with_spec - 2, Some(s("Ares Predator V"))));
    // A DisableSpecializationEffects improvement (as `<skill>` bonuses make)
    // takes the bonus off the skill and the weapon.
    let imp = chummer_core::improvement::Improvement { kind: s("DisableSpecializationEffects"), improved_name: s("Pistols"), enabled: true, ..Default::default() };
    ch.improvements.list.push(imp);
    let v = engine.sheet(&ch).skills.into_iter().find(|x| x.guid == pistols).unwrap();
    assert_eq!(v.spec_bonus, 0);
    assert_eq!(pool(&engine, &ch, "Ares Predator V"), (with_spec - 4, None));
}
