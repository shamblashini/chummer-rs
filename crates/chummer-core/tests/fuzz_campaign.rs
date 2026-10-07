//! Damaged campaign files: loading fails cleanly or gives a campaign the
//! GM screen can use (roster, encounters, feed) without panicking.

mod common;

use chummer_core::campaign::damage::{self, Attack, Tracks};
use chummer_core::campaign::{Campaign, Combatant, Encounter, Member, MemberKind};
use chummer_core::character::Character;
use chummer_core::dice::Rng;
use common::{iters, no_panic, Prng};

/// A campaign with a bit of everything.
fn sample() -> Campaign {
    let ch = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let mut c = Campaign::new("Seattle Nights");
    c.gm_notes = "notes".into();
    let a = c.add(Member::embedded(MemberKind::Player, &ch));
    c.add(Member::linked(MemberKind::Npc, std::path::Path::new("runners/ghost.chum5"), &ch));
    let mut e = Encounter::new("Ambush");
    e.combatants.push(Combatant::for_member(a, "Davis"));
    e.combatants.push(Combatant::ad_hoc("Guard 2", 8, 1, 10, 10));
    e.new_round(&mut Rng::seeded(3), |_| None);
    c.encounters.push(e);
    c
}

/// What the GM screen does with a loaded campaign.
fn exercise(c: &Campaign) {
    let _ = c.grouped();
    for m in &c.members {
        let _ = c.next_number(&m.name);
        let _ = m.load_character(Some(std::path::Path::new("/nonexistent")));
        let _ = m.linked_path(None);
    }
    for item in &c.log {
        let _ = chummer_core::chargen::iso_from_unix(item.at.div_euclid(1000));
    }
    let again = Campaign::from_json(&c.to_json()).expect("a loaded campaign saves and loads");
    assert_eq!(&again, c, "campaign JSON round trip");
    let mut rng = Rng::seeded(1);
    for e in &c.encounters {
        let mut e = e.clone();
        let _ = e.order();
        let _ = e.current();
        let _ = e.advance();
        let _ = e.has_next_pass();
        let _ = e.next_pass();
        for i in 0..e.combatants.len() {
            e.spend(i, 10);
            e.reroll(i, &mut rng, None);
            e.blitz(i, &mut rng);
            e.seize(i);
            if let Some(t) = e.combatants[i].track.clone() {
                let tracks = Tracks { physical: t.physical, stun: t.stun, overflow: 3, physical_filled: t.physical_filled, stun_filled: t.stun_filled };
                let _ = damage::apply(tracks, 6, false);
                let _ = damage::wound_modifier(t.physical_filled, t.stun_filled, t.physical, 3);
            }
        }
        e.new_round(&mut rng, |_| None);
        e.reset();
    }
}

const NUMBERS: &[&str] = &["-1", "0", "4294967295", "4294967296", "2147483647", "-2147483648", "9223372036854775807", "-9223372036854775808", "1e400", "1.5", "null", "\"x\"", "[]", "{}", "true"];

fn mutate(src: &str, rng: &mut Prng) -> (String, Vec<u8>) {
    match rng.below(5) {
        0 => {
            let mut at = rng.below(src.len() + 1);
            while !src.is_char_boundary(at) {
                at -= 1;
            }
            (format!("truncate at {at}"), src.as_bytes()[..at].to_vec())
        }
        1 => {
            let mut b = src.as_bytes().to_vec();
            let at = rng.below(b.len());
            b[at] = rng.next_u64() as u8;
            (format!("byte {at} = {:#04x}", b[at]), b)
        }
        _ => {
            // Replace the values of a few JSON numbers / scalars.
            let spots: Vec<usize> = src.match_indices(": ").map(|(i, _)| i + 2).collect();
            let mut out = src.to_owned();
            let mut desc = String::from("values");
            let mut picks: Vec<usize> = (0..1 + rng.below(6)).map(|_| *rng.pick(&spots)).collect();
            picks.sort_unstable_by(|a, b| b.cmp(a));
            picks.dedup();
            for at in picks {
                let rest = &out[at..];
                let end = rest.find([',', '\n', '}', ']']).unwrap_or(rest.len());
                if rest.starts_with(['{', '[']) {
                    continue;
                }
                let v = *rng.pick(NUMBERS);
                desc.push_str(&format!(" @{at}={v}"));
                out.replace_range(at..at + end, v);
            }
            (desc, out.into_bytes())
        }
    }
}

/// Known bug class: integer overflow on extreme values (see fuzz_load).
fn is_known_overflow(panic: &str) -> bool {
    panic.contains("with overflow")
}

