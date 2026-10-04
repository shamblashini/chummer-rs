use chummer_core::calc;
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn sheet(engine: &Engine, ch: &Character) -> (calc::Sheet, chargen::Budget) {
    let rules = engine.rules_for(ch);
    let sheet = calc::compute(ch, &rules, Some(&engine.store), Some(&engine.catalog));
    let settings = engine.settings.resolve(&ch.field("settings")).unwrap();
    let b = chargen::budget(ch, &sheet, &rules, settings);
    (sheet, b)
}

#[test]
fn mundane_human_priority() {
    let engine = Engine::load().unwrap();
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Human".into(),
        metavariant: None,
        // Heritage D, Talent E, Attributes A, Skills B, Resources C
        priorities: Priorities(['D', 'E', 'A', 'B', 'C']),
        talent: "Mundane".into(),
        talent_skills: vec![],
        name: "Test Runner".into(),
    };
    let ch = chargen::create(&engine, &spec).unwrap();
    let (s, b) = sheet(&engine, &ch);
    assert_eq!(b.attribute_points, (24, 0));
    assert_eq!(b.special_points, (3, 0));
    assert_eq!(b.skill_points, (36, 0));
    assert_eq!(b.skill_group_points, (5, 0));
    assert_eq!(b.nuyen, (140000.0, 0.0));
    assert_eq!(b.karma, (25, 0));
    assert_eq!(s.attr("BOD"), 1);
    assert_eq!(s.attr("EDG"), 2, "human EDG minimum is 2");
    assert!(!ch.mag_enabled());
    assert!(ch.skills.len() > 60, "all active skills present");
    // The file reloads to the same state.
    let again = Character::from_str(&ch.to_xml_string()).unwrap();
    assert_eq!(again.attributes, ch.attributes);
    assert_eq!(again.skills.len(), ch.skills.len());
    assert_eq!(again.field("settings"), STANDARD);
    assert_eq!(again.field("appversion"), chargen::CHUMMER_APP_VERSION);
    assert!(chargen::validity_problems(&ch, &b, engine.settings.resolve(STANDARD).unwrap()).is_empty());
}

#[test]
fn elf_magician_gets_magic_and_free_skills() {
    let engine = Engine::load().unwrap();
    let talents = chargen::talent_options(&engine.store, engine.settings.resolve(STANDARD).unwrap(), 'A');
    let mage = talents.iter().find(|t| t.value == "Magician").unwrap();
    let options = chargen::talent_skill_options(&engine.store, mage);
    assert!(options.contains(&"Spellcasting".to_owned()));
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Elf".into(),
        metavariant: None,
        priorities: Priorities(['B', 'A', 'C', 'D', 'E']),
        talent: "Magician".into(),
        talent_skills: vec!["Spellcasting".into(), "Summoning".into()],
        name: "Mage".into(),
    };
    let mut ch = chargen::create(&engine, &spec).unwrap();
    assert!(ch.mag_enabled(), "Magician quality's enableattribute ran");
    assert!(ch.is_magician());
    assert_eq!(ch.field("essenceatspecialstart"), "6");
    let (s, b) = sheet(&engine, &ch);
    assert_eq!(s.attr("MAG"), 6);
    let spell = s.skills.iter().find(|k| k.name == "Spellcasting").unwrap();
    assert_eq!(spell.rating, 5, "free talent skill at 5");
    assert_eq!(b.free_spells.0, 10);
    // Low-light vision is a racial quality and costs nothing.
    assert!(ch.items("qualities", "quality").iter().any(|q| q.get("name") == "Low-Light Vision"));
    assert_eq!(b.positive_quality_karma, 0);

    // Add a quality and check karma, then remove it again.
    let qdoc = engine.store.doc("qualities.xml").unwrap();
    let rec = data::find(&qdoc, "qualities", "quality", "Ambidextrous").unwrap();
    let karma = rec.el().get_i32("karma").unwrap();
    let guid = chargen::add_quality(&mut ch, &engine.store, rec, None);
    let (_, b2) = sheet(&engine, &ch);
    assert_eq!(b2.karma.1, b.karma.1 + karma);
    assert!(ch.improvements.list.iter().any(|i| i.kind == "Ambidextrous"));
    chargen::remove_quality(&mut ch, &guid);
    assert!(!ch.improvements.list.iter().any(|i| i.kind == "Ambidextrous"));

    // Finish creation.
    let (_, b3) = sheet(&engine, &ch);
    let settings = engine.settings.resolve(STANDARD).unwrap().clone();
    chargen::finalize(&mut ch, &b3, &settings);
    assert!(ch.created);
    assert_eq!(ch.karma, 7, "unspent karma carries over up to 7");
    assert_eq!(ch.nuyen, 5000.0, "unspent nuyen carries over up to 5000");
    assert_eq!(ch.items("expenses", "expense").len(), 2);
}

#[test]
fn sum_to_ten_validation() {
    let engine = Engine::load().unwrap();
    let s2t = engine.settings.presets.iter().find(|p| p.build_method() == "SumtoTen").unwrap();
    assert!(Priorities(['A', 'A', 'C', 'E', 'E']).validate(s2t).is_ok()); // 4+4+2+0+0
    assert!(Priorities(['A', 'A', 'A', 'E', 'E']).validate(s2t).is_err());
    let std = engine.settings.resolve(STANDARD).unwrap();
    assert!(Priorities(['A', 'A', 'C', 'E', 'E']).validate(std).is_err());
}

