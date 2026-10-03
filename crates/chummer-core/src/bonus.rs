//! The bonus processor: turns a data `<bonus>` node into improvements.
//!
//! Port of `ImprovementManager.CreateImprovements` and the handlers in
//! `AddImprovementCollection.cs`. A bonus may ask the user to pick
//! something (a skill, an attribute, free text). As in Chummer5a, one
//! selected value is shared by the whole bonus; it ends up in the item's
//! `<extra>`. Callers first ask [`choices`], then pass the answer to
//! [`apply`].

use crate::character::Character;
use crate::data::DataStore;
use crate::improvement::Improvement;
use crate::xml::Element;

/// The object granting the bonus.
#[derive(Debug, Clone)]
pub struct BonusSource {
    /// `ImprovementSource`, e.g. `"Quality"`, `"Cyberware"`.
    pub kind: String,
    /// GUID of the item; becomes every improvement's `sourcename`.
    pub guid: String,
    pub name: String,
    pub rating: i32,
}

/// A selection the user must make before the bonus can be applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The bonus node that asks, e.g. `"selectskill"`.
    pub node: String,
    pub prompt: String,
    /// Allowed answers. Empty means free text.
    pub options: Vec<String>,
}

/// What applying a bonus produced.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub improvements: Vec<Improvement>,
    /// The selected value, to store in the item's `<extra>`.
    pub selected: Option<String>,
    /// Character fields to set, e.g. `("magenabled", "True")`.
    pub flags: Vec<(String, String)>,
    /// Bonus node types this port cannot process yet.
    pub unsupported: Vec<String>,
}

/// Everything a handler needs.
pub struct Ctx<'a> {
    pub ch: &'a Character,
    pub store: &'a DataStore,
    pub src: &'a BonusSource,
    /// The shared selected value (`ForcedValue` / `_strSelectedValue`).
    pub selected: Option<String>,
    pub out: Outcome,
}

impl Ctx<'_> {
    /// A new improvement from this source with Chummer's defaults.
    pub fn imp(&self, kind: &str, improved_name: &str) -> Improvement {
        Improvement {
            kind: kind.into(),
            improved_name: improved_name.into(),
            source_name: self.src.guid.clone(),
            source: self.src.kind.clone(),
            rating: 1,
            enabled: true,
            ..Default::default()
        }
    }

    pub fn push(&mut self, i: Improvement) {
        self.out.improvements.push(i);
    }
}

/// Selections needed to apply `bonus`.
pub fn choices(ch: &Character, store: &DataStore, bonus: &Element, src: &BonusSource) -> Vec<Choice> {
    let mut v = Vec::new();
    for node in bonus.elements() {
        if let Some(c) = handlers::choice_for(ch, store, node, src) {
            if !v.iter().any(|x: &Choice| x.node == c.node) {
                v.push(c);
            }
        }
    }
    v
}

/// Apply a bonus. `forced` answers any selection (the item's `<extra>`).
pub fn apply(ch: &Character, store: &DataStore, bonus: &Element, src: &BonusSource, forced: Option<&str>) -> Outcome {
    let mut ctx = Ctx { ch, store, src, selected: forced.map(str::to_owned), out: Outcome::default() };
    for node in bonus.elements() {
        if !handlers::apply_node(&mut ctx, node) {
            ctx.out.unsupported.push(node.name.clone());
        }
    }
    ctx.out.selected = ctx.selected.clone();
    ctx.out
}

mod handlers {
    use super::{Choice, Ctx};
    use crate::character::Character;
    use crate::data::DataStore;
    use crate::xml::Element;

    pub fn choice_for(_ch: &Character, _store: &DataStore, _node: &Element, _src: &super::BonusSource) -> Option<Choice> {
        None
    }

    /// Returns false when the node type is not handled.
    pub fn apply_node(_ctx: &mut Ctx<'_>, _node: &Element) -> bool {
        false
    }
}
