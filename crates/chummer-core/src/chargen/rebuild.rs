//! "Change Priorities" / "Change Metatype" in creation mode: Chummer opens
//! the metatype and priority selection again on the existing character
//! (`CharacterCreate.ChangeMetatype` → `SelectMetatypePriority`
//! `MetatypeSelected`, or `SelectMetatypeKarma` for karma builds) and
//! applies the new choice over what is there.
//!
//! What it does here, as Chummer does:
//! - a new metatype or metavariant (`Character.Create`): the metatype's
//!   improvements and qualities go, the attribute limits are replaced
//!   (points and karma kept, cut to the new maximums), the new bonus and
//!   racial qualities are added, and selected qualities that require or
//!   forbid a metatype are dropped when they no longer qualify;
//! - the five priority letters, the talent and its free skills; the
//!   starting nuyen, attribute, special, skill and skill group points and
//!   the metatype karma from the priority tables; the talent's qualities,
//!   magic/resonance/depth limits, spell, complex form and A.I. program
//!   limits.
//!
//! Not ported: switching build methods (chummer-rs changes settings only
//! within one build method), and so the conversion of karma-bought
//! attributes to points and the removal of `onlyprioritygiven` qualities;
//! critter metatypes' powers, natural weapons and skills (they are not
//! offered at a heritage priority); a forced value for an exotic talent
//! skill.

use crate::character::{uses_priority_tables, Character};
use crate::data::{self, Record};
use crate::engine::Engine;
use crate::improvement::Improvement;
use crate::xml::Element;

use super::{add_quality_with_source, apply_metatype_extras, heritage_options, priority_node, remove_quality, talent_options, talent_skill_options, Priorities, TalentOption};

/// What the selection dialog collects.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    pub metatype: String,
    pub metavariant: Option<String>,
    /// Priority builds: Heritage, Talent, Attributes, Skills, Resources.
    /// `None` for karma builds.
    pub priorities: Option<Priorities>,
    /// The talent's `<value>` ("Magician", "Mundane"); priority builds.
    pub talent: String,
    /// The talent's free skills or skill groups.
    pub talent_skills: Vec<String>,
}

impl Choice {
    /// The character's current choice, to start the dialog from.
    pub fn of(ch: &Character) -> Choice {
        let priority = uses_priority_tables(&ch.field("buildmethod"));
        let letter = |k: &str| ch.field(k).chars().next().unwrap_or('E');
        let priorities = priority.then(|| Priorities([letter("prioritymetatype"), letter("priorityspecial"), letter("priorityattributes"), letter("priorityskills"), letter("priorityresources")]));
        Choice {
            metatype: ch.field("metatype"),
            metavariant: Some(ch.field("metavariant")).filter(|v| !v.is_empty()),
            priorities,
            talent: if priority { ch.field("prioritytalent") } else { String::new() },
            // `<priorityskills>` is both the Skills letter and, a second
            // element of the same name, the list of talent skills.
            talent_skills: ch.doc.children_named("priorityskills").flat_map(|p| p.children_named("priorityskill")).map(Element::text).collect(),
        }
    }
}

/// Whether the talent may be taken with `metatype` (its `<required>` and
/// `<forbidden>` metatypes).
pub fn talent_allowed(t: &TalentOption, metatype: &str) -> bool {
    let has = |which: &str| -> Vec<String> { t.node.child(which).map(|w| w.children_named("oneof").flat_map(|o| o.children_named("metatype")).map(|e| e.text()).collect()).unwrap_or_default() };
    let forbidden = has("forbidden");
    let required = has("required");
    !forbidden.iter().any(|m| m == metatype) && (required.is_empty() || required.iter().any(|m| m == metatype))
}

