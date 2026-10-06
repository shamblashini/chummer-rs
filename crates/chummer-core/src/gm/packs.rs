//! PACKS kits: `packs.xml` (Run Faster p. 63 "Prepackaged Advantages for
//! Character Kits"), `SelectPACKSKit`, `CharacterCreate.AddPACKSKit` and
//! `CreatePACKSKit`.
//!
//! Chummer merges the user's `custom_*_packs.xml` files from its `packs`
//! folder into `packs.xml`; here that folder is [`packs_dir`]. Kits are a
//! creation-mode tool: their items are bought at their normal cost and the
//! kit's `nuyenbp` is added to the karma spent on nuyen.

use std::path::{Path, PathBuf};

use crate::calc::Sheet;
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::improvement::fmt_num;
use crate::items::{self, find_by_guid_mut, Purchase};
use crate::settings::CharacterSettings;
use crate::xml::{self, Element, Node};

pub const FILE: &str = "packs.xml";
pub const CUSTOM: &str = "Custom";

/// The user's PACKS folder: `$XDG_DATA_HOME/chummer-rs/packs`, next to the
/// user custom data root.
pub fn packs_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("chummer-rs").join("packs"))
}

/// `packs.xml` with the custom data and the PACKS folder's files merged in
/// (`CharacterSettings.LoadData("packs.xml")`).
pub fn load(store: &DataStore, dir: Option<&Path>) -> Element {
    let mut doc = store.doc(FILE).map(|d| (*d).clone()).unwrap_or_else(|_| Element::new("chummer"));
    if let Some(d) = dir.filter(|d| d.is_dir()) {
        let mut report = crate::custom_data::MergeReport::default();
        crate::custom_data::apply_directory(&mut doc, FILE, d, &mut report);
    }
    doc
}

/// Kit categories in file order.
pub fn categories(doc: &Element) -> Vec<String> {
    data::categories(doc)
}

/// (name, category) of every kit.
pub fn kits(doc: &Element) -> Vec<(String, String)> {
    data::records(doc, "packs", "pack").into_iter().map(|r| (r.name(), r.category())).collect()
}

pub fn find_kit<'a>(doc: &'a Element, name: &str, category: &str) -> Option<&'a Element> {
    doc.child("packs")?.children_named("pack").find(|p| p.get("name") == name && p.get("category") == category)
}

/// A data record from the enabled books (`TryGetNodeByNameOrId` with the
/// settings' `BookXPath`).
fn find_in<'a>(doc: &'a Element, container: &str, item: &'a str, name: &str, books: &[String]) -> Option<Record<'a>> {
    let ok = |e: &Element| books.is_empty() || books.contains(&e.get("source"));
    let c = doc.child(container)?;
    c.children_named(item)
        .find(|e| e.get("id").eq_ignore_ascii_case(name) && ok(e))
        .or_else(|| c.children_named(item).find(|e| e.get("name") == name && ok(e)))
        .map(Record)
}

/// The kit's `<name>` text and its `select` attribute (gear and spells
/// write `<name select="...">`; qualities put `select` on the item).
fn name_select(e: &Element) -> (String, Option<String>) {
    match e.child("name") {
        Some(n) => (n.text().trim().to_owned(), n.attr("select").or(e.attr("select")).filter(|s| !s.is_empty()).map(str::to_owned)),
        None => (e.text().trim().to_owned(), e.attr("select").filter(|s| !s.is_empty()).map(str::to_owned)),
    }
}

// ---------------------------------------------------------------------------
// Preview (`SelectPACKSKit.lstKits_SelectedIndexChanged`)
// ---------------------------------------------------------------------------

/// The kit's contents by section, as (English section label, lines).
pub fn contents(kit: &Element) -> Vec<(&'static str, Vec<String>)> {
    let mut v: Vec<(&'static str, Vec<String>)> = Vec::new();
    for node in kit.elements() {
        let lines: Vec<String> = match node.name.as_str() {
            "attributes" => node.elements().map(|a| format!("{} {}", a.name.to_ascii_uppercase(), a.text().trim())).collect(),
            "qualities" => node.elements().flat_map(|k| k.children_named("quality")).map(|q| with_select(q.text().trim(), q.attr("select"))).collect(),
            "nuyenbp" => vec![format!("{} {}", "Starting Nuyen Karma:", node.text().trim())],
            "skills" => node.elements().map(|s| format!("{} {}", s.get("name"), s.get("rating"))).collect(),
            "knowledgeskills" => node.children_named("skill").map(|s| format!("{} {}", s.get("name"), s.get("rating"))).collect(),
            "selectmartialart" => vec![node.attr("select").unwrap_or_default().to_owned()],
            "martialarts" => node.children_named("martialart").map(|m| m.get("name")).collect(),
            "complexforms" | "programs" | "spells" | "powers" => node.elements().map(|e| {
                let (n, s) = name_select(e);
                with_select(&n, s.as_deref())
            }).collect(),
            "spirits" => node.children_named("spirit").map(|s| format!("{} ({} {})", s.get("name"), "Force", s.get("force"))).collect(),
            "lifestyles" => node.children_named("lifestyle").map(|l| format!("{} ({})", l.get("name"), l.get("baselifestyle"))).collect(),
            "cyberwares" | "biowares" | "armors" | "weapons" | "vehicles" => node.elements().map(item_line).collect(),
            "gears" => node.children_named("gear").map(gear_line).collect(),
            _ => continue,
        };
        let label = match node.name.as_str() {
            "attributes" => "Attributes",
            "qualities" => "Qualities",
            "nuyenbp" => "Nuyen",
            "skills" => "Skills",
            "knowledgeskills" => "Knowledge Skills",
            "selectmartialart" => "Select Martial Art",
            "martialarts" => "Martial Arts",
            "powers" => "Powers",
            "complexforms" | "programs" => "Programs",
            "spells" => "Spells",
            "spirits" => "Spirits",
            "lifestyles" => "Lifestyles",
            "cyberwares" => "Cyberware",
            "biowares" => "Bioware",
            "armors" => "Armor",
            "weapons" => "Weapons",
            "gears" => "Gear",
            _ => "Vehicles",
        };
        v.push((label, lines));
    }
    v
}

fn with_select(name: &str, select: Option<&str>) -> String {
    match select.filter(|s| !s.is_empty()) {
        Some(s) => format!("{name} ({s})"),
        None => name.to_owned(),
    }
}

fn item_line(e: &Element) -> String {
    let mut s = e.get("name");
    if let Some(r) = e.child_text("rating").filter(|r| !r.is_empty()) {
        s += &format!(" R{r}");
    }
    if let Some(g) = e.child_text("grade").filter(|g| !g.is_empty()) {
        s += &format!(" ({g})");
    }
    s
}

fn gear_line(g: &Element) -> String {
    let (n, sel) = name_select(g);
    let mut s = with_select(&n, sel.as_deref());
    if let Some(r) = g.child_text("rating").filter(|r| !r.is_empty()) {
        s += &format!(" R{r}");
    }
    if let Some(q) = g.child_text("qty").filter(|q| !q.is_empty()) {
        s += &format!(" ×{q}");
    }
    s
}

// ---------------------------------------------------------------------------
// Applying a kit (`CharacterCreate.AddPACKSKit`)
// ---------------------------------------------------------------------------

/// What applying a kit did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KitReport {
    /// "Kind: name" of every item added.
    pub added: Vec<String>,
    /// Entries that could not be applied, with the reason.
    pub skipped: Vec<String>,
}

