//! The user folders ([`chummer_core::paths`]) with `XDG_DATA_HOME` and
//! `XDG_CONFIG_HOME` pointing at a scratch directory: the layout, custom
//! data, sheets, kits and rulesets found there. One test, as it changes
//! the process environment.

use std::path::PathBuf;

use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::gm::packs;
use chummer_core::paths::{self, UserDir};
use chummer_core::{custom_data, print, settings};

#[test]
fn user_folders_from_the_environment() {
    let t = std::env::temp_dir().join(format!("chummer-user-folders-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    // The only test in this binary: nothing else reads the environment.
    std::env::set_var("XDG_DATA_HOME", t.join("data"));
    std::env::set_var("XDG_CONFIG_HOME", t.join("config"));
    let data_root = t.join("data/chummer-rs");
    let config_root = t.join("config/chummer-rs");
    assert_eq!(paths::data_root(), Some(data_root.clone()));
    assert_eq!(paths::config_root(), Some(config_root.clone()));
    assert_eq!(paths::state_root(), Some(data_root.clone()));

    // The old `packs` folder becomes `kits`; the folders get READMEs.
    std::fs::create_dir_all(data_root.join("packs")).unwrap();
    std::fs::write(data_root.join("packs/custom_old_packs.xml"), "<chummer/>").unwrap();
    let log = paths::init();
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(data_root.join("kits/custom_old_packs.xml").is_file());
    assert!(data_root.join("migration.log").is_file());
    for d in UserDir::ALL {
        assert!(paths::user_dir(d).unwrap().is_dir(), "{d:?}");
    }
    assert!(data_root.join("customdata/README.txt").is_file());
    assert!(paths::init().is_empty(), "the second start moves nothing");

    // Every user path goes through the roots.
    assert_eq!(settings::user_settings_dir(), Some(config_root.join("settings")));
    assert_eq!(packs::packs_dir(), Some(data_root.join("kits")));
    assert_eq!(chummer_core::sources::SourcebookLibrary::config_path(), Some(config_root.join("sourcebooks.xml")));

    // Custom data in the user folder is found and merged.
    let mine = data_root.join("customdata/My House Rules");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(
        mine.join("custom_qualities.xml"),
        "<chummer><qualities><quality><id>5b0e6f1a-6c55-4c1e-9a52-7a3b8e3f0a11</id><name>User Folder Quality</name><karma>3</karma><category>Positive</category><source>SR5</source><page>1</page></quality></qualities></chummer>",
    )
    .unwrap();
    let roots = custom_data::default_roots();
    assert_eq!(roots.last(), Some(&data_root.join("customdata")));
    let engine = Engine::load().unwrap();
    let dir = engine.custom_data_directories().iter().find(|d| d.name == "My House Rules").expect("the user's folder is listed");
    let store = engine.store_for_dirs(vec![dir.path.clone()]);
    let doc = store.doc("qualities.xml").unwrap();
    assert!(data::find(&doc, "qualities", "quality", "User Folder Quality").is_some());
    assert!(data::find(&engine.store.doc("qualities.xml").unwrap(), "qualities", "quality", "User Folder Quality").is_none());

    // A user sheet is offered and found by name.
    let sheet: PathBuf = data_root.join("sheets/My Sheet.xsl");
    std::fs::write(&sheet, "<xsl:stylesheet/>").unwrap();
    assert!(print::available_sheets("en-us").iter().any(|(n, p)| n == "My Sheet" && *p == sheet));
    assert_eq!(print::find_sheet("en-us", "my sheet"), Some(sheet));

    let _ = std::fs::remove_dir_all(&t);
}
