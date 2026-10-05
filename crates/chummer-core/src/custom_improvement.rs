//! Custom improvements: the one-off modifiers a GM adds on the
//! Improvements tab.
//!
//! Port of `CreateImprovement` and the Improvements tab handlers of
//! `CharacterCareer`. `data/improvements.xml` lists the selectable types.
//! Each type names a bonus node (`<internal>`), the fields the form shows
//! and an XML template. The form fills the template, wraps it in
//! `<bonus><internal>..</internal></bonus>` and runs it through the bonus
//! processor with `ImprovementSource.Custom`, so a custom improvement is
//! exactly what the same bonus on a quality would make. The first
//! improvement made then gets `<custom>True</custom>`, the name, the type
//! id, the group, the notes and the sort order.
//!
//! Groups ("locations") are saved as `<improvementgroups>` in the
//! character document.

use crate::bonus::{self, BonusSource};
use crate::character::Character;
use crate::data::{self, DataStore};
use crate::improvement::{bool_str, fmt_num, Improvement};
use crate::settings::CharacterSettings;
use crate::xml::{self, Element};

pub const FILE: &str = "improvements.xml";
/// `ImprovementSource` of improvements made here.
pub const SOURCE: &str = "Custom";

/// One `<field>` of an improvement type.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Val,
    Min,
    Max,
    Aug,
    /// Shown in the Augmented box, relabelled "Percent".
    Percent,
    ApplyToRating,
    Free,
    /// `SelectAttribute`, `SelectSkill`, ...: pick a value from a list.
    Select(String),
    /// `SelectXPath`: pick the `<name>` of the nodes at `xpath` in `file`.
    SelectXPath { file: String, xpath: String },
}

/// An entry of `improvements.xml`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImprovementType {
    pub id: String,
    pub name: String,
    /// The bonus node the template goes into, e.g. `specificattribute`.
    pub internal: String,
    pub fields: Vec<Field>,
    pub template: String,
    /// Attributes of `<xml>`, copied onto the bonus node (`forced="True"`).
    pub template_attrs: Vec<(String, String)>,
    /// Help text (`altpage` when translated, else `page`).
    pub page: String,
}

