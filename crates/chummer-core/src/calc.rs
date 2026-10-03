//! Rules math: attribute totals, essence, initiative, condition monitors,
//! limits, derived pools and skill dice pools.
//!
//! Formulas follow Chummer5a (`Attribute.Core.cs`, `Character.cs`,
//! `Skill.cs`). Comments name the C# member each function ports.

use std::collections::HashMap;

use crate::character::Character;
use crate::data::DataStore;
use crate::expr::{self, standard_round, AttributeSource};
use crate::improvement::{Field, Improvement, Query};
use crate::settings::CharacterSettings;
use crate::skills::{KnowledgeSkill, Skill};
use crate::xml::Element;

/// Integer division rounded away from zero (`DivAwayFromZero`).
pub fn div_away_from_zero(a: i32, b: i32) -> i32 {
    let q = a / b;
    if a % b != 0 && ((a < 0) == (b < 0)) {
        q + 1
    } else if a % b != 0 {
        q - 1
    } else {
        q
    }
}

/// Options the calculations read from a settings preset, with Chummer's
/// defaults so a character can be computed without one.
#[derive(Debug, Clone)]
pub struct Rules {
    pub unclamp_attribute_minimum: bool,
    pub essence_decimals: u32,
    pub dont_round_essence: bool,
    pub min_initiative_dice: i32,
    pub max_initiative_dice: i32,
    pub min_astral_initiative_dice: i32,
    pub max_astral_initiative_dice: i32,
    pub min_coldsim_dice: i32,
    pub max_coldsim_dice: i32,
    pub min_hotsim_dice: i32,
    pub max_hotsim_dice: i32,
    pub knowledge_points_expression: String,
    pub contact_points_expression: String,
    pub max_skill_rating_create: i32,
    pub max_skill_rating_career: i32,
    pub karma_attribute: i32,
    pub karma_new_active_skill: i32,
    pub karma_improve_active_skill: i32,
    pub karma_new_knowledge_skill: i32,
    pub karma_improve_knowledge_skill: i32,
    pub karma_new_skill_group: i32,
    pub karma_improve_skill_group: i32,
    pub karma_specialization: i32,
    pub karma_knowledge_specialization: i32,
    pub karma_quality: i32,
    pub karma_spell: i32,
    pub karma_contact: i32,
    pub karma_new_complex_form: i32,
    pub karma_initiation: i32,
    pub karma_initiation_flat: i32,
    pub karma_metamagic: i32,
    pub limb_count: i32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            unclamp_attribute_minimum: false,
            essence_decimals: 2,
            dont_round_essence: false,
            min_initiative_dice: 1,
            max_initiative_dice: 5,
            min_astral_initiative_dice: 3,
            max_astral_initiative_dice: 5,
            min_coldsim_dice: 3,
            max_coldsim_dice: 5,
            min_hotsim_dice: 4,
            max_hotsim_dice: 5,
            knowledge_points_expression: "({INTUnaug} + {LOGUnaug}) * 2".into(),
            contact_points_expression: "{CHAUnaug} * 3".into(),
            max_skill_rating_create: 6,
            max_skill_rating_career: 12,
            karma_attribute: 5,
            karma_new_active_skill: 2,
            karma_improve_active_skill: 2,
            karma_new_knowledge_skill: 1,
            karma_improve_knowledge_skill: 1,
            karma_new_skill_group: 5,
            karma_improve_skill_group: 5,
            karma_specialization: 7,
            karma_knowledge_specialization: 7,
            karma_quality: 1,
            karma_spell: 5,
            karma_contact: 1,
            karma_new_complex_form: 4,
            karma_initiation: 3,
            karma_initiation_flat: 10,
            karma_metamagic: 15,
            limb_count: 6,
        }
    }
}

impl Rules {
    pub fn from_settings(s: &CharacterSettings) -> Rules {
        let d = Rules::default();
        Rules {
            unclamp_attribute_minimum: s.flag("unclampattributeminimum"),
            essence_decimals: s.essence_decimals(),
            dont_round_essence: s.flag("donotroundessenceinternally"),
            min_initiative_dice: s.int("mininitiativedice", d.min_initiative_dice),
            max_initiative_dice: s.int("maxinitiativedice", d.max_initiative_dice),
            min_astral_initiative_dice: s.int("minastralinitiativedice", d.min_astral_initiative_dice),
            max_astral_initiative_dice: s.int("maxastralinitiativedice", d.max_astral_initiative_dice),
            min_coldsim_dice: s.int("mincoldsiminitiativedice", d.min_coldsim_dice),
            max_coldsim_dice: s.int("maxcoldsiminitiativedice", d.max_coldsim_dice),
            min_hotsim_dice: s.int("minhotsiminitiativedice", d.min_hotsim_dice),
            max_hotsim_dice: s.int("maxhotsiminitiativedice", d.max_hotsim_dice),
            knowledge_points_expression: s.knowledge_points_expression(),
            contact_points_expression: s.contact_points_expression(),
            max_skill_rating_create: s.int("maxskillratingcreate", d.max_skill_rating_create),
            max_skill_rating_career: d.max_skill_rating_career,
            karma_attribute: s.karma("karmaattribute", d.karma_attribute),
            karma_new_active_skill: s.karma("karmanewactiveskill", d.karma_new_active_skill),
            karma_improve_active_skill: s.karma("karmaimproveactiveskill", d.karma_improve_active_skill),
            karma_new_knowledge_skill: s.karma("karmanewknowledgeskill", d.karma_new_knowledge_skill),
            karma_improve_knowledge_skill: s.karma("karmaimproveknowledgeskill", d.karma_improve_knowledge_skill),
            karma_new_skill_group: s.karma("karmanewskillgroup", d.karma_new_skill_group),
            karma_improve_skill_group: s.karma("karmaimproveskillgroup", d.karma_improve_skill_group),
            karma_specialization: s.karma("karmaspecialization", d.karma_specialization),
            karma_knowledge_specialization: s.karma("karmaknospecialization", d.karma_knowledge_specialization),
            karma_quality: s.karma("karmaquality", d.karma_quality),
            karma_spell: s.karma("karmaspell", d.karma_spell),
            karma_contact: s.karma("karmacontact", d.karma_contact),
            karma_new_complex_form: s.karma("karmanewcomplexform", d.karma_new_complex_form),
            karma_initiation: s.karma("karmainitiation", d.karma_initiation),
            karma_initiation_flat: s.karma("karmainitiationflat", d.karma_initiation_flat),
            karma_metamagic: s.karma("karmametamagic", d.karma_metamagic),
            limb_count: s.int("limbcount", d.limb_count),
        }
    }
}

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

