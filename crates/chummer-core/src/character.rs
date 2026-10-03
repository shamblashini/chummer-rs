//! A Shadowrun character as stored in a `.chum5` file.
//!
//! The parsed document stays in [`Character::doc`] so that saving writes
//! back every element, including the many this port does not model yet.
//! The typed fields below are loaded from the document and written back
//! into it by [`Character::to_document`].

use std::path::{Path, PathBuf};

use crate::attributes::Attribute;
use crate::improvement::{bool_str, Improvement, Improvements};
use crate::skills::{KnowledgeSkill, Skill, SkillGroup};
use crate::xml::{self, Element};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is not a valid character file: {1}")]
    Xml(PathBuf, xml::XmlError),
    #[error("{0} is not a Chummer character (root element <{1}>)")]
    NotCharacter(PathBuf, String),
}

/// Text fields shown on the "Character Info" tab.
pub const INFO_FIELDS: &[(&str, &str)] = &[
    ("name", "Name"),
    ("alias", "Alias"),
    ("playername", "Player"),
    ("metatype", "Metatype"),
    ("metavariant", "Metavariant"),
    ("sex", "Sex"),
    ("age", "Age"),
    ("height", "Height"),
    ("weight", "Weight"),
    ("eyes", "Eyes"),
    ("hair", "Hair"),
    ("skin", "Skin"),
];

/// Long text fields.
pub const TEXT_FIELDS: &[(&str, &str)] = &[
    ("concept", "Concept"),
    ("description", "Description"),
    ("background", "Background"),
    ("notes", "Notes"),
    ("gamenotes", "Game Notes"),
];

#[derive(Debug, Clone)]
pub struct Character {
    /// The full document. Typed fields are authoritative for what they
    /// cover; everything else is read and written here directly.
    pub doc: Element,
    pub file: Option<PathBuf>,
    pub attributes: Vec<Attribute>,
    pub skills: Vec<Skill>,
    pub skill_groups: Vec<SkillGroup>,
    pub knowledge_skills: Vec<KnowledgeSkill>,
    pub improvements: Improvements,
    pub karma: i32,
    pub nuyen: f64,
    /// `true` once the character has left creation ("career mode").
    pub created: bool,
    pub physical_cm_filled: i32,
    pub stun_cm_filled: i32,
    pub dirty: bool,
}

/// Priority and Sum-to-Ten builds spend attribute points; Karma and Life
/// Module builds do not.
pub fn uses_priority_tables(build_method: &str) -> bool {
    matches!(build_method, "Priority" | "SumtoTen")
}

