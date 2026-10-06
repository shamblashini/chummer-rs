//! The command layer: determinism, undo/redo, replay and snapshots.

use std::path::Path;

use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, RecordRef, Session};
use chummer_core::contacts::ContactType;
use chummer_core::engine::Engine;
use chummer_core::items::Purchase;

fn load(name: &str) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.chum5"));
    Character::load(&p).unwrap()
}

fn fixtures() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".chum5").map(str::to_owned))
        .collect();
    v.sort();
    v
}

fn s(v: &str) -> String {
    v.to_owned()
}

fn gain(karma: bool, amount: f64) -> Command {
    Command::ManualExpense { karma, gain: true, expense: ManualExpense { amount, reason: s("Run payout"), ..Default::default() } }
}

/// Career-mode changes touching the ledger, items, skills, contacts and
/// the calendar (new GUIDs and dates).
fn career_script(ch: &Character) -> Vec<Command> {
    let skill = ch.skills.iter().find(|s| s.base + s.karma > 0 && s.base + s.karma < 6).expect("a skill to raise").guid.clone();
    vec![
        Command::SetField { key: s("alias"), value: s("Replay") },
        gain(true, 40.0),
        gain(false, 5000.0),
        Command::RaiseSkill { skill },
        Command::AddItem { tag: s("gear"), record: RecordRef { id: String::new(), name: s("Medkit") }, purchase: Purchase { rating: 3, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } },
        Command::LearnKnowledgeSkill { name: s("Seattle Gangs"), kind: s("Street") },
        Command::AddContact { kind: ContactType::Contact },
        Command::AddWeek,
        Command::SetPhysicalDamage { filled: 2 },
    ]
}

fn creation_script(ch: &Character) -> Vec<Command> {
    let skill = ch.skills.first().expect("skills").guid.clone();
    vec![
        Command::SetAttributeBase { attribute: s("BOD"), value: 2 },
        Command::SetSkillBase { skill: skill.clone(), value: 2 },
        Command::AddSpecialization { skill, name: s("Testing") },
        Command::AddKnowledgeSkill { name: s("Chess"), kind: s("Interest"), native: false },
        Command::AddItem { tag: s("gear"), record: RecordRef { id: String::new(), name: s("Medkit") }, purchase: Purchase { rating: 2, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } },
        Command::AddContact { kind: ContactType::Enemy },
        Command::SetField { key: s("background"), value: s("Born in the Barrens.") },
    ]
}

fn envelopes(cmds: Vec<Command>) -> Vec<Envelope> {
    cmds.into_iter().enumerate().map(|(i, c)| Envelope::new(c, 1000 + i as u64, 3_000_000_000_000 + i as i64 * 60_000, "")).collect()
}

#[test]
fn saved_xml_is_a_fixed_point() {
    // Reloading what we save gives the same save: the canonical form, and
    // what snapshots and hashes rely on.
    for name in fixtures() {
        let ch = load(&name);
        let xml = ch.to_xml_string();
        let again = Character::from_str(&xml).unwrap();
        assert_eq!(again.to_xml_string(), xml, "{name}");
        let back = command::restore(&command::snapshot(&ch)).unwrap();
        assert_eq!(command::state_hash(&back), command::state_hash(&ch), "{name} snapshot");
    }
}

#[test]
fn applying_to_clones_gives_identical_characters() {
    let engine = Engine::load().unwrap();
    for (name, career) in [("Munin_Career", true), ("Munin", false)] {
        let base = load(name);
        let log = envelopes(if career { career_script(&base) } else { creation_script(&base) });
        let mut a = base.clone();
        let mut b = base.clone();
        for env in &log {
            command::apply(&mut a, &engine, env).unwrap_or_else(|e| panic!("{name}: {:?}: {e}", env.cmd));
        }
        command::replay(&mut b, &engine, &log).unwrap();
        assert_eq!(a.to_xml_string(), b.to_xml_string(), "{name}");
        assert_ne!(command::state_hash(&a), command::state_hash(&base), "{name} changed");
        // Through the wire formats too.
        let mut c = base.clone();
        for env in &log {
            let wire = Envelope::from_bytes(&env.to_bytes()).unwrap();
            command::apply(&mut c, &engine, &wire).unwrap();
        }
        assert_eq!(command::state_hash(&c), command::state_hash(&a), "{name} via postcard");
    }
}

