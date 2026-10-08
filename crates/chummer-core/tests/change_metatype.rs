//! "Change Priorities" / "Change Metatype" in creation mode
//! (`chargen::rebuild`, `Command::ChangeMetatype`).

use chummer_core::chargen::rebuild::Choice;
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::command::{self, Command, Session};
use chummer_core::engine::Engine;

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn engine() -> &'static Engine {
    static E: std::sync::OnceLock<Engine> = std::sync::OnceLock::new();
    E.get_or_init(|| Engine::load().unwrap())
}

fn new_character(metatype: &str, prios: [char; 5], talent: &str, skills: &[&str]) -> Character {
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: metatype.into(),
        metavariant: None,
        priorities: Priorities(prios),
        talent: talent.into(),
        talent_skills: skills.iter().map(|s| s.to_string()).collect(),
        name: "Runner".into(),
    };
    chargen::create(engine(), &spec).unwrap()
}

fn budget(ch: &Character) -> chargen::Budget {
    let rules = engine().rules_for(ch);
    let sheet = engine().sheet(ch);
    let settings = engine().settings.resolve(&ch.field("settings")).unwrap();
    chargen::budget(ch, &sheet, &rules, settings)
}

fn change(ch: &mut Character, choice: Choice) -> Result<command::Applied, command::Rejected> {
    command::apply(ch, engine(), &command::Envelope::new(Command::ChangeMetatype { choice }, 1, 1_700_000_000_000, ""))
}

fn attr(ch: &Character, n: &str) -> (i32, i32, i32, i32) {
    let a = ch.attribute(n).unwrap();
    (a.metatype_min, a.metatype_max, a.base, a.karma)
}

/// The same character made from scratch with the new choice gets the same
/// budgets as the changed one.
fn same_as_new(changed: &Character, choice: &Choice) {
    let fresh = new_character(&choice.metatype, choice.priorities.unwrap().0, &choice.talent, &choice.talent_skills.iter().map(String::as_str).collect::<Vec<_>>());
    let (a, b) = (budget(changed), budget(&fresh));
    assert_eq!((a.attribute_points.0, a.special_points.0, a.skill_points.0, a.skill_group_points.0), (b.attribute_points.0, b.special_points.0, b.skill_points.0, b.skill_group_points.0));
    assert_eq!(a.nuyen.0, b.nuyen.0);
    assert_eq!(a.karma.1, b.karma.1, "metatype karma and the rest");
    for n in ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES"] {
        assert_eq!(attr(changed, n).0..attr(changed, n).1, attr(&fresh, n).0..attr(&fresh, n).1, "{n} limits");
    }
    let quals = |c: &Character| {
        let mut q: Vec<(String, String)> = c.items("qualities", "quality").iter().map(|q| (q.get("name"), q.get("qualitysource"))).collect();
        q.sort();
        q
    };
    assert_eq!(quals(changed), quals(&fresh));
}

#[test]
fn swapping_two_priorities_moves_their_budgets() {
    let mut ch = new_character("Human", ['D', 'E', 'A', 'B', 'C'], "Mundane", &[]);
    assert!(command::apply(&mut ch, engine(), &command::Envelope::new(Command::SetAttributeBase { attribute: "BOD".into(), value: 3 }, 1, 0, "")).is_ok());
    let before = budget(&ch);
    let mut choice = Choice::of(&ch);
    assert_eq!(choice.priorities, Some(Priorities(['D', 'E', 'A', 'B', 'C'])));
    // Attributes A <-> Skills B.
    choice.priorities = Some(Priorities(['D', 'E', 'B', 'A', 'C']));
    let r = change(&mut ch, choice.clone()).unwrap();
    assert!(r.changed);
    assert_eq!(r.description, "Changed priorities (DEBAC) and metatype Human");
    let after = budget(&ch);
    assert_eq!((ch.field("priorityattributes"), ch.field("priorityskills")), ("B".into(), "A".into()));
    assert!(after.attribute_points.0 < before.attribute_points.0);
    assert!(after.skill_points.0 > before.skill_points.0);
    assert_eq!(attr(&ch, "BOD").2, 3, "points already spent stay");
    same_as_new(&ch, &choice);
    // The same choice again changes nothing.
    assert!(!change(&mut ch, choice).unwrap().changed);
}

