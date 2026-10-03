//! Bundles the data a front end needs: game data, skill catalog and
//! settings presets.

use crate::calc::{self, Rules, Sheet, SkillCatalog};
use crate::character::Character;
use crate::data::{DataError, DataStore};
use crate::settings::{self, SettingsLibrary};

pub struct Engine {
    pub store: DataStore,
    pub catalog: SkillCatalog,
    pub settings: SettingsLibrary,
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
        let settings = SettingsLibrary::load(&store, settings::user_settings_dir().as_deref())?;
        Ok(Engine { store, catalog, settings })
    }

    /// House rules for a character, from its `<settings>` preset.
    pub fn rules_for(&self, ch: &Character) -> Rules {
        self.settings.resolve(&ch.field("settings")).map(Rules::from_settings).unwrap_or_default()
    }

    pub fn sheet(&self, ch: &Character) -> Sheet {
        calc::compute(ch, &self.rules_for(ch), Some(&self.store), Some(&self.catalog))
    }

    /// Save, refreshing the export totals first.
    pub fn save(&self, ch: &mut Character, path: &std::path::Path) -> std::io::Result<()> {
        let rules = self.rules_for(ch);
        let sheet = calc::compute(ch, &rules, Some(&self.store), Some(&self.catalog));
        calc::stamp_totals(ch, &sheet, &rules);
        ch.save(path)
    }
}
