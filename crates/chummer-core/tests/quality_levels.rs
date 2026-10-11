//! Qualities with levels (`<limit>` > 1 and no `<nolevels>`): each level
//! is one more `<quality>` element, as Chummer saves them; karma and
//! `<bonus>` count per copy, `<firstlevelbonus>` once.

use chummer_core::calc;
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::command::{Command, RecordRef, Session};
use chummer_core::engine::Engine;
use chummer_core::items::{self, Purchase};
use chummer_core::sections;
use chummer_core::tree::{self, Entry};

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn mystic_adept(engine: &Engine) -> Character {
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Human".into(),
        metavariant: None,
        priorities: Priorities(['E', 'B', 'A', 'C', 'D']),
        talent: "Mystic Adept".into(),
        talent_skills: vec!["Spellcasting".into(), "Summoning".into()],
        name: "Levels".into(),
    };
    chargen::create(engine, &spec).unwrap()
}

fn add(name: &str, level: i32) -> Command {
    Command::AddItem { tag: "quality".into(), record: RecordRef { id: String::new(), name: name.into() }, purchase: Purchase { rating: level, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } }
}

fn copies<'a>(ch: &'a Character, name: &str) -> Vec<&'a chummer_core::xml::Element> {
    ch.items("qualities", "quality").into_iter().filter(|q| q.get("name") == name).collect()
}

fn budget(engine: &Engine, ch: &Character) -> chargen::Budget {
    let rules = engine.rules_for(ch);
    let sheet = calc::compute(ch, &rules, Some(&engine.store), Some(&engine.catalog));
    chargen::budget(ch, &sheet, &rules, engine.settings().resolve(&ch.field("settings")).unwrap())
}

/// Quality rows the Qualities tree shows with this name.
fn tree_rows(ch: &Character, name: &str) -> usize {
    fn walk(nodes: &[tree::ItemNode], name: &str) -> usize {
        nodes.iter().map(|n| usize::from(matches!(n.value, Entry::Item { el, .. } if el.get("name") == name)) + walk(&n.children, name)).sum()
    }
    walk(&tree::section_tree(&ch.doc, &sections::QUALITIES), name)
}

#[test]
fn level_limits_follow_the_data() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("qualities.xml").unwrap();
    let rec = |n: &str| chummer_core::data::find(&doc, "qualities", "quality", n).unwrap().el().clone();
    assert_eq!(items::quality::level_limit(&rec("Focused Concentration"), false), Some(6));
    assert_eq!(items::quality::level_limit(&rec("Gremlins"), false), Some(4));
    assert_eq!(items::quality::level_limit(&rec("Infirm"), true), Some(5));
    // No limit, limit False, and limit with <nolevels/>: no levels.
    assert_eq!(items::quality::level_limit(&rec("Ambidextrous"), false), None);
    assert_eq!(items::quality::level_limit(&rec("Restricted Gear"), false), None);
    assert_eq!(items::quality::level_limit(&rec("Dealer Connection"), false), None);
    // A formula limit ({arm} - 1) does not parse as a level count.
    assert_eq!(items::quality::level_limit(&rec("Ambidextrous"), false), None);
}

#[test]
fn adding_at_a_level_adds_that_many_copies_and_their_karma() {
    let engine = Engine::load().unwrap();
    let mut s = Session::new(mystic_adept(&engine));
    let before = budget(&engine, s.ch()).positive_quality_karma;
    s.apply(&engine, add("Focused Concentration", 3)).unwrap();
    let ch = s.ch();
    assert_eq!(copies(ch, "Focused Concentration").len(), 3);
    assert_eq!(budget(&engine, ch).positive_quality_karma - before, 12, "3 levels x 4 karma");
    assert_eq!(items::quality::levels(ch, copies(ch, "Focused Concentration")[0]), 3);
    assert_eq!(tree_rows(ch, "Focused Concentration"), 1, "the levels are one row");
    assert_eq!(items::quality::shown_level(ch, &engine.store, copies(ch, "Focused Concentration")[0]), Some(3));

    // "Add" at a level already reached adds nothing; at 5 it adds 2.
    assert!(s.apply(&engine, add("Focused Concentration", 3)).is_err());
    s.apply(&engine, add("Focused Concentration", 5)).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 5);
    // The limit caps it.
    s.apply(&engine, add("Focused Concentration", 9)).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 6);
    assert_eq!(budget(&engine, s.ch()).positive_quality_karma - before, 24);

    // A quality without levels: one copy whatever the rating.
    s.apply(&engine, add("Ambidextrous", 3)).unwrap();
    assert_eq!(copies(s.ch(), "Ambidextrous").len(), 1);
    assert_eq!(items::quality::shown_level(s.ch(), &engine.store, copies(s.ch(), "Ambidextrous")[0]), None);
}