/// Every intermediate value of an attribute, for display and debugging.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttributeValues {
    pub name: String,
    pub metatype_min: i32,
    pub metatype_max: i32,
    pub metatype_aug_max: i32,
    pub total_min: i32,
    pub total_max: i32,
    pub total_aug_max: i32,
    pub base: i32,
    pub free_base: i32,
    pub karma: i32,
    pub total_base: i32,
    /// Natural value (`Value`).
    pub value: i32,
    /// Augmentation bonus, after the augmented-maximum clamp.
    pub augment: i32,
    pub total: i32,
}

fn is_special_zero(name: &str) -> bool {
    matches!(name, "EDG" | "MAG" | "MAGAdept" | "RES" | "DEP")
}

const CUSTOMIZATION: &[(&str, &[&str])] = &[
    ("AGI", &["Customized Agility", "Cyberlimb Customization, Agility (2050)"]),
    ("STR", &["Customized Strength", "Cyberlimb Customization, Strength (2050)"]),
];
const ENHANCEMENT: &[(&str, &[&str])] = &[
    ("AGI", &["Enhanced Agility", "Cyberlimb Augmentation, Agility (2050)"]),
    ("STR", &["Enhanced Strength", "Cyberlimb Augmentation, Strength (2050)"]),
];

fn ware_children(e: &Element) -> impl Iterator<Item = &Element> {
    e.child("children").into_iter().flat_map(|c| c.children_named("cyberware"))
}

fn is_limb(e: &Element) -> bool {
    !e.get("limbslot").trim().is_empty() || (e.get_bool("inheritattributes").unwrap_or(false) && ware_children(e).any(is_limb))
}

/// Number of limbs a character has, optionally for one slot (`LimbCount`).
pub fn limb_count(ch: &Character, rules: &Rules, slot: &str) -> i32 {
    if slot.is_empty() {
        return rules.limb_count + ch.improvements.val_int("AddLimb", None);
    }
    1 + ch.improvements.val_int("AddLimb", Some(slot)) + i32::from(slot == "arm" || slot == "leg")
}

/// The limb's own base STR/AGI. Older saves lack `minstrength`, so fall
/// back to the data record, then to Chummer's default of 3.
fn limb_base(e: &Element, abbrev: &str, store: Option<&DataStore>) -> i32 {
    let key = if abbrev == "STR" { "minstrength" } else { "minagility" };
    if let Some(v) = e.get_i32(key) {
        return v;
    }
    store
        .and_then(|s| s.doc("cyberware.xml").ok())
        .and_then(|doc| {
            let c = doc.child("cyberwares")?;
            let id = e.get("sourceid");
            let rec = c
                .children_named("cyberware")
                .find(|r| !id.is_empty() && r.get("id").eq_ignore_ascii_case(&id))
                .or_else(|| c.children_named("cyberware").find(|r| r.get("name") == e.get("name")))?;
            rec.get_i32(key)
        })
        .unwrap_or(3)
}

/// `Cyberware.GetAttributeTotalValue` for STR/AGI.
fn limb_attribute_total(ch: &Character, e: &Element, abbrev: &str, max: i32, aug_max: i32, store: Option<&DataStore>) -> i32 {
    if e.get_bool("inheritattributes").unwrap_or(false) {
        let vals: Vec<i32> = ware_children(e).map(|c| limb_attribute_total(ch, c, abbrev, max, aug_max, store)).filter(|v| *v > 0).collect();
        return vals.iter().sum::<i32>() / (vals.len().max(1) as i32);
    }
    if e.get("category") != "Cyberlimb" && !is_limb(e) {
        return 0;
    }
    let pick = |table: &[(&str, &[&str])]| -> Option<i32> {
        let names = table.iter().find(|(a, _)| *a == abbrev)?.1;
        ware_children(e).filter(|c| names.contains(&c.get("name").as_str())).map(|c| c.get_i32("rating").unwrap_or(0)).max()
    };
    let value = pick(CUSTOMIZATION).unwrap_or_else(|| limb_base(e, abbrev, store)).min(max);
    let cap = 4;
    let bonus = (pick(ENHANCEMENT).unwrap_or(0) + ch.improvements.val_int("CyberlimbAttributeBonus", Some(abbrev))).min(cap);
    (value + bonus).min(aug_max)
}

