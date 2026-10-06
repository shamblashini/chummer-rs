//! Career-mode karma for the magic editors: spells with their
//! `SelectSpell` options, martial arts and their techniques, binding foci,
//! mystic adept power points, metamagics and echoes, critter powers and
//! spirit fettering (`CharacterCareer.cs`, `MartialArt.Purchase`,
//! `Spirit.Fettered`).

use super::ledger::{book_karma, ExpenseUndo, KarmaExpenseType};
use super::{require_career, require_karma, CareerError, CareerRules};
use crate::calc;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::engine::Engine;
use crate::items::magic::{account, critterpower, martialart, metamagic, spell, spirit};

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

/// Whether the preset gives mystic adepts a second MAG attribute
/// (`MysAdeptSecondMAGAttribute`) and the character is one.
pub(super) fn uses_second_mag(engine: &Engine, ch: &Character) -> bool {
    ch.is_adept() && ch.is_magician() && engine.settings.resolve(&ch.field("settings")).is_some_and(|s| s.flag("mysadeptsecondmagattribute"))
}

/// The MAG totals a magic limit is checked against: MAG, plus MAGAdept for
/// a mystic adept with the second-MAG house rule. A limit must hold for
/// every one of them.
pub(super) fn mag_limits(engine: &Engine, ch: &Character, rules: &calc::Rules) -> Vec<i32> {
    let mut v = vec![calc::attribute_values(ch, "MAG", rules).total];
    if uses_second_mag(engine, ch) {
        v.push(calc::attribute_values(ch, "MAGAdept", rules).total);
    }
    v
}

/// `Gear.CurrentDisplayName` of a focus as the ledger shows it:
/// "Power Focus (Force: 3) (Agility)".
fn focus_display_name(gear: &crate::xml::Element) -> String {
    let mut s = gear.get("name");
    let rating = gear.get_i32("rating").unwrap_or(0);
    if rating > 0 {
        let label = match gear.get("ratinglabel").as_str() {
            "" if matches!(gear.get("category").as_str(), "Foci" | "Metamagic Foci") => "Force".to_owned(),
            "" | "String_Rating" => "Rating".to_owned(),
            l => l.strip_prefix("String_").unwrap_or(l).to_owned(),
        };
        s += &format!(" ({label}: {rating})");
    }
    let extra = gear.get("extra");
    if !extra.is_empty() {
        s += &format!(" ({extra})");
    }
    s
}

/// Bind a focus and pay its binding karma (`treFoci_BeforeCheck`): the
/// bound foci may number at most MAG and their forces add up to at most
/// MAG × 5 (MAGAdept too for a mystic adept with a second MAG), unless the
/// character ignores rules. The gear is marked bonded and its bonus
/// applied. Returns the focus guid.
pub fn bind_focus(ch: &mut Character, engine: &Engine, gear_guid: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let gear = crate::items::find_by_guid_mut(ch.items_mut("gears"), gear_guid).map(|g| g.clone()).ok_or_else(|| CareerError::NotFound(format!("focus {gear_guid}")))?;
    if ch.items("foci", "focus").iter().any(|f| f.get("gearid").eq_ignore_ascii_case(gear_guid)) {
        return Err(CareerError::Refused(format!("{} is already bound", gear.get("name"))));
    }
    if !ch.flag("ignorerules") {
        check_foci_limits(ch, engine, gear.get_i32("rating").unwrap_or(0))?;
    }
    let cost = focus_karma_cost(engine, ch, &gear);
    require_karma(ch, cost)?;
    let guid = account::bind_focus(ch, gear_guid).ok_or_else(|| CareerError::Refused(format!("{} is already bound", gear.get("name"))))?;
    account::set_focus_bonded(ch, &engine.store, gear_guid, true);
    book_karma(ch, -cost, format!("Bound {}", focus_display_name(&gear)), ExpenseUndo::karma(KarmaExpenseType::BindFocus, guid.clone()));
    Ok(guid)
}

