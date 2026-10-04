//! Magic and resonance print elements: tradition, spells, powers,
//! spirits, complex forms, martial arts, initiation, critter powers, and
//! the limit modifiers.
//!
//! Spell and complex-form fields (type, range, duration, DV/FV) are the
//! saved codes; in English they equal Chummer's display strings.

use super::{add, bool_text, copy, copy_bool, num, Ctx};
use crate::expr::{self, standard_round};
use crate::xml::Element;

/// `guid`, `sourceid`, `name`, `fullname`, `name_english`, `fullname_english`.
fn names(ctx: &Ctx, out: &mut Element, item: &Element, file: &str, full: &str) {
    ids(out, item);
    name_block(ctx, out, item, file, full);
}

/// `guid` and `sourceid`.
fn ids(out: &mut Element, item: &Element) {
    copy(out, item, "guid");
    add(out, "sourceid", [item.get("sourceid"), item.get("id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default());
}

/// `name`, `fullname`, `name_english`, `fullname_english`.
fn name_block(ctx: &Ctx, out: &mut Element, item: &Element, file: &str, full: &str) {
    let name = ctx.tr_name(file, item);
    add(out, "name", name.clone());
    add(out, "fullname", format!("{name}{full}"));
    add(out, "name_english", item.get("name"));
    add(out, "fullname_english", format!("{}{full}", item.get("name")));
}

fn extra_suffix(item: &Element) -> String {
    let extra = item.get("extra");
    if extra.is_empty() { String::new() } else { format!(" ({extra})") }
}

fn source_page(out: &mut Element, item: &Element) {
    copy(out, item, "source");
    copy(out, item, "page");
}

/// Both `x` and `x_english` from one saved field.
fn pair(out: &mut Element, item: &Element, field: &str) {
    let v = item.get(field);
    add(out, field, v.clone());
    add(out, &format!("{field}_english"), v);
}

/// Replace attribute codes with their totals and evaluate (`WIL + LOG`).
fn eval_attributes(ctx: &Ctx, s: &str) -> i32 {
    let mut e = s.to_owned();
    for a in ["MAGAdept", "BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES", "DEP"] {
        e = e.replace(a, &ctx.sheet.attr(a).to_string());
    }
    expr::evaluate_num(&e).map(standard_round).unwrap_or(0)
}

/// `Tradition.Print`. Reads the `<tradition>` element of current saves,
/// or the flat `<tradition>`/`<traditiondrain>` fields of older ones.
pub fn tradition(ctx: &Ctx) -> Option<Element> {
    let ch = ctx.ch;
    let saved = ch.doc.child("tradition").filter(|t| t.elements().next().is_some()).cloned().or_else(|| legacy_tradition(ctx))?;
    let kind = saved.get("traditiontype");
    if kind.is_empty() || kind == "None" {
        return None;
    }
    let mut out = Element::new("tradition");
    ids(&mut out, &saved);
    add(&mut out, "istechnomancertradition", bool_text(kind == "RES"));
    name_block(ctx, &mut out, &saved, if kind == "RES" { "streams.xml" } else { "traditions.xml" }, &extra_suffix(&saved));
    add(&mut out, "extra", saved.get("extra"));
    add(&mut out, "extra_english", saved.get("extra"));
    if kind == "MAG" {
        let spirits = ["spiritcombat", "spiritdetection", "spirithealth", "spiritillusion", "spiritmanipulation", "spiritform"];
        for f in spirits {
            add(&mut out, f, ctx.lang.data_name("critters.xml", "", &saved.get(f)));
        }
        for f in spirits {
            add(&mut out, &format!("{f}_english"), saved.get(f));
        }
    }
    let drain = saved.get("drain");
    add(&mut out, "drainattributes", drain.clone());
    add(&mut out, "drainattributes_english", drain.clone());
    let resist = ch.improvements.val_int(if kind == "RES" { "FadingResist" } else { "DrainResist" }, None);
    add(&mut out, "drainvalue", (eval_attributes(ctx, &drain) + resist).to_string());
    source_page(&mut out, &saved);
    Some(out)
}

/// Saves before 5.200 kept the tradition as flat character fields.
fn legacy_tradition(ctx: &Ctx) -> Option<Element> {
    let ch = ctx.ch;
    let (kind, name, drain) = if ch.mag_enabled() && !ch.field("tradition").is_empty() {
        ("MAG", ch.field("tradition"), ch.field("traditiondrain"))
    } else if ch.res_enabled() && !ch.field("stream").is_empty() {
        ("RES", ch.field("stream"), ch.field("streamdrain"))
    } else {
        return None;
    };
    let mut t = Element::new("tradition");
    add(&mut t, "traditiontype", kind);
    add(&mut t, "name", name);
    add(&mut t, "drain", drain);
    for f in ["spiritcombat", "spiritdetection", "spirithealth", "spiritillusion", "spiritmanipulation"] {
        add(&mut t, f, ch.field(f));
    }
    Some(t)
}

/// `<wrapper>` of `LimitModifier.Print` plus improvement-made modifiers.
pub fn limit_modifiers(ctx: &Ctx, wrapper: &str, limit: &str) -> Element {
    let mut out = Element::new(wrapper);
    for m in ctx.ch.items("limitmodifiers", "limitmodifier").into_iter().filter(|m| m.get("limit") == limit) {
        out.push(limit_modifier(ctx, m));
    }
    for i in ctx.ch.improvements.of_kind("LimitModifier").filter(|i| i.improved_name == limit) {
        let mut e = Element::new("limitmodifier");
        let mut name = format!("{}: {}{}", object_name(ctx, &i.source_name, &i.custom_name), if i.val > 0.0 { "+" } else { "" }, num(i.val));
        if !i.condition.is_empty() {
            name.push_str(&format!(", {}", i.condition));
        }
        add(&mut e, "name", name);
        if ctx.opts.notes {
            add(&mut e, "notes", i.notes.clone());
        }
        out.push(e);
    }
    out
}

/// `Character.GetObjectName`: the name of the item with this guid.
pub fn object_name(ctx: &Ctx, guid: &str, custom: &str) -> String {
    if !custom.is_empty() {
        return custom.to_owned();
    }
    fn find(e: &Element, guid: &str) -> Option<String> {
        if e.get("guid").eq_ignore_ascii_case(guid) && e.child("name").is_some() {
            return Some(e.get("name"));
        }
        e.elements().filter(|c| c.name != "improvements").find_map(|c| find(c, guid))
    }
    if guid.is_empty() { String::new() } else { find(&ctx.ch.doc, guid).unwrap_or_else(|| guid.to_owned()) }
}

/// `LimitModifier.Print`.
fn limit_modifier(ctx: &Ctx, m: &Element) -> Element {
    let mut out = Element::new("limitmodifier");
    copy(&mut out, m, "guid");
    let bonus = m.get_i32("bonus").unwrap_or(0);
    let cond = m.get("condition");
    let full = format!("{}: {}{bonus}{}", m.get("name"), if bonus > 0 { "+" } else { "" }, if cond.is_empty() { String::new() } else { format!(" ({cond})") });
    add(&mut out, "fullname", full.clone());
    copy(&mut out, m, "name");
    add(&mut out, "fullname_english", full);
    add(&mut out, "name_english", m.get("name"));
    add(&mut out, "bonus", bonus.to_string());
    copy(&mut out, m, "limit");
    add(&mut out, "condition", cond.clone());
    add(&mut out, "condition_english", cond);
    ctx.notes(&mut out, m);
    out
}

/// `<mentorspirits>` (`MentorSpirit.Print`).
pub fn mentor_spirits(ctx: &Ctx) -> Element {
    let mut out = Element::new("mentorspirits");
    for m in ctx.ch.items("mentorspirits", "mentorspirit") {
        let mut e = Element::new("mentorspirit");
        copy(&mut e, m, "guid");
        add(&mut e, "sourceid", m.get("id"));
        copy(&mut e, m, "mentortype");
        add(&mut e, "name", ctx.tr_name("mentors.xml", m));
        add(&mut e, "name_english", m.get("name"));
        for f in ["advantage", "disadvantage"] {
            copy(&mut e, m, f);
        }
        for f in ["advantage", "disadvantage"] {
            add(&mut e, &format!("{f}_english"), m.get(f));
        }
        let choice = |n: &str| m.child(n).map(|c| c.get("name")).unwrap_or_default();
        add(&mut e, "extra", m.get("extra"));
        add(&mut e, "extrachoice1", choice("choice1"));
        add(&mut e, "extrachoice2", choice("choice2"));
        add(&mut e, "extra_english", m.get("extra"));
        add(&mut e, "extrachoice1_english", choice("choice1"));
        add(&mut e, "extrachoice2_english", choice("choice2"));
        source_page(&mut e, m);
        copy_bool(&mut e, m, "mentormask");
        ctx.notes(&mut e, m);
        out.push(e);
    }
    out
}

fn skill_pool(ctx: &Ctx, name: &str) -> i32 {
    ctx.sheet.skills.iter().find(|s| s.name == name).map_or(0, |s| s.pool)
}

/// `Spell.Print`.
pub fn spell(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("spell");
    let mut full = String::new();
    for (flag, key) in [("limited", "String_SpellLimited"), ("alchemical", "String_SpellAlchemical"), ("extended", "String_SpellExtended")] {
        if item.get_bool(flag).unwrap_or(false) {
            full.push_str(&format!(" ({})", ctx.s(key)));
        }
    }
    full.push_str(&extra_suffix(item));
    names(ctx, &mut out, item, "spells.xml", &full);
    // Chummer writes name, fullname, name_english, fullname_english.
    for f in ["descriptors", "category", "type", "range", "damage", "duration", "dv"] {
        if f == "category" {
            let cat = item.get("category");
            add(&mut out, "category", ctx.tr_category("spells.xml", &cat));
            add(&mut out, "category_english", cat);
        } else {
            pair(&mut out, item, f);
        }
    }
    add(&mut out, "alchemy", bool_text(item.get_bool("alchemical").unwrap_or(false)));
    copy_bool(&mut out, item, "limited");
    copy_bool(&mut out, item, "barehandedadept");
    let skill = if item.get_bool("alchemical").unwrap_or(false) {
        "Alchemy"
    } else if item.get("category") == "Rituals" {
        "Ritual Spellcasting"
    } else if item.get("category") == "Enchantments" {
        "Artificing"
    } else {
        "Spellcasting"
    };
    add(&mut out, "dicepool", skill_pool(ctx, skill).to_string());
    source_page(&mut out, item);
    copy(&mut out, item, "extra");
    ctx.notes(&mut out, item);
    out
}

/// `Power.Print`.
pub fn power(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("power");
    let rating = item.get_i32("rating").unwrap_or(0);
    let levels = item.get_bool("levels").unwrap_or(false);
    let full = format!("{}{}", extra_suffix(item), if levels && rating > 0 { format!(" ({rating})") } else { String::new() });
    names(ctx, &mut out, item, "powers.xml", &full);
    copy(&mut out, item, "extra");
    add(&mut out, "extra_english", item.get("extra"));
    let ppl = item.get_f64("pointsperlevel").unwrap_or(0.0);
    add(&mut out, "pointsperlevel", num(ppl));
    add(&mut out, "adeptway", num(item.get_f64("adeptway").unwrap_or(0.0)));
    add(&mut out, "rating", if levels { rating } else { 0 }.to_string());
    let total = ppl * f64::from(rating.max(1)) + item.get_f64("extrapointcost").unwrap_or(0.0) - item.get_f64("freepoints").unwrap_or(0.0);
    add(&mut out, "totalpoints", num(total.max(0.0)));
    pair(&mut out, item, "action");
    source_page(&mut out, item);
    ctx.notes(&mut out, item);
    let mut list = Element::new("enhancements");
    for e in item.child("enhancements").into_iter().flat_map(|c| c.children_named("enhancement")) {
        list.push(enhancement(ctx, e));
    }
    out.push(list);
    out
}

/// `Enhancement.Print`.
pub fn enhancement(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("enhancement");
    names(ctx, &mut out, item, "powers.xml", "");
    source_page(&mut out, item);
    copy(&mut out, item, "improvementsource");
    ctx.notes(&mut out, item);
    out
}

/// `Spirit.Print`.
pub fn spirit(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("spirit");
    copy(&mut out, item, "guid");
    let name = item.get("name");
    add(&mut out, "name", ctx.lang.data_name("critters.xml", "", &name));
    add(&mut out, "name_english", name.clone());
    copy(&mut out, item, "crittername");
    copy_bool(&mut out, item, "fettered");
    copy_bool(&mut out, item, "bound");
    add(&mut out, "services", item.get_i32("services").unwrap_or(0).to_string());
    let force = item.get_i32("force").unwrap_or(0);
    add(&mut out, "force", force.to_string());
    let sprite = item.get("type") == "Sprite";
    add(&mut out, "ratinglabel", ctx.s(if sprite { "String_Rating" } else { "String_Force" }));
    if let Some(rec) = critter(ctx, &name) {
        let mut attrs = Element::new("spiritattributes");
        for a in ["bod", "agi", "rea", "str", "cha", "int", "wil", "log", "ini"] {
            let Some(f) = rec.child_text(&format!("{a}min")) else { continue };
            let v = expr::evaluate_num(&f.replace('F', &force.to_string())).map(standard_round).unwrap_or(0).max(1);
            add(&mut attrs, a, v.to_string());
        }
        out.push(attrs);
        add(&mut out, "source", rec.get("source"));
        add(&mut out, "page", rec.get("page"));
    }
    copy_bool(&mut out, item, "bound");
    add(&mut out, "type", if sprite { "Sprite" } else { "Spirit" });
    ctx.notes(&mut out, item);
    out
}

fn critter(ctx: &Ctx, name: &str) -> Option<Element> {
    let doc = ctx.engine.store.doc("critters.xml").ok()?;
    let found = doc.child("metatypes")?.children_named("metatype").find(|m| m.get("name") == name).cloned();
    found
}

/// `ComplexForm.Print`.
pub fn complex_form(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("complexform");
    names(ctx, &mut out, item, "complexforms.xml", &extra_suffix(item));
    pair(&mut out, item, "duration");
    pair(&mut out, item, "fv");
    pair(&mut out, item, "target");
    source_page(&mut out, item);
    ctx.notes(&mut out, item);
    out
}

/// `AIProgram.Print`.
pub fn ai_program(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("aiprogram");
    names(ctx, &mut out, item, "programs.xml", &extra_suffix(item));
    copy(&mut out, item, "requiresprogram");
    copy(&mut out, item, "category");
    source_page(&mut out, item);
    ctx.notes(&mut out, item);
    out
}

/// `MartialArt.Print` with its `MartialArtTechnique.Print` children.
pub fn martial_art(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("martialart");
    names(ctx, &mut out, item, "martialarts.xml", "");
    source_page(&mut out, item);
    add(&mut out, "cost", item.get_i32("cost").unwrap_or(7).to_string());
    let mut list = Element::new("martialarttechniques");
    for t in item.child("martialarttechniques").into_iter().flat_map(|c| c.children_named("martialarttechnique")) {
        let mut e = Element::new("martialarttechnique");
        copy(&mut e, t, "guid");
        add(&mut e, "sourceid", t.get("sourceid"));
        add(&mut e, "name", ctx.lang.data_name("martialarts.xml", &t.get("sourceid"), &t.get("name")));
        add(&mut e, "name_english", t.get("name"));
        ctx.notes(&mut e, t);
        source_page(&mut e, t);
        list.push(e);
    }
    out.push(list);
    ctx.notes(&mut out, item);
    out
}

/// `Metamagic.Print`.
pub fn metamagic(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("metamagic");
    let file = if item.get("improvementsource") == "Echo" { "echoes.xml" } else { "metamagic.xml" };
    names(ctx, &mut out, item, file, &extra_suffix(item));
    source_page(&mut out, item);
    add(&mut out, "grade", item.get_i32("grade").unwrap_or(0).to_string());
    copy(&mut out, item, "improvementsource");
    ctx.notes(&mut out, item);
    out
}

/// `Art.Print`.
pub fn art(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("art");
    names(ctx, &mut out, item, "metamagic.xml", "");
    source_page(&mut out, item);
    copy(&mut out, item, "improvementsource");
    ctx.notes(&mut out, item);
    out
}

/// The `<initiationgrade>` wrapper: one `InitiationGrade.Print` per
/// grade followed by the metamagics, arts and enhancements of that grade.
pub fn initiation_grades(ctx: &Ctx) -> Element {
    let mut out = Element::new("initiationgrade");
    for g in ctx.ch.items("initiationgrades", "initiationgrade") {
        let grade = g.get_i32("grade").unwrap_or(0);
        let mut e = Element::new("initiationgrade");
        copy(&mut e, g, "guid");
        add(&mut e, "grade", grade.to_string());
        for f in ["group", "ordeal", "schooling", "technomancer"] {
            copy_bool(&mut e, g, f);
        }
        ctx.notes(&mut e, g);
        out.push(e);
        let of_grade = |c: &str, i: &'static str, f: fn(&Ctx, &Element) -> Element| {
            let mut l = Element::new(c);
            for x in ctx.ch.items(c, i).into_iter().filter(|x| x.get_i32("grade").unwrap_or(-1) == grade) {
                l.push(f(ctx, x));
            }
            l
        };
        out.push(of_grade("metamagics", "metamagic", metamagic));
        out.push(of_grade("arts", "art", art));
        out.push(of_grade("enhancements", "enhancement", enhancement));
    }
    out
}

/// `CritterPower.Print`.
pub fn critter_power(ctx: &Ctx, item: &Element) -> Element {
    let mut out = Element::new("critterpower");
    names(ctx, &mut out, item, "critterpowers.xml", &extra_suffix(item));
    copy(&mut out, item, "extra");
    add(&mut out, "extra_english", item.get("extra"));
    let cat = item.get("category");
    add(&mut out, "category", ctx.tr_category("critterpowers.xml", &cat));
    add(&mut out, "category_english", cat);
    for f in ["type", "action", "range", "duration"] {
        pair(&mut out, item, f);
    }
    add(&mut out, "karma", item.get_i32("karma").unwrap_or(0).to_string());
    source_page(&mut out, item);
    ctx.notes(&mut out, item);
    out
}