/// `(limb count, sum of limb values)` over installed limbs.
fn process_cyberlimbs<'a>(
    ch: &Character,
    list: impl Iterator<Item = &'a Element>,
    abbrev: &str,
    max: i32,
    aug_max: i32,
    rules: &Rules,
    store: Option<&DataStore>,
) -> (i32, i32) {
    let (mut count, mut total) = (0, 0);
    for w in list {
        if is_limb(w) {
            let slot = w.get("limbslot");
            let n = match w.get("limbslotcount").trim() {
                s if s.eq_ignore_ascii_case("all") => limb_count(ch, rules, &slot),
                s => s.parse().unwrap_or(1),
            };
            count += n;
            total += limb_attribute_total(ch, w, abbrev, max, aug_max, store) * n;
        } else {
            let (c, t) = process_cyberlimbs(ch, ware_children(w), abbrev, max, aug_max, rules, store);
            count += c;
            total += t;
        }
    }
    (count, total)
}

/// `CharacterAttrib` totals for one attribute.
pub fn attribute_values(ch: &Character, name: &str, rules: &Rules) -> AttributeValues {
    attribute_values_with(ch, name, rules, None)
}

/// As [`attribute_values`], with data access for cyberlimb base values.
pub fn attribute_values_with(ch: &Character, name: &str, rules: &Rules, store: Option<&DataStore>) -> AttributeValues {
    let Some(a) = ch.attribute(name) else {
        return AttributeValues { name: name.into(), ..Default::default() };
    };
    let imps = &ch.improvements;
    let base_key = format!("{name}Base");
    let list_x = imps.winners(Query::named("Attribute", name), Field::Val);
    let list_base = imps.winners(Query::named("Attribute", &base_key), Field::Val);
    let rated = |i: &&Improvement, f: fn(&Improvement) -> f64| f(i) * f64::from(i.rating);
    let sum = |l: &[&Improvement], f: fn(&Improvement) -> f64| -> i32 { l.iter().map(|i| rated(i, f)).sum::<f64>() as i32 };

    // ReplaceAttribute: the last enabled one with a nonzero field wins.
    let replace = |f: fn(&Improvement) -> f64| -> Option<i32> {
        imps.of_kind("ReplaceAttribute").filter(|i| i.improved_name == name && f(i) != 0.0).last().map(|i| f(i) as i32)
    };
    let shapeshifter = a.category == "Shapeshifter";
    let metatype_min = if shapeshifter { a.metatype_min } else { replace(|i| i.min).unwrap_or(a.metatype_min) };
    let mut metatype_max = if shapeshifter { a.metatype_max } else { replace(|i| i.max).unwrap_or(a.metatype_max) };
    let metatype_aug_max = if shapeshifter { a.metatype_aug_max } else { replace(|i| i.aug_max).unwrap_or(a.metatype_aug_max) };
    if name == "ESS" {
        metatype_max += imps.val_int("EssenceMax", None);
    }

    let min_mods = sum(&list_x, |i| i.min) + sum(&list_base, |i| i.min);
    let max_mods = sum(&list_x, |i| i.max) + sum(&list_base, |i| i.max);
    let aug_max_mods = sum(&list_x, |i| i.aug_max);

    let mut raw_min = metatype_min + min_mods;
    if !rules.unclamp_attribute_minimum {
        raw_min = raw_min.max(0);
    }
    let total_max = (metatype_max + max_mods).max(0);
    let clamped = imps.has_named("AttributeMaxClamp", name);
    let total_aug_max = if clamped { total_max } else { (metatype_aug_max + max_mods + aug_max_mods).max(0) };
    let is_critter = ch.flag("iscritter");
    let total_min = if raw_min < 1 {
        if is_critter || total_max == 0 || is_special_zero(name) { 0 } else { 1 }
    } else {
        raw_min
    };

    let base = a.base;
    let free_base = standard_round(imps.val("Attributelevel", Some(name)).min(f64::from(metatype_max - metatype_min)));
    let total_base = (base + free_base + raw_min).max(total_min);
    let value_mods = standard_round(imps.aug("Attribute", &base_key));
    let value = ((base + free_base + raw_min + value_mods).max(total_min) + a.karma).min(total_max);

    let mut clamp = metatype_aug_max - metatype_max + aug_max_mods;
    if clamped {
        clamp = clamp.min(total_max - value);
    }
    let augment = standard_round(imps.aug("Attribute", name)).min(clamp);

    let meat = value + augment;
    let mut total = meat;
    if name == "STR" || name == "AGI" {
        let top = ch.doc.child("cyberwares").into_iter().flat_map(|c| c.children_named("cyberware"));
        let (count, mut limb_total) = process_cyberlimbs(ch, top, name, total_max, total_aug_max, rules, store);
        if count > 0 {
            let max_limbs = limb_count(ch, rules, "").max(1);
            limb_total += meat.max(0) * (max_limbs - count).max(0);
            total = (limb_total + max_limbs - 1) / max_limbs;
        }
    }
    let mut total = total.min(total_aug_max);
    if total < 1 {
        let is_ai = ch.dep_enabled() && ch.attribute("BOD").is_some_and(|b| b.metatype_max == 0);
        total = if is_critter || metatype_max == 0 || matches!(name, "EDG" | "RES" | "MAG" | "MAGAdept") || (name == "DEP" && !is_ai) {
            0
        } else {
            1
        };
    }
    AttributeValues {
        name: name.into(),
        metatype_min,
        metatype_max,
        metatype_aug_max,
        total_min,
        total_max,
        total_aug_max,
        base,
        free_base,
        karma: a.karma,
        total_base,
        value,
        augment,
        total,
    }
}

/// Karma spent on an attribute's karma levels (`TotalKarmaCost`, sync).
pub fn attribute_karma_cost(v: &AttributeValues, rules: &Rules) -> i32 {
    if v.karma <= 0 {
        return 0;
    }
    let tb = v.total_base;
    (2 * tb + v.karma + 1) * v.karma / 2 * rules.karma_attribute
}

