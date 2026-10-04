//! Custom improvements from the Improvements tab (`CreateImprovement`).

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::custom_improvement::{self as custom, CustomError, Form};
use chummer_core::engine::Engine;

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn agi(engine: &Engine, ch: &Character) -> i32 {
    engine.sheet(ch).attributes.iter().find(|a| a.name == "AGI").unwrap().total
}

fn plus_one_agility() -> Form {
    Form { type_id: "specificattribute".into(), name: "GM bonus".into(), select: "AGI".into(), val: 1.0, ..Default::default() }
}

fn index_of(ch: &Character, guid: &str) -> usize {
    ch.improvements.list.iter().position(|i| i.source_name == guid).unwrap()
}

#[test]
fn types_load_from_data() {
    let engine = Engine::load().unwrap();
    let types = custom::types(&engine.store);
    assert!(types.len() > 80, "{} types", types.len());
    let t = custom::find_type(&engine.store, "specificattribute").unwrap();
    assert_eq!(t.internal, "specificattribute");
    assert!(t.has(&custom::Field::Val) && t.selection().is_some());
    let walk = custom::find_type(&engine.store, "walkmultiplier").unwrap();
    let ch = fixture("Glessner.chum5");
    assert_eq!(custom::options(&ch, &engine.store, None, walk.selection().unwrap()), ["Fly", "Ground", "Swim"]);
    let attrs = custom::options(&ch, &engine.store, None, t.selection().unwrap());
    assert!(attrs.contains(&"AGI".to_owned()) && !attrs.contains(&"ESS".to_owned()));
}

#[test]
fn every_type_builds_a_bonus() {
    let engine = Engine::load().unwrap();
    for t in custom::types(&engine.store) {
        let f = Form { type_id: t.id.clone(), name: "x".into(), select: "AGI".into(), val: 1.0, ..Default::default() };
        let b = custom::bonus_xml(&t, &f).unwrap_or_else(|e| panic!("{}: {e}", t.id));
        assert_eq!(b.elements().next().unwrap().name, t.internal, "{}", t.id);
    }
}

#[test]
fn custom_agility_changes_the_attribute_in_both_modes() {
    let engine = Engine::load().unwrap();
    for name in ["Glessner.chum5", "Fuzzy-chargen.chum5"] {
        let mut ch = fixture(name);
        let before = agi(&engine, &ch);
        let guid = custom::create(&mut ch, &engine.store, &plus_one_agility(), "", None).unwrap();
        assert_eq!(agi(&engine, &ch), before + 1, "{name}");
        let n = index_of(&ch, &guid);
        let i = &ch.improvements.list[n];
        assert!(i.custom && i.enabled);
        assert_eq!((i.kind.as_str(), i.improved_name.as_str(), i.source.as_str()), ("Attribute", "AGI", "Custom"));
        assert_eq!((i.custom_name.as_str(), i.custom_id.as_str()), ("GM bonus", "specificattribute"));

        // disabling reverts, enabling restores
        assert!(custom::set_enabled(&mut ch, n, false));
        assert_eq!(agi(&engine, &ch), before, "{name}");
        assert!(custom::set_enabled(&mut ch, n, true));
        assert_eq!(agi(&engine, &ch), before + 1);

        // a group toggles together
        assert!(custom::add_group(&mut ch, "Drugs"));
        custom::set_group(&mut ch, n, "Drugs");
        assert_eq!(custom::set_group_enabled(&mut ch, "Drugs", false), 1);
        assert_eq!(agi(&engine, &ch), before);
        assert_eq!(custom::set_group_enabled(&mut ch, "Drugs", true), 1);

        assert!(custom::remove(&mut ch, &guid));
        assert_eq!(agi(&engine, &ch), before);
        assert!(custom::listed(&ch).iter().all(|&k| ch.improvements.list[k].source_name != guid));
    }
}