impl Character {
    pub fn load(path: &Path) -> Result<Character, LoadError> {
        let src = std::fs::read_to_string(path).map_err(|e| LoadError::Io(path.to_owned(), e))?;
        let mut c = Character::from_str(&src).map_err(|e| match e {
            LoadError::Xml(_, x) => LoadError::Xml(path.to_owned(), x),
            LoadError::NotCharacter(_, r) => LoadError::NotCharacter(path.to_owned(), r),
            other => other,
        })?;
        c.file = Some(path.to_owned());
        Ok(c)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(src: &str) -> Result<Character, LoadError> {
        let doc = xml::parse(src).map_err(|e| LoadError::Xml(PathBuf::new(), e))?;
        Character::from_document(doc)
    }

    pub fn from_document(doc: Element) -> Result<Character, LoadError> {
        if doc.name != "character" {
            return Err(LoadError::NotCharacter(PathBuf::new(), doc.name.clone()));
        }
        let created = doc.get_bool("created").unwrap_or(false);
        let base_unlocked = uses_priority_tables(&doc.get("buildmethod"));
        let attributes = doc
            .child("attributes")
            .map(|a| a.children_named("attribute").map(|e| Attribute::load(e, base_unlocked, created)).collect())
            .unwrap_or_default();
        let improvements = Improvements {
            career: created,
            list: doc
                .child("improvements")
                .map(|i| i.children_named("improvement").map(Improvement::from_xml).collect())
                .unwrap_or_default(),
        };
        let ns = doc.child("newskills");
        let skills = ns
            .and_then(|n| n.child("skills"))
            .map(|s| s.children_named("skill").map(Skill::from_xml).collect())
            .unwrap_or_default();
        let knowledge_skills = ns
            .and_then(|n| n.child("knoskills"))
            .map(|s| s.children_named("skill").map(KnowledgeSkill::from_xml).collect())
            .unwrap_or_default();
        let skill_groups = ns
            .and_then(|n| n.child("groups"))
            .map(|s| s.children_named("group").map(SkillGroup::from_xml).collect())
            .unwrap_or_default();
        Ok(Character {
            karma: doc.get_i32("karma").unwrap_or(0),
            nuyen: doc.get_f64("nuyen").unwrap_or(0.0),
            created,
            physical_cm_filled: doc.get_i32("physicalcmfilled").unwrap_or(0),
            stun_cm_filled: doc.get_i32("stuncmfilled").unwrap_or(0),
            attributes,
            skills,
            skill_groups,
            knowledge_skills,
            improvements,
            doc,
            file: None,
            dirty: false,
        })
    }

    /// Write typed fields back into a copy of the document.
    pub fn to_document(&self) -> Element {
        let mut doc = self.doc.clone();
        // Chummer parses <appversion> to pick load shims, so leave it as
        // loaded and record our own version separately.
        doc.set_child_text("chummerrsversion", env!("CARGO_PKG_VERSION"));
        doc.set_child_text("karma", self.karma.to_string());
        doc.set_child_text("nuyen", crate::improvement::fmt_num(self.nuyen));
        doc.set_child_text("created", bool_str(self.created));
        doc.set_child_text("physicalcmfilled", self.physical_cm_filled.to_string());
        doc.set_child_text("stuncmfilled", self.stun_cm_filled.to_string());

        let attrs = doc.child_or_insert("attributes");
        for a in &self.attributes {
            let existing = attrs.elements_mut().find(|e| e.name == "attribute" && e.get("name") == a.name);
            if let Some(e) = existing {
                a.write_into(e);
                continue;
            }
            let mut e = Element::new("attribute");
            a.write_into(&mut e);
            attrs.push(e);
        }

        let imps = doc.child_or_insert("improvements");
        imps.children.clear();
        for i in &self.improvements.list {
            imps.push(i.to_xml());
        }

        let ns = doc.child_or_insert("newskills");
        let skills = ns.child_or_insert("skills");
        for s in &self.skills {
            if let Some(e) = skills.elements_mut().find(|e| e.get("guid").eq_ignore_ascii_case(&s.guid)) {
                s.write_into(e);
            }
        }
        let kno = ns.child_or_insert("knoskills");
        for s in &self.knowledge_skills {
            if let Some(e) = kno.elements_mut().find(|e| e.get("guid").eq_ignore_ascii_case(&s.guid)) {
                s.write_into(e);
            }
        }
        let groups = ns.child_or_insert("groups");
        for g in &self.skill_groups {
            if let Some(e) = groups.elements_mut().find(|e| e.get("name") == g.name) {
                g.write_into(e);
            }
        }
        doc
    }

    pub fn to_xml_string(&self) -> String {
        self.to_document().to_xml_string()
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("chum5.tmp");
        std::fs::write(&tmp, self.to_xml_string())?;
        std::fs::rename(&tmp, path)?;
        self.file = Some(path.to_owned());
        self.dirty = false;
        Ok(())
    }

    // ----- simple accessors over the document -----

    pub fn field(&self, key: &str) -> String {
        self.doc.get(key)
    }

    pub fn set_field(&mut self, key: &str, value: impl Into<String>) {
        self.doc.set_child_text(key, value);
        self.dirty = true;
    }

    pub fn name(&self) -> String {
        self.field("name")
    }

    /// Alias if set, else name, else "Unnamed Character".
    pub fn display_name(&self) -> String {
        let alias = self.field("alias");
        if !alias.trim().is_empty() {
            return alias;
        }
        let name = self.field("name");
        if !name.trim().is_empty() {
            return name;
        }
        "Unnamed Character".into()
    }

    pub fn flag(&self, key: &str) -> bool {
        self.doc.get_bool(key).unwrap_or(false)
    }

    pub fn mag_enabled(&self) -> bool {
        self.flag("magenabled")
    }
    pub fn res_enabled(&self) -> bool {
        self.flag("resenabled")
    }
    pub fn dep_enabled(&self) -> bool {
        self.flag("depenabled")
    }
    pub fn is_adept(&self) -> bool {
        self.flag("adept")
    }
    pub fn is_magician(&self) -> bool {
        self.flag("magician")
    }
    pub fn is_technomancer(&self) -> bool {
        self.flag("technomancer")
    }

    pub fn attribute(&self, abbrev: &str) -> Option<&Attribute> {
        // Shapeshifters save both a Standard and a Shapeshifter set; the
        // standard set comes first.
        self.attributes.iter().find(|a| a.name == abbrev)
    }

    pub fn attribute_mut(&mut self, abbrev: &str) -> Option<&mut Attribute> {
        self.attributes.iter_mut().find(|a| a.name == abbrev)
    }

    /// Items of a section, e.g. `items("gears", "gear")`.
    pub fn items<'a>(&'a self, container: &str, item: &'a str) -> Vec<&'a Element> {
        self.doc.child(container).map(|c| c.children_named(item).collect()).unwrap_or_default()
    }

    pub fn items_mut(&mut self, container: &str) -> &mut Element {
        self.dirty = true;
        self.doc.child_or_insert(container)
    }

    /// Remove a top-level item by guid, along with the improvements it made.
    pub fn remove_item(&mut self, container: &str, guid: &str) -> bool {
        let Some(c) = self.doc.child_mut(container) else { return false };
        let before = c.children.len();
        c.children.retain(|n| !matches!(n, xml::Node::Element(e) if e.get("guid").eq_ignore_ascii_case(guid)));
        let removed = c.children.len() != before;
        if removed {
            self.improvements.remove_from_source(guid);
            self.dirty = true;
        }
        removed
    }
}
