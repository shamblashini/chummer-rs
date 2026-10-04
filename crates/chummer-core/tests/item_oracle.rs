//! Rebuild every saved item in the fixtures from its data record plus the
//! choices stored in it, and compare field by field with what Chummer5a
//! wrote. Each item kind plugs in through `items::rebuild`.
//!
//! Run with ITEM_ORACLE_VERBOSE=1 to see mismatches, ITEM_ORACLE_KIND=gear
//! to focus on one kind.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::items;
use chummer_core::xml::Element;

/// Exact matches per kind must not fall below these. Raise as kinds land.
const BASELINES: &[(&str, usize)] = &[
    ("quality", 291),
    ("vehicle", 24),
    ("mod", 33),
    ("weaponmount", 10),
    ("cyberware", 139),
    ("spell", 100),
    ("power", 64),
    ("complexform", 17),
    ("spirit", 7),
    ("metamagic", 2),
    ("martialart", 5),
    ("critterpower", 0),
    ("mentorspirit", 4),
];
const BASELINES: &[(&str, usize)] = &[("quality", 291), ("gear", 1236), ("lifestyle", 24)];
const BASELINES: &[(&str, usize)] = &[("quality", 291), ("armor", 54), ("armormod", 80), ("weapon", 67), ("accessory", 140)];

/// Never compared: per-instance state, user input, or presentation.
const COMMON_IGNORE: &[&str] = &[
    "guid", "notes", "notesColor", "location", "parentid", "matrixcmfilled", "matrixcmbonus", "discountedcost", "wirelesson", "equipped",
    "active", "homenode", "sortorder", "mainmugshotindex", "mugshots",
];

/// Saved element tags the oracle visits, mapped to `items` kind tags.
const TAGS: &[&str] = &[
    "quality", "gear", "cyberware", "armor", "armormod", "weapon", "accessory", "vehicle", "mod", "weaponmount", "lifestyle", "drug", "spell", "power",
    "complexform", "spirit", "metamagic", "martialart", "critterpower", "mentorspirit",
];

fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
    for c in e.elements() {
        if TAGS.contains(&c.name.as_str()) && !c.get("guid").is_empty() {
            out.push(c);
        }
        walk(c, out);
    }
}

/// Text of a subtree without attributes or layout, for comparing bonuses.
fn flat(e: &Element) -> String {
    let mut s = format!("<{}>", e.name);
    let kids: Vec<&Element> = e.elements().collect();
    if kids.is_empty() {
        s.push_str(e.text().trim());
    }
    for k in kids {
        s.push_str(&flat(k));
    }
    s
}

/// Field-level differences between saved and rebuilt. Fields the rebuild
/// writes but the (older) save lacks are fine.
fn diff(tag: &str, saved: &Element, rebuilt: &Element) -> Vec<String> {
    let ignore = items::ignored(tag);
    let mut out = Vec::new();
    for s in saved.elements() {
        let name = s.name.as_str();
        if COMMON_IGNORE.contains(&name) || ignore.contains(&name) {
            continue;
        }
        let Some(r) = rebuilt.child(name) else {
            out.push(format!("{name}: missing"));
            continue;
        };
        let has_kids = s.elements().next().is_some() || r.elements().next().is_some();
        if has_kids {
            // Nested lists: compare how many entries; other subtrees by content.
            let ns = s.elements().count();
            let nr = r.elements().count();
            let list_like = s.elements().all(|k| k.child("guid").is_some()) && ns > 0;
            if list_like {
                if ns != nr {
                    out.push(format!("{name}: {ns} saved vs {nr} rebuilt children"));
                }
            } else if flat(s) != flat(r) {
                out.push(format!("{name}: subtree differs"));
            }
        } else if s.text().trim() != r.text().trim() {
            out.push(format!("{name}: {:?} vs {:?}", s.text().trim(), r.text().trim()));
        }
    }
    out
}

#[test]
fn items_rebuild_from_data() {
    let store = DataStore::discover().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let focus = std::env::var("ITEM_ORACLE_KIND").ok();
    let verbose = std::env::var("ITEM_ORACLE_VERBOSE").is_ok();

    // kind -> (exact, total, unsupported)
    let mut stats: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    // kind -> field -> count of mismatches
    let mut fields: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();
    for f in &files {
        let ch = Character::load(f).unwrap();
        let mut items_found = Vec::new();
        walk(&ch.doc, &mut items_found);
        for it in items_found {
            let tag = it.name.as_str();
            if focus.as_deref().is_some_and(|k| k != tag) {
                continue;
            }
            let st = stats.entry(tag.to_owned()).or_default();
            st.1 += 1;
            let Some(rebuilt) = items::rebuild(tag, &ch, &store, it) else {
                st.2 += 1;
                continue;
            };
            let d = diff(tag, it, &rebuilt);
            if d.is_empty() {
                st.0 += 1;
            } else {
                let fm = fields.entry(tag.to_owned()).or_default();
                for x in &d {
                    *fm.entry(x.split(':').next().unwrap_or("").to_owned()).or_default() += 1;
                }
                if verbose && samples.len() < 300 {
                    samples.push(format!("{} [{tag}] {:?}: {}", f.file_name().unwrap().to_string_lossy(), it.get("name"), d.join("; ")));
                }
            }
        }
    }
    eprintln!("item oracle (exact / total, unsupported):");
    for (k, (ok, n, un)) in &stats {
        let top: Vec<String> = fields.get(k).map(|m| {
            let mut v: Vec<_> = m.iter().collect();
            v.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            v.into_iter().take(6).map(|(f, c)| format!("{f}×{c}")).collect()
        }).unwrap_or_default();
        eprintln!("  {k:<14} {ok:>4} / {n:<4} unsupported {un:<4} {}", top.join(" "));
    }
    for s in &samples {
        eprintln!("{s}");
    }
    for (k, min) in BASELINES {
        let got = stats.get(*k).map_or(0, |s| s.0);
        assert!(got >= *min, "{k}: {got} exact matches, baseline {min}");
    }
}
