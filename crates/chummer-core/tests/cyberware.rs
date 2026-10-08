//! Adding, pricing and removing cyberware/bioware on a fixture character.

use std::path::PathBuf;

use chummer_core::calc::Rules;
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::essence_loss;
use chummer_core::items::{self, cyberware, Purchase};
use chummer_core::xml::Element;

fn harmony() -> Character {
    // Creation mode, magician, essence 6, no ware.
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Harmony.chum5")).unwrap()
}

fn find<'a>(ch: &'a Character, guid: &str) -> &'a Element {
    fn walk<'a>(e: &'a Element, g: &str) -> Option<&'a Element> {
        if e.get("guid") == g {
            return Some(e);
        }
        e.elements().find_map(|c| walk(c, g))
    }
    walk(ch.doc.child("cyberwares").unwrap(), guid).unwrap()
}

fn add(ch: &mut Character, store: &DataStore, tag: &str, name: &str, p: Purchase) -> String {
    let kind = items::kind(tag).unwrap();
    let doc = store.doc(kind.file).unwrap();
    let rec = data::find(&doc, kind.data_container, kind.data_item, name).unwrap();
    items::add(tag, ch, store, rec, &p).unwrap()
}

#[test]
fn control_rig_with_included_datajack() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let g = add(&mut ch, &store, "cyberware", "Control Rig", Purchase { rating: 2, grade: Some("Alphaware".into()), ..Default::default() });
    let w = find(&ch, &g);
    assert_eq!(w.get("grade"), "Alphaware");
    assert_eq!(w.get("rating"), "2");
    assert_eq!(w.get("improvementsource"), "Cyberware");
    assert_eq!(w.get("sourceid"), w.get("sourceid").to_lowercase());
    let kids: Vec<&Element> = w.child("children").unwrap().children_named("cyberware").collect();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].get("name"), "Datajack");
    assert_eq!(kids[0].get("cost"), "0");
    assert_eq!(kids[0].get("grade"), "Alphaware");
    assert_eq!(kids[0].get("parentid"), g);
    // FixedValues(43000,97000,208000) at rating 2, × 1.2 for alphaware.
    assert!((cyberware::cost(&ch, &store, w) - 97000.0 * 1.2).abs() < 0.01);
    // Essence: 2 × 0.8; the datajack is not added to the parent's essence.
    assert!((cyberware::essence(&ch, &store, &Rules::default(), w) - 1.6).abs() < 1e-9);
    // The rebuilt element equals what was added.
    assert_eq!(cyberware::rebuild(&ch, &store, w).unwrap(), w.clone());
}

#[test]
fn rating_is_clamped_and_forced_grade_respected() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let g = add(&mut ch, &store, "cyberware", "Control Rig", Purchase { rating: 9, ..Default::default() });
    assert_eq!(find(&ch, &g).get("rating"), "3");
    assert_eq!(find(&ch, &g).get("grade"), "Standard");
    let g = add(&mut ch, &store, "cyberware", "Control Rig", Purchase { rating: 0, ..Default::default() });
    assert_eq!(find(&ch, &g).get("rating"), "1");
}

#[test]
fn bioware_bonus_and_essence_loss() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let g = add(&mut ch, &store, "bioware", "Muscle Toner", Purchase { rating: 4, ..Default::default() });
    let w = find(&ch, &g);
    assert_eq!(w.name, "cyberware");
    assert_eq!(w.get("improvementsource"), "Bioware");
    let agi: Vec<_> = ch.improvements.list.iter().filter(|i| i.source_name == g).collect();
    assert!(agi.iter().any(|i| i.kind == "Attribute" && i.improved_name == "AGI" && i.source == "Bioware" && i.aug == 4.0 && i.unique_name == "muscle"));
    // 0.8 essence lost: MAG maximum and minimum drop by 1.
    essence_loss::refresh(&mut ch, &store, &Rules::default());
    let mag = ch.improvements.list.iter().find(|i| i.source == "EssenceLossChargen" && i.improved_name == "MAG").unwrap();
    assert_eq!((mag.min, mag.max), (-1.0, -1.0));
    // Removing the ware removes its improvements; refresh drops the loss.
    assert!(cyberware::remove(&mut ch, &g));
    assert!(!ch.improvements.list.iter().any(|i| i.source_name == g));
    essence_loss::refresh(&mut ch, &store, &Rules::default());
    assert!(!ch.improvements.list.iter().any(|i| i.source == "EssenceLossChargen"));
}

#[test]
fn pair_bonus_applies_to_the_second_of_a_pair() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let a = add(&mut ch, &store, "cyberware", "Skimmers", Purchase::default());
    assert!(!ch.improvements.list.iter().any(|i| i.source_name == format!("{a}Pair")));
    let b = add(&mut ch, &store, "cyberware", "Skimmers", Purchase::default());
    assert!(ch.improvements.list.iter().any(|i| i.source_name == format!("{b}Pair")));
}

