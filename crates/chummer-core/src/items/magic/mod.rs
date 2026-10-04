//! Magic and resonance: spells, adept powers, complex forms, spirits and
//! sprites, metamagic and echoes, initiation and submersion, martial arts,
//! critter powers, mentor spirits and foci.
//!
//! Each kind lives in its own submodule with the `items` contract
//! (`element`, `add`, `choices`, `rebuild`, `IGNORE`). This module routes
//! the `items` entry points by kind tag and holds the bonus hooks that
//! `bonus/handlers.rs` calls. Accounting (power points, free spells, karma
//! costs) is in [`account`]; the GUI summary in [`summary`].

pub mod account;
pub mod complexform;
pub mod critterpower;
pub mod hooks;
pub mod initiation;
pub mod martialart;
pub mod mentor;
pub mod metamagic;
pub mod power;
pub mod spell;
pub mod spirit;
pub mod summary;

pub use account::{
    bind_focus, complex_form_counts, complex_form_karma, complex_form_karma_cost, focus_binding_karma, initiation_karma, power_points, power_points_with,
    spell_counts, spell_karma, spell_karma_cost, unbind_focus, FormCounts, SpellCounts,
};
pub use hooks::{bonus_add_magic, bonus_critterpowers, bonus_power, bonus_spirit};
pub use mentor::{add_mentor, mentor_outcome, set_mentor_choices};
pub use summary::{magic_summary, magic_summary_with, MagicSummary};

use crate::bonus::{self, BonusSource, Choice, Outcome};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

use super::Purchase;

/// Fields the oracle does not compare for magic items, on top of the
/// common list. Per-instance state (rating, grade, toggles) is copied from
/// the saved element by `rebuild`, so only fields that cannot be rebuilt
/// are listed here.
pub const IGNORE: &[&str] = &[
    // martial art: 5.18x-5.20x saved a `rating`; current saves do not
    "rating",
];

/// `tag` is the item kind tag (see `items::KINDS`).
pub fn choices(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    let kind = match tag {
        "spell" => "Spell",
        "power" => "Power",
        "complexform" => "ComplexForm",
        "metamagic" => "Metamagic",
        "martialart" => "MartialArt",
        "critterpower" => "CritterPower",
        _ => return Vec::new(),
    };
    let src = BonusSource { kind: kind.into(), guid: String::new(), name: rec.name(), rating: p.rating.max(1) };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Add a record of kind `tag`. Returns the new guid.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    match tag {
        "spell" => Ok(spell::add(ch, store, rec, p.answer.as_deref(), &spell::SpellOptions::default())),
        "power" => Ok(power::add(ch, store, rec, p.rating.max(1), p.answer.as_deref())),
        "complexform" => Ok(complexform::add(ch, store, rec, p.answer.as_deref())),
        "spirit" => Ok(spirit::add(ch, rec, p.rating.max(1), p.qty() as i32, true)),
        "metamagic" => metamagic::add(ch, store, rec, p.answer.as_deref()),
        "martialart" => Ok(martialart::add(ch, store, rec, p.answer.as_deref())),
        "critterpower" => Ok(critterpower::add(ch, store, rec, p.rating, p.answer.as_deref())),
        _ => Err(format!("adding {tag} is not supported")),
    }
}

