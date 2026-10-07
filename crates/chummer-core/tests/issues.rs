//! Creation issues (`chargen::issues`) and the guided-creation steps.

use std::path::PathBuf;

use chummer_core::calc;
use chummer_core::chargen::guide::{self, Step};
use chummer_core::chargen::issues::{self, Area, Issue, IssueKind, IssueTab, Severity};
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::engine::Engine;

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn issues_of(engine: &Engine, ch: &Character) -> Vec<Issue> {
    let rules = engine.rules_for(ch);
    let store = engine.store_for_character(ch);
    let sheet = calc::compute(ch, &rules, Some(&store), Some(&engine.catalog));
    let settings = engine.settings.resolve(&ch.field("settings")).unwrap();
    let b = chargen::budget_with(ch, &sheet, &rules, settings, Some(&store));
    issues::issues(ch, &b, &sheet, settings, Some(&store))
}

fn new_char(engine: &Engine, settings_id: &str, metatype: &str, priorities: [char; 5], talent: &str) -> Character {
    let spec = NewCharacter {
        settings_id: settings_id.into(),
        metatype: metatype.into(),
        metavariant: None,
        priorities: Priorities(priorities),
        talent: talent.into(),
        talent_skills: vec![],
        name: "Issue Runner".into(),
    };
    chargen::create(engine, &spec).unwrap()
}

fn find(list: &[Issue], kind: IssueKind) -> Option<&Issue> {
    list.iter().find(|i| i.kind == kind)
}

#[test]
fn fresh_priority_character_has_only_warnings() {
    let engine = Engine::load().unwrap();
    let ch = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    let list = issues_of(&engine, &ch);
    assert!(list.iter().all(|i| !i.is_error()), "{list:?}");
    let skills = find(&list, IssueKind::SkillPointsLeft).expect("unspent skill points");
    assert_eq!(skills.severity, Severity::Warning);
    assert_eq!(skills.tab(), Some(IssueTab::Skills));
    assert_eq!(skills.message(), "36 Active Skill points left to spend");
    let attrs = find(&list, IssueKind::AttributePointsLeft).unwrap();
    assert_eq!((attrs.area, attrs.tab()), (Area::Attributes, Some(IssueTab::Common)));
    assert_eq!(attrs.args, ["24"]);
    assert!(find(&list, IssueKind::SpecialPointsLeft).is_some());
    assert!(find(&list, IssueKind::SkillGroupPointsLeft).is_some());
    // 140,000¥ unspent: more than 5,000¥ carries over.
    let ny = find(&list, IssueKind::NuyenCarryOver).unwrap();
    assert_eq!(ny.tab(), None, "character-wide totals have no tab");
    // 25 karma unspent, 7 carry over.
    assert_eq!(find(&list, IssueKind::KarmaCarryOver).unwrap().args, ["25", "7"]);
    // A street name is still missing.
    assert_eq!(find(&list, IssueKind::NoAlias).unwrap().severity, Severity::Info);
    // Errors sort first, then warnings, then infos.
    assert!(list.windows(2).all(|w| w[0].severity <= w[1].severity));
    // In career mode there is nothing to report.
    let mut done = ch.clone();
    done.created = true;
    assert!(issues_of(&engine, &done).is_empty());
}

#[test]
fn overspent_points_are_errors() {
    let engine = Engine::load().unwrap();
    let mut ch = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    // 24 attribute points: put 5 into each of five attributes.
    for a in ch.attributes.iter_mut().filter(|a| ["BOD", "AGI", "REA", "STR", "WIL"].contains(&a.name.as_str())) {
        a.base = 5;
    }
    let list = issues_of(&engine, &ch);
    let over = find(&list, IssueKind::AttributePointsOver).expect("overspent attributes");
    assert_eq!(over.severity, Severity::Error);
    assert_eq!(over.message(), "1 over allotted Attribute point limit");
    // All five are at the human maximum of 6, only one may be.
    assert_eq!(find(&list, IssueKind::TooManyAttributesAtMax).unwrap().args, ["5", "1"]);
    assert!(find(&list, IssueKind::AttributePointsLeft).is_none());
    let settings = engine.settings.resolve(STANDARD).unwrap();
    let rules = engine.rules_for(&ch);
    let sheet = calc::compute(&ch, &rules, Some(&engine.store), Some(&engine.catalog));
    let b = chargen::budget(&ch, &sheet, &rules, settings);
    let problems = chargen::validity_problems(&ch, &sheet, &b, settings, Some(&engine.store));
    assert!(problems.iter().any(|p| p.contains("over allotted Attribute point limit")), "{problems:?}");
}

