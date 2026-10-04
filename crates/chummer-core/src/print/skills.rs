//! `<skills>` (`SkillsSection.Print`): active skills, skill groups and
//! knowledge skills, flat in one container. Ratings and pools come from
//! the calc [`crate::calc::Sheet`].

use super::{add, bool_text, Ctx};
use crate::calc::SkillValues;
use crate::skills::Specialization;
use crate::xml::Element;

const FILE: &str = "skills.xml";

/// `SkillsSection.Print`.
pub fn skills(ctx: &Ctx) -> Element {
    let mut out = Element::new("skills");
    for (sk, v) in ctx.ch.skills.iter().zip(&ctx.sheet.skills) {
        if !v.disabled {
            out.push(skill(ctx, v, &sk.suid, &sk.specs, sk.buy_with_karma, &sk.notes, &sk.specific));
        }
    }
    for g in ctx.ch.skill_groups.iter().filter(|g| g.rating() > 0) {
        out.push(skill_group(ctx, g));
    }
    for (k, v) in ctx.ch.knowledge_skills.iter().zip(&ctx.sheet.knowledge_skills) {
        out.push(skill(ctx, v, &k.suid, &k.specs, false, &k.notes, ""));
    }
    out
}

/// `SkillGroup.Print`.
fn skill_group(ctx: &Ctx, g: &crate::skills::SkillGroup) -> Element {
    let mut out = Element::new("skillgroup");
    add(&mut out, "guid", g.id.clone());
    add(&mut out, "name", ctx.lang.data_name(FILE, "", &g.name));
    add(&mut out, "name_english", g.name.clone());
    let max = if ctx.ch.created { ctx.rules.max_skill_rating_career } else { ctx.rules.max_skill_rating_create };
    for (k, v) in [("rating", g.rating()), ("ratingmax", max), ("base", g.base), ("karma", g.karma)] {
        add(&mut out, k, v.to_string());
    }
    add(&mut out, "isbroken", bool_text(false));
    out
}

/// `Skill.Print` (active, exotic and knowledge skills alike).
fn skill(ctx: &Ctx, v: &SkillValues, suid: &str, specs: &[Specialization], karma: bool, notes: &str, specific: &str) -> Element {
    let mut out = Element::new("skill");
    add(&mut out, "guid", v.guid.clone());
    add(&mut out, "suid", suid);
    add(&mut out, "name", ctx.lang.data_name(FILE, suid, &v.name));
    add(&mut out, "name_english", v.name.clone());
    group_fields(ctx, &mut out, v);
    add(&mut out, "skillcategory", ctx.lang.data_name(FILE, "", &v.category));
    add(&mut out, "skillcategory_english", v.category.clone());
    add(&mut out, "grouped", bool_text(is_grouped(ctx, v)));
    add(&mut out, "default", bool_text(v.default));
    for f in ["requiresgroundmovement", "requiresswimmovement", "requiresflymovement"] {
        add(&mut out, f, bool_text(false));
    }
    let max = if ctx.ch.created { ctx.rules.max_skill_rating_career } else { ctx.rules.max_skill_rating_create };
    add(&mut out, "rating", v.rating.to_string());
    add(&mut out, "ratingmax", max.to_string());
    let pool = if v.native { 0 } else { v.pool };
    add(&mut out, "specializedrating", (pool + v.spec_bonus).to_string());
    add(&mut out, "total", pool.to_string());
    add(&mut out, "knowledge", bool_text(v.knowledge));
    add(&mut out, "exotic", bool_text(!specific.is_empty()));
    add(&mut out, "buywithkarma", bool_text(karma));
    add(&mut out, "base", v.base.to_string());
    add(&mut out, "karma", v.karma.to_string());
    add(&mut out, "spec", display_spec(v, specific));
    add(&mut out, "attribute", v.attribute.clone());
    add(&mut out, "displayattribute", ctx.s(&format!("String_Attribute{}Short", v.attribute)));
    if ctx.opts.notes {
        add(&mut out, "notes", notes);
    }
    add(&mut out, "source", v.source.clone());
    add(&mut out, "page", v.page.clone());
    let attr = ctx.sheet.attr(&v.attribute);
    add(&mut out, "attributemod", attr.to_string());
    let poolmod = if v.rating > 0 && !v.native { (pool - v.rating - attr - ctx.sheet.wound_modifier).max(0) } else { 0 };
    add(&mut out, "ratingmod", poolmod.to_string());
    add(&mut out, "poolmod", poolmod.to_string());
    let language = v.knowledge && v.category == "Language";
    add(&mut out, "islanguage", bool_text(language));
    add(&mut out, "isnativelanguage", bool_text(v.native));
    add(&mut out, "bp", v.karma_cost.to_string());
    out.push(specializations(v, specs));
    out
}

/// `skillgroup` / `skillgroup_english` (localized "None" when ungrouped).
fn group_fields(ctx: &Ctx, out: &mut Element, v: &SkillValues) {
    if v.group.is_empty() {
        add(out, "skillgroup", ctx.s("String_None"));
        add(out, "skillgroup_english", ctx.s("String_None"));
    } else {
        add(out, "skillgroup", ctx.lang.data_name(FILE, "", &v.group));
        add(out, "skillgroup_english", v.group.clone());
    }
}

/// `Skill.IsGrouped`: the skill's group has a rating.
fn is_grouped(ctx: &Ctx, v: &SkillValues) -> bool {
    !v.group.is_empty() && ctx.ch.skill_groups.iter().any(|g| g.name == v.group && g.rating() > 0)
}

/// `Skill.DisplaySpecialization` (exotic skills show their specific).
fn display_spec(v: &SkillValues, specific: &str) -> String {
    if !specific.is_empty() {
        return specific.to_owned();
    }
    if v.native {
        return String::new();
    }
    v.specs.join(", ")
}

/// `<skillspecializations>` (`SkillSpecialization.Print`).
fn specializations(v: &SkillValues, specs: &[Specialization]) -> Element {
    let mut out = Element::new("skillspecializations");
    for s in specs {
        let mut e = Element::new("skillspecialization");
        add(&mut e, "guid", s.guid.clone());
        add(&mut e, "name", s.name.clone());
        add(&mut e, "free", bool_text(s.free));
        add(&mut e, "expertise", bool_text(s.expertise));
        add(&mut e, "specbonus", if s.expertise { 3.max(v.spec_bonus) } else { v.spec_bonus.min(2) }.to_string());
        out.push(e);
    }
    out
}
