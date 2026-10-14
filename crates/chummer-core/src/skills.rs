//! Active skills, knowledge skills and skill groups as saved in `.chum5`.

use crate::improvement::bool_str;
use crate::xml::Element;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Specialization {
    pub guid: String,
    pub name: String,
    pub free: bool,
    pub expertise: bool,
}

impl Specialization {
    fn from_xml(e: &Element) -> Self {
        Specialization {
            guid: e.get("guid"),
            name: e.get("name"),
            free: e.get_bool("free").unwrap_or(false),
            expertise: e.get_bool("expertise").unwrap_or(false),
        }
    }
    fn to_xml(&self) -> Element {
        let mut e = Element::new("spec");
        e.push(Element::with_text("guid", self.guid.clone()));
        e.push(Element::with_text("name", self.name.clone()));
        // `SkillSpecialization.Save` writes both flags, True or False.
        e.push(Element::with_text("free", bool_str(self.free)));
        e.push(Element::with_text("expertise", bool_str(self.expertise)));
        e
    }
}

/// The name a new player-chosen specialization gets: any text, trimmed.
/// Refused when empty or when `specs` already has it (ignoring case).
/// Chummer refuses only the empty name; see docs/deviations.md.
pub fn new_spec_name(specs: &[Specialization], name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a specialization needs a name".into());
    }
    if specs.iter().any(|s| s.name.trim().eq_ignore_ascii_case(name)) {
        return Err(format!("the skill already has the specialization {name}"));
    }
    Ok(name.to_owned())
}

fn read_specs(e: &Element) -> Vec<Specialization> {
    e.child("specs").map(|s| s.children_named("spec").map(Specialization::from_xml).collect()).unwrap_or_default()
}

fn write_specs(e: &mut Element, specs: &[Specialization]) {
    let s = e.child_or_insert("specs");
    s.children.clear();
    for sp in specs {
        s.push(sp.to_xml());
    }
}

/// An active skill. Its name, attribute and group come from `skills.xml`
/// via `suid`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Skill {
    pub guid: String,
    /// Id of the skill's record in `skills.xml`.
    pub suid: String,
    pub category: String,
    pub base: i32,
    pub karma: i32,
    pub buy_with_karma: bool,
    pub specs: Vec<Specialization>,
    pub notes: String,
    /// Exotic skills carry a user-chosen specific (e.g. a weapon).
    pub specific: String,
}

impl Skill {
    pub fn from_xml(e: &Element) -> Self {
        Skill {
            guid: e.get("guid"),
            suid: e.get("suid"),
            category: e.get("skillcategory"),
            base: e.get_i32("base").unwrap_or(0),
            karma: e.get_i32("karma").unwrap_or(0),
            buy_with_karma: e.get_bool("buywithkarma").unwrap_or(false),
            specs: read_specs(e),
            notes: e.get("notes"),
            specific: e.get("specific"),
        }
    }

    pub fn write_into(&self, e: &mut Element) {
        e.set_child_text("base", self.base.to_string());
        e.set_child_text("karma", self.karma.to_string());
        e.set_child_text("notes", self.notes.clone());
        if e.child("buywithkarma").is_some() || self.buy_with_karma {
            e.set_child_text("buywithkarma", bool_str(self.buy_with_karma));
        }
        if !self.specs.is_empty() || e.child("specs").is_some() {
            write_specs(e, &self.specs);
        }
    }
}

/// A knowledge or language skill. Name and type are free-form.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KnowledgeSkill {
    pub guid: String,
    pub suid: String,
    pub name: String,
    /// `Academic`, `Interest`, `Language`, `Professional` or `Street`.
    pub kind: String,
    pub base: i32,
    pub karma: i32,
    pub native_language: bool,
    pub specs: Vec<Specialization>,
    pub notes: String,
}

impl KnowledgeSkill {
    pub fn from_xml(e: &Element) -> Self {
        KnowledgeSkill {
            guid: e.get("guid"),
            suid: e.get("suid"),
            name: e.get("name"),
            kind: e.child_text("type").unwrap_or_else(|| e.get("skillcategory")),
            base: e.get_i32("base").unwrap_or(0),
            karma: e.get_i32("karma").unwrap_or(0),
            native_language: e.get_bool("isnativelanguage").unwrap_or(false),
            specs: read_specs(e),
            notes: e.get("notes"),
        }
    }

    pub fn write_into(&self, e: &mut Element) {
        e.set_child_text("name", self.name.clone());
        e.set_child_text("type", self.kind.clone());
        e.set_child_text("base", self.base.to_string());
        e.set_child_text("karma", self.karma.to_string());
        e.set_child_text("notes", self.notes.clone());
        if self.native_language || e.child("isnativelanguage").is_some() {
            e.set_child_text("isnativelanguage", bool_str(self.native_language));
        }
        if !self.specs.is_empty() || e.child("specs").is_some() {
            write_specs(e, &self.specs);
        }
    }