#[test]
fn damaged_campaign_files_never_panic() {
    let c = sample();
    let json = c.to_json();
    let packed = chummer_core::chum5lz::compress(json.as_bytes()).unwrap();
    let mut failures = Vec::new();
    let mut loaded = 0;
    for i in 0..iters(150) {
        let seed = common::base_seed() ^ i as u64;
        let mut rng = Prng::new(seed);
        let (desc, bytes) = if i % 5 == 4 {
            let mut b = packed.clone();
            let at = rng.below(b.len());
            b[at] ^= 1 << rng.below(8);
            (format!("compressed bit flip at {at}"), b)
        } else {
            mutate(&json, &mut rng)
        };
        match no_panic(|| Campaign::from_bytes(&bytes).map(|c| exercise(&c))) {
            Ok(Ok(())) => loaded += 1,
            Ok(Err(_)) => {}
            Err(p) => failures.push(format!("seed {seed:#x} [{desc}]: {p}")),
        }
    }
    eprintln!("{loaded} damaged campaigns loaded");
    assert!(failures.is_empty(), "{} panics:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn odd_campaign_documents() {
    let deep = format!("{{\"format\": \"chummer-rs campaign\", \"x\": {}{}}}", "[".repeat(100_000), "]".repeat(100_000));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("spaces", b"   ".to_vec()),
        ("brace", b"{".to_vec()),
        ("empty object", b"{}".to_vec()),
        ("wrong format", br#"{"format": "something else"}"#.to_vec()),
        ("format only", br#"{"format": "chummer-rs campaign"}"#.to_vec()),
        ("bad id", br#"{"format": "chummer-rs campaign", "id": "zz"}"#.to_vec()),
        ("non-ascii id", "{\"format\": \"chummer-rs campaign\", \"id\": \"ééééééééééééééééé\"}".as_bytes().to_vec()),
        ("member without character", br#"{"format": "chummer-rs campaign", "members": [{}]}"#.to_vec()),
        ("bad embedded xml", br#"{"format": "chummer-rs campaign", "members": [{"character": {"storage": "embedded", "xml": "<character><"}}]}"#.to_vec()),
        ("unknown storage", br#"{"format": "chummer-rs campaign", "members": [{"character": {"storage": "cloud"}}]}"#.to_vec()),
        ("traversal link", br#"{"format": "chummer-rs campaign", "members": [{"character": {"storage": "linked", "path": "../../../../etc/passwd"}}]}"#.to_vec()),
        ("deeply nested", deep.into_bytes()),
        ("lzma header only", vec![0x5D, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        ("not utf-8", b"{\"format\": \"\xff\"}".to_vec()),
    ];
    let mut failures = Vec::new();
    for (name, b) in &cases {
        if let Err(p) = no_panic(|| Campaign::from_bytes(b).map(|c| exercise(&c))) {
            failures.push(format!("{name}: {p}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let dir = common::temp_dir("campaign");
    assert!(Campaign::load(&dir).is_err(), "a directory");
    assert!(Campaign::load(&dir.join("missing.chummercampaign")).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

/// Extreme counters and scores in a campaign file reach the initiative
/// and damage arithmetic.
#[test]
fn extreme_encounter_values() {
    let mut failures = Vec::new();
    let mut known = Vec::new();
    for name in ["Ganger 4294967295", "Ganger 4294967294", "Ganger 0"] {
        let mut c = sample();
        c.members[0].name = name.into();
        if let Err(p) = no_panic(|| c.next_number("Ganger")) {
            failures.push(format!("next_number after {name:?}: {p}"));
        }
    }
    for (round, pass, score, base, dice) in [
        (u32::MAX, u32::MAX, i32::MIN, i32::MAX, u32::MAX),
        (u32::MAX, 1 << 31, i32::MAX, i32::MIN, 0),
        (0, 0, i32::MIN + 5, i32::MAX - 3, 5),
    ] {
        let mut c = sample();
        let e = &mut c.encounters[0];
        e.round = round;
        e.pass = pass;
        for x in &mut e.combatants {
            x.score = score;
            x.base = base;
            x.dice = dice;
            x.track = Some(chummer_core::campaign::initiative::AdHocTrack { physical: i32::MAX, stun: i32::MIN, physical_filled: i32::MAX, stun_filled: i32::MAX });
        }
        let text = c.to_json();
        let label = format!("round {round} pass {pass} score {score} base {base} dice {dice}");
        match no_panic(|| Campaign::from_json(&text).map(|c| exercise(&c))) {
            Ok(r) => assert!(r.is_ok(), "{label}: {r:?}"),
            Err(p) if is_known_overflow(&p) => known.push(format!("{label}: {p}")),
            Err(p) => failures.push(format!("{label}: {p}")),
        }
    }
    for s in ["", "P", "99999999999P", "8P AP-99999999999", "8 S, AP +", "８P", "8\u{301}P", "-8P", "8PAP-−4"] {
        if let Err(p) = no_panic(|| Attack::parse(s)) {
            failures.push(format!("Attack::parse({s:?}): {p}"));
        }
    }
    if !known.is_empty() {
        eprintln!("known overflow panics:\n{}", known.join("\n"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(known.is_empty(), "overflow in encounter arithmetic:\n{}", known.join("\n"));
}