/// Apply `choice` to a character in creation mode. On an error nothing
/// was changed (the caller works on a copy: [`crate::command::apply`]
/// restores the character when a command fails).
pub fn apply(ch: &mut Character, engine: &Engine, choice: &Choice) -> Result<(), String> {
    if ch.created {
        return Err("The metatype and priorities can only be changed during creation.".into());
    }
    let store = &engine.store;
    let settings = engine.settings.resolve(&ch.field("settings")).ok_or("The character's settings file was not found.")?.clone();
    let priority = uses_priority_tables(&settings.build_method());
    let metatypes = store.doc("metatypes.xml").map_err(|e| e.to_string())?;
    let mt = data::find(&metatypes, "metatypes", "metatype", &choice.metatype).ok_or_else(|| format!("Unknown metatype {}.", choice.metatype))?;
    let mut metavariant = choice.metavariant.clone().filter(|v| !v.is_empty());
    // A shapeshifter must have a metavariant; Chummer defaults to Human.
    if mt.category() == "Shapeshifter" && metavariant.is_none() {
        metavariant = Some("Human".into());
    }
    let mv = match &metavariant {
        Some(v) => Some(mt.el().child("metavariants").and_then(|m| m.children_named("metavariant").find(|x| x.get("name") == *v || x.get("id") == *v)).ok_or_else(|| format!("{} has no metavariant {v}.", mt.name()))?),
        None => None,
    };
    let node: &Element = if mt.category() == "Shapeshifter" { mt.el() } else { mv.unwrap_or(mt.el()) };

    // ----- validation (SelectMetatypePriority's messages) -----
    let mut talent = None;
    let mut heritage = (0, 0);
    if priority {
        let prios = choice.priorities.ok_or("Choose the priorities.")?;
        prios.validate(&settings)?;
        let options = heritage_options(store, &settings, prios.get("Heritage"));
        let h = options.iter().find(|h| h.metatype == mt.name()).ok_or_else(|| format!("{} is not available at Heritage priority {}.", mt.name(), prios.get("Heritage")))?;
        heritage = match &metavariant {
            Some(v) => h.metavariants.iter().find(|m| m.0 == *v).map(|m| (m.1, m.2)).ok_or_else(|| format!("{v} is not available at Heritage priority {}.", prios.get("Heritage")))?,
            None => (h.special, h.karma),
        };
        let talents = talent_options(store, &settings, prios.get("Talent"));
        if talents.is_empty() {
            return Err("There are no Magic or Resonance choices at this priority.".into());
        }
        let t = talents.into_iter().find(|t| t.value == choice.talent).ok_or_else(|| format!("{} is not available at Talent priority {}.", choice.talent, prios.get("Talent")))?;
        if !talent_allowed(&t, &mt.name()) {
            return Err(format!("{} is not available to {}.", t.display, mt.name()));
        }
        let qty = t.skill_qty().max(0) as usize;
        let skills: Vec<&String> = choice.talent_skills.iter().filter(|s| !s.trim().is_empty()).collect();
        if skills.len() != qty {
            return Err("Please select a skill for the Magic or Resonance choice.".into());
        }
        if (1..skills.len()).any(|i| skills[..i].contains(&skills[i])) {
            return Err("You cannot select the same skill twice.".into());
        }
        if qty > 0 {
            let allowed = talent_skill_options(store, &t);
            if let Some(bad) = skills.iter().find(|s| !allowed.contains(s)) {
                return Err(format!("{bad} cannot be chosen for {}.", t.display));
            }
        }
        talent = Some(t);
    }

    // ----- a new metatype or metavariant (Character.Create) -----
    let old_variant = Some(ch.field("metavariant")).filter(|v| !v.is_empty());
    let metatype_changed = ch.field("metatype") != mt.name() || old_variant.as_deref() != mv.map(|v| v.get("name")).as_deref();
    if metatype_changed {
        let qdoc = store.doc("qualities.xml").map_err(|e| e.to_string())?;
        // Selected qualities whose requirements name a metatype: checked again afterwards.
        let restricted: Vec<(String, String)> = ch
            .items("qualities", "quality")
            .iter()
            .filter(|q| !matches!(q.get("qualitysource").as_str(), "Improvement" | "QualityLevelImprovement" | "Heritage" | "Metatype" | "MetatypeRemovable" | "MetatypeRemovedAtChargen"))
            .filter_map(|q| {
                let rec = data::find(&qdoc, "qualities", "quality", &q.get("name"))?;
                let names_metatype = |k: &str| {
                    rec.el().child(k).is_some_and(|r| {
                        let mut found = Vec::new();
                        r.descendants("metatype", &mut found);
                        r.descendants("metavariant", &mut found);
                        !found.is_empty()
                    })
                };
                (names_metatype("required") || names_metatype("forbidden")).then(|| (q.get("guid"), q.get("name")))
            })
            .collect();
        ch.improvements.list.retain(|i| i.source != "Metatype" && i.source != "Metavariant");
        let metatype_qualities: Vec<String> = ch
            .items("qualities", "quality")
            .iter()
            .filter(|q| matches!(q.get("qualitysource").as_str(), "Metatype" | "MetatypeRemovable" | "MetatypeRemovedAtChargen"))
            .map(|q| q.get("guid"))
            .collect();
        for g in metatype_qualities {
            remove_quality(ch, &g);
        }
        set_metatype_fields(ch, &settings, mt, mv, node);
        set_attribute_limits(ch, engine, mt, node);
        apply_metatype_extras(ch, store, mt, if mt.category() == "Shapeshifter" { mv.unwrap_or(mt.el()) } else { node })?;
        let sheet = engine.sheet(ch);
        let chr: &Character = ch;
        let failing: Vec<String> = restricted
            .iter()
            .filter(|(_, name)| {
                let check = crate::requirements::Check { ch: chr, sheet: &sheet, ignore_quality: Some(name) };
                data::find(&qdoc, "qualities", "quality", name).is_some_and(|rec| !crate::requirements::unmet(rec.el(), &check).is_empty())
            })
            .map(|(g, _)| g.clone())
            .collect();
        for g in failing {
            remove_quality(ch, &g);
        }
    }
    if !priority {
        // SelectMetatypeKarma: the metatype costs karma.
        let karma = mv.and_then(|v| v.get_i32("karma")).unwrap_or_else(|| mt.el().get_i32("karma").unwrap_or(0));
        ch.set_field("metatypebp", (karma * settings.int("metatypecostskarmamultiplier", 1)).to_string());
        return Ok(());
    }
    let prios = choice.priorities.expect("checked");
    let talent = talent.expect("checked");

    // ----- the priorities -----
    let old_special = (ch.field("priorityspecial").chars().next(), ch.field("prioritytalent"), Choice::of(ch).talent_skills);
    for (k, cat) in [("prioritymetatype", "Heritage"), ("priorityattributes", "Attributes"), ("priorityspecial", "Talent"), ("priorityskills", "Skills"), ("priorityresources", "Resources")] {
        // Older saves write "D,1" (letter, Sum-to-Ten value); 5.226 writes
        // the letter. A letter that did not change keeps its form.
        if !ch.field(k).starts_with(prios.get(cat)) {
            ch.set_field(k, prios.get(cat).to_string());
        }
    }
    ch.set_field("prioritytalent", talent.value.clone());
    let skills: Vec<String> = choice.talent_skills.iter().filter(|s| !s.trim().is_empty()).cloned().collect();
    let mut ps = Element::new("priorityskills");
    for s in &skills {
        ps.push(Element::with_text("priorityskill", s.clone()));
    }
    // The second `<priorityskills>` (the first is the Skills letter).
    let has_list = ch.doc.children_named("priorityskills").count() > 1;
    if has_list {
        if let Some(old) = ch.doc.elements_mut().filter(|e| e.name == "priorityskills").nth(1) {
            *old = ps;
        }
    } else {
        ch.doc.push(ps);
    }
    let nuyen = priority_node(store, &settings, "Resources", prios.get("Resources")).and_then(|n| n.get_f64("resources")).unwrap_or(0.0);
    ch.nuyen = nuyen;
    ch.set_field("nuyen", crate::improvement::fmt_num(nuyen));
    ch.set_field("startingnuyen", crate::improvement::fmt_num(nuyen));

    // ----- the talent (when it, its priority or its skills changed) -----
    let talent_changed = old_special != (Some(prios.get("Talent")), talent.value.clone(), skills.clone());
    // chummer-rs deviates from Chummer (LB-46): Chummer applies the
    // talent's magic/resonance limits only when the talent changed, but a
    // new metatype has just reset them to the metatype's (MAG 1 for a
    // priority-B magician), so they are applied after a new metatype too.
    if talent_changed || metatype_changed {
        apply_talent(ch, engine, &talent, node, &skills)?;
    }

    // ----- points from the tables -----
    let special = heritage.0 + talent.int("specialattribpoints");
    ch.set_field("special", special.to_string());
    ch.set_field("totalspecial", special.to_string());
    ch.set_field("metatypebp", heritage.1.to_string());
    let mut attributes = priority_node(store, &settings, "Attributes", prios.get("Attributes")).and_then(|n| n.get_i32("attributes")).unwrap_or(0);
    if node.child("halveattributepoints").is_some() || mt.el().child("halveattributepoints").is_some() {
        attributes /= 2;
    }
    ch.set_field("totalattributes", attributes.to_string());
    let skills_node = priority_node(store, &settings, "Skills", prios.get("Skills"));
    let ns = ch.items_mut("newskills");
    ns.set_child_text("skillptsmax", skills_node.as_ref().and_then(|n| n.get_i32("skills")).unwrap_or(0).to_string());
    ns.set_child_text("skillgrpsmax", skills_node.as_ref().and_then(|n| n.get_i32("skillgroups")).unwrap_or(0).to_string());
    Ok(())
}

