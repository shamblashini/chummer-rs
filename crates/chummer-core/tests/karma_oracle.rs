//! Creation-mode fixtures save the karma left (`<karma>`). Starting karma
//! minus what chummer-rs computes as spent must give the same amount.
//!
//! Every creation fixture was saved by Chummer 5.20x with `<settings>`
//! `default.xml`: the author's own settings file, which is not available.
//! It resolves to the built-in Standard preset here. Known differences:
//!
//! - House rules of that settings file. Barrett, Blindfire, Fuzzy-chargen,
//!   Gangerbean, Harmony, Munin, Popstar and Ushi Resub have more than the
//!   quality limit in negative qualities and only match when the excess
//!   gives no karma (`exceednegativequalitiesnobonus`); Barrett has 78
//!   karma of them, which Chummer only allows with the excess rules on.
//!   resub (and Harmony) only match when mystic adept power points are
//!   bought with free spells first (`priorityspellsasadeptpowers`). One
//!   override of the shared settings file explains all nine, so the second
//!   test checks them with it.
//! - Version drift. Ocelot2.0 is a changeling whose metagenic qualities are
//!   one point out of balance: current Chummer charges that 1 karma in
//!   `CalculateBP`, 5.202 only charged it when finishing creation.
//! - Legacy gameplay options. Apex Predator and prime were made with the
//!   Prime Runner option (35 karma, contact multiplier 6, quality limit
//!   35); they match through the saved `<buildkarma>`, `<contactpoints>`
//!   and `<gameplayoptionqualitylimit>`.
//! - Bastion and Blindfire were also made with limb count 5 (nuyen only).

use std::path::PathBuf;

use chummer_core::{calc, chargen, character::Character, engine::Engine, settings::CharacterSettings};

/// Exact matches with the resolved settings must not fall below this.
const BASELINE: usize = 17;
/// Exact matches with the legacy settings file's house rules.
const HOUSE_RULE_BASELINE: usize = 26;

/// Settings flags the fixtures' shared `default.xml` evidently had.
const LEGACY_HOUSE_RULES: [&str; 2] = ["exceednegativequalitiesnobonus", "priorityspellsasadeptpowers"];

/// Count exact matches; `adjust` may change the resolved settings.
fn run(label: &str, adjust: impl Fn(&Character, &mut CharacterSettings)) -> usize {
    let engine = Engine::load().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let (mut ok, mut total) = (0, 0);
    for f in files {
        let ch = Character::load(&f).unwrap();
        if ch.created {
            continue;
        }
        let Some(settings) = engine.settings.resolve(&ch.field("settings")) else { continue };
        let mut settings = settings.clone();
        adjust(&ch, &mut settings);
        let store = engine.store_for_character(&ch);
        let rules = engine.rules_for(&ch);
        let sheet = calc::compute(&ch, &rules, Some(&store), Some(&engine.catalog));
        let b = chargen::budget_with(&ch, &sheet, &rules, &settings, Some(&store));
        let saved = ch.doc.get_i32("karma").unwrap_or(0);
        total += 1;
        let hit = b.karma_left() == saved;
        if hit {
            ok += 1;
        }
        eprintln!("{} {:<28} saved {saved:>4} computed {:>4} (start {}, spent {})", if hit { "ok  " } else { "DIFF" }, f.file_name().unwrap().to_string_lossy(), b.karma_left(), b.karma.0, b.karma.1);
    }
    eprintln!("karma oracle ({label}): {ok} of {total} creation-mode characters");
    ok
}

#[test]
fn creation_karma_left_matches_chummer() {
    assert!(run("resolved settings", |_, _| {}) >= BASELINE);
}

#[test]
fn creation_karma_left_with_legacy_house_rules() {
    let ok = run("legacy default.xml house rules", |ch, settings| {
        if ch.field("settings") == "default.xml" {
            for flag in LEGACY_HOUSE_RULES {
                settings.raw.set_child_text(flag, "True");
            }
        }
    });
    assert!(ok >= HOUSE_RULE_BASELINE);
}