#[test]
fn new_guids_and_dates_come_from_the_envelope() {
    let engine = Engine::load().unwrap();
    let base = load("Munin_Career");
    let env = Envelope::new(gain(true, 5.0), 99, 3_000_000_000_000, "");
    let mut a = base.clone();
    command::apply(&mut a, &engine, &env).unwrap();
    let e = chummer_core::career::entries(&a).into_iter().find(|e| e.reason == "Run payout").unwrap();
    assert_eq!(e.date, env.at_iso());
    let other = Envelope { seed: 100, ..env.clone() };
    let mut b = base.clone();
    command::apply(&mut b, &engine, &other).unwrap();
    let f = chummer_core::career::entries(&b).into_iter().find(|e| e.reason == "Run payout").unwrap();
    assert_ne!(e.guid, f.guid, "another seed, another guid");
}

#[test]
fn rejected_commands_change_nothing() {
    let engine = Engine::load().unwrap();
    let mut ch = load("Munin_Career");
    ch.nuyen = 0.0;
    let before = ch.to_xml_string();
    let buy = Command::AddItem { tag: s("gear"), record: RecordRef { id: String::new(), name: s("Medkit") }, purchase: Purchase { rating: 6, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } };
    let r = command::apply(&mut ch, &engine, &Envelope::new(buy, 1, 0, ""));
    assert!(r.is_err(), "cannot pay for it");
    assert_eq!(ch.to_xml_string(), before);
    assert!(command::apply(&mut ch, &engine, &Envelope::new(Command::RaiseSkill { skill: s("no-such-guid") }, 1, 0, "")).is_err());
    assert_eq!(ch.to_xml_string(), before);
}

fn fixed_clock() -> i64 {
    3_100_000_000_000
}

#[test]
fn undo_and_redo_restore_exact_states() {
    let engine = Engine::load().unwrap();
    let base = load("Munin_Career");
    let mut session = Session::with_seed(base.clone(), 5);
    let mut hashes = vec![session.state_hash()];
    for cmd in career_script(&base) {
        session.apply(&engine, cmd).unwrap();
        hashes.push(session.state_hash());
    }
    let n = hashes.len() - 1;
    assert_eq!(session.version() as usize, n);
    assert!(session.undo_label().unwrap().contains("Set physical damage"));
    for i in (0..n).rev() {
        session.undo().unwrap();
        assert_eq!(session.state_hash(), hashes[i], "undo to {i}");
    }
    assert!(!session.can_undo());
    assert_eq!(session.version(), 0);
    for (i, h) in hashes.iter().enumerate().skip(1) {
        session.redo().unwrap();
        assert_eq!(session.state_hash(), *h, "redo to {i}");
    }
    assert!(!session.can_redo());
    // A new command after an undo drops the redo stack.
    session.undo();
    session.apply(&engine, Command::SetField { key: s("alias"), value: s("Other") }).unwrap();
    assert!(!session.can_redo());
}

#[test]
fn edits_in_a_burst_coalesce() {
    let engine = Engine::load().unwrap();
    let base = load("Munin");
    let mut session = Session::with_seed(base.clone(), 5).with_clock(fixed_clock);
    for text in ["B", "Bo", "Bor", "Born"] {
        session.apply(&engine, Command::SetField { key: s("background"), value: s(text) }).unwrap();
    }
    assert_eq!(session.version(), 1, "one step for one burst of typing");
    assert_eq!(session.ch().field("background"), "Born");
    session.apply(&engine, Command::SetField { key: s("concept"), value: s("Face") }).unwrap();
    assert_eq!(session.version(), 2);
    session.undo();
    session.undo();
    assert_eq!(session.state_hash(), command::state_hash(&base));
    // Nothing merges into a step that was undone to.
    session.redo();
    session.apply(&engine, Command::SetField { key: s("background"), value: s("Bornx") }).unwrap();
    assert_eq!(session.version(), 2);
}

