//! Spending karma in career mode. Each purchase logs a negative karma entry
//! whose undo names what was bought.

use super::ledger::{book_karma, push_entry, ExpenseEntry, ExpenseType, ExpenseUndo, KarmaExpenseType, NuyenExpenseType};
use super::{require_career, require_karma, CareerError, CareerRules};
use crate::calc::{self, Sheet, SkillValues};
use crate::character::Character;
use crate::chargen;
use crate::data::Record;
use crate::engine::Engine;
use crate::expr::standard_round;
use crate::improvement::{bool_str, Field, Improvement, Query};
use crate::items::{self, Purchase};
use crate::requirements::{self, Check};
use crate::skills::Specialization;
use crate::xml::Element;

fn sheet(engine: &Engine, ch: &Character, cr: &CareerRules) -> Sheet {
    calc::compute(ch, &cr.rules, Some(&engine.store), Some(&engine.catalog))
}

// ---------------------------------------------------------------------------
// Cost modifiers (the `*KarmaCost` / `*KarmaCostMultiplier` improvements)
// ---------------------------------------------------------------------------

/// Extra karma and multiplier from cost improvements, as every
/// `UpgradeKarmaCost` sums them.
#[derive(Debug, Clone, Copy)]
struct CostMods {
    extra: f64,
    mult: f64,
}

impl CostMods {
    fn new() -> Self {
        CostMods { extra: 0.0, mult: 1.0 }
    }

    /// The improvement applies to the rating being bought: `min <= next`
    /// and `max == 0 || next <= max`.
    fn in_window(i: &Improvement, next: i32, check_max: bool) -> bool {
        let next = f64::from(next);
        i.min <= next && (!check_max || i.max == 0.0 || next <= i.max)
    }

    /// Add `extra_kind` values and `mult_kind` percentages for `name`
    /// (`GetCachedImprovementListForValueOf(.., blnIncludeNonImproved: true)`).
    fn add(mut self, ch: &Character, extra_kind: &str, mult_kind: &str, name: &str, next: i32, check_max: bool) -> Self {
        let imps = &ch.improvements;
        for i in imps.winners(Query::named(extra_kind, name).with_non_improved(), Field::Val) {
            if Self::in_window(i, next, check_max) {
                self.extra += i.val;
            }
        }
        for i in imps.winners(Query::named(mult_kind, name).with_non_improved(), Field::Val) {
            if Self::in_window(i, next, check_max) {
                self.mult *= i.val / 100.0;
            }
        }
        self
    }

    fn apply(self, cost: i32) -> i32 {
        if self.mult != 1.0 {
            standard_round(f64::from(cost) * self.mult + self.extra)
        } else {
            cost + standard_round(self.extra)
        }
    }
}

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

/// Attributes whose cost ignores `AlternateMetatypeAttributeKarma`.
const ALTERNATE_KARMA_EXCEPTIONS: &[&str] = &["MAG", "RES", "DEP", "MAGAdept"];

/// `CharacterAttrib.UpgradeKarmaCost`: karma to raise an attribute by one,
/// with cost improvements. `None` at the maximum.
pub fn attribute_cost(ch: &Character, cr: &CareerRules, abbrev: &str) -> Option<i32> {
    let v = calc::attribute_values(ch, abbrev, &cr.rules);
    if ch.attribute(abbrev).is_none() || v.value >= v.total_max {
        return None;
    }
    let k = cr.rules.karma_attribute;
    let mut cost = if v.value == 0 { k } else { (v.value + 1) * k };
    if cr.alternate_metatype_attribute_karma && !ALTERNATE_KARMA_EXCEPTIONS.contains(&abbrev) {
        cost -= (v.metatype_min - 1) * k;
    }
    let cost = CostMods::new().add(ch, "AttributeKarmaCost", "AttributeKarmaCostMultiplier", abbrev, v.value + 1, true).apply(cost);
    Some(cost.max(k.min(1)))
}

/// [`attribute_cost`] with the character's own settings.
pub fn attribute_upgrade_karma_cost(engine: &Engine, ch: &Character, abbrev: &str) -> Option<i32> {
    attribute_cost(ch, &CareerRules::for_character(engine, ch), abbrev)
}

/// Raise an attribute by one point of karma (`CharacterAttrib.Upgrade`).
/// Returns the expense guid.
pub fn improve_attribute(ch: &mut Character, engine: &Engine, abbrev: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let cost = attribute_cost(ch, &cr, abbrev).ok_or_else(|| CareerError::AtMaximum(abbrev.into()))?;
    require_karma(ch, cost)?;
    let value = calc::attribute_values(ch, abbrev, &cr.rules).value;
    let a = ch.attribute_mut(abbrev).ok_or_else(|| CareerError::NotFound(abbrev.into()))?;
    a.karma += 1;
    let reason = format!("Attribute {abbrev} {value} -> {}", value + 1);
    Ok(book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::ImproveAttribute, abbrev)))
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

