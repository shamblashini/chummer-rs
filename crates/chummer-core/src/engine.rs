//! Bundles the data a front end needs: game data, skill catalog and
//! settings presets.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::calc::{self, Rules, Sheet, SkillCatalog};
use crate::character::Character;
use crate::custom_data::{self, CustomDataDirectory};
use crate::data::{DataError, DataStore};
use crate::settings::{self, CharacterSettings, SettingsLibrary};

pub struct Engine {
    /// The game data without custom data.
    pub store: DataStore,
    pub catalog: SkillCatalog,
    /// The presets, replaced as a whole by [`Engine::reload_settings`]. A
    /// snapshot ([`Engine::settings`]) stays valid while it is in use.
    settings: RwLock<Arc<SettingsLibrary>>,
    /// The user settings folder the presets were read from.
    settings_dir: Option<PathBuf>,
    /// Folders scanned for custom data directories.
    custom_roots: Vec<PathBuf>,
    /// Known custom data directories, read on first use.
    custom_dirs: OnceLock<Vec<CustomDataDirectory>>,
    /// One store per distinct list of enabled directories.
    stores: Mutex<HashMap<Vec<PathBuf>, Arc<DataStore>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("game data not found; set CHUMMER_RESOURCES to the directory that contains data/ and lang/")]
    NoData,
    #[error(transparent)]
    Data(#[from] DataError),
}

impl Engine {
    pub fn load() -> Result<Engine, EngineError> {
        let store = DataStore::discover().ok_or(EngineError::NoData)?;
        Engine::with_store(store)
    }

    pub fn with_store(store: DataStore) -> Result<Engine, EngineError> {
        let catalog = SkillCatalog::load(&store)?;
        let settings_dir = settings::user_settings_dir();
        let settings = SettingsLibrary::load(&store, settings_dir.as_deref())?;
        Ok(Engine {
            store,
            catalog,
            settings: RwLock::new(Arc::new(settings)),
            settings_dir,
            custom_roots: custom_data::default_roots(),
            custom_dirs: OnceLock::new(),
            stores: Mutex::new(HashMap::new()),
        })
    }

    /// Read user presets from `dir` instead of the user settings folder
    /// (tests, portable setups).
    pub fn with_settings_dir(self, dir: Option<PathBuf>) -> Result<Engine, DataError> {
        let lib = SettingsLibrary::load(&self.store, dir.as_deref())?;
        *self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(lib);
        Ok(Engine { settings_dir: dir, ..self })
    }

    /// The settings presets as they are now. Callers keep the snapshot for
    /// as long as they use it; a reload does not change it.
    pub fn settings(&self) -> Arc<SettingsLibrary> {
        self.settings.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// The presets to change in place (tests); a snapshot taken earlier
    /// keeps the old ones.
    pub fn settings_mut(&mut self) -> &mut SettingsLibrary {
        Arc::make_mut(self.settings.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// The folder user presets are read from and saved to.
    pub fn settings_dir(&self) -> Option<PathBuf> {
        self.settings_dir.clone()
    }

    /// Read the presets again from the game data and the user settings
    /// folder, after a preset was added, imported or saved. Everything
    /// that asks [`Engine::settings`] afterwards sees the new presets;
    /// open characters re-resolve theirs when told to.
    pub fn reload_settings(&self) -> Result<(), DataError> {
        let lib = SettingsLibrary::load(&self.store, self.settings_dir.as_deref())?;
        *self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(lib);
        Ok(())
    }

    /// Look for custom data directories in `roots` instead of the defaults.
    pub fn with_custom_data_roots(mut self, roots: Vec<PathBuf>) -> Engine {
        self.custom_roots = roots;
        self.custom_dirs = OnceLock::new();
        self.stores.lock().unwrap().clear();
        self
    }

    /// Every custom data directory found (`GlobalSettings.CustomDataDirectoryInfos`),
    /// sorted by name within each root.
    pub fn custom_data_directories(&self) -> &[CustomDataDirectory] {
        self.custom_dirs.get_or_init(|| self.custom_roots.iter().flat_map(|r| custom_data::discover(r)).collect())
    }

    /// The directories a settings preset enables, in load order.
    pub fn enabled_custom_data(&self, settings: &CharacterSettings) -> Vec<&CustomDataDirectory> {
        custom_data::enabled_directories(&settings.raw, self.custom_data_directories())
    }

    /// Game data with a settings preset's custom data applied. Stores are
    /// cached per list of enabled directories; documents load lazily.
    pub fn store_for(&self, settings: &CharacterSettings) -> Arc<DataStore> {
        let dirs: Vec<PathBuf> = self.enabled_custom_data(settings).into_iter().map(|d| d.path.clone()).collect();
        self.store_for_dirs(dirs)
    }

    /// Game data with exactly these custom data directories applied, in order.
    pub fn store_for_dirs(&self, dirs: Vec<PathBuf>) -> Arc<DataStore> {
        let mut stores = self.stores.lock().unwrap();
        stores.entry(dirs.clone()).or_insert_with(|| Arc::new(self.store.with_enabled_custom_data(dirs))).clone()
    }

    /// Game data for a character, from its `<settings>` preset.
    pub fn store_for_character(&self, ch: &Character) -> Arc<DataStore> {
        match self.settings().resolve(&ch.field("settings")) {
            Some(s) => self.store_for(s),
            None => self.store_for_dirs(Vec::new()),
        }
    }

    /// House rules for a character, from its `<settings>` preset.
    pub fn rules_for(&self, ch: &Character) -> Rules {
        self.settings().resolve(&ch.field("settings")).map(Rules::from_settings).unwrap_or_default()
    }

    /// Computed values, using the game data as modified by the character's
    /// enabled custom data.
    pub fn sheet(&self, ch: &Character) -> Sheet {
        let store = self.store_for_character(ch);
        calc::compute(ch, &self.rules_for(ch), Some(&store), Some(&self.catalog))
    }

    /// Save, refreshing the export totals first.
    pub fn save(&self, ch: &mut Character, path: &std::path::Path) -> std::io::Result<()> {
        self.save_with(ch, path, &crate::chumrs::Extras::default())
    }

    /// [`Engine::save`] with what a `.chumrs` keeps besides the character.
    pub fn save_with(&self, ch: &mut Character, path: &std::path::Path, extras: &crate::chumrs::Extras) -> std::io::Result<()> {
        let rules = self.rules_for(ch);
        let store = self.store_for_character(ch);
        let sheet = calc::compute(ch, &rules, Some(&store), Some(&self.catalog));
        calc::stamp_totals(ch, &sheet, &rules);
        ch.save_with(path, extras)
    }
}