    /// Knowledge skills link to INT or LOG by type.
    pub fn attribute(&self) -> &'static str {
        match self.kind.as_str() {
            "Interest" | "Street" | "Language" => "INT",
            _ => "LOG",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkillGroup {
    pub id: String,
    pub name: String,
    pub base: i32,
    pub karma: i32,
}

impl SkillGroup {
    pub fn from_xml(e: &Element) -> Self {
        SkillGroup { id: e.get("id"), name: e.get("name"), base: e.get_i32("base").unwrap_or(0), karma: e.get_i32("karma").unwrap_or(0) }
    }

    pub fn write_into(&self, e: &mut Element) {
        e.set_child_text("base", self.base.to_string());
        e.set_child_text("karma", self.karma.to_string());
    }

    pub fn rating(&self) -> i32 {
        self.base + self.karma
    }
}

/// Where a suggested specialization comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecSource {
    /// The skill's `<specs>` in skills.xml.
    Data,
    /// Offered by an improvement (`SkillSpecializationOption`): a martial
    /// art offers its own name for Unarmed Combat.
    Granted,
    /// `[Martial Art]` in the data: one of the character's martial arts.
    MartialArt,
    /// A weapon the character has that uses the skill.
    OwnWeapon,
    /// A weapon from weapons.xml (`SelectSpec`'s list for combat skills).
    Weapon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSuggestion {
    pub name: String,
    pub source: SpecSource,
}

/// Suggestions for a new specialization of an active skill. Any text is
/// still allowed; these fill the list under the box.
///
/// Chummer offers the data's `<specs>` and the `SkillSpecializationOption`
/// improvements' names (`CGLSpecializations`), and, for "Combat Active" skills
/// in its career dialog (`SelectSpec`), the weapons.xml weapons whose
/// category is the skill's name or whose `spec`/`spec2` is one of the
/// data's specs, from enabled books. chummer-rs also expands
/// `[Martial Art]` to the character's martial arts (for arts without that
/// option, e.g. custom data), leaves out the other
/// `[...]` placeholders (they are hints, not names), and puts the
/// character's own weapons for the skill first (docs/deviations.md).
/// Specializations the skill has already are left out.
pub fn spec_suggestions(engine: &crate::engine::Engine, ch: &crate::character::Character, skill_guid: &str) -> Vec<SpecSuggestion> {
    if let Some(k) = ch.knowledge_skills.iter().find(|s| s.guid.eq_ignore_ascii_case(skill_guid)) {
        return knowledge_suggestions(engine, ch, k);
    }
    let Some(skill) = ch.skills.iter().find(|s| s.guid.eq_ignore_ascii_case(skill_guid)) else { return Vec::new() };
    let Some(def) = engine.catalog.get(&skill.suid) else { return Vec::new() };
    let mut out: Vec<SpecSuggestion> = Vec::new();
    let mut push = |name: String, source: SpecSource| {
        let taken = |n: &str| skill.specs.iter().any(|s| s.name.trim().eq_ignore_ascii_case(n));
        if !name.trim().is_empty() && !taken(&name) && !out.iter().any(|o| o.name == name) {
            out.push(SpecSuggestion { name, source });
        }
    };
    let mut data: Vec<&String> = def.specs.iter().filter(|s| !s.starts_with('[')).collect();
    data.sort();
    for s in data {
        push(s.clone(), SpecSource::Data);
    }
    let mut granted: Vec<String> = ch.improvements.of_kind("SkillSpecializationOption").filter(|i| i.improved_name == def.name).map(|i| i.unique_name.clone()).collect();
    granted.sort();
    for g in granted {
        push(g, SpecSource::Granted);
    }
    if def.specs.iter().any(|s| s == "[Martial Art]") {
        let mut arts: Vec<String> = ch.items("martialarts", "martialart").iter().map(|m| m.get("name")).collect();
        arts.sort();
        for a in arts {
            push(a, SpecSource::MartialArt);
        }
    }
    if def.category != "Combat Active" {
        return out;
    }
    let mut own: Vec<String> = ch.items("weapons", "weapon").iter().filter(|w| crate::items::weapon::weapon_skill(w) == def.name).map(|w| w.get("name")).collect();
    own.sort();
    for w in own {
        push(w, SpecSource::OwnWeapon);
    }
    let store = engine.store_for_character(ch);
    let books = engine.settings().resolve_for(ch).map(crate::settings::CharacterSettings::books).unwrap_or_default();
    if let Ok(doc) = store.doc("weapons.xml") {
        let in_specs = |v: String| !v.is_empty() && def.specs.contains(&v);
        let mut list: Vec<String> = crate::data::records(&doc, "weapons", "weapon")
            .into_iter()
            .filter(|r| !r.hidden() && (books.is_empty() || books.contains(&r.source())))
            .filter(|r| r.category() == def.name || in_specs(r.get("spec")) || in_specs(r.get("spec2")))
            .map(|r| r.name())
            .collect();
        list.sort();
        for w in list {
            push(w, SpecSource::Weapon);
        }
    }
    out
}

/// A knowledge skill's suggestions (`SelectSpec` in "Knowledge" mode and
/// `CGLSpecializations`): the `<specs>` of its skills.xml record (by id,
/// else by name; a custom-named skill has none), without `[...]`
/// placeholders, then names offered by improvements.
fn knowledge_suggestions(engine: &crate::engine::Engine, ch: &crate::character::Character, k: &KnowledgeSkill) -> Vec<SpecSuggestion> {
    let def = engine.catalog.get(&k.suid).filter(|d| d.name == k.name).or_else(|| engine.catalog.knowledge.iter().find(|d| d.name == k.name));
    let taken = |n: &str| k.specs.iter().any(|s| s.name.trim().eq_ignore_ascii_case(n));
    let mut data: Vec<String> = def.map(|d| d.specs.iter().filter(|s| !s.starts_with('[')).cloned().collect()).unwrap_or_default();
    data.sort();
    let mut granted: Vec<String> = ch.improvements.of_kind("SkillSpecializationOption").filter(|i| i.improved_name == k.name).map(|i| i.unique_name.clone()).collect();
    granted.sort();
    let mut out: Vec<SpecSuggestion> = Vec::new();
    for (name, source) in data.into_iter().map(|n| (n, SpecSource::Data)).chain(granted.into_iter().map(|n| (n, SpecSource::Granted))) {
        if !name.trim().is_empty() && !taken(&name) && !out.iter().any(|o| o.name == name) {
            out.push(SpecSuggestion { name, source });
        }
    }
    out
}
