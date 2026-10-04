//! Career mode: the karma and nuyen expense log, and spending karma and
//! nuyen after creation.
//!
//! Chummer5a keeps every career-mode change in `<expenses>` as an
//! `ExpenseLogEntry` (`Backend/Uniques/Expenses.cs`). Each entry has an
//! `<undo>` block naming what was bought, so the spend can be reversed.
//! The spending code lives in `CharacterCareer.cs`, `CharacterAttrib.Upgrade`,
//! `Skill.Upgrade`, `SkillGroup.Upgrade` and `Skill.AddSpecialization`.
//!
//! The log stays in [`Character::doc`]: existing entries are read on demand
//! and never rewritten (their decimal formatting, e.g. `830.000`, is kept),
//! new entries are inserted in date order.
//!
//! - [`ledger`]: the entry model, load/save, manual entries, totals and
//!   street cred.
//! - [`karma`]: spending karma on attributes, skills, qualities, spells and
//!   initiation.
//! - [`nuyen`]: spending nuyen and selling items.
//! - [`undo`]: reversing an entry.

pub mod karma;
pub mod ledger;
pub mod nuyen;
pub mod undo;

pub use karma::*;
pub use ledger::*;
pub use nuyen::*;
pub use undo::*;

use crate::calc::Rules;
use crate::character::Character;
use crate::engine::Engine;
use crate::settings::CharacterSettings;

/// Why a career-mode action was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CareerError {
    #[error("the character is not in career mode")]
    NotCareer,
    #[error("not enough karma: needs {need}, has {have}")]
    NotEnoughKarma { need: i32, have: i32 },
    #[error("not enough nuyen: needs {need}¥, has {have}¥")]
    NotEnoughNuyen { need: f64, have: f64 },
    #[error("{0} cannot be raised any further")]
    AtMaximum(String),
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("undoing a {0} expense is not supported yet")]
    Unsupported(String),
}

/// The house rules career mode reads. Chummer keeps these in
/// `CharacterSettings`; the karma costs come from [`Rules`].
#[derive(Debug, Clone)]
pub struct CareerRules {
    pub rules: Rules,
    /// `MaxSkillRating` (career maximum of active skills and groups).
    pub max_skill_rating: i32,
    /// `MaxKnowledgeSkillRating`.
    pub max_knowledge_skill_rating: i32,
    /// `DontDoubleQualityPurchases`.
    pub dont_double_quality_purchases: bool,
    /// `DontDoubleQualityRefunds`.
    pub dont_double_quality_refunds: bool,
    /// `AlternateMetatypeAttributeKarma`.
    pub alternate_metatype_attribute_karma: bool,
    /// `CompensateSkillGroupKarmaDifference`.
    pub compensate_skill_group_karma_difference: bool,
    /// `SpecializationsBreakSkillGroups`.
    pub specializations_break_skill_groups: bool,
    /// `UseCalculatedPublicAwareness`.
    pub use_calculated_public_awareness: bool,
    /// `NuyenPerBPWftM` ("working for the man": karma to nuyen).
    pub nuyen_per_bp_wftm: f64,
    /// `NuyenPerBPWftP` ("working for the people": nuyen to karma).
    pub nuyen_per_bp_wftp: f64,
    /// `KarmaMAGInitiation{Group,Ordeal,Schooling}Percent`.
    pub mag_initiation_percent: [f64; 3],
    /// `KarmaRESInitiation{Group,Ordeal,Schooling}Percent`.
    pub res_initiation_percent: [f64; 3],
}

impl Default for CareerRules {
    fn default() -> Self {
        CareerRules {
            rules: Rules::default(),
            max_skill_rating: 12,
            max_knowledge_skill_rating: 12,
            dont_double_quality_purchases: false,
            dont_double_quality_refunds: false,
            alternate_metatype_attribute_karma: false,
            compensate_skill_group_karma_difference: false,
            specializations_break_skill_groups: true,
            use_calculated_public_awareness: false,
            nuyen_per_bp_wftm: 2000.0,
            nuyen_per_bp_wftp: 2000.0,
            // Chummer's defaults; the settings file does not store them.
            mag_initiation_percent: [0.1, 0.1, 0.1],
            res_initiation_percent: [0.1, 0.2, 0.1],
        }
    }
}

impl CareerRules {
    /// Read the career options of a settings preset (`CharacterSettings.Load`).
    pub fn from_settings(s: &CharacterSettings) -> CareerRules {
        let d = CareerRules::default();
        let flag = |k: &str, default: bool| s.raw.get_bool(k).unwrap_or(default);
        let wftm = s.raw.get_f64("nuyenperbpwftm").or_else(|| s.raw.get_f64("nuyenperbp")).unwrap_or(d.nuyen_per_bp_wftm);
        CareerRules {
            rules: Rules::from_settings(s),
            max_skill_rating: s.int("maxskillrating", d.max_skill_rating),
            max_knowledge_skill_rating: s.int("maxknowledgeskillrating", d.max_knowledge_skill_rating),
            dont_double_quality_purchases: flag("dontdoublequalities", d.dont_double_quality_purchases),
            dont_double_quality_refunds: flag("dontdoublequalityrefunds", d.dont_double_quality_refunds),
            alternate_metatype_attribute_karma: flag("alternatemetatypeattributekarma", d.alternate_metatype_attribute_karma),
            compensate_skill_group_karma_difference: flag("compensateskillgroupkarmadifference", d.compensate_skill_group_karma_difference),
            specializations_break_skill_groups: flag("specializationsbreakskillgroups", d.specializations_break_skill_groups),
            use_calculated_public_awareness: flag("usecalculatedpublicawareness", d.use_calculated_public_awareness),
            nuyen_per_bp_wftm: wftm,
            nuyen_per_bp_wftp: s.raw.get_f64("nuyenperbpwftp").unwrap_or(wftm),
            ..d
        }
    }

    /// The career rules of a character's settings preset.
    pub fn for_character(engine: &Engine, ch: &Character) -> CareerRules {
        engine.settings.resolve(&ch.field("settings")).map(CareerRules::from_settings).unwrap_or_default()
    }
}

/// Refuse anything but career mode.
fn require_career(ch: &Character) -> Result<(), CareerError> {
    if ch.created {
        Ok(())
    } else {
        Err(CareerError::NotCareer)
    }
}

/// Refuse a karma spend the character cannot afford.
fn require_karma(ch: &Character, cost: i32) -> Result<(), CareerError> {
    if ch.karma < cost {
        Err(CareerError::NotEnoughKarma { need: cost, have: ch.karma })
    } else {
        Ok(())
    }
}

/// Refuse a nuyen spend the character cannot afford.
fn require_nuyen(ch: &Character, cost: f64) -> Result<(), CareerError> {
    if ch.nuyen + 1e-9 < cost {
        Err(CareerError::NotEnoughNuyen { need: cost, have: ch.nuyen })
    } else {
        Ok(())
    }
}