/// The talent part of `MetatypeSelected`: its qualities (kept when they
/// are already there), magic/resonance/depth limits, spell, complex form
/// and program limits, and the free skills.
fn apply_talent(ch: &mut Character, engine: &Engine, talent: &TalentOption, node: &Element, skills: &[String]) -> Result<(), String> {
    let store = &engine.store;
    let qdoc = store.doc("qualities.xml").map_err(|e| e.to_string())?;
    let wanted: Vec<Record<'_>> = talent.node.child("qualities").into_iter().flat_map(|q| q.children_named("quality")).filter_map(|q| data::find(&qdoc, "qualities", "quality", &q.text())).collect();
    let old: Vec<(String, String)> = ch.items("qualities", "quality").iter().filter(|q| q.get("qualitysource") == "Heritage").map(|q| (q.get("guid"), q.get("name"))).collect();
    let mut keep: Vec<String> = Vec::new();
    for rec in &wanted {
        match old.iter().find(|(g, n)| *n == rec.name() && !keep.contains(g)) {
            Some((g, _)) => keep.push(g.clone()),
            None => {
                let g = add_quality_with_source(ch, store, *rec, None, "Heritage", false);
                keep.push(g);
            }
        }
    }
    for (g, _) in old.iter().filter(|(g, _)| !keep.contains(g)) {
        remove_quality(ch, g);
    }
    // MAG (and MAGAdept), RES, DEP: the talent's value as the minimum.
    for (key, names) in [("magic", &["MAG", "MAGAdept"][..]), ("resonance", &["RES"][..]), ("depth", &["DEP"][..])] {
        let min = talent.node.get_i32(key).unwrap_or(1);
        let short = &key[..3];
        let node_max = crate::gm::expression_to_int(node.child_text(&format!("{short}max")).as_deref(), 0, 0, 0);
        let max = talent.node.get_i32(&format!("max{key}")).unwrap_or(node_max.max(min));
        for n in names {
            if let Some(a) = ch.attribute_mut(n) {
                a.metatype_min = min;
                a.metatype_max = max;
                a.metatype_aug_max = max;
            }
        }
    }
    ch.set_field("spelllimit", talent.int("spells").to_string());
    ch.set_field("cfplimit", talent.int("cfp").to_string());
    ch.set_field("ainormalprogramlimit", talent.int("ainormalprogramlimit").to_string());
    ch.set_field("aiadvancedprogramlimit", talent.int("aiadvancedprogramlimit").to_string());
    ch.improvements.list.retain(|i| !(i.source == "Heritage" && matches!(i.kind.as_str(), "SkillBase" | "SkillGroupBase")));
    let kind = if talent.grouped() { "SkillGroupBase" } else { "SkillBase" };
    for s in skills {
        ch.improvements.list.push(Improvement { kind: kind.into(), improved_name: s.clone(), source: "Heritage".into(), val: f64::from(talent.skill_val()), rating: 1, enabled: true, ..Default::default() });
    }
    Ok(())
}

