//! Saving through the engine, as `.chum5` and `.chum5lz`, keeps every
//! fixture's state hash, and a second save writes the same bytes.
//! (`commands.rs::saved_xml_is_a_fixed_point` covers the plain
//! `to_xml_string` / snapshot round trip.)

mod common;

use chummer_core::character::Character;
use chummer_core::command;
use common::engine;

#[test]
fn engine_saves_keep_the_state_hash() {
    let dir = common::temp_dir("roundtrip");
    let mut failures = Vec::new();
    let fixtures = common::fixtures();
    assert_eq!(fixtures.len(), 34);
    for path in &fixtures {
        let name = common::fixture_name(path);
        let ch = Character::load(path).unwrap();
        let hash = command::state_hash(&ch);
        // The hash does not depend on where the character came from.
        let mut moved = ch.clone();
        moved.file = None;
        moved.dirty = true;
        if command::state_hash(&moved) != hash {
            failures.push(format!("{name}: hash depends on file/dirty"));
        }
        if ch.to_document() != Character::from_document(ch.to_document()).unwrap().to_document() {
            failures.push(format!("{name}: to_document is not idempotent"));
        }
        let mut saved_texts = Vec::new();
        // Compressing is slow in debug builds: `.chum5lz` for the smaller
        // fixtures only, unless this is a long run.
        let small = std::fs::metadata(path).unwrap().len() < 250_000 || std::env::var_os("CHUMMER_FUZZ_ITERS").is_some();
        let exts: &[&str] = if small { &["chum5", "chum5lz"] } else { &["chum5"] };
        for ext in exts {
            let out = dir.join(format!("out.{ext}"));
            let mut c = ch.clone();
            engine().save(&mut c, &out).unwrap();
            let back = Character::load(&out).unwrap_or_else(|e| panic!("{name}.{ext}: {e}"));
            if command::state_hash(&back) != hash {
                failures.push(format!("{name}: saving as .{ext} changed the state hash"));
            }
            // Saving what was loaded again writes the same text.
            let first = chummer_core::chum5lz::read_text(&out).unwrap();
            if *ext == "chum5" {
                let mut again = back.clone();
                engine().save(&mut again, &out).unwrap();
                if chummer_core::chum5lz::read_text(&out).unwrap() != first {
                    failures.push(format!("{name}: a second .{ext} save differs from the first"));
                }
            }
            saved_texts.push(first);
        }
        if saved_texts.len() == 2 && saved_texts[0] != saved_texts[1] {
            failures.push(format!("{name}: .chum5 and .chum5lz saves hold different XML"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