/// Rating bonuses that count as learned rating (`RatingModifiers`).
fn rating_mods(ch: &Character, key: &str) -> i32 {
    standard_round(ch.improvements.of_kind("Skill").filter(|i| i.add_to_rating && i.improved_name == key).map(|i| i.val).sum())
}

/// `Skill.TotalBaseRating`: learned rating without hardwires.
fn total_base_rating(ch: &Character, v: &SkillValues) -> i32 {
    v.base + v.karma + rating_mods(ch, &v.name)
}

/// What the cost formulas need to know about one skill.
#[derive(Debug, Clone)]
struct SkillInfo<'a> {
    v: &'a SkillValues,
    tbr: i32,
    max: i32,
}

fn skill_info<'a>(ch: &Character, cr: &CareerRules, sheet: &'a Sheet, guid: &str) -> Option<SkillInfo<'a>> {
    let v = sheet.skills.iter().chain(sheet.knowledge_skills.iter()).find(|v| v.guid.eq_ignore_ascii_case(guid))?;
    let max = if v.knowledge {
        cr.max_knowledge_skill_rating
    } else {
        cr.max_skill_rating + ch.improvements.of_kind("Skill").filter(|i| i.improved_name == v.name).map(|i| i.max as i32).sum::<i32>()
    };
    Some(SkillInfo { v, tbr: total_base_rating(ch, v), max })
}

/// The skill-group adjustment of `CompensateSkillGroupKarmaDifference`.
fn group_compensation(ch: &Character, cr: &CareerRules, sheet: &Sheet, s: &SkillInfo<'_>) -> i32 {
    if !cr.compensate_skill_group_karma_difference || s.v.group.is_empty() {
        return 0;
    }
    let members: Vec<&SkillValues> = sheet.skills.iter().filter(|m| m.group == s.v.group).collect();
    let upper = members.iter().filter(|m| m.guid != s.v.guid && !m.disabled).map(|m| total_base_rating(ch, m)).min();
    if upper.is_none_or(|u| u <= s.tbr) {
        return 0;
    }
    let naked = members.iter().filter(|m| m.guid == s.v.guid || !m.disabled).count() as i32;
    let r = &cr.rules;
    let (group_cost, naked_cost) = if s.tbr == 0 {
        (r.karma_new_skill_group, naked * r.karma_new_active_skill)
    } else {
        ((s.tbr + 1) * r.karma_improve_skill_group, naked * (s.tbr + 1) * r.karma_improve_active_skill)
    };
    group_cost - naked_cost
}

/// `Skill.UpgradeKarmaCost` for an active skill.
fn active_skill_cost(ch: &Character, cr: &CareerRules, sheet: &Sheet, s: &SkillInfo<'_>) -> Option<i32> {
    if s.tbr >= s.max {
        return None;
    }
    let r = &cr.rules;
    let next = s.tbr + 1;
    let (mut cost, option) = if s.tbr == 0 {
        (r.karma_new_active_skill, r.karma_new_active_skill)
    } else {
        (next * r.karma_improve_active_skill, r.karma_improve_active_skill)
    };
    let adjust = group_compensation(ch, cr, sheet, s);
    cost += adjust;
    let cost = CostMods::new()
        .add(ch, "ActiveSkillKarmaCost", "ActiveSkillKarmaCostMultiplier", &s.v.name, next, true)
        .add(ch, "SkillCategoryKarmaCost", "SkillCategoryKarmaCostMultiplier", &s.v.category, next, true)
        .apply(cost);
    Some(cost.max(option.min(1) + adjust))
}

/// `KnowledgeSkill.UpgradeKarmaCost`.
fn knowledge_skill_cost(ch: &Character, cr: &CareerRules, s: &SkillInfo<'_>) -> Option<i32> {
    if s.tbr >= s.max {
        return None;
    }
    let r = &cr.rules;
    let next = s.tbr + 1;
    let (cost, option) = if s.tbr == 0 {
        (r.karma_new_knowledge_skill, r.karma_new_knowledge_skill)
    } else {
        (next * r.karma_improve_knowledge_skill, r.karma_improve_knowledge_skill)
    };
    let imps = &ch.improvements;
    let min_override = [(s.v.name.as_str(), true), (s.v.category.as_str(), false)]
        .into_iter()
        .flat_map(|(name, non_improved)| {
            let q = Query::named("KnowledgeSkillKarmaCostMinimum", name);
            imps.winners(if non_improved { q.with_non_improved() } else { q }, Field::Val)
        })
        .filter(|i| CostMods::in_window(i, next, true))
        .map(|i| standard_round(i.val))
        .min();
    let cost = CostMods::new()
        .add(ch, "KnowledgeSkillKarmaCost", "KnowledgeSkillKarmaCostMultiplier", &s.v.name, next, true)
        .add(ch, "SkillCategoryKarmaCost", "SkillCategoryKarmaCostMultiplier", &s.v.category, next, true)
        .apply(cost);
    Some(cost.max(min_override.unwrap_or(option.min(1))))
}