#[test]
fn save_and_load_keep_custom_improvements() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Glessner.chum5");
    let before = agi(&engine, &ch);
    assert!(custom::add_group(&mut ch, "Session 12"));
    let guid = custom::create(&mut ch, &engine.store, &plus_one_agility(), "Session 12", None).unwrap();
    let n = index_of(&ch, &guid);
    custom::set_notes(&mut ch, n, "until the end of the run");
    custom::set_enabled(&mut ch, n, false);

    let xml = ch.to_xml_string();
    for tag in ["<custom>True</custom>", "<customname>GM bonus</customname>", "<customid>specificattribute</customid>", "<customgroup>Session 12</customgroup>", "<improvementgroup>Session 12</improvementgroup>"] {
        assert!(xml.contains(tag), "missing {tag}");
    }
    let mut back = Character::from_str(&xml).unwrap();
    assert_eq!(back.improvements.list, ch.improvements.list);
    assert_eq!(custom::groups(&back), ["Session 12"]);
    let i = &back.improvements.list[n];
    assert!(!i.enabled && i.custom);
    assert_eq!(i.notes, "until the end of the run");
    assert_eq!(agi(&engine, &back), before);
    custom::set_enabled(&mut back, n, true);
    assert_eq!(agi(&engine, &back), before + 1);
    // and a second round trip is stable
    assert_eq!(Character::from_str(&back.to_xml_string()).unwrap().improvements.list, back.improvements.list);
}

#[test]
fn edit_replaces_and_keeps_notes() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Glessner.chum5");
    let before = agi(&engine, &ch);
    let guid = custom::create(&mut ch, &engine.store, &plus_one_agility(), "", None).unwrap();
    let n = index_of(&ch, &guid);
    custom::set_notes(&mut ch, n, "note");
    ch.improvements.list[n].order = 3;

    let t = custom::find_type(&engine.store, "specificattribute").unwrap();
    let mut f = Form::from_improvement(&ch.improvements.list[n], Some(&t));
    assert_eq!(f, plus_one_agility());
    f.val = 2.0;
    let g2 = custom::create(&mut ch, &engine.store, &f, "", Some(&guid)).unwrap();
    assert!(ch.improvements.list.iter().all(|i| i.source_name != guid));
    let i = &ch.improvements.list[index_of(&ch, &g2)];
    assert_eq!((i.notes.as_str(), i.order), ("note", 3));
    assert_eq!(agi(&engine, &ch), before + 2);
}

#[test]
fn validation_and_groups() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Glessner.chum5");
    let mut f = plus_one_agility();
    f.name.clear();
    assert_eq!(custom::create(&mut ch, &engine.store, &f, "", None), Err(CustomError::NoName));
    f.select.clear();
    assert_eq!(custom::create(&mut ch, &engine.store, &f, "", None), Err(CustomError::NoSelection));

    assert!(custom::add_group(&mut ch, "A"));
    assert!(!custom::add_group(&mut ch, "A"));
    let g = custom::create(&mut ch, &engine.store, &plus_one_agility(), "A", None).unwrap();
    assert!(custom::rename_group(&mut ch, "A", "B"));
    assert_eq!(ch.improvements.list[index_of(&ch, &g)].custom_group, "B");
    assert!(custom::remove_group(&mut ch, "B"));
    assert!(custom::groups(&ch).is_empty());
    assert_eq!(ch.improvements.list[index_of(&ch, &g)].custom_group, "");
}

#[test]
fn deleting_undoes_created_objects_and_flags() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Fuzzy-chargen.chum5");
    // Enable Special Attribute (RES), then delete it.
    let had_res = ch.res_enabled();
    let f = Form { type_id: "enableattribute".into(), name: "Emerged".into(), select: "RES".into(), ..Default::default() };
    let g = custom::create(&mut ch, &engine.store, &f, "", None).unwrap();
    assert!(ch.res_enabled());
    custom::remove(&mut ch, &g);
    assert_eq!(ch.res_enabled(), had_res);

    // A free spell is created and removed with its improvement.
    let mut ch = fixture("Glessner.chum5");
    let spells = ch.items("spells", "spell").len();
    let f = Form { type_id: "addspell".into(), name: "Gift".into(), select: "Fireball".into(), ..Default::default() };
    let g = custom::create(&mut ch, &engine.store, &f, "", None).unwrap();
    assert_eq!(ch.items("spells", "spell").len(), spells + 1);
    custom::remove(&mut ch, &g);
    assert_eq!(ch.items("spells", "spell").len(), spells);
}