/// The metatype fields `Character.Create` sets.
fn set_metatype_fields(ch: &mut Character, settings: &crate::settings::CharacterSettings, mt: Record<'_>, mv: Option<&Element>, node: &Element) {
    ch.set_field("metatype", mt.name());
    ch.set_field("metatypeid", mt.id());
    ch.set_field("metavariant", mv.map(|v| v.get("name")).unwrap_or_default());
    ch.set_field("metavariantid", mv.map(|v| v.get("id")).unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into()));
    ch.set_field("metatypecategory", mt.category());
    ch.set_field("movement", mt.get("movement"));
    for k in ["walk", "run", "sprint"] {
        let v = node.child_text(k).or_else(|| mt.el().child_text(k)).unwrap_or_else(|| if k == "run" { "4/0/0".into() } else { "2/1/0".into() });
        ch.set_field(k, v);
    }
    for k in ["walkalt", "runalt", "sprintalt"] {
        let base = k.trim_end_matches("alt");
        ch.set_field(k, node.child_text(k).or_else(|| node.child_text(base)).unwrap_or_default());
    }
    let dice = node.get_i32("initiativedice").or_else(|| mt.el().get_i32("initiativedice")).unwrap_or(settings.int("mininitiativedice", 1));
    ch.set_field("initiativedice", dice.to_string());
}