fn skill_cost(ch: &Character, cr: &CareerRules, sheet: &Sheet, s: &SkillInfo<'_>) -> Option<i32> {
    if s.v.knowledge {
        knowledge_skill_cost(ch, cr, s)
    } else {
        active_skill_cost(ch, cr, sheet, s)
    }
}

/// Karma to raise an active or knowledge skill by one (`UpgradeKarmaCost`).
/// `None` at the maximum or for an unknown guid.
pub fn skill_upgrade_karma_cost(engine: &Engine, ch: &Character, skill_guid: &str) -> Option<i32> {
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let s = skill_info(ch, &cr, &sheet, skill_guid)?;
    skill_cost(ch, &cr, &sheet, &s)
}

/// Add one karma rating to a skill by guid. Returns false if not found.
fn bump_skill_karma(ch: &mut Character, guid: &str, delta: i32) -> bool {
    if let Some(s) = ch.skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(guid)) {
        s.karma = (s.karma + delta).max(0);
    } else if let Some(k) = ch.knowledge_skills.iter_mut().find(|k| k.guid.eq_ignore_ascii_case(guid)) {
        k.karma = (k.karma + delta).max(0);
    } else {
        return false;
    }
    ch.dirty = true;
    true
}

/// Raise an active or knowledge skill by one (`Skill.Upgrade`). The undo is
/// `AddSkill` when raising from 0, `ImproveSkill` otherwise. Returns the
/// expense guid.
pub fn improve_skill(ch: &mut Character, engine: &Engine, skill_guid: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let s = skill_info(ch, &cr, &sheet, skill_guid).ok_or_else(|| CareerError::NotFound(format!("skill {skill_guid}")))?;
    let cost = skill_cost(ch, &cr, &sheet, &s).ok_or_else(|| CareerError::AtMaximum(s.v.name.clone()))?;
    require_karma(ch, cost)?;
    let label = if s.v.knowledge { "Knowledge Skill" } else { "Active Skill" };
    let reason = format!("{label} {} {} -> {}", s.v.name, s.tbr, s.tbr + 1);
    let kind = if s.tbr == 0 { KarmaExpenseType::AddSkill } else { KarmaExpenseType::ImproveSkill };
    let guid = s.v.guid.clone();
    bump_skill_karma(ch, &guid, 1);
    Ok(book_karma(ch, -cost, reason, ExpenseUndo::karma(kind, guid)))
}

/// Add a knowledge skill and buy its first rating (the career "Add" button
/// adds it free at 0, then `Upgrade`). Returns the skill's guid.
pub fn learn_knowledge_skill(ch: &mut Character, engine: &Engine, name: &str, kind: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    chargen::add_knowledge_skill(ch, name, kind, false);
    let guid = ch.knowledge_skills.last().map(|k| k.guid.clone()).unwrap_or_default();
    if let Err(e) = improve_skill(ch, engine, &guid) {
        chargen::remove_knowledge_skill(ch, &guid);
        return Err(e);
    }
    Ok(guid)
}

// ---------------------------------------------------------------------------
// Skill groups
// ---------------------------------------------------------------------------

fn group_members<'a>(sheet: &'a Sheet, name: &str) -> Vec<&'a SkillValues> {
    sheet.skills.iter().filter(|v| !name.is_empty() && v.group == name).collect()
}

/// `SkillGroup.HasAnyBreakingSkills`: enabled members differ in learned
/// rating, or (with `SpecializationsBreakSkillGroups`) one has a specialization.
fn group_broken(ch: &Character, cr: &CareerRules, members: &[&SkillValues]) -> bool {
    let enabled: Vec<&&SkillValues> = members.iter().filter(|m| !m.disabled).collect();
    if enabled.len() <= 1 {
        return false;
    }
    if cr.specializations_break_skill_groups && enabled.iter().any(|m| !m.specs.is_empty()) {
        return true;
    }
    let first = total_base_rating(ch, enabled[0]);
    enabled.iter().any(|m| total_base_rating(ch, m) != first)
}

