//! Bonus handlers that create magic objects (`AddImprovementCollection`:
//! `specificpower`, `selectpowers`, `critterpowers`, `optionalpowers`,
//! `addspell`, `addcomplexform`, `addart`, `addmetamagic`, `addecho`,
//! `addspirit`, `limitspiritcategory`).
//!
//! New objects go to `ctx.out.added`; the improvements they make
//! themselves are sourced to their own guid, the improvement that links
//! them to the granting item to `ctx.src`.

use super::{critterpower, metamagic, power, spell};
use crate::bonus::{BonusSource, Choice, Ctx};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Collect an object's own outcome into the bonus being applied.
fn absorb(ctx: &mut Ctx<'_>, container: &str, el: Element, own: crate::bonus::Outcome) {
    ctx.out.improvements.extend(own.improvements);
    ctx.out.added.extend(own.added);
    ctx.out.added.push((container.into(), el));
}

/// Improvement linking a created object to the granting item.
fn link(ctx: &mut Ctx<'_>, kind: &str, guid: &str) {
    let i = ctx.imp(kind, guid);
    ctx.push(i);
}

// ---------------------------------------------------------------------------
// Adept powers
// ---------------------------------------------------------------------------

/// Hook for `specificpower` and `selectpowers`.
pub fn bonus_power(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    // If the character isn't an adept or mystic adept, nothing happens.
    if !ctx.ch.is_adept() {
        return true;
    }
    match node.name.as_str() {
        "specificpower" => specific_power(ctx, node),
        _ => select_powers(ctx, node),
    }
}

/// A power the character has, or one this bonus already added, with
/// this name (and extra, when given).
fn existing_power(ctx: &Ctx<'_>, name: &str, extra: Option<&str>) -> Option<Element> {
    let fits = |p: &Element| p.get("name") == name && extra.is_none_or(|x| p.get("extra") == x);
    ctx.ch
        .items("powers", "power")
        .into_iter()
        .find(|p| fits(p))
        .cloned()
        .or_else(|| ctx.out.added.iter().filter(|(c, _)| c == "powers").map(|(_, e)| e).find(|p| fits(p)).cloned())
}

/// Create a power for a bonus (`Power.Create(node, 0, bonusoverride)`)
/// unless the character has it: returns (name, extra, levels enabled).
fn grant_power(ctx: &mut Ctx<'_>, rec: Record<'_>, extra: Option<&str>, bonus_override: Option<&Element>, levels: i32) -> (String, String, bool) {
    let name = rec.name();
    let levels_enabled = rec.el().get_bool("levels").unwrap_or(false);
    // The new power would ask for its own selection; Chummer then keeps the
    // existing copy with the same name and extra. Without a prompt, an
    // existing power of this name stands in for that answer.
    if let Some(p) = existing_power(ctx, &name, extra) {
        return (name, p.get("extra"), levels_enabled);
    }
    let guid = crate::items::new_guid();
    let rating = if levels_enabled { levels.max(1) } else { 1 };
    let src = BonusSource { kind: "Power".into(), guid: guid.clone(), name: name.clone(), rating };
    let own = super::apply_bonus(ctx.ch, ctx.store, bonus_override.or(rec.el().child("bonus")), &src, extra);
    let extra = own.selected.clone().or(extra.map(str::to_owned)).unwrap_or_default();
    let st = power::PowerState { rating: 0, extra: extra.clone(), ..Default::default() };
    let el = power::element(rec, &guid, &st, bonus_override);
    absorb(ctx, "powers", el, own);
    (name, extra, levels_enabled)
}

/// The free-levels (or free-points, with `<pointsperlevel>`) improvement.
fn free_levels_improvement(ctx: &mut Ctx<'_>, name: &str, extra: &str, levels: i32, points: bool) {
    let kind = if points { "AdeptPowerFreePoints" } else { "AdeptPowerFreeLevels" };
    let mut i = ctx.imp(kind, name);
    i.unique_name = extra.to_owned();
    i.rating = levels;
    ctx.push(i);
}