/// Karma to raise an attribute by one (`UpgradeKarmaCost`), `None` at max.
pub fn attribute_upgrade_cost(v: &AttributeValues, rules: &Rules) -> Option<i32> {
    if v.value >= v.total_max {
        return None;
    }
    let cost = if v.value == 0 { rules.karma_attribute } else { (v.value + 1) * rules.karma_attribute };
    Some(cost.max(rules.karma_attribute.min(1)))
}

// ---------------------------------------------------------------------------
// Skill catalog (from skills.xml)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SkillDef {
    pub id: String,
    pub name: String,
    pub attribute: String,
    pub category: String,
    pub group: String,
    pub default: bool,
    pub exotic: bool,
    pub specs: Vec<String>,
    pub source: String,
    pub page: String,
}

impl SkillDef {
    fn from_xml(e: &Element) -> Self {
        SkillDef {
            id: e.get("id").to_ascii_lowercase(),
            name: e.get("name"),
            attribute: e.get("attribute"),
            category: e.get("category"),
            group: e.get("skillgroup"),
            default: e.get_bool("default").unwrap_or(false),
            exotic: e.get_bool("exotic").unwrap_or(false),
            specs: e.child("specs").map(|s| s.children_named("spec").map(Element::text).collect()).unwrap_or_default(),
            source: e.get("source"),
            page: e.get("page"),
        }
    }
}

/// Active and knowledge skill definitions, indexed by id.
#[derive(Debug, Clone, Default)]
pub struct SkillCatalog {
    pub active: Vec<SkillDef>,
    pub knowledge: Vec<SkillDef>,
    pub groups: Vec<String>,
    by_id: HashMap<String, (bool, usize)>,
}

impl SkillCatalog {
    pub fn load(store: &DataStore) -> Result<Self, crate::data::DataError> {
        let doc = store.doc("skills.xml")?;
        let read = |c: &str| -> Vec<SkillDef> {
            doc.child(c).map(|s| s.children_named("skill").map(SkillDef::from_xml).collect()).unwrap_or_default()
        };
        let active = read("skills");
        let knowledge = read("knowledgeskills");
        let groups = doc.child("skillgroups").map(|g| g.children_named("name").map(Element::text).collect()).unwrap_or_default();
        let mut by_id = HashMap::new();
        for (i, d) in active.iter().enumerate() {
            by_id.insert(d.id.clone(), (true, i));
        }
        for (i, d) in knowledge.iter().enumerate() {
            by_id.entry(d.id.clone()).or_insert((false, i));
        }
        Ok(SkillCatalog { active, knowledge, groups, by_id })
    }

    pub fn get(&self, id: &str) -> Option<&SkillDef> {
        let (active, i) = *self.by_id.get(&id.to_ascii_lowercase())?;
        Some(if active { &self.active[i] } else { &self.knowledge[i] })
    }
}

// ---------------------------------------------------------------------------
// Whole-character sheet
// ---------------------------------------------------------------------------

/// A skill's computed values.
#[derive(Debug, Clone, Default)]
pub struct SkillValues {
    pub guid: String,
    pub name: String,
    pub attribute: String,
    pub category: String,
    pub group: String,
    pub base: i32,
    pub karma: i32,
    pub rating: i32,
    pub pool: i32,
    /// Bonus dice when a specialization applies (2, or 3 for expertise).
    pub spec_bonus: i32,
    pub specs: Vec<String>,
    pub default: bool,
    pub knowledge: bool,
    pub native: bool,
    pub karma_cost: i32,
}

/// Everything shown on the character's summary panel.
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub attributes: Vec<AttributeValues>,
    pub essence: f64,
    pub cyberware_essence: f64,
    pub bioware_essence: f64,
    pub wound_modifier: i32,
    pub initiative: i32,
    pub initiative_dice: i32,
    pub astral_initiative: i32,
    pub astral_initiative_dice: i32,
    pub matrix_cold_initiative: i32,
    pub matrix_cold_dice: i32,
    pub matrix_hot_initiative: i32,
    pub matrix_hot_dice: i32,
    pub physical_cm: i32,
    pub stun_cm: i32,
    pub cm_overflow: i32,
    pub cm_threshold: i32,
    pub limit_physical: i32,
    pub limit_mental: i32,
    pub limit_social: i32,
    pub limit_astral: i32,
    pub composure: i32,
    pub judge_intentions: i32,
    pub lift_carry: i32,
    pub memory: i32,
    pub armor: i32,
    pub knowledge_points: i32,
    pub knowledge_points_used: i32,
    pub contact_points: i32,
    pub skills: Vec<SkillValues>,
    pub knowledge_skills: Vec<SkillValues>,
    pub attribute_karma_spent: i32,
    pub skill_karma_spent: i32,
}

impl Sheet {
    pub fn attr(&self, name: &str) -> i32 {
        self.attributes.iter().find(|a| a.name == name).map_or(0, |a| a.total)
    }
    pub fn attr_values(&self, name: &str) -> Option<&AttributeValues> {
        self.attributes.iter().find(|a| a.name == name)
    }
}

/// Attribute tokens for expressions (`{STR}`, `{AGIUnaug}`, ...).
pub struct SheetAttributes<'a>(pub &'a [AttributeValues]);

impl AttributeSource for SheetAttributes<'_> {
    fn attribute_token(&self, token: &str) -> Option<i32> {
        for (suffix, pick) in [
            ("Unaug", (|a: &AttributeValues| a.value) as fn(&AttributeValues) -> i32),
            ("Base", |a| a.total_base),
            ("Minimum", |a| a.total_min),
            ("Maximum", |a| a.total_max),
        ] {
            if let Some(n) = token.strip_suffix(suffix) {
                if let Some(a) = self.0.iter().find(|a| a.name == n) {
                    return Some(pick(a));
                }
            }
        }
        self.0.iter().find(|a| a.name == token).map(|a| a.total)
    }
}