/// `SkillGroup.UpgradeKarmaCost`. The rating it prices is the lowest
/// learned rating of its enabled skills.
fn group_cost(ch: &Character, cr: &CareerRules, members: &[&SkillValues], name: &str) -> Option<i32> {
    let enabled: Vec<&&SkillValues> = members.iter().filter(|m| !m.disabled).collect();
    if enabled.is_empty() {
        return None;
    }
    let rating = enabled.iter().map(|m| total_base_rating(ch, m)).min().unwrap_or(0);
    let r = &cr.rules;
    let (cost, option) = if rating == 0 {
        (r.karma_new_skill_group, r.karma_new_skill_group)
    } else if cr.max_skill_rating > rating {
        ((rating + 1) * r.karma_improve_skill_group, r.karma_improve_skill_group)
    } else {
        return None;
    };
    let next = rating + 1;
    let mut mods = CostMods::new().add(ch, "SkillGroupKarmaCost", "SkillGroupKarmaCostMultiplier", name, next, true);
    let categories: Vec<&str> = enabled.iter().map(|m| m.category.as_str()).collect();
    for i in ch.improvements.active().filter(|i| categories.contains(&i.improved_name.as_str()) && CostMods::in_window(i, next, true)) {
        match i.kind.as_str() {
            "SkillGroupCategoryKarmaCost" => mods.extra += i.val,
            "SkillGroupCategoryKarmaCostMultiplier" => mods.mult *= i.val / 100.0,
            _ => {}
        }
    }
    Some(mods.apply(cost).max(option.min(1)))
}

/// Karma to raise a skill group by one. `None` if it cannot be raised.
pub fn skill_group_upgrade_karma_cost(engine: &Engine, ch: &Character, group_name: &str) -> Option<i32> {
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let members = group_members(&sheet, group_name);
    if group_broken(ch, &cr, &members) {
        return None;
    }
    group_cost(ch, &cr, &members, group_name)
}

/// Raise a skill group by one (`SkillGroup.Upgrade`). Refused while the
/// group is broken. Returns the expense guid.
pub fn improve_skill_group(ch: &mut Character, engine: &Engine, group_name: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let members = group_members(&sheet, group_name);
    if group_broken(ch, &cr, &members) {
        return Err(CareerError::Refused(format!("the {group_name} skill group is broken")));
    }
    let cost = group_cost(ch, &cr, &members, group_name).ok_or_else(|| CareerError::AtMaximum(group_name.into()))?;
    require_karma(ch, cost)?;
    let g = ch.skill_groups.iter_mut().find(|g| g.name == group_name).ok_or_else(|| CareerError::NotFound(group_name.into()))?;
    let rating = g.rating();
    g.karma += 1;
    let id = if g.id.is_empty() { g.name.clone() } else { g.id.clone() };
    let reason = format!("Skill Group {group_name} {rating} -> {}", rating + 1);
    Ok(book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::ImproveSkillGroup, id)))
}

/// `SkillGroup.KarmaUnbroken` in career mode: every skill's own rating
/// reaches the highest bought with points, so the group's karma can drop.
pub(super) fn group_karma_unbroken(ch: &Character, engine: &Engine, group_name: &str) -> bool {
    let members: Vec<&crate::skills::Skill> =
        ch.skills.iter().filter(|s| engine.catalog.get(&s.suid).is_some_and(|d| d.group == group_name)).collect();
    if members.is_empty() {
        return false;
    }
    let imps = &ch.improvements;
    let free = |s: &crate::skills::Skill, kind: &str| {
        let key = engine.catalog.get(&s.suid).map(|d| d.name.clone()).unwrap_or_default();
        imps.val_int(kind, Some(&key))
    };
    let high = members.iter().map(|s| s.base + free(s, "SkillBase")).max().unwrap_or(0);
    members.iter().all(|s| s.base + free(s, "SkillBase") + s.karma + free(s, "SkillLevel") >= high)
}

// ---------------------------------------------------------------------------
// Specializations
// ---------------------------------------------------------------------------

/// Karma for a new specialization in career mode (`Skill.AddSpecialization`).
/// `None` when the skill cannot have one (rating 0).
pub fn specialization_karma_cost(engine: &Engine, ch: &Character, skill_guid: &str) -> Option<i32> {
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let s = skill_info(ch, &cr, &sheet, skill_guid)?;
    spec_cost(ch, &cr, &s)
}

fn spec_cost(ch: &Character, cr: &CareerRules, s: &SkillInfo<'_>) -> Option<i32> {
    if s.tbr <= 0 || s.v.native {
        return None;
    }
    let price = if s.v.knowledge { cr.rules.karma_knowledge_specialization } else { cr.rules.karma_specialization };
    let mods = CostMods::new().add(
        ch,
        "SkillCategorySpecializationKarmaCost",
        "SkillCategorySpecializationKarmaCostMultiplier",
        &s.v.category,
        s.tbr,
        false,
    );
    Some(mods.apply(price))
}