/// `specificpower`: grant free levels of a named power.
fn specific_power(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let name = node.get("name");
    if name.is_empty() {
        return true;
    }
    let Ok(doc) = ctx.store.doc("powers.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "powers", "power", &name) else { return false };
    // int.TryParse: expressions such as "Rating" give 0.
    let val: i32 = node.get("val").trim().parse().unwrap_or(0);
    let (pname, extra, levels_enabled) = grant_power(ctx, rec, None, node.child("bonusoverride"), val);
    let levels = if levels_enabled { val } else { 1 };
    free_levels_improvement(ctx, &pname, &extra, levels, !node.get("pointsperlevel").trim().is_empty());
    true
}

/// Split "Name (Extra)" when the whole string is not a power name.
fn power_and_extra<'a>(doc: &'a Element, answer: &str) -> Option<(Record<'a>, Option<String>)> {
    if let Some(r) = crate::data::find(doc, "powers", "power", answer) {
        return Some((r, None));
    }
    let (base, rest) = answer.split_once(" (")?;
    let extra = rest.strip_suffix(')')?;
    crate::data::find(doc, "powers", "power", base).map(|r| (r, Some(extra.to_owned())))
}

/// `selectpowers`: the user picks powers that get free levels or points.
fn select_powers(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(answer) = ctx.answer() else { return false };
    let Ok(doc) = ctx.store.doc("powers.xml") else { return false };
    let Some((rec, extra)) = power_and_extra(&doc, &answer) else { return false };
    for sel in node.children_named("selectpower") {
        let levels = ctx.int(&sel.get("val"));
        let (pname, pextra, _) = grant_power(ctx, rec, extra.as_deref(), node.child("bonusoverride"), levels);
        let display = if pextra.is_empty() { pname.clone() } else { format!("{pname} ({pextra})") };
        ctx.selected = Some(display);
        free_levels_improvement(ctx, &pname, &pextra, levels, !sel.get("pointsperlevel").trim().is_empty());
    }
    true
}

// ---------------------------------------------------------------------------
// Critter powers
// ---------------------------------------------------------------------------

/// Hook for `critterpowers` and `optionalpowers`.
pub fn bonus_critterpowers(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    match node.name.as_str() {
        "critterpowers" => critter_powers(ctx, node),
        _ => optional_powers(ctx, node),
    }
}

