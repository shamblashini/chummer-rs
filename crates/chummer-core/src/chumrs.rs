//! `.chumrs`: chummer-rs's own character file.
//!
//! A [`crate::container`] (ZIP + `manifest.json`) with format
//! `"chummer-rs character"` holding:
//!
//! - `character.xml`: the character in canonical form
//!   ([`crate::command::canonical`], the same XML a `.chum5` holds, less
//!   the totals Chummer reads only for export), with each mugshot moved out
//!   to its own entry: the `<mugshot>` stays, empty, with an
//!   `entry="mugshots/0.png"` attribute naming the image;
//! - `mugshots/<n>.<ext>`: the images, as the bytes the base64 decoded to
//!   (stored, not compressed again);
//! - `history.json` (optional): what was changed when, for the History
//!   view across sessions;
//! - `guide.json` (optional): where guided creation was.
//!
//! The engine works on the XML as before; this only changes how it is
//! stored. `docs/file-format.md` is the full specification.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::character::{Character, LoadError};
use crate::container::{self, Archive, ContainerError, Kind, Limits};
use crate::xml::{self, Element, Node};

/// File extension, without the dot.
pub const EXTENSION: &str = "chumrs";
/// The manifest's `format`.
pub const FORMAT: &str = "chummer-rs character";
/// The schema version this build writes.
pub const SCHEMA_VERSION: u32 = 1;

pub const CHARACTER: &str = "character.xml";
pub const HISTORY: &str = "history.json";
pub const GUIDE: &str = "guide.json";
/// The attribute on an emptied `<mugshot>` naming its image entry.
pub const MUGSHOT_ATTR: &str = "entry";

/// Most history items kept in a file (the oldest go first).
pub const HISTORY_LIMIT: usize = 2000;

/// Upgrades of older `.chumrs` files, by schema version. Version 1 is the
/// first; a change that older readers would misread adds a step here.
pub static MIGRATIONS: &[container::Migration] = &[];

pub static KIND: Kind = Kind { format: FORMAT, noun: "character", schema_version: SCHEMA_VERSION, migrations: MIGRATIONS, limits: LIMITS };

const LIMITS: Limits = Limits { file: 256 << 20, entries: 4096, entry: crate::chum5lz::MAX_DECOMPRESSED, total: 512 << 20, manifest: 4 << 20 };

/// One change in the history: when, by whom, what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    /// Unix time in milliseconds.
    pub at: i64,
    /// Who made it; empty for the local user.
    #[serde(default)]
    pub author: String,
    /// What it did ("Raised Pistols to 5 (10 karma)").
    pub description: String,
}

/// Where guided creation was: the current step and the steps visited, as
/// `chargen::guide::Step` ids.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideState {
    pub step: String,
    #[serde(default)]
    pub visited: Vec<String>,
}

/// What a `.chumrs` holds besides the character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// Oldest first.
    pub history: Vec<HistoryItem>,
    pub guide: Option<GuideState>,
    /// When the file was first written (kept across saves).
    pub created: Option<String>,
}

/// Whether `path` names a `.chumrs` (by extension).
pub fn is_chumrs(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// The image type of `bytes`, by its magic number, as a file extension.
fn image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "bin"
    }
}

/// Standard base64 with padding, no line breaks.
pub fn encode_base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Move the character's mugshots (`<character><mugshots><mugshot>`) out
/// of `doc` into image entries named `<prefix>mugshots/<n>.<ext>`. Only a
/// mugshot that is one plain text of base64 which encodes back to the same
/// text moves, so putting it back ([`insert_mugshots`]) is exact; any
/// other stays inline.
pub fn extract_mugshots(doc: &mut Element, prefix: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let Some(shots) = doc.child_mut("mugshots") else { return out };
    for (i, m) in shots.elements_mut().filter(|e| e.name == "mugshot").enumerate() {
        if !m.attrs.is_empty() {
            continue;
        }
        let [Node::Text(t)] = m.children.as_slice() else { continue };
        let Some(bytes) = crate::contacts::decode_base64(t) else { continue };
        if bytes.is_empty() || encode_base64(&bytes) != *t {
            continue;
        }
        let name = format!("{prefix}mugshots/{i}.{}", image_ext(&bytes));
        m.children.clear();
        m.set_attr(MUGSHOT_ATTR, name.clone());
        out.push((name, bytes));
    }
    out
}