/// Oracle: rebuild a saved element of kind `tag`.
pub fn rebuild(tag: &str, ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    match tag {
        "spell" => spell::rebuild(store, saved),
        "power" => power::rebuild(store, saved),
        "complexform" => complexform::rebuild(store, saved),
        "spirit" => spirit::rebuild(store, saved),
        "metamagic" => metamagic::rebuild(store, saved),
        "martialart" => martialart::rebuild(store, saved),
        "critterpower" => critterpower::rebuild(store, saved),
        "mentorspirit" => mentor::rebuild(ch, store, saved),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Builds a saved element field by field, in `Save` order.
pub(crate) struct Out(pub Element);

impl Out {
    pub fn new(tag: &str) -> Self {
        Out(Element::new(tag))
    }
    pub fn put(&mut self, k: &str, v: impl Into<String>) {
        self.0.push(Element::with_text(k, v.into()));
    }
    pub fn flag(&mut self, k: &str, v: bool) {
        self.put(k, crate::improvement::bool_str(v));
    }
    pub fn push(&mut self, e: Element) {
        self.0.push(e);
    }
}

/// `TryGetMultiLineStringFieldQuickly("altnotes") ?? ("notes")`.
pub(crate) fn data_notes(e: &Element) -> String {
    e.child_text("altnotes").unwrap_or_else(|| e.get("notes"))
}

/// `TryGetBoolFieldQuickly` with a default.
pub(crate) fn data_bool(e: &Element, k: &str, default: bool) -> bool {
    e.get_bool(k).unwrap_or(default)
}

/// A decimal field as .NET writes it back: the data text, or "0".
pub(crate) fn data_dec(e: &Element, k: &str) -> String {
    let t = e.get(k);
    let t = t.trim();
    if t.parse::<f64>().is_ok() { t.to_owned() } else { "0".to_owned() }
}

/// True when a bonus-like node holds nothing: no child elements and no
/// text. (`XmlNode.IsNullOrInnerTextIsEmpty` also treats nodes such as
/// `<bonus><unarmeddvphysical /></bonus>` as empty, which would lose the
/// bonus on save; the 5.18x-5.20x saves kept them, and so does this port.)
pub(crate) fn inner_text_empty(e: &Element) -> bool {
    e.elements().next().is_none() && e.text().trim().is_empty()
}

/// `"<tag>" + node.InnerXml + "</tag>"`, or an empty `<tag>` when the
/// node has no text (how `Power`, `CritterPower` and `MentorSpirit` save
/// their bonus nodes).
pub(crate) fn inner_copy(node: Option<&Element>, tag: &str) -> Element {
    match node {
        Some(n) if !inner_text_empty(n) => {
            let mut c = n.clone();
            c.name = tag.into();
            c.attrs.clear();
            c
        }
        _ => Element::new(tag),
    }
}

/// `node.OuterXml`, or an empty `<bonus>` (how `Metamagic`, `Art` and
/// `Enhancement` save theirs).
pub(crate) fn outer_copy(node: Option<&Element>) -> Element {
    node.cloned().unwrap_or_else(|| Element::new("bonus"))
}

/// Find the data record a saved item came from: by `sourceid` (or the
/// legacy `id`), then by name.
pub(crate) fn find_saved<'a>(doc: &'a Element, container: &str, item: &'a str, saved: &Element) -> Option<Record<'a>> {
    for k in ["sourceid", "id"] {
        let id = saved.get(k);
        if !id.is_empty() {
            if let Some(r) = doc.child(container)?.children_named(item).find(|e| e.get("id").eq_ignore_ascii_case(&id)) {
                return Some(Record(r));
            }
        }
    }
    crate::data::find(doc, container, item, &saved.get("name"))
}

/// Older saves use other names for some fields. Copy the current field
/// under the legacy name when the saved element has it, so the oracle
/// still compares the value.
pub(crate) fn add_legacy_aliases(rebuilt: &mut Element, saved: &Element, pairs: &[(&str, &str)]) {
    for (legacy, current) in pairs {
        if saved.child(legacy).is_some() && rebuilt.child(legacy).is_none() {
            let v = rebuilt.get(current);
            rebuilt.push(Element::with_text(*legacy, v));
        }
    }
}

/// Where the saved and rebuilt texts of `fields` are the same number
/// spelled differently (".5" in older data, "0.5" now), keep the saved
/// spelling.
pub(crate) fn keep_numeric_spelling(rebuilt: &mut Element, saved: &Element, fields: &[&str]) {
    for f in fields {
        let (Some(a), Some(b)) = (saved.get_f64(f), rebuilt.get_f64(f)) else { continue };
        if a == b {
            rebuilt.set_child_text(f, saved.get(f));
        }
    }
}

/// Apply a bonus node for a new magic item (`ImprovementManager.CreateImprovements`).
pub(crate) fn apply_bonus(ch: &Character, store: &DataStore, node: Option<&Element>, src: &BonusSource, forced: Option<&str>) -> Outcome {
    match node {
        Some(b) if b.elements().next().is_some() => bonus::apply(ch, store, b, src, forced),
        _ => Outcome::default(),
    }
}

/// Store a new item, the objects its bonus created and its improvements.
pub(crate) fn commit(ch: &mut Character, store: &DataStore, container: &str, el: Element, out: &Outcome) {
    ch.items_mut(container).push(el);
    crate::items::place_added(ch, store, &out.added);
    super::apply_outcome(ch, out);
}

/// A bonus source for a new item.
pub(crate) fn source(kind: &str, guid: &str, name: &str, rating: i32) -> BonusSource {
    BonusSource { kind: kind.into(), guid: guid.into(), name: name.into(), rating }
}
