//! Property tests for the command layer on random command sequences.
//!
//! Commands are made from `Command::examples()` (one of every variant) by
//! replacing their strings, numbers and flags with values from the
//! fixture (real GUIDs, skill and item names) or junk and extremes. For
//! each sequence:
//!
//! - applying the same envelopes to two clones gives byte-identical saves;
//! - a rejected or no-op command leaves the state hash unchanged;
//! - envelopes sent through postcard and JSON apply to the same result;
//! - undoing every step of a `Session` gives back the original hash, and
//!   redoing them the final one;
//! - `snapshot`/`restore` keeps the hash, and damaged snapshots fail
//!   cleanly.
//!
//! Failures name the fixture, the seed and the command. Rerun one with
//! `CHUMMER_FUZZ_SEED=<seed>`; `CHUMMER_FUZZ_ITERS=<n>` multiplies the
//! number of sequences.

mod common;

use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command, Envelope, Session};
use chummer_core::xml::Element;
use common::{engine, iters, no_panic, Prng};
use serde_json::Value;

const T0: i64 = 3_000_000_000_000;

fn clock() -> i64 {
    T0
}

/// Values a command may get: what the character has, and junk.
struct Pools {
    strings: Vec<String>,
    snapshot: Vec<u8>,
}

fn collect_texts(e: &Element, name: &str, out: &mut Vec<String>) {
    for c in e.elements() {
        if c.name == name {
            let t = c.text();
            if !t.trim().is_empty() && t.len() < 80 {
                out.push(t);
            }
        }
        collect_texts(c, name, out);
    }
}

fn pools(ch: &Character) -> Pools {
    let mut strings = Vec::new();
    collect_texts(&ch.doc, "guid", &mut strings);
    let mut names = Vec::new();
    collect_texts(&ch.doc, "name", &mut names);
    names.sort();
    names.dedup();
    strings.extend(names.into_iter().take(200));
    strings.extend(ch.improvements.list.iter().map(|i| i.source_name.clone()));
    for s in [
        "", " ", "BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES", "DEP", "ESS", "Firearms", "Athletics", "Pistols", "gear", "weapon", "armor",
        "cyberware", "quality", "lifestyle", "vehicle", "mod", "gears", "weapons", "Medkit", "Ares Predator V", "Armor Jacket", "Stunbolt", "Hermetic", "Metamagic", "Echo",
        "Centering", "notes", "location", "alias", "name", "karma", "nuyen", "metatype", "created", "settings", "buildmethod", "Standard.xml",
        "223a11ff-80e0-428b-89a9-6ef1c243b8b6", "00000000-0000-0000-0000-000000000000", "not-a-guid", "../../etc/passwd", "<x>&amp;</x>", "]]>", "\u{0}", "é¥«»",
        "Street", "Professional", "Interest", "Language", "MentorSpirit", "Bear", "Attribute", "Skill",
    ] {
        strings.push(s.to_owned());
    }
    strings.push("x".repeat(5000));
    Pools { strings, snapshot: command::snapshot(ch) }
}

const INTS: &[i64] = &[i32::MIN as i64, -1000, -10, -1, 0, 1, 2, 3, 4, 6, 10, 12, 100, 1000, i32::MAX as i64, u32::MAX as i64, i64::MAX];
const FLOATS: &[f64] = &[-1e300, -1000.5, -1.0, 0.0, 0.25, 0.5, 1.0, 2.0, 100.0, 5000.0, 1e9, 1e300];

/// Replace leaves of `v` with random values from the pools.
fn scramble(v: &mut Value, rng: &mut Prng, p: &Pools) {
    match v {
        Value::String(s) => {
            if rng.chance(3, 4) {
                *s = rng.pick(&p.strings).clone();
            }
        }
        Value::Bool(b) => *b = rng.chance(1, 2),
        Value::Number(n) => {
            if rng.chance(1, 2) {
                return;
            }
            *v = if n.is_f64() { serde_json::json!(*rng.pick(FLOATS)) } else { serde_json::json!(*rng.pick(INTS)) };
        }
        Value::Array(a) => {
            // `Revert` snapshots and drug components: sometimes the real
            // snapshot, sometimes a damaged one.
            if a.iter().all(Value::is_number) && a.len() != 3 {
                let mut bytes = if rng.chance(1, 2) { p.snapshot.clone() } else { Vec::new() };
                if !bytes.is_empty() && rng.chance(1, 2) {
                    let at = rng.below(bytes.len());
                    if rng.chance(1, 2) {
                        bytes.truncate(at);
                    } else {
                        bytes[at] ^= 0x55;
                    }
                }
                *v = serde_json::json!(bytes);
                return;
            }
            for x in a {
                scramble(x, rng, p);
            }
        }
        Value::Object(m) => {
            for (_, x) in m.iter_mut() {
                scramble(x, rng, p);
            }
        }
        Value::Null => {
            if rng.chance(1, 3) {
                *v = Value::String(rng.pick(&p.strings).clone());
            }
        }
    }
}

