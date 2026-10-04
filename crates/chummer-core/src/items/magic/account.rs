//! Magic accounting: power points, free spells and complex forms, and the
//! karma costs of spells, complex forms, initiation and foci.

use super::power;
use crate::calc::{Rules, Sheet};
use crate::character::Character;
use crate::expr::standard_round;
use crate::improvement::{Field, Query};
use crate::settings::CharacterSettings;

// ---------------------------------------------------------------------------
// Power points
// ---------------------------------------------------------------------------

/// Mystic adept: both adept and magician.
fn mystic_adept(ch: &Character) -> bool {
    ch.is_adept() && ch.is_magician()
}

/// The MAG value powers are capped by (`Power.MAGAttributeObject`):
/// MAGAdept with the second-MAG-attribute house rule for mystic adepts.
pub fn adept_mag(ch: &Character, sheet: &Sheet, second_mag_attribute: bool) -> i32 {
    if second_mag_attribute && mystic_adept(ch) { sheet.attr("MAGAdept") } else { sheet.attr("MAG") }
}

/// `Character.PowerPointsTotal` and `PowerPointsUsed` with the default
/// settings (mystic adepts buy power points: `<magsplitadept>`).
pub fn power_points(ch: &Character, sheet: &Sheet) -> (f64, f64) {
    power_points_inner(ch, sheet, false)
}

/// [`power_points`] honouring the preset's `mysadeptsecondmagattribute`.
pub fn power_points_with(ch: &Character, sheet: &Sheet, settings: &CharacterSettings) -> (f64, f64) {
    power_points_inner(ch, sheet, settings.flag("mysadeptsecondmagattribute"))
}

fn power_points_inner(ch: &Character, sheet: &Sheet, second_mag: bool) -> (f64, f64) {
    let mag = adept_mag(ch, sheet, second_mag);
    // UseMysticAdeptPPs: mystic adepts without the second MAG attribute.
    let base = if mystic_adept(ch) && !second_mag { f64::from(ch.doc.get_i32("magsplitadept").unwrap_or(0)) } else { f64::from(mag) };
    let total = (base + ch.improvements.val("AdeptPowerPoints", None)).max(0.0);
    let used = ch.items("powers", "power").iter().map(|p| power::power_point_cost(ch, p, mag)).sum();
    (total, used)
}

// ---------------------------------------------------------------------------
// Spells
// ---------------------------------------------------------------------------

/// Spells bought at creation against the free spell limit
/// (`CharacterCreate.CalculateBP`, spells section).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellCounts {
    /// Paid spells, rituals and alchemical preparations (grade 0, not free).
    pub spells: i32,
    pub rituals: i32,
    pub preparations: i32,
    /// Free spells: priority (`<spelllimit>`) plus SpellLimit, FreeSpells,
    /// FreeSpellsATT and FreeSpellsSkill improvements.
    pub free: i32,
    /// Of `free`, how many only buy touch-range spells.
    pub free_touch_only: i32,
    /// What is left after the free ones: (spells, rituals, preparations).
    pub over: (i32, i32, i32),
}

fn half_or_whole(v: i32, unique: &str) -> i32 {
    if unique.contains("half") { crate::calc::div_away_from_zero(v, 2) } else { v }
}

/// Count spells against the free limit.
pub fn spell_counts(ch: &Character, sheet: &Sheet) -> SpellCounts {
    let mut c = SpellCounts::default();
    let mut touch = 0;
    let mut by_category: Vec<String> = Vec::new();
    for s in ch.items("spells", "spell") {
        if s.get_i32("grade").unwrap_or(0) != 0 || s.get_bool("freebonus").unwrap_or(false) {
            continue;
        }
        let cat = s.get("category");
        by_category.push(cat.clone());
        if s.get_bool("alchemical").unwrap_or(false) {
            c.preparations += 1;
        } else if cat == "Rituals" {
            c.rituals += 1;
        } else {
            c.spells += 1;
            if matches!(s.get("range").as_str(), "T" | "T (A)") {
                touch += 1;
            }
        }
    }
    let mut limit = ch.improvements.val("SpellLimit", None) + ch.improvements.val("FreeSpells", None);
    let mut limit_touch = 0;
    for i in ch.improvements.of_kind("FreeSpellsATT") {
        let v = half_or_whole(sheet.attr(&i.improved_name), &i.unique_name);
        if i.unique_name.contains("touchonly") { limit_touch += v } else { limit += f64::from(v) }
    }
    for i in ch.improvements.of_kind("FreeSpellsSkill") {
        let Some(sk) = sheet.skills.iter().find(|s| s.name == i.improved_name) else { continue };
        let v = half_or_whole(sk.base + sk.karma, &i.unique_name);
        if i.unique_name.contains("touchonly") { limit_touch += v } else { limit += f64::from(v) }
        // A specialization in a spell category makes one spell of it free.
        c.spells -= sk.specs.iter().filter(|sp| by_category.contains(sp)).count() as i32;
    }
    c.spells -= touch - (touch - limit_touch).max(0);
    c.free = ch.doc.get_i32("spelllimit").unwrap_or(0) + standard_round(limit);
    c.free_touch_only = limit_touch;
    c.over = over_free(&c, 0);
    c
}

