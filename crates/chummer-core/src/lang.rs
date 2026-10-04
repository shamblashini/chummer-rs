//! UI strings and data translations from `lang/*.xml`.
//!
//! `xx-yy.xml` holds UI strings keyed by name. `xx-yy_data.xml` holds
//! translated names of data records, grouped by data file. Lookups fall
//! back to English, then to the key itself.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::xml::{self, Element};

#[derive(Debug, Default, Clone)]
pub struct Language {
    pub code: String,
    pub name: String,
    strings: HashMap<String, String>,
    /// Keys defined by this language's own file (not inherited from English).
    own: HashSet<String>,
    /// Normalized en-us text -> key, for [`Language::tr`].
    reverse: HashMap<String, String>,
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
        let stem = path.file_stem().and_then(|s| s.to_str());
        let english = stem == Some("en-us");
        let own = stem == Some(&self.code);
        if let Some(strings) = root.child("strings") {
            for s in strings.children_named("string") {
                let (key, text) = (s.get("key"), s.get("text"));
                if english {
                    self.index_english(&key, &text);
                }
                if own {
                    self.own.insert(key.clone());
                }
                self.strings.insert(key, text);
            }
        }
    }

    fn index_english(&mut self, key: &str, text: &str) {
        let norm = normalize(text);
        if norm.is_empty() {
            return;
        }
        match self.reverse.get(&norm) {
            Some(old) if key_rank(old) <= key_rank(key) => {}
            _ => {
                self.reverse.insert(norm, key.to_owned());
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

    /// Key of the en-us string whose text matches `english` (ignoring case,
    /// surrounding whitespace, a trailing colon and `&` accelerators).
    pub fn key_for(&self, english: &str) -> Option<&str> {
        self.reverse.get(&normalize(english)).map(String::as_str)
    }

    /// Whether `english` maps to a key this language's own file translates.
    pub fn translates(&self, english: &str) -> bool {
        self.key_for(english).is_some_and(|k| self.own.contains(k))
    }

    /// Translate an English UI label via the en-us text of Chummer's language
    /// files. The label keeps its own trailing colon or ellipsis (or lack of
    /// one); unknown labels, and labels whose translation only differs in
    /// case or accelerators, come back unchanged.
    pub fn tr(&self, english: &str) -> String {
        let norm = normalize(english);
        let Some(text) = self.reverse.get(&norm).and_then(|k| self.strings.get(k)) else {
            return english.to_owned();
        };
        if normalize(text) == norm {
            return english.to_owned();
        }
        let (core, _) = split_suffix(text);
        let (_, suffix) = split_suffix(english);
        let lead = &english[..english.len() - english.trim_start().len()];
        format!("{lead}{}{suffix}", strip_accelerators(core.trim()))
    }

    /// [`Language::tr`] over a fixed list of labels, e.g. column headers.
    pub fn tr_all<const N: usize>(&self, english: [&str; N]) -> [String; N] {
        english.map(|e| self.tr(e))
    }

    /// [`Language::tr`] for a template with Chummer's `{0}`, `{1}`, ...
    /// placeholders, which are then filled from `args`.
    pub fn tr_fmt(&self, english: &str, args: &[&dyn std::fmt::Display]) -> String {
        let mut out = self.tr(english);
        for (i, a) in args.iter().enumerate() {
            out = out.replace(&format!("{{{i}}}"), &a.to_string());
        }
        out
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

/// Lookup form of a UI text: accelerators resolved, trailing colon and
/// ellipsis dropped, lowercased.
fn normalize(text: &str) -> String {
    strip_accelerators(split_suffix(text).0.trim()).to_lowercase()
}

/// Split a label into its text and a trailing run of `:`, `...`, `…` and
/// whitespace.
fn split_suffix(text: &str) -> (&str, &str) {
    let core = text.trim_end_matches(|c: char| c == ':' || c == '.' || c == '…' || c.is_whitespace());
    (core, &text[core.len()..])
}

/// WinForms accelerators: `&&` is a literal `&`, `&X` marks X as hotkey.
fn strip_accelerators(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('&') => {
                out.push('&');
                chars.next();
            }
            Some(n) if !n.is_whitespace() => {}
            _ => out.push('&'),
        }
    }
    out
}

/// Preference among keys sharing one English text: generic UI prefixes
/// first, then the shortest (least specialised) key.
fn key_rank(key: &str) -> (usize, usize, String) {
    const PREFIXES: [&str; 8] = ["Tab_", "String_", "Label_", "Button_", "Checkbox_", "Menu_", "Title_", "Node_"];
    let p = PREFIXES.iter().position(|p| key.starts_with(p)).unwrap_or(PREFIXES.len());
    (p, key.len(), key.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(code: &str) -> Language {
        Language::load(&crate::data::resource_dir("lang").unwrap(), code)
    }

    #[test]
    fn accelerators() {
        assert_eq!(strip_accelerators("&File"), "File");
        assert_eq!(strip_accelerators("Clothing && Armor"), "Clothing & Armor");
        assert_eq!(strip_accelerators("Rock & Roll"), "Rock & Roll");
        assert_eq!(normalize(" &Search: "), "search");
    }

    #[test]
    fn reverse_lookup_de() {
        let de = lang("de-de");
        assert_eq!(de.tr("Skills"), "Fertigkeiten");
        assert_eq!(de.tr("Karma"), "Karma");
        assert_eq!(de.tr("No such label here"), "No such label here");
        assert_eq!(de.tr("Skills:"), "Fertigkeiten:");
        assert_eq!(de.tr("Clothing & Armor"), de.s("Tab_Armor").replace("&&", "&"));
        assert_eq!(de.tr("File"), "Datei");
        assert_ne!(de.tr("Open"), "Open");
        assert_eq!(de.tr("Open…"), format!("{}…", de.tr("Open")));
        assert_eq!(de.tr("  Skills: "), "  Fertigkeiten: ");
        assert!(de.translates("Skills"));
        assert!(!de.translates("No such label here"));
    }

    #[test]
    fn english_is_identity() {
        let en = lang("en-us");
        assert_eq!(en.tr("Skills"), "Skills");
        assert_eq!(en.tr("File"), "File");
        assert_eq!(en.tr("Clothing & Armor"), "Clothing & Armor");
        assert_eq!(en.tr("karma"), "karma");
        assert_eq!(lang("de-de").tr("karma"), "karma");
        assert_eq!(Language::default().tr("Skills"), "Skills");
    }

    #[test]
    fn placeholders() {
        let en = lang("en-us");
        assert_eq!(en.tr_fmt("{0} over allotted Knowledge Skill point limit", &[&3]), "3 over allotted Knowledge Skill point limit");
    }
}
