//! Saving a loaded character must not lose or change data.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::xml::{self, Element};

/// Strip fields our writer is expected to change.
fn normalize(mut e: Element) -> Element {
    e.remove_children("chummerrsversion");
    if let Some(attrs) = e.child_mut("attributes") {
        for a in attrs.elements_mut() {
            // legacy fields are converted to base/karma on load
            a.remove_children("value");
            a.remove_children("createkarma");
            a.remove_children("augmodifier");
            a.remove_children("base");
            a.remove_children("karma");
            a.remove_children("metatypecategory");
        }
    }
    e
}

#[test]
fn every_fixture_saves_losslessly() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "chum5") {
            continue;
        }
        let ch = Character::load(&path).unwrap();
        let saved = ch.to_xml_string();
        let again = Character::from_str(&saved).unwrap();
        // typed state survives
        assert_eq!(ch.attributes, again.attributes, "{}", path.display());
        assert_eq!(ch.skills, again.skills, "{}", path.display());
        assert_eq!(ch.knowledge_skills, again.knowledge_skills, "{}", path.display());
        assert_eq!(ch.improvements.list, again.improvements.list, "{}", path.display());
        assert_eq!(ch.karma, again.karma);
        assert_eq!(ch.doc.get("appversion"), again.doc.get("appversion"), "appversion must survive for Chummer5a");
        // and everything else in the document is untouched
        let original = xml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let a = normalize(original);
        let b = normalize(xml::parse(&saved).unwrap());
        let (ca, cb) = (a.elements().count(), b.elements().count());
        // (`chummerrsversion` was removed by normalize)
        assert_eq!(ca, cb, "{}: top-level element count", path.display());
        for (x, y) in a.elements().zip(b.elements()) {
            if x.name == "improvements" || x.name == "newskills" || x.name == "nuyen" {
                continue; // re-serialized from typed data
            }
            assert_eq!(x, y, "{}: <{}> changed", path.display(), x.name);
        }
        n += 1;
    }
    assert_eq!(n, 34);
}

#[test]
fn edits_are_written() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/BLUE.chum5");
    let mut ch = Character::load(&path).unwrap();
    ch.karma = 42;
    ch.set_field("alias", "GREEN");
    ch.attribute_mut("BOD").unwrap().karma = 2;
    ch.skills[0].karma = 3;
    let again = Character::from_str(&ch.to_xml_string()).unwrap();
    assert_eq!(again.karma, 42);
    assert_eq!(again.display_name(), "GREEN");
    assert_eq!(again.attribute("BOD").unwrap().karma, 2);
    assert_eq!(again.skills[0].karma, 3);
}