/// Create a critter power granted by a bonus (grade -1) and link it.
fn grant_critter_power(ctx: &mut Ctx<'_>, name: &str, rating: i32, forced: Option<&str>) -> bool {
    let Ok(doc) = ctx.store.doc("critterpowers.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "powers", "power", name) else { return false };
    let (el, own) = critterpower::create(ctx.ch, ctx.store, rec, rating, forced, -1);
    let guid = el.get("guid");
    absorb(ctx, "critterpowers", el, own);
    link(ctx, "CritterPower", &guid);
    true
}

/// `critterpowers`: every listed power, with its `rating`/`select`.
fn critter_powers(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    for p in node.children_named("power") {
        let rating = p.attr("rating").filter(|r| !r.is_empty()).map_or(0, |r| ctx.int(r));
        let forced = p.attr("select").map(str::to_owned);
        if !grant_critter_power(ctx, &p.text(), rating, forced.as_deref()) {
            return false;
        }
    }
    true
}

/// `optionalpowers`: pick `count` of the listed powers. The answer names
/// the power; with a single option no answer is needed.
fn optional_powers(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let forced = ctx.answer();
    let options: Vec<(String, Option<String>)> = node
        .children_named("optionalpower")
        .map(|o| (o.text(), o.attr("select").map(str::to_owned)))
        .filter(|(n, _)| forced.as_deref().is_none_or(|f| f == n))
        .collect();
    let count = if forced.is_some() { 1 } else { node.attr("count").map_or(1, |c| ctx.int(c).max(1)) };
    let pick = match (options.as_slice(), forced.is_some()) {
        ([one], _) => one.clone(),
        (many, true) if !many.is_empty() => many[0].clone(),
        _ => return false,
    };
    for _ in 0..count {
        if !grant_critter_power(ctx, &pick.0, 0, pick.1.as_deref()) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Spells, complex forms, arts, metamagics, echoes
// ---------------------------------------------------------------------------

/// Hook for `addspell`, `addcomplexform`, `addart`, `addmetamagic`, `addecho`.
pub fn bonus_add_magic(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    match node.name.as_str() {
        "addspell" => add_spell(ctx, node),
        "addcomplexform" => add_complex_form(ctx, node),
        "addart" => add_art(ctx, node),
        "addecho" => add_metamagic(ctx, node, "Echo"),
        _ => add_metamagic(ctx, node, "Metamagic"),
    }
}

fn attr_true(node: &Element, k: &str) -> bool {
    node.attr(k).is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

/// `addspell`: a named spell, with the variant flags from the attributes.
fn add_spell(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc("spells.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "spells", "spell", node.text().trim()) else { return false };
    // A spell with <selecttext> asks for its text.
    let extra = if rec.el().path("bonus/selecttext").is_some() {
        match ctx.answer() {
            Some(a) => a,
            None => return false,
        }
    } else {
        String::new()
    };
    let guid = crate::items::new_guid();
    let src = BonusSource { kind: "Spell".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
    let own = super::apply_bonus(ctx.ch, ctx.store, rec.el().child("bonus"), &src, Some(&extra));
    let o = spell::SpellOptions {
        limited: attr_true(node, "limited"),
        extended: attr_true(node, "extended"),
        alchemical: attr_true(node, "alchemical"),
        barehanded_adept: attr_true(node, "barehandedadept") || attr_true(node, "usesunarmed"),
        grade: -1,
        ..Default::default()
    };
    let extra = own.selected.clone().filter(|s| !s.is_empty()).unwrap_or(extra);
    absorb(ctx, "spells", spell::element(rec, &guid, &extra, &o), own);
    link(ctx, "Spell", &guid);
    true
}

/// `addcomplexform`: a named complex form (grade -1).
fn add_complex_form(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc("complexforms.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "complexforms", "complexform", node.text().trim()) else { return false };
    let guid = crate::items::new_guid();
    let src = BonusSource { kind: "ComplexForm".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
    let own = super::apply_bonus(ctx.ch, ctx.store, rec.el().child("bonus"), &src, None);
    let extra = own.selected.clone().unwrap_or_default();
    absorb(ctx, "complexforms", super::complexform::element(rec, &guid, &extra, -1), own);
    link(ctx, "ComplexForm", &guid);
    true
}

/// `addart`: a named art from metamagic.xml (grade -1). The
/// requirements check is skipped (treated as `forced`).
fn add_art(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc("metamagic.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "arts", "art", node.text().trim()) else { return false };
    let guid = crate::items::new_guid();
    let src = BonusSource { kind: "Metamagic".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
    let own = super::apply_bonus(ctx.ch, ctx.store, rec.el().child("bonus"), &src, None);
    absorb(ctx, "arts", metamagic::art_element(rec, &guid, "Metamagic", -1), own);
    link(ctx, "Art", &guid);
    true
}

/// `addmetamagic` / `addecho`: a named metamagic or echo (grade -1), its
/// selection forced by the `select` attribute. The requirements check is
/// skipped (treated as `forced`).
fn add_metamagic(ctx: &mut Ctx<'_>, node: &Element, isrc: &str) -> bool {
    let (file, container, item) = metamagic::data_path(isrc);
    let Ok(doc) = ctx.store.doc(file) else { return false };
    let Some(rec) = crate::data::find(&doc, container, item, node.text().trim()) else { return false };
    let forced = node.attr("select").map(str::to_owned);
    let (el, own) = metamagic::create(ctx.ch, ctx.store, rec, isrc, forced.as_deref(), -1);
    let guid = el.get("guid");
    absorb(ctx, "metamagics", el, own);
    link(ctx, isrc, &guid);
    true
}

// ---------------------------------------------------------------------------
// Spirits
// ---------------------------------------------------------------------------

/// Hook for `addspirit` and `limitspiritcategory`.
pub fn bonus_spirit(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let allowed: Vec<String> = node.children_named("spirit").map(Element::text).collect();
    if node.name == "limitspiritcategory" {
        return pick_spirits(ctx, &allowed, "LimitSpiritCategory", 1, false);
    }
    let mut selections = 1;
    if let Some(skill) = node.attr("skill").filter(|s| !s.is_empty()) {
        let divisor = node.attr("ratingdivisor").and_then(|d| d.trim().parse::<i32>().ok()).filter(|d| *d > 0).unwrap_or(1);
        let mut i = ctx.imp("AddSpiritSkill", skill);
        i.rating = divisor;
        ctx.push(i);
        selections = skill_base_rating(ctx, skill) / divisor;
    }
    pick_spirits(ctx, &allowed, "AddSpirit", selections, true)
}

/// `Skill.TotalBaseRating` (learned rating plus its group's).
fn skill_base_rating(ctx: &Ctx<'_>, skill: &str) -> i32 {
    let Ok(doc) = ctx.store.doc("skills.xml") else { return 0 };
    let Some(rec) = crate::data::find(&doc, "skills", "skill", skill) else { return 0 };
    let own = ctx.ch.skills.iter().find(|s| s.suid.eq_ignore_ascii_case(&rec.id())).map_or(0, |s| s.base + s.karma);
    let group = rec.get("skillgroup");
    let grp = ctx.ch.skill_groups.iter().find(|g| !group.is_empty() && g.name == group).map_or(0, |g| g.base + g.karma);
    own + grp
}

/// Spirit names a selection may offer (`AddSpiritOrSprite`): traditions.xml
/// spirits, plus critters.xml "Spirits" for `addspirit`.
fn spirit_options(store: &DataStore, allowed: &[String], with_critters: bool) -> Vec<String> {
    let ok = |n: &str| allowed.is_empty() || allowed.iter().any(|a| a == n);
    let mut out: Vec<String> = store.doc("traditions.xml").ok().map(|d| crate::data::records(&d, "spirits", "spirit").iter().map(|r| r.name()).filter(|n| ok(n)).collect()).unwrap_or_default();
    if with_critters {
        if let Ok(d) = store.doc("critters.xml") {
            out.extend(crate::data::records(&d, "metatypes", "metatype").iter().filter(|r| r.category() == "Spirits").map(|r| r.name()).filter(|n| ok(n)));
        }
    }
    // Most spirits are in both files.
    let mut seen = std::collections::HashSet::new();
    out.retain(|n| seen.insert(n.clone()));
    out
}

/// One improvement per selection; answers are the comma-separated parts of
/// the forced value, consumed in order (`TakeNextForcedValue`) across the
/// spirit nodes of this bonus. A single option needs no answer.
fn pick_spirits(ctx: &mut Ctx<'_>, allowed: &[String], kind: &str, selections: i32, with_critters: bool) -> bool {
    let options = spirit_options(ctx.store, allowed, with_critters);
    let answers: Vec<String> = ctx.answer().map(|a| a.split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let taken = ctx.out.improvements.iter().filter(|i| matches!(i.kind.as_str(), "AddSpirit" | "LimitSpiritCategory")).count();
    for n in 0..selections.max(0) as usize {
        let pick = match answers.get(taken + n) {
            Some(a) if options.contains(a) => a.clone(),
            None if options.len() == 1 => options[0].clone(),
            _ => return false,
        };
        let i = ctx.imp(kind, &pick);
        ctx.push(i);
    }
    true
}

// ---------------------------------------------------------------------------
// Selections
// ---------------------------------------------------------------------------

/// The selection `selectpowers`, `optionalpowers`, `addspirit` and
/// `limitspiritcategory` need (for `bonus::choices`). `addspirit` with
/// several selections takes a comma-separated answer.
pub fn choice(ch: &Character, store: &DataStore, node: &Element, src: &BonusSource) -> Option<Choice> {
    let mk = |prompt: String, options: Vec<String>| Some(Choice { node: node.name.clone(), prompt, options });
    match node.name.as_str() {
        "selectpowers" => {
            if !ch.is_adept() {
                return None;
            }
            let doc = store.doc("powers.xml").ok()?;
            let names = crate::data::records(&doc, "powers", "power").iter().filter(|r| !r.hidden()).map(|r| r.name()).collect();
            mk(format!("Choose an adept power for {}", src.name), names)
        }
        "optionalpowers" => {
            let opts: Vec<String> = node.children_named("optionalpower").map(Element::text).collect();
            (opts.len() > 1).then(|| Choice { node: node.name.clone(), prompt: format!("Choose a critter power for {}", src.name), options: opts })
        }
        _ => {
            let allowed: Vec<String> = node.children_named("spirit").map(Element::text).collect();
            let opts = spirit_options(store, &allowed, node.name == "addspirit");
            (opts.len() > 1).then(|| Choice { node: node.name.clone(), prompt: format!("Choose a spirit type for {}", src.name), options: opts })
        }
    }
}
