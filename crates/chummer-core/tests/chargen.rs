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