struct Applier<'a> {
    store: &'a DataStore,
    books: Vec<String>,
    report: KitReport,
}

impl Applier<'_> {
    fn doc(&self, file: &str) -> Option<std::sync::Arc<Element>> {
        self.store.doc(file).ok()
    }

    fn ok(&mut self, kind: &str, name: &str, r: Result<String, String>) -> Option<String> {
        match r {
            Ok(g) => {
                self.report.added.push(format!("{kind}: {name}"));
                Some(g)
            }
            Err(e) => {
                self.report.skipped.push(format!("{kind}: {name} ({e})"));
                None
            }
        }
    }

    fn missing(&mut self, kind: &str, name: &str) {
        self.report.skipped.push(format!("{kind}: {name} (not in the enabled books)"));
    }
}

/// Apply a kit to a creation-mode character, in `AddPACKSKit` order, with
/// the kit's attributes, skills, knowledge skills and adept powers.
// chummer-rs deviates from Chummer (LB-09): Chummer 5.226 lists a kit's
// attributes, skills, knowledge skills and powers but applies none of them
// (Chummer 5.193 still applied attributes; skills and powers were TODO).
// Here they are applied, and kits differ from Chummer 5.226.
pub fn apply(ch: &mut Character, store: &DataStore, settings: Option<&CharacterSettings>, kit: &Element) -> KitReport {
    let mut a = Applier { store, books: settings.map(CharacterSettings::books).unwrap_or_default(), report: KitReport::default() };
    let rules = settings.map(crate::calc::Rules::from_settings).unwrap_or_default();
    qualities(ch, &mut a, kit);
    if let Some(n) = kit.child("attributes") {
        attributes(ch, &mut a, &rules, settings, n);
    }
    if let Some(n) = kit.child("skills") {
        skills(ch, &mut a, &rules, n);
    }
    if let Some(n) = kit.child("knowledgeskills") {
        knowledge_skills(ch, &mut a, &rules, n);
    }
    powers(ch, &mut a, kit);
    if let Some(n) = kit.child("selectmartialart") {
        select_martial_art(ch, &mut a, n.attr("select").unwrap_or_default());
    }
    martial_arts(ch, &mut a, kit);
    complex_forms(ch, &mut a, kit);
    programs(ch, &mut a, kit);
    spells(ch, &mut a, kit);
    spirits(ch, &mut a, kit);
    lifestyles(ch, &mut a, kit);
    if let Some(bp) = kit.child_text("nuyenbp").and_then(|t| t.trim().parse::<f64>().ok()) {
        modify_nuyen_bp(ch, settings, bp);
    }
    armors(ch, &mut a, kit);
    weapons(ch, &mut a, kit);
    for (list, item) in [("cyberwares", "cyberware"), ("biowares", "bioware")] {
        for w in kit.child(list).into_iter().flat_map(|c| c.children_named(item)) {
            ware(ch, &mut a, w, Parent::Character);
        }
    }
    for g in kit.child("gears").into_iter().flat_map(|c| c.children_named("gear")) {
        gear(ch, &mut a, g, None);
    }
    vehicles(ch, &mut a, kit);
    ch.dirty = true;
    a.report
}

/// The character's computed values, with the skill catalog (skill groups
/// and rating modifiers need it).
fn sheet_of(ch: &Character, a: &Applier<'_>, rules: &crate::calc::Rules) -> Sheet {
    let catalog = crate::calc::SkillCatalog::load(a.store).ok();
    crate::calc::compute(ch, rules, Some(a.store), catalog.as_ref())
}

/// Split `points` between creation points (at most `left`; none outside
/// priority builds) and karma.
fn split_points(ch: &Character, points: i32, left: i32) -> (i32, i32) {
    let left = if crate::character::uses_priority_tables(&ch.field("buildmethod")) { left.max(0) } else { 0 };
    let base = points.min(left);
    (base, points - base)
}

/// `<attributes>`: each value is `value - (metatype minimum - 1)`, as
/// `CreatePACKSKit` writes it. Like Chummer 5's old `AddPACKSKit`, the
/// listed attributes are first reset to their minimum. The levels above
/// the minimum are bought with attribute (or special attribute) points
/// while there are any, then with karma; values are capped at the
/// maximum, and at one below it once `maxnumbermaxattributescreate`
/// standard attributes are at their maximum.
fn attributes(ch: &mut Character, a: &mut Applier<'_>, rules: &crate::calc::Rules, settings: Option<&CharacterSettings>, node: &Element) {
    let name_of = |tag: &str| if tag.eq_ignore_ascii_case("magadept") { "MAGAdept".to_owned() } else { tag.to_ascii_uppercase() };
    let enabled = |ch: &Character, n: &str| match n {
        "MAG" | "MAGAdept" => ch.mag_enabled(),
        "RES" => ch.res_enabled(),
        "DEP" => ch.dep_enabled(),
        _ => true,
    };
    let wanted: Vec<(String, i32)> = node.elements().filter_map(|e| Some((name_of(&e.name), e.text().trim().parse::<i32>().ok()?))).collect();
    for (n, _) in &wanted {
        if let Some(at) = ch.attribute_mut(n) {
            at.base = 0;
            at.karma = 0;
        }
    }
    let sheet = sheet_of(ch, a, rules);
    let allowed_at_max = settings.map_or(1, |s| s.int("maxnumbermaxattributescreate", 1));
    let standard: Vec<&str> = crate::attributes::PHYSICAL.iter().chain(crate::attributes::MENTAL).copied().collect();
    let mut at_max = 0;
    for (n, kit_value) in wanted {
        let Some(v) = sheet.attributes.iter().find(|x| x.name == n) else {
            a.report.skipped.push(format!("Attribute: {n} (not on the character)"));
            continue;
        };
        if !enabled(ch, &n) {
            a.report.skipped.push(format!("Attribute: {n} (not enabled)"));
            continue;
        }
        let fixed = v.value - v.base - v.karma;
        let mut target = kit_value + v.metatype_min - 1;
        let is_standard = standard.contains(&n.as_str());
        if is_standard && target >= v.total_max {
            if at_max >= allowed_at_max {
                target = v.total_max - 1;
                a.report.skipped.push(format!("Attribute: {n} lowered to {target} (only {allowed_at_max} at the maximum)"));
            } else {
                at_max += 1;
            }
        }
        if target > v.total_max {
            a.report.skipped.push(format!("Attribute: {n} capped at {}", v.total_max));
        }
        let points = (target.min(v.total_max) - fixed).max(0);
        let pool: &[&str] = if is_standard { &standard } else { &["EDG", "MAG", "MAGAdept", "RES", "DEP"] };
        let total = ch.doc.get_i32(if is_standard { "totalattributes" } else { "totalspecial" }).unwrap_or(0);
        let used: i32 = ch.attributes.iter().filter(|x| pool.contains(&x.name.as_str())).map(|x| x.base).sum();
        let (base, karma) = split_points(ch, points, total - used);
        if let Some(at) = ch.attribute_mut(&n) {
            at.base = base;
            at.karma = karma;
        }
        a.report.added.push(format!("Attribute: {n} {}", fixed + points));
    }
}