#[test]
fn a_new_metatype_replaces_limits_and_racial_qualities() {
    let mut ch = new_character("Human", ['D', 'E', 'A', 'B', 'C'], "Mundane", &[]);
    command::apply(&mut ch, engine(), &command::Envelope::new(Command::SetAttributeBase { attribute: "AGI".into(), value: 5 }, 1, 0, "")).unwrap();
    let mut choice = Choice::of(&ch);
    choice.metatype = "Elf".into();
    change(&mut ch, choice.clone()).unwrap();
    assert_eq!(ch.field("metatype"), "Elf");
    let agi = attr(&ch, "AGI");
    assert_eq!((agi.0, agi.1), (2, 7));
    assert_eq!(agi.2, 5, "kept (Elf AGI 2..7 allows 5 points)");
    assert!(ch.items("qualities", "quality").iter().any(|q| q.get("name") == "Low-Light Vision" && q.get("qualitysource") == "Metatype"));
    same_as_new(&ch, &choice);
    // Back to human: the elf's vision goes, and a CHA bought up to the
    // elf's maximum is cut to the human one.
    command::apply(&mut ch, engine(), &command::Envelope::new(Command::SetAttributeBase { attribute: "CHA".into(), value: 7 }, 1, 0, "")).unwrap();
    choice.metatype = "Human".into();
    change(&mut ch, choice.clone()).unwrap();
    assert!(!ch.items("qualities", "quality").iter().any(|q| q.get("name") == "Low-Light Vision"));
    let cha = attr(&ch, "CHA");
    assert_eq!((cha.0, cha.1), (1, 6));
    assert_eq!(cha.2, 5, "cut to the human maximum (6 − 1)");
    same_as_new(&ch, &choice);
}

/// LB-46: a magician who changes metatype keeps the talent's MAG minimum.
#[test]
fn a_magician_keeps_the_talent_magic_after_a_new_metatype() {
    let mut ch = new_character("Human", ['C', 'B', 'A', 'D', 'E'], "Magician", &["Spellcasting", "Summoning"]);
    let mag = attr(&ch, "MAG");
    assert!(mag.0 > 1, "{mag:?}");
    let mut choice = Choice::of(&ch);
    assert_eq!(choice.talent_skills, ["Spellcasting", "Summoning"]);
    choice.metatype = "Dwarf".into();
    change(&mut ch, choice.clone()).unwrap();
    assert_eq!(attr(&ch, "MAG").0, mag.0);
    assert!(ch.items("qualities", "quality").iter().any(|q| q.get("name") == "Magician" && q.get("qualitysource") == "Heritage"));
    same_as_new(&ch, &choice);
    // Mundane now: the Magician quality and the free skills go.
    let mut mundane = choice.clone();
    mundane.priorities = Some(Priorities(['C', 'E', 'A', 'D', 'B']));
    mundane.talent = "Mundane".into();
    mundane.talent_skills.clear();
    change(&mut ch, mundane.clone()).unwrap();
    assert!(!ch.items("qualities", "quality").iter().any(|q| q.get("name") == "Magician"));
    assert!(!ch.improvements.list.iter().any(|i| i.source == "Heritage"));
    same_as_new(&ch, &mundane);
}