/// Spells, rituals and preparations left once the free spells are used
/// up in that order, with `extra` more spells to pay for.
fn over_free(c: &SpellCounts, extra: i32) -> (i32, i32, i32) {
    let (mut s, mut r, mut p) = (c.spells + extra, c.rituals, c.preparations);
    for _ in 0..c.free.max(0) {
        if s > 0 {
            s -= 1;
        } else if r > 0 {
            r -= 1;
        } else if p > 0 {
            p -= 1;
        } else {
            break;
        }
    }
    (s.max(0), r.max(0), p.max(0))
}

/// `Character.SpellKarmaCost(category)`: KarmaSpell plus NewSpellKarmaCost,
/// times the NewSpellKarmaCostMultiplier percentages.
pub fn spell_karma_cost(ch: &Character, rules: &Rules, category: &str) -> i32 {
    let q = |k| Query::named(k, category).with_non_improved();
    let mut cost = f64::from(rules.karma_spell) + ch.improvements.sum(q("NewSpellKarmaCost"), Field::Val);
    let mult: f64 = ch
        .improvements
        .of_kind("NewSpellKarmaCostMultiplier")
        .filter(|i| i.improved_name.is_empty() || i.improved_name == category)
        .map(|i| i.val / 100.0)
        .product();
    if mult != 1.0 {
        cost *= mult;
    }
    standard_round(cost).max(0)
}

/// Karma the spells beyond the free ones cost at creation (0 in career
/// mode, where each new spell costs [`spell_karma_cost`]).
pub fn spell_karma(ch: &Character, sheet: &Sheet, rules: &Rules) -> i32 {
    spell_karma_with_extra(ch, sheet, rules, 0)
}

/// [`spell_karma`] with `extra` spells more, e.g. mystic adept power
/// points bought with free spells (`PrioritySpellsAsAdeptPowers`).
pub fn spell_karma_with_extra(ch: &Character, sheet: &Sheet, rules: &Rules, extra: i32) -> i32 {
    if ch.created {
        return 0;
    }
    let (s, r, p) = over_free(&spell_counts(ch, sheet), extra);
    s * spell_karma_cost(ch, rules, "Spells") + r * spell_karma_cost(ch, rules, "Rituals") + p * spell_karma_cost(ch, rules, "Preparations")
}

// ---------------------------------------------------------------------------
// Complex forms
// ---------------------------------------------------------------------------

/// Complex forms bought at creation against `<cfplimit>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormCounts {
    /// Forms with grade 0 (not granted by a bonus).
    pub forms: i32,
    pub free: i32,
}

pub fn complex_form_counts(ch: &Character) -> FormCounts {
    let forms = ch.items("complexforms", "complexform").iter().filter(|f| f.get_i32("grade").unwrap_or(0) == 0).count() as i32;
    FormCounts { forms, free: ch.doc.get_i32("cfplimit").unwrap_or(0) }
}

/// `Character.ComplexFormKarmaCost`.
pub fn complex_form_karma_cost(ch: &Character, rules: &Rules) -> i32 {
    let mut cost = f64::from(rules.karma_new_complex_form) + ch.improvements.val("NewComplexFormKarmaCost", None);
    let mult: f64 = ch.improvements.of_kind("NewComplexFormKarmaCostMultiplier").map(|i| i.val / 100.0).product();
    if mult != 1.0 {
        cost *= mult;
    }
    standard_round(cost).max(0)
}

/// Karma for forms beyond the free ones at creation (0 in career mode,
/// where each new form costs [`complex_form_karma_cost`]).
pub fn complex_form_karma(ch: &Character, rules: &Rules) -> i32 {
    let per = complex_form_karma_cost(ch, rules);
    if ch.created {
        return 0;
    }
    let c = complex_form_counts(ch);
    (c.forms - c.free).max(0) * per
}

// ---------------------------------------------------------------------------
// Initiation and foci
// ---------------------------------------------------------------------------

fn percent(settings: &CharacterSettings, key: &str, default: f64) -> f64 {
    settings.raw.child("karmacost").and_then(|k| k.get_f64(key)).or_else(|| settings.raw.get_f64(key)).unwrap_or(default)
}

/// `InitiationGrade.KarmaCost`: KarmaInitiationFlat + grade × KarmaInitiation,
/// less the group/ordeal/schooling discounts.
pub fn initiation_karma(rules: &Rules, settings: &CharacterSettings, grade: i32, technomancer: bool, o: super::initiation::GradeOptions) -> i32 {
    let cost = f64::from(rules.karma_initiation_flat) + f64::from(grade) * f64::from(rules.karma_initiation);
    let kind = if technomancer { "res" } else { "mag" };
    let mut mult = 1.0;
    for (on, what, default) in [(o.group, "group", 0.1), (o.ordeal, "ordeal", if technomancer { 0.2 } else { 0.1 }), (o.schooling, "schooling", 0.1)] {
        if on {
            mult -= percent(settings, &format!("karma{kind}initiation{what}percent"), default);
        }
    }
    standard_round(cost * mult)
}

