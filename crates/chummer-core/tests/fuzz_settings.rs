//! Importing hostile settings files: odd names, odd content, odd file
//! names. Import fails with a message or installs a sanitised copy, never
//! panics, and never writes outside the user settings directory.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chummer_core::data::DataStore;
use chummer_core::settings::{self, ImportMode, SettingsLibrary};
use common::{no_panic, Prng};

/// Every file under `dir`, recursively.
fn listing(dir: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p.clone());
            }
            out.insert(p);
        }
    }
    out
}

fn settings_file(name: &str, extra: &str) -> String {
    format!("<?xml version=\"1.0\"?><settings><id>00000000-0000-0000-0000-000000000000</id><name>{name}</name><buildmethod>Priority</buildmethod>{extra}</settings>")
}

/// Plan and import `text` saved as `file_name` in a scratch "downloads"
/// folder under `root`, into `root/user`, with every mode.
fn try_import(store: &DataStore, root: &Path, file_name: &str, text: &[u8]) -> Result<Vec<String>, String> {
    let downloads = root.join("downloads");
    let user = root.join("user");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    let src = downloads.join(file_name);
    if std::fs::write(&src, text).is_err() {
        return Ok(vec!["cannot create source file".into()]);
    }
    let mut results = Vec::new();
    for mode in [ImportMode::New, ImportMode::KeepBoth, ImportMode::Overwrite, ImportMode::KeepBoth] {
        let lib = SettingsLibrary::load(store, Some(&user)).map_err(|e| e.to_string())?;
        let r = match lib.plan_import(&src, &user) {
            Ok(plan) => match settings::import(&plan, &lib, &user, &mode) {
                Ok(done) => {
                    if done.path.parent() != Some(user.as_path()) {
                        return Err(format!("installed outside the settings folder: {}", done.path.display()));
                    }
                    // The installed file loads as a preset.
                    let lib = SettingsLibrary::load(store, Some(&user)).map_err(|e| e.to_string())?;
                    if lib.find(&done.key).is_none() {
                        return Err(format!("installed {} but the library cannot find key {:?}", done.path.display(), done.key));
                    }
                    format!("{mode:?}: installed {}", done.key)
                }
                Err(e) => format!("{mode:?}: {e}"),
            },
            Err(e) => format!("plan: {e}"),
        };
        results.push(r);
    }
    let _ = std::fs::remove_file(&src);
    Ok(results)
}

#[test]
fn hostile_settings_files() {
    let store = DataStore::discover().expect("game data");
    let root = common::temp_dir("settings");
    let long = "n".repeat(250);
    let huge = "9".repeat(10_000);
    let cases: Vec<(String, Vec<u8>)> = vec![
        ("plain.xml".into(), settings_file("Plain", "").into_bytes()),
        ("traversal.xml".into(), settings_file("../../../evil", "").into_bytes()),
        ("slashes.xml".into(), settings_file("a/b\\c:d", "").into_bytes()),
        ("dots.xml".into(), settings_file("..", "").into_bytes()),
        ("empty-name.xml".into(), settings_file("", "").into_bytes()),
        ("space-name.xml".into(), settings_file("   ", "").into_bytes()),
        ("long-name.xml".into(), settings_file(&long, "").into_bytes()),
        ("nul-name.xml".into(), settings_file("a&#0;b", "").into_bytes()),
        ("huge-values.xml".into(), settings_file("Huge", &format!("<availability>{huge}</availability><essenceformat>#,0.{}</essenceformat><karmacost><karmaattribute>-2147483648</karmaattribute></karmacost><nuyenmaxbp>1e400</nuyenmaxbp>", "0".repeat(400))).into_bytes()),
        ("wrong-types.xml".into(), settings_file("Types", "<availability>lots</availability><books><book><x/></book></books><karmacost>5</karmacost>").into_bytes()),
        ("no-name.xml".into(), b"<settings><id>x</id></settings>".to_vec()),
        ("other-root.xml".into(), b"<character><name>x</name></character>".to_vec()),
        ("nested.xml".into(), b"<chummer><x><settings><name>Nested</name></settings></x></chummer>".to_vec()),
        ("not-xml.xml".into(), b"\x00\x01\x02 not xml".to_vec()),
        ("not-utf8.xml".into(), b"<settings><name>\xff\xfe</name></settings>".to_vec()),
        ("empty.xml".into(), Vec::new()),
        ("deep.xml".into(), format!("<settings><name>Deep</name>{}{}</settings>", "<a>".repeat(50_000), "</a>".repeat(50_000)).into_bytes()),
        ("no-extension".into(), settings_file("NoExt", "").into_bytes()),
        ("..xml".into(), settings_file("DotDot", "").into_bytes()),
        ("UPPER.XML".into(), settings_file("Upper", "").into_bytes()),
        (format!("{}.xml", "f".repeat(240)), settings_file("Long file", "").into_bytes()),
        ("Standard.xml".into(), settings_file("Standard", "").into_bytes()),
    ];
    let mut failures = Vec::new();
    for (file_name, text) in &cases {
        let before: BTreeSet<PathBuf> = listing(&root).into_iter().filter(|p| !p.starts_with(root.join("user")) && !p.starts_with(root.join("downloads"))).collect();
        match no_panic(|| try_import(&store, &root, file_name, text)) {
            Ok(Ok(r)) => eprintln!("{file_name}: {r:?}"),
            Ok(Err(e)) => failures.push(format!("{file_name}: {e}")),
            Err(p) => failures.push(format!("{file_name}: panic {p}")),
        }
        let after: BTreeSet<PathBuf> = listing(&root).into_iter().filter(|p| !p.starts_with(root.join("user")) && !p.starts_with(root.join("downloads"))).collect();
        if before != after {
            failures.push(format!("{file_name}: wrote outside the settings folder: {:?}", after.difference(&before).collect::<Vec<_>>()));
        }
    }
    // Every installed file is inside `user`, and loads back as a preset.
    let lib = SettingsLibrary::load(&store, Some(&root.join("user"))).unwrap();
    for p in lib.presets.iter().filter(|p| p.file.is_some()) {
        let _ = no_panic(|| (p.essence_decimals(), p.max_availability(), p.karma("karmaattribute", 5), p.books(), p.key()))
            .unwrap_or_else(|e| panic!("{}: {e}", p.name()));
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Mutated copies of a real exported preset.
#[test]
fn mutated_settings_files() {
    let store = DataStore::discover().expect("game data");
    let lib = SettingsLibrary::load(&store, None).unwrap();
    let text = settings::export_string(&lib.presets[0].raw);
    let mut failures = Vec::new();
    let mut rng = Prng::new(common::base_seed());
    for i in 0..common::iters(30) {
        let mut b = text.clone().into_bytes();
        let desc = if i % 2 == 0 {
            let at = rng.below(b.len());
            b.truncate(at);
            format!("truncated at {at}")
        } else {
            let at = rng.below(b.len());
            b[at] = rng.next_u64() as u8;
            format!("byte {at} = {:#04x}", b[at])
        };
        let s = String::from_utf8_lossy(&b);
        if let Err(p) = no_panic(|| settings::parse_settings_file(&s).map(|e| settings::export_string(&e))) {
            failures.push(format!("{desc}: {p}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
