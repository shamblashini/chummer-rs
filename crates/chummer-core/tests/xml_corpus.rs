use std::path::PathBuf;

use chummer_core::xml;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn roundtrip_dir(dir: PathBuf, ext: &str) -> usize {
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let tree = xml::parse(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let again = xml::parse(&tree.to_xml_string()).unwrap();
        assert!(tree == again, "{} does not round-trip", path.display());
        n += 1;
    }
    n
}

#[test]
fn all_data_files_roundtrip() {
    assert!(roundtrip_dir(repo_root().join("resources/data"), "xml") == 42);
    assert!(roundtrip_dir(repo_root().join("resources/lang"), "xml") >= 10);
}

#[test]
fn all_fixture_characters_roundtrip() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    assert_eq!(roundtrip_dir(dir, "chum5"), 34);
}
