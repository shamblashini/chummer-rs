//! Character attributes (BOD, AGI, ... MAG, RES, EDG, ESS, DEP).

use crate::xml::Element;

pub const PHYSICAL: &[&str] = &["BOD", "AGI", "REA", "STR"];
pub const MENTAL: &[&str] = &["CHA", "INT", "LOG", "WIL"];
pub const SPECIAL: &[&str] = &["EDG", "MAG", "MAGAdept", "RES", "DEP", "ESS"];

pub fn long_name(abbrev: &str) -> &'static str {
    match abbrev {
        "BOD" => "Body",
        "AGI" => "Agility",
        "REA" => "Reaction",
        "STR" => "Strength",
        "CHA" => "Charisma",
        "INT" => "Intuition",
        "LOG" => "Logic",
        "WIL" => "Willpower",
        "EDG" => "Edge",
        "MAG" => "Magic",
        "MAGAdept" => "Magic (Adept)",
        "RES" => "Resonance",
        "DEP" => "Depth",
        "ESS" => "Essence",
        _ => "",
    }
}

/// One attribute as saved. Computed values live in [`crate::calc`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attribute {
    pub name: String,
    pub metatype_min: i32,
    pub metatype_max: i32,
    pub metatype_aug_max: i32,
    /// Points bought with attribute (priority) points.
    pub base: i32,
    /// Points bought with karma.
    pub karma: i32,
    /// `Standard`, `Special` or `Shapeshifter`.
    pub category: String,
    /// The total Chummer computed when it saved the file.
    pub saved_total: Option<i32>,
}

impl Attribute {
    /// Parse an attribute without load-time fixups.
    pub fn from_xml(e: &Element) -> Self {
        Attribute {
            name: e.get("name"),
            metatype_min: e.get_i32("metatypemin").unwrap_or(1),
            metatype_max: e.get_i32("metatypemax").unwrap_or(6),
            metatype_aug_max: e.get_i32("metatypeaugmax").unwrap_or(10),
            base: e.get_i32("base").unwrap_or(0),
            karma: e.get_i32("karma").unwrap_or(0),
            category: e.get("metatypecategory"),
            saved_total: e.get_i32("totalvalue"),
        }
    }

    /// Parse with Chummer's load-time fixups (`CharacterAttrib.LoadCore`).
    /// `base_unlocked` is true for priority-table builds.
    pub fn load(e: &Element, base_unlocked: bool, created: bool) -> Self {
        let mut a = Attribute::from_xml(e);
        if !base_unlocked && !created {
            a.base = 0;
        }
        // Old files store an absolute <value> instead of base + karma.
        if let Some(mut v) = e.get_i32("value") {
            v -= a.metatype_min;
            if base_unlocked {
                a.base = (a.base - a.metatype_min).max(0);
                v -= a.base;
            }
            if v > 0 {
                a.karma = v;
            }
        }
        if let Some(k) = e.get_i32("createkarma") {
            a.karma += k;
        }
        a.base = a.base.max(0);
        a.karma = a.karma.max(0);
        if !PHYSICAL.contains(&a.name.as_str()) && !MENTAL.contains(&a.name.as_str()) {
            a.category = "Special".into();
        } else if a.category.is_empty() {
            a.category = "Standard".into();
        }
        a
    }

    pub fn write_into(&self, e: &mut Element) {
        e.remove_children("value");
        e.remove_children("createkarma");
        e.remove_children("augmodifier");
        e.set_child_text("name", self.name.clone());
        e.set_child_text("metatypemin", self.metatype_min.to_string());
        e.set_child_text("metatypemax", self.metatype_max.to_string());
        e.set_child_text("metatypeaugmax", self.metatype_aug_max.to_string());
        e.set_child_text("base", self.base.to_string());
        e.set_child_text("karma", self.karma.to_string());
        e.set_child_text("metatypecategory", self.category.clone());
    }
}
