//! Regenerate essence-loss improvements for every creation-mode fixture and
//! compare with the `EssenceLossChargen` improvements Chummer5a saved.

use std::path::PathBuf;

use chummer_core::calc::{self, Rules, SkillCatalog};
use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::essence_loss;
use chummer_core::improvement::Improvement;

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "chum5")).collect();
    v.sort();
    v
}

/// The per-fixture house rules `tests/oracle.rs` uses.
fn rules_for(fname: &str) -> Rules {
    let mut rules = Rules::default();
    match fname {
        "Bastion.chum5" | "Blindfire.chum5" => rules.limb_count = 5,
        "Fuzzy-chargen.chum5" => rules.essence_decimals = 3,
        _ => {}
    }
    rules
}

/// Essence-loss improvements as a sorted, comparable list.
fn essence_loss(ch: &Character) -> Vec<String> {
    let mut v: Vec<String> = ch
        .improvements
        .list
        .iter()
        .filter(|i| i.source.starts_with("EssenceLoss"))
        .map(|i: &Improvement| format!("{} {} {} min={} max={} aug={} val={}", i.source, i.kind, i.improved_name, i.min, i.max, i.aug, i.val))
        .collect();
    v.sort();
    v
}

#[test]
fn creation_mode_essence_loss_matches_saved() {
    let store = DataStore::discover().unwrap();
    let cat = SkillCatalog::load(&store).unwrap();
    let (mut checked, mut matched, mut with_loss) = (0, 0, 0);
    let mut bad = Vec::new();
    for path in fixtures() {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let mut ch = Character::load(&path).unwrap();
        if ch.created {
            continue;
        }
        let rules = rules_for(&fname);
        let saved = essence_loss(&ch);
        essence_loss::refresh(&mut ch, &store, &rules);
        let got = essence_loss(&ch);
        checked += 1;
        if !saved.is_empty() {
            with_loss += 1;
        }
        if got == saved {
            matched += 1;
        } else {
            bad.push(format!("{fname}:\n  saved {saved:?}\n  got   {got:?}"));
        }
        // Regenerated improvements keep the attribute totals Chummer saved.
        let sheet = calc::compute(&ch, &rules, Some(&store), Some(&cat));
        for a in &ch.attributes {
            if let (Some(t), true) = (a.saved_total, ["MAG", "MAGAdept", "RES", "DEP"].contains(&a.name.as_str())) {
                assert_eq!(sheet.attr(&a.name), t, "{fname} {} total after refresh", a.name);
            }
        }
    }
    eprintln!("essence loss: {matched} / {checked} creation-mode fixtures match ({with_loss} with saved essence loss)");
    for b in &bad {
        eprintln!("{b}");
    }
    assert!(checked >= 25);
    assert!(bad.is_empty(), "{} of {checked} differ", bad.len());
}

#[test]
fn career_mode_raw_is_left_alone() {
    let store = DataStore::discover().unwrap();
    for path in fixtures() {
        let mut ch = Character::load(&path).unwrap();
        if !ch.created {
            continue;
        }
        let before = essence_loss(&ch);
        essence_loss::refresh(&mut ch, &store, &Rules::default());
        assert_eq!(essence_loss(&ch), before, "{}", path.display());
    }
}

#[test]
fn mundane_characters_lose_stale_improvements() {
    let store = DataStore::discover().unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Barrett.chum5");
    let mut ch = Character::load(&path).unwrap();
    ch.improvements.list.push(Improvement { source: "EssenceLossChargen".into(), kind: "Attribute".into(), improved_name: "MAG".into(), min: -1.0, max: -1.0, enabled: true, rating: 1, ..Default::default() });
    essence_loss::refresh(&mut ch, &store, &Rules::default());
    assert!(essence_loss(&ch).is_empty());
}
