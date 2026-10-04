//! Critters and NPCs: `File → New Critter` (`ChummerMainForm.mnuNewCritter`,
//! `SelectMetatypeKarma` with `critters.xml`, `Character.Create`).
//!
//! A critter is a career-mode character with `<iscritter>` and
//! `<ignorerules>` set whose metatype comes from `critters.xml`. Spirits,
//! sprites and other force creatures are built at a chosen Force: their
//! attribute limits, skill ratings and power ratings are expressions of `F`.

use super::{expression_to_dec, expression_to_int};
use crate::bonus::{self, BonusSource};
use crate::chargen::{CHUMMER_APP_VERSION, CHUMMER_MIN_APP_VERSION};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::engine::Engine;
use crate::improvement::Improvement;
use crate::items::{self, magic::critterpower, new_guid, Purchase};
use crate::settings::CharacterSettings;
use crate::xml::Element;

pub const FILE: &str = "critters.xml";
const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// How the Force field of the critter dialog behaves.
#[derive(Debug, Clone, PartialEq)]
pub enum ForceKind {
    /// Not a force creature.
    None,
    /// `<forcecreature/>`: Force (or Level with `<forceislevels/>`), 1-100.
    Force { levels: bool },
    /// `essmax` like `2D6`: the dice are rolled; the field takes the result.
    Dice { dice: i32 },
}

impl ForceKind {
    pub fn max(&self) -> i32 {
        match self {
            ForceKind::None => 0,
            ForceKind::Force { .. } => 100,
            ForceKind::Dice { dice } => dice * 6,
        }
    }
}

/// One entry of the critter list.
#[derive(Debug, Clone)]
pub struct CritterOption {
    pub id: String,
    pub name: String,
    pub category: String,
    pub force: ForceKind,
    /// (id, name) of each metavariant.
    pub metavariants: Vec<(String, String)>,
    pub source: String,
    pub page: String,
}

impl CritterOption {
    /// Spirit categories offer a possession-based tradition
    /// (`chkPossessionBased`, shown for categories ending in "Spirits").
    pub fn offers_possession(&self) -> bool {
        self.category.ends_with("Spirits")
    }
}

/// `SelectMetatypeKarma.RefreshSelectedMetavariant`: the Force field.
pub fn force_kind(node: &Element) -> ForceKind {
    let essmax = node.get("essmax");
    if let Some(pos) = essmax.find("D6") {
        let dice = if pos > 0 { essmax[pos - 1..pos].parse().unwrap_or(1) } else { 1 };
        return ForceKind::Dice { dice };
    }
    if node.child("forcecreature").is_some() {
        return ForceKind::Force { levels: node.child("forceislevels").is_some() };
    }
    ForceKind::None
}