/// Buy a specialization for an active or knowledge skill. Returns the new
/// specialization's guid (the undo's object id).
pub fn buy_specialization(ch: &mut Character, engine: &Engine, skill_guid: &str, spec: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let sheet = sheet(engine, ch, &cr);
    let s = skill_info(ch, &cr, &sheet, skill_guid).ok_or_else(|| CareerError::NotFound(format!("skill {skill_guid}")))?;
    let cost = spec_cost(ch, &cr, &s).ok_or_else(|| CareerError::Refused(format!("{} cannot have a specialization", s.v.name)))?;
    require_karma(ch, cost)?;
    let reason = format!("Learned Specialization {} ({spec})", s.v.name);
    let sp = Specialization { guid: items::new_guid(), name: spec.to_owned(), free: false, expertise: false };
    let spec_guid = sp.guid.clone();
    if let Some(sk) = ch.skills.iter_mut().find(|x| x.guid.eq_ignore_ascii_case(skill_guid)) {
        sk.specs.push(sp);
    } else if let Some(k) = ch.knowledge_skills.iter_mut().find(|x| x.guid.eq_ignore_ascii_case(skill_guid)) {
        k.specs.push(sp);
    }
    book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::AddSpecialization, spec_guid.clone()));
    Ok(spec_guid)
}

// ---------------------------------------------------------------------------
// Qualities
// ---------------------------------------------------------------------------

fn is_negative(rec: Record<'_>) -> bool {
    rec.category() == "Negative"
}

/// The record's karma with its `<costdiscount>` when that applies.
fn quality_bp(engine: &Engine, ch: &Character, cr: &CareerRules, rec: Record<'_>) -> i32 {
    let mut bp = rec.el().get_i32("karma").unwrap_or(0);
    if let Some(d) = rec.el().child("costdiscount") {
        let sheet = sheet(engine, ch, cr);
        if requirements::unmet(d, &Check { ch, sheet: &sheet, ignore_quality: None }).is_empty() {
            let v = d.get_i32("value").unwrap_or(0);
            bp += if is_negative(rec) { -v } else { v };
        }
    }
    bp
}

/// The Beast's Way and the Spiritual Way include the Mentor Spirit.
fn quality_is_free(ch: &Character, rec: Record<'_>) -> bool {
    rec.name() == "Mentor Spirit"
        && ch.items("qualities", "quality").iter().any(|q| matches!(q.get("name").as_str(), "The Beast's Way" | "The Spiritual Way"))
}

/// Karma a quality costs in career mode (`tsQualityAdd` in
/// `CharacterCareer.cs`): its karma × `KarmaQuality`, doubled unless the
/// settings or `<doublecareer>False` say otherwise.
pub fn quality_karma_cost(engine: &Engine, ch: &Character, rec: Record<'_>) -> i32 {
    let cr = CareerRules::for_character(engine, ch);
    quality_cost(engine, ch, &cr, rec)
}

fn quality_cost(engine: &Engine, ch: &Character, cr: &CareerRules, rec: Record<'_>) -> i32 {
    if quality_is_free(ch, rec) {
        return 0;
    }
    let mut cost = quality_bp(engine, ch, cr, rec) * cr.rules.karma_quality;
    if !cr.dont_double_quality_purchases && rec.el().get("doublecareer") != "False" {
        cost *= 2;
    }
    cost
}

/// Add a quality in career mode. A positive quality costs its career karma
/// (unless it does not contribute to BP); a negative one is free and logs a
/// 0 karma entry. Returns the quality's guid.
pub fn add_quality(ch: &mut Character, engine: &Engine, rec: Record<'_>, answer: Option<&str>) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let cost = quality_cost(engine, ch, &cr, rec);
    let negative = is_negative(rec);
    let pays = !negative && rec.el().get_bool("contributetobp").unwrap_or(true);
    if pays && rec.el().get("stagedpurchase") != "True" {
        require_karma(ch, cost)?;
    }
    let guid = chargen::add_quality(ch, &engine.store, rec, answer);
    let undo = ExpenseUndo::karma(KarmaExpenseType::AddQuality, guid.clone());
    if negative {
        book_karma(ch, 0, format!("Gained Negative Quality {}", rec.name()), undo);
    } else if pays {
        book_karma(ch, -cost, format!("Gained Positive Quality {}", rec.name()), undo);
    }
    Ok(guid)
}

/// Remove a quality in career mode (`RemoveQuality` in `CharacterCareer.cs`).
/// Buying off a negative quality costs −karma × `KarmaQuality` (doubled
/// unless `DontDoubleQualityRefunds`); a positive quality with
/// `<refundkarmaonremove>` refunds its cost. Returns the expense guid when
/// one was logged.
pub fn remove_quality(ch: &mut Character, engine: &Engine, quality_guid: &str) -> Result<Option<String>, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let q = ch
        .items("qualities", "quality")
        .into_iter()
        .find(|q| q.get("guid").eq_ignore_ascii_case(quality_guid))
        .cloned()
        .ok_or_else(|| CareerError::NotFound(format!("quality {quality_guid}")))?;
    check_removable(ch, &q)?;
    let name = q.get("name");
    let bp = q.get_i32("bp").unwrap_or(0);
    let double = q.get_bool("doublecareer").unwrap_or(true);
    let mut undo = ExpenseUndo::karma(KarmaExpenseType::RemoveQuality, quality_source_id(engine, &q));
    undo.extra = q.get("extra");
    let expense = if q.get("qualitytype") == "Negative" {
        let mut cost = -(bp * cr.rules.karma_quality);
        if !cr.dont_double_quality_refunds {
            cost *= 2;
        }
        require_karma(ch, cost)?;
        Some(book_karma(ch, -cost, format!("Removed Negative Quality {name}"), undo))
    } else if refunds_on_remove(engine, &q) {
        let mut refund = bp * cr.rules.karma_quality;
        if !cr.dont_double_quality_purchases && double {
            refund *= 2;
        }
        let mut entry = ExpenseEntry::new(f64::from(refund), format!("Swapped Positive Quality {name} for Karma"), ExpenseType::Karma).with_undo(undo);
        entry.refund = true;
        ch.karma += refund;
        Some(push_entry(ch, &entry))
    } else {
        None
    };
    chargen::remove_quality(ch, quality_guid);
    Ok(expense)
}