impl ImprovementType {
    fn from_xml(e: &Element) -> Option<Self> {
        let id = e.get("id");
        if id.is_empty() {
            return None;
        }
        let fields = e
            .child("fields")
            .map(|f| {
                f.children_named("field")
                    .filter_map(|n| {
                        Some(match n.text().trim() {
                            "val" => Field::Val,
                            "min" => Field::Min,
                            "max" => Field::Max,
                            "aug" => Field::Aug,
                            "percent" => Field::Percent,
                            "applytorating" => Field::ApplyToRating,
                            "free" => Field::Free,
                            "SelectXPath" => Field::SelectXPath { file: n.attr("xml").unwrap_or_default().into(), xpath: n.attr("xpath").unwrap_or_default().into() },
                            s if s.to_ascii_lowercase().starts_with("select") => Field::Select(s.to_owned()),
                            _ => return None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let xml = e.child("xml");
        let page = e.child_text("altpage").unwrap_or_else(|| e.get("page"));
        Some(ImprovementType {
            name: e.child_text("name").unwrap_or_else(|| id.clone()),
            id,
            internal: e.get("internal"),
            fields,
            template: xml.map(Element::text).unwrap_or_default(),
            template_attrs: xml.map(|x| x.attrs.clone()).unwrap_or_default(),
            page,
        })
    }

    pub fn has(&self, f: &Field) -> bool {
        self.fields.contains(f)
    }

    /// The selection field, if the type asks for one (`_strSelect`).
    pub fn selection(&self) -> Option<&Field> {
        self.fields.iter().rev().find(|f| matches!(f, Field::Select(_) | Field::SelectXPath { .. }))
    }
}

/// Every improvement type, in file order.
pub fn types(store: &DataStore) -> Vec<ImprovementType> {
    let Ok(doc) = store.doc(FILE) else { return Vec::new() };
    doc.child("improvements").map(|c| c.children_named("improvement").filter_map(ImprovementType::from_xml).collect()).unwrap_or_default()
}

/// The type with this id. Two entries share `cyberseeker`; as in Chummer
/// the first wins.
pub fn find_type(store: &DataStore, id: &str) -> Option<ImprovementType> {
    types(store).into_iter().find(|t| t.id == id)
}

/// What the Create Improvement form holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    pub type_id: String,
    pub name: String,
    pub val: f64,
    pub min: f64,
    pub max: f64,
    /// Augmented, or Percent for types with a `percent` field.
    pub aug: f64,
    pub apply_to_rating: bool,
    pub free: bool,
    pub select: String,
}

impl Form {
    /// The form for editing an existing custom improvement
    /// (`CreateImprovement_Load` with `EditImprovementObject`).
    pub fn from_improvement(i: &Improvement, t: Option<&ImprovementType>) -> Form {
        let has = |f: &Field| t.is_none_or(|t| t.has(f));
        let val = match i.kind.as_str() {
            // Attribute improvements store the Value in Augmented.
            "Attribute" => i.aug,
            // Adept power level improvements store it in Rating.
            "AdeptPowerFreeLevels" => f64::from(i.rating),
            _ => i.val,
        };
        Form {
            type_id: i.custom_id.clone(),
            name: i.custom_name.clone(),
            val: if has(&Field::Val) { val } else { 0.0 },
            min: if has(&Field::Min) { i.min } else { 0.0 },
            max: if has(&Field::Max) { i.max } else { 0.0 },
            // Chummer leaves Augmented empty on edit; the attribute types
            // keep it in AugmentedMaximum, so restore it from there.
            aug: if has(&Field::Aug) && matches!(i.kind.as_str(), "Attribute" | "ReplaceAttribute") { i.aug_max } else { 0.0 },
            apply_to_rating: has(&Field::ApplyToRating) && i.add_to_rating,
            free: false,
            select: if t.is_none_or(|t| t.selection().is_some()) { i.improved_name.clone() } else { String::new() },
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CustomError {
    /// `Message_SelectItem`.
    #[error("You must select an item by clicking the Select Value button.")]
    NoSelection,
    /// `Message_ImprovementName`.
    #[error("Please enter a name for this Improvement.")]
    NoName,
    #[error("unknown improvement type {0}")]
    UnknownType(String),
    #[error("the improvement template is not valid XML: {0}")]
    BadTemplate(String),
    #[error("this improvement had no effect ({0})")]
    NoEffect(String),
}

/// Fill a type's template from the form and build the `<bonus>` node
/// (`AcceptForm`).
pub fn bonus_xml(t: &ImprovementType, f: &Form) -> Result<Element, CustomError> {
    let num = |v: f64| fmt_num((v * 100.0).round() / 100.0);
    let body = t
        .template
        .replace("{val}", &num(f.val))
        .replace("{min}", &num(f.min))
        .replace("{max}", &num(f.max))
        .replace("{aug}", &num(f.aug))
        .replace("{percent}", &num(f.aug))
        .replace("{free}", &bool_str(f.free))
        .replace("{select}", &xml::escape(&f.select, false))
        .replace("{applytorating}", if f.apply_to_rating { "<applytorating>True</applytorating>" } else { "" });
    let attrs: String = t.template_attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", xml::escape(v, true))).collect();
    let src = format!("<bonus><{0}{attrs}>{body}</{0}></bonus>", t.internal);
    xml::parse(&src).map_err(|e| CustomError::BadTemplate(e.to_string()))
}

/// Check the form the way `AcceptForm` does: a selection when the type
/// needs one, then a name.
pub fn validate(t: &ImprovementType, f: &Form) -> Result<(), CustomError> {
    if t.selection().is_some() && f.select.trim().is_empty() {
        return Err(CustomError::NoSelection);
    }
    if f.name.trim().is_empty() {
        return Err(CustomError::NoName);
    }
    Ok(())
}

/// Create a custom improvement in `group` ("" for none), or replace the
/// one from source `edit`. Returns the new source GUID.
pub fn create(ch: &mut Character, store: &DataStore, f: &Form, group: &str, edit: Option<&str>) -> Result<String, CustomError> {
    let t = find_type(store, &f.type_id).ok_or_else(|| CustomError::UnknownType(f.type_id.clone()))?;
    validate(&t, f)?;
    let bonus_el = bonus_xml(&t, f)?;
    let guid = crate::items::new_guid();
    let src = BonusSource { kind: SOURCE.into(), guid: guid.clone(), name: f.name.clone(), rating: 1 };
    let out = bonus::apply(ch, store, &bonus_el, &src, None);
    if out.improvements.is_empty() {
        let why = if out.unsupported.is_empty() { t.internal.clone() } else { out.unsupported.join(", ") };
        return Err(CustomError::NoEffect(why));
    }
    // The edited improvement's notes and order carry over to the new one.
    let (notes, order) = match edit.and_then(|g| first_of(ch, g)) {
        Some(i) => (i.notes.clone(), i.order),
        None => (String::new(), 0),
    };
    if let Some(g) = edit {
        remove(ch, g);
    }
    crate::items::place_added(ch, store, &out.added);
    crate::items::apply_outcome(ch, &out);
    // Chummer marks only the first improvement as custom; the others keep
    // the Custom source and are listed and removed with it.
    if let Some(i) = ch.improvements.list.iter_mut().find(|i| i.source_name == guid) {
        i.custom = true;
        i.custom_name = f.name.clone();
        i.custom_id = t.id.clone();
        i.custom_group = group.to_owned();
        i.notes = notes;
        i.order = order;
    }
    ch.dirty = true;
    Ok(guid)
}

/// The improvement that carries the custom name for a source.
fn first_of<'a>(ch: &'a Character, source: &str) -> Option<&'a Improvement> {
    let mut it = ch.improvements.list.iter().filter(|i| i.source_name.eq_ignore_ascii_case(source));
    let all: Vec<&Improvement> = it.by_ref().collect();
    all.iter().find(|i| i.custom).or(all.first()).copied()
}

/// Indexes of the improvements the tab lists for editing: those from the
/// Custom and Drug sources (`RefreshCustomImprovements`).
pub fn listed(ch: &Character) -> Vec<usize> {
    ch.improvements.list.iter().enumerate().filter(|(_, i)| i.source == SOURCE || i.source == "Drug").map(|(n, _)| n).collect()
}

/// Delete every improvement from a source, the objects its bonus created
/// (spells, qualities, ...), and the tab or attribute flags it enabled
/// (`ImprovementManager.RemoveImprovements`).
pub fn remove(ch: &mut Character, source: &str) -> bool {
    remove_source(ch, source, 0)
}

fn remove_source(ch: &mut Character, source: &str, depth: u32) -> bool {
    let (gone, keep): (Vec<Improvement>, Vec<Improvement>) =
        std::mem::take(&mut ch.improvements.list).into_iter().partition(|i| i.source_name.eq_ignore_ascii_case(source));
    ch.improvements.list = keep;
    if gone.is_empty() {
        return false;
    }
    sync_flags(ch, &gone);
    if depth < 8 {
        for i in &gone {
            // A created object is linked by an improvement named after its GUID.
            if is_guid(&i.improved_name) && !i.improved_name.eq_ignore_ascii_case(source) {
                remove_source(ch, &i.improved_name, depth + 1);
                ch.remove_item_anywhere(&i.improved_name);
            }
        }
    }
    ch.dirty = true;
    true
}

fn is_guid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(n, c)| if matches!(n, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Turn an improvement on or off (`EnableImprovements` /
/// `DisableImprovements`).
pub fn set_enabled(ch: &mut Character, index: usize, enabled: bool) -> bool {
    let Some(i) = ch.improvements.list.get_mut(index) else { return false };
    if i.enabled == enabled {
        return false;
    }
    i.enabled = enabled;
    let i = i.clone();
    sync_flags(ch, std::slice::from_ref(&i));
    ch.dirty = true;
    true
}

/// Enable or disable every custom improvement of a group ("" for the
/// Selected Improvements root). Returns how many changed.
pub fn set_group_enabled(ch: &mut Character, group: &str, enabled: bool) -> usize {
    let hits: Vec<usize> =
        ch.improvements.list.iter().enumerate().filter(|(_, i)| i.custom && i.enabled != enabled && i.custom_group == group).map(|(n, _)| n).collect();
    for &n in &hits {
        set_enabled(ch, n, enabled);
    }
    hits.len()
}

pub fn set_notes(ch: &mut Character, index: usize, notes: &str) {
    if let Some(i) = ch.improvements.list.get_mut(index) {
        i.notes = notes.to_owned();
        ch.dirty = true;
    }
}

/// Move an improvement into another group (drag and drop in Chummer).
pub fn set_group(ch: &mut Character, index: usize, group: &str) {
    if let Some(i) = ch.improvements.list.get_mut(index) {
        i.custom_group = group.to_owned();
        ch.dirty = true;
    }
}

/// Keep `magenabled`, `magician` and the like in step with the
/// improvements that grant them: when an affected `enableattribute` or
/// `enabletab` improvement goes away, the flag stays only if another
/// active one remains.
fn sync_flags(ch: &mut Character, affected: &[Improvement]) {
    for i in affected {
        let Some(flag) = special_flag(i) else { continue };
        let on = ch.improvements.active().any(|j| special_flag(j) == Some(flag));
        if ch.flag(flag) != on {
            ch.set_field(flag, bool_str(on));
        }
    }
}

fn special_flag(i: &Improvement) -> Option<&'static str> {
    match (i.kind.as_str(), i.unique_name.as_str(), i.improved_name.as_str()) {
        ("Attribute", "enableattribute", "MAG") => Some("magenabled"),
        ("Attribute", "enableattribute", "RES") => Some("resenabled"),
        ("Attribute", "enableattribute", "DEP") => Some("depenabled"),
        ("SpecialTab", "enabletab", "Magician") => Some("magician"),
        ("SpecialTab", "enabletab", "Adept") => Some("adept"),
        ("SpecialTab", "enabletab", "Technomancer") => Some("technomancer"),
        ("SpecialTab", "enabletab", "Advanced Programs") => Some("ai"),
        ("SpecialTab", "enabletab", "Critter") => Some("critter"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// The character's improvement groups, in order.
pub fn groups(ch: &Character) -> Vec<String> {
    ch.doc.child("improvementgroups").map(|g| g.children_named("improvementgroup").map(Element::text).collect()).unwrap_or_default()
}

fn write_groups(ch: &mut Character, list: &[String]) {
    let g = ch.items_mut("improvementgroups");
    g.children.clear();
    for n in list {
        g.push(Element::with_text("improvementgroup", n.clone()));
    }
}

/// Add a group (`cmdAddImprovementGroup`). Empty and duplicate names are
/// refused.
pub fn add_group(ch: &mut Character, name: &str) -> bool {
    let name = name.trim();
    let mut list = groups(ch);
    if name.is_empty() || list.iter().any(|g| g == name) {
        return false;
    }
    list.push(name.to_owned());
    write_groups(ch, &list);
    true
}

/// Rename a group and move its improvements along
/// (`tsImprovementRenameLocation`).
pub fn rename_group(ch: &mut Character, old: &str, new: &str) -> bool {
    let new = new.trim();
    let mut list = groups(ch);
    let Some(pos) = list.iter().position(|g| g == old) else { return false };
    if new.is_empty() || list.iter().any(|g| g == new) {
        return false;
    }
    list[pos] = new.to_owned();
    write_groups(ch, &list);
    for i in ch.improvements.list.iter_mut().filter(|i| i.custom_group == old) {
        i.custom_group = new.to_owned();
    }
    true
}

/// Delete a group; its improvements move back to Selected Improvements
/// (`DoDeleteImprovement`).
pub fn remove_group(ch: &mut Character, name: &str) -> bool {
    let mut list = groups(ch);
    let before = list.len();
    list.retain(|g| g != name);
    if list.len() == before {
        return false;
    }
    write_groups(ch, &list);
    for i in ch.improvements.list.iter_mut().filter(|i| i.custom_group == name) {
        i.custom_group.clear();
    }
    true
}

// ---------------------------------------------------------------------------
// Selection lists
// ---------------------------------------------------------------------------

/// The values a selection field offers (`cmdChangeSelection_Click`),
/// in English as they are saved.
pub fn options(ch: &Character, store: &DataStore, settings: Option<&CharacterSettings>, field: &Field) -> Vec<String> {
    use crate::attributes::{MENTAL, PHYSICAL, SPECIAL};
    let names = |file: &str, container: &str, item: &str| -> Vec<String> {
        store.doc(file).map(|d| data::records(&d, container, item).iter().filter(|r| !r.hidden()).map(|r| r.name()).collect()).unwrap_or_default()
    };
    let cats = |file: &str| store.doc(file).map(|d| data::categories(&d)).unwrap_or_default();
    let mystic_second_mag = ch.is_adept() && ch.is_magician() && settings.is_some_and(|s| s.flag("mysadeptsecondmagattribute"));
    let mut v: Vec<String> = match field {
        Field::Select(s) => match s.as_str() {
            "SelectAttribute" => PHYSICAL
                .iter()
                .chain(MENTAL)
                .chain(SPECIAL)
                .filter(|a| match **a {
                    "ESS" => false,
                    "MAG" => ch.mag_enabled(),
                    "MAGAdept" => ch.mag_enabled() && mystic_second_mag,
                    "RES" => ch.res_enabled(),
                    "DEP" => ch.dep_enabled(),
                    _ => true,
                })
                .map(|a| (*a).to_owned())
                .collect(),
            "SelectPhysicalAttribute" => PHYSICAL.iter().map(|a| (*a).to_owned()).collect(),
            "SelectMentalAttribute" => MENTAL.iter().map(|a| (*a).to_owned()).collect(),
            "SelectSpecialAttribute" => SPECIAL.iter().filter(|a| **a != "ESS").map(|a| (*a).to_owned()).collect(),
            "SelectSkill" => {
                let mut v = names("skills.xml", "skills", "skill");
                v.sort();
                v
            }
            "SelectKnowSkill" => {
                let mut v: Vec<String> = ch.knowledge_skills.iter().map(|k| k.name.clone()).collect();
                for n in names("skills.xml", "knowledgeskills", "skill") {
                    if !v.contains(&n) {
                        v.push(n);
                    }
                }
                v.sort();
                v
            }
            "SelectSkillCategory" => cats("skills.xml"),
            "SelectSkillGroup" => store
                .doc("skills.xml")
                .map(|d| d.child("skillgroups").map(|g| g.children_named("name").map(Element::text).collect()).unwrap_or_default())
                .unwrap_or_default(),
            "SelectWeaponCategory" => cats("weapons.xml"),
            "SelectSpellCategory" => cats("spells.xml"),
            "SelectSpell" => names("spells.xml", "spells", "spell"),
            "SelectComplexForm" => names("complexforms.xml", "complexforms", "complexform"),
            "SelectAdeptPower" => names("powers.xml", "powers", "power"),
            "SelectMetamagic" => names("metamagic.xml", "metamagics", "metamagic"),
            "SelectEcho" => names("echoes.xml", "echoes", "echo"),
            "SelectActionDicePool" => names("actions.xml", "actions", "action"),
            _ => Vec::new(),
        },
        Field::SelectXPath { file, xpath } => {
            let mut v = xpath_names(store, file, xpath);
            v.sort();
            v
        }
        _ => Vec::new(),
    };
    v.dedup();
    v
}

/// Names of the nodes at a plain `/chummer/a/b` path
/// (`PopulateSelectValueComboAsync`): their `<name>`, else their text.
fn xpath_names(store: &DataStore, file: &str, xpath: &str) -> Vec<String> {
    let Ok(doc) = store.doc(file) else { return Vec::new() };
    let mut parts: Vec<&str> = xpath.split('/').filter(|p| !p.is_empty()).collect();
    if parts.first() == Some(&doc.name.as_str()) {
        parts.remove(0);
    }
    let Some((last, path)) = parts.split_last() else { return Vec::new() };
    let mut node: &Element = &doc;
    for p in path {
        match node.child(p) {
            Some(n) => node = n,
            None => return Vec::new(),
        }
    }
    node.children_named(last)
        .map(|e| e.child_text("name").filter(|n| !n.is_empty()).unwrap_or_else(|| e.text().trim().to_owned()))
        .filter(|n| !n.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(internal: &str, template: &str, fields: Vec<Field>) -> ImprovementType {
        ImprovementType { id: internal.into(), name: internal.into(), internal: internal.into(), fields, template: template.into(), template_attrs: Vec::new(), page: String::new() }
    }

    #[test]
    fn template_is_filled_like_accept_form() {
        let t = ty("specificattribute", "<name>{select}</name><val>{val}</val><min>{min}</min><max>{max}</max><aug>{aug}</aug>", vec![Field::Select("SelectAttribute".into()), Field::Val]);
        let f = Form { select: "AGI".into(), val: 1.0, name: "GM bonus".into(), ..Default::default() };
        let b = bonus_xml(&t, &f).unwrap();
        let s = b.to_xml_string();
        let body = s.split_once("?>").map_or(s.as_str(), |(_, b)| b);
        assert_eq!(body.split_whitespace().collect::<String>(), "<bonus><specificattribute><name>AGI</name><val>1</val><min>0</min><max>0</max><aug>0</aug></specificattribute></bonus>");
        let mut t = ty("addspell", "{select}", vec![Field::Select("SelectSpell".into())]);
        t.template_attrs = vec![("forced".into(), "True".into())];
        let b = bonus_xml(&t, &Form { select: "Fire & Ice".into(), ..Default::default() }).unwrap();
        let n = b.child("addspell").unwrap();
        assert_eq!(n.attr("forced"), Some("True"));
        assert_eq!(n.text(), "Fire & Ice");
        let t = ty("skillgroup", "<name>{select}</name><val>{val}</val>{applytorating}", vec![Field::ApplyToRating]);
        let b = bonus_xml(&t, &Form { val: 0.5, apply_to_rating: true, ..Default::default() }).unwrap();
        assert_eq!(b.child("skillgroup").unwrap().get("applytorating"), "True");
        assert_eq!(b.child("skillgroup").unwrap().get("val"), "0.5");
    }

    #[test]
    fn validation_order() {
        let t = ty("x", "", vec![Field::Select("SelectSkill".into())]);
        assert_eq!(validate(&t, &Form::default()), Err(CustomError::NoSelection));
        assert_eq!(validate(&t, &Form { select: "Pistols".into(), ..Default::default() }), Err(CustomError::NoName));
        let t = ty("x", "", vec![Field::Val]);
        assert_eq!(validate(&t, &Form { name: "n".into(), ..Default::default() }), Ok(()));
    }

    #[test]
    fn guid_detection() {
        assert!(is_guid("0b8f2c8e-1d2a-4b3c-9d4e-5f6a7b8c9d0e"));
        assert!(!is_guid("AGI"));
    }
}