/// The count and force limits of `treFoci_BeforeCheck` for one more focus
/// of `force`: bound foci and bonded stacked foci count.
fn check_foci_limits(ch: &Character, engine: &Engine, force: i32) -> Result<(), CareerError> {
    let gears: Vec<crate::xml::Element> = ch.items("foci", "focus").iter().filter_map(|f| find_gear(ch, &f.get("gearid"))).collect();
    let stacks: Vec<&crate::xml::Element> = ch.items("stackedfoci", "stackedfocus").into_iter().filter(|s| s.get_bool("bonded").unwrap_or(false)).collect();
    let count = 1 + gears.len() as i32 + stacks.len() as i32;
    let stack_force = |s: &crate::xml::Element| -> i32 {
        s.child("gears").map(|g| g.children_named("gear").map(|x| x.get_i32("rating").unwrap_or(0)).sum()).unwrap_or(0)
    };
    let total = force
        + gears.iter().filter(|g| g.get_bool("bonded").unwrap_or(false)).map(|g| g.get_i32("rating").unwrap_or(0)).sum::<i32>()
        + stacks.iter().map(|s| stack_force(s)).sum::<i32>();
    let cr = CareerRules::for_character(engine, ch);
    let limits = mag_limits(engine, ch, &cr.rules);
    if limits.iter().any(|&m| total > m * 5) {
        return Err(CareerError::Refused("the total force of bound foci cannot exceed MAG × 5".into()));
    }
    if limits.iter().any(|&m| count > m) {
        return Err(CareerError::Refused("the number of bound foci cannot exceed MAG".into()));
    }
    Ok(())
}