#[test]
fn skill_issues_point_at_the_skill() {
    let engine = Engine::load().unwrap();
    let mut ch = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    let pistols = ch.skills.iter().find(|k| engine.catalog.get(&k.suid).is_some_and(|d| d.name == "Pistols")).unwrap().guid.clone();
    chargen::add_specialization(&mut ch, &pistols, "Revolvers");
    chargen::add_specialization(&mut ch, &pistols, "Semi-Automatics");
    let list = issues_of(&engine, &ch);
    let spec = find(&list, IssueKind::MultipleSpecializations).unwrap();
    assert_eq!(spec.item.as_deref(), Some(pistols.as_str()));
    assert_eq!(spec.area, Area::ActiveSkills);
    assert_eq!(spec.message(), "Pistols has more than one specialization");
    // Two specializations used two skill points.
    assert_eq!(find(&list, IssueKind::SkillPointsLeft).unwrap().args, ["34"]);
    // Spending them all removes the warning; one more is an error.
    ch.skills.iter_mut().find(|s| s.guid == pistols).unwrap().base = 6;
    for s in ch.skills.iter_mut().filter(|s| s.guid != pistols).take(5) {
        s.base = 6;
    }
    let list = issues_of(&engine, &ch);
    assert!(find(&list, IssueKind::SkillPointsLeft).is_none());
    assert_eq!(find(&list, IssueKind::SkillPointsOver).unwrap().args, ["2"]);
}

#[test]
fn karma_build_has_no_point_pools() {
    let engine = Engine::load().unwrap();
    let pb = engine.settings.presets.iter().find(|p| p.build_method() == "Karma").unwrap().id();
    let ch = new_char(&engine, &pb, "Elf", ['A', 'B', 'C', 'D', 'E'], "Mundane");
    let list = issues_of(&engine, &ch);
    for k in [IssueKind::AttributePointsLeft, IssueKind::SkillPointsLeft, IssueKind::SpecialPointsLeft, IssueKind::SkillGroupPointsLeft] {
        assert!(find(&list, k).is_none(), "{k:?} in {list:?}");
    }
    // 800 - 40 karma left: far above the carry-over.
    assert_eq!(find(&list, IssueKind::KarmaCarryOver).unwrap().severity, Severity::Warning);
}

#[test]
fn magician_needs_a_tradition() {
    let engine = Engine::load().unwrap();
    let mut ch = new_char(&engine, STANDARD, "Human", ['D', 'A', 'B', 'C', 'E'], "Magician");
    let list = issues_of(&engine, &ch);
    let t = find(&list, IssueKind::NoTradition).expect("no tradition");
    assert_eq!((t.severity, t.tab()), (Severity::Error, Some(IssueTab::Magician)));
    assert!(find(&list, IssueKind::FreeSpellsLeft).is_some());
    chargen::set_tradition(&mut ch, &engine.store, "Hermetic").unwrap();
    assert!(find(&issues_of(&engine, &ch), IssueKind::NoTradition).is_none());
}

#[test]
fn high_contacts_and_left_points() {
    let engine = Engine::load().unwrap();
    let mut ch = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    // Charisma 1: three free contact points.
    assert_eq!(find(&issues_of(&engine, &ch), IssueKind::ContactPointsLeft).unwrap().args, ["3"]);
    chargen::add_contact(&mut ch, "Fixer", "Fixer", 6, 2);
    let list = issues_of(&engine, &ch);
    let high = find(&list, IssueKind::HighContact).unwrap();
    assert_eq!(high.tab(), Some(IssueTab::Relationships));
    assert!(high.item.is_some());
    assert!(find(&list, IssueKind::ContactPointsLeft).is_none());
}

#[test]
fn fixtures_report_without_panicking() {
    let engine = Engine::load().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let mut creation = 0;
    for f in files {
        let ch = Character::load(&f).unwrap();
        if engine.settings.resolve(&ch.field("settings")).is_none() {
            continue;
        }
        let list = issues_of(&engine, &ch);
        if ch.created {
            assert!(list.is_empty(), "{}", f.display());
            continue;
        }
        creation += 1;
        eprintln!("{}: {:?}", f.file_name().unwrap().to_string_lossy(), list.iter().map(Issue::message).collect::<Vec<_>>());
        // Item issues name items the character has.
        for i in list.iter().filter_map(|i| i.item.as_ref()) {
            let known = ch.skills.iter().any(|s| &s.guid == i) || ch.knowledge_skills.iter().any(|s| &s.guid == i) || chummer_core::items::edit::find(&ch, i).is_some() || ch.items("contacts", "contact").iter().any(|c| &c.get("guid") == i) || ch.items("qualities", "quality").iter().any(|c| &c.get("guid") == i);
            assert!(known, "{}: unknown item {i}", f.display());
        }
    }
    assert!(creation > 5);
}

