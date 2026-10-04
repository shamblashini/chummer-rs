//! Karma costs of attributes, skills, knowledge skills and skill groups
//! with their cost-modifier improvements (`Attribute.TotalKarmaCost`,
//! `Skill.RangeCost`, `Skill.CurrentKarmaCost`,
//! `KnowledgeSkill.CurrentKarmaCost`, `SkillGroup.CurrentKarmaCost`).
//!
//! Not ported: `KnowledgeSkillKarmaCostMinimum` and the
//! `CompensateSkillGroupKarmaDifference`, `AlternateMetatypeAttributeKarma`
//! and `ReverseAttributePriorityOrder` house rules.

use super::{AttributeValues, Rules, SkillValues};
use crate::character::Character;
use crate::expr::standard_round;
use crate::improvement::{Field, Improvement, Query};
use crate::skills::{SkillGroup, Specialization};

/// Improvements of `kind` for `name`, including ones with no improved name
/// (`GetCachedImprovementListForValueOf(..., blnIncludeNonImproved: true)`).
fn named<'a>(ch: &'a Character, kind: &'a str, name: &'a str) -> Vec<&'a Improvement> {
    ch.improvements.winners(Query::named(kind, name).with_non_improved(), Field::Val)
}

/// Flat extra for the levels of `lower+1..=upper` an improvement's
/// Minimum/Maximum window covers.
fn window_extra(i: &Improvement, lower: i32, upper: i32) -> f64 {
    let max = if i.max == 0.0 { i32::MAX } else { i.max as i32 };
    i.val * f64::from(upper.min(max) - lower.max(i.min as i32 - 1))
}

/// Per-level extra and multiplier from (extra kind, multiplier kind, name)
/// triples whose Minimum is at most `gate`.
fn modifiers(ch: &Character, kinds: &[(&str, &str, &str)], gate: i32, lower: i32, upper: i32) -> (f64, f64) {
    let (mut extra, mut mult) = (0.0, 1.0);
    for (extra_kind, mult_kind, name) in kinds {
        extra += named(ch, extra_kind, name).iter().filter(|i| i.min as i32 <= gate).map(|i| window_extra(i, lower, upper)).sum::<f64>();
        mult *= named(ch, mult_kind, name).iter().filter(|i| i.min as i32 <= gate).map(|i| i.val / 100.0).product::<f64>();
    }
    (extra, mult)
}

/// `cost * mult + extra` rounded, as Chummer applies cost improvements.
fn apply(cost: i32, extra: f64, mult: f64) -> i32 {
    if mult != 1.0 {
        standard_round(f64::from(cost) * mult + extra)
    } else {
        cost + standard_round(extra)
    }
}

/// `Attribute.TotalKarmaCost`: the karma levels above the total base, with
/// AttributeKarmaCost(Multiplier) improvements.
pub fn attribute(ch: &Character, v: &AttributeValues, rules: &Rules) -> i32 {
    let cost = super::attribute_karma_cost(v, rules);
    if v.karma <= 0 {
        return cost;
    }
    let kinds = [("AttributeKarmaCost", "AttributeKarmaCostMultiplier", v.name.as_str())];
    let (extra, mult) = modifiers(ch, &kinds, v.value, v.total_base, v.value);
    apply(cost, extra, mult).max(0)
}

/// Triangle cost of raising a skill from `lower` to `upper` with no
/// improvements (the first level costs `new_cost`).
pub fn skill_range_cost(lower: i32, upper: i32, new_cost: i32, improve_cost: i32) -> i32 {
    if lower >= upper {
        return 0;
    }
    let tri = (upper * (upper + 1) - lower * (lower + 1)) / 2;
    if lower == 0 {
        (tri - 1) * improve_cost + new_cost
    } else {
        tri * improve_cost
    }
}

/// What an active skill's cost depends on besides its ratings.
pub struct ActiveSkill<'a> {
    /// `DictionaryKey`: the name, with the specific for exotic skills.
    pub key: &'a str,
    pub category: &'a str,
    pub exotic: bool,
    pub buy_with_karma: bool,
    pub specs: &'a [Specialization],
}

