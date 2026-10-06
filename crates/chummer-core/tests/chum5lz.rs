//! `.chum5lz` compressed saves. `tests/chum5lz/fixer-chummer.chum5lz` was
//! written by Chummer5a 5.225 itself (a `chummer-cli new` Elf, opened in
//! Chummer and saved with "Save As" as a compressed save).

use std::path::PathBuf;

use chummer_core::character::Character;
use chummer_core::chum5lz;

fn chummer_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/chum5lz/fixer-chummer.chum5lz")
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-rs-chum5lz-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The reference decoder: lzma-rs, a separate implementation.
fn lzma_rs_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(bytes), &mut out).unwrap();
    out
}

#[test]
fn decodes_a_chummer_made_file() {
    let bytes = std::fs::read(chummer_file()).unwrap();
    // Chummer's "Balanced" header: lc3 lp0 pb2, 16 MiB dictionary, unknown size.
    assert_eq!(&bytes[..13], &[0x5D, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let xml = chum5lz::decompress(&bytes).unwrap();
    assert_eq!(xml, lzma_rs_decode(&bytes));
    // Chummer compresses its XmlWriter output: BOM, declaration, CRLF.
    assert!(xml.starts_with(b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<character>"));

    let ch = Character::load(&chummer_file()).unwrap();
    assert_eq!(ch.name(), "Lz Fixer");
    assert_eq!(ch.field("metatype"), "Elf");
    assert_eq!(ch.field("appversion"), "5.225.1183");
}

#[test]
fn save_and_load_round_trip() {
    let dir = scratch("round");
    let mut ch = Character::load(&chummer_file()).unwrap();
    ch.set_field("alias", "Squeezed");
    let lz = dir.join("out.chum5lz");
    ch.save(&lz).unwrap();
    assert_eq!(ch.file.as_deref(), Some(lz.as_path()));
    let bytes = std::fs::read(&lz).unwrap();
    assert_eq!(&bytes[..13], &[0x5D, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    // Our encoder's stream decodes with an independent decoder.
    assert_eq!(lzma_rs_decode(&bytes), ch.to_xml_string().as_bytes());
    let again = Character::load(&lz).unwrap();
    assert_eq!(again.field("alias"), "Squeezed");
    assert_eq!(again.to_xml_string(), ch.to_xml_string());

    // The same character as plain .chum5 is the same XML, uncompressed.
    let plain = dir.join("out.chum5");
    ch.save(&plain).unwrap();
    assert_eq!(std::fs::read(&plain).unwrap(), chum5lz::decompress(&bytes).unwrap());
    assert!(bytes.len() * 4 < std::fs::metadata(&plain).unwrap().len() as usize);
    // No temporary files left behind.
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["out.chum5", "out.chum5lz"]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn roster_lists_compressed_saves() {
    let dir = scratch("roster");
    std::fs::copy(chummer_file(), dir.join("a.chum5lz")).unwrap();
    let v = chummer_core::roster::scan(&[dir.clone()]);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].display_name(), "Lz Fixer");
    assert!(v[0].error.is_none());
    std::fs::remove_dir_all(dir).ok();
}
