//! Compare creation karma left with the saved <karma> (debugging aid).
use chummer_core::{calc, chargen, character::Character, engine::Engine};
fn main() {
    let engine = Engine::load().unwrap();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for f in files {
        let ch = Character::load(&f).unwrap();
        if ch.created { continue; }
        let st = engine.store_for_character(&ch);
        let rules = engine.rules_for(&ch);
        let sheet = calc::compute(&ch, &rules, Some(&st), Some(&engine.catalog));
        let Some(set) = engine.settings.resolve(&ch.field("settings")) else { continue };
        let b = chargen::budget_with(&ch, &sheet, &rules, set, Some(&st));
        println!("{:<28} saved karma {:>4} buildkarma {:>4} | ours start {} spent {} left {}", f.file_name().unwrap().to_string_lossy(), ch.field("karma"), ch.field("buildkarma"), b.karma.0, b.karma.1, b.karma_left());
    }
}
