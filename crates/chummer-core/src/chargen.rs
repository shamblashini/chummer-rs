//! Character creation: priority tables, the new-character file, point
//! budgets and finalising into career mode.
//!
//! Follows `SelectMetatypePriority.cs` (priority choice), `Character.Create`
//! (metatype application), `Character.Save` (file layout) and
//! `CharacterCreate.cs` (`CalculateBP`, `CheckCharacterValidity`).

use crate::bonus::{self, BonusSource};
use crate::calc::{Rules, Sheet};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::engine::Engine;
use crate::improvement::{bool_str, Improvement};
use crate::items::{self, new_guid};
use crate::settings::CharacterSettings;
use crate::xml::Element;

/// Version written as `<appversion>`. Chummer5a uses it to choose load
/// fix-ups, so it must be a real Chummer version, not ours.
pub const CHUMMER_APP_VERSION: &str = "5.226.0";
pub const CHUMMER_MIN_APP_VERSION: &str = "5.214.1";

pub const CATEGORIES: [&str; 5] = ["Heritage", "Talent", "Attributes", "Skills", "Resources"];
pub const LETTERS: [char; 5] = ['A', 'B', 'C', 'D', 'E'];

/// A priority letter for each category, in [`CATEGORIES`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Priorities(pub [char; 5]);

impl Priorities {
    pub fn get(&self, category: &str) -> char {
        CATEGORIES.iter().position(|c| *c == category).map_or('E', |i| self.0[i])
    }

    /// Sum-to-Ten value of a letter (`priortysumtotenvalues`).
    pub fn sum_to_ten_value(letter: char) -> i32 {
        match letter {
            'A' => 4,
            'B' => 3,
            'C' => 2,
            'D' => 1,
            _ => 0,
        }
    }

    pub fn sum(&self) -> i32 {
        self.0.iter().map(|l| Self::sum_to_ten_value(*l)).sum()
    }

    /// Priority builds use each letter once; Sum-to-Ten needs the values
    /// to add up to the setting's total.
    pub fn validate(&self, settings: &CharacterSettings) -> Result<(), String> {
        match settings.build_method().as_str() {
            "SumtoTen" => {
                let want = settings.int("sumtoten", 10);
                if self.sum() == want {
                    Ok(())
                } else {
                    Err(format!("Priority values add up to {}, need {want}", self.sum()))
                }
            }
            _ => {
                let mut l = self.0.to_vec();
                l.sort();
                let mut want: Vec<char> = settings.text("priorityarray", "ABCDE").chars().collect();
                want.sort();
                if l == want {
                    Ok(())
                } else {
                    Err("Use each priority letter once".into())
                }
            }
        }
    }
}

/// The `<priority>` node for a category and letter.
pub fn priority_node(store: &DataStore, settings: &CharacterSettings, category: &str, letter: char) -> Option<Element> {
    let doc = store.doc("priorities.xml").ok()?;
    let table = settings.text("prioritytable", "Standard");
    let l = letter.to_string();
    let found = doc
        .child("priorities")?
        .children_named("priority")
        .find(|p| p.get("category") == category && p.get("value") == l && (p.child("prioritytable").is_none() || p.get("prioritytable") == table))
        .cloned();
    found
}

/// A metatype offered at a heritage priority.
#[derive(Debug, Clone)]
pub struct HeritageOption {
    pub metatype: String,
    /// Special attribute points.
    pub special: i32,
    pub karma: i32,
    /// (name, special points, karma)
    pub metavariants: Vec<(String, i32, i32)>,
}