/// Put the images back into the `<mugshot>`s [`extract_mugshots`] emptied.
pub fn insert_mugshots(doc: &mut Element, archive: &Archive) -> Result<(), ContainerError> {
    let Some(shots) = doc.child_mut("mugshots") else { return Ok(()) };
    for m in shots.elements_mut().filter(|e| e.name == "mugshot") {
        let Some(name) = m.attr(MUGSHOT_ATTR).map(str::to_owned) else { continue };
        let bytes = archive.get(&name).ok_or_else(|| ContainerError::Corrupt(format!("the mugshot {name} is missing")))?;
        m.attrs.retain(|(k, _)| k != MUGSHOT_ATTR);
        m.children = vec![Node::Text(encode_base64(bytes))];
    }
    Ok(())
}

/// The canonical XML of `ch` with its mugshots moved out (see
/// [`extract_mugshots`]).
pub fn split(ch: &Character, prefix: &str) -> (String, Vec<(String, Vec<u8>)>) {
    split_document(crate::command::canonical_document(ch), prefix)
}

/// [`split`] for a character document.
pub fn split_document(mut doc: Element, prefix: &str) -> (String, Vec<(String, Vec<u8>)>) {
    let shots = extract_mugshots(&mut doc, prefix);
    (doc.to_xml_string(), shots)
}

/// The character document stored as `entry` in `archive`, mugshots back
/// in place.
pub fn join_document(archive: &Archive, entry: &str) -> Result<Element, LoadError> {
    let text = archive.text(entry).map_err(|e| format_err(&e))?.ok_or_else(|| format_err(&ContainerError::Corrupt(format!("{entry} is missing"))))?;
    let mut doc = xml::parse(text).map_err(|e| LoadError::Xml(Default::default(), e))?;
    insert_mugshots(&mut doc, archive).map_err(|e| format_err(&e))?;
    Ok(doc)
}

fn format_err(e: &ContainerError) -> LoadError {
    LoadError::Format(Default::default(), e.to_string())
}

/// The `.chumrs` bytes for `ch` with `extras`.
pub fn to_bytes(ch: &Character, extras: &Extras) -> Vec<u8> {
    let mut a = Archive::new();
    let (xml, shots) = split(ch, "");
    a.put(CHARACTER, xml);
    for (name, bytes) in shots {
        a.put(name, bytes);
    }
    if !extras.history.is_empty() {
        let skip = extras.history.len().saturating_sub(HISTORY_LIMIT);
        a.put(HISTORY, serde_json::to_vec_pretty(&extras.history[skip..]).expect("history serialises"));
    }
    if let Some(g) = &extras.guide {
        a.put(GUIDE, serde_json::to_vec_pretty(g).expect("guide state serialises"));
    }
    a.manifest.created = extras.created.clone().unwrap_or_default();
    // For listings without parsing the XML; the essence is the export
    // total the canonical form leaves out, put back as <totaless>.
    let mut summary = serde_json::Map::new();
    summary.insert("name".into(), ch.display_name().into());
    if let Some(ess) = ch.doc.child_text("totaless") {
        summary.insert("essence".into(), ess.into());
    }
    a.manifest.extra.insert("summary".into(), summary.into());
    container::encode(&KIND, &a)
}

