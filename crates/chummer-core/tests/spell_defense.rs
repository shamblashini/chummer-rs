//! Spell defense pools (Chummer's Spell Defense tab).

use std::path::PathBuf;

use chummer_core::calc::spell_defense;
use chummer_core::character::Character;
use chummer_core::custom_improvement::{self as custom, Form};
use chummer_core::engine::Engine;

fn pool(engine: &Engine, ch: &Character, key: &str) -> i32 {
    let s = engine.sheet(ch);
    spell_defense(ch, &s).into_iter().find(|(k, _)| *k == key).unwrap().1
}

#[test]
fn formulas_and_damage_resistance() {
    let engine = Engine::load().unwrap();
    let mut ch = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Glessner.chum5")).unwrap();
    let s = engine.sheet(&ch);
    let pools = spell_defense(&ch, &s);
    assert_eq!(pools.len(), 17);
    assert_eq!(pool(&engine, &ch, "Label_SpellDefenseDecAttWIL"), 2 * s.attr("WIL"));
    assert_eq!(pool(&engine, &ch, "Label_SpellDefenseManipPhysical"), s.attr("BOD") + s.attr("STR"));
    let before = pool(&engine, &ch, "Label_SpellDefenseIndirect");
    assert_eq!(before, s.attr("BOD") + s.armor);
    let f = Form { type_id: "damageresistance".into(), name: "GM bonus".into(), val: 2.0, ..Default::default() };
    custom::create(&mut ch, &engine.store, &f, "", None).unwrap();
    assert_eq!(pool(&engine, &ch, "Label_SpellDefenseIndirect"), before + 2);
}
