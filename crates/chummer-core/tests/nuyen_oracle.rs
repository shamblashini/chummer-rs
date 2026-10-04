//! Creation-mode fixtures save the nuyen left after shopping. Starting
//! nuyen (+ karma converted) minus what chummer-rs computes for everything
//! owned must give the same amount.

use std::path::PathBuf;

use chummer_core::chargen;
use chummer_core::character::Character;
use chummer_core::engine::Engine;

/// Exact matches must not fall below this.
const BASELINE: usize = 6;

#[test]
fn creation_nuyen_left_matches_chummer() {
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
        let Some(saved) = ch.doc.get_f64("nuyen") else { continue };
        let store = engine.store_for_character(&ch);
        let start = ch.doc.get_f64("startingnuyen").unwrap_or(0.0) + f64::from(ch.doc.get_i32("nuyenbp").unwrap_or(0)) * 2000.0;
        let left = start - chargen::nuyen_spent(&ch, Some(&store));
        total += 1;
        let hit = (left - saved).abs() < 0.5;
        if hit {
            ok += 1;
        }
        eprintln!("{} {:<28} saved {saved:>10} computed {left:>10}", if hit { "ok  " } else { "DIFF" }, f.file_name().unwrap().to_string_lossy());
    }
    eprintln!("nuyen oracle: {ok} of {total} creation-mode characters");
    assert!(ok >= BASELINE);
}
