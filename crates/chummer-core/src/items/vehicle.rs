//! Vehicle items. Not implemented yet: see the module docs in `items`.

use crate::bonus::Choice;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

use super::Purchase;

pub const IGNORE: &[&str] = &[];

/// `tag` is the item kind tag (see `items::KINDS`).
pub fn choices(_tag: &str, _ch: &Character, _store: &DataStore, _rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    Vec::new()
}

pub fn add(tag: &str, _ch: &mut Character, _store: &DataStore, _rec: Record<'_>, _p: &Purchase) -> Result<String, String> {
    Err(format!("adding {tag} is not implemented yet"))
}

pub fn rebuild(_tag: &str, _ch: &Character, _store: &DataStore, _saved: &Element) -> Option<Element> {
    None
}