/// `<skills>`: `<skillgroup>` (name, rating) first, then `<skill>`
/// (name, rating, spec). Ratings are capped at the creation maximum and
/// bought with skill (group) points while there are any, then karma.
fn skills(ch: &mut Character, a: &mut Applier<'_>, rules: &crate::calc::Rules, node: &Element) {
    let cap = rules.max_skill_rating_create;
    let rating = |e: &Element| e.get_i32("rating").unwrap_or(0).clamp(0, cap);
    for g in node.children_named("skillgroup") {
        let name = g.get("name");
        let group_max = ch.doc.child("newskills").and_then(|n| n.get_i32("skillgrpsmax")).unwrap_or(0);
        let used: i32 = ch.skill_groups.iter().filter(|x| x.name != name).map(|x| x.base).sum();
        let (base, karma) = split_points(ch, rating(g), group_max - used);
        let Some(sg) = ch.skill_groups.iter_mut().find(|x| x.name == name) else {
            a.missing("Skill Group", &name);
            continue;
        };
        sg.base = base;
        sg.karma = karma;
        a.report.added.push(format!("Skill Group: {name} {}", base + karma));
    }
    let Some(sdoc) = a.doc("skills.xml") else { return };
    for k in node.children_named("skill") {
        let name = k.get("name");
        let Some(rec) = data::find(&sdoc, "skills", "skill", &name) else {
            a.missing("Skill", &name);
            continue;
        };
        let id = rec.id();
        if !ch.skills.iter().any(|x| x.suid.eq_ignore_ascii_case(&id)) {
            super::critter::push_skill(ch, super::critter::skill_element(rec, ""));
        }
        let Some(guid) = ch.skills.iter().find(|x| x.suid.eq_ignore_ascii_case(&id)).map(|x| x.guid.clone()) else { continue };
        if let Some(sk) = ch.skills.iter_mut().find(|x| x.guid == guid) {
            sk.base = 0;
            sk.karma = 0;
        }
        let sheet = sheet_of(ch, a, rules);
        // The skill's own points are 0 here, so the rest of its rating
        // (group, improvements) is fixed.
        let fixed = sheet.skills.iter().find(|x| x.guid == guid).map_or(0, |x| x.total_base);
        let points = (rating(k) - fixed).max(0);
        // A group with base points replaces the skill's own base
        // (`Skill.Base`), so its extra levels are karma.
        let group = rec.get("skillgroup");
        let group_has_base = !group.is_empty() && ch.skill_groups.iter().any(|g| g.name == group && g.base > 0);
        let left = if group_has_base { 0 } else { skill_points_left(ch, sheet.knowledge_points) };
        let (base, karma) = split_points(ch, points, left);
        let spec = k.get("spec");
        if let Some(sk) = ch.skills.iter_mut().find(|x| x.guid == guid) {
            sk.base = base;
            sk.karma = karma;
            if !spec.is_empty() && !sk.specs.iter().any(|x| x.name == spec) {
                crate::chargen::add_specialization(ch, &guid, &spec);
            }
        }
        a.report.added.push(format!("Skill: {name} {}", fixed + points));
    }
}

/// Skill points left (as `chargen::budget_with` counts them).
fn skill_points_left(ch: &Character, knowledge_points: i32) -> i32 {
    let max = ch.doc.child("newskills").and_then(|n| n.get_i32("skillptsmax")).unwrap_or(0);
    let specs = |s: &[crate::skills::Specialization]| s.iter().filter(|x| !x.free).count() as i32;
    let active: i32 = ch.skills.iter().map(|s| s.base + if s.buy_with_karma { 0 } else { specs(&s.specs) }).sum();
    let kno: i32 = ch.knowledge_skills.iter().filter(|k| !k.native_language).map(|k| k.base + specs(&k.specs)).sum();
    max - active - (kno - knowledge_points).max(0)
}

/// `<knowledgeskills><skill>` (name, rating, spec, category): added when
/// missing, bought with free knowledge points while there are any, then
/// karma.
fn knowledge_skills(ch: &mut Character, a: &mut Applier<'_>, rules: &crate::calc::Rules, node: &Element) {
    let cap = rules.max_skill_rating_create;
    let sdoc = a.doc("skills.xml");
    for k in node.children_named("skill") {
        let name = k.get("name");
        if name.is_empty() {
            continue;
        }
        if !ch.knowledge_skills.iter().any(|x| x.name == name) {
            let kind = sdoc.as_ref().and_then(|d| data::find(d, "knowledgeskills", "skill", &name).map(|r| r.category())).unwrap_or_else(|| k.get("category"));
            crate::chargen::add_knowledge_skill(ch, &name, &kind, false);
        }
        let sheet = sheet_of(ch, a, rules);
        let used: i32 = ch.knowledge_skills.iter().filter(|x| !x.native_language && x.name != name).map(|x| x.base + x.specs.iter().filter(|s| !s.free).count() as i32).sum();
        let (base, karma) = split_points(ch, k.get_i32("rating").unwrap_or(0).clamp(0, cap), sheet.knowledge_points - used);
        let spec = k.get("spec");
        if let Some(ks) = ch.knowledge_skills.iter_mut().find(|x| x.name == name) {
            ks.base = base;
            ks.karma = karma;
            if !spec.is_empty() && !ks.specs.iter().any(|x| x.name == spec) {
                ks.specs.push(crate::skills::Specialization { guid: crate::items::new_guid(), name: spec, free: false, expertise: false });
            }
        }
        a.report.added.push(format!("Knowledge Skill: {name} {}", base + karma));
    }
}