/// A random command: an example of a random variant with scrambled
/// fields, retried until it deserialises (some numbers do not fit).
fn random_command(rng: &mut Prng, p: &Pools, examples: &[Command]) -> Command {
    let ex = rng.pick(examples);
    let base = serde_json::to_value(ex).unwrap();
    for _ in 0..8 {
        let mut v = base.clone();
        scramble(&mut v, rng, p);
        if let Ok(c) = serde_json::from_value::<Command>(v) {
            return c;
        }
    }
    ex.clone()
}

/// Commands that JSON cannot carry: non-finite floats.
fn non_finite_commands(p: &Pools, rng: &mut Prng) -> Vec<Command> {
    let g = rng.pick(&p.strings).clone();
    let mut v = Vec::new();
    for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        v.push(Command::SetNuyen { value: x });
        v.push(Command::SetItemQuantity { guid: g.clone(), qty: x });
        v.push(Command::SellItem { guid: g.clone(), fraction: x });
        v.push(Command::ManualExpense { karma: rng.chance(1, 2), gain: rng.chance(1, 2), expense: ManualExpense { amount: x, reason: "fuzz".into(), ..Default::default() } });
    }
    v
}

fn env_label(env: &Envelope) -> String {
    let mut j = serde_json::to_string(&env.cmd).unwrap_or_else(|_| format!("{:?}", env.cmd));
    if j.len() > 300 {
        j.truncate(300);
        j.push('…');
    }
    format!("seed {} cmd {j}", env.seed)
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Changed,
    Unchanged,
    Rejected(String),
}

fn run(ch: &mut Character, env: &Envelope) -> Outcome {
    match command::apply(ch, engine(), env) {
        Ok(a) if a.changed => Outcome::Changed,
        Ok(_) => Outcome::Unchanged,
        Err(e) => Outcome::Rejected(e.reason),
    }
}

/// Check every property on one sequence; `Err` describes the first
/// violation.
fn check_sequence(name: &str, base: &Character, log: &[Envelope]) -> Result<(), String> {
    let base_hash = command::state_hash(base);
    // A: step by step, checking rejected and no-op commands.
    let mut a = base.clone();
    let mut outcomes = Vec::new();
    for env in log {
        let before = command::state_hash(&a);
        let before_xml = a.to_xml_string();
        let o = run(&mut a, env);
        if o != Outcome::Changed && command::state_hash(&a) != before {
            return Err(format!("{name}: {o:?} changed the character: {}", env_label(env)));
        }
        if o != Outcome::Changed && a.to_xml_string() != before_xml {
            return Err(format!("{name}: {o:?} changed the saved XML: {}", env_label(env)));
        }
        outcomes.push(o);
    }
    let final_xml = a.to_xml_string();
    let final_hash = command::state_hash(&a);

    // B: the same envelopes on another clone.
    let mut b = base.clone();
    for (i, env) in log.iter().enumerate() {
        let o = run(&mut b, env);
        if o != outcomes[i] {
            return Err(format!("{name}: step {i} gave {o:?}, first time {:?}: {}", outcomes[i], env_label(env)));
        }
    }
    if b.to_xml_string() != final_xml {
        return Err(format!("{name}: two clones differ after the same envelopes"));
    }

    // Wire formats.
    let mut c = base.clone();
    let mut d = base.clone();
    let mut json_ok = true;
    for env in log {
        let wire = Envelope::from_bytes(&env.to_bytes()).map_err(|e| format!("{name}: postcard round trip failed: {e}: {}", env_label(env)))?;
        run(&mut c, &wire);
        match Envelope::from_json(&env.to_json()) {
            Ok(back) => {
                run(&mut d, &back);
            }
            Err(e) => {
                if command_is_finite(&env.cmd) {
                    return Err(format!("{name}: JSON round trip failed: {e}: {}", env_label(env)));
                }
                json_ok = false;
            }
        }
    }
    if command::state_hash(&c) != final_hash {
        return Err(format!("{name}: applying postcard copies gave another result"));
    }
    if json_ok && command::state_hash(&d) != final_hash {
        return Err(format!("{name}: applying JSON copies gave another result"));
    }

    // Session: undo everything, redo everything.
    let mut s = Session::with_seed(base.clone(), 7).with_clock(clock);
    for env in log {
        let _ = s.apply_envelope(engine(), env.clone());
    }
    if s.state_hash() != final_hash {
        return Err(format!("{name}: a Session applying the envelopes ended elsewhere"));
    }
    let mut steps = 0;
    while s.undo().is_some() {
        steps += 1;
    }
    if s.state_hash() != base_hash {
        return Err(format!("{name}: undoing {steps} steps did not restore the original"));
    }
    while s.redo().is_some() {}
    if s.state_hash() != final_hash {
        return Err(format!("{name}: redoing {steps} steps did not give the final state"));
    }

    // Snapshots.
    let back = command::restore(&command::snapshot(&a)).map_err(|e| format!("{name}: restore failed: {e}"))?;
    if command::state_hash(&back) != final_hash {
        return Err(format!("{name}: snapshot/restore changed the hash"));
    }
    // And the saved file.
    let reloaded = Character::from_str(&final_xml).map_err(|e| format!("{name}: saved XML does not load: {e}"))?;
    if reloaded.to_xml_string() != final_xml {
        return Err(format!("{name}: saved XML is not a fixed point after the commands"));
    }
    Ok(())
}