/// `AttributeSection.Create`: the metatype's limits for every attribute;
/// the points and karma already spent stay, cut to what the new limits
/// allow (`PriorityMaximum`, then `KarmaMaximum`).
fn set_attribute_limits(ch: &mut Character, engine: &Engine, mt: Record<'_>, node: &Element) {
    let mut kept: Vec<(String, i32, i32)> = Vec::new();
    for name in crate::expr::ATTRIBUTE_NAMES {
        let key = match *name {
            "MAGAdept" => "mag".to_owned(),
            n => n.to_ascii_lowercase(),
        };
        let g = |suffix: &str, default: i32| node.get_i32(&format!("{key}{suffix}")).or_else(|| mt.el().get_i32(&format!("{key}{suffix}"))).unwrap_or(default);
        let (min, max, aug) = (g("min", 1), g("max", 6), g("aug", 10));
        if let Some(a) = ch.attribute_mut(name) {
            kept.push((name.to_string(), a.base, a.karma));
            a.metatype_min = min;
            a.metatype_max = max;
            a.metatype_aug_max = aug;
            a.base = 0;
            a.karma = 0;
        }
    }
    let rules = engine.rules_for(ch);
    let store = engine.store_for_character(ch);
    for (name, base, karma) in kept {
        let v = crate::calc::attribute_values_with(ch, &name, &rules, Some(&store));
        let base = base.min((v.total_max - v.free_base - v.raw_min).max(0));
        if let Some(a) = ch.attribute_mut(&name) {
            a.base = base;
        }
        let v = crate::calc::attribute_values_with(ch, &name, &rules, Some(&store));
        if let Some(a) = ch.attribute_mut(&name) {
            a.karma = karma.min((v.total_max - v.total_base).max(0));
        }
    }
}