/// `<powers><power>` (name with `select`, rating): adept powers, for
/// adepts and mystic adepts.
fn powers(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let list: Vec<&Element> = kit.child("powers").into_iter().flat_map(|k| k.children_named("power")).collect();
    if list.is_empty() {
        return;
    }
    if !ch.is_adept() {
        a.report.skipped.push("Powers: the character is not an adept".into());
        return;
    }
    let Some(doc) = a.doc("powers.xml") else { return };
    for p in list {
        let (name, select) = name_select(p);
        let Some(rec) = find_in(&doc, "powers", "power", &name, &a.books) else {
            a.missing("Power", &name);
            continue;
        };
        let purchase = Purchase { rating: p.get_i32("rating").unwrap_or(0), answer: select, ..purchase(0, None) };
        a.ok("Power", &name, items::add("power", ch, a.store, rec, &purchase));
    }
}

/// `Character.ModifyNuyenBP`: add, clamped to 0 .. the maximum karma for nuyen.
fn modify_nuyen_bp(ch: &mut Character, settings: Option<&CharacterSettings>, value: f64) {
    if value == 0.0 {
        return;
    }
    let max = f64::from(settings.map_or(10, |s| s.int("nuyenmaxbp", 10))) + ch.improvements.val("NuyenMaxBP", None);
    let cur = ch.doc.get_f64("nuyenbp").unwrap_or(0.0);
    ch.set_field("nuyenbp", fmt_num((cur + value).min(max).max(0.0)));
}

fn qualities(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("qualities.xml") else { return };
    for q in kit.child("qualities").into_iter().flat_map(|k| k.elements()).flat_map(|k| k.children_named("quality")) {
        let name = q.text().trim().to_owned();
        let Some(rec) = find_in(&doc, "qualities", "quality", &name, &a.books) else {
            a.missing("Quality", &name);
            continue;
        };
        crate::chargen::add_quality(ch, a.store, rec, q.attr("select").filter(|s| !s.is_empty()));
        a.report.added.push(format!("Quality: {name}"));
    }
}

/// `selectmartialart`: Chummer asks with the `select` value forced, which
/// adds that art; without one the user picks from the Martial Arts tab.
fn select_martial_art(ch: &mut Character, a: &mut Applier<'_>, forced: &str) {
    let rec = a.doc("martialarts.xml").and_then(|d| find_in(&d, "martialarts", "martialart", forced, &a.books).map(|r| r.el().clone()));
    match rec.filter(|_| !forced.is_empty()) {
        Some(rec) => {
            items::magic::martialart::add(ch, a.store, Record(&rec), None);
            a.report.added.push(format!("Martial Art: {forced}"));
        }
        None => a.report.skipped.push(format!("Select Martial Art: {forced} (add it from the Martial Arts tab)")),
    }
}

fn martial_arts(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("martialarts.xml") else { return };
    for m in kit.child("martialarts").into_iter().flat_map(|k| k.children_named("martialart")) {
        let name = m.get("name");
        let Some(rec) = find_in(&doc, "martialarts", "martialart", &name, &a.books) else {
            a.missing("Martial Art", &name);
            continue;
        };
        let guid = items::magic::martialart::add(ch, a.store, rec, None);
        a.report.added.push(format!("Martial Art: {name}"));
        for t in m.child("techniques").into_iter().flat_map(|t| t.children_named("technique")) {
            let tname = t.child_text("name").unwrap_or_else(|| t.text()).trim().to_owned();
            let r = items::magic::martialart::add_technique(ch, a.store, &guid, &tname);
            a.ok("Technique", &tname, r);
        }
    }
}

fn complex_forms(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("complexforms.xml") else { return };
    for c in kit.child("complexforms").into_iter().flat_map(|k| k.children_named("complexform")) {
        let (name, select) = name_select(c);
        let Some(rec) = find_in(&doc, "complexforms", "complexform", &name, &a.books) else {
            a.missing("Complex Form", &name);
            continue;
        };
        items::magic::complexform::add(ch, a.store, rec, select.as_deref());
        a.report.added.push(format!("Complex Form: {name}"));
    }
}

/// `<programs><program><name>`: A.I. programs, bought like the player's
/// own (`candelete` True, no selection forced).
fn programs(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("programs.xml") else { return };
    for p in kit.child("programs").into_iter().flat_map(|k| k.children_named("program")) {
        let (name, _) = name_select(p);
        let Some(rec) = find_in(&doc, "programs", "program", &name, &a.books) else {
            a.missing("Program", &name);
            continue;
        };
        items::aiprogram::add(ch, a.store, rec, None, true);
        a.report.added.push(format!("Program: {name}"));
    }
}

fn spells(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("spells.xml") else { return };
    for s in kit.child("spells").into_iter().flat_map(|k| k.children_named("spell")) {
        let (name, select) = name_select(s);
        let category = s.get("category");
        // Make sure the spell has not already been added to the character.
        if ch.items("spells", "spell").iter().any(|x| x.get("name") == name && x.get("category") == category) {
            continue;
        }
        let rec = doc.child("spells").and_then(|c| {
            c.children_named("spell")
                .find(|e| (e.get("name") == name || e.get("id").eq_ignore_ascii_case(&name)) && e.get("category") == category && (a.books.is_empty() || a.books.contains(&e.get("source"))))
        });
        let Some(rec) = rec.map(Record) else {
            a.missing("Spell", &name);
            continue;
        };
        items::magic::spell::add(ch, a.store, rec, select.as_deref(), &Default::default());
        a.report.added.push(format!("Spell: {name}"));
    }
}

fn spirits(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("critters.xml") else { return };
    for s in kit.child("spirits").into_iter().flat_map(|k| k.children_named("spirit")) {
        let name = s.get("name");
        let Some(rec) = data::find(&doc, "metatypes", "metatype", &name) else {
            a.missing("Spirit", &name);
            continue;
        };
        items::magic::spirit::add(ch, rec, s.get_i32("force").unwrap_or(0), s.get_i32("services").unwrap_or(0), true);
        a.report.added.push(format!("Spirit: {name}"));
    }
}

