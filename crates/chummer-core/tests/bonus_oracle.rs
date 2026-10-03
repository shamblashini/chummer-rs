//! Replay every `<bonus>` saved in the fixtures through the bonus
//! processor and compare with the improvements Chummer5a created for the
//! same item. The item's `<extra>` is the answer to any selection prompt.
//!
//! This prints coverage per bonus type and fails if exact matches drop
//! below the recorded baseline.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chummer_core::bonus::{self, BonusSource};
use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::improvement::Improvement;
use chummer_core::xml::Element;

/// Exact matches must not fall below this. Raise it as handlers land.
const BASELINE: usize = 627;

fn is_guid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}

/// The fields that must agree. GUID-valued fields are generated, so any
/// GUID matches any GUID.
fn key(i: &Improvement) -> String {
    let g = |s: &str| if is_guid(s) { "<guid>".to_owned() } else { s.to_owned() };
    format!(
        "{}|{}|{}|v{}|a{}|min{}|max{}|am{}|r{}|u{}|c{}|atr{}",
        i.kind,
        g(&i.improved_name),
        i.source,
        i.val,
        i.aug,
        i.min,
        i.max,
        i.aug_max,
        i.rating,
        g(&i.unique_name),
        i.condition,
        i.add_to_rating
    )
}

fn source_kind(tag: &str, e: &Element) -> &'static str {
    match tag {
        "quality" => "Quality",
        "power" => "Power",
        "cyberware" => {
            if e.get("improvementsource") == "Bioware" {
                "Bioware"
            } else {
                "Cyberware"
            }
        }
        "gear" => "Gear",
        "armor" => "Armor",
        "armormod" => "ArmorMod",
        "mentorspirit" => "MentorSpirit",
        "tradition" => "Tradition",
        "mod" => "VehicleMod",
        "metamagic" => "Metamagic",
        "critterpower" => "CritterPower",
        _ => "",
    }
}

fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
    for c in e.elements() {
        if c.child("bonus").is_some_and(|b| b.elements().next().is_some()) && !c.get("guid").is_empty() {
            out.push(c);
        }
        walk(c, out);
    }
}

/// `<bonus unique>` of the item's data record, looked up by name.
fn data_unique(store: &DataStore, it: &Element) -> String {
    let (file, container) = match it.name.as_str() {
        "gear" => ("gear.xml", "gears"),
        "armor" => ("armor.xml", "armors"),
        "armormod" => ("armor.xml", "mods"),
        "cyberware" if it.get("improvementsource") == "Bioware" => ("bioware.xml", "biowares"),
        "cyberware" => ("cyberware.xml", "cyberwares"),
        "quality" => ("qualities.xml", "qualities"),
        _ => return String::new(),
    };
    let Ok(doc) = store.doc(file) else { return String::new() };
    doc.child(container)
        .and_then(|c| c.elements().find(|e| e.get("name") == it.get("name")))
        .and_then(|e| e.child("bonus"))
        .and_then(|b| b.attr("unique").map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn bonus_processor_reproduces_saved_improvements() {
    let store = DataStore::discover().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();

    // per bonus node type: (exact, total)
    let mut per_type: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut exact, mut total) = (0, 0);
    let mut samples: Vec<String> = Vec::new();
    for f in &files {
        let ch = Character::load(f).unwrap();
        let mut items = Vec::new();
        walk(&ch.doc, &mut items);
        for it in items {
            let kind = source_kind(&it.name, it);
            if kind.is_empty() {
                continue;
            }
            let guid = it.get("guid").to_ascii_lowercase();
            let saved: Vec<&Improvement> = ch.improvements.list.iter().filter(|i| i.source_name.to_ascii_lowercase() == guid).collect();
            if saved.is_empty() {
                continue;
            }
            if saved.iter().any(|i| i.kind == "LimitModifier" && !is_guid(&i.improved_name)) {
                continue;
            }
            // Mentor spirits also apply the bonuses of the chosen
            // <choice1>/<choice2>, which are not in <bonus>.
            if it.name == "mentorspirit" {
                continue;
            }
            // Wireless, pair and first-level bonuses share the item guid;
            // only compare items whose improvements all come from <bonus>.
            if ["wirelessbonus", "pairbonus", "firstlevelbonus"].iter().any(|b| it.child(b).is_some_and(|x| x.elements().next().is_some())) {
                continue;
            }
            let bonus_el = it.child("bonus").unwrap();
            let src = BonusSource {
                kind: kind.into(),
                guid: it.get("guid"),
                name: it.get("name"),
                rating: it.get_i32("rating").filter(|r| *r > 0).unwrap_or(1),
            };
            let extra = it.get("extra");
            let forced = Some(extra.as_str());
            let out = bonus::apply(&ch, &store, bonus_el, &src, forced);
            // Version differences between the fixtures (5.18x-5.202) and
            // current Chummer5a, which this port follows:
            // - LimitModifier improvements used the item name as unique
            //   name; current code uses the bonus's unique name. Even older
            //   files used the limit name as improved name; those items
            //   are skipped.
            // - Saved bonuses drop the <bonus unique> attribute, so unique
            //   names that come from it cannot be reproduced; compared
            //   without it. (The GUI applies bonuses from the data, which
            //   keeps the attribute.)
            let item_name = it.get("name");
            let mut want: Vec<String> = saved
                .iter()
                .map(|i| {
                    let mut i = (*i).clone();
                    if i.kind == "LimitModifier" && i.unique_name == item_name {
                        i.unique_name.clear();
                    }
                    if bonus_el.attr("unique").is_none() && !i.unique_name.is_empty() && i.unique_name == data_unique(&store, it) {
                        i.unique_name.clear();
                    }
                    key(&i)
                })
                .collect();
            // Nested items (e.g. from addqualities) carry their own source.
            let mut got: Vec<String> = out.improvements.iter().filter(|i| i.source_name == src.guid).map(key).collect();
            want.sort();
            got.sort();
            let ok = want == got && out.unsupported.is_empty();
            total += 1;
            if ok {
                exact += 1;
            }
            for t in bonus_el.elements() {
                let e = per_type.entry(t.name.clone()).or_default();
                e.1 += 1;
                if ok {
                    e.0 += 1;
                }
            }
            if !ok && samples.len() < 400 {
                samples.push(format!(
                    "{} [{}] {:?} extra={extra:?}\n    want {want:?}\n    got  {got:?}{}",
                    f.file_name().unwrap().to_string_lossy(),
                    it.name,
                    it.get("name"),
                    if out.unsupported.is_empty() { String::new() } else { format!("\n    unsupported {:?}", out.unsupported) }
                ));
            }
        }
    }
    eprintln!("bonus oracle: {exact} of {total} items reproduce exactly");
    let mut rows: Vec<_> = per_type.iter().collect();
    rows.sort_by_key(|(_, (ok, n))| std::cmp::Reverse(n - ok));
    for (t, (ok, n)) in rows.iter().take(40) {
        eprintln!("  {t:<45} {ok:>4}/{n}");
    }
    if std::env::var("BONUS_ORACLE_VERBOSE").is_ok() {
        for s in &samples {
            eprintln!("{s}");
        }
    }
    assert!(exact >= BASELINE, "regression: {exact} exact matches, baseline {BASELINE}");
}