#[test]
fn templates_are_unique() {
    let t = issues::templates();
    let set: std::collections::HashSet<_> = t.iter().collect();
    assert_eq!(set.len(), t.len());
}

// ----- guided creation -----

#[test]
fn steps_follow_the_build_method() {
    let engine = Engine::load().unwrap();
    let mundane = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    let steps = guide::steps_for("Priority", &mundane);
    assert_eq!(
        steps,
        [
            Step::Concept,
            Step::Attributes,
            Step::SpecialAttributes,
            Step::Qualities,
            Step::ActiveSkills,
            Step::KnowledgeSkills,
            Step::Cyberware,
            Step::Gear,
            Step::Vehicles,
            Step::Contacts,
            Step::CharacterInfo,
            Step::Review
        ]
    );
    let mage = new_char(&engine, STANDARD, "Human", ['D', 'A', 'B', 'C', 'E'], "Magician");
    let steps = guide::steps_for("Priority", &mage);
    assert!(steps.contains(&Step::Spells) && !steps.contains(&Step::AdeptPowers) && !steps.contains(&Step::ComplexForms));
    assert!(steps.iter().position(|s| *s == Step::Spells) > steps.iter().position(|s| *s == Step::KnowledgeSkills));

    let karma = guide::steps_for("Karma", &mundane);
    assert_eq!(&karma[..4], [Step::Concept, Step::Qualities, Step::Attributes, Step::SpecialAttributes]);
    let life = guide::steps_for("LifeModule", &mundane);
    assert_eq!(&life[..3], [Step::Concept, Step::LifeModules, Step::Qualities]);
    assert_eq!(*life.last().unwrap(), Step::Review);
    for s in &life {
        assert_eq!(Step::parse(s.id()), Some(*s));
        assert!(!s.explanation("LifeModule").is_empty());
    }
    assert_eq!(Step::Concept.source("SumtoTen"), ("RF", "62"));
    assert_eq!(Step::Qualities.source("Priority"), ("SR5", "71"));
}

#[test]
fn step_status_tracks_issues() {
    let engine = Engine::load().unwrap();
    let mut ch = new_char(&engine, STANDARD, "Human", ['D', 'E', 'A', 'B', 'C'], "Mundane");
    let steps = guide::steps_for("Priority", &ch);
    let list = issues_of(&engine, &ch);
    // Fresh: the first step with something to do is the attributes; the
    // concept (done in the wizard) counts as visited.
    let start = guide::suggested(&steps, &list);
    assert_eq!(steps[start], Step::Attributes);
    let mut visited = guide::visited_before(&steps, start);
    visited.push(steps[start]);
    let st = guide::status(&steps, &visited, &list);
    assert!(st[0].done());
    assert_eq!(st[start].warnings, 1);
    assert!(!st[start].done());
    assert_eq!(guide::focus(&st, IssueTab::Common), Some(start));
    // Next skips nothing yet: special attributes are next.
    assert_eq!(guide::next(&st, start), Some(start + 1));
    let skills = steps.iter().position(|s| *s == Step::ActiveSkills).unwrap();
    assert_eq!(st[skills].warnings, 2, "skill points and skill group points");
    assert_eq!(guide::focus(&st, IssueTab::Skills), Some(skills));
    assert_eq!(guide::tab_done(&st, IssueTab::Common), Some(false));
    assert_eq!(guide::progress(&st), (1, steps.len() - 1));

    // Overspent attributes are errors, and pull the suggestion back.
    for a in ch.attributes.iter_mut().filter(|a| ["BOD", "AGI", "REA", "STR", "WIL"].contains(&a.name.as_str())) {
        a.base = 5;
    }
    let list = issues_of(&engine, &ch);
    let st = guide::status(&steps, &steps, &list);
    let attrs = steps.iter().position(|s| *s == Step::Attributes).unwrap();
    assert_eq!(st[attrs].errors, 2);
    assert_eq!(guide::suggested(&steps, &list), attrs);
    // The review step collects every error, and its list has them first.
    assert_eq!(st.last().unwrap().errors, 2);
    assert!(Step::Review.todo(&list)[0].is_error());
}