/// Look up a cyberware/bioware grade's essence multiplier.
fn grade_multiplier(store: Option<&DataStore>, bioware: bool, grade: &str) -> f64 {
    let Some(store) = store else { return 1.0 };
    let file = if bioware { "bioware.xml" } else { "cyberware.xml" };
    let Ok(doc) = store.doc(file) else { return 1.0 };
    doc.child("grades")
        .and_then(|g| g.children_named("grade").find(|e| e.get("name") == grade))
        .and_then(|e| e.get_f64("ess"))
        .unwrap_or(1.0)
}

fn round_ess(x: f64, rules: &Rules) -> f64 {
    if rules.dont_round_essence {
        x
    } else {
        expr::round_away(x, rules.essence_decimals)
    }
}

/// Essence cost of one installed piece of ware and its children
/// (`Cyberware.CalculatedESS`, common case).
pub fn ware_essence(ch: &Character, e: &Element, attrs: &dyn AttributeSource, store: Option<&DataStore>, rules: &Rules, parent_grade: Option<&str>) -> f64 {
    if e.get_bool("prototypetranshuman").unwrap_or(false) && ch.flag("prototypetranshuman") {
        return 0.0;
    }
    let name = e.get("name");
    let rating = e.get_i32("rating").unwrap_or(0);
    let bioware = e.get("improvementsource") == "Bioware";
    if name == "Essence Hole" {
        return f64::from(rating) / 100.0;
    }
    if name == "Essence Antihole" {
        return -f64::from(rating) / 100.0;
    }
    let grade = parent_grade.map(str::to_owned).unwrap_or_else(|| e.get("grade"));
    let min_rating = e.get_i32("minrating").unwrap_or(0);
    let ess_str = expr::fixed_values(e.get("ess").trim(), rating).replace("MinRating", &min_rating.to_string());
    let base_ess = expr::value_to_dec(&ess_str, rating, attrs);
    let mut mult = grade_multiplier(store, bioware, &grade) + e.get_f64("extraessadditivemultiplier").unwrap_or(0.0);
    if !e.get("suite").is_empty() && e.get_bool("suite").unwrap_or(false) {
        mult -= 0.1;
    }
    let mut total_mult = e.get_f64("extraessmultiplicativemultiplier").unwrap_or(1.0);
    if total_mult == 0.0 {
        total_mult = 1.0;
    }
    total_mult *= 1.0 - e.get_f64("essdiscount").unwrap_or(0.0) / 100.0;
    if e.get("forcegrade") != "None" {
        let imps = &ch.improvements;
        let (cost, total) = if bioware { ("BiowareEssCost", "BiowareTotalEssMultiplier") } else { ("CyberwareEssCost", "CyberwareTotalEssMultiplier") };
        mult -= imps.of_kind(cost).map(|i| 1.0 - i.val / 100.0).sum::<f64>();
        if !ch.created {
            let nr = if bioware { "BiowareEssCostNonRetroactive" } else { "CyberwareEssCostNonRetroactive" };
            mult -= imps.of_kind(nr).map(|i| 1.0 - i.val / 100.0).sum::<f64>();
        }
        total_mult *= imps.of_kind(total).map(|i| i.val / 100.0).product::<f64>();
        if bioware && e.get("category") == "Basic" {
            mult -= imps.of_kind("BasicBiowareEssCost").map(|i| 1.0 - i.val / 100.0).sum::<f64>();
        }
    }
    let modifier = (mult * total_mult).max(0.0);
    let mut ess = round_ess(base_ess * modifier, rules);
    if let Some(children) = e.child("children") {
        for c in children.children_named("cyberware") {
            if c.get_bool("addtoparentess").unwrap_or(false) {
                ess += ware_essence(ch, c, attrs, store, rules, Some(&grade));
            }
        }
    }
    ess
}

