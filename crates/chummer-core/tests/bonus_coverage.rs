//! Every bonus node type in the game data must be either handled by the
//! bonus processor or on the documented ignore list below.
//!
//! Walks each `<bonus>`, `<wirelessbonus>`, `<firstlevelbonus>` and
//! `<pairbonus>` in `resources/data/*.xml`, wraps each child alone in a
//! `<bonus>` and applies it to a fixture character. Selections are
//! answered with the first option `bonus::choices` offers (or a dummy
//! text for free-text prompts). An occurrence is handled when `apply`
//! does not report its node type unsupported on at least one of the
//! fixtures. Since that needs an answer for every selecting node, it
//! also checks that each one has a `choice_for` entry.
//!
//! Prints the unhandled node names with their counts and fails if there
//! are any.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chummer_core::bonus::{self, BonusSource};
use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::xml::{self, Element};

/// Node names the bonus processor knowingly does not turn into
/// improvements, with the reason. `true`: `apply` accepts them silently
/// (Chummer5a skips them too); `false`: `apply` reports them unsupported,
/// as Chummer5a has no method for them and aborts the bonus.
const IGNORE: &[(&str, bool, &str)] = &[
    // Vehicle mod stats. VehicleMod.Create runs its bonus with
    // blnAddImprovementsToCharacter = false, which ignores unknown
    // methods; Vehicle reads these nodes itself.
    ("handling", true, "vehicle mod stat, read by Vehicle"),
    ("offroadhandling", true, "vehicle mod stat, read by Vehicle"),
    ("accel", true, "vehicle mod stat, read by Vehicle"),
    ("offroadaccel", true, "vehicle mod stat, read by Vehicle"),
    ("speed", true, "vehicle mod stat, read by Vehicle"),
    ("offroadspeed", true, "vehicle mod stat, read by Vehicle"),
    ("seats", true, "vehicle mod stat, read by Vehicle"),
    ("sensor", true, "vehicle mod stat, read by Vehicle"),
    ("pilot", true, "vehicle mod stat, read by Vehicle"),
    ("body", true, "vehicle mod stat, read by Vehicle"),
    ("devicerating", true, "vehicle mod stat, read by Vehicle"),
    // Drug component effects, read by Drug (items/drug.rs), never by the
    // improvement manager.
    ("attribute", true, "drug effect, read by Drug"),
    ("limit", true, "drug effect, read by Drug"),
    ("quality", true, "drug effect, read by Drug"),
    // Data errors upstream: no AddImprovementCollection method exists, so
    // Chummer5a logs "Tried to get unknown bonus" and rolls the bonus back.
    ("astralreputation", false, "no Chummer5a method (Astral Beacon data bug)"),
    ("defensetest", false, "no Chummer5a method (data bug)"),
    ("addquality", false, "no Chummer5a method; belongs inside <addqualities> (traditions.xml data bug)"),
];

/// Occurrences that fail because the data itself is wrong: (node, text
/// in the node, reason). Chummer5a fails on them too.
const DATA_ERRORS: &[(&str, &str, &str)] = &[
    ("critterpowers", "Reduced Sense", "power missing from critterpowers.xml"),
    ("addspirit", "Corpse Cadavre", "spirit missing from traditions.xml and critters.xml"),
    // lifemodules.xml names qualities that qualities.xml does not have.
    ("addqualities", "SINner: National", "quality missing from qualities.xml"),
    ("addqualities", "Home Ground (You Know A Guy)", "quality missing from qualities.xml"),
    ("addqualities", "Phobia (Mild, Common)", "quality missing from qualities.xml"),
    ("addqualities", "Corporate Pariah", "quality missing from qualities.xml"),
    ("addqualities", "Silence Is Golden", "quality missing from qualities.xml"),
    ("addqualities", "Too Pretty to Hit", "quality missing from qualities.xml"),
    ("addqualities", "<options>", "<addquality><options> is not a Chummer5a form"),
];

/// Fixtures tried in order; an occurrence passes on the first that works.
const FIXTURES: &[&str] = &["Pañcama.chum5", "Ushi Resub.chum5", "prime.chum5", "Munin.chum5", "Barrett.chum5"];

const BONUS_TAGS: &[&str] = &["bonus", "wirelessbonus", "firstlevelbonus", "pairbonus"];

/// Apply `node` alone, then (for nodes that read what an earlier sibling
/// selected) within its whole bonus; true when `apply` does not report
/// the node's type unsupported.
fn works(ch: &Character, store: &DataStore, parent: &Element, node: &Element) -> bool {
    let mut b = Element::new("bonus");
    if let Some(u) = parent.attr("unique") {
        b.set_attr("unique", u);
    }
    b.push(node.clone());
    try_apply(ch, store, &b, node) || try_apply(ch, store, parent, node)
}