#[test]
fn invalid_choices_are_refused_and_change_nothing() {
    let base = new_character("Human", ['D', 'E', 'A', 'B', 'C'], "Mundane", &[]);
    let hash = command::state_hash(&base);
    let good = Choice::of(&base);
    let cases = [
        Choice { priorities: Some(Priorities(['A', 'A', 'B', 'C', 'D'])), ..good.clone() },
        Choice { metatype: "Troll".into(), ..good.clone() },
        Choice { priorities: Some(Priorities(['D', 'A', 'E', 'B', 'C'])), talent: "Magician".into(), talent_skills: vec!["Spellcasting".into()], ..good.clone() },
        Choice { priorities: Some(Priorities(['D', 'A', 'E', 'B', 'C'])), talent: "Magician".into(), talent_skills: vec!["Spellcasting".into(), "Spellcasting".into()], ..good.clone() },
        Choice { priorities: Some(Priorities(['D', 'A', 'E', 'B', 'C'])), talent: "Magician".into(), talent_skills: vec!["Spellcasting".into(), "Pistols".into()], ..good.clone() },
        Choice { metatype: "No Such Metatype".into(), ..good.clone() },
        Choice { priorities: None, ..good.clone() },
    ];
    for c in cases {
        let mut ch = base.clone();
        let e = change(&mut ch, c.clone()).expect_err(&format!("{c:?}"));
        assert!(!e.reason.is_empty());
        assert_eq!(command::state_hash(&ch), hash, "{c:?}");
    }
    // Career mode: refused.
    let mut career = Character::load(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Munin_Career.chum5")).unwrap();
    let c = Choice::of(&career);
    assert!(change(&mut career, c).unwrap_err().reason.contains("during creation"));
}

#[test]
fn karma_builds_change_the_metatype_and_its_karma() {
    let pb = engine().settings.presets.iter().find(|p| p.build_method() == "Karma").unwrap().clone();
    let spec = NewCharacter { settings_id: pb.key(), metatype: "Human".into(), metavariant: None, priorities: Priorities(['A', 'B', 'C', 'D', 'E']), talent: "Mundane".into(), talent_skills: vec![], name: "PB".into() };
    let mut ch = chargen::create(engine(), &spec).unwrap();
    let mut choice = Choice::of(&ch);
    assert!(choice.priorities.is_none());
    choice.metatype = "Ork".into();
    let r = change(&mut ch, choice).unwrap();
    assert_eq!(r.description, "Changed metatype to Ork");
    let fresh = chargen::create(engine(), &NewCharacter { metatype: "Ork".into(), ..spec }).unwrap();
    assert_eq!(ch.field("metatypebp"), fresh.field("metatypebp"));
    assert_eq!(budget(&ch).karma, budget(&fresh).karma);
}

#[test]
fn undo_restores_everything() {
    let ch = new_character("Human", ['D', 'E', 'A', 'B', 'C'], "Mundane", &[]);
    let hash = command::state_hash(&ch);
    let mut s = Session::with_seed(ch.clone(), 3);
    let mut choice = Choice::of(&ch);
    choice.metatype = "Elf".into();
    choice.priorities = Some(Priorities(['D', 'E', 'B', 'A', 'C']));
    s.apply(engine(), Command::ChangeMetatype { choice }).unwrap();
    assert_ne!(s.state_hash(), hash);
    s.undo().unwrap();
    assert_eq!(s.state_hash(), hash);
    // The saved file loads with the change.
    s.redo().unwrap();
    let back = Character::from_str(&s.ch().to_xml_string()).unwrap();
    assert_eq!(back.field("metatype"), "Elf");
}

/// A Chummer save (Miko: an elf magician, priorities saved as "D,1"):
/// swapping Attributes and Skills keeps everything else as it was.
#[test]
fn swapping_priorities_on_a_chummer_save() {
    let mut ch = Character::load(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Miko.chum5")).unwrap();
    let before = budget(&ch);
    let quals = |c: &Character| c.items("qualities", "quality").iter().map(|q| q.get("guid")).collect::<Vec<_>>();
    let (q0, imps0, nuyen0) = (quals(&ch), ch.improvements.list.len(), ch.nuyen);
    let mut choice = Choice::of(&ch);
    assert_eq!(choice.priorities, Some(Priorities(['D', 'A', 'B', 'C', 'E'])));
    assert_eq!(choice.talent, "Magician");
    choice.priorities = Some(Priorities(['D', 'A', 'C', 'B', 'E']));
    change(&mut ch, choice).unwrap();
    let after = budget(&ch);
    assert_eq!((after.attribute_points.0, after.skill_points.0), (16, 36), "{:?} -> {:?}", before.attribute_points, after.attribute_points);
    assert_eq!(after.special_points.0, before.special_points.0);
    assert_eq!(after.nuyen.0, before.nuyen.0);
    assert_eq!((ch.field("prioritymetatype"), ch.field("priorityattributes"), ch.field("priorityskills")), ("D,1".into(), "C".into(), "B".into()));
    assert_eq!(quals(&ch), q0, "the same qualities, untouched");
    assert_eq!(ch.improvements.list.len(), imps0);
    assert_eq!(Choice::of(&ch).talent_skills, ["Counterspelling", "Spellcasting"]);
    assert_eq!(ch.nuyen, nuyen0, "Resources did not change: what is left stays");
    // Resources E <-> Attributes C: what is left moves by the difference.
    let mut choice = Choice::of(&ch);
    choice.priorities = Some(Priorities(['D', 'A', 'E', 'B', 'C']));
    let start0 = budget(&ch).nuyen.0;
    change(&mut ch, choice).unwrap();
    let start1 = budget(&ch).nuyen.0;
    assert!(start1 > start0);
    assert_eq!(ch.nuyen - nuyen0, start1 - start0);
}