/// Whether every float in the command is finite. JSON writes NaN and the
/// infinities as `null`; see `bug_non_finite_numbers_do_not_survive_json`.
fn command_is_finite(c: &Command) -> bool {
    let dbg = format!("{c:?}");
    !(dbg.contains("NaN") || dbg.contains(": inf") || dbg.contains(": -inf"))
}

/// Known bug class: integer overflow on extreme values (see fuzz_load).
fn is_known_overflow(panic: &str) -> bool {
    panic.contains("with overflow")
}

const FIXTURES: &[&str] = &["Davis Jones", "Soma (Career)", "Draught"];

#[test]
fn random_command_sequences() {
    let examples = Command::examples();
    let mut failures = Vec::new();
    let mut known = Vec::new();
    let mut tally = [0usize; 3];
    let started = std::time::Instant::now();
    for (fi, name) in FIXTURES.iter().enumerate() {
        let base = Character::load(&common::fixtures_dir().join(format!("{name}.chum5"))).unwrap();
        let p = pools(&base);
        for round in 0..iters(2) {
            let seed = common::base_seed() ^ ((fi as u64) << 40) ^ round as u64;
            let mut rng = Prng::new(seed);
            let len = 4 + rng.below(6);
            let mut cmds: Vec<Command> = (0..len).map(|_| random_command(&mut rng, &p, &examples)).collect();
            if round % 2 == 1 {
                let extra = non_finite_commands(&p, &mut rng);
                cmds.push(rng.pick(&extra).clone());
            }
            let log: Vec<Envelope> = cmds.into_iter().enumerate().map(|(i, c)| Envelope::new(c, seed.wrapping_add(i as u64), T0 + i as i64 * 1000, "")).collect();
            for env in &log {
                let mut probe = base.clone();
                if let Ok(o) = no_panic(|| run(&mut probe, env)) {
                    tally[match o {
                        Outcome::Changed => 0,
                        Outcome::Unchanged => 1,
                        Outcome::Rejected(_) => 2,
                    }] += 1;
                }
            }
            match no_panic(|| check_sequence(name, &base, &log)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let cmds: Vec<String> = log.iter().map(env_label).collect();
                    failures.push(format!("seed {seed:#x}: {e}\n    commands: {}", cmds.join("\n              ")));
                }
                Err(panic) => {
                    let cmds: Vec<String> = log.iter().map(env_label).collect();
                    let msg = format!("{name} seed {seed:#x}: panic {panic}\n    commands: {}", cmds.join("\n              "));
                    if is_known_overflow(&panic) {
                        known.push(msg);
                    } else {
                        failures.push(msg);
                    }
                }
            }
        }
    }
    eprintln!("commands changed/unchanged/rejected (on the base): {tally:?} in {:?}", started.elapsed());
    if !known.is_empty() {
        eprintln!("{} known overflow panics:\n{}", known.len(), known.join("\n"));
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// The cheap subset of [`check_sequence`] for one command: a rejected or
/// no-op command changes nothing, and the postcard copy applied to
/// another clone gives the same save.
fn check_one(name: &str, base: &Character, env: &Envelope) -> Result<(), String> {
    let mut a = base.clone();
    let o = run(&mut a, env);
    if o != Outcome::Changed && command::state_hash(&a) != command::state_hash(base) {
        return Err(format!("{name}: {o:?} changed the character: {}", env_label(env)));
    }
    let wire = Envelope::from_bytes(&env.to_bytes()).map_err(|e| format!("{name}: postcard: {e}: {}", env_label(env)))?;
    let mut b = base.clone();
    let o2 = run(&mut b, &wire);
    if o2 != o || b.to_xml_string() != a.to_xml_string() {
        return Err(format!("{name}: second application differs ({o:?} / {o2:?}): {}", env_label(env)));
    }
    Ok(())
}

/// Every example command on a creation and a career character: the hand
/// written values, so this always covers each variant once.
#[test]
fn every_variant_on_real_characters() {
    let mut failures = Vec::new();
    for name in ["Davis Jones", "Soma (Career)"] {
        let base = Character::load(&common::fixtures_dir().join(format!("{name}.chum5"))).unwrap();
        let log: Vec<Envelope> = Command::examples().into_iter().enumerate().map(|(i, c)| Envelope::new(c, i as u64, T0, "gm")).collect();
        for env in &log {
            match no_panic(|| check_one(name, &base, env)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => failures.push(e),
                Err(p) => failures.push(format!("{name}: panic {p}: {}", env_label(env))),
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// `restore` on damaged snapshots: errors, never panics.
#[test]
fn damaged_snapshots_never_panic() {
    let base = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let snap = command::snapshot(&base);
    let mut failures = Vec::new();
    let mut rng = Prng::new(common::base_seed());
    for i in 0..iters(40) {
        let mut b = snap.clone();
        let desc = match i % 3 {
            0 => {
                let at = rng.below(b.len());
                b.truncate(at);
                format!("truncated to {at}")
            }
            1 => {
                let at = rng.below(b.len());
                b[at] = rng.next_u64() as u8;
                format!("byte {at} = {:#04x}", b[at])
            }
            _ => {
                let n = rng.below(200);
                b = rng.bytes(n);
                "random bytes".into()
            }
        };
        if let Err(p) = no_panic(|| command::restore(&b).map(|c| command::state_hash(&c))) {
            failures.push(format!("{desc}: {p}"));
        }
    }
    // A snapshot of something that is not a character.
    let not_char = chummer_core::chum5lz::compress(b"<settings />").unwrap();
    assert!(command::restore(&not_char).is_err());
    let not_utf8 = chummer_core::chum5lz::compress(b"<character>\xff</character>").unwrap();
    assert!(command::restore(&not_utf8).is_err());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Envelope decoding from junk: errors, never panics.
#[test]
fn junk_envelopes_never_panic() {
    let mut rng = Prng::new(common::base_seed());
    let good = Envelope::new(Command::examples().remove(0), 1, T0, "").to_bytes();
    for i in 0..iters(500) {
        let b = if i % 2 == 0 {
            let n = rng.below(64);
            rng.bytes(n)
        } else {
            let mut g = good.clone();
            let at = rng.below(g.len());
            g[at] = rng.next_u64() as u8;
            g.truncate(rng.below(g.len() + 1));
            g
        };
        no_panic(|| Envelope::from_bytes(&b).ok()).unwrap_or_else(|p| panic!("postcard {b:02x?}: {p}"));
        let s = String::from_utf8_lossy(&b).into_owned();
        no_panic(|| (Envelope::from_json(&s).ok(), command::parse_log(&s, T0).ok())).unwrap_or_else(|p| panic!("json {s:?}: {p}"));
    }
    // A huge declared length in postcard must not allocate it.
    let mut huge = vec![0u8];
    huge.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    assert!(Envelope::from_bytes(&huge).is_err());
}

/// Envelopes that a peer may send with extreme times, applied through a
/// `Session` (which merges quick successive edits by time difference).
#[test]
fn extreme_envelope_times() {
    let base = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let mut s = Session::with_seed(base, 1).with_clock(clock);
    for (i, at) in [i64::MIN, i64::MAX, 0, -1, i64::MIN, i64::MAX].into_iter().enumerate() {
        let env = Envelope::new(Command::SetField { key: "alias".into(), value: format!("A{i}") }, i as u64, at, "");
        let _ = env.at_iso();
        no_panic(|| s.apply_envelope(engine(), env).map(|r| r.changed)).unwrap_or_else(|p| panic!("at {at}: {p}")).unwrap();
    }
    assert_eq!(s.ch().field("alias"), "A5");
}

/// JSON writes NaN and the infinities as `null`, which does not read
/// back: a command log holding one (`SetNuyen`, a quantity typed as
/// "NaN") cannot be replayed from JSON. Postcard carries them.
#[test]
#[ignore = "BUG: non-finite f64 in a Command serialises to JSON null and fails to deserialise"]
fn bug_non_finite_numbers_do_not_survive_json() {
    let env = Envelope::new(Command::SetNuyen { value: f64::NAN }, 1, T0, "");
    Envelope::from_json(&env.to_json()).unwrap();
}


/// Field names in commands become element names in the save: a name with
/// a space (an item name picked up by the fuzzer) made a save that no
/// longer loaded. Such commands are rejected now.
#[test]
fn field_names_must_be_element_names() {
    let base = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let contact = base.items("contacts", "contact").first().map(|c| c.get("guid")).unwrap_or_default();
    let item = base.items("gears", "gear").first().map(|c| c.get("guid")).unwrap_or_default();
    for bad in ["Telescoping Mirror on a Stick", "", "<x>", "a/b", "1abc", "x\"y", "a b"] {
        for cmd in [
            Command::SetField { key: bad.into(), value: "v".into() },
            Command::SetContactField { contact: contact.clone(), key: bad.into(), value: "v".into() },
            Command::SetItemText { guid: item.clone(), field: bad.into(), value: "v".into() },
        ] {
            let mut ch = base.clone();
            let env = Envelope::new(cmd, 1, T0, "");
            let r = command::apply(&mut ch, engine(), &env);
            assert!(r.is_err(), "{bad:?} accepted: {}", env_label(&env));
            assert_eq!(command::state_hash(&ch), command::state_hash(&base));
        }
    }
    // Real field names still work, and the save loads.
    let mut ch = base.clone();
    for (key, value) in [("alias", "Ghost"), ("gamenotes", "x"), ("nuyenbp", "5"), ("élan_1.x-y", "ok")] {
        command::apply(&mut ch, engine(), &Envelope::new(Command::SetField { key: key.into(), value: value.into() }, 1, T0, "")).unwrap();
    }
    Character::from_str(&ch.to_xml_string()).unwrap();
}

/// Extreme numbers in commands reach the same unchecked `i32` arithmetic
/// as extreme numbers in files (see fuzz_load's
/// `bug_extreme_numbers_overflow_rules_math`).
#[test]
#[ignore = "BUG: i32 overflow in karma costs / custom spell drain on extreme command values (debug builds panic, release wraps)"]
fn bug_extreme_command_values_overflow() {
    let base = Character::load(&common::fixtures_dir().join("Draught.chum5")).unwrap();
    let group = base.skill_groups.first().map(|g| g.name.clone()).unwrap_or_default();
    let design = chummer_core::gm::custom_spell::SpellDesign {
        mods: [true; chummer_core::gm::custom_spell::MODIFIER_SLOTS],
        effects: i32::MAX,
        name: "Fuzz".into(),
        ..Default::default()
    };
    let manipulation = chummer_core::gm::custom_spell::SpellDesign { category: "Manipulation".into(), ..design.clone() };
    let mut failures = Vec::new();
    for cmd in [Command::SetGroupKarma { group, value: i32::MAX }, Command::AddCustomSpell { design }, Command::AddCustomSpell { design: manipulation }] {
        let env = Envelope::new(cmd, 1, T0, "");
        let mut ch = base.clone();
        if let Err(p) = no_panic(|| run(&mut ch, &env)) {
            failures.push(format!("{}: {p}", env_label(&env)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