#[test]
fn creation_rules_for_specs_gender_and_magic_skills() {
    let engine = Engine::load().unwrap();
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Human".into(),
        metavariant: None,
        priorities: Priorities(['D', 'E', 'A', 'B', 'C']),
        talent: "Mundane".into(),
        talent_skills: vec![],
        name: "Rules".into(),
    };
    let mut ch = chargen::create(&engine, &spec).unwrap();
    // New files use <gender>; old ones keep <sex>.
    ch.set_field("sex", "Female");
    assert_eq!(ch.doc.get("gender"), "Female");
    assert!(ch.doc.child("sex").is_none());

    // Spellcasting is present but disabled for a mundane.
    let (s, _) = sheet(&engine, &ch);
    assert!(s.skills.iter().find(|k| k.name == "Spellcasting").unwrap().disabled);
    assert!(!s.skills.iter().find(|k| k.name == "Pistols").unwrap().disabled);

    // Two specializations on one skill block finishing creation.
    let guid = ch.skills.iter().find(|k| engine.catalog.get(&k.suid).is_some_and(|d| d.name == "Pistols")).unwrap().guid.clone();
    chargen::add_specialization(&mut ch, &guid, "Revolvers");
    chargen::add_specialization(&mut ch, &guid, "Semi-Automatics");
    let (_, b) = sheet(&engine, &ch);
    let problems = chargen::validity_problems(&ch, &b, engine.settings.resolve(STANDARD).unwrap());
    assert!(problems.iter().any(|p| p.contains("Pistols has more than one specialization")), "{problems:?}");
}

#[test]
fn karma_point_buy_build() {
    let engine = Engine::load().unwrap();
    let pb = engine.settings.presets.iter().find(|p| p.build_method() == "Karma").unwrap().clone();
    let elf_karma = chargen::karma_metatypes(&engine.store).into_iter().find(|m| m.metatype == "Elf").unwrap().karma;
    let spec = NewCharacter {
        settings_id: pb.id(),
        metatype: "Elf".into(),
        metavariant: None,
        priorities: Priorities(['A', 'B', 'C', 'D', 'E']),
        talent: "Mundane".into(),
        talent_skills: vec![],
        name: "Point Buy".into(),
    };
    let mut ch = chargen::create(&engine, &spec).unwrap();
    let (_, b) = sheet(&engine, &ch);
    assert_eq!(b.karma, (800, elf_karma), "metatype costs karma");
    assert_eq!(b.attribute_points.0, 0);
    assert_eq!(b.skill_points.0, 0);
    // Raising an attribute with karma costs karma.
    ch.attribute_mut("AGI").unwrap().karma = 1;
    let (_, b2) = sheet(&engine, &ch);
    assert_eq!(b2.karma.1, elf_karma + 3 * 5, "AGI 2 -> 3 at 5 karma per point");
    // Becoming a magician is a quality.
    let qdoc = engine.store.doc("qualities.xml").unwrap();
    let mage = chummer_core::data::find(&qdoc, "qualities", "quality", "Magician").unwrap();
    chargen::add_quality(&mut ch, &engine.store, mage, None);
    assert!(ch.mag_enabled() && ch.is_magician());
    let (_, b3) = sheet(&engine, &ch);
    assert!(b3.karma.1 > b2.karma.1);
}

#[test]
fn life_modules_build() {
    let engine = Engine::load().unwrap();
    let lm = engine.settings.presets.iter().find(|p| p.build_method() == "LifeModule").unwrap().clone();
    let spec = NewCharacter {
        settings_id: lm.id(),
        metatype: "Human".into(),
        metavariant: None,
        priorities: Priorities(['A', 'B', 'C', 'D', 'E']),
        talent: "Mundane".into(),
        talent_skills: vec![],
        name: "Life".into(),
    };
    let mut ch = chargen::create(&engine, &spec).unwrap();
    let (stages, modules) = chargen::life_modules(&engine.store);
    assert_eq!(stages.first().map(String::as_str), Some("Nationality"));
    let ucas = modules.iter().find(|m| m.name == "United Canadian American States").unwrap();
    let (_, before) = sheet(&engine, &ch);
    let v = ucas.versions.first().map(|v| v.0.clone());
    chargen::add_life_module(&mut ch, &engine.store, &ucas.id, v.as_deref()).unwrap();
    let (s, after) = sheet(&engine, &ch);
    assert_eq!(after.karma.1, before.karma.1 + ucas.karma);
    assert_eq!(after.positive_quality_karma, before.positive_quality_karma, "life modules don't count toward the quality limit");
    // General UCAS gives +1 LOG and a level of Etiquette.
    assert_eq!(s.attr_values("LOG").unwrap().free_base, 1);
    assert!(ch.improvements.list.iter().any(|i| i.kind == "SkillLevel" && i.improved_name == "Etiquette"));
    let q = ch.items("qualities", "quality").into_iter().find(|q| q.get("qualitytype") == "LifeModule").unwrap();
    assert_eq!(q.get("stage"), "Nationality");
}
