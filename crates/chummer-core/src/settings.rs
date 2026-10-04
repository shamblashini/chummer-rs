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
    /// What a character's `<settings>` stores: the file name for user
    /// presets, the GUID for built-ins (`CharacterSettings.DictionaryKey`).
    pub fn key(&self) -> String {
        match self.file.as_deref().and_then(std::path::Path::file_name) {
            Some(f) => f.to_string_lossy().into_owned(),
            None => self.id(),
        }
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
    /// id, or a name. Falls back to Standard (see [`Self::fallback`]); use
    /// [`Self::missing_preset`] to tell the user when that happens.
    pub fn resolve(&self, key: &str) -> Option<&CharacterSettings> {
        self.find(key).or_else(|| self.fallback())
    }

    /// The preset `key` names, without any fallback.
    pub fn find(&self, key: &str) -> Option<&CharacterSettings> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        self.presets
            .iter()
            .find(|p| p.file.as_deref().and_then(Path::file_name).and_then(|f| f.to_str()) == Some(key))
            .or_else(|| self.presets.iter().find(|p| key != EMPTY_GUID && p.id().eq_ignore_ascii_case(key)))
            .or_else(|| self.presets.iter().find(|p| p.name() == key))
    }

    /// What characters use when their preset is missing: built-in Standard
    /// (`GlobalSettings.DefaultCharacterSettingDefaultValue`), else the first.
    pub fn fallback(&self) -> Option<&CharacterSettings> {
        self.presets.iter().find(|p| p.file.is_none() && p.id() == STANDARD_ID).or_else(|| self.presets.first())
    }

    /// `Some(name)` when a character's `<settings>` value names a preset
    /// that is not installed, where Chummer5a would say "Cannot Find
    /// Settings File". The name is the key without `.xml`, as Chummer shows
    /// it. An empty value is not missing: Chummer uses its default then.
    pub fn missing_preset(&self, key: &str) -> Option<String> {
        let key = key.trim();
        if key.is_empty() || self.find(key).is_some() {
            return None;
        }
        Some(Path::new(key).file_stem().map_or_else(|| key.to_owned(), |s| s.to_string_lossy().into_owned()))
    }

    /// Check a settings file before [`import`]: whether it parses, and what
    /// it would clash with in `user_dir`.
    pub fn plan_import(&self, src: &Path, user_dir: &Path) -> Result<ImportPlan, String> {
        let text = std::fs::read_to_string(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let root = parse_settings_file(&text).map_err(|e| format!("{}: {e}", src.display()))?;
        let file_name = src.file_name().map(|f| f.to_string_lossy().into_owned()).ok_or("no file name")?;
        let file_name = if file_name.to_ascii_lowercase().ends_with(".xml") { file_name } else { format!("{file_name}.xml") };
        let target = user_dir.join(&file_name);
        let file_clash = match std::fs::read_to_string(&target) {
            Ok(existing) if existing == export_string(&root) || existing == text => Some(FileClash::Identical),
            Ok(_) => Some(FileClash::Different),
            Err(_) => None,
        };
        let name = root.get("name");
        let name_clash = self.presets.iter().any(|p| p.name() == name && p.file.as_deref().and_then(Path::file_name).and_then(|f| f.to_str()) != Some(file_name.as_str()));
        Ok(ImportPlan { root, file_name, file_clash, name_clash })
    }
}

/// GUID of the built-in "Standard" preset.
pub const STANDARD_ID: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

/// What Chummer5a writes as `<id>` for user presets. A user file with any
/// other id counts as built-in there and is keyed by that GUID instead of
/// its file name, so characters referring to the file would not find it.
pub const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// Read a Chummer settings file: the first `<settings>` element holding a
/// `<name>` (Chummer loads `.//settings`).
pub fn parse_settings_file(text: &str) -> Result<Element, String> {
    let root = xml::parse(text).map_err(|e| e.to_string())?;
    fn find(e: &Element) -> Option<&Element> {
        if e.name == "settings" && e.child("name").is_some() {
            return Some(e);
        }
        e.elements().find_map(find)
    }
    let mut found = find(&root).cloned().ok_or("not a Chummer settings file (no <settings> with a <name>)")?;
    if found.get("name").trim().is_empty() {
        return Err("the settings file has an empty <name>".into());
    }
    found.name = "settings".into();
    Ok(found)
}