pub fn heritage_options(store: &DataStore, settings: &CharacterSettings, letter: char) -> Vec<HeritageOption> {
    let Some(node) = priority_node(store, settings, "Heritage", letter) else { return Vec::new() };
    node.child("metatypes")
        .map(|m| {
            m.children_named("metatype")
                .map(|mt| HeritageOption {
                    metatype: mt.get("name"),
                    special: mt.get_i32("value").unwrap_or(0),
                    karma: mt.get_i32("karma").unwrap_or(0),
                    metavariants: mt
                        .child("metavariants")
                        .map(|v| v.children_named("metavariant").map(|x| (x.get("name"), x.get_i32("value").unwrap_or(0), x.get_i32("karma").unwrap_or(0))).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A magic/resonance option at a talent priority.
#[derive(Debug, Clone)]
pub struct TalentOption {
    pub display: String,
    /// `prioritytalent` value, e.g. `Magician`, `Mundane`.
    pub value: String,
    pub node: Element,
}

impl TalentOption {
    pub fn int(&self, k: &str) -> i32 {
        self.node.get_i32(k).unwrap_or(0)
    }
    pub fn skill_qty(&self) -> i32 {
        self.node.get_i32("skillqty").or_else(|| self.node.get_i32("skillgroupqty")).unwrap_or(0)
    }
    pub fn skill_val(&self) -> i32 {
        self.node.get_i32("skillval").or_else(|| self.node.get_i32("skillgroupval")).unwrap_or(0)
    }
    pub fn grouped(&self) -> bool {
        self.node.child("skillgroupqty").is_some() || self.node.get("skilltype") == "grouped" || self.node.get("skillgrouptype") == "grouped"
    }
}

pub fn talent_options(store: &DataStore, settings: &CharacterSettings, letter: char) -> Vec<TalentOption> {
    let Some(node) = priority_node(store, settings, "Talent", letter) else { return Vec::new() };
    node.child("talents")
        .map(|t| t.children_named("talent").map(|e| TalentOption { display: e.get("name"), value: e.get("value"), node: e.clone() }).collect())
        .unwrap_or_default()
}

/// Skills (or skill groups) a talent's free skills can go to.
pub fn talent_skill_options(store: &DataStore, talent: &TalentOption) -> Vec<String> {
    let n = &talent.node;
    if talent.grouped() {
        if let Some(c) = n.child("skillgroupchoices") {
            return c.children_named("skillgroup").map(Element::text).collect();
        }
        return store
            .doc("skills.xml")
            .ok()
            .and_then(|d| d.child("skillgroups").map(|g| g.children_named("name").map(Element::text).collect()))
            .unwrap_or_default();
    }
    if let Some(c) = n.child("skillchoices") {
        return c.children_named("skill").map(Element::text).collect();
    }
    let Ok(doc) = store.doc("skills.xml") else { return Vec::new() };
    let kind = n.get("skilltype");
    let mut v: Vec<String> = data::records(&doc, "skills", "skill")
        .into_iter()
        .filter(|r| r.get("exotic") != "True")
        .filter(|r| {
            let (cat, group, attr) = (r.category(), r.get("skillgroup"), r.get("attribute"));
            match kind.as_str() {
                "magic" => cat == "Magical Active" || cat == "Pseudo-Magical Active",
                "resonance" => cat == "Resonance Active" || group == "Cracking" || group == "Electronics",
                "matrix" => group == "Cracking" || group == "Electronics",
                // The data's xpath variant: no RES/DEP skills and no grouped magic skills.
                "xpath" => attr != "RES" && attr != "DEP" && (cat != "Magical Active" || group.is_empty()),
                _ => true,
            }
        })
        .map(|r| r.name())
        .collect();
    v.sort();
    v
}

/// What the new-character wizard collects.
#[derive(Debug, Clone)]
pub struct NewCharacter {
    pub settings_id: String,
    pub metatype: String,
    pub metavariant: Option<String>,
    pub priorities: Priorities,
    pub talent: String,
    pub talent_skills: Vec<String>,
    pub name: String,
}

fn find_metatype<'a>(doc: &'a Element, name: &str) -> Option<Record<'a>> {
    data::find(doc, "metatypes", "metatype", name)
}

/// Starting resources of a new character.
struct StartBudget {
    special: i32,
    metatype_karma: i32,
    talent: TalentOption,
    attributes: i32,
    skills: i32,
    groups: i32,
    nuyen: i32,
}

/// Priority and Sum-to-Ten: resources come from the priority table.
fn priority_budget(store: &DataStore, settings: &CharacterSettings, spec: &NewCharacter, mt: Record<'_>, node: &Element) -> Result<StartBudget, String> {
    let heritage = heritage_options(store, settings, spec.priorities.get("Heritage"));
    let h = heritage.iter().find(|h| h.metatype == spec.metatype).ok_or("metatype not available at this heritage priority")?;
    let (special, metatype_karma) = match &spec.metavariant {
        Some(v) => h.metavariants.iter().find(|m| m.0 == *v).map(|m| (m.1, m.2)).ok_or("metavariant not available at this priority")?,
        None => (h.special, h.karma),
    };
    let talents = talent_options(store, settings, spec.priorities.get("Talent"));
    let talent = talents.iter().find(|t| t.value == spec.talent).cloned().ok_or("talent not available at this priority")?;
    let attr_node = priority_node(store, settings, "Attributes", spec.priorities.get("Attributes"));
    let mut attributes = attr_node.as_ref().and_then(|n| n.get_i32("attributes")).unwrap_or(0);
    if node.child("halveattributepoints").is_some() || mt.el().child("halveattributepoints").is_some() {
        attributes /= 2;
    }
    let skills_node = priority_node(store, settings, "Skills", spec.priorities.get("Skills"));
    Ok(StartBudget {
        special,
        metatype_karma,
        talent,
        attributes,
        skills: skills_node.as_ref().and_then(|n| n.get_i32("skills")).unwrap_or(0),
        groups: skills_node.as_ref().and_then(|n| n.get_i32("skillgroups")).unwrap_or(0),
        nuyen: priority_node(store, settings, "Resources", spec.priorities.get("Resources")).and_then(|n| n.get_i32("resources")).unwrap_or(0),
    })
}

/// Karma (point buy) and Life Module builds: no points, the metatype costs
/// karma (`SelectMetatypeKarma`), magic comes from qualities bought later.
fn karma_build_budget(settings: &CharacterSettings, mt: Record<'_>, mv: Option<&Element>) -> StartBudget {
    let karma = mv.and_then(|v| v.get_i32("karma")).unwrap_or_else(|| mt.el().get_i32("karma").unwrap_or(0));
    let mult = settings.int("metatypecostskarmamultiplier", 1);
    let mundane = Element::with_text("talent", "");
    StartBudget {
        special: 0,
        metatype_karma: karma * mult,
        talent: TalentOption { display: "Mundane".into(), value: "Mundane".into(), node: mundane },
        attributes: 0,
        skills: 0,
        groups: 0,
        nuyen: 0,
    }
}

/// All metatypes for karma builds, with their karma cost.
pub fn karma_metatypes(store: &DataStore) -> Vec<HeritageOption> {
    let Ok(doc) = store.doc("metatypes.xml") else { return Vec::new() };
    data::records(&doc, "metatypes", "metatype")
        .into_iter()
        .map(|m| HeritageOption {
            metatype: m.name(),
            special: 0,
            karma: m.el().get_i32("karma").unwrap_or(0),
            metavariants: m
                .el()
                .child("metavariants")
                .map(|v| v.children_named("metavariant").map(|x| (x.get("name"), 0, x.get_i32("karma").unwrap_or(0))).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// Build a new creation-mode character (`Character.Create` +
/// `SelectMetatypePriority.MetatypeSelected` or `SelectMetatypeKarma`).
pub fn create(engine: &Engine, spec: &NewCharacter) -> Result<Character, String> {
    let store = &engine.store;
    let settings = engine.settings.resolve(&spec.settings_id).ok_or("unknown settings preset")?.clone();
    let karma_build = !crate::character::uses_priority_tables(&settings.build_method());
    if !karma_build {
        spec.priorities.validate(&settings)?;
    }
    let metatypes = store.doc("metatypes.xml").map_err(|e| e.to_string())?;
    let mt = find_metatype(&metatypes, &spec.metatype).ok_or_else(|| format!("unknown metatype {}", spec.metatype))?;
    let mv = spec.metavariant.as_ref().and_then(|v| {
        mt.el().child("metavariants").and_then(|m| m.children_named("metavariant").find(|x| x.get("name") == *v))
    });
    // Attributes come from the metavariant node when one is chosen.
    let node: &Element = mv.unwrap_or(mt.el());

    let b = if karma_build { karma_build_budget(&settings, mt, mv) } else { priority_budget(store, &settings, spec, mt, node)? };
    let (special, metatype_karma, talent) = (b.special, b.metatype_karma, b.talent.clone());
    let (attr_points, skill_points, group_points, nuyen) = (b.attributes, b.skills, b.groups, b.nuyen);
    let special_total = special + talent.int("specialattribpoints");

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
    put(&mut doc, "metatypebp", metatype_karma.to_string());
    put(&mut doc, "metavariant", mv.map(|v| v.get("name")).unwrap_or_default());
    put(&mut doc, "metavariantid", mv.map(|v| v.get("id")).unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into()));
    put(&mut doc, "metatypecategory", mt.category());
    put(&mut doc, "movement", mt.get("movement"));
    for k in ["walk", "run", "sprint"] {
        let v = node.child_text(k).or_else(|| mt.el().child_text(k)).unwrap_or_else(|| if k == "run" { "4/0/0".into() } else { "2/1/0".into() });
        put(&mut doc, k, v.clone());
    }
    for k in ["walkalt", "runalt", "sprintalt"] {
        let base = k.trim_end_matches("alt");
        let v = node.child_text(k).or_else(|| node.child_text(base)).unwrap_or_default();
        put(&mut doc, k, v);
    }
    put(&mut doc, "initiativedice", mt.el().get_i32("initiativedice").unwrap_or(settings.int("mininitiativedice", 1)).to_string());
    for (k, cat) in [("prioritymetatype", "Heritage"), ("priorityattributes", "Attributes"), ("priorityspecial", "Talent"), ("priorityskills", "Skills"), ("priorityresources", "Resources")] {
        put(&mut doc, k, spec.priorities.get(cat).to_string());
    }
    put(&mut doc, "prioritytalent", talent.value.clone());
    let mut ps = Element::new("priorityskills");
    for s in &spec.talent_skills {
        ps.push(Element::with_text("priorityskill", s.clone()));
    }
    doc.push(ps);
    put(&mut doc, "name", spec.name.clone());
    put(&mut doc, "mainmugshotindex", "-1".into());
    doc.push(Element::new("mugshots"));
    for k in ["gender", "age", "eyes", "height", "weight", "skin", "hair", "description", "background", "concept", "notes", "alias", "playername", "gamenotes"] {
        doc.push(Element::new(k));
    }
    put(&mut doc, "primaryarm", "Right".into());
    put(&mut doc, "karma", "0".into());
    put(&mut doc, "special", special_total.to_string());
    put(&mut doc, "totalspecial", special_total.to_string());
    put(&mut doc, "totalattributes", attr_points.to_string());
    put(&mut doc, "edgeused", "0".into());
    put(&mut doc, "contactpoints", "0".into());
    put(&mut doc, "spelllimit", talent.int("spells").to_string());
    put(&mut doc, "cfplimit", talent.int("cfp").to_string());
    put(&mut doc, "ainormalprogramlimit", "0".into());
    put(&mut doc, "aiadvancedprogramlimit", "0".into());
    for k in ["streetcred", "notoriety", "publicawareness", "burntstreetcred"] {
        put(&mut doc, k, "0".into());
    }
    put(&mut doc, "created", "False".into());
    put(&mut doc, "nuyen", nuyen.to_string());
    put(&mut doc, "startingnuyen", nuyen.to_string());
    put(&mut doc, "nuyenbp", "0".into());
    for k in ["adept", "magician", "technomancer", "ai", "cyberwaredisabled", "initiationdisabled", "critter"] {
        put(&mut doc, k, "False".into());
    }
    put(&mut doc, "prototypetranshuman", "0".into());

    // Attributes: metatype limits, with the talent fixing MAG/RES/DEP.
    let mut attrs = Element::new("attributes");
    let talent_limit = |key: &str| -> Option<(i32, i32)> {
        let v = talent.node.get_i32(key)?;
        let maxk = format!("max{key}");
        let max = talent.node.get_i32(&maxk).unwrap_or(v.max(node.get_i32(&format!("{}max", &key[..3])).unwrap_or(6)));
        Some((v, max))
    };
    for name in crate::expr::ATTRIBUTE_NAMES {
        let key = match *name {
            "MAGAdept" => "mag".to_owned(),
            n => n.to_ascii_lowercase(),
        };
        let g = |suffix: &str, default: i32| node.get_i32(&format!("{key}{suffix}")).or_else(|| mt.el().get_i32(&format!("{key}{suffix}"))).unwrap_or(default);
        let (mut min, mut max, mut aug) = (g("min", 1), g("max", 6), g("aug", 10));
        let special = matches!(*name, "MAG" | "MAGAdept" | "RES" | "DEP");
        if special {
            let tl = match *name {
                "MAG" | "MAGAdept" => talent_limit("magic"),
                "RES" => talent_limit("resonance"),
                _ => talent_limit("depth"),
            };
            if let Some((v, m)) = tl {
                min = v;
                max = m;
                aug = m;
            }
        }
        let mut a = Element::new("attribute");
        a.push(Element::with_text("name", *name));
        a.push(Element::with_text("metatypemin", min.to_string()));
        a.push(Element::with_text("metatypemax", max.to_string()));
        a.push(Element::with_text("metatypeaugmax", aug.to_string()));
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
    put(&mut doc, "initiategrade", "0".into());
    put(&mut doc, "submersiongrade", "0".into());
    put(&mut doc, "physicalcmfilled", "0".into());
    put(&mut doc, "stuncmfilled", "0".into());

    // Every non-exotic active skill from the enabled books, at 0.
    let skills_doc = store.doc("skills.xml").map_err(|e| e.to_string())?;
    let books = settings.books();
    let mut ns = Element::new("newskills");
    ns.push(Element::with_text("skillptsmax", skill_points.to_string()));
    ns.push(Element::with_text("skillgrpsmax", group_points.to_string()));
    let mut skills = Element::new("skills");
    let mut groups: Vec<String> = Vec::new();
    for r in data::records(&skills_doc, "skills", "skill") {
        if r.get("exotic") == "True" || r.hidden() || (!books.is_empty() && !books.contains(&r.source())) {
            continue;
        }
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
        skills.push(s);
        let g = r.get("skillgroup");
        if !g.is_empty() && !groups.contains(&g) {
            groups.push(g);
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

    let mut ch = Character::from_document(doc).map_err(|e| e.to_string())?;

    // Metatype bonus and racial qualities.
    let src = BonusSource { kind: "Metatype".into(), guid: mt.id(), name: mt.name(), rating: 1 };
    if let Some(b) = node.child("bonus").filter(|b| b.elements().next().is_some()) {
        let out = bonus::apply(&ch, store, b, &src, None);
        finish_outcome(&mut ch, store, out);
    }
    let qdoc = store.doc("qualities.xml").map_err(|e| e.to_string())?;
    if let Some(qs) = node.child("qualities") {
        for kind in ["positive", "negative"] {
            for q in qs.child(kind).into_iter().flat_map(|k| k.children_named("quality")) {
                if let Some(rec) = data::find(&qdoc, "qualities", "quality", &q.text()) {
                    let source = if q.attr("removable").is_some_and(|v| v.eq_ignore_ascii_case("true")) { "MetatypeRemovable" } else { "Metatype" };
                    add_quality_with_source(&mut ch, store, rec, q.attr("select"), source, false);
                }
            }
        }
    }
    // Talent qualities (Magician, Adept, Technomancer, ...).
    if let Some(qs) = talent.node.child("qualities") {
        for q in qs.children_named("quality") {
            if let Some(rec) = data::find(&qdoc, "qualities", "quality", &q.text()) {
                add_quality_with_source(&mut ch, store, rec, None, "Heritage", false);
            }
        }
    }
    // Every character has an Unarmed Attack (Character.Create).
    if let Ok(wdoc) = store.doc("weapons.xml") {
        if let Some(rec) = data::find(&wdoc, "weapons", "weapon", "Unarmed Attack") {
            let p = crate::items::Purchase { free: true, ..Default::default() };
            if let Ok(g) = crate::items::add("weapon", &mut ch, store, rec, &p) {
                if let Some(w) = crate::items::find_by_guid_mut(&mut ch.doc, &g) {
                    w.set_child_text("included", "True");
                    w.set_child_text("equipped", "True");
                }
            }
        }
    }
    // Free talent skills.
    let kind = if talent.grouped() { "SkillGroupBase" } else { "SkillBase" };
    for s in &spec.talent_skills {
        ch.improvements.list.push(Improvement {
            kind: kind.into(),
            improved_name: s.clone(),
            source: "Heritage".into(),
            val: f64::from(talent.skill_val()),
            rating: 1,
            enabled: true,
            ..Default::default()
        });
    }
    ch.dirty = true;
    Ok(ch)
}

fn finish_outcome(ch: &mut Character, store: &DataStore, out: bonus::Outcome) {
    items::place_added(ch, store, &out.added);
    items::apply_outcome(ch, &out);
}

/// Add a quality with a given `qualitysource`. `counts` decides whether it
/// costs karma and counts toward the quality limit.
pub fn add_quality_with_source(ch: &mut Character, store: &DataStore, rec: Record<'_>, answer: Option<&str>, source: &str, counts: bool) -> String {
    let guid = new_guid();
    let src = BonusSource { kind: "Quality".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
    let out = rec.el().child("bonus").map(|b| bonus::apply(ch, store, b, &src, answer)).unwrap_or_default();
    let extra = out.selected.clone().or(answer.map(str::to_owned)).unwrap_or_default();
    let mut q = items::quality_element(rec, &guid, source, &extra);
    if !counts {
        q.set_child_text("contributetolimit", "False");
        q.set_child_text("contributetobp", "False");
    }
    ch.items_mut("qualities").push(q);
    finish_outcome(ch, store, out);
    guid
}

/// Add a quality the player picked, with its bonus and nested objects.
pub fn add_quality(ch: &mut Character, store: &DataStore, rec: Record<'_>, answer: Option<&str>) -> String {
    add_quality_with_source(ch, store, rec, answer, "Selected", true)
}

/// Remove a quality and everything it granted (improvements, nested
/// qualities, limit modifiers, mentor spirits).
pub fn remove_quality(ch: &mut Character, guid: &str) {
    remove_with_children(ch, "qualities", guid);
}

fn remove_with_children(ch: &mut Character, container: &str, guid: &str) {
    let owned: Vec<(String, String)> = ch
        .improvements
        .list
        .iter()
        .filter(|i| i.source_name.eq_ignore_ascii_case(guid))
        .filter_map(|i| match i.kind.as_str() {
            "SpecificQuality" => Some(("qualities".to_owned(), i.improved_name.clone())),
            "LimitModifier" => Some(("limitmodifiers".to_owned(), i.improved_name.clone())),
            "MentorSpirit" | "Paragon" => Some(("mentorspirits".to_owned(), i.improved_name.clone())),
            _ => None,
        })
        .collect();
    ch.remove_item(container, guid);
    for (c, g) in owned {
        remove_with_children(ch, &c, &g);
    }
}

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// Remaining creation resources. Negative means overspent.
#[derive(Debug, Clone, Default)]
pub struct Budget {
    pub attribute_points: (i32, i32),
    pub special_points: (i32, i32),
    pub skill_points: (i32, i32),
    pub skill_group_points: (i32, i32),
    pub knowledge_points: (i32, i32),
    pub contact_points: (i32, i32),
    /// (start, spent)
    pub karma: (i32, i32),
    pub positive_quality_karma: i32,
    pub negative_quality_karma: i32,
    pub quality_limit: i32,
    pub nuyen: (f64, f64),
    pub free_spells: (i32, i32),
    /// Adept power points (total, used).
    pub power_points: Option<(f64, f64)>,
}

impl Budget {
    pub fn left(p: (i32, i32)) -> i32 {
        p.0 - p.1
    }
    pub fn karma_left(&self) -> i32 {
        self.karma.0 - self.karma.1
    }
    pub fn nuyen_left(&self) -> f64 {
        self.nuyen.0 - self.nuyen.1
    }
}

/// Cost of an item (and its children), evaluating data expressions at the
/// item's rating. `None` when the cost depends on context we do not model.
pub fn item_cost(e: &Element) -> f64 {
    let rating = e.get_i32("rating").unwrap_or(0);
    let qty = e.get_f64("qty").unwrap_or(1.0);
    let raw = e.get("cost");
    let s = crate::expr::fixed_values(raw.trim(), rating).replace("MinRating", &e.get_i32("minrating").unwrap_or(0).to_string());
    let s = s.replace("{Rating}", &rating.to_string()).replace("Rating", &rating.to_string());
    let own = if s.trim().is_empty() {
        0.0
    } else if crate::expr::needs_evaluation(&s) {
        crate::expr::evaluate_num(&s).unwrap_or(0.0)
    } else {
        crate::expr::parse_plain(&s).unwrap_or(0.0)
    };
    let mut total = own * qty;
    for c in ["children", "gears", "armormods", "accessories", "mods"] {
        if let Some(k) = e.child(c) {
            for child in k.elements() {
                if child.get_bool("includedinparent").unwrap_or(false) || child.get_bool("included").unwrap_or(false) {
                    continue;
                }
                total += item_cost(child);
            }
        }
    }
    total
}

pub fn budget(ch: &Character, sheet: &Sheet, rules: &Rules, settings: &CharacterSettings) -> Budget {
    budget_with(ch, sheet, rules, settings, None)
}

/// As [`budget`], with game data for exact cyberware grade costs.
pub fn budget_with(ch: &Character, sheet: &Sheet, rules: &Rules, settings: &CharacterSettings, store: Option<&DataStore>) -> Budget {
    let mut b = Budget::default();
    let base_sum = |names: &[&str]| -> i32 { ch.attributes.iter().filter(|a| names.contains(&a.name.as_str())).map(|a| a.base).sum() };
    let std_attrs: Vec<&str> = crate::attributes::PHYSICAL.iter().chain(crate::attributes::MENTAL).copied().collect();
    b.attribute_points = (ch.doc.get_i32("totalattributes").unwrap_or(0), base_sum(&std_attrs));
    let mut special_names = vec!["EDG", "MAG", "RES", "DEP"];
    if ch.is_adept() && ch.is_magician() {
        special_names.push("MAGAdept");
    }
    b.special_points = (ch.doc.get_i32("totalspecial").unwrap_or(0), base_sum(&special_names));
    let ns = ch.doc.child("newskills");
    let skill_max = ns.and_then(|n| n.get_i32("skillptsmax")).unwrap_or(0);
    let group_max = ns.and_then(|n| n.get_i32("skillgrpsmax")).unwrap_or(0);
    let spec_cost = |specs: &[crate::skills::Specialization], karma: bool| if karma { 0 } else { specs.iter().filter(|s| !s.free).count() as i32 };
    let active_sp: i32 = ch.skills.iter().map(|s| s.base + spec_cost(&s.specs, s.buy_with_karma)).sum();
    let kno_sp: i32 = ch.knowledge_skills.iter().filter(|k| !k.native_language).map(|k| k.base + spec_cost(&k.specs, false)).sum();
    // Knowledge skills overflowing the free knowledge points use skill points.
    let kno_overflow = (kno_sp - sheet.knowledge_points).max(0);
    b.skill_points = (skill_max, active_sp + kno_overflow);
    b.knowledge_points = (sheet.knowledge_points, kno_sp.min(sheet.knowledge_points));
    b.skill_group_points = (group_max, ch.skill_groups.iter().map(|g| g.base).sum());

    // Contacts: points are CHA x3; anything above costs karma.
    let contact_cost: i32 = ch
        .items("contacts", "contact")
        .iter()
        .filter(|c| !c.get_bool("free").unwrap_or(false))
        .map(|c| c.get_i32("connection").unwrap_or(0) + c.get_i32("loyalty").unwrap_or(0))
        .sum();
    b.contact_points = (sheet.contact_points, contact_cost);

    // Qualities.
    // Karma for qualities (all that contribute to BP) and, separately, the
    // part that counts toward the quality limit.
    let (mut pos, mut neg, mut pos_limit, mut neg_limit) = (0, 0, 0, 0);
    for q in ch.items("qualities", "quality") {
        if !q.get_bool("contributetobp").unwrap_or(true) || q.get("qualitysource") != "Selected" && q.get("qualitysource") != "Improvement" {
            continue;
        }
        let bp = q.get_i32("bp").unwrap_or(0) * rules.karma_quality;
        let limited = q.get_bool("contributetolimit").unwrap_or(true);
        if q.get("qualitytype") == "Negative" {
            neg += bp.abs();
            if limited {
                neg_limit += bp.abs();
            }
        } else {
            pos += bp;
            if limited {
                pos_limit += bp;
            }
        }
    }
    b.positive_quality_karma = pos_limit;
    b.negative_quality_karma = neg_limit;
    b.quality_limit = settings.int("qualitykarmalimit", 25);

    // Spells and complex forms beyond the free ones cost karma.
    let counts = crate::items::magic::spell_counts(ch, sheet);
    b.free_spells = (counts.free, counts.spells + counts.rituals + counts.preparations);
    let spell_karma = crate::items::magic::spell_karma(ch, sheet, rules) + crate::items::magic::complex_form_karma(ch, rules);
    if ch.is_adept() {
        b.power_points = Some(crate::items::magic::power_points(ch, sheet));
    }

    let nuyen_bp = ch.doc.get_i32("nuyenbp").unwrap_or(0);
    let start = settings.int("buildpoints", 25);
    let spent = ch.doc.get_i32("metatypebp").unwrap_or(0) + pos - neg + sheet.attribute_karma_spent + sheet.skill_karma_spent
        + (contact_cost - sheet.contact_points).max(0) * rules.karma_contact
        + spell_karma
        + nuyen_bp;
    b.karma = (start, spent);

    let starting = ch.doc.get_f64("startingnuyen").unwrap_or(0.0) + f64::from(nuyen_bp) * f64::from(settings.int("nuyenperbpwftm", 2000));
    b.nuyen = (starting, nuyen_spent(ch, store));
    b
}

/// Nuyen spent on everything the character owns, with each kind's own
/// cost rules (`CalculateNuyenCreateMode`).
pub fn nuyen_spent(ch: &Character, store: Option<&DataStore>) -> f64 {
    use crate::items::{armor, cyberware, drug, gear, lifestyle, vehicle, weapon};
    let sum = |c: &str, i: &str, f: &dyn Fn(&Element) -> f64| ch.items(c, i).into_iter().map(f).sum::<f64>();
    let ware = match store {
        Some(st) => sum("cyberwares", "cyberware", &|e| cyberware::cost(ch, st, e)),
        None => sum("cyberwares", "cyberware", &item_cost),
    };
    ware + sum("gears", "gear", &gear::cost)
        + sum("armors", "armor", &armor::cost)
        + sum("weapons", "weapon", &weapon::cost)
        + match store {
            Some(st) => sum("vehicles", "vehicle", &|e| vehicle::cost_with(ch, st, e)),
            None => sum("vehicles", "vehicle", &vehicle::cost),
        }
        + sum("drugs", "drug", &drug::cost)
        + sum("lifestyles", "lifestyle", &|e| lifestyle::total_cost(ch, e))
}

/// Problems that block finishing creation (`CheckCharacterValidity`).
pub fn validity_problems(ch: &Character, b: &Budget, settings: &CharacterSettings) -> Vec<String> {
    let mut p = Vec::new();
    let mut check = |left: i32, what: &str| {
        if left < 0 {
            p.push(format!("{what} overspent by {}", -left));
        }
    };
    check(Budget::left(b.attribute_points), "Attribute points");
    check(Budget::left(b.special_points), "Special attribute points");
    check(Budget::left(b.skill_points), "Skill points");
    check(Budget::left(b.skill_group_points), "Skill group points");
    check(b.karma_left(), "Karma");
    if let Some((total, used)) = b.power_points {
        if used > total + 1e-9 {
            p.push(format!("Power points overspent: {used} of {total}"));
        }
    }
    if b.nuyen_left() < 0.0 {
        p.push(format!("Nuyen overspent by {}", crate::format::nuyen(-b.nuyen_left())));
    }
    if b.positive_quality_karma > b.quality_limit {
        p.push(format!("Positive qualities cost {} karma, limit is {}", b.positive_quality_karma, b.quality_limit));
    }
    if b.negative_quality_karma > b.quality_limit {
        p.push(format!("Negative qualities give {} karma, limit is {}", b.negative_quality_karma, b.quality_limit));
    }
    let at_max = ch
        .attributes
        .iter()
        .filter(|a| a.category == "Standard" && a.metatype_max > 0)
        .filter(|a| a.metatype_min + a.base + a.karma >= a.metatype_max)
        .count() as i32;
    let allowed = settings.int("maxnumbermaxattributescreate", 1);
    if at_max > allowed {
        p.push(format!("{at_max} attributes at their maximum, only {allowed} allowed"));
    }
    if !ch.created {
        let skill_name = |suid: &str| ch.doc.child("newskills").and_then(|n| n.child("skills")).and_then(|s| s.elements().find(|e| e.get("suid").eq_ignore_ascii_case(suid))).map(|e| e.get("name")).unwrap_or_default();
        for s in ch.skills.iter().filter(|s| s.specs.iter().filter(|x| !x.free).count() > 1) {
            p.push(format!("{} has more than one specialization", skill_name(&s.suid)));
        }
        for k in ch.knowledge_skills.iter().filter(|k| k.specs.len() > 1) {
            p.push(format!("{} has more than one specialization", k.name));
        }
    }
    if (ch.is_magician() || ch.is_adept()) && ch.mag_enabled() && ch.doc.child("tradition").is_none_or(|t| t.get("name").is_empty()) && ch.is_magician() {
        p.push("Magicians need a tradition (choose one on the Magic tab)".into());
    }
    p
}

/// Finish creation: keep up to 7 karma and 5,000¥, switch to career mode
/// and log the starting values (`SaveCharacterAsCreated`).
pub fn finalize(ch: &mut Character, b: &Budget, settings: &CharacterSettings) {
    let karma = b.karma_left().clamp(0, settings.karma("karmacarryover", 7).max(0));
    let nuyen = b.nuyen_left().clamp(0.0, f64::from(settings.int("nuyencarryover", 5000)));
    ch.karma = karma;
    ch.nuyen = nuyen;
    ch.created = true;
    ch.improvements.career = true;
    let now = now_iso();
    let exp = ch.items_mut("expenses");
    for (amount, kind, reason) in [(f64::from(karma), "Karma", "Starting Karma"), (nuyen, "Nuyen", "Starting Nuyen")] {
        if amount == 0.0 {
            continue;
        }
        let mut e = Element::new("expense");
        e.push(Element::with_text("guid", new_guid()));
        e.push(Element::with_text("date", now.clone()));
        e.push(Element::with_text("amount", crate::improvement::fmt_num(amount)));
        e.push(Element::with_text("reason", reason));
        e.push(Element::with_text("type", kind));
        e.push(Element::with_text("refund", bool_str(false)));
        let mut undo = Element::new("undo");
        undo.push(Element::with_text("karmatype", "ManualAdd"));
        undo.push(Element::with_text("nuyentype", "ManualAdd"));
        undo.push(Element::new("objectid"));
        undo.push(Element::with_text("qty", "0"));
        undo.push(Element::new("extra"));
        e.push(undo);
        exp.push(e);
    }
    ch.dirty = true;
}

/// The current UTC time in .NET's sortable `"s"` format, as expense dates use.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", tod / 3600, tod % 3600 / 60, tod % 60)
}

/// Settings presets that use priority tables (what the wizard offers).
pub fn priority_presets(engine: &Engine) -> Vec<&CharacterSettings> {
    engine.settings.presets.iter().filter(|p| crate::character::uses_priority_tables(&p.build_method())).collect()
}

/// Presets the new-character wizard offers: priority tables and karma
/// point buy and life modules.
pub fn creation_presets(engine: &Engine) -> Vec<&CharacterSettings> {
    engine.settings.presets.iter().filter(|p| matches!(p.build_method().as_str(), "Priority" | "SumtoTen" | "Karma" | "LifeModule")).collect()
}

// ---------------------------------------------------------------------------
// Smaller additions the creation screens need
// ---------------------------------------------------------------------------

/// Set the character's magical tradition from `traditions.xml`, replacing
/// any previous one and its bonus.
pub fn set_tradition(ch: &mut Character, store: &DataStore, name: &str) -> Result<(), String> {
    let doc = store.doc("traditions.xml").map_err(|e| e.to_string())?;
    let rec = data::find(&doc, "traditions", "tradition", name).ok_or("unknown tradition")?;
    if let Some(old) = ch.doc.child("tradition").map(|t| t.get("guid")) {
        ch.improvements.remove_from_source(&old);
    }
    let guid = new_guid();
    let mut t = Element::new("tradition");
    t.push(Element::with_text("guid", guid.clone()));
    t.push(Element::with_text("traditiontype", "MAG"));
    t.push(Element::with_text("id", rec.id()));
    t.push(Element::with_text("name", rec.name()));
    t.push(Element::new("extra"));
    t.push(Element::with_text("spiritform", rec.el().child_text("spiritform").unwrap_or_else(|| "Materialization".into())));
    // The save stores the drain without braces ("WIL + LOG").
    t.push(Element::with_text("drain", rec.get("drain").replace(['{', '}'], "")));
    t.push(Element::with_text("source", rec.source()));
    t.push(Element::with_text("page", rec.page()));
    for k in ["spiritcombat", "spiritdetection", "spirithealth", "spiritillusion", "spiritmanipulation"] {
        t.push(Element::with_text(k, rec.el().child("spirits").map(|s| s.get(k)).unwrap_or_default()));
    }
    t.push(Element::new("spirits"));
    let bonus_el = rec.el().child("bonus").cloned().unwrap_or_else(|| Element::new("bonus"));
    t.push(bonus_el.clone());
    ch.doc.remove_children("tradition");
    ch.doc.push(t);
    if bonus_el.elements().next().is_some() {
        let src = BonusSource { kind: "Tradition".into(), guid, name: rec.name(), rating: 1 };
        let out = bonus::apply(ch, store, &bonus_el, &src, None);
        finish_outcome(ch, store, out);
    }
    ch.dirty = true;
    Ok(())
}

/// Add a knowledge or language skill.
pub fn add_knowledge_skill(ch: &mut Character, name: &str, kind: &str, native: bool) {
    let guid = new_guid();
    let mut e = Element::new("skill");
    e.push(Element::with_text("guid", guid.clone()));
    e.push(Element::with_text("suid", "00000000-0000-0000-0000-000000000000"));
    e.push(Element::with_text("isknowledge", "True"));
    e.push(Element::with_text("skillcategory", kind));
    e.push(Element::with_text("karma", "0"));
    e.push(Element::with_text("base", "0"));
    e.push(Element::new("notes"));
    e.push(Element::with_text("name", name));
    e.push(Element::with_text("type", kind));
    e.push(Element::with_text("isnativelanguage", bool_str(native)));
    ch.doc.child_or_insert("newskills").child_or_insert("knoskills").push(e.clone());
    ch.knowledge_skills.push(crate::skills::KnowledgeSkill::from_xml(&e));
    ch.dirty = true;
}

pub fn remove_knowledge_skill(ch: &mut Character, guid: &str) {
    ch.knowledge_skills.retain(|k| !k.guid.eq_ignore_ascii_case(guid));
    if let Some(k) = ch.doc.child_mut("newskills").and_then(|n| n.child_mut("knoskills")) {
        k.children.retain(|n| !matches!(n, crate::xml::Node::Element(e) if e.get("guid").eq_ignore_ascii_case(guid)));
    }
    ch.dirty = true;
}

/// Add a contact with a connection and loyalty rating.
pub fn add_contact(ch: &mut Character, name: &str, role: &str, connection: i32, loyalty: i32) {
    let mut c = Element::new("contact");
    let mut put = |k: &str, v: &str| c.push(Element::with_text(k, v));
    put("name", name);
    put("role", role);
    put("location", "");
    put("connection", &connection.to_string());
    put("loyalty", &loyalty.to_string());
    for k in ["metatype", "gender", "age", "contacttype", "preferredpayment", "hobbiesvice", "personallife"] {
        put(k, "");
    }
    put("type", "Contact");
    for k in ["file", "relative", "notes", "groupname"] {
        put(k, "");
    }
    put("colour", "-986896");
    for k in ["group", "family", "blackmail", "free"] {
        put(k, "False");
    }
    put("groupenabled", "True");
    put("guid", &new_guid());
    put("mainmugshotindex", "-1");
    c.push(Element::new("mugshots"));
    ch.items_mut("contacts").push(c);
}

/// Add a specialization to an active skill.
pub fn add_specialization(ch: &mut Character, skill_guid: &str, spec: &str) {
    if let Some(s) = ch.skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(skill_guid)) {
        s.specs.push(crate::skills::Specialization { guid: new_guid(), name: spec.to_owned(), free: false, expertise: false });
        ch.dirty = true;
    }
}

// ---------------------------------------------------------------------------
// Life Modules build
// ---------------------------------------------------------------------------

/// A life module and its versions (`lifemodules.xml`).
#[derive(Debug, Clone)]
pub struct LifeModule {
    pub id: String,
    pub stage: String,
    pub name: String,
    pub karma: i32,
    /// (id, name) of each version; empty when the module has none.
    pub versions: Vec<(String, String)>,
}

/// Stages in order, then the modules of each stage.
pub fn life_modules(store: &DataStore) -> (Vec<String>, Vec<LifeModule>) {
    let Ok(doc) = store.doc("lifemodules.xml") else { return (Vec::new(), Vec::new()) };
    let mut stages: Vec<(i32, String)> = doc
        .child("stages")
        .map(|s| s.children_named("stage").map(|e| (e.attr("order").and_then(|o| o.parse().ok()).unwrap_or(99), e.text())).collect())
        .unwrap_or_default();
    stages.sort();
    let modules = data::records(&doc, "modules", "module")
        .into_iter()
        .map(|m| LifeModule {
            id: m.id(),
            stage: m.get("stage"),
            name: m.name(),
            karma: m.el().get_i32("karma").unwrap_or(0),
            versions: m.el().child("versions").map(|v| v.children_named("version").map(|x| (x.get("id"), x.get("name"))).collect()).unwrap_or_default(),
        })
        .collect();
    (stages.into_iter().map(|(_, s)| s).collect(), modules)
}

/// Add a life module as a `LifeModule` quality, running the chosen
/// version's bonus (`SelectLifeModule` + `Quality.Create`).
pub fn add_life_module(ch: &mut Character, store: &DataStore, module_id: &str, version_id: Option<&str>) -> Result<String, String> {
    let doc = store.doc("lifemodules.xml").map_err(|e| e.to_string())?;
    let m = data::find(&doc, "modules", "module", module_id).ok_or("unknown life module")?;
    let version = version_id.and_then(|v| m.el().child("versions").and_then(|vs| vs.children_named("version").find(|x| x.get("id").eq_ignore_ascii_case(v))));
    // A quality-shaped record: module identity, version effects.
    let mut rec = Element::new("quality");
    rec.push(Element::with_text("id", version.map(|v| v.get("id")).unwrap_or_else(|| m.id())));
    let name = match version {
        Some(v) if !v.get("name").is_empty() => format!("{} ({})", m.name(), v.get("name")),
        _ => m.name(),
    };
    rec.push(Element::with_text("name", name));
    rec.push(Element::with_text("karma", m.get("karma")));
    rec.push(Element::with_text("category", "LifeModule"));
    rec.push(Element::with_text("contributetolimit", "False"));
    rec.push(Element::with_text("source", m.el().child_text("source").unwrap_or_else(|| "RF".into())));
    rec.push(Element::with_text("page", m.get("page")));
    if let Some(b) = version.and_then(|v| v.child("bonus")).or_else(|| m.el().child("bonus")) {
        rec.push(b.clone());
    }
    let guid = add_quality_with_source(ch, store, Record(&rec), None, "Selected", true);
    if let Some(q) = crate::items::find_by_guid_mut(ch.items_mut("qualities"), &guid) {
        q.set_child_text("stage", m.get("stage"));
    }
    Ok(guid)
}