fn lifestyles(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("lifestyles.xml") else { return };
    for l in kit.child("lifestyles").into_iter().flat_map(|k| k.children_named("lifestyle")) {
        let base = l.get("baselifestyle");
        let Some(rec) = data::find(&doc, "lifestyles", "lifestyle", &base) else {
            a.missing("Lifestyle", &base);
            continue;
        };
        let o = items::lifestyle::Options {
            name: l.get("name"),
            comforts: l.get_i32("comforts").unwrap_or(0),
            security: l.get_i32("security").unwrap_or(0),
            area: l.get_i32("area").unwrap_or(0),
            ..Default::default()
        };
        let Some(guid) = a.ok("Lifestyle", &base, items::lifestyle::add_with(ch, a.store, rec, &o)) else { continue };
        for q in l.child("qualities").into_iter().flat_map(|q| q.children_named("quality")) {
            let qname = q.text().trim().to_owned();
            let Some(qrec) = data::find(&doc, "qualities", "quality", &qname) else {
                a.missing("Lifestyle quality", &qname);
                continue;
            };
            let r = items::lifestyle::add_quality(ch, a.store, &guid, qrec, None, false);
            a.ok("Lifestyle quality", &qname, r);
        }
    }
}

fn purchase(rating: i32, parent: Option<&str>) -> Purchase {
    Purchase { rating, parent: parent.map(str::to_owned), cost_multiplier: 1.0, ..Default::default() }
}

fn armors(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("armor.xml") else { return };
    for e in kit.child("armors").into_iter().flat_map(|k| k.children_named("armor")) {
        let name = e.get("name");
        let Some(rec) = find_in(&doc, "armors", "armor", &name, &a.books) else {
            a.missing("Armor", &name);
            continue;
        };
        let mut rating = e.get_i32("rating").unwrap_or(0);
        let Some(guid) = a.ok("Armor", &name, items::add("armor", ch, a.store, rec, &purchase(rating, None))) else { continue };
        for m in e.child("mods").into_iter().flat_map(|m| m.children_named("mod")) {
            let mname = m.get("name");
            let Some(mrec) = find_in(&doc, "mods", "mod", &mname, &a.books) else {
                a.missing("Armor mod", &mname);
                continue;
            };
            // A mod without a rating takes the armor's (or the last mod's).
            if m.child("rating").is_some() {
                rating = m.get_i32("rating").unwrap_or(0);
            }
            if let Some(mg) = a.ok("Armor mod", &mname, items::add("armormod", ch, a.store, mrec, &purchase(rating, Some(&guid)))) {
                for g in m.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
                    gear(ch, a, g, Some(&mg));
                }
            }
        }
        for g in e.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
            gear(ch, a, g, Some(&guid));
        }
    }
}

/// A kit weapon with its accessories and underbarrel weapon. `parent` is
/// the weapon an underbarrel goes on.
fn weapon(ch: &mut Character, a: &mut Applier<'_>, doc: &Element, e: &Element, name: &str, parent: Option<&str>) -> Option<String> {
    let Some(rec) = find_in(doc, "weapons", "weapon", name, &a.books) else {
        a.missing("Weapon", name);
        return None;
    };
    let guid = a.ok("Weapon", name, items::add("weapon", ch, a.store, rec, &purchase(0, parent)))?;
    for acc in e.child("accessories").into_iter().flat_map(|x| x.children_named("accessory")) {
        let aname = acc.get("name");
        let Some(arec) = find_in(doc, "accessories", "accessory", &aname, &a.books) else {
            a.missing("Weapon accessory", &aname);
            continue;
        };
        let mount = acc.child_text("mount").filter(|m| !m.is_empty()).unwrap_or_else(|| "Internal".into());
        let extra = acc.child_text("extramount").filter(|m| !m.is_empty()).unwrap_or_else(|| "None".into());
        let r = items::weapon::add_accessory(ch, a.store, arec, &guid, &mount, &extra, 0, false);
        if let Some(ag) = a.ok("Weapon accessory", &aname, r) {
            for g in acc.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
                gear(ch, a, g, Some(&ag));
            }
        }
    }
    if parent.is_none() {
        if let Some(ub) = e.child("underbarrel") {
            let uname = ub.child_text("name").unwrap_or_else(|| ub.text()).trim().to_owned();
            weapon(ch, a, doc, ub, &uname, Some(&guid));
        }
    }
    Some(guid)
}

fn weapons(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("weapons.xml") else { return };
    for e in kit.child("weapons").into_iter().flat_map(|k| k.children_named("weapon")) {
        weapon(ch, a, &doc, e, &e.get("name"), None);
    }
}

#[derive(Clone, Copy)]
enum Parent<'a> {
    Character,
    Ware(&'a str),
    VehicleMod(&'a str),
}

/// `AddPACKSCyberwareAsync`: cyberware.xml first, else bioware.xml; nested
/// ware and gear.
fn ware(ch: &mut Character, a: &mut Applier<'_>, e: &Element, parent: Parent<'_>) {
    let name = e.get("name");
    if name.is_empty() {
        return;
    }
    let cyber = a.doc("cyberware.xml");
    let bio = a.doc("bioware.xml");
    let found = cyber.as_deref().and_then(|d| find_in(d, "cyberwares", "cyberware", &name, &a.books)).map(|r| ("cyberware", r.el().clone()));
    let found = found.or_else(|| bio.as_deref().and_then(|d| find_in(d, "biowares", "bioware", &name, &a.books)).map(|r| ("bioware", r.el().clone())));
    let Some((tag, rec)) = found else {
        a.missing("Cyberware", &name);
        return;
    };
    let mut p = purchase(e.get_i32("rating").unwrap_or(0), None);
    p.grade = e.child_text("grade").filter(|g| !g.is_empty());
    if let Parent::Ware(g) = parent {
        p.parent = Some(g.to_owned());
    }
    let label = if tag == "bioware" { "Bioware" } else { "Cyberware" };
    let Some(guid) = a.ok(label, &name, items::add(tag, ch, a.store, Record(&rec), &p)) else { return };
    if let Parent::VehicleMod(m) = parent {
        move_into(ch, &guid, m, "cyberwares");
    }
    for c in e.child("cyberwares").into_iter().flat_map(|c| c.elements()) {
        ware(ch, a, c, Parent::Ware(&guid));
    }
    for g in e.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
        gear(ch, a, g, Some(&guid));
    }
}

/// `AddPACKSGearAsync`: by name and category, with `name/@select`,
/// `rating`, `qty` and nested gear. Returns the new gear's guid.
fn gear(ch: &mut Character, a: &mut Applier<'_>, e: &Element, parent: Option<&str>) -> Option<String> {
    let doc = a.doc("gear.xml")?;
    let (name, select) = name_select(e);
    let category = e.get("category");
    let rec = doc.child("gears").and_then(|c| {
        c.children_named("gear").find(|g| {
            (g.get("name") == name || g.get("id").eq_ignore_ascii_case(&name)) && (category.is_empty() || g.get("category") == category) && (a.books.is_empty() || a.books.contains(&g.get("source")))
        })
    });
    let Some(rec) = rec.map(Record) else {
        a.missing("Gear", &name);
        return None;
    };
    let mut p = purchase(e.get_i32("rating").unwrap_or(0), parent);
    p.qty = e.get_f64("qty").unwrap_or(1.0);
    p.answer = select;
    let guid = a.ok("Gear", &name, items::add("gear", ch, a.store, rec, &p))?;
    for c in e.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
        gear(ch, a, c, Some(&guid));
    }
    Some(guid)
}

