//! Section trees over the fixtures: every saved item shows up exactly once,
//! and building plus flattening the biggest list stays cheap.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use chummer_core::sections;
use chummer_core::tree::{flatten, section_tree, Entry};
use chummer_core::xml::{self, Element};

/// Item element names that the tree shows, per section container.
const SHOWN: &[&str] = &["gear", "cyberware", "armor", "armormod", "weapon", "accessory", "vehicle", "mod", "weaponmount", "quality", "spell", "contact", "martialart", "martialarttechnique"];

fn count_items(e: &Element, out: &mut HashMap<String, usize>) {
    for c in e.elements() {
        if SHOWN.contains(&c.name.as_str()) && !c.get("guid").is_empty() && c.child("name").is_some() {
            *out.entry(c.get("guid").to_lowercase()).or_default() += 1;
        }
        count_items(c, out);
    }
}

#[test]
fn every_item_appears_once() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let secs = [sections::GEAR, sections::CYBERWARE, sections::ARMOR, sections::WEAPONS, sections::VEHICLES, sections::QUALITIES, sections::SPELLS, sections::CONTACTS, sections::MARTIAL_ARTS];
    let mut files = 0;
    for f in std::fs::read_dir(&dir).unwrap() {
        let p = f.unwrap().path();
        if p.extension().is_none_or(|e| e != "chum5") {
            continue;
        }
        files += 1;
        let doc = xml::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
        for sec in &secs {
            let mut want = HashMap::new();
            if let Some(c) = doc.child(sec.container) {
                count_items(c, &mut want);
            }
            let tree = section_tree(&doc, sec);
            let mut got: HashMap<String, usize> = HashMap::new();
            for r in flatten(&tree, &|_| true) {
                if let Entry::Item { el, .. } = r.node.value {
                    if want.contains_key(&el.get("guid").to_lowercase()) {
                        *got.entry(el.get("guid").to_lowercase()).or_default() += 1;
                    }
                }
            }
            assert_eq!(got, want, "{} {}", p.display(), sec.container);
        }
    }
    assert!(files > 20);
}

#[test]
fn big_gear_tree_is_cheap() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Serpent.chum5");
    let doc = xml::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
    let t = Instant::now();
    let mut rows = 0;
    for _ in 0..100 {
        let tree = section_tree(&doc, &sections::GEAR);
        rows = flatten(&tree, &|_| true).len();
    }
    let per_frame = t.elapsed() / 100;
    assert!(rows > 100, "{rows}");
    // Generous even for unoptimised test builds.
    assert!(per_frame.as_millis() < 20, "{per_frame:?} per build");
}