#[test]
fn raising_and_lowering_the_level() {
    let engine = Engine::load().unwrap();
    let mut s = Session::new(mystic_adept(&engine));
    let before = budget(&engine, s.ch()).positive_quality_karma;
    s.apply(&engine, add("Focused Concentration", 1)).unwrap();
    let guid = copies(s.ch(), "Focused Concentration")[0].get("guid");
    assert_eq!(items::quality::shown_level(s.ch(), &engine.store, copies(s.ch(), "Focused Concentration")[0]), Some(1), "Name 1 for a leveled quality");
    s.apply(&engine, Command::SetQualityLevel { guid: guid.clone(), level: 4 }).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 4);
    assert_eq!(budget(&engine, s.ch()).positive_quality_karma - before, 16);
    s.apply(&engine, Command::SetQualityLevel { guid: guid.clone(), level: 2 }).unwrap();
    let left = copies(s.ch(), "Focused Concentration");
    assert_eq!(left.len(), 2);
    assert!(left.iter().any(|q| q.get("guid") == guid), "lowering keeps the selected copy");
    assert_eq!(budget(&engine, s.ch()).positive_quality_karma - before, 8);
    // Above the limit is capped; a quality without levels is refused.
    s.apply(&engine, Command::SetQualityLevel { guid: guid.clone(), level: 10 }).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 6);
    s.apply(&engine, add("Ambidextrous", 0)).unwrap();
    let amb = copies(s.ch(), "Ambidextrous")[0].get("guid");
    assert!(s.apply(&engine, Command::SetQualityLevel { guid: amb, level: 2 }).is_err());
    // Removing the quality removes every level.
    s.apply(&engine, Command::RemoveItem { container: "qualities".into(), guid }).unwrap();
    assert!(copies(s.ch(), "Focused Concentration").is_empty());
    assert_eq!(tree_rows(s.ch(), "Focused Concentration"), 0);
}

#[test]
fn bonuses_count_per_level_and_first_level_bonus_once() {
    let engine = Engine::load().unwrap();
    let mut s = Session::new(mystic_adept(&engine));
    let neg_before = budget(&engine, s.ch()).negative_quality_karma;
    s.apply(&engine, add("Gremlins", 3)).unwrap();
    let ch = s.ch();
    let gremlins: Vec<String> = copies(ch, "Gremlins").iter().map(|q| q.get("guid")).collect();
    assert_eq!(gremlins.len(), 3);
    let notoriety: Vec<_> = ch.improvements.list.iter().filter(|i| i.kind == "Notoriety" && gremlins.contains(&i.source_name)).collect();
    assert_eq!(notoriety.len(), 1, "Gremlins' <firstlevelbonus> notoriety applies once");
    assert_eq!(notoriety[0].source_name, gremlins[0], "on the first level");
    assert_eq!(budget(&engine, ch).negative_quality_karma - neg_before, 12, "3 levels x 4 karma");

    // Infirm: BOD/AGI/REA/STR maximum -1 per level, the clamp once.
    let max_before = calc::attribute_values(s.ch(), "BOD", &engine.rules_for(s.ch())).total_max;
    s.apply(&engine, add("Infirm", 2)).unwrap();
    let ch = s.ch();
    let infirm: Vec<String> = copies(ch, "Infirm").iter().map(|q| q.get("guid")).collect();
    assert_eq!(infirm.len(), 2);
    assert_eq!(calc::attribute_values(ch, "BOD", &engine.rules_for(ch)).total_max, max_before - 2);
    let clamps = ch.improvements.list.iter().filter(|i| i.kind == "AttributeMaxClamp" && infirm.contains(&i.source_name)).count();
    assert_eq!(clamps, 4, "four attributes, first level only");
}

#[test]
fn levels_round_trip_as_separate_quality_elements() {
    let engine = Engine::load().unwrap();
    let mut s = Session::new(mystic_adept(&engine));
    s.apply(&engine, add("Focused Concentration", 3)).unwrap();
    s.apply(&engine, add("Gremlins", 2)).unwrap();
    let xml = s.ch().to_xml_string();
    // As Chummer writes them: one <quality> per level, each with its karma.
    assert_eq!(xml.matches("<name>Focused Concentration</name>").count(), 3);
    assert_eq!(xml.matches("<name>Gremlins</name>").count(), 2);
    let again = Character::from_str(&xml).unwrap();
    assert_eq!(again.to_xml_string(), xml, "lossless");
    let fc = copies(&again, "Focused Concentration");
    assert_eq!(items::quality::levels(&again, fc[0]), 3);
    assert!(fc.iter().all(|q| q.get("bp") == "4"));
    let (a, b) = (budget(&engine, s.ch()), budget(&engine, &again));
    assert_eq!((a.positive_quality_karma, a.negative_quality_karma), (b.positive_quality_karma, b.negative_quality_karma));
}

#[test]
fn career_levels_cost_karma_each() {
    let engine = Engine::load().unwrap();
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Soma (Career).chum5");
    let mut ch = Character::load(&p).unwrap();
    assert!(ch.created);
    ch.karma = 100;
    let mut s = Session::new(ch);
    s.apply(&engine, add("Focused Concentration", 2)).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 2);
    // Career: 4 x 2 (doubled) per level, one expense each.
    assert_eq!(s.ch().karma, 100 - 16);
    let guid = copies(s.ch(), "Focused Concentration")[0].get("guid");
    s.apply(&engine, Command::SetQualityLevel { guid: guid.clone(), level: 3 }).unwrap();
    assert_eq!(s.ch().karma, 100 - 24);
    // Not enough karma for the next level: refused, nothing changes.
    let mut poor = s.ch().clone();
    poor.karma = 3;
    let mut p = Session::new(poor);
    assert!(p.apply(&engine, Command::SetQualityLevel { guid: guid.clone(), level: 4 }).is_err());
    assert_eq!(copies(p.ch(), "Focused Concentration").len(), 3);
    // Lowering in career removes a level (no refund for a positive one).
    s.apply(&engine, Command::SetQualityLevel { guid, level: 1 }).unwrap();
    assert_eq!(copies(s.ch(), "Focused Concentration").len(), 1);
    assert_eq!(s.ch().karma, 100 - 24);
}