fn vehicles(ch: &mut Character, a: &mut Applier<'_>, kit: &Element) {
    let Some(doc) = a.doc("vehicles.xml") else { return };
    let wdoc = a.doc("weapons.xml");
    for e in kit.child("vehicles").into_iter().flat_map(|k| k.children_named("vehicle")) {
        let name = e.get("name");
        let Some(rec) = find_in(&doc, "vehicles", "vehicle", &name, &a.books) else {
            a.missing("Vehicle", &name);
            continue;
        };
        let Some(vguid) = a.ok("Vehicle", &name, items::add("vehicle", ch, a.store, rec, &purchase(0, None))) else { continue };
        // The default sensor that comes with the vehicle.
        let is_free_sensor = |g: &Element| g.get("category") == "Sensors" && g.get("cost") == "0" && g.get_i32("rating").unwrap_or(0) == 0;
        let default_sensor = items_in(ch, &vguid, "gears").into_iter().find(|g| is_free_sensor(g)).map(|g| g.get("guid"));
        for m in e.child("mods").into_iter().flat_map(|m| m.children_named("mod")) {
            let mname = m.get("name");
            let Some(mrec) = find_in(&doc, "mods", "mod", &mname, &a.books) else {
                a.missing("Vehicle mod", &mname);
                continue;
            };
            let r = items::add("mod", ch, a.store, mrec, &purchase(m.get_i32("rating").unwrap_or(0), Some(&vguid)));
            if let Some(mg) = a.ok("Vehicle mod", &mname, r) {
                for c in m.child("cyberwares").into_iter().flat_map(|c| c.children_named("cyberware")) {
                    ware(ch, a, c, Parent::VehicleMod(&mg));
                }
            }
        }
        for g in e.child("gears").into_iter().flat_map(|g| g.children_named("gear")) {
            let Some(gg) = gear(ch, a, g, Some(&vguid)) else { continue };
            // A sensor replaces the vehicle's base sensor.
            let replaces = crate::items::find_by_guid_mut(&mut ch.doc, &gg).is_some_and(|x| is_free_sensor(x));
            if let (true, Some(ds)) = (replaces, default_sensor.as_deref()) {
                ch.remove_item_anywhere(ds);
            }
        }
        let Some(wdoc) = wdoc.as_deref() else { continue };
        for w in e.child("weapons").into_iter().flat_map(|w| w.children_named("weapon")) {
            let wname = w.get("name");
            let category = find_in(wdoc, "weapons", "weapon", &wname, &a.books).map(|r| r.category()).unwrap_or_default();
            let Some(wg) = weapon(ch, a, wdoc, w, &wname, None) else { continue };
            // The first weapon mount of the vehicle takes it.
            let mount = items_in(ch, &vguid, "mods")
                .into_iter()
                .find(|m| m.get("name").contains("Weapon Mount") || (!m.get("weaponmountcategories").is_empty() && m.get("weaponmountcategories").contains(&category)))
                .map(|m| m.get("guid"))
                // LIKELY-BUG(LB-23): deviates from Chummer (fixed here). See docs/likely-bugs.md.
                // Chummer only looks at mods; vehicles with built-in weapon
                // mounts (drones) would lose the weapon there.
                .or_else(|| items_in(ch, &vguid, "weaponmounts").into_iter().find(|m| m.child("weapons").is_none_or(|w| w.elements().next().is_none())).map(|m| m.get("guid")));
            match mount {
                Some(mg) => {
                    move_into(ch, &wg, &mg, "weapons");
                    if let Some(x) = find_by_guid_mut(&mut ch.doc, &wg) {
                        x.set_child_text("parentid", vguid.clone());
                    }
                }
                None => a.report.skipped.push(format!("Weapon: {wname} (no weapon mount on {name}; added to the character)")),
            }
        }
    }
}

/// Clones of the children in `container` of the item `guid`.
fn items_in(ch: &mut Character, guid: &str, container: &str) -> Vec<Element> {
    find_by_guid_mut(&mut ch.doc, guid).and_then(|v| v.child(container)).map(|c| c.elements().cloned().collect()).unwrap_or_default()
}

/// Move the item `guid` into the `container` of item `parent`.
fn move_into(ch: &mut Character, guid: &str, parent: &str, container: &str) {
    fn take(e: &mut Element, guid: &str) -> Option<Element> {
        let pos = e.children.iter().position(|n| matches!(n, Node::Element(c) if c.get("guid").eq_ignore_ascii_case(guid)));
        if let Some(i) = pos {
            if let Node::Element(c) = e.children.remove(i) {
                return Some(c);
            }
        }
        e.elements_mut().find_map(|c| take(c, guid))
    }
    let Some(item) = take(&mut ch.doc, guid) else { return };
    match find_by_guid_mut(&mut ch.doc, parent) {
        Some(p) => p.child_or_insert(container).push(item),
        None => ch.doc.push(item),
    }
}

// ---------------------------------------------------------------------------
// Creating a kit (`CreatePACKSKit`)
// ---------------------------------------------------------------------------

/// The `CreatePACKSKit` checkboxes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KitParts {
    pub attributes: bool,
    pub qualities: bool,
    pub starting_nuyen: bool,
    pub martial_arts: bool,
    pub spells: bool,
    pub complex_forms: bool,
    pub cyberware: bool,
    pub lifestyles: bool,
    pub armor: bool,
    pub weapons: bool,
    pub gear: bool,
    pub vehicles: bool,
}

impl Default for KitParts {
    fn default() -> Self {
        KitParts {
            attributes: true,
            qualities: true,
            starting_nuyen: true,
            martial_arts: true,
            spells: true,
            complex_forms: true,
            cyberware: true,
            lifestyles: true,
            armor: true,
            weapons: true,
            gear: true,
            vehicles: true,
        }
    }
}

fn text(name: &str, v: impl Into<String>) -> Element {
    Element::with_text(name, v)
}

/// `<name select="extra">Name</name>` (the `select` only when set).
fn named(name: &str, extra: &str) -> Element {
    let mut n = text("name", name);
    if !extra.is_empty() {
        n.set_attr("select", extra);
    }
    n
}

fn list<'a>(e: &'a Element, container: &str, item: &'a str) -> Vec<&'a Element> {
    e.child(container).map(|c| c.children_named(item).collect()).unwrap_or_default()
}

