//! In-play tracking at the table: weapon ammunition and the condition
//! monitors of devices and vehicles (Edge is in `career::actions`).
//! Everything is stored in the `.chum5` under Chummer5a's own
//! element names.

pub mod ai;
pub mod ammo;
pub mod matrix;
pub mod vehicle;

use crate::character::Character;
use crate::xml::Element;

/// Set a saved item's integer field (a condition monitor), clamped to
/// `0..=max`. Returns whether it changed.
pub(crate) fn set_filled(ch: &mut Character, guid: &str, field: &str, value: i32, max: i32) -> bool {
    let v = value.clamp(0, max.max(0));
    let Some(e) = find_mut(&mut ch.doc, guid) else { return false };
    if e.get_i32(field) == Some(v) {
        return false;
    }
    e.set_child_text(field, v.to_string());
    ch.dirty = true;
    true
}

/// `.chum5` element with this guid anywhere in `e` (items only, not
/// improvements).
pub(crate) fn find<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if guid.is_empty() {
        return None;
    }
    if e.get("guid").eq_ignore_ascii_case(guid) && e.name != "improvement" {
        return Some(e);
    }
    e.elements().filter(|c| c.name != "improvements").find_map(|c| find(c, guid))
}

pub(crate) fn find_mut<'a>(e: &'a mut Element, guid: &str) -> Option<&'a mut Element> {
    if guid.is_empty() {
        return None;
    }
    if e.get("guid").eq_ignore_ascii_case(guid) && e.name != "improvement" {
        return Some(e);
    }
    e.elements_mut().filter(|c| c.name != "improvements").find_map(|c| find_mut(c, guid))
}
