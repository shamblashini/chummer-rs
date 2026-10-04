//! Character roster (`CharacterRoster` / `CharacterCache`): summaries of
//! every `.chum5` in a set of folders, without loading full characters.

use std::path::{Path, PathBuf};

use crate::xml;

/// What the roster shows for one file.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub alias: String,
    pub metatype: String,
    pub metavariant: String,
    pub career: bool,
    pub karma: String,
    pub essence: String,
    pub build_method: String,
    pub player: String,
    pub concept: String,
    /// Load error, when the file could not be read.
    pub error: Option<String>,
}

impl Entry {
    pub fn display_name(&self) -> String {
        if !self.alias.trim().is_empty() {
            self.alias.clone()
        } else if !self.name.trim().is_empty() {
            self.name.clone()
        } else {
            self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        }
    }
}

/// Summarise one character file.
pub fn summarize(path: &Path) -> Entry {
    let mut e = Entry { path: path.to_owned(), ..Default::default() };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            e.error = Some(err.to_string());
            return e;
        }
    };
    match xml::parse(&src) {
        Ok(doc) if doc.name == "character" => {
            e.name = doc.get("name");
            e.alias = doc.get("alias");
            e.metatype = doc.get("metatype");
            e.metavariant = doc.get("metavariant");
            e.career = doc.get_bool("created").unwrap_or(false);
            e.karma = doc.get("karma");
            e.essence = doc.get("totaless");
            e.build_method = doc.get("buildmethod");
            e.player = doc.get("playername");
            e.concept = doc.get("concept");
        }
        Ok(doc) => e.error = Some(format!("not a character (<{}>)", doc.name)),
        Err(err) => e.error = Some(err.to_string()),
    }
    e
}

/// All `.chum5` files under the folders (two levels deep), summarised and
/// sorted by name.
pub fn scan(folders: &[PathBuf]) -> Vec<Entry> {
    let mut files = Vec::new();
    for f in folders {
        collect(f, 0, &mut files);
    }
    files.sort();
    files.dedup();
    let mut v: Vec<Entry> = files.iter().map(|p| summarize(p)).collect();
    v.sort_by_key(|e| e.display_name().to_lowercase());
    v
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() && depth < 2 {
            collect(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("chum5")) {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scans_fixtures() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let v = super::scan(&[dir]);
        assert_eq!(v.len(), 34);
        assert!(v.iter().all(|e| e.error.is_none()));
        let blue = v.iter().find(|e| e.display_name() == "BLUE").unwrap();
        assert_eq!(blue.metatype, "Ork");
    }
}
