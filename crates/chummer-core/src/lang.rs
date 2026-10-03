//! UI strings and data translations from `lang/*.xml`.
//!
//! `xx-yy.xml` holds UI strings keyed by name. `xx-yy_data.xml` holds
//! translated names of data records, grouped by data file. Lookups fall
//! back to English, then to the key itself.

use std::collections::HashMap;
use std::path::Path;

use crate::xml::{self, Element};

#[derive(Debug, Default, Clone)]
pub struct Language {
    pub code: String,
    pub name: String,
    strings: HashMap<String, String>,
    /// file -> (id or English name) -> translation
    data: HashMap<String, HashMap<String, String>>,
}

impl Language {
    /// Load `code` (e.g. `"de-de"`) from a lang directory, layered over English.
    pub fn load(lang_dir: &Path, code: &str) -> Language {
        let mut lang = Language { code: code.to_owned(), ..Default::default() };
        lang.merge_strings(&lang_dir.join("en-us.xml"));
        if code != "en-us" {
            lang.merge_strings(&lang_dir.join(format!("{code}.xml")));
            lang.load_data(&lang_dir.join(format!("{code}_data.xml")));
        }
        lang
    }

    /// All `xx-yy.xml` files in the directory, as `(code, display name)`.
    pub fn available(lang_dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(lang_dir) else { return out };
        for entry in rd.flatten() {
            let p = entry.path();
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if p.extension().and_then(|e| e.to_str()) != Some("xml") || stem.contains('_') {
                continue;
            }
            let name = std::fs::read_to_string(&p)
                .ok()
                .and_then(|s| xml::parse(&s).ok())
                .map(|r| r.get("name"))
                .unwrap_or_else(|| stem.to_owned());
            out.push((stem.to_owned(), name));
        }
        out.sort();
        out
    }

    fn merge_strings(&mut self, path: &Path) {
        let Some(root) = read(path) else { return };
        if self.name.is_empty() || path.file_stem().and_then(|s| s.to_str()) == Some(&self.code) {
            self.name = root.get("name");
        }
        if let Some(strings) = root.child("strings") {
            for s in strings.children_named("string") {
                self.strings.insert(s.get("key"), s.get("text"));
            }
        }
    }

    fn load_data(&mut self, path: &Path) {
        let Some(root) = read(path) else { return };
        for file in root.children_named("chummer") {
            let Some(fname) = file.attr("file") else { continue };
            let map = self.data.entry(fname.to_owned()).or_default();
            for container in file.elements() {
                collect_translations(container, map);
            }
        }
    }

    /// UI string by key, e.g. `s("String_Karma")`.
    pub fn s(&self, key: &str) -> String {
        self.strings.get(key).cloned().unwrap_or_else(|| key.to_owned())
    }

    pub fn has(&self, key: &str) -> bool {
        self.strings.contains_key(key)
    }

    /// Translated display name for a data record, by id first, then name.
    pub fn data_name(&self, file: &str, id: &str, english: &str) -> String {
        let Some(map) = self.data.get(file) else { return english.to_owned() };
        map.get(&id.to_ascii_lowercase())
            .or_else(|| map.get(english))
            .cloned()
            .unwrap_or_else(|| english.to_owned())
    }
}

fn read(path: &Path) -> Option<Element> {
    xml::parse(&std::fs::read_to_string(path).ok()?).ok()
}

fn collect_translations(container: &Element, map: &mut HashMap<String, String>) {
    for item in container.elements() {
        // `<name translate="X">English</name>` lists (skill groups, categories)
        if let Some(t) = item.attr("translate") {
            map.entry(item.text()).or_insert_with(|| t.to_owned());
            continue;
        }
        let Some(tr) = item.child_text("translate") else { continue };
        if let Some(id) = item.child_text("id") {
            map.insert(id.to_ascii_lowercase(), tr.clone());
        }
        if let Some(name) = item.child_text("name") {
            map.entry(name).or_insert(tr);
        }
    }
}
