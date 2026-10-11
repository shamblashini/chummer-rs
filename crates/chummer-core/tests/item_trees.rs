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
            // A quality's levels (copies with one source id, source name
            // and extra) are one node, the first copy (`RefreshQualities`).
            if sec.container == "qualities" {
                let mut seen = std::collections::HashSet::new();
                for q in doc.child("qualities").into_iter().flat_map(|c| c.children_named("quality")) {
                    let (id, extra, source, _) = chummer_core::items::quality::level_key(q);
                    if !seen.insert((id, source, extra)) {
                        want.remove(&q.get("guid").to_lowercase());
                    }
                }
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

/// `edit::with_index` answers `find` and `parent` exactly as the search
/// does: the same element (by address) for every item of every fixture,
/// unknown guids too.
#[test]
fn indexed_lookups_match_the_search() {
    use chummer_core::items::edit;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for f in std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")) {
        let ch = chummer_core::character::Character::load(&f).unwrap();
        let mut guids = Vec::new();
        fn collect(e: &Element, out: &mut Vec<String>) {
            if edit::is_item(e) {
                out.push(e.get("guid"));
            }
            e.elements().for_each(|c| collect(c, out));
        }
        collect(&ch.doc, &mut guids);
        guids.push("00000000-0000-0000-0000-000000000000".into());
        let plain: Vec<_> = guids.iter().map(|g| (edit::find(&ch, g).map(|e| e as *const Element), edit::parent(&ch, g).map(|e| e as *const Element))).collect();
        let indexed: Vec<_> = edit::with_index(&ch, || guids.iter().map(|g| (edit::find(&ch, g).map(|e| e as *const Element), edit::parent(&ch, g).map(|e| e as *const Element))).collect());
        assert_eq!(plain, indexed, "{}", f.display());
        // Upper-case guids find the same items.
        let upper = edit::with_index(&ch, || guids.iter().map(|g| edit::find(&ch, &g.to_ascii_uppercase()).map(|e| e as *const Element)).collect::<Vec<_>>());
        assert_eq!(plain.iter().map(|p| p.0).collect::<Vec<_>>(), upper);
        checked += guids.len();
    }
    assert!(checked > 1000, "{checked}");
    // Outside the call the index is gone: another character's lookups search.
    let ch = fixture_doc_character();
    assert!(edit::find(&ch, "nope").is_none());
}

fn fixture_doc_character() -> chummer_core::character::Character {
    chummer_core::character::Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Serpent.chum5")).unwrap()
}