/// Compute the full sheet. `store` enables grade lookups for essence;
/// `catalog` enables skill names and attributes.
pub fn compute(ch: &Character, rules: &Rules, store: Option<&DataStore>, catalog: Option<&SkillCatalog>) -> Sheet {
    let imps = &ch.improvements;
    let names: Vec<String> = ch.attributes.iter().map(|a| a.name.clone()).fold(Vec::new(), |mut v, n| {
        if !v.contains(&n) {
            v.push(n);
        }
        v
    });
    let attributes: Vec<AttributeValues> = names.iter().map(|n| attribute_values_with(ch, n, rules, store)).collect();
    let lookup = Sheet { attributes: attributes.clone(), ..Default::default() };
    let at = |n: &str| lookup.attr(n);
    let src = SheetAttributes(&lookup.attributes);

    // Essence
    let mut cyber = 0.0;
    let mut bio = 0.0;
    for w in ch.items("cyberwares", "cyberware") {
        let e = ware_essence(ch, w, &src, store, rules, None);
        if w.get("improvementsource") == "Bioware" {
            bio += e;
        } else {
            cyber += e;
        }
    }
    let ess_max = lookup.attr_values("ESS").map_or(6, |a| a.metatype_max);
    let essence = if imps.has("CyborgEssence") {
        0.1
    } else {
        f64::from(ess_max) + imps.val("EssencePenalty", None) + imps.val("EssencePenaltyT100", None) / 100.0 - cyber - bio
    };

    // Condition monitors and wound modifier
    let physical_cm = 8 + div_away_from_zero(at("BOD"), 2) + imps.val_int("PhysicalCM", None);
    let stun_cm = 8 + div_away_from_zero(at("WIL"), 2) + imps.val_int("StunCM", None);
    let cm_threshold = 3 + imps.val_int("CMThreshold", None);
    let offset = imps.val_int("CMThresholdOffset", None);
    let ignore_stun = imps.has("IgnoreCMPenaltyStun");
    let ignore_phys = imps.has("IgnoreCMPenaltyPhysical");
    let pen = |filled: i32, cm: i32| -> i32 { (offset - filled.min(cm)).min(0) / cm_threshold };
    let wound = if ignore_phys { 0 } else { pen(ch.physical_cm_filled, physical_cm) }
        + if ignore_stun { 0 } else { pen(ch.stun_cm_filled, stun_cm) };

    // Initiative
    let init_dice_base = ch.doc.get_i32("initiativedice").unwrap_or(rules.min_initiative_dice);
    let initiative = (at("INT") + at("REA") + wound + imps.val_int("Initiative", None)).max(0);
    let initiative_dice = (init_dice_base + imps.val_int("InitiativeDice", None) + imps.val_int("InitiativeDiceAdd", None)).min(rules.max_initiative_dice);
    let matrix_dice = imps.val_int("MatrixInitiativeDice", None);
    let matrix_init = imps.val_int("MatrixInitiative", None);
    let commlink_dp = ch
        .items("gears", "gear")
        .into_iter()
        .find(|g| g.get_bool("active").unwrap_or(false) && g.get("category") == "Commlinks")
        .and_then(|g| g.get_i32("devicerating"))
        .unwrap_or(0);

    // Limits
    let ess_round = standard_round(essence);
    let limit_physical = div_away_from_zero(2 * at("STR") + at("BOD") + at("REA"), 3) + imps.val_int("PhysicalLimit", None);
    let limit_mental = div_away_from_zero(2 * at("LOG") + at("INT") + at("WIL"), 3) + imps.val_int("MentalLimit", None);
    let limit_social = div_away_from_zero(2 * at("CHA") + at("WIL") + ess_round, 3) + imps.val_int("SocialLimit", None);

    // Knowledge and contact points
    let knowledge_points = standard_round(expr::evaluate_num(&expr::substitute_attributes(&rules.knowledge_points_expression, &src)).unwrap_or(0.0))
        + imps.val_int("FreeKnowledgeSkills", None);
    let contact_points = standard_round(expr::evaluate_num(&expr::substitute_attributes(&rules.contact_points_expression, &src)).unwrap_or(0.0))
        + imps.val_int("ContactPoints", None);

    let armor = standard_round(armor_rating(ch, at("STR")));
    let mut s = Sheet { attributes, ..Default::default() };

    s.essence = essence;
    s.cyberware_essence = cyber;
    s.bioware_essence = bio;
    s.wound_modifier = wound;
    s.initiative = initiative;
    s.initiative_dice = initiative_dice;
    s.astral_initiative = at("INT") * 2 + wound;
    s.astral_initiative_dice = rules.min_astral_initiative_dice.min(rules.max_astral_initiative_dice);
    s.matrix_cold_initiative = at("INT") + commlink_dp + wound + matrix_init;
    s.matrix_cold_dice = (rules.min_coldsim_dice + matrix_dice).min(rules.max_coldsim_dice);
    s.matrix_hot_initiative = s.matrix_cold_initiative;
    s.matrix_hot_dice = (rules.min_hotsim_dice + matrix_dice).min(rules.max_hotsim_dice);
    s.physical_cm = physical_cm;
    s.stun_cm = stun_cm;
    s.cm_overflow = at("BOD") + imps.val_int("CMOverflow", None) + 1;
    s.cm_threshold = cm_threshold;
    s.limit_physical = limit_physical;
    s.limit_mental = limit_mental;
    s.limit_social = limit_social;
    s.limit_astral = limit_mental.max(limit_social);
    s.composure = at("WIL") + at("CHA") + imps.val_int("Composure", None) + wound;
    s.judge_intentions = at("INT") + at("CHA") + standard_round(imps.val("JudgeIntentions", None) + imps.val("JudgeIntentionsOffense", None)) + wound;
    s.lift_carry = at("STR") + at("BOD") + imps.val_int("LiftAndCarry", None) + wound;
    s.memory = at("LOG") + at("WIL") + imps.val_int("Memory", None) + wound;
    s.armor = armor;
    s.knowledge_points = knowledge_points;
    s.contact_points = contact_points;
    s.attribute_karma_spent = s.attributes.iter().map(|a| attribute_karma_cost(a, rules)).sum();

    if let Some(cat) = catalog {
        let skills: Vec<SkillValues> = ch.skills.iter().map(|sk| skill_values(ch, &s, sk, cat, rules)).collect();
        let kno: Vec<SkillValues> = ch.knowledge_skills.iter().map(|k| knowledge_values(ch, &s, k, rules)).collect();
        s.knowledge_points_used = ch.knowledge_skills.iter().map(|k| if k.native_language { 0 } else { k.base }).sum();
        s.skill_karma_spent = skills.iter().chain(kno.iter()).map(|v| v.karma_cost).sum();
        s.skills = skills;
        s.knowledge_skills = kno;
    }
    s
}

