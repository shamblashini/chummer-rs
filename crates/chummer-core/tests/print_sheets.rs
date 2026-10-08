//! Character sheets: every fixture renders through Chummer's XSLT sheets
//! with `xsltproc`, without errors, and the HTML shows the character.

use std::path::{Path, PathBuf};

use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::print;

/// The default sheet plus the most used others.
const SHEETS: &[&str] = &[
    print::DEFAULT_SHEET,
    "Shadowrun 5",
    "Shadowrun 5 (Skills grouped by Rating)",
    "Fancy Blocks",
    "Game Master Summary",
];

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "chum5")).collect();
    v.sort();
    v
}

fn have_xsltproc() -> bool {
    print::xsltproc_path().is_some_and(|p| std::process::Command::new(p).arg("--version").output().is_ok())
}

/// Decode the entities libxslt's HTML output uses for non-ASCII text.
fn decode(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e < 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ntilde" => Some('ñ'),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn sheet_list_has_default() {
    let sheets = print::available_sheets("en-us");
    assert!(sheets.len() >= 20, "{sheets:?}");
    for name in SHEETS {
        let p = print::find_sheet("en-us", name).unwrap_or_else(|| panic!("{name} missing"));
        assert!(p.is_file(), "{}", p.display());
    }
    let de = print::available_sheets("de-de");
    assert!(de.iter().all(|(_, p)| p.parent().unwrap().ends_with("de-de")));
}

#[test]
fn every_fixture_renders() {
    if !have_xsltproc() {
        eprintln!("xsltproc not installed; skipping");
        return;
    }
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let dir = std::env::temp_dir().join(format!("chummer-rs-sheets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for f in fixtures() {
        let ch = Character::load(&f).unwrap();
        let xml = print::print_xml(&ch, &engine, &lang);
        let sheet = engine.sheet(&ch);
        let name = ch.display_name();
        let bod = sheet.attr("BOD").to_string();
        let skill = sheet.skills.iter().find(|s| s.rating > 0 && !s.disabled).map(|s| s.name.clone());
        for s in SHEETS {
            let xsl = print::find_sheet("en-us", s).unwrap();
            let out = dir.join("out.html");
            let label = format!("{} / {s}", f.file_name().unwrap().to_string_lossy());
            match print::render_report(&xml, &xsl, &out) {
                Err(e) => failures.push(format!("{label}: {e}")),
                Ok(r) if !r.warnings.trim().is_empty() => failures.push(format!("{label}: {}", r.warnings.trim())),
                Ok(_) => {
                    let html = decode(&std::fs::read_to_string(&out).unwrap());
                    if !name.trim().is_empty() && !html.contains(name.trim()) {
                        failures.push(format!("{label}: name {name:?} missing"));
                    }
                    if !html.contains(&format!(">{bod}<")) && !html.contains(&format!(" {bod}")) && !html.contains(&format!(">{bod} ")) {
                        failures.push(format!("{label}: BOD {bod} missing"));
                    }
                    if let Some(sk) = &skill {
                        if !html.contains(sk.as_str()) {
                            failures.push(format!("{label}: skill {sk:?} missing"));
                        }
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn translated_sheet_renders() {
    if !have_xsltproc() {
        return;
    }
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "de-de");
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Soma (Career).chum5");
    let ch = Character::load(&f).unwrap();
    let xml = print::print_xml(&ch, &engine, &lang);
    let xsl = print::find_sheet("de-de", "Shadowrun 5").unwrap();
    let out = std::env::temp_dir().join(format!("chummer-rs-de-{}.html", std::process::id()));
    let r = print::render_report(&xml, &xsl, &out).unwrap();
    assert!(r.warnings.trim().is_empty(), "{}", r.warnings);
    let html = decode(&std::fs::read_to_string(&out).unwrap());
    std::fs::remove_file(&out).ok();
    assert!(html.contains("Soma"));
    assert!(html.contains("Konstitution") || html.contains("KON"), "German labels missing");
}

#[test]
fn print_xml_shape() {
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Soma (Career).chum5");
    let ch = Character::load(&f).unwrap();
    let root = print::print_xml(&ch, &engine, &lang);
    assert_eq!(root.name, "characters");
    let c = root.child("character").unwrap();
    assert_eq!(c.get("name"), "Unnamed Character");
    assert_eq!(c.get("alias"), "Soma");
    let sheet = engine.sheet(&ch);
    let bod = c.children_named("attributes").flat_map(|a| a.children_named("attribute")).find(|a| a.get("name_english") == "BOD").unwrap();
    assert_eq!(bod.get("total"), sheet.attr("BOD").to_string());
    assert_eq!(c.get("init"), format!("{} + {}d6", sheet.initiative, sheet.initiative_dice));
    assert_eq!(c.get("created"), "True");
    assert!(!c.get("nuyen").contains('¥'));
    let skills = c.child("skills").unwrap();
    assert!(skills.children_named("skill").any(|s| s.get("name") == "Spellcasting" && s.get("total") != "0"));
    assert!(c.child("expenses").is_none());
}