#[test]
fn cost_with_grade_suite_and_children() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let g = add(&mut ch, &store, "cyberware", "Cybereyes Basic System", Purchase { rating: 2, grade: Some("Betaware".into()), ..Default::default() });
    // 4000 × 2 − 2000 = 6000, × 1.5 for betaware; Image Link is included at 0.
    assert!((cyberware::cost(&ch, &store, find(&ch, &g)) - 9000.0).abs() < 0.01);
    let mut w = find(&ch, &g).clone();
    w.set_child_text("suite", "True");
    assert!((cyberware::cost(&ch, &store, &w) - 8100.0).abs() < 0.01);
    w.set_child_text("discountedcost", "True");
    assert!((cyberware::cost(&ch, &store, &w) - 7290.0).abs() < 0.01);
}

#[test]
fn quality_grants_free_ware() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let doc = store.doc("qualities.xml").unwrap();
    let rec = data::find(&doc, "qualities", "quality", "Busted Cyberware").unwrap();
    let q = chummer_core::chargen::add_quality(&mut ch, &store, rec, Some("Bad wiring"));
    let ware = ch.items("cyberwares", "cyberware").into_iter().find(|w| w.get("name") == "Busted Ware").cloned().unwrap();
    assert_eq!(ware.get("parentid"), q);
    assert_eq!(ware.get("cost"), "0");
    assert_eq!(ware.get("grade"), "None");
    let free = ch.improvements.list.iter().find(|i| i.kind == "FreeWare").unwrap();
    assert_eq!(free.improved_name, ware.get("guid"));
    assert_eq!(free.source_name, q);
    assert_eq!(cyberware::rebuild(&ch, &store, &ware).unwrap(), ware);
}

#[test]
fn remove_takes_bonus_objects_along() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let before = ch.items("limitmodifiers", "limitmodifier").len();
    let g = add(&mut ch, &store, "cyberware", "Math SPU", Purchase::default());
    assert_eq!(ch.items("limitmodifiers", "limitmodifier").len(), before + 1);
    assert!(cyberware::remove(&mut ch, &g));
    assert_eq!(ch.items("limitmodifiers", "limitmodifier").len(), before);
    assert!(!ch.improvements.list.iter().any(|i| i.source_name == g));
}

#[test]
fn refresh_does_not_dirty_an_unchanged_character() {
    let store = DataStore::discover().unwrap();
    let mut ch = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Munin.chum5")).unwrap();
    essence_loss::refresh(&mut ch, &store, &Rules::default());
    assert!(!ch.dirty);
}

#[test]
fn grade_list() {
    let store = DataStore::discover().unwrap();
    let ch = harmony();
    let doc = store.doc("cyberware.xml").unwrap();
    let rec = data::find(&doc, "cyberwares", "cyberware", "Control Rig").unwrap();
    let g = cyberware::grades(&ch, &store, false, rec);
    assert!(g.contains(&"Standard".to_owned()) && g.contains(&"Alphaware".to_owned()));
    assert!(!g.iter().any(|n| n == "None" || n.contains("Adapsin") || n.contains("Burnout")));
}

#[test]
fn availability_includes_grade() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let g = add(&mut ch, &store, "cyberware", "Control Rig", Purchase { rating: 2, grade: Some("Deltaware".into()), ..Default::default() });
    // (2 × 5)R + 8 for deltaware.
    assert_eq!(cyberware::availability(&ch, &store, find(&ch, &g)).to_string(), "18R");
}

#[test]
fn cost_shares_add_up_to_the_total() {
    // Ghile Mear's cyberlegs: customizations priced from the limb's
    // minimum rating, plus spurs and skimmers.
    let store = DataStore::discover().unwrap();
    let ch = Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Ghile Mear.chum5")).unwrap();
    let mut legs = 0;
    for w in ch.doc.child("cyberwares").unwrap().children_named("cyberware") {
        let shares = cyberware::cost_shares(&ch, &store, w);
        let total = cyberware::cost(&ch, &store, w);
        let sum: f64 = shares.iter().map(|(_, v)| v).sum();
        assert!((sum - total).abs() < 0.01, "{}: shares {shares:?} sum {sum} vs total {total}", w.get("name"));
        assert!(shares.iter().all(|(_, v)| *v >= 0.0), "{shares:?}");
        assert_eq!(shares.last().map(|(g, _)| g.clone()), Some(w.get("guid")), "the ware's own share comes last");
        if w.get("name") == "Obvious Full Leg" {
            legs += 1;
            let own = shares.last().unwrap().1;
            assert!((own - 15000.0).abs() < 0.01, "the leg itself costs 15,000¥, got {own}");
            assert_eq!(shares.len(), 5, "the leg and its four mods");
        }
    }
    assert_eq!(legs, 2);
}
