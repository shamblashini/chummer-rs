//! Gear items. Not implemented yet: see the module docs in `items`.

use crate::bonus::Choice;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

use super::Purchase;

pub const IGNORE: &[&str] = &[];

pub fn choices(_ch: &Character, _store: &DataStore, _rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    Vec::new()
}

pub fn add(_ch: &mut Character, _store: &DataStore, _rec: Record<'_>, _p: &Purchase) -> Result<String, String> {
    Err("adding gear is not implemented yet".into())
}

pub fn rebuild(_ch: &Character, _store: &DataStore, _saved: &Element) -> Option<Element> {
    None
}

/// Bonus handler hook (see `bonus/handlers.rs`). Return false while unsupported.
pub fn bonus_addgear(_ctx: &mut crate::bonus::Ctx<'_>, _node: &Element) -> bool {
    false
}