/// The critters the dialog lists: `critters.xml` metatypes from the
/// enabled books (all books when `books` is empty), sorted by name.
pub fn critter_options(store: &DataStore, books: &[String]) -> Vec<CritterOption> {
    let Ok(doc) = store.doc(FILE) else { return Vec::new() };
    let mut v: Vec<CritterOption> = data::records(&doc, "metatypes", "metatype")
        .into_iter()
        .filter(|r| books.is_empty() || books.contains(&r.source()))
        .map(|r| CritterOption {
            id: r.id(),
            name: r.name(),
            category: r.category(),
            force: force_kind(r.el()),
            metavariants: r.el().child("metavariants").map(|m| m.children_named("metavariant").map(|x| (x.get("id"), x.get("name"))).collect()).unwrap_or_default(),
            source: r.source(),
            page: r.page(),
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Categories in `critters.xml` order.
pub fn categories(store: &DataStore) -> Vec<String> {
    store.doc(FILE).map(|d| data::categories(&d)).unwrap_or_default()
}

/// What the critter dialog collects.
#[derive(Debug, Clone, Default)]
pub struct NewCritter {
    pub settings_id: String,
    /// Name or id of the `critters.xml` metatype.
    pub metatype: String,
    pub metavariant: Option<String>,
    /// Force (or level, or rolled dice); 0 for critters without one.
    pub force: i32,
    /// "Possession" or "Inhabitation" for a possession-based tradition.
    pub possession: Option<String>,
    /// Picks for the metatype's `optionalpowers` (one per slot).
    pub optional_powers: Vec<String>,
    pub name: String,
}

/// The node attributes and powers come from: the metavariant if chosen.
fn char_node<'a>(mt: Record<'a>, metavariant: Option<&str>) -> (&'a Element, Option<&'a Element>) {
    let mv = metavariant.filter(|v| !v.is_empty() && *v != EMPTY_GUID).and_then(|v| {
        mt.el().child("metavariants").and_then(|m| m.children_named("metavariant").find(|x| x.get("id").eq_ignore_ascii_case(v) || x.get("name") == v))
    });
    (mv.unwrap_or(mt.el()), mv)
}

/// The `optionalpowers` of a critter's bonus: how many it may pick at
/// `force` and from which powers. `None` when it has none.
pub fn optional_power_slots(store: &DataStore, metatype: &str, metavariant: Option<&str>, force: i32) -> Option<(usize, Vec<String>)> {
    let doc = store.doc(FILE).ok()?;
    let mt = data::find(&doc, "metatypes", "metatype", metatype)?;
    let (node, _) = char_node(mt, metavariant);
    let op = node.path("bonus/optionalpowers")?;
    let options: Vec<String> = op.children_named("optionalpower").map(Element::text).collect();
    // `count` is evaluated with the attributes as they stand right after
    // `AttributeSection.Create`: {MAG} is the metatype minimum.
    let count = match op.attr("count") {
        Some(c) if crate::expr::needs_evaluation(c) => {
            let mag = expression_to_int(Some(&node.get("magmin")), force, 0, 1);
            let s = c.replace("{MAG}", &mag.to_string());
            crate::expr::evaluate_num(&s).map_or(1, crate::expr::standard_round)
        }
        Some(c) => c.trim().parse::<f64>().map_or(1, |v| crate::expr::standard_round(v).max(1)),
        None => 1,
    };
    Some((count.max(0) as usize, options))
}

/// Build a new critter (`mnuNewCritter_Click` + `Character.Create`).
pub fn create(engine: &Engine, spec: &NewCritter) -> Result<Character, String> {
    let settings = engine.settings.resolve(&spec.settings_id).ok_or("unknown settings preset")?.clone();
    let store = engine.store_for(&settings);
    create_with(&store, &settings, spec)
}

/// [`create`] with an explicit data store and settings preset.
pub fn create_with(store: &DataStore, settings: &CharacterSettings, spec: &NewCritter) -> Result<Character, String> {
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let mt = data::find(&doc, "metatypes", "metatype", &spec.metatype).ok_or_else(|| format!("unknown critter {}", spec.metatype))?;
    let (node, mv) = char_node(mt, spec.metavariant.as_deref());
    let force = if force_kind(node) == ForceKind::None && force_kind(mt.el()) == ForceKind::None { 0 } else { spec.force };
    let category = mt.category();

    let mut ch = Character::from_document(skeleton(store, settings, mt, mv, node, force, spec)).map_err(|e| e.to_string())?;
    let src = BonusSource { kind: "Metatype".into(), guid: mt.id(), name: mt.name(), rating: 1 };

    // Metatype bonus. Optional powers are picked separately (below).
    if let Some(b) = node.child("bonus").filter(|b| b.elements().next().is_some()) {
        let mut b = b.clone();
        b.remove_children("optionalpowers");
        let out = bonus::apply(&ch, store, &b, &src, None);
        finish(&mut ch, store, out);
    }
    add_qualities(&mut ch, store, node);
    let powers = store.doc("critterpowers.xml").map_err(|e| e.to_string())?;
    for p in node.child("powers").into_iter().flat_map(|p| p.children_named("power")) {
        let Some(rec) = data::find(&powers, "powers", "power", p.text().trim()) else { continue };
        let rating = expression_to_int(p.attr("rating"), force, 0, 0);
        add_metatype_power(&mut ch, store, rec, rating, p.attr("select"));
    }
    add_unarmed_attack(&mut ch, store);
    add_skills(&mut ch, store, node, force);
    add_complex_forms(&mut ch, store, node);
    add_ware(&mut ch, store, node, force);
    add_gear(&mut ch, store, node, force);

    // Sprites can never have physical attributes.
    if ch.dep_enabled() || category.ends_with("Sprite") || category.ends_with("Sprites") {
        for a in ch.attributes.iter_mut().filter(|a| ["BOD", "AGI", "REA", "STR", "MAG", "MAGAdept"].contains(&a.name.as_str())) {
            (a.metatype_min, a.metatype_max, a.metatype_aug_max, a.base, a.karma) = (0, 0, 0, 0, 0);
        }
    }
    if category == "Spirits" {
        spirit_manifestation(&mut ch, store, &powers, spec.possession.as_deref());
    }
    // `optionalpowers`: one `SelectOptionalPower` per slot, each granted
    // through a bonus forced to the pick (grade -1, a CritterPower improvement).
    if let Some(op) = node.path("bonus/optionalpowers") {
        let (count, options) = optional_power_slots(store, &mt.id(), spec.metavariant.as_deref(), force).unwrap_or_default();
        let picks: Vec<String> = if options.len() == 1 { vec![options[0].clone(); count] } else { spec.optional_powers.iter().take(count).cloned().collect() };
        for pick in picks.iter().filter(|p| options.contains(p)) {
            let mut b = Element::new("bonus");
            b.push(op.clone());
            let out = bonus::apply(&ch, store, &b, &src, Some(pick));
            finish(&mut ch, store, out);
        }
    }
    ch.dirty = true;
    Ok(ch)
}

fn finish(ch: &mut Character, store: &DataStore, out: bonus::Outcome) {
    items::place_added(ch, store, &out.added);
    items::apply_outcome(ch, &out);
}

/// A metatype improvement (`ImprovementManager.CreateImprovement` with
/// source Metatype and no source name).
fn metatype_imp(kind: &str, improved_name: &str, val: f64) -> Improvement {
    Improvement { kind: kind.into(), improved_name: improved_name.into(), source: "Metatype".into(), val, rating: 1, enabled: true, ..Default::default() }
}

/// The saved character: the same skeleton `chargen::create` writes, with
/// the critter's metatype, attribute limits at `force` and critter flags.
fn skeleton(store: &DataStore, settings: &CharacterSettings, mt: Record<'_>, mv: Option<&Element>, node: &Element, force: i32, spec: &NewCritter) -> Element {
    let mut doc = Element::new("character");
    let put = |doc: &mut Element, k: &str, v: String| doc.push(Element::with_text(k, v));
    put(&mut doc, "createdversion", CHUMMER_APP_VERSION.into());
    put(&mut doc, "minimumappversion", CHUMMER_MIN_APP_VERSION.into());
    put(&mut doc, "appversion", CHUMMER_APP_VERSION.into());
    put(&mut doc, "chummerrsversion", env!("CARGO_PKG_VERSION").into());
    put(&mut doc, "gameedition", "SR5".into());
    put(&mut doc, "settings", settings.key());
    put(&mut doc, "buildmethod", settings.build_method());
    let mut sources = Element::new("sources");
    for b in settings.books() {
        sources.push(Element::with_text("source", b));
    }
    doc.push(sources);
    put(&mut doc, "metatype", mt.name());
    put(&mut doc, "metatypeid", mt.id());
    put(&mut doc, "metatypebp", node.get_i32("karma").unwrap_or(0).to_string());
    put(&mut doc, "metavariant", mv.map(|v| v.get("name")).unwrap_or_default());
    put(&mut doc, "metavariantid", mv.map(|v| v.get("id")).unwrap_or_else(|| EMPTY_GUID.into()));
    put(&mut doc, "metatypecategory", mt.category());
    put(&mut doc, "movement", mt.get("movement"));
    for k in ["walk", "run", "sprint"] {
        let v = node.child_text(k).or_else(|| mt.el().child_text(k)).unwrap_or_else(|| if k == "run" { "4/0/0".into() } else { "2/1/0".into() });
        put(&mut doc, k, v);
    }
    for k in ["walkalt", "runalt", "sprintalt"] {
        let base = k.trim_end_matches("alt");
        let v = node.child_text(k).or_else(|| node.child_text(base)).unwrap_or_default();
        put(&mut doc, k, v);
    }
    put(&mut doc, "initiativedice", node.get_i32("initiativedice").unwrap_or(settings.int("mininitiativedice", 1)).to_string());
    for k in ["prioritymetatype", "priorityattributes", "priorityspecial", "priorityskills", "priorityresources", "prioritytalent"] {
        doc.push(Element::new(k));
    }
    doc.push(Element::new("priorityskills"));
    put(&mut doc, "name", spec.name.clone());
    put(&mut doc, "mainmugshotindex", "-1".into());
    doc.push(Element::new("mugshots"));
    for k in ["gender", "age", "eyes", "height", "weight", "skin", "hair", "description", "background", "concept", "notes", "alias", "playername", "gamenotes"] {
        doc.push(Element::new(k));
    }
    put(&mut doc, "primaryarm", "Right".into());
    // `SetIgnoreRules(true)`, `SetIsCritter(true)` before the metatype is chosen.
    put(&mut doc, "ignorerules", "True".into());
    put(&mut doc, "iscritter", "True".into());
    put(&mut doc, "karma", "0".into());
    for k in ["special", "totalspecial", "totalattributes", "edgeused", "contactpoints", "spelllimit", "cfplimit", "ainormalprogramlimit", "aiadvancedprogramlimit"] {
        put(&mut doc, k, "0".into());
    }
    for k in ["streetcred", "notoriety", "publicawareness", "burntstreetcred"] {
        put(&mut doc, k, "0".into());
    }
    // `SetCreated(true)`: critters open in career mode.
    put(&mut doc, "created", "True".into());
    for k in ["nuyen", "startingnuyen", "nuyenbp"] {
        put(&mut doc, k, "0".into());
    }
    for k in ["adept", "magician", "technomancer", "ai", "cyberwaredisabled", "initiationdisabled", "critter"] {
        put(&mut doc, k, "False".into());
    }
    put(&mut doc, "prototypetranshuman", "0".into());

    // `AttributeSection.Create(charNode, intForce)`.
    let mut attrs = Element::new("attributes");
    for name in crate::expr::ATTRIBUTE_NAMES {
        let key = match *name {
            "MAGAdept" => "mag".to_owned(),
            n => n.to_ascii_lowercase(),
        };
        let lim = |suffix: &str| expression_to_int(node.child_text(&format!("{key}{suffix}")).as_deref(), force, 0, 1);
        let mut a = Element::new("attribute");
        a.push(Element::with_text("name", *name));
        a.push(Element::with_text("metatypemin", lim("min").to_string()));
        a.push(Element::with_text("metatypemax", lim("max").to_string()));
        a.push(Element::with_text("metatypeaugmax", lim("aug").to_string()));
        a.push(Element::with_text("base", "0"));
        a.push(Element::with_text("karma", "0"));
        let cat = if crate::attributes::PHYSICAL.contains(name) || crate::attributes::MENTAL.contains(name) { "Standard" } else { "Special" };
        a.push(Element::with_text("metatypecategory", cat));
        attrs.push(a);
    }
    doc.push(attrs);
    for k in ["magenabled", "resenabled", "depenabled"] {
        put(&mut doc, k, "False".into());
    }
    for k in ["initiategrade", "submersiongrade", "physicalcmfilled", "stuncmfilled"] {
        put(&mut doc, k, "0".into());
    }

    // Every non-exotic active skill from the enabled books, at 0.
    let books = settings.books();
    let mut ns = Element::new("newskills");
    ns.push(Element::with_text("skillptsmax", "0"));
    ns.push(Element::with_text("skillgrpsmax", "0"));
    let mut skills = Element::new("skills");
    let mut groups: Vec<String> = Vec::new();
    if let Ok(skills_doc) = store.doc("skills.xml") {
        for r in data::records(&skills_doc, "skills", "skill") {
            if r.get("exotic") == "True" || r.hidden() || (!books.is_empty() && !books.contains(&r.source())) {
                continue;
            }
            skills.push(skill_element(r, ""));
            let g = r.get("skillgroup");
            if !g.is_empty() && !groups.contains(&g) {
                groups.push(g);
            }
        }
    }
    ns.push(skills);
    ns.push(Element::new("knoskills"));
    ns.push(Element::new("skilljackknowledgeskills"));
    let mut gs = Element::new("groups");
    for g in groups {
        let mut e = Element::new("group");
        e.push(Element::with_text("karma", "0"));
        e.push(Element::with_text("base", "0"));
        e.push(Element::with_text("isbroken", "False"));
        e.push(Element::with_text("id", new_guid()));
        e.push(Element::with_text("name", g));
        gs.push(e);
    }
    ns.push(gs);
    doc.push(ns);
    for c in [
        "contacts", "spells", "foci", "stackedfoci", "powers", "spirits", "complexforms", "aiprograms", "martialarts", "limitmodifiers", "armors", "weapons",
        "cyberwares", "qualities", "lifestyles", "gears", "vehicles", "metamagics", "arts", "enhancements", "critterpowers", "initiationgrades", "improvements",
        "sustainedobjects", "drugs", "mentorspirits", "expenses", "gearlocations", "armorlocations", "vehiclelocations", "weaponlocations", "improvementgroups",
        "calendar",
    ] {
        doc.push(Element::new(c));
    }
    doc
}

/// A saved active `<skill>` (`Skill.Save`; exotic skills add `<specific>`).
fn skill_element(r: Record<'_>, specific: &str) -> Element {
    let mut s = Element::new("skill");
    s.push(Element::with_text("guid", new_guid()));
    s.push(Element::with_text("suid", r.id()));
    s.push(Element::with_text("isknowledge", "False"));
    s.push(Element::with_text("skillcategory", r.category()));
    s.push(Element::with_text("karma", "0"));
    s.push(Element::with_text("base", "0"));
    s.push(Element::new("notes"));
    s.push(Element::with_text("name", r.name()));
    s.push(Element::with_text("buywithkarma", "False"));
    if r.get("exotic") == "True" {
        s.push(Element::with_text("specific", specific));
    }
    s
}

/// Metatype qualities (`qualities/*/quality`), not counting toward limits.
fn add_qualities(ch: &mut Character, store: &DataStore, node: &Element) {
    let Ok(qdoc) = store.doc("qualities.xml") else { return };
    let Some(qs) = node.child("qualities") else { return };
    for q in qs.elements().flat_map(|k| k.children_named("quality")) {
        if let Some(rec) = data::find(&qdoc, "qualities", "quality", q.text().trim()) {
            let source = if q.attr("removable").is_some_and(|v| v.eq_ignore_ascii_case("true")) { "MetatypeRemovable" } else { "Metatype" };
            crate::chargen::add_quality_with_source(ch, store, rec, q.attr("select"), source, false);
        }
    }
}

/// A metatype critter power: `CountTowardsLimit = false` and a
/// `CritterPower` improvement linking it to the metatype.
fn add_metatype_power(ch: &mut Character, store: &DataStore, rec: Record<'_>, rating: i32, forced: Option<&str>) -> String {
    let (mut el, out) = critterpower::create(ch, store, rec, rating, forced.filter(|f| !f.is_empty()), 0);
    el.set_child_text("counttowardslimit", "False");
    let guid = el.get("guid");
    ch.items_mut("critterpowers").push(el);
    finish(ch, store, out);
    ch.improvements.list.push(metatype_imp("CritterPower", &guid, 0.0));
    guid
}

fn has_power(ch: &Character, f: impl Fn(&str) -> bool) -> bool {
    ch.items("critterpowers", "critterpower").iter().any(|p| f(&p.get("name")))
}

/// Spirits materialize unless their tradition is possession-based, in which
/// case Materialization makes way for Possession or Inhabitation.
fn spirit_manifestation(ch: &mut Character, store: &DataStore, powers: &Element, possession: Option<&str>) {
    match possession.filter(|p| !p.is_empty()) {
        Some(method) => {
            let mat: Vec<String> = ch.items("critterpowers", "critterpower").iter().filter(|p| p.get("name") == "Materialization").map(|p| p.get("guid")).collect();
            if let Some(g) = mat.first() {
                ch.remove_item("critterpowers", g);
                ch.improvements.list.retain(|i| !(i.kind == "CritterPower" && i.improved_name.eq_ignore_ascii_case(g)));
            }
            if !has_power(ch, |n| n.contains(method)) {
                if let Some(rec) = data::find(powers, "powers", "power", method) {
                    add_metatype_power(ch, store, rec, 0, None);
                }
            }
        }
        None => {
            if !has_power(ch, |n| n == "Materialization" || n.contains("Possession") || n.contains("Inhabitation")) {
                if let Some(rec) = data::find(powers, "powers", "power", "Materialization") {
                    add_metatype_power(ch, store, rec, 0, None);
                }
            }
        }
    }
}

/// Every character has an Unarmed Attack.
fn add_unarmed_attack(ch: &mut Character, store: &DataStore) {
    if ch.items("weapons", "weapon").iter().any(|w| w.get("name") == "Unarmed Attack") {
        return;
    }
    let Ok(wdoc) = store.doc("weapons.xml") else { return };
    let Some(rec) = data::find(&wdoc, "weapons", "weapon", "Unarmed Attack") else { return };
    let p = Purchase { free: true, ..Default::default() };
    if let Ok(g) = items::add("weapon", ch, store, rec, &p) {
        if let Some(w) = items::find_by_guid_mut(&mut ch.doc, &g) {
            // Unarmed Attack can never be removed.
            w.set_child_text("parentid", new_guid());
            w.set_child_text("included", "True");
            w.set_child_text("equipped", "True");
        }
    }
}

/// `skills/skill`, `skills/group` and `skills/knowledge` ratings as
/// metatype `SkillLevel` / `SkillGroupLevel` improvements.
fn add_skills(ch: &mut Character, store: &DataStore, node: &Element, force: i32) {
    let Some(sk) = node.child("skills") else { return };
    let Ok(sdoc) = store.doc("skills.xml") else { return };
    for s in sk.children_named("skill") {
        let name = s.text().trim().to_owned();
        let rating = s.attr("rating").filter(|r| !r.is_empty());
        if let Some(r) = rating {
            ch.improvements.list.push(metatype_imp("SkillLevel", &name, expression_to_dec(Some(r), force)));
        }
        let spec = s.attr("spec").unwrap_or_default().to_owned();
        let Some(rec) = data::find(&sdoc, "skills", "skill", &name) else { continue };
        let id = rec.id();
        if rec.get("exotic") == "True" {
            // `AddExoticSkill(name, spec)`: the spec is the exotic specific.
            let has = ch.skills.iter().any(|x| x.suid.eq_ignore_ascii_case(&id) && x.specific == spec);
            if !has {
                push_skill(ch, skill_element(rec, &spec));
            }
            continue;
        }
        if !ch.skills.iter().any(|x| x.suid.eq_ignore_ascii_case(&id)) {
            if rating.is_none() {
                continue;
            }
            // Asked to improve a skill the books left out: add it.
            push_skill(ch, skill_element(rec, ""));
        }
        if spec.is_empty() {
            continue;
        }
        let Some(skill) = ch.skills.iter_mut().find(|x| x.suid.eq_ignore_ascii_case(&id)) else { continue };
        if skill.specs.iter().any(|x| x.name == spec) {
            continue;
        }
        let guid = new_guid();
        skill.specs.push(crate::skills::Specialization { guid: guid.clone(), name: spec.clone(), free: false, expertise: false });
        let mut i = metatype_imp("SkillSpecialization", &name, 0.0);
        i.unique_name = guid;
        ch.improvements.list.push(i);
    }
    for g in sk.children_named("group") {
        if let Some(r) = g.attr("rating").filter(|r| !r.is_empty()) {
            let v = expression_to_int(Some(r), force, 0, 0);
            ch.improvements.list.push(metatype_imp("SkillGroupLevel", g.text().trim(), f64::from(v)));
        }
    }
    for k in sk.children_named("knowledge") {
        let name = k.text().trim().to_owned();
        let Some(r) = k.attr("rating").filter(|r| !name.is_empty() && !r.is_empty()) else { continue };
        if !ch.knowledge_skills.iter().any(|x| x.name == name) {
            let kind = data::find(&sdoc, "knowledgeskills", "skill", &name).map(|r| r.category()).unwrap_or_else(|| k.attr("category").unwrap_or_default().to_owned());
            crate::chargen::add_knowledge_skill(ch, &name, &kind, false);
        }
        ch.improvements.list.push(metatype_imp("SkillLevel", &name, expression_to_dec(Some(r), force)));
    }
}

fn push_skill(ch: &mut Character, e: Element) {
    ch.skills.push(crate::skills::Skill::from_xml(&e));
    ch.doc.child_or_insert("newskills").child_or_insert("skills").push(e);
}

/// Complex forms the critter comes with (sprites), grade -1.
fn add_complex_forms(ch: &mut Character, store: &DataStore, node: &Element) {
    let Ok(doc) = store.doc("complexforms.xml") else { return };
    for c in node.child("complexforms").into_iter().flat_map(|c| c.children_named("complexform")) {
        let Some(rec) = data::find(&doc, "complexforms", "complexform", c.text().trim()) else { continue };
        let guid = items::magic::complexform::add(ch, store, rec, None);
        if let Some(e) = items::find_by_guid_mut(&mut ch.doc, &guid) {
            e.set_child_text("grade", "-1");
        }
        ch.improvements.list.push(metatype_imp("ComplexForm", &guid, 0.0));
    }
}

/// Cyberware and bioware the critter comes with, grade None, free.
fn add_ware(ch: &mut Character, store: &DataStore, node: &Element, force: i32) {
    for (list, item, tag, file, container) in [("cyberwares", "cyberware", "cyberware", "cyberware.xml", "cyberwares"), ("biowares", "bioware", "bioware", "bioware.xml", "biowares")] {
        let Ok(doc) = store.doc(file) else { continue };
        for n in node.child(list).into_iter().flat_map(|c| c.children_named(item)) {
            let Some(rec) = data::find(&doc, container, item, n.text().trim()) else { continue };
            let p = Purchase {
                rating: expression_to_int(n.attr("rating"), force, 0, 0),
                grade: Some("None".into()),
                answer: n.attr("select").filter(|s| !s.is_empty()).map(str::to_owned),
                free: true,
                cost_multiplier: 1.0,
                ..Default::default()
            };
            if let Ok(guid) = items::add(tag, ch, store, rec, &p) {
                ch.improvements.list.push(metatype_imp("FreeWare", &guid, 0.0));
            }
        }
    }
}

/// Gear the critter comes with (programs for A.I.s), at no cost.
fn add_gear(ch: &mut Character, store: &DataStore, node: &Element, force: i32) {
    let Ok(doc) = store.doc("gear.xml") else { return };
    for g in node.child("gears").into_iter().flat_map(|c| c.children_named("gear")) {
        let (name, cat) = (g.get("name"), g.get("category"));
        let Some(rec) = doc.child("gears").and_then(|c| c.children_named("gear").find(|x| x.get("name") == name && x.get("category") == cat)).map(Record) else { continue };
        let p = Purchase {
            rating: if g.child("rating").is_some() { expression_to_int(g.child_text("rating").as_deref(), force, 0, 0) } else { 1 },
            qty: if g.child("quantity").is_some() { expression_to_dec(g.child_text("quantity").as_deref(), force) } else { 1.0 },
            answer: g.attr("select").filter(|s| !s.is_empty()).map(str::to_owned),
            free: true,
            cost_multiplier: 1.0,
            ..Default::default()
        };
        if let Ok(guid) = items::add("gear", ch, store, rec, &p) {
            if let Some(e) = items::find_by_guid_mut(&mut ch.doc, &guid) {
                e.set_child_text("cost", "0");
                e.set_child_text("parentid", new_guid());
            }
            ch.improvements.list.push(metatype_imp("Gear", &guid, 0.0));
        }
    }
}
