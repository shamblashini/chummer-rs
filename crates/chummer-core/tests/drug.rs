//! Drugs: custom drugs from components, ready-made records, effects and
//! the improvements `Drug.GenerateImprovement` makes.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::items::{self, drug, Purchase};

fn harmony() -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Harmony.chum5")).unwrap()
}

#[test]
fn custom_drug_from_components() {
    let store = DataStore::discover().unwrap();
    let d = drug::custom_drug(&store, "Pep", "Street Cooked", &[("Charmer", 0)], "g1").unwrap();
    assert_eq!(d.get("category"), "Custom Drug");
    assert_eq!(d.get("grade"), "Street Cooked");
    assert_eq!(d.get("sourceid"), "00000000-0000-0000-0000-000000000000");
    let c = d.path("drugcomponents/drugcomponent").unwrap();
    assert_eq!(c.get("name"), "Charmer");
    assert_eq!(c.get("level"), "0");
    assert_eq!(c.get("rating"), "6");
    assert_eq!(c.get("threshold"), "2");
    let fx = drug::effects(&d);
    assert_eq!(fx.attributes, vec![("CHA".to_owned(), 1.0), ("AGI".to_owned(), -1.0)]);
    assert_eq!(fx.limits, vec![("Social".to_owned(), 1)]);
    assert_eq!(drug::cost(&d), 75.0);
    let (imps, quals) = drug::generate_improvements(&d);
    assert!(quals.is_empty());
    assert_eq!(imps.len(), 3);
    let cha = imps.iter().find(|i| i.improved_name == "CHA").unwrap();
    assert_eq!((cha.kind.as_str(), cha.aug, cha.source.as_str(), cha.source_name.as_str()), ("Attribute", 1.0, "Drug", "g1"));
    assert!(cha.custom && !cha.enabled);
    assert_eq!(cha.custom_group, "Pep");
    assert_eq!(cha.custom_name, "Pep - CHA +1");
    assert!(imps.iter().any(|i| i.kind == "SocialLimit" && i.val == 1.0));
}

#[test]
fn custom_drug_needs_one_foundation() {
    let store = DataStore::discover().unwrap();
    assert!(drug::custom_drug(&store, "x", "Standard", &[], "g").is_err());
    assert!(drug::custom_drug(&store, "x", "Standard", &[("Charmer", 0), ("Tank", 0)], "g").is_err());
    assert!(drug::custom_drug(&store, "x", "Standard", &[("Charmer", 7)], "g").is_err());
}

#[test]
fn ready_made_drug_record() {
    let store = DataStore::discover().unwrap();
    let mut ch = harmony();
    let kind = items::kind("drug").unwrap();
    let doc = store.doc(kind.file).unwrap();
    let rec = data::find(&doc, kind.data_container, kind.data_item, "Cram").unwrap();
    let g = items::add("drug", &mut ch, &store, rec, &Purchase { qty: 3.0, ..Default::default() }).unwrap();
    let d = ch.items("drugs", "drug").into_iter().find(|d| d.get("guid") == g).unwrap().clone();
    assert_eq!(d.get("quantity"), "3");
    assert_eq!(d.get("availability"), "2R");
    let fx = drug::effects(&d);
    assert_eq!(fx.attributes, vec![("REA".to_owned(), 1.0)]);
    assert_eq!(fx.initiative_dice, 1);
    assert_eq!(drug::cost(&d), 10.0);
    let imps: Vec<_> = ch.improvements.list.iter().filter(|i| i.source_name == g).collect();
    assert_eq!(imps.len(), 2);
    assert!(imps.iter().all(|i| !i.enabled && i.source == "Drug"));
}
