use chummer_core::data::{self, DataStore};
use chummer_core::lang::Language;

#[test]
fn every_browsable_collection_has_records() {
    let store = DataStore::discover().expect("resources/data");
    for (label, file, container, item) in data::BROWSABLE {
        let doc = store.doc(file).unwrap();
        let recs = data::records(&doc, container, item);
        assert!(!recs.is_empty(), "{label}: no <{item}> in {file}/{container}");
    }
}

#[test]
fn known_records_resolve() {
    let store = DataStore::discover().unwrap();
    let skills = store.doc("skills.xml").unwrap();
    let pistols = data::find(&skills, "skills", "skill", "Pistols").unwrap();
    assert_eq!(pistols.get("attribute"), "AGI");
    assert_eq!(pistols.get("skillgroup"), "Firearms");
    let by_id = data::find(&skills, "skills", "skill", &pistols.id().to_uppercase()).unwrap();
    assert_eq!(by_id.name(), "Pistols");
    assert!(data::categories(&skills).contains(&"Combat Active".to_owned()));
}

#[test]
fn german_translations_load() {
    let dir = data::resource_dir("lang").unwrap();
    let de = Language::load(&dir, "de-de");
    assert_eq!(de.s("String_BP"), "GP");
    assert_eq!(de.data_name("skills.xml", "b52f7575-eebf-41c4-938d-df3397b5ee68", "Aeronautics Mechanic"), "Luftfahrtmechanik");
    assert_eq!(de.data_name("skills.xml", "", "Athletics"), "Athletik");
    // English fallback for keys missing from a translation
    assert!(!de.s("Message_InvalidTextFound_Title").is_empty());
    assert!(Language::available(&dir).iter().any(|(c, _)| c == "fr-fr"));
}
