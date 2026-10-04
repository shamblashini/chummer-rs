//! Career-mode karma for the magic editors: spells with their
//! `SelectSpell` options, martial art techniques, binding foci and mystic
//! adept power points (`CharacterCareer.cs`).

use super::ledger::{book_karma, ExpenseUndo, KarmaExpenseType};
use super::{require_career, require_karma, CareerError, CareerRules};
use crate::calc;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::engine::Engine;
use crate::items::magic::{account, martialart, spell};

/// `KarmaTechnique` default.
const KARMA_TECHNIQUE: i32 = 5;
/// `KarmaMysticAdeptPowerPoint` default.
const KARMA_MYSTIC_ADEPT_PP: i32 = 5;

fn settings_karma(engine: &Engine, ch: &Character, key: &str, default: i32) -> i32 {
    engine.settings.resolve(&ch.field("settings")).map_or(default, |s| s.karma(key, default))
}

/// Add a spell with its options and pay for it (`tsAddSpell`): rituals and
/// alchemical preparations cost their own category; a "free" spell costs
/// nothing. A failed payment removes the spell again. Returns its guid.
pub fn learn_spell_with(ch: &mut Character, engine: &Engine, store: &DataStore, rec: Record<'_>, extra: Option<&str>, o: &spell::SpellOptions) -> Result<String, CareerError> {
    require_career(ch)?;
    let guid = spell::add(ch, store, rec, extra, o);
    if let Err(e) = super::pay_for_spell(ch, engine, &guid) {
        ch.improvements.remove_from_source(&guid);
        ch.remove_item("spells", &guid);
        return Err(e);
    }
    Ok(guid)
}

/// Karma for the next technique of a martial art: the first one comes
/// with the art, every further one costs `KarmaTechnique`.
pub fn technique_karma_cost(engine: &Engine, ch: &Character, art_guid: &str) -> i32 {
    let has = ch
        .items("martialarts", "martialart")
        .into_iter()
        .find(|a| a.get("guid").eq_ignore_ascii_case(art_guid))
        .and_then(|a| a.child("martialarttechniques"))
        .is_some_and(|t| t.children_named("martialarttechnique").next().is_some());
    if has {
        settings_karma(engine, ch, "karmatechnique", KARMA_TECHNIQUE)
    } else {
        0
    }
}

/// Learn a technique of a martial art the character knows and pay for it
/// (`tsMartialArtsAddTechnique_Click`). Returns the technique's guid.
pub fn learn_technique(ch: &mut Character, engine: &Engine, store: &DataStore, art_guid: &str, technique: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cost = technique_karma_cost(engine, ch, art_guid);
    require_karma(ch, cost)?;
    let guid = martialart::add_technique(ch, store, art_guid, technique).map_err(CareerError::Refused)?;
    book_karma(ch, -cost, format!("Learned Technique {technique}"), ExpenseUndo::karma(KarmaExpenseType::AddMartialArtTechnique, guid.clone()));
    Ok(guid)
}

/// Karma to bind a focus gear item (`Focus.BindingKarmaCost`).
pub fn focus_karma_cost(engine: &Engine, ch: &Character, gear: &crate::xml::Element) -> i32 {
    let fallback = crate::settings::CharacterSettings { raw: crate::xml::Element::new("settings"), file: None };
    account::focus_binding_karma(ch, engine.settings.resolve(&ch.field("settings")).unwrap_or(&fallback), gear)
}

/// Bind a focus and pay its binding karma (`treFoci_BeforeCheck`).
/// Returns the focus guid.
pub fn bind_focus(ch: &mut Character, engine: &Engine, gear_guid: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let gear = crate::items::find_by_guid_mut(ch.items_mut("gears"), gear_guid).map(|g| g.clone()).ok_or_else(|| CareerError::NotFound(format!("focus {gear_guid}")))?;
    let cost = focus_karma_cost(engine, ch, &gear);
    require_karma(ch, cost)?;
    let guid = account::bind_focus(ch, gear_guid).ok_or_else(|| CareerError::Refused(format!("{} is already bound", gear.get("name"))))?;
    book_karma(ch, -cost, format!("Bound Focus {}", gear.get("name")), ExpenseUndo::karma(KarmaExpenseType::BindFocus, guid.clone()));
    Ok(guid)
}

/// Karma for one more mystic adept power point.
pub fn power_point_karma_cost(engine: &Engine, ch: &Character) -> i32 {
    settings_karma(engine, ch, "karmamysadpp", KARMA_MYSTIC_ADEPT_PP)
}

/// A mystic adept buys one power point (`cmdIncreasePowerPoints_Click`),
/// up to MAG. Returns the expense guid.
pub fn buy_power_point(ch: &mut Character, engine: &Engine) -> Result<String, CareerError> {
    require_career(ch)?;
    if !(ch.is_adept() && ch.is_magician()) {
        return Err(CareerError::Refused("only mystic adepts buy power points".into()));
    }
    let cr = CareerRules::for_character(engine, ch);
    let pp = ch.doc.get_i32("magsplitadept").unwrap_or(0);
    let mag = calc::attribute_values(ch, "MAG", &cr.rules).total;
    if pp + 1 > mag {
        return Err(CareerError::AtMaximum("Power points".into()));
    }
    let cost = power_point_karma_cost(engine, ch);
    require_karma(ch, cost)?;
    ch.doc.set_child_text("magsplitadept", (pp + 1).to_string());
    Ok(book_karma(ch, -cost, "Purchased Power Point", ExpenseUndo::karma(KarmaExpenseType::AddPowerPoint, "")))
}