/// A preset as a standalone settings file, in the layout Chummer5a keeps in
/// its `settings` folder (`CharacterSettings.Save` with the source GUID
/// cleared, as its "Save As" does).
pub fn export_string(raw: &Element) -> String {
    let mut root = raw.clone();
    root.name = "settings".into();
    root.set_child_text("id", EMPTY_GUID);
    root.to_xml_string()
}

/// Write `preset` to `path` as a standalone settings file.
pub fn export(preset: &CharacterSettings, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, export_string(&preset.raw))
}

/// Copy a preset into `dir` as a new user file named after `name`. Returns
/// the new file's path.
pub fn duplicate(preset: &CharacterSettings, name: &str, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.xml", file_stem_for(name)));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut root = preset.raw.clone();
    root.set_child_text("name", name);
    std::fs::write(&path, export_string(&root)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A file name stem made from a preset name.
pub fn file_stem_for(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// An existing user file with the same name as the one being imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClash {
    /// Same content: importing again changes nothing.
    Identical,
    Different,
}

/// A checked settings file, ready for [`import`].
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub root: Element,
    /// File name in the user directory. Characters refer to user presets
    /// by file name, so the import keeps it unless told to rename.
    pub file_name: String,
    pub file_clash: Option<FileClash>,
    /// Another installed preset already shows this `<name>`.
    pub name_clash: bool,
}

impl ImportPlan {
    pub fn name(&self) -> String {
        self.root.get("name")
    }
}

/// How to resolve an import clash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportMode {
    /// Fail on a file clash with different content.
    New,
    /// Replace the user file of the same name.
    Overwrite,
    /// Keep both: save under a free file name, and give the preset a free
    /// display name.
    KeepBoth,
}

/// Install a planned settings file into `user_dir` and return its path.
/// Its `<id>` is cleared (see [`EMPTY_GUID`]); a clashing display name gets
/// a number appended unless the file replaces the one holding that name.
pub fn import(plan: &ImportPlan, lib: &SettingsLibrary, user_dir: &Path, mode: &ImportMode) -> Result<PathBuf, String> {
    std::fs::create_dir_all(user_dir).map_err(|e| e.to_string())?;
    let mut root = plan.root.clone();
    let mut target = user_dir.join(&plan.file_name);
    match (plan.file_clash, mode) {
        (Some(FileClash::Identical), ImportMode::New | ImportMode::Overwrite) => return Ok(target),
        (Some(FileClash::Different), ImportMode::New) => {
            return Err(format!("{} already exists with different content", target.display()));
        }
        (Some(_), ImportMode::KeepBoth) => {
            let stem = Path::new(&plan.file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            target = (2..).map(|i| user_dir.join(format!("{stem}_{i}.xml"))).find(|p| !p.exists()).expect("free file name");
        }
        _ => {}
    }
    let target_name = target.file_name().and_then(|f| f.to_str()).unwrap_or_default().to_owned();
    let taken = |n: &str| lib.presets.iter().any(|p| p.name() == n && p.file.as_deref().and_then(Path::file_name).and_then(|f| f.to_str()) != Some(target_name.as_str()));
    let name = root.get("name");
    if taken(&name) {
        let free = (2..).map(|i| format!("{name} ({i})")).find(|n| !taken(n)).expect("free name");
        root.set_child_text("name", free);
    }
    std::fs::write(&target, export_string(&root)).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Switch a character to another preset, as Chummer5a's "Change Settings
/// File" does: `<settings>` takes the preset's key and `<buildmethod>`
/// follows the preset (Chummer saves `Settings.BuildMethod`). In creation
/// mode Chummer re-runs metatype and priority selection when the build
/// method changes; that is not ported, so such a switch is refused.
pub fn switch_character(ch: &mut crate::character::Character, preset: &CharacterSettings) -> Result<(), String> {
    let new_bm = preset.build_method();
    let old_bm = ch.field("buildmethod");
    let old_bm = if old_bm.is_empty() { "Priority".to_owned() } else { old_bm };
    if !ch.created && new_bm != old_bm {
        return Err(format!(
            "The selected build method ({new_bm}) is different from the existing build method of the character ({old_bm}). Switching build methods during creation is not supported yet."
        ));
    }
    ch.set_field("settings", preset.key());
    ch.set_field("buildmethod", new_bm);
    Ok(())
}

/// Default user settings directory: `$XDG_CONFIG_HOME/chummer-rs/settings`.
pub fn user_settings_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("chummer-rs").join("settings"))
}
