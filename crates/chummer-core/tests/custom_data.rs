//! Custom data directories from `resources/customdata` applied to the
//! bundled game data.

use std::path::PathBuf;
use std::sync::Arc;

use chummer_core::custom_data::{self, CustomDataDirectory, MergeReport};
use chummer_core::data::{self, DataStore};
use chummer_core::engine::Engine;
use chummer_core::xml::{self, Element};

fn base() -> DataStore {
    DataStore::discover().expect("resources/data")
}

fn directories() -> Vec<CustomDataDirectory> {
    let root = data::resource_dir("customdata").expect("resources/customdata");
    custom_data::discover(&root)
}

fn directory(name: &str) -> PathBuf {
    directories().into_iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no directory {name}")).path
}

/// `file` with one directory applied.
fn merged(dir: &str, file: &str) -> Arc<Element> {
    let store = base().with_enabled_custom_data(vec![directory(dir)]);
    let doc = store.doc(file).unwrap();
    assert!(store.custom_data_warnings().is_empty(), "{:?}", store.custom_data_warnings());
    doc
}

/// Files that legitimately change nothing in the bundled data.
const NO_OPS: &[&str] = &[
    // Only a comment.
    "Neon Anarchy House Rules/amend_drugcomponents.xml",
    // Its one range ("Carbines") already exists, so `custom_` skips it.
    "Neon Anarchy House Rules/custom_ranges.xml",
    // No mentor choice grants Automatics in the current data.
    "Remove Automatics as a Skill/amend_mentors.xml",
];

