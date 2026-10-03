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
        if self.free {
            e.push(Element::with_text("free", "True"));
        }
        if self.expertise {
            e.push(Element::with_text("expertise", "True"));
        }
        e
    }
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