/// Settings key of the karma multiplier for a focus (`Focus.BindingKarmaCost`).
fn focus_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "Qi Focus" => "karmaqifocus",
        "Sustaining Focus" => "karmasustainingfocus",
        "Counterspelling Focus" => "karmacounterspellingfocus",
        "Banishing Focus" => "karmabanishingfocus",
        "Binding Focus" => "karmabindingfocus",
        "Weapon Focus" => "karmaweaponfocus",
        "Spellcasting Focus" => "karmaspellcastingfocus",
        "Ritual Spellcasting Focus" => "karmaritualspellcastingfocus",
        "Spell Shaping Focus" => "karmaspellshapingfocus",
        "Summoning Focus" => "karmasummoningfocus",
        "Alchemical Focus" => "karmaalchemicalfocus",
        "Centering Focus" => "karmacenteringfocus",
        "Masking Focus" => "karmamaskingfocus",
        "Disenchanting Focus" => "karmadisenchantingfocus",
        "Power Focus" => "karmapowerfocus",
        "Flexible Signature Focus" => "karmaflexiblesignaturefocus",
        _ => return None,
    })
}

/// `Focus.BindingKarmaCost` for a focus gear item: Force × the focus
/// multiplier, plus FocusBindingKarmaCost/Multiplier improvements.
pub fn focus_binding_karma(ch: &Character, settings: &CharacterSettings, gear: &crate::xml::Element) -> i32 {
    let mut name = gear.get("name");
    let extra = gear.get("extra");
    let mut extra_cost = 0.0;
    for (suffix, d) in [(", Individualized, Complete", -2.0), (", Individualized, Partial", -1.0)] {
        if let Some(n) = name.strip_suffix(suffix) {
            name = n.to_owned();
            extra_cost = d;
            break;
        }
    }
    if let Some(p) = name.find('(') {
        name = name[..p.saturating_sub(1)].to_owned();
    }
    if let Some(p) = name.find(',') {
        name.truncate(p);
    }
    let mut mult = focus_key(&name).map_or(1.0, |k| f64::from(settings.karma(k, 1)));
    for i in ch.improvements.active().filter(|i| i.improved_name == name) {
        let target_ok = if extra.trim().is_empty() { i.target.is_empty() } else { i.target.is_empty() || i.target.contains(&extra) };
        if !target_ok {
            continue;
        }
        match i.kind.as_str() {
            "FocusBindingKarmaCost" => extra_cost += i.val,
            "FocusBindingKarmaMultiplier" => mult += i.val,
            _ => {}
        }
    }
    let rating = f64::from(gear.get_i32("rating").unwrap_or(0));
    standard_round(rating * mult + extra_cost)
}

/// Bind a focus: add a `<focus>` for a gear item (`Focus.Save`). Returns
/// the focus guid, or `None` when the gear is already bound.
pub fn bind_focus(ch: &mut Character, gear_guid: &str) -> Option<String> {
    if ch.items("foci", "focus").iter().any(|f| f.get("gearid").eq_ignore_ascii_case(gear_guid)) {
        return None;
    }
    let guid = crate::items::new_guid();
    let mut f = crate::xml::Element::new("focus");
    f.push(crate::xml::Element::with_text("guid", guid.clone()));
    f.push(crate::xml::Element::with_text("gearid", gear_guid));
    ch.items_mut("foci").push(f);
    Some(guid)
}

/// Unbind a focus by its gear's guid.
pub fn unbind_focus(ch: &mut Character, gear_guid: &str) -> bool {
    let Some(c) = ch.doc.child_mut("foci") else { return false };
    let before = c.children.len();
    c.children.retain(|n| !matches!(n, crate::xml::Node::Element(e) if e.get("gearid").eq_ignore_ascii_case(gear_guid)));
    let changed = before != c.children.len();
    ch.dirty |= changed;
    changed
}

/// Mark a focus gear item bonded or not (`Gear.Bonded`). Foci only grant
/// their bonus while bonded, so binding creates the gear's bonus
/// improvements (at its rating, with its saved selection) and unbinding
/// removes them. Weapon foci keep their bonus either way.
pub fn set_focus_bonded(ch: &mut Character, store: &crate::data::DataStore, gear_guid: &str, bonded: bool) -> bool {
    let Some(g) = crate::items::find_by_guid_mut(ch.items_mut("gears"), gear_guid) else { return false };
    g.set_child_text("bonded", crate::improvement::bool_str(bonded));
    let gear = g.clone();
    let bonus = gear.child("bonus").filter(|b| b.elements().next().is_some() && b.child("selectweapon").is_none());
    if let Some(b) = bonus {
        ch.improvements.remove_from_source(gear_guid);
        if bonded {
            let src = crate::bonus::BonusSource { kind: "Gear".into(), guid: gear.get("guid"), name: gear.get("name"), rating: gear.get_i32("rating").unwrap_or(1) };
            let extra = gear.get("extra");
            let out = crate::bonus::apply(ch, store, b, &src, Some(extra.as_str()).filter(|s| !s.is_empty()));
            crate::items::apply_outcome(ch, &out);
        }
    }
    ch.dirty = true;
    true
}