/// `Skill.RangeCost` for an active skill, with ActiveSkillKarmaCost(Multiplier)
/// and SkillCategoryKarmaCost(Multiplier) improvements.
fn active_range_cost(ch: &Character, s: &ActiveSkill<'_>, lower: i32, upper: i32, rules: &Rules) -> i32 {
    if lower >= upper {
        return 0;
    }
    let cost = skill_range_cost(lower, upper, rules.karma_new_active_skill, rules.karma_improve_active_skill);
    let kinds = [
        ("ActiveSkillKarmaCost", "ActiveSkillKarmaCostMultiplier", s.key),
        ("SkillCategoryKarmaCost", "SkillCategoryKarmaCostMultiplier", s.category),
    ];
    let (extra, mult) = modifiers(ch, &kinds, lower, lower, upper);
    apply(cost, extra, mult)
}

/// Specialization karma with SkillCategorySpecializationKarmaCost(Multiplier).
fn spec_cost(ch: &Character, category: &str, count: i32, per: i32, total: i32) -> i32 {
    let (mut extra, mut mult) = (0.0, 1.0);
    for i in named(ch, "SkillCategorySpecializationKarmaCost", category).iter().filter(|i| i.min as i32 <= total) {
        extra += i.val * f64::from(count);
    }
    for i in named(ch, "SkillCategorySpecializationKarmaCostMultiplier", category).iter().filter(|i| i.min as i32 <= total) {
        mult *= i.val / 100.0;
    }
    apply(count * per, extra, mult)
}

/// `Skill.CurrentKarmaCost`. `group_range` is the skill group's karma
/// levels `(lower, upper)` when the group has karma; those levels are paid
/// by the group, not the skill.
pub fn active_skill(ch: &Character, s: &ActiveSkill<'_>, lower: i32, total: i32, group_range: Option<(i32, i32)>, rules: &Rules) -> i32 {
    if total == 0 {
        return 0;
    }
    let cost = match group_range {
        Some((group_lower, group_upper)) => active_range_cost(ch, s, lower, group_lower, rules) + active_range_cost(ch, s, group_upper, total, rules),
        None => active_range_cost(ch, s, lower, total, rules),
    };
    if s.exotic {
        return cost.max(0);
    }
    let priority = crate::character::uses_priority_tables(&ch.field("buildmethod"));
    let count = if s.buy_with_karma || !priority { s.specs.iter().filter(|x| !x.free).count() as i32 } else { 0 };
    (cost + spec_cost(ch, s.category, count, rules.karma_specialization, total)).max(0)
}

/// `KnowledgeSkill.CurrentKarmaCost` without specializations (Chummer
/// charges them only for `BuyWithKarma`, which the port does not model for
/// knowledge skills; they come out of knowledge points).
pub fn knowledge_skill(ch: &Character, name: &str, category: &str, lower: i32, total: i32, rules: &Rules) -> i32 {
    let improve = rules.karma_improve_knowledge_skill;
    let mut cost = f64::from((total * (total + 1) - lower * (lower + 1)) / 2 * improve);
    if lower == 0 && cost > 0.0 {
        cost += f64::from(rules.karma_new_knowledge_skill - improve);
    }
    let kinds = [
        ("KnowledgeSkillKarmaCost", "KnowledgeSkillKarmaCostMultiplier", name),
        ("SkillCategoryKarmaCost", "SkillCategoryKarmaCostMultiplier", category),
    ];
    let (extra, mult) = modifiers(ch, &kinds, total, lower, total);
    standard_round(cost * mult + extra).max(0)
}

/// SkillGroupCategoryKarmaCost(Multiplier) improvements on any category of
/// the group's skills, with SkillGroup's own create/career test.
fn group_category_modifiers(ch: &Character, categories: &[&str], lower: i32, upper: i32) -> (f64, f64) {
    let (mut extra, mut mult) = (0.0, 1.0);
    for i in &ch.improvements.list {
        let mode_ok = i.condition.is_empty() || (i.condition == "career") == ch.created || (i.condition == "create") != ch.created;
        if !i.enabled || !mode_ok || !categories.contains(&i.improved_name.as_str()) || i.min as i32 > lower {
            continue;
        }
        match i.kind.as_str() {
            "SkillGroupCategoryKarmaCost" => extra += window_extra(i, lower, upper),
            "SkillGroupCategoryKarmaCostMultiplier" => mult *= i.val / 100.0,
            _ => {}
        }
    }
    (extra, mult)
}

