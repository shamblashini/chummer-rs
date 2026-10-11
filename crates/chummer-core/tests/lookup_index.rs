//! The item index (`edit::with_index`) changes how items are found, not
//! what is found: every lookup an item table or the Play page makes
//! answers the same inside the index as without it, for every item of
//! every fixture.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::items::{edit, place};
use chummer_core::play::ammo;
use chummer_core::xml::Element;

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    v.sort();
    v
}

fn guids(e: &Element, out: &mut Vec<String>) {
    if edit::is_item(e) {
        out.push(e.get("guid"));
    }
    for c in e.elements() {
        guids(c, out);
    }
}

/// Everything asked of one item, as text.
fn answers(ch: &Character, store: &DataStore, guid: &str) -> String {
    let rules = chummer_core::calc::Rules::default();
    let mut s = format!(
        "{:?} {:?} {:?} {} {} {:?} {:?} {} {:?} {:?}",
        edit::find(ch, guid).map(|e| e.name.clone()),
        edit::parent(ch, guid).map(|p| p.get("guid")),
        edit::rating_range(ch, store, guid),
        edit::total_cost(ch, store, guid),
        edit::availability(ch, store, guid),
        edit::essence(ch, store, &rules, guid),
        edit::capacity(ch, guid),
        edit::is_included(ch, guid),
        place::root_container(ch, guid),
        place::check(ch, store, place::Candidate::Owned(guid), &place::Dest::Top, false).err(),
    );
    if let Some(w) = edit::find(ch, guid).filter(|e| e.name == "weapon") {
        s += &format!(" {} {:?} {:?}", ammo::capacity(ch, w), ammo::loaded(ch, w).map(|g| g.get("guid")), ammo::reloadable(ch, Some(store), guid));
    }
    s
}

#[test]
fn lookups_answer_the_same_inside_the_index() {
    let store = DataStore::discover().expect("resources/data");
    let mut items = 0;
    for path in fixtures() {
        let ch = Character::load(&path).unwrap();
        let mut all = Vec::new();
        guids(&ch.doc, &mut all);
        let plain: Vec<String> = all.iter().map(|g| answers(&ch, &store, g)).collect();
        let indexed: Vec<String> = edit::with_index(&ch, || all.iter().map(|g| answers(&ch, &store, g)).collect());
        for ((g, a), b) in all.iter().zip(&plain).zip(&indexed) {
            assert_eq!(a, b, "{}: item {g}", path.display());
        }
        items += all.len();
    }
    assert!(items > 1000, "only {items} items in the fixtures");
}