/// The kit `CreatePACKSKit` writes for the character's current things.
pub fn from_character(ch: &Character, sheet: &Sheet, settings: Option<&CharacterSettings>, name: &str, parts: KitParts) -> Element {
    let mut pack = Element::new("pack");
    pack.push(text("name", name));
    pack.push(text("category", CUSTOM));
    if parts.attributes {
        let mut attrs = Element::new("attributes");
        let value = |n: &str| sheet.attributes.iter().find(|a| a.name == n).map_or(0, |a| a.value - (a.metatype_min - 1));
        for n in ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG"] {
            attrs.push(text(&n.to_ascii_lowercase(), value(n).to_string()));
        }
        if ch.mag_enabled() {
            attrs.push(text("mag", value("MAG").to_string()));
            let mystic = ch.is_adept() && ch.is_magician();
            if mystic && settings.is_some_and(|s| s.flag("mysadeptsecondmagattribute")) {
                attrs.push(text("magadept", value("MAGAdept").to_string()));
            }
        }
        if ch.res_enabled() {
            attrs.push(text("res", value("RES").to_string()));
        }
        if ch.dep_enabled() {
            attrs.push(text("dep", value("DEP").to_string()));
        }
        pack.push(attrs);
    }
    if parts.qualities {
        let qs = ch.items("qualities", "quality");
        let of = |t: &str| qs.iter().filter(|q| q.get("qualitytype") == t).collect::<Vec<_>>();
        let (pos, neg) = (of("Positive"), of("Negative"));
        let mut q = Element::new("qualities");
        // chummer-rs deviates from Chummer (LB-06): Chummer tests
        // `blnPositive` for both lists, so a kit with only negative qualities
        // was written with none (and one with only positive ones got an
        // empty <negative/>). Each list is written when it has qualities.
        for (tag, items) in [("positive", &pos), ("negative", &neg)] {
            if !items.is_empty() {
                let mut k = Element::new(tag);
                for x in items.iter() {
                    let mut e = text("quality", x.get("name"));
                    if !x.get("extra").is_empty() {
                        e.set_attr("select", x.get("extra"));
                    }
                    k.push(e);
                }
                q.push(k);
            }
        }
        pack.push(q);
    }
    if parts.starting_nuyen {
        let mut bp = ch.doc.get_f64("nuyenbp").unwrap_or(0.0);
        if !crate::character::uses_priority_tables(&ch.field("buildmethod")) {
            bp /= 2.0;
        }
        pack.push(text("nuyenbp", fmt_num(bp)));
    }
    if parts.martial_arts {
        let mut ms = Element::new("martialarts");
        for m in ch.items("martialarts", "martialart") {
            let mut e = Element::new("martialart");
            e.push(text("name", m.get("name")));
            let techs = list(m, "martialarttechniques", "martialarttechnique");
            if !techs.is_empty() {
                let mut t = Element::new("techniques");
                for x in techs {
                    t.push(text("technique", x.get("name")));
                }
                e.push(t);
            }
            ms.push(e);
        }
        pack.push(ms);
    }
    if parts.spells {
        let mut ss = Element::new("spells");
        for s in ch.items("spells", "spell") {
            let mut e = Element::new("spell");
            e.push(named(&s.get("name"), &s.get("extra")));
            e.push(text("category", s.get("category")));
            ss.push(e);
        }
        pack.push(ss);
    }
    if parts.complex_forms {
        let mut cs = Element::new("complexforms");
        for c in ch.items("complexforms", "complexform") {
            let mut e = Element::new("complexform");
            e.push(named(&c.get("name"), &c.get("extra")));
            cs.push(e);
        }
        pack.push(cs);
    }
    if parts.cyberware {
        let ware = ch.items("cyberwares", "cyberware");
        for (bio, list_tag, item_tag) in [(false, "cyberwares", "cyberware"), (true, "biowares", "bioware")] {
            let of: Vec<&&Element> = ware.iter().filter(|w| items::cyberware::is_bioware(w) == bio).collect();
            if of.is_empty() {
                continue;
            }
            let mut l = Element::new(list_tag);
            for w in of {
                let mut e = Element::new(item_tag);
                e.push(text("name", w.get("name")));
                let r = w.get_i32("rating").unwrap_or(0);
                if r > 0 {
                    e.push(text("rating", r.to_string()));
                }
                e.push(text("grade", w.get("grade")));
                let kids: Vec<&Element> = list(w, "children", "cyberware").into_iter().filter(|c| c.get("capacity") != "[*]").collect();
                if !kids.is_empty() {
                    let mut ks = Element::new("cyberwares");
                    for c in kids {
                        let mut k = Element::new(if items::cyberware::is_bioware(c) { "bioware" } else { "cyberware" });
                        k.push(text("name", c.get("name")));
                        let r = c.get_i32("rating").unwrap_or(0);
                        if r > 0 {
                            k.push(text("rating", r.to_string()));
                        }
                        push_gears(&mut k, c, "gears");
                        ks.push(k);
                    }
                    e.push(ks);
                }
                push_gears(&mut e, w, "gears");
                l.push(e);
            }
            pack.push(l);
        }
    }
    if parts.lifestyles {
        let mut ls = Element::new("lifestyles");
        for l in ch.items("lifestyles", "lifestyle") {
            let mut e = Element::new("lifestyle");
            e.push(text("name", l.get("name")));
            e.push(text("months", l.get("months")));
            if !l.get("baselifestyle").is_empty() {
                // An advanced lifestyle: write out its properties.
                for k in ["cost", "dice", "multiplier", "baselifestyle"] {
                    e.push(text(k, l.get(k)));
                }
                let qs = list(l, "lifestylequalities", "lifestylequality");
                if !qs.is_empty() {
                    let mut q = Element::new("qualities");
                    for x in qs {
                        q.push(text("quality", x.get("name")));
                    }
                    e.push(q);
                }
            }
            ls.push(e);
        }
        pack.push(ls);
    }
    if parts.armor {
        let mut a = Element::new("armors");
        for x in ch.items("armors", "armor") {
            let mut e = Element::new("armor");
            e.push(text("name", x.get("name")));
            let mods = list(x, "armormods", "armormod");
            if !mods.is_empty() {
                let mut ms = Element::new("mods");
                for m in mods {
                    let mut me = Element::new("mod");
                    me.push(text("name", m.get("name")));
                    let r = m.get_i32("rating").unwrap_or(0);
                    if r > 0 {
                        me.push(text("rating", r.to_string()));
                    }
                    ms.push(me);
                }
                e.push(ms);
            }
            push_gears(&mut e, x, "gears");
            a.push(e);
        }
        pack.push(a);
    }
    if parts.weapons {
        let mut ws = Element::new("weapons");
        for w in ch.items("weapons", "weapon") {
            let cat = w.get("category");
            if cat == "Cyberware" || cat == "Gear" || w.get("name") == "Unarmed Attack" {
                continue;
            }
            ws.push(weapon_entry(w, false));
        }
        pack.push(ws);
    }
    if parts.gear {
        let mut tmp = Element::new("x");
        push_gears(&mut tmp, &ch.doc, "gears");
        if let Some(g) = tmp.child("gears") {
            pack.push(g.clone());
        }
    }
    if parts.vehicles {
        let mut vs = Element::new("vehicles");
        for v in ch.items("vehicles", "vehicle") {
            let mut e = Element::new("vehicle");
            e.push(text("name", v.get("name")));
            let mods = list(v, "mods", "mod");
            if !mods.is_empty() {
                let mut ms = Element::new("mods");
                for m in mods.iter().filter(|m| !m.get_bool("included").unwrap_or(false)) {
                    let mut me = Element::new("mod");
                    me.push(text("name", m.get("name")));
                    let r = m.get_i32("rating").unwrap_or(0);
                    if r > 0 {
                        me.push(text("rating", r.to_string()));
                    }
                    ms.push(me);
                }
                e.push(ms);
            }
            let weapons: Vec<&Element> = mods.iter().flat_map(|m| list(m, "weapons", "weapon")).collect();
            if !weapons.is_empty() {
                let mut ws = Element::new("weapons");
                for w in weapons {
                    ws.push(weapon_entry(w, true));
                }
                e.push(ws);
            }
            push_gears(&mut e, v, "gears");
            vs.push(e);
        }
        pack.push(vs);
    }
    pack
}

