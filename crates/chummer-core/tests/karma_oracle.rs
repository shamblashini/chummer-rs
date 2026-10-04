//! Creation-mode fixtures save the karma left (`<karma>`). Starting karma
//! minus what chummer-rs computes as spent must give the same amount.

use std::path::PathBuf;

use chummer_core::{calc, chargen, character::Character, engine::Engine};

/// Exact matches must not fall below this.
const BASELINE: usize = 12;

#[test]
fn creation_karma_left_matches_chummer() {
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
        let store = engine.store_for_character(&ch);
        let rules = engine.rules_for(&ch);
        let sheet = calc::compute(&ch, &rules, Some(&store), Some(&engine.catalog));
        let b = chargen::budget_with(&ch, &sheet, &rules, settings, Some(&store));
        let saved = ch.doc.get_i32("karma").unwrap_or(0);
        total += 1;
        let hit = b.karma_left() == saved;
        if hit {
            ok += 1;
        }
        eprintln!("{} {:<28} saved {saved:>4} computed {:>4} (start {}, spent {})", if hit { "ok  " } else { "DIFF" }, f.file_name().unwrap().to_string_lossy(), b.karma_left(), b.karma.0, b.karma.1);
    }
    eprintln!("karma oracle: {ok} of {total} creation-mode characters");
    assert!(ok >= BASELINE);
}
