//! Times the phases of saving a big character (run with --ignored --nocapture).
use std::time::Instant;
use chummer_core::{calc, character::Character, engine::Engine};

#[test]
#[ignore]
fn save_phases() {
    let Ok(engine) = Engine::load() else { return };
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Ghile Mear.chum5");
    let t = Instant::now();
    let ch = Character::load(&p).unwrap();
    eprintln!("load {:?}", t.elapsed());
    let t = Instant::now();
    let mut copy = ch.clone();
    eprintln!("clone {:?}", t.elapsed());
    let rules = engine.rules_for(&copy);
    let store = engine.store_for_character(&copy);
    let t = Instant::now();
    let sheet = calc::compute(&copy, &rules, Some(&store), Some(&engine.catalog));
    eprintln!("compute {:?}", t.elapsed());
    let t = Instant::now();
    calc::stamp_totals(&mut copy, &sheet, &rules);
    eprintln!("stamp {:?}", t.elapsed());
    let t = Instant::now();
    let doc = copy.to_document();
    eprintln!("to_document {:?}", t.elapsed());
    let t = Instant::now();
    let s = doc.to_xml_string();
    eprintln!("to_xml_string {:?} ({} bytes)", t.elapsed(), s.len());
    let out = std::env::temp_dir().join("chummer-save-timing.chum5");
    let t = Instant::now();
    chummer_core::chum5lz::write_text(&out, &s).unwrap();
    eprintln!("write {:?}", t.elapsed());
    let out = std::env::temp_dir().join("chummer-save-timing.chum5lz");
    let t = Instant::now();
    chummer_core::chum5lz::write_text(&out, &s).unwrap();
    eprintln!("write lz {:?}", t.elapsed());
}

#[test]
#[ignore]
fn snapshot_presets() {
    use std::io::Write;
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Ghile Mear.chum5");
    let ch = Character::load(&p).unwrap();
    let t = Instant::now();
    let snap = chummer_core::command::snapshot(&ch);
    eprintln!("snapshot (balanced) {:?} -> {} bytes", t.elapsed(), snap.len());
    let t = Instant::now();
    let _ = chummer_core::command::restore(&snap).unwrap();
    eprintln!("restore {:?}", t.elapsed());
    let t = Instant::now();
    let h = chummer_core::command::state_hash(&ch);
    eprintln!("state_hash {:?} {}", t.elapsed(), h[0]);
    let text = ch.to_xml_string();
    for preset in [0u32, 1, 2, 3, 6] {
        let t = Instant::now();
        let mut w = lzma_rust2::LzmaWriter::new_use_header(Vec::new(), &lzma_rust2::LzmaOptions::with_preset(preset), None).unwrap();
        w.write_all(text.as_bytes()).unwrap();
        let out = w.finish().unwrap();
        eprintln!("preset {preset}: {:?} -> {} bytes", t.elapsed(), out.len());
    }
}