/// A kit `<weapon>`: name, own accessories and own underbarrel weapons.
/// Vehicle weapons are written without accessory gear and with every
/// underbarrel weapon, as `CreatePACKSKit` does.
fn weapon_entry(w: &Element, vehicle: bool) -> Element {
    let mut e = Element::new("weapon");
    e.push(text("name", w.get("name")));
    let accs = list(w, "accessories", "accessory");
    if !accs.is_empty() {
        let mut l = Element::new("accessories");
        for a in accs.into_iter().filter(|a| !a.get_bool("included").unwrap_or(false)) {
            let mut ae = Element::new("accessory");
            ae.push(text("name", a.get("name")));
            ae.push(text("mount", a.get("mount")));
            ae.push(text("extramount", a.get("extramount")));
            if !vehicle {
                push_gears(&mut ae, a, "gears");
            }
            l.push(ae);
        }
        e.push(l);
    }
    for u in list(w, "underbarrel", "weapon").into_iter().filter(|u| vehicle || !u.get_bool("included").unwrap_or(false)) {
        e.push(text("underbarrel", u.get("name")));
    }
    e
}

/// `CreatePACKSKit.WriteGear`: the gear in `owner/container` (gear
/// children are in `children`), skipping gear included in its parent.
fn push_gears(out: &mut Element, owner: &Element, container: &str) {
    let gears: Vec<&Element> = list(owner, container, "gear").into_iter().filter(|g| !g.get_bool("includedinparent").unwrap_or(false)).collect();
    if gears.is_empty() {
        return;
    }
    let mut l = Element::new("gears");
    for g in gears {
        let mut e = Element::new("gear");
        e.push(named(&g.get("name"), &g.get("extra")));
        e.push(text("category", g.get("category")));
        let r = g.get_i32("rating").unwrap_or(0);
        if r > 0 {
            e.push(text("rating", r.to_string()));
        }
        let q = g.get_f64("qty").unwrap_or(1.0);
        if q != 1.0 {
            e.push(text("qty", fmt_num(q)));
        }
        push_gears(&mut e, g, "children");
        l.push(e);
    }
    out.push(l);
}

/// Chummer's file name rule: `custom_` prefix and `_packs.xml` suffix are
/// added when missing.
pub fn normalize_file_name(name: &str) -> String {
    let mut f = name.trim().to_owned();
    if !f.to_ascii_lowercase().starts_with("custom_") {
        f = format!("custom_{f}");
    }
    if !f.to_ascii_lowercase().ends_with("_packs.xml") {
        f += "_packs.xml";
    }
    f
}

/// Why a kit could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("Please enter a name for your Kit.")]
    NoName,
    #[error("Please enter a file name to save the Kit to.")]
    NoFileName,
    #[error("A Kit named \"{0}\" already exists in a data file. Please choose a different name.")]
    Duplicate(String),
    #[error("{0}")]
    Io(String),
}

/// Write `pack` into `dir/<file>` (`CreatePACKSKit.cmdOK_Click`): the
/// existing kits of that file are kept. `merged` is the loaded packs data,
/// to refuse a second Custom kit of the same name.
pub fn save(dir: &Path, file: &str, pack: &Element, merged: &Element) -> Result<PathBuf, SaveError> {
    let name = pack.get("name");
    if name.trim().is_empty() {
        return Err(SaveError::NoName);
    }
    if file.trim().is_empty() {
        return Err(SaveError::NoFileName);
    }
    if find_kit(merged, &name, CUSTOM).is_some() {
        return Err(SaveError::Duplicate(name));
    }
    std::fs::create_dir_all(dir).map_err(|e| SaveError::Io(e.to_string()))?;
    let path = dir.join(normalize_file_name(file));
    let mut root = read_or_new(&path)?;
    root.child_or_insert("packs").push(pack.clone());
    write(&path, &root)?;
    Ok(path)
}

fn read_or_new(path: &Path) -> Result<Element, SaveError> {
    if !path.exists() {
        return Ok(Element::new("chummer"));
    }
    let src = std::fs::read_to_string(path).map_err(|e| SaveError::Io(e.to_string()))?;
    xml::parse(&src).map_err(|e| SaveError::Io(format!("{}: {e}", path.display())))
}

fn write(path: &Path, root: &Element) -> Result<(), SaveError> {
    std::fs::write(path, root.to_xml_string()).map_err(|e| SaveError::Io(e.to_string()))
}

/// Delete a Custom kit from every `custom_*_packs.xml` in `dir`
/// (`SelectPACKSKit.cmdDelete_Click`). Returns whether one was removed.
pub fn delete(dir: &Path, name: &str) -> Result<bool, SaveError> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(false) };
    let mut removed = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let file = path.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !file.starts_with("custom_") || !file.ends_with("_packs.xml") {
            continue;
        }
        let Ok(mut root) = read_or_new(&path) else { continue };
        let Some(packs) = root.child_mut("packs") else { continue };
        let before = packs.children.len();
        packs.children.retain(|n| !matches!(n, Node::Element(p) if p.get("name") == name && p.get("category") == CUSTOM));
        if packs.children.len() != before {
            write(&path, &root)?;
            removed = true;
        }
    }
    Ok(removed)
}