fn try_apply(ch: &Character, store: &DataStore, b: &Element, node: &Element) -> bool {
    let src = BonusSource { kind: "Quality".into(), guid: "00000000-0000-0000-0000-000000000001".into(), name: "Coverage".into(), rating: 1 };
    let answer = bonus::choices(ch, store, b, &src).first().map(|c| c.options.first().cloned().unwrap_or_else(|| "Test".into()));
    let out = bonus::apply(ch, store, b, &src, answer.as_deref());
    if !out.unsupported.is_empty() && std::env::var("BONUS_COVERAGE_VERBOSE").is_ok() {
        eprintln!("    answer {answer:?} -> unsupported {:?}", out.unsupported);
    }
    // Nested objects report their own unanswered prompts by their own
    // names; only this node's type counts here.
    !out.unsupported.contains(&node.name)
}

#[test]
fn every_bonus_node_is_handled_or_ignored() {
    let store = DataStore::discover().unwrap();
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let chars: Vec<Character> = FIXTURES.iter().map(|f| Character::load(&fx.join(f)).unwrap()).collect();

    let mut files: Vec<PathBuf> = std::fs::read_dir(store.data_dir()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "xml")).collect();
    files.sort();

    // name -> (occurrences, failing occurrences, sample file)
    let mut seen: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
    for f in &files {
        let doc = xml::parse(&std::fs::read_to_string(f).unwrap()).unwrap();
        let fname = f.file_name().unwrap().to_string_lossy().into_owned();
        let mut nodes: Vec<(&Element, &Element)> = Vec::new();
        for tag in BONUS_TAGS {
            let mut ps = Vec::new();
            doc.descendants(tag, &mut ps);
            for p in ps {
                nodes.extend(p.elements().map(|n| (p, n)));
            }
        }
        for (parent, node) in nodes {
            let e = seen.entry(node.name.clone()).or_insert((0, 0, fname.clone()));
            e.0 += 1;
            let xml = node.to_xml_string();
            if DATA_ERRORS.iter().any(|(n, needle, _)| *n == node.name && xml.contains(needle)) {
                continue;
            }
            if !chars.iter().any(|ch| works(ch, &store, parent, node)) {
                e.1 += 1;
                e.2.clone_from(&fname);
                if std::env::var("BONUS_COVERAGE_VERBOSE").is_ok() {
                    eprintln!("{fname}: {}", xml.chars().take(300).collect::<String>());
                }
            }
        }
    }

    let total_names = seen.len();
    let total_occ: usize = seen.values().map(|v| v.0).sum();
    let mut unhandled = Vec::new();
    let mut ignored = Vec::new();
    let mut wrong = Vec::new();
    for (name, (n, bad, file)) in &seen {
        match IGNORE.iter().find(|(i, _, _)| i == name) {
            // The ignore list must describe what apply does.
            Some((_, accepted, _)) if (*bad == 0) != *accepted => wrong.push(format!("{name}: {bad}/{n} unsupported, ignore entry says accepted={accepted}")),
            Some((_, _, why)) => ignored.push((name.clone(), *n, *why)),
            None if *bad > 0 => unhandled.push((name.clone(), *n, *bad, file.clone())),
            None => {}
        }
    }
    for (i, _, _) in IGNORE {
        assert!(seen.contains_key(*i), "ignore entry {i} does not occur in the data");
    }
    let ign_occ: usize = ignored.iter().map(|x| x.1).sum();
    let un_occ: usize = unhandled.iter().map(|x| x.1).sum();
    eprintln!("bonus coverage: {total_names} node names, {total_occ} occurrences");
    eprintln!("  handled: {} names, {} occurrences", total_names - ignored.len() - unhandled.len(), total_occ - ign_occ - un_occ);
    eprintln!("  ignored: {} names, {} occurrences", ignored.len(), ign_occ);
    for (n, c, why) in &ignored {
        eprintln!("    {n:<28} {c:>4}  {why}");
    }
    eprintln!("  unhandled: {} names, {} occurrences", unhandled.len(), un_occ);
    for (n, c, bad, file) in &unhandled {
        eprintln!("    {n:<28} {bad:>4}/{c:<4} e.g. {file}");
    }
    for w in &wrong {
        eprintln!("  wrong ignore entry: {w}");
    }
    assert!(wrong.is_empty(), "{} ignore entries disagree with apply", wrong.len());
    assert!(unhandled.is_empty(), "{} bonus node names are neither handled nor ignored", unhandled.len());
}