/// Simplified `GetArmorRatingWithImprovement`: the best equipped armor plus
/// "+N" accessories (capped at STR), plus general Armor improvements.
fn armor_rating(ch: &Character, strength: i32) -> f64 {
    let imps = &ch.improvements;
    let equipped: Vec<&Element> = ch.items("armors", "armor").into_iter().filter(|a| a.get_bool("equipped").unwrap_or(false)).collect();
    let armor_guids: Vec<String> = equipped
        .iter()
        .flat_map(|a| {
            let mut v = vec![a.get("guid").to_ascii_lowercase()];
            if let Some(mods) = a.child("armormods") {
                v.extend(mods.children_named("armormod").map(|m| m.get("guid").to_ascii_lowercase()));
            }
            v
        })
        .collect();
    let general: f64 = imps.of_kind("Armor").filter(|i| !armor_guids.contains(&i.source_name.to_ascii_lowercase())).map(|i| i.val).sum();
    if equipped.is_empty() {
        return general;
    }
    let piece_value = |a: &Element| -> (bool, i32) {
        let raw = a.child_text("armoroverride").filter(|s| !s.trim().is_empty() && s.trim() != "0").unwrap_or_else(|| a.get("armor"));
        let stacking = raw.trim().starts_with('+');
        let mut v = expr::parse_plain(raw.trim().trim_start_matches('+')).unwrap_or(0.0) as i32;
        v -= a.get_i32("damage").unwrap_or(0);
        if let Some(mods) = a.child("armormods") {
            for m in mods.children_named("armormod") {
                if m.get_bool("equipped").unwrap_or(true) {
                    let r = m.get_i32("rating").unwrap_or(0);
                    v += expr::value_to_int(&m.get("armor"), r, &expr::NoAttributes);
                }
            }
        }
        let own: f64 = imps
            .of_kind("Armor")
            .filter(|i| i.source_name.eq_ignore_ascii_case(&a.get("guid")))
            .map(|i| i.val)
            .sum();
        (stacking, v + standard_round(own))
    };
    let mut best: Option<i32> = None;
    let mut stack = 0;
    for a in &equipped {
        let (stacking, v) = piece_value(a);
        if stacking {
            stack += v;
        } else {
            best = Some(best.map_or(v, |b| b.max(v)));
        }
    }
    let stack = stack.min(strength);
    f64::from(best.unwrap_or(0) + stack) + general
}

/// Plain sum of improvement values relevant to an active skill's pool.
fn skill_pool_bonus(ch: &Character, key: &str, def: &SkillDef, attribute: &str) -> f64 {
    let mut b = 0.0;
    for i in ch.improvements.active() {
        if i.add_to_rating {
            continue;
        }
        let excluded = !i.exclude.is_empty() && i.exclude.split(',').any(|x| x.trim() == key);
        let hit = match i.kind.as_str() {
            "Skill" => i.improved_name == key,
            "SkillGroup" => !def.group.is_empty() && i.improved_name == def.group && !excluded,
            "SkillCategory" => i.improved_name == def.category && !excluded,
            "SkillAttribute" => i.improved_name == attribute && !excluded,
            "SkillLinkedAttribute" => i.improved_name == def.attribute && !excluded,
            "EnhancedArticulation" => def.category == "Physical Active" && matches!(attribute, "BOD" | "AGI" | "REA" | "STR"),
            _ => false,
        };
        if hit {
            b += i.val;
        }
    }
    b
}

fn rating_modifiers(ch: &Character, key: &str) -> i32 {
    standard_round(ch.improvements.of_kind("Skill").filter(|i| i.add_to_rating && i.improved_name == key).map(|i| i.val).sum())
}

/// Karma for raising a skill from `lower` to `upper` (`RangeCost`).
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

fn skill_values(ch: &Character, sheet: &Sheet, sk: &Skill, cat: &SkillCatalog, rules: &Rules) -> SkillValues {
    let def = cat.get(&sk.suid).cloned().unwrap_or_else(|| SkillDef { name: "(unknown skill)".into(), ..Default::default() });
    let key = if def.exotic && !sk.specific.is_empty() { format!("{} ({})", def.name, sk.specific) } else { def.name.clone() };
    let imps = &ch.improvements;
    let attribute = imps
        .of_kind("SwapSkillAttribute")
        .filter(|i| i.target == key)
        .last()
        .map(|i| i.improved_name.clone())
        .unwrap_or_else(|| def.attribute.clone());
    let rating_max = if ch.created { rules.max_skill_rating_career } else { rules.max_skill_rating_create }
        + imps.of_kind("Skill").filter(|i| i.improved_name == key).map(|i| i.max as i32).sum::<i32>();
    let group = ch.skill_groups.iter().find(|g| g.name == def.group && !def.group.is_empty());
    let free_base = imps.val_int("SkillBase", Some(&key));
    let free_karma = imps.val_int("SkillLevel", Some(&key));
    let (group_base, group_karma) = group
        .map(|g| {
            let gfree = imps.val_int("SkillGroupBase", Some(&g.name));
            let glvl = imps.val_int("SkillGroupLevel", Some(&g.name));
            ((g.base + gfree).min(rating_max), (g.karma + glvl).min(rating_max))
        })
        .unwrap_or((0, 0));
    let base = if group_base > 0 { (group_base + free_base).min(rating_max) } else { (sk.base + free_base).min(rating_max) };
    let karma = (sk.karma + free_karma + group_karma).min(rating_max);
    let total_base_rating = base + karma + rating_modifiers(ch, &key);
    let hardwire = imps.of_kind("Hardwire").filter(|i| i.improved_name == key).map(|i| i.val as i32).max();
    let rating = total_base_rating.max(hardwire.unwrap_or(0));

    let blocked = ["BlockSkillDefault", "BlockSkillCategoryDefault", "BlockSkillGroupDefault"].iter().any(|k| {
        imps.of_kind(k).any(|i| i.improved_name.is_empty() || i.improved_name == key || i.improved_name == def.category || i.improved_name == def.group)
    });
    let allowed = imps.of_kind("AllowSkillDefault").any(|i| i.improved_name.is_empty() || i.improved_name == key);
    let default = !blocked && (allowed || def.default);
    let default_mod = if imps.of_kind("RemoveSkillDefaultPenalty").any(|i| i.improved_name == key)
        || imps.of_kind("RemoveSkillCategoryDefaultPenalty").any(|i| i.improved_name == def.category || i.improved_name == def.group)
    {
        0
    } else {
        -1
    };
    let disabled = imps.of_kind("SkillDisable").any(|i| i.improved_name == key)
        || (attribute == "MAG" && !ch.mag_enabled())
        || (attribute == "RES" && !ch.res_enabled())
        || (attribute == "DEP" && !ch.dep_enabled());
    let a = sheet.attr(&attribute);
    let bonus = standard_round(skill_pool_bonus(ch, &key, &def, &attribute));
    let pool = if disabled || a <= 0 {
        0
    } else if rating > 0 {
        (rating + a + bonus + sheet.wound_modifier).max(0)
    } else if default {
        (a + bonus + default_mod + sheet.wound_modifier).max(0)
    } else {
        0
    };
    let spec_bonus = if def.exotic || sk.specs.is_empty() || total_base_rating == 0 {
        0
    } else if sk.specs.iter().any(|s| s.expertise) {
        3
    } else {
        2
    };
    let lower = base + free_karma + rating_modifiers(ch, &key);
    let mut karma_cost =
        if total_base_rating == 0 { 0 } else { skill_range_cost(lower, total_base_rating, rules.karma_new_active_skill, rules.karma_improve_active_skill) };
    let priority = matches!(ch.field("buildmethod").as_str(), "Priority" | "SumtoTen");
    if sk.buy_with_karma || !priority {
        karma_cost += sk.specs.iter().filter(|s| !s.free).count() as i32 * rules.karma_specialization;
    }
    SkillValues {
        guid: sk.guid.clone(),
        name: key,
        attribute,
        category: def.category.clone(),
        group: def.group.clone(),
        base,
        karma,
        rating,
        pool,
        spec_bonus,
        specs: sk.specs.iter().map(|s| s.name.clone()).collect(),
        default,
        knowledge: false,
        native: false,
        karma_cost,
    }
}

