use chummer_core::character::Character;
use chummer_core::engine::Engine;

#[test]
fn engine_save_updates_totals_and_reloads() {
    let engine = Engine::load().unwrap();
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Munin_Career.chum5");
    let mut ch = Character::load(&src).unwrap();
    ch.attribute_mut("LOG").unwrap().karma += 1;
    let dir = std::env::temp_dir().join(format!("chummer-rs-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("munin.chum5");
    engine.save(&mut ch, &out).unwrap();
    assert!(!ch.dirty);
    let back = Character::load(&out).unwrap();
    let log = back.attribute("LOG").unwrap();
    assert_eq!(log.saved_total, Some(engine.sheet(&back).attr("LOG")));
    assert_eq!(log.saved_total, Some(6));
    assert_eq!(back.doc.get("appversion"), ch.doc.get("appversion"));
    std::fs::remove_dir_all(&dir).ok();
}
