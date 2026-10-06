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

/// Chummer refreshes essence loss when Essence changes. A career save in
/// which that happened (it has `EssenceLoss` improvements) holds Chummer's
/// output, and refreshing reproduces it. On every career save a refresh
/// burns no karma, keeps the saved MAG/RES/DEP totals and is idempotent
/// (LB-33: the RAW career branch was not ported before). Saves never
/// refreshed in career (Draught) may gain a DEP improvement Chummer would
/// also add, on a DEP whose maximum is 0 anyway (compare Munin_Career).
#[test]
fn career_mode_essence_loss_matches_saved() {
    let engine = chummer_core::engine::Engine::load().unwrap();
    let (mut checked, mut with_loss) = (0, 0);
    let mut bad = Vec::new();
    for path in fixtures() {
        let mut ch = Character::load(&path).unwrap();
        if !ch.created {
            continue;
        }
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let rules = engine.rules_for(&ch);
        let store = engine.store_for_character(&ch);
        let before = essence_loss(&ch);
        let karma: Vec<(String, i32)> = ch.attributes.iter().map(|a| (a.name.clone(), a.karma)).collect();
        let pp = ch.doc.get("magsplitadept");
        essence_loss::refresh(&mut ch, &store, &rules);
        let once = essence_loss(&ch);
        checked += 1;
        if before.iter().any(|b| b.starts_with("EssenceLoss ")) {
            with_loss += 1;
            if once != before {
                bad.push(format!("{fname}:\n  saved {before:?}\n  got   {once:?}"));
            }
        }
        let after: Vec<(String, i32)> = ch.attributes.iter().map(|a| (a.name.clone(), a.karma)).collect();
        assert_eq!(after, karma, "{fname}: no karma burnt");
        assert_eq!(ch.doc.get("magsplitadept"), pp, "{fname}: no power points burnt");
        let sheet = calc::compute(&ch, &rules, Some(&store), None);
        for a in &ch.attributes {
            if let (Some(t), true) = (a.saved_total, ["MAG", "MAGAdept", "RES", "DEP"].contains(&a.name.as_str())) {
                assert_eq!(sheet.attr(&a.name), t, "{fname} {} total after refresh", a.name);
            }
        }
        essence_loss::refresh(&mut ch, &store, &rules);
        assert_eq!(essence_loss(&ch), once, "{fname}: a second refresh changes nothing");
    }
    eprintln!("career essence loss: {checked} career fixtures, {with_loss} refreshed in career");
    for b in &bad {
        eprintln!("{b}");
    }
    assert!(checked >= 5 && with_loss >= 1);
    assert!(bad.is_empty(), "{} of {with_loss} differ", bad.len());
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

/// RAW career mode (SR5 p. 95; LB-33): new essence loss lowers the MAG
/// maximum and minimum; once the minimum cannot drop further, karma
/// levels burn, once per point lost.
#[test]
fn career_mode_essence_loss_lowers_mag_and_burns_karma() {
    use chummer_core::data;
    use chummer_core::items::{self, Purchase};
    let engine = chummer_core::engine::Engine::load().unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Wesson.chum5");
    let mut ch = Character::load(&path).unwrap();
    assert!(ch.created && ch.mag_enabled());
    let rules = engine.rules_for(&ch);
    let store = engine.store_for_character(&ch);
    {
        let mag = ch.attribute_mut("MAG").unwrap();
        mag.metatype_min = 1;
        mag.karma = 2;
    }
    let mag = |ch: &Character| calc::attribute_values_with(ch, "MAG", &rules, Some(&store));
    assert_eq!((mag(&ch).total_max, mag(&ch).total), (6, 3));
    let add = |ch: &mut Character, name: &str| {
        let kind = items::kind("bioware").unwrap();
        let doc = store.doc(kind.file).unwrap();
        let rec = data::find(&doc, kind.data_container, kind.data_item, name).unwrap();
        items::add("bioware", ch, &store, rec, &Purchase { rating: 4, ..Default::default() }).unwrap();
        essence_loss::refresh(ch, &store, &rules);
    };
    // 0.8 essence: maximum and minimum drop by 1; the karma stays.
    add(&mut ch, "Muscle Toner");
    // RES and DEP get the same improvements, as in Chummer.
    let career: Vec<String> = essence_loss(&ch).into_iter().filter(|s| s.starts_with("EssenceLoss ")).collect();
    let all = ["DEP", "MAG", "MAGAdept", "RES"].map(|a| format!("EssenceLoss Attribute {a} min=-1 max=-1 aug=0 val=0"));
    assert_eq!(career, all);
    assert_eq!(ch.attribute("MAG").unwrap().karma, 2);
    assert_eq!(mag(&ch).total_max, 5);
    // 1.6 essence: a second point; the minimum (1) cannot drop to -1, so
    // one karma level burns.
    add(&mut ch, "Muscle Augmentation");
    assert_eq!(ch.attribute("MAG").unwrap().karma, 1);
    assert_eq!(mag(&ch).total_max, 4);
    assert!(essence_loss(&ch).contains(&"EssenceLoss Attribute MAG min=-2 max=-2 aug=0 val=0".to_owned()));
    // Refreshing again burns nothing.
    essence_loss::refresh(&mut ch, &store, &rules);
    assert_eq!(ch.attribute("MAG").unwrap().karma, 1);
}