/// Read a `.chumrs` from its bytes.
pub fn from_bytes(bytes: &[u8]) -> Result<(Character, Extras), LoadError> {
    let a = container::decode(&KIND, bytes).map_err(|e| format_err(&e))?;
    let mut doc = join_document(&a, CHARACTER)?;
    if let Some(ess) = a.manifest.extra.get("summary").and_then(|s| s.get("essence")).and_then(|e| e.as_str()) {
        if doc.child("totaless").is_none() {
            doc.set_child_text("totaless", ess);
        }
    }
    let ch = Character::from_document(doc)?;
    // Extras are a convenience: a damaged one is dropped, not fatal.
    let history = a.get(HISTORY).and_then(|b| serde_json::from_slice(b).ok()).unwrap_or_default();
    let guide = a.get(GUIDE).and_then(|b| serde_json::from_slice(b).ok());
    let created = Some(a.manifest.created.clone()).filter(|c| !c.is_empty());
    Ok((ch, Extras { history, guide, created }))
}

/// Write `ch` with `extras` to `path` as a `.chumrs` (atomically).
pub fn write(path: &Path, ch: &Character, extras: &Extras) -> std::io::Result<()> {
    container::atomic_write(path, &to_bytes(ch, extras))
}

/// Load a character file of any kind (`.chumrs`, `.chum5`, `.chum5lz`,
/// told apart by content), with the extras a `.chumrs` has (none for a
/// Chummer file).
pub fn load_any(path: &Path) -> Result<(Character, Extras), LoadError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::Io(path.to_owned(), e))?;
    let (mut ch, extras) = from_any_bytes(&bytes, crate::chum5lz::is_chum5lz(path)).map_err(|e| e.with_path(path))?;
    ch.file = Some(path.to_owned());
    Ok((ch, extras))
}

/// A character from the bytes of any character file; `lzma` when a file
/// that is not a container should be decompressed (a `.chum5lz`).
pub fn from_any_bytes(bytes: &[u8], lzma: bool) -> Result<(Character, Extras), LoadError> {
    if container::is_container(bytes) {
        return from_bytes(bytes);
    }
    let text = crate::chum5lz::text_from_bytes(bytes, lzma).map_err(|e| LoadError::Io(Default::default(), e))?;
    Ok((Character::from_str(&text)?, Extras::default()))
}

/// Write a character to `path` in the format its extension names:
/// `.chumrs` (with `extras`), `.chum5lz`, or else plain `.chum5` XML.
pub fn save_any(path: &Path, ch: &Character, extras: &Extras) -> std::io::Result<()> {
    if is_chumrs(path) {
        write(path, ch, extras)
    } else {
        crate::chum5lz::write_text(path, &ch.to_xml_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_decoder() {
        for n in 0..40u8 {
            let data: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            let s = encode_base64(&data);
            assert_eq!(s.len() % 4, 0);
            assert_eq!(crate::contacts::decode_base64(&s).unwrap(), data);
        }
        assert_eq!(encode_base64(b"Man"), "TWFu");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
        assert_eq!(encode_base64(b"M"), "TQ==");
    }

    fn doc_with(shots: &[&str]) -> Element {
        let inner: String = shots.iter().map(|s| format!("<mugshot>{s}</mugshot>")).collect();
        xml::parse(&format!("<character><name>X</name><mainmugshotindex>0</mainmugshotindex><mugshots>{inner}</mugshots></character>")).unwrap()
    }

    #[test]
    fn mugshots_move_out_and_back_exactly() {
        let png = encode_base64(b"\x89PNG\r\n\x1a\nrest of the image");
        let odd = "iVBORw0KGgo"; // no padding: does not re-encode the same
        let doc = doc_with(&[&png, odd, "  "]);
        let mut d = doc.clone();
        let shots = extract_mugshots(&mut d, "");
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].0, "mugshots/0.png");
        assert!(!d.to_xml_string().contains(&png));
        assert!(d.to_xml_string().contains(odd));
        let mut a = Archive::new();
        for (n, b) in shots {
            a.put(n, b);
        }
        insert_mugshots(&mut d, &a).unwrap();
        assert_eq!(d.to_xml_string(), doc.to_xml_string());
        // A missing image is damage, not a silent loss.
        let mut d = doc.clone();
        extract_mugshots(&mut d, "");
        assert!(insert_mugshots(&mut d, &Archive::new()).is_err());
    }
}