#[test]
fn the_log_replays_to_the_live_state() {
    let engine = Engine::load().unwrap();
    for name in ["Munin_Career", "Munin"] {
        let base = load(name);
        let mut session = Session::new(base.clone());
        let script = if base.created { career_script(&base) } else { creation_script(&base) };
        for cmd in script {
            session.apply(&engine, cmd).unwrap();
        }
        // Undo a step and take another path, as a user would.
        session.undo();
        session.apply(&engine, Command::SetField { key: s("notes"), value: s("after undo") }).unwrap();
        for text in ["a", "ab", "abc"] {
            session.apply(&engine, Command::SetField { key: s("concept"), value: s(text) }).unwrap();
        }
        let mut fresh = load(name);
        command::replay(&mut fresh, &engine, &session.envelopes()).unwrap();
        assert_eq!(command::state_hash(&fresh), session.state_hash(), "{name}");
        assert!(session.log().iter().all(|l| !l.description.is_empty()));
    }
}

#[test]
fn descriptions_name_the_change_and_its_cost() {
    let engine = Engine::load().unwrap();
    let base = load("Munin_Career");
    let mut session = Session::new(base.clone());
    session.apply(&engine, gain(true, 40.0)).unwrap();
    let skill = base.skills.iter().find(|s| s.base + s.karma > 0 && s.base + s.karma < 6).unwrap().guid.clone();
    let r = session.apply(&engine, Command::RaiseSkill { skill }).unwrap();
    assert!(r.description.starts_with("Raised "), "{}", r.description);
    assert!(r.description.ends_with(" karma)"), "{}", r.description);
}

#[test]
fn saving_does_not_change_the_state() {
    let engine = Engine::load().unwrap();
    let mut session = Session::new(load("Munin"));
    session.apply(&engine, Command::SetField { key: s("alias"), value: s("Saved") }).unwrap();
    let h = session.state_hash();
    let dir = std::env::temp_dir().join(format!("chummer-cmd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved.chum5");
    session.save(&engine, &path).unwrap();
    assert_eq!(session.state_hash(), h);
    assert!(!session.ch().dirty);
    assert_eq!(session.ch().file.as_deref(), Some(path.as_path()));
    let reloaded = Character::load(&path).unwrap();
    assert_eq!(reloaded.field("alias"), "Saved");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "timing; run with --release --ignored --nocapture"]
fn apply_latency() {
    let engine = Engine::load().unwrap();
    for name in ["Ghile Mear", "Munin_Career", "Munin"] {
        let mut session = Session::new(load(name));
        let t = std::time::Instant::now();
        for i in 0..20 {
            session.apply(&engine, Command::SetField { key: s("notes"), value: format!("n{i}") }).unwrap();
        }
        println!("{name}: {:?} per SetField", t.elapsed() / 20);
    }
}

#[test]
fn undo_steps_share_unchanged_sections() {
    // Ghile Mear's mugshots are most of the file; a hundred undo steps must
    // not keep a hundred copies of them. Checked indirectly: undo across
    // many steps still restores exact states (deltas rebuild correctly).
    let engine = Engine::load().unwrap();
    let base = load("Ghile Mear");
    let mut session = Session::with_seed(base.clone(), 9);
    let mut hashes = vec![session.state_hash()];
    for i in 0..12 {
        let cmd = if i % 2 == 0 { Command::AddContact { kind: ContactType::Contact } } else { Command::SetField { key: format!("notes{i}"), value: s("x") } };
        session.apply(&engine, cmd).unwrap();
        hashes.push(session.state_hash());
    }
    for i in (0..12).rev() {
        session.undo().unwrap();
        assert_eq!(session.state_hash(), hashes[i]);
    }
    for h in hashes.iter().skip(1) {
        session.redo().unwrap();
        assert_eq!(session.state_hash(), *h);
    }
}
