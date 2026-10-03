//! Access to Chummer's XML game data (`data/*.xml`).
//!
//! The C# code queries these files with XPath; this module keeps the same
//! "query the document" model rather than modelling all ~40 schemas as
//! structs. Each file is parsed once, on first use, and shared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::xml::{self, Element};

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("cannot parse {path}: {source}")]
    Xml { path: PathBuf, source: xml::XmlError },
}

/// A data record: one `<skill>`, `<quality>`, `<gear>`, and so on.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a>(pub &'a Element);

impl<'a> Record<'a> {
    pub fn el(&self) -> &'a Element {
        self.0
    }
    pub fn id(&self) -> String {
        self.0.get("id")
    }
    pub fn name(&self) -> String {
        self.0.get("name")
    }
    pub fn category(&self) -> String {
        self.0.get("category")
    }
    pub fn source(&self) -> String {
        self.0.get("source")
    }
    pub fn page(&self) -> String {
        self.0.get("page")
    }
    pub fn get(&self, field: &str) -> String {
        self.0.get(field)
    }
    /// `<hide />` marks records that must not be offered for selection.
    pub fn hidden(&self) -> bool {
        self.0.child("hide").is_some()
    }
}

/// Lazily loaded, shared view of the data directory.
pub struct DataStore {
    data_dir: PathBuf,
    cache: Mutex<HashMap<String, Arc<Element>>>,
}

impl DataStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), cache: Mutex::new(HashMap::new()) }
    }

    /// Locate the bundled `resources/data` directory.
    ///
    /// Search order: `$CHUMMER_DATA`, next to the executable, the system
    /// install dir, then the source tree (for `cargo run`).
    pub fn discover() -> Option<Self> {
        resource_dir("data").map(Self::new)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Parsed root element of a data file, e.g. `doc("skills.xml")`.
    pub fn doc(&self, file: &str) -> Result<Arc<Element>, DataError> {
        if let Some(d) = self.cache.lock().unwrap().get(file) {
            return Ok(d.clone());
        }
        let path = self.data_dir.join(file);
        let src = std::fs::read_to_string(&path).map_err(|source| DataError::Io { path: path.clone(), source })?;
        let root = Arc::new(xml::parse(&src).map_err(|source| DataError::Xml { path, source })?);
        self.cache.lock().unwrap().insert(file.to_owned(), root.clone());
        Ok(root)
    }
}

/// Find a bundled resource directory (`data`, `lang`, `sheets`, ...).
pub fn resource_dir(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("CHUMMER_RESOURCES") {
        candidates.push(PathBuf::from(dir).join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resources").join(name));
            candidates.push(dir.join("../share/chummer-rs").join(name));
        }
    }
    candidates.push(PathBuf::from("/usr/share/chummer-rs").join(name));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources").join(name));
    candidates.into_iter().find(|p| p.is_dir())
}

/// Records inside a document: `<root><container><item/>...</container></root>`.
pub fn records<'a>(doc: &'a Element, container: &str, item: &'a str) -> Vec<Record<'a>> {
    doc.child(container)
        .map(|c| c.children_named(item).map(Record).collect())
        .unwrap_or_default()
}

/// Find one record by `<id>` (case-insensitive, as GUIDs are) or `<name>`.
pub fn find<'a>(doc: &'a Element, container: &str, item: &'a str, key: &str) -> Option<Record<'a>> {
    let c = doc.child(container)?;
    c.children_named(item)
        .find(|e| e.get("id").eq_ignore_ascii_case(key))
        .or_else(|| c.children_named(item).find(|e| e.get("name") == key))
        .map(Record)
}

/// `<categories><category>..</category></categories>` as plain strings.
pub fn categories(doc: &Element) -> Vec<String> {
    doc.child("categories")
        .map(|c| c.children_named("category").map(Element::text).collect())
        .unwrap_or_default()
}

/// The `(file, container, item)` triple for each record kind the UI browses.
pub const BROWSABLE: &[(&str, &str, &str, &str)] = &[
    ("Armor", "armor.xml", "armors", "armor"),
    ("Bioware", "bioware.xml", "biowares", "bioware"),
    ("Books", "books.xml", "books", "book"),
    ("Complex Forms", "complexforms.xml", "complexforms", "complexform"),
    ("Critter Powers", "critterpowers.xml", "powers", "power"),
    ("Critters", "critters.xml", "metatypes", "metatype"),
    ("Cyberware", "cyberware.xml", "cyberwares", "cyberware"),
    ("Drugs", "drugcomponents.xml", "drugs", "drug"),
    ("Echoes", "echoes.xml", "echoes", "echo"),
    ("Gear", "gear.xml", "gears", "gear"),
    ("Lifestyles", "lifestyles.xml", "lifestyles", "lifestyle"),
    ("Martial Arts", "martialarts.xml", "martialarts", "martialart"),
    ("Mentors", "mentors.xml", "mentors", "mentor"),
    ("Metamagic", "metamagic.xml", "metamagics", "metamagic"),
    ("Metatypes", "metatypes.xml", "metatypes", "metatype"),
    ("Adept Powers", "powers.xml", "powers", "power"),
    ("Programs", "programs.xml", "programs", "program"),
    ("Qualities", "qualities.xml", "qualities", "quality"),
    ("Skills", "skills.xml", "skills", "skill"),
    ("Knowledge Skills", "skills.xml", "knowledgeskills", "skill"),
    ("Spells", "spells.xml", "spells", "spell"),
    ("Traditions", "traditions.xml", "traditions", "tradition"),
    ("Vehicles", "vehicles.xml", "vehicles", "vehicle"),
    ("Vehicle Mods", "vehicles.xml", "mods", "mod"),
    ("Weapons", "weapons.xml", "weapons", "weapon"),
    ("Weapon Accessories", "weapons.xml", "accessories", "accessory"),
];