fn find_gear(ch: &Character, guid: &str) -> Option<crate::xml::Element> {
    fn walk(e: &crate::xml::Element, guid: &str) -> Option<crate::xml::Element> {
        if e.get("guid").eq_ignore_ascii_case(guid) {
            return Some(e.clone());
        }
        e.elements().find_map(|c| walk(c, guid))
    }
    ch.doc.child("gears").and_then(|g| walk(g, guid))
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

/// `KarmaMetamagic` default.
const KARMA_METAMAGIC: i32 = 15;
/// `KarmaSpiritFettering` default.
const KARMA_SPIRIT_FETTERING: i32 = 3;

/// Learn a martial art and pay its `cost` in karma (`MartialArt.Purchase`).
/// Returns the art's guid.
pub fn learn_martial_art(ch: &mut Character, engine: &Engine, rec: Record<'_>, technique: Option<&str>) -> Result<String, CareerError> {
    require_career(ch)?;
    let cost = rec.el().get_i32("cost").unwrap_or(7);
    require_karma(ch, cost)?;
    let guid = martialart::add(ch, &engine.store, rec, technique);
    book_karma(ch, -cost, format!("Learned Martial Art {}", rec.name()), ExpenseUndo::karma(KarmaExpenseType::AddMartialArt, guid.clone()));
    Ok(guid)
}

/// Whether a grade already has its free metamagic or echo: one metamagic,
/// or one spell learned at it (enchantment, ritual), takes the slot.
fn grade_slot_taken(ch: &Character, grade: i32) -> bool {
    let at = |e: &&crate::xml::Element| e.get_i32("grade") == Some(grade);
    ch.items("metamagics", "metamagic").iter().any(at) || ch.items("spells", "spell").iter().any(at)
}

/// Karma for one more metamagic or echo at `grade`: nothing for the first,
/// `KarmaMetamagic` for each further one.
pub fn metamagic_karma_cost(engine: &Engine, ch: &Character, grade: i32) -> i32 {
    if grade_slot_taken(ch, grade) {
        settings_karma(engine, ch, "karmametamagic", KARMA_METAMAGIC)
    } else {
        0
    }
}

/// Learn a metamagic (an echo when the character has RES) at an initiation
/// or submersion `grade` (`tsMetamagicAddMetamagic_Click`). The first one
/// at a grade is free; any further one costs `KarmaMetamagic`. Returns its
/// guid.
pub fn learn_metamagic(ch: &mut Character, engine: &Engine, rec: Record<'_>, forced: Option<&str>, grade: i32) -> Result<String, CareerError> {
    require_career(ch)?;
    if grade < 1 || !ch.items("initiationgrades", "initiationgrade").iter().any(|g| g.get_i32("grade") == Some(grade)) {
        return Err(CareerError::NotFound(format!("grade {grade}")));
    }
    let pay = grade_slot_taken(ch, grade);
    let cost = if pay { metamagic_karma_cost(engine, ch, grade) } else { 0 };
    require_karma(ch, cost)?;
    let echo = ch.res_enabled();
    let guid = metamagic::add_at(ch, &engine.store, rec, if echo { "Echo" } else { "Metamagic" }, forced, grade);
    let name = ch.items("metamagics", "metamagic").into_iter().find(|m| m.get("guid") == guid).map(|m| m.get("name")).unwrap_or_default();
    if pay {
        let what = if echo { "Echo" } else { "Metamagic" };
        book_karma(ch, -cost, format!("{what} {name}"), ExpenseUndo::karma(KarmaExpenseType::AddMetamagic, guid.clone()));
    }
    Ok(guid)
}

/// Add a critter power and pay its `karma` (`cmdAddCritterPower_Click`).
/// Chummer logs the purchase even when the power costs nothing. Returns
/// its guid.
pub fn learn_critter_power(ch: &mut Character, engine: &Engine, rec: Record<'_>, rating: i32, forced: Option<&str>) -> Result<String, CareerError> {
    require_career(ch)?;
    let cost = rec.el().get_i32("karma").unwrap_or(0);
    require_karma(ch, cost)?;
    let guid = critterpower::add(ch, &engine.store, rec, rating, forced);
    book_karma(ch, -cost, format!("Purchased Critter Power {}", rec.name()), ExpenseUndo::karma(KarmaExpenseType::AddCritterPower, guid.clone()));
    Ok(guid)
}

/// Karma to fetter a spirit or sprite: Force × `KarmaSpiritFettering` for
/// spirits, Force for sprites.
pub fn fettering_karma_cost(engine: &Engine, ch: &Character, spirit: &crate::xml::Element) -> i32 {
    let force = spirit.get_i32("force").unwrap_or(1);
    if spirit.get("type") == "Sprite" {
        force
    } else {
        force * settings_karma(engine, ch, "karmaspiritfettering", KARMA_SPIRIT_FETTERING)
    }
}

/// Fetter or release a spirit or sprite (the `Spirit.Fettered` setter in
/// career mode). Fettering pays [`fettering_karma_cost`] and returns the
/// expense guid; releasing costs and refunds nothing, and returns `None`.
// LIKELY-BUG(LB-01): undoing the SpiritFettering expense (career/undo.rs) refunds the karma but leaves the spirit fettered and the MAG -1 improvement in place. See docs/likely-bugs.md.
pub fn set_spirit_fettered(ch: &mut Character, engine: &Engine, spirit_guid: &str, fettered: bool) -> Result<Option<String>, CareerError> {
    require_career(ch)?;
    let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid").eq_ignore_ascii_case(spirit_guid)).cloned().ok_or_else(|| CareerError::NotFound(format!("spirit {spirit_guid}")))?;
    if s.get_bool("fettered").unwrap_or(false) == fettered {
        return Ok(None);
    }
    if !fettered {
        spirit::check_release(ch, &s).map_err(CareerError::Refused)?;
        spirit::set_fettered(ch, spirit_guid, false).map_err(CareerError::Refused)?;
        return Ok(None);
    }
    spirit::check_fetter(ch, &s).map_err(CareerError::Refused)?;
    let cost = fettering_karma_cost(engine, ch, &s);
    require_karma(ch, cost)?;
    spirit::set_fettered(ch, spirit_guid, true).map_err(CareerError::Refused)?;
    let reason = format!("Fettered a Spirit {}", s.get("name"));
    Ok(Some(book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::SpiritFettering, s.get("guid")))))
}
