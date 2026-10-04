//! Sharing house rules: export a preset as a Chummer settings file, import
//! it on another machine, and detect characters whose preset is missing.

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::settings::{self, FileClash, ImportMode, SettingsLibrary, EMPTY_GUID, STANDARD_ID};
use chummer_core::xml;

fn store() -> DataStore {
    DataStore::discover().expect("resources/data")
}

/// A fresh empty directory under the system temp dir.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chummer-rs-{tag}-{}-{}", std::process::id(), chummer_core::items::new_guid()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn export_import_round_trip() {
    let store = store();
    let gm_dir = temp_dir("gm");
    let gm = SettingsLibrary::load(&store, Some(&gm_dir)).unwrap();
    // The GM makes house rules from Street Level and changes a value.
    let street = gm.presets.iter().find(|p| p.name() == "Street Level").unwrap();
    let path = settings::duplicate(street, "Seattle Nights", &gm_dir).unwrap();
    let mut raw = xml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    raw.child_or_insert("karmacost").set_child_text("karmaattribute", "4");
    std::fs::write(&path, raw.to_xml_string()).unwrap();
    let gm = SettingsLibrary::load(&store, Some(&gm_dir)).unwrap();
    let house = gm.find("Seattle_Nights.xml").expect("duplicate is installed");
    // Chummer5a keys a user file by name only when its id is the empty GUID.
    assert_eq!(house.id(), EMPTY_GUID);

    let shared = temp_dir("share").join("Seattle_Nights.xml");
    settings::export(house, &shared).unwrap();
    let text = std::fs::read_to_string(&shared).unwrap();
    assert!(text.starts_with("<?xml"));
    let root = xml::parse(&text).unwrap();
    assert_eq!(root.name, "settings");
    assert_eq!(root.get("id"), EMPTY_GUID);

    // A player imports it into an empty settings folder.
    let player_dir = temp_dir("player");
    let player = SettingsLibrary::load(&store, Some(&player_dir)).unwrap();
    assert_eq!(player.missing_preset("Seattle_Nights.xml").as_deref(), Some("Seattle_Nights"));
    let plan = player.plan_import(&shared, &player_dir).unwrap();
    assert_eq!(plan.file_name, "Seattle_Nights.xml");
    assert_eq!(plan.file_clash, None);
    assert!(!plan.name_clash);
    let installed = settings::import(&plan, &player, &player_dir, &ImportMode::New).unwrap();
    assert_eq!(installed, player_dir.join("Seattle_Nights.xml"));

    let player = SettingsLibrary::load(&store, Some(&player_dir)).unwrap();
    assert_eq!(player.missing_preset("Seattle_Nights.xml"), None);
    let got = player.find("Seattle_Nights.xml").unwrap();
    assert_eq!(got.name(), "Seattle Nights");
    assert_eq!(got.key(), "Seattle_Nights.xml");
    assert_eq!(got.karma("karmaattribute", 5), 4);
    assert_eq!(got.build_method(), street.build_method());
    assert_eq!(got.books(), street.books());
    assert_eq!(got.int("buildpoints", 0), street.int("buildpoints", 0));

    // Importing the same file again is a no-op.
    let again = player.plan_import(&shared, &player_dir).unwrap();
    assert_eq!(again.file_clash, Some(FileClash::Identical));
    assert!(!again.name_clash);
    settings::import(&again, &player, &player_dir, &ImportMode::New).unwrap();
    assert_eq!(std::fs::read_dir(&player_dir).unwrap().count(), 1);

    for d in [gm_dir, player_dir, shared.parent().unwrap().to_owned()] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn import_clash_keeps_both_or_overwrites() {
    let store = store();
    let dir = temp_dir("clash");
    let lib = SettingsLibrary::load(&store, Some(&dir)).unwrap();
    let standard = lib.find(STANDARD_ID).unwrap();
    settings::duplicate(standard, "House", &dir).unwrap();
    let lib = SettingsLibrary::load(&store, Some(&dir)).unwrap();

    // A different file with the same file name and display name.
    let src_dir = temp_dir("clash-src");
    let src = src_dir.join("House.xml");
    let mut raw = lib.find("House.xml").unwrap().raw.clone();
    raw.set_child_text("buildpoints", "30");
    std::fs::write(&src, raw.to_xml_string()).unwrap();

    let plan = lib.plan_import(&src, &dir).unwrap();
    assert_eq!(plan.file_clash, Some(FileClash::Different));
    assert!(!plan.name_clash, "the clashing name belongs to the file being replaced");
    assert!(settings::import(&plan, &lib, &dir, &ImportMode::New).is_err());

    let kept = settings::import(&plan, &lib, &dir, &ImportMode::KeepBoth).unwrap();
    assert_eq!(kept, dir.join("House_2.xml"));
    let lib = SettingsLibrary::load(&store, Some(&dir)).unwrap();
    assert_eq!(lib.find("House.xml").unwrap().int("buildpoints", 0), standard.int("buildpoints", 0));
    assert_eq!(lib.find("House_2.xml").unwrap().name(), "House (2)");
    assert_eq!(lib.find("House_2.xml").unwrap().int("buildpoints", 0), 30);

    let plan = lib.plan_import(&src, &dir).unwrap();
    settings::import(&plan, &lib, &dir, &ImportMode::Overwrite).unwrap();
    let lib = SettingsLibrary::load(&store, Some(&dir)).unwrap();
    assert_eq!(lib.find("House.xml").unwrap().int("buildpoints", 0), 30);
    assert_eq!(lib.find("House.xml").unwrap().name(), "House");

    // A built-in exported under a new file name: the name clashes, the id
    // is cleared so it does not shadow the built-in in Chummer5a.
    let exported = src_dir.join("MyStandard.xml");
    settings::export(standard, &exported).unwrap();
    let plan = lib.plan_import(&exported, &dir).unwrap();
    assert!(plan.name_clash);
    settings::import(&plan, &lib, &dir, &ImportMode::New).unwrap();
    let lib = SettingsLibrary::load(&store, Some(&dir)).unwrap();
    let mine = lib.find("MyStandard.xml").unwrap();
    assert_eq!(mine.name(), "Standard (2)");
    assert_eq!(mine.id(), EMPTY_GUID);
    assert_eq!(lib.find(STANDARD_ID).unwrap().file, None);

    for d in [dir, src_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn import_rejects_non_settings_files() {
    let lib = SettingsLibrary::load(&store(), None).unwrap();
    let dir = temp_dir("bad");
    let f = dir.join("char.xml");
    std::fs::write(&f, "<character><name>x</name></character>").unwrap();
    assert!(lib.plan_import(&f, &dir).is_err());
    std::fs::write(&f, "<settings><name>x</name").unwrap();
    assert!(lib.plan_import(&f, &dir).is_err());
    // Chummer loads `.//settings`, so a wrapped preset is accepted.
    std::fs::write(&f, "<chummer><settings><name>Wrapped</name></settings></chummer>").unwrap();
    assert_eq!(lib.plan_import(&f, &dir).unwrap().name(), "Wrapped");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_preset_is_detected() {
    let lib = SettingsLibrary::load(&store(), None).unwrap();
    // Built-ins by GUID and by name are installed.
    assert_eq!(lib.missing_preset(STANDARD_ID), None);
    assert_eq!(lib.missing_preset("Street Level"), None);
    // No <settings> at all means Chummer's default, not a missing preset.
    assert_eq!(lib.missing_preset(""), None);
    // Old characters name `default.xml`, which current Chummer does not ship.
    assert_eq!(lib.missing_preset("default.xml").as_deref(), Some("default"));
    assert_eq!(lib.missing_preset("Some GM's rules.xml").as_deref(), Some("Some GM's rules"));
    assert!(lib.find("default.xml").is_none());
    // Budgets then use Standard.
    assert_eq!(lib.resolve("default.xml").unwrap().id(), STANDARD_ID);

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let f = std::fs::read_dir(fixtures).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "chum5")).unwrap();
    let ch = Character::load(&f).unwrap();
    assert_eq!(lib.missing_preset(&ch.field("settings")).as_deref(), Some("default"));
}

#[test]
fn switching_preset_keeps_build_method_in_creation() {
    let lib = SettingsLibrary::load(&store(), None).unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixtures).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let mut ch = files.iter().map(|f| Character::load(f).unwrap()).find(|c| !c.created && c.field("buildmethod") == "Priority").unwrap();
    let karma = lib.presets.iter().find(|p| p.file.is_none() && p.build_method() == "Karma").unwrap();
    assert!(settings::switch_character(&mut ch, karma).is_err());
    assert_eq!(ch.field("settings"), "default.xml", "a refused switch leaves <settings> alone");
    let street = lib.find("Street Level").unwrap();
    settings::switch_character(&mut ch, street).unwrap();
    assert_eq!(ch.field("settings"), street.id());
    assert_eq!(lib.missing_preset(&ch.field("settings")), None);
    assert!(ch.dirty);
}