#[test]
fn every_directory_applies_cleanly() {
    let store = base();
    let dirs = directories();
    assert!(dirs.len() >= 50, "found {} directories", dirs.len());
    let mut problems = Vec::new();
    for dir in &dirs {
        if let Some(e) = &dir.manifest_error {
            problems.push(format!("{}: manifest: {e}", dir.name));
        }
        let files = dir.affected_files();
        assert!(!files.is_empty(), "{}: no data files", dir.name);
        let mut total = 0;
        for file in files {
            let mut doc = (*store.base_doc(&file).unwrap()).clone();
            let report: MergeReport = custom_data::apply(&mut doc, &file, std::slice::from_ref(&dir.path));
            problems.extend(report.warnings.iter().cloned());
            assert!(!report.files.is_empty(), "{}: nothing applied to {file}", dir.name);
            for (f, n) in report.files.iter().zip(&report.mutations) {
                let short = format!("{}/{}", dir.name, f.file_name().unwrap().to_string_lossy());
                if *n == 0 && !NO_OPS.contains(&short.as_str()) {
                    problems.push(format!("{short}: no effect"));
                }
                total += n;
            }
            // The merged document still serializes and parses.
            let again = xml::parse(&doc.to_xml_string()).unwrap();
            assert_eq!(again.name, "chummer");
        }
        assert!(total > 0, "{}: no changes", dir.name);
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn manifests_are_read() {
    let dirs = directories();
    let bone = dirs.iter().find(|d| d.name == "Bone Lacing Adds to Body").unwrap();
    assert_eq!(bone.guid(), Some("091a9694-4186-4c2d-96fc-d4dc2ae62505"));
    assert_eq!(bone.save_key(), "091a9694-4186-4c2d-96fc-d4dc2ae62505>1.0");
    assert!(bone.manifest.as_ref().unwrap().description("en-us").unwrap().contains("Bone Lacing"));
    // `Manifest.xml` with a capital M is still found.
    let critter = dirs.iter().find(|d| d.name == "Critter Prices").unwrap();
    assert!(critter.manifest.is_some());
    assert!(dirs.iter().all(|d| d.manifest_error.is_none()));
}

#[test]
fn bone_lacing_adds_to_body() {
    let base_doc = base().doc("cyberware.xml").unwrap();
    let id = "76523da7-271d-459a-9502-3f7c8d54be6a";
    let before = data::find(&base_doc, "cyberwares", "cyberware", id).unwrap();
    assert!(before.el().path("bonus/damageresistance").is_some());

    let doc = merged("Bone Lacing Adds to Body", "cyberware.xml");
    let lacing = data::find(&doc, "cyberwares", "cyberware", id).unwrap();
    assert_eq!(lacing.name(), "Bone Lacing (Aluminum)");
    let bonus = lacing.el().child("bonus").unwrap();
    assert!(bonus.child("damageresistance").is_none(), "remove dropped damageresistance");
    let attr = bonus.child("specificattribute").unwrap();
    assert_eq!((attr.get("name").as_str(), attr.get("val").as_str()), ("BOD", "2"));
}

#[test]
fn remove_automatics_removes_the_skill() {
    assert!(data::find(&base().doc("skills.xml").unwrap(), "skills", "skill", "Automatics").is_some());
    let doc = merged("Remove Automatics as a Skill", "skills.xml");
    assert!(data::find(&doc, "skills", "skill", "Automatics").is_none());
    assert!(data::find(&doc, "skills", "skill", "Longarms").is_some());
}

#[test]
fn custom_files_add_records() {
    let base_doc = base().doc("qualities.xml").unwrap();
    let before = data::records(&base_doc, "qualities", "quality").len();
    let doc = merged("Delnar's One Karma Qualities", "qualities.xml");
    let after = data::records(&doc, "qualities", "quality");
    assert!(after.len() > before, "{} -> {}", before, after.len());
    // Every added record is findable by its id.
    for rec in &after[before..] {
        assert!(data::find(&doc, "qualities", "quality", &rec.id()).is_some(), "{}", rec.name());
    }
}

#[test]
fn neotokyo_regexreplace_rewrites_avail() {
    let base_doc = base().doc("weapons.xml").unwrap();
    let doc = merged("Shadowrun Missions Neotokyo Rules", "weapons.xml");
    // A light pistol with physical damage: `<n>[FR]` becomes `2+<n>F`.
    let before = data::find(&base_doc, "weapons", "weapon", "Ares Light Fire 70").unwrap();
    let after = data::find(&doc, "weapons", "weapon", "Ares Light Fire 70").unwrap();
    let n: String = before.get("avail").chars().filter(char::is_ascii_digit).collect();
    assert_eq!(after.get("avail"), format!("2+{n}F"), "was {}", before.get("avail"));
}

#[test]
fn directories_apply_in_order_and_cache() {
    let store = base().with_enabled_custom_data(vec![directory("Bone Lacing Adds to Body"), directory("Remove Automatics as a Skill")]);
    let a = store.doc("cyberware.xml").unwrap();
    let b = store.doc("cyberware.xml").unwrap();
    assert!(Arc::ptr_eq(&a, &b), "merged documents are cached");
    // Files no directory touches come straight from the base cache.
    let books = store.doc("books.xml").unwrap();
    assert!(Arc::ptr_eq(&books, &store.base_doc("books.xml").unwrap()));
    assert!(data::find(&store.doc("skills.xml").unwrap(), "skills", "skill", "Automatics").is_none());
}

#[test]
fn settings_presets_select_their_custom_data() {
    let engine = Engine::load().unwrap();
    let full_house = engine.settings.presets.iter().find(|p| p.name() == "Full House").unwrap();
    let names: Vec<&str> = engine.enabled_custom_data(full_house).iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Chrome Flesh Stealth Errata"), "{names:?}");
    assert!(!names.contains(&"German Data Changes"), "{names:?}");

    let standard = engine.settings.resolve("Standard").unwrap();
    assert!(engine.enabled_custom_data(standard).is_empty());
    assert!(engine.store_for(standard).custom_data_dirs().is_empty());

    let store = engine.store_for(full_house);
    assert!(Arc::ptr_eq(&store, &engine.store_for(full_house)), "one store per enabled set");
    let errata = directory("Chrome Flesh Stealth Errata");
    assert!(store.custom_data_dirs().contains(&errata));
    // Something Chrome Flesh errata changes differs from the base data.
    let dir = CustomDataDirectory::load(&errata);
    let changed = dir.affected_files().into_iter().any(|f| *store.doc(&f).unwrap() != *engine.store.doc(&f).unwrap());
    assert!(changed);
    assert!(store.custom_data_warnings().is_empty(), "{:?}", store.custom_data_warnings());

    let sum10 = engine.settings.presets.iter().find(|p| p.name() == "Sum-to-Ten Improved (German)").unwrap();
    let names: Vec<String> = engine.enabled_custom_data(sum10).iter().map(|d| d.name.clone()).collect();
    assert_eq!(names, ["German Data Changes", "Sum-to-Ten Improved"]);
    assert!(custom_data::check_dependencies(&engine.enabled_custom_data(sum10)).is_empty());
}

#[test]
fn save_keys_resolve_by_guid() {
    let dirs = directories();
    let settings = xml::parse(
        "<setting><customdatadirectorynames>\
           <customdatadirectoryname><directoryname>091a9694-4186-4c2d-96fc-d4dc2ae62505&gt;1.0</directoryname><order>1</order><enabled>True</enabled></customdatadirectoryname>\
           <customdatadirectoryname><directoryname>remove automatics as a skill</directoryname><order>0</order><enabled>True</enabled></customdatadirectoryname>\
           <customdatadirectoryname><directoryname>Critter Prices</directoryname><order>2</order><enabled>False</enabled></customdatadirectoryname>\
           <customdatadirectoryname><directoryname>No Such Directory</directoryname><order>3</order><enabled>True</enabled></customdatadirectoryname>\
         </customdatadirectorynames></setting>",
    )
    .unwrap();
    let names: Vec<&str> = custom_data::enabled_directories(&settings, &dirs).iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Remove Automatics as a Skill", "Bone Lacing Adds to Body"]);
}