fn knowledge_values(ch: &Character, sheet: &Sheet, k: &KnowledgeSkill, rules: &Rules) -> SkillValues {
    let attribute = k.attribute().to_owned();
    let imps = &ch.improvements;
    let rating_max = if ch.created { rules.max_skill_rating_career } else { rules.max_skill_rating_create };
    let free_base = imps.val_int("SkillBase", Some(&k.name));
    let free_karma = imps.val_int("SkillLevel", Some(&k.name));
    let base = (k.base + free_base).min(rating_max);
    let karma = (k.karma + free_karma).min(rating_max);
    let total = base + karma + rating_modifiers(ch, &k.name);
    let a = sheet.attr(&attribute);
    let bonus = standard_round(
        imps.active()
            .filter(|i| !i.add_to_rating && ((i.kind == "Skill" && i.improved_name == k.name) || (i.kind == "SkillCategory" && i.improved_name == k.kind)))
            .map(|i| i.val)
            .sum(),
    );
    let pool = if k.native_language {
        i32::MAX
    } else if total > 0 && a > 0 {
        (total + a + bonus + sheet.wound_modifier).max(0)
    } else {
        0
    };
    let l = base + free_karma;
    let mut karma_cost = (total * (total + 1) - l * (l + 1)) / 2 * rules.karma_improve_knowledge_skill;
    if l == 0 && karma_cost > 0 {
        karma_cost += rules.karma_new_knowledge_skill - rules.karma_improve_knowledge_skill;
    }
    SkillValues {
        guid: k.guid.clone(),
        name: k.name.clone(),
        attribute,
        category: k.kind.clone(),
        group: String::new(),
        base,
        karma,
        rating: total,
        pool,
        spec_bonus: if k.specs.is_empty() || total == 0 { 0 } else { 2 },
        specs: k.specs.iter().map(|s| s.name.clone()).collect(),
        default: false,
        knowledge: true,
        native: k.native_language,
        karma_cost: karma_cost.max(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_away() {
        assert_eq!(div_away_from_zero(5, 2), 3);
        assert_eq!(div_away_from_zero(4, 2), 2);
        assert_eq!(div_away_from_zero(-5, 2), -3);
        assert_eq!(div_away_from_zero(10, 3), 4);
    }

    #[test]
    fn range_cost() {
        // New skill to 1: KNAS. 0 -> 3 at 2/2: (6-1)*2 + 2 = 12
        assert_eq!(skill_range_cost(0, 1, 2, 2), 2);
        assert_eq!(skill_range_cost(0, 3, 2, 2), 12);
        assert_eq!(skill_range_cost(3, 4, 2, 2), 8);
    }

    #[test]
    fn attribute_karma() {
        let rules = Rules::default();
        let v = AttributeValues { total_base: 3, karma: 2, ..Default::default() };
        // 4*5 + 5*5 = 45
        assert_eq!(attribute_karma_cost(&v, &rules), 45);
    }
}

/// Refresh the export-only totals Chummer writes (`<totalvalue>` per
/// attribute and `<totaless>`) so other tools reading the file see
/// current numbers.
pub fn stamp_totals(ch: &mut Character, sheet: &Sheet) {
    if let Some(attrs) = ch.doc.child_mut("attributes") {
        for e in attrs.elements_mut() {
            let name = e.get("name");
            if let Some(v) = sheet.attr_values(&name) {
                e.set_child_text("totalvalue", v.total.to_string());
            }
        }
    }
    ch.doc.set_child_text("totaless", format!("{:.2}", sheet.essence));
}