/// Metatype qualities and those granted by an improvement stay.
fn check_removable(ch: &Character, q: &Element) -> Result<(), CareerError> {
    let guid = q.get("guid");
    match q.get("qualitysource").as_str() {
        "Metatype" => Err(CareerError::Refused(format!("{} comes from the metatype", q.get("name")))),
        "Improvement" | "QualityLevelImprovement"
            if ch.improvements.list.iter().any(|i| i.kind == "SpecificQuality" && i.improved_name.eq_ignore_ascii_case(&guid)) =>
        {
            Err(CareerError::Refused(format!("{} was granted by another item", q.get("name"))))
        }
        _ => Ok(()),
    }
}

/// The data key of a saved quality: `<sourceid>`, `<id>` in pre-5.214
/// saves, or the record found by name (`Quality.Load`).
fn quality_key(q: &Element) -> String {
    ["sourceid", "id"].iter().filter_map(|k| q.child_text(k)).find(|s| !s.trim().is_empty()).unwrap_or_else(|| q.get("name"))
}

/// `Quality.SourceIDString`, looked up by name for old saves.
fn quality_source_id(engine: &Engine, q: &Element) -> String {
    let key = quality_key(q);
    let Ok(doc) = engine.store.doc("qualities.xml") else { return key };
    crate::data::find(&doc, "qualities", "quality", &key).map(|r| r.id()).unwrap_or(key)
}

fn refunds_on_remove(engine: &Engine, q: &Element) -> bool {
    let Ok(doc) = engine.store.doc("qualities.xml") else { return false };
    crate::data::find(&doc, "qualities", "quality", &quality_key(q)).is_some_and(|r| r.el().child("refundkarmaonremove").is_some())
}

// ---------------------------------------------------------------------------
// Spells and complex forms
// ---------------------------------------------------------------------------

/// `Character.SpellKarmaCost(category)`: `KarmaSpell` plus
/// `NewSpellKarmaCost` improvements, times the multipliers.
pub fn spell_karma_cost(engine: &Engine, ch: &Character, category: &str) -> i32 {
    let cr = CareerRules::for_character(engine, ch);
    spell_cost(ch, &cr, category)
}

fn spell_cost(ch: &Character, cr: &CareerRules, category: &str) -> i32 {
    let imps = &ch.improvements;
    let mut cost = f64::from(cr.rules.karma_spell) + imps.sum(Query::named("NewSpellKarmaCost", category).with_non_improved(), Field::Val);
    let mult: f64 = imps
        .winners(Query::named("NewSpellKarmaCostMultiplier", category).with_non_improved(), Field::Val)
        .iter()
        .map(|i| i.val / 100.0)
        .product();
    if mult != 1.0 {
        cost *= mult;
    }
    standard_round(cost).max(0)
}

/// `Character.ComplexFormKarmaCost`.
pub fn complex_form_karma_cost(engine: &Engine, ch: &Character) -> i32 {
    let cr = CareerRules::for_character(engine, ch);
    complex_form_cost(ch, &cr)
}

fn complex_form_cost(ch: &Character, cr: &CareerRules) -> i32 {
    let imps = &ch.improvements;
    let mut cost = f64::from(cr.rules.karma_new_complex_form) + imps.val("NewComplexFormKarmaCost", None);
    let mult: f64 = imps.winners(Query::new("NewComplexFormKarmaCostMultiplier"), Field::Val).iter().map(|i| i.val / 100.0).product();
    if mult != 1.0 {
        cost *= mult;
    }
    standard_round(cost).max(0)
}

/// The cost category of a spell: alchemical preparations and rituals have
/// their own (`tsAddSpell` in `CharacterCareer.cs`).
fn spell_category(spell: &Element) -> &'static str {
    if spell.get_bool("alchemical").unwrap_or(false) {
        "Preparations"
    } else if spell.get("category") == "Rituals" {
        "Rituals"
    } else {
        "Spells"
    }
}

