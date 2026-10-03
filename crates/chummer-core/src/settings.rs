//! Character settings (house rules): built-in presets from
//! `data/settings.xml` and user files from the settings directory.

use std::path::{Path, PathBuf};

use crate::data::{DataError, DataStore};
use crate::xml::{self, Element};

/// One settings preset. Unknown options stay in `raw`; the accessors cover
/// the options the engine uses.
#[derive(Debug, Clone)]
pub struct CharacterSettings {
    pub raw: Element,
    /// File name for user presets, `None` for built-ins.
    pub file: Option<PathBuf>,
}

impl CharacterSettings {
    pub fn id(&self) -> String {
        self.raw.get("id")
    }
    pub fn name(&self) -> String {
        self.raw.get("name")
    }
    pub fn flag(&self, key: &str) -> bool {
        self.raw.get_bool(key).unwrap_or(false)
    }
    pub fn int(&self, key: &str, default: i32) -> i32 {
        self.raw.get_i32(key).unwrap_or(default)
    }
    pub fn text(&self, key: &str, default: &str) -> String {
        self.raw.child_text(key).unwrap_or_else(|| default.to_owned())
    }
    /// A karma cost from `<karmacost>`, e.g. `karma("karmaattribute", 5)`.
    pub fn karma(&self, key: &str, default: i32) -> i32 {
        self.raw.child("karmacost").and_then(|k| k.get_i32(key)).unwrap_or(default)
    }
    /// Source books enabled by this preset.
    pub fn books(&self) -> Vec<String> {
        self.raw
            .child("books")
            .map(|b| b.children_named("book").map(Element::text).collect())
            .unwrap_or_default()
    }
    pub fn build_method(&self) -> String {
        self.text("buildmethod", "Priority")
    }
    pub fn max_availability(&self) -> i32 {
        self.int("availability", 12)
    }
    pub fn knowledge_points_expression(&self) -> String {
        self.text("knowledgepointsexpression", "({INTUnaug} + {LOGUnaug}) * 2")
    }
    pub fn contact_points_expression(&self) -> String {
        self.text("contactpointsexpression", "{CHAUnaug} * 3")
    }
    pub fn essence_decimals(&self) -> u32 {
        // "#,0.00" -> 2 decimals
        let fmt = self.text("essenceformat", "#,0.00");
        fmt.split('.').nth(1).map(|d| d.len() as u32).unwrap_or(0)
    }
}

/// All presets: built-ins first, then user files.
pub struct SettingsLibrary {
    pub presets: Vec<CharacterSettings>,
}

impl SettingsLibrary {
    pub fn load(store: &DataStore, user_dir: Option<&Path>) -> Result<Self, DataError> {
        let doc = store.doc("settings.xml")?;
        let mut presets: Vec<CharacterSettings> = doc
            .child("settings")
            .map(|s| s.children_named("setting").map(|e| CharacterSettings { raw: e.clone(), file: None }).collect())
            .unwrap_or_default();
        if let Some(dir) = user_dir {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("xml") {
                        continue;
                    }
                    let Ok(src) = std::fs::read_to_string(&p) else { continue };
                    let Ok(root) = xml::parse(&src) else { continue };
                    // User files are `<settings>` roots holding one preset.
                    let raw = if root.name == "settings" && root.child("name").is_some() { root } else { continue };
                    presets.push(CharacterSettings { raw, file: Some(p) });
                }
            }
        }
        Ok(Self { presets })
    }

    /// Resolve a character's `<settings>` value: a user file name, a preset
    /// id, or a name. Falls back to the first built-in ("Standard").
    pub fn resolve(&self, key: &str) -> Option<&CharacterSettings> {
        let key = key.trim();
        self.presets
            .iter()
            .find(|p| p.file.as_deref().and_then(Path::file_name).and_then(|f| f.to_str()) == Some(key))
            .or_else(|| self.presets.iter().find(|p| p.id().eq_ignore_ascii_case(key)))
            .or_else(|| self.presets.iter().find(|p| p.name() == key))
            .or_else(|| self.presets.first())
    }
}

/// Default user settings directory: `$XDG_CONFIG_HOME/chummer-rs/settings`.
pub fn user_settings_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("chummer-rs").join("settings"))
}