/// `SkillGroup.CurrentKarmaCost`: the group's karma levels below the lowest
/// enabled member's rating. A cost of exactly one level-1 triangle uses
/// KarmaNewSkillGroup (Chummer's own test).
pub fn skill_group(ch: &Character, g: &SkillGroup, members: &[&SkillValues], rules: &Rules) -> i32 {
    if g.karma == 0 {
        return 0;
    }
    let upper = members.iter().filter(|m| !m.disabled).map(|m| m.total_base).min().unwrap_or(0);
    let lower = upper - g.karma;
    let tri = (upper * (upper + 1) - lower * (lower + 1)) / 2;
    let cost = tri * if tri == 1 { rules.karma_new_skill_group } else { rules.karma_improve_skill_group };
    let kinds = [("SkillGroupKarmaCost", "SkillGroupKarmaCostMultiplier", g.name.as_str())];
    let (mut extra, mut mult) = modifiers(ch, &kinds, lower, lower, upper);
    let mut categories: Vec<&str> = members.iter().map(|m| m.category.as_str()).collect();
    categories.dedup();
    let (e, m) = group_category_modifiers(ch, &categories, lower, upper);
    extra += e;
    mult *= m;
    apply(cost, extra, mult).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(kind: &str, val: f64, min: f64, max: f64) -> Improvement {
        Improvement { kind: kind.into(), val, min, max, enabled: true, ..Default::default() }
    }

    fn character(imps: Vec<Improvement>) -> Character {
        let mut ch = Character::from_str("<character><created>False</created><buildmethod>Karma</buildmethod></character>").unwrap();
        ch.improvements.list = imps;
        ch
    }

    /// Jack of All Trades: -1 per level up to 5, +2 per level from 6, both
    /// with no improved name. Values hand-computed from `Skill.RangeCost`.
    #[test]
    fn active_skill_cost_windows() {
        let ch = character(vec![imp("ActiveSkillKarmaCost", -1.0, 0.0, 5.0), imp("ActiveSkillKarmaCost", 2.0, 6.0, 0.0)]);
        let rules = Rules::default();
        let s = ActiveSkill { key: "Pistols", category: "Combat Active", exotic: false, buy_with_karma: false, specs: &[] };
        // 3 -> 7: 22 levels x 2 = 44, -1 x (5 - 3); the +2 needs Minimum <= 3.
        assert_eq!(active_skill(&ch, &s, 3, 7, None, &rules), 42);
        // 6 -> 7: 14, -1 x (5 - 6) (Chummer's own sign quirk), +2 x (7 - 6).
        assert_eq!(active_skill(&ch, &s, 6, 7, None, &rules), 17);
    }

    #[test]
    fn category_multiplier_applies_before_extra() {
        let mut academic = imp("SkillCategoryKarmaCostMultiplier", 50.0, 0.0, 0.0);
        academic.improved_name = "Academic".into();
        let mut minus = imp("SkillCategoryKarmaCost", -1.0, 3.0, 0.0);
        minus.improved_name = "Academic".into();
        let ch = character(vec![academic, minus]);
        let rules = Rules::default();
        // 0 -> 2 at 1/1: 3 x 0.5 = 1.5, below Minimum 3 no extra -> 2.
        assert_eq!(knowledge_skill(&ch, "History", "Academic", 0, 2, &rules), 2);
        // 0 -> 4: 10 x 0.5 = 5, -1 x (4 - 2) -> 3.
        assert_eq!(knowledge_skill(&ch, "History", "Academic", 0, 4, &rules), 3);
    }

    #[test]
    fn grouped_skill_skips_group_levels() {
        let ch = character(vec![]);
        let rules = Rules::default();
        let s = ActiveSkill { key: "Con", category: "Social Active", exotic: false, buy_with_karma: false, specs: &[] };
        // Group karma covers 2 -> 3; the skill pays 1 -> 2 and 3 -> 4.
        assert_eq!(active_skill(&ch, &s, 1, 4, Some((2, 3)), &rules), 4 + 8);
    }
}