fn find_item<'a>(ch: &'a Character, container: &str, tag: &'a str, guid: &str) -> Option<&'a Element> {
    ch.items(container, tag).into_iter().find(|e| e.get("guid").eq_ignore_ascii_case(guid))
}

/// Pay karma for a spell already on the character (bonus spells set
/// `<freebonus>` and cost nothing). Returns the expense guid, or `None`
/// when free.
pub fn pay_for_spell(ch: &mut Character, engine: &Engine, spell_guid: &str) -> Result<Option<String>, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let spell = find_item(ch, "spells", "spell", spell_guid).ok_or_else(|| CareerError::NotFound(format!("spell {spell_guid}")))?;
    if spell.get_bool("freebonus").unwrap_or(false) {
        return Ok(None);
    }
    let cost = spell_cost(ch, &cr, spell_category(spell));
    let reason = format!("Learned Spell {}", spell.get("name"));
    require_karma(ch, cost)?;
    Ok(Some(book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::AddSpell, spell_guid))))
}

/// Add a spell from data and pay for it. Returns the spell's guid.
pub fn learn_spell(ch: &mut Character, engine: &Engine, rec: Record<'_>, purchase: &Purchase) -> Result<String, CareerError> {
    learn_item(ch, engine, "spell", "spells", rec, purchase, |ch, cr| spell_cost(ch, cr, if rec.category() == "Rituals" { "Rituals" } else { "Spells" }), |ch, g| {
        pay_for_spell(ch, engine, g).map(|_| ())
    })
}

/// Pay karma for a complex form already on the character. Returns the
/// expense guid.
pub fn pay_for_complex_form(ch: &mut Character, engine: &Engine, form_guid: &str) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    let form = find_item(ch, "complexforms", "complexform", form_guid).ok_or_else(|| CareerError::NotFound(format!("complex form {form_guid}")))?;
    let cost = complex_form_cost(ch, &cr);
    let reason = format!("Learned Complex Form {}", form.get("name"));
    require_karma(ch, cost)?;
    Ok(book_karma(ch, -cost, reason, ExpenseUndo::karma(KarmaExpenseType::AddComplexForm, form_guid)))
}

/// Add a complex form from data and pay for it. Returns its guid.
pub fn learn_complex_form(ch: &mut Character, engine: &Engine, rec: Record<'_>, purchase: &Purchase) -> Result<String, CareerError> {
    learn_item(ch, engine, "complexform", "complexforms", rec, purchase, complex_form_cost, |ch, g| {
        pay_for_complex_form(ch, engine, g).map(|_| ())
    })
}

/// Check the cost first, add the item through [`items::add`], then pay;
/// a failed payment removes the item again.
#[allow(clippy::too_many_arguments)]
fn learn_item(
    ch: &mut Character,
    engine: &Engine,
    tag: &str,
    container: &str,
    rec: Record<'_>,
    purchase: &Purchase,
    cost: impl Fn(&Character, &CareerRules) -> i32,
    pay: impl Fn(&mut Character, &str) -> Result<(), CareerError>,
) -> Result<String, CareerError> {
    require_career(ch)?;
    let cr = CareerRules::for_character(engine, ch);
    if !purchase.free {
        require_karma(ch, cost(ch, &cr))?;
    }
    let guid = items::add(tag, ch, &engine.store, rec, purchase).map_err(CareerError::Refused)?;
    if !purchase.free {
        if let Err(e) = pay(ch, &guid) {
            ch.remove_item(container, &guid);
            return Err(e);
        }
    }
    Ok(guid)
}

// ---------------------------------------------------------------------------
// Initiation and submersion
// ---------------------------------------------------------------------------

/// Discounts chosen when joining a grade.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InitiationOptions {
    pub group: bool,
    pub ordeal: bool,
    pub schooling: bool,
}

/// Nuyen that initiation schooling costs.
pub const SCHOOLING_NUYEN: f64 = 10_000.0;

/// Technomancers submerge; everyone else with magic initiates.
fn submerges(ch: &Character) -> bool {
    !ch.mag_enabled() && ch.res_enabled()
}

/// Initiate grade (non-technomancer grades) or submersion grade.
pub fn grade_count(ch: &Character, technomancer: bool) -> i32 {
    ch.items("initiationgrades", "initiationgrade").iter().filter(|g| g.get_bool("res").unwrap_or(false) == technomancer).count() as i32
}

/// `InitiationGrade.KarmaCost`: `(flat + grade × KarmaInitiation) ×
/// (1 − discounts)`, rounded up.
pub fn grade_karma_cost(cr: &CareerRules, grade: i32, technomancer: bool, o: InitiationOptions) -> i32 {
    let p = if technomancer { cr.res_initiation_percent } else { cr.mag_initiation_percent };
    let mut mult = 1.0;
    for (on, pct) in [o.group, o.ordeal, o.schooling].into_iter().zip(p) {
        if on {
            mult -= pct;
        }
    }
    let cost = f64::from(cr.rules.karma_initiation_flat + grade * cr.rules.karma_initiation) * mult;
    // Round in decimal steps so 13 × 0.8 is 10.4, not 10.400000000000002.
    standard_round((cost * 1e6).round() / 1e6)
}

/// Karma for the character's next initiation or submersion grade.
pub fn initiation_karma_cost(engine: &Engine, ch: &Character, o: InitiationOptions) -> i32 {
    let cr = CareerRules::for_character(engine, ch);
    let tech = submerges(ch);
    grade_karma_cost(&cr, grade_count(ch, tech) + 1, tech, o)
}

/// Join the next initiation (or submersion) grade (`cmdAddMetamagic_Click`).
/// Schooling also costs 10,000¥ for initiates, logged as its own entry.
/// Returns the grade's guid.
pub fn add_initiation_grade(ch: &mut Character, engine: &Engine, o: InitiationOptions) -> Result<String, CareerError> {
    require_career(ch)?;
    let tech = submerges(ch);
    if !tech && !ch.mag_enabled() {
        return Err(CareerError::Refused("only magicians, adepts and technomancers can initiate".into()));
    }
    let cr = CareerRules::for_character(engine, ch);
    let grade = grade_count(ch, tech);
    // The grade cannot pass RES, or MAG (and MAGAdept for a mystic adept
    // with the second-MAG house rule).
    let limits = if tech { vec![calc::attribute_values(ch, "RES", &cr.rules).total] } else { super::magic::mag_limits(engine, ch, &cr.rules) };
    if limits.iter().any(|&m| grade + 1 > m) {
        let attr = if tech { "RES" } else { "MAG" };
        return Err(CareerError::AtMaximum(format!("{attr} limits the grade to {grade}")));
    }
    let cost = grade_karma_cost(&cr, grade + 1, tech, o);
    require_karma(ch, cost)?;
    let school_nuyen = !tech && o.schooling;
    if school_nuyen {
        super::require_nuyen(ch, SCHOOLING_NUYEN)?;
    }
    let guid = items::new_guid();
    ch.items_mut("initiationgrades").push(grade_element(&guid, grade + 1, tech, o));
    sync_grades(ch);
    let label = if tech { "Submersion Grade" } else { "Initiate Grade" };
    let reason = format!("{label} {grade} -> {}", grade + 1);
    book_karma(ch, -cost, reason.clone(), ExpenseUndo::karma(KarmaExpenseType::ImproveInitiateGrade, guid.clone()));
    if school_nuyen {
        let undo = ExpenseUndo::nuyen(NuyenExpenseType::ImproveInitiateGrade, guid.clone(), SCHOOLING_NUYEN);
        super::ledger::book_nuyen(ch, -SCHOOLING_NUYEN, reason, Some(undo));
    }
    Ok(guid)
}

/// `InitiationGrade.Save`.
fn grade_element(guid: &str, grade: i32, tech: bool, o: InitiationOptions) -> Element {
    let mut e = Element::new("initiationgrade");
    e.push(Element::with_text("guid", guid));
    e.push(Element::with_text("res", bool_str(tech)));
    e.push(Element::with_text("grade", grade.to_string()));
    e.push(Element::with_text("group", bool_str(o.group)));
    e.push(Element::with_text("ordeal", bool_str(o.ordeal)));
    e.push(Element::with_text("schooling", bool_str(o.schooling)));
    e.push(Element::new("notes"));
    e
}

/// Recount the grades into `<initiategrade>`/`<submersiongrade>` and keep
/// their attribute-maximum improvements in step (the `InitiateGrade` and
/// `SubmersionGrade` setters).
pub(super) fn sync_grades(ch: &mut Character) {
    let init = grade_count(ch, false);
    let sub = grade_count(ch, true);
    ch.doc.set_child_text("initiategrade", init.to_string());
    ch.doc.set_child_text("submersiongrade", sub.to_string());
    set_grade_improvements(ch, "Initiation", &["MAG", "MAGAdept"], init);
    set_grade_improvements(ch, "Submersion", &["RES"], sub);
    ch.dirty = true;
}

/// One `Attribute` improvement per attribute with `max = 1` and
/// `rating = grade`, removed at grade 0.
fn set_grade_improvements(ch: &mut Character, source: &str, attrs: &[&str], grade: i32) {
    let list = &mut ch.improvements.list;
    if grade == 0 {
        list.retain(|i| i.source != source);
        return;
    }
    for a in attrs {
        match list.iter_mut().find(|i| i.source == source && i.improved_name == *a) {
            Some(i) => i.rating = grade,
            None => list.push(Improvement {
                improved_name: (*a).into(),
                kind: "Attribute".into(),
                source: source.into(),
                max: 1.0,
                rating: grade,
                enabled: true,
                ..Default::default()
            }),
        }
    }
}
