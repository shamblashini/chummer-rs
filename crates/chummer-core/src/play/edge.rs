//! Edge points spent this session: `Character.EdgeUsed`, saved as
//! `<edgeused>`, with `cmdEdgeSpent_Click` / `cmdEdgeGained_Click`.

use crate::calc::Sheet;
use crate::character::Character;

/// `Character.EdgeUsed`.
pub fn used(ch: &Character) -> i32 {
    ch.doc.get_i32("edgeused").unwrap_or(0).max(0)
}

/// `EDG.TotalValue`.
pub fn total(sheet: &Sheet) -> i32 {
    sheet.attr("EDG").max(0)
}

/// `Character.EdgeRemaining`. Chummer lowers `EdgeUsed` when EDG drops
/// below it; here the remainder is just never negative.
pub fn remaining(ch: &Character, sheet: &Sheet) -> i32 {
    (total(sheet) - used(ch)).max(0)
}

fn set_used(ch: &mut Character, v: i32) {
    ch.doc.set_child_text("edgeused", v.max(0).to_string());
    ch.dirty = true;
}

/// Why an Edge button did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeError {
    /// `Message_CannotSpendEdge`.
    NoneLeft,
    /// `Message_CannotRegainEdge`.
    AtMaximum,
}

impl EdgeError {
    /// The en-us text of Chummer's message.
    pub fn message(self) -> &'static str {
        match self {
            EdgeError::NoneLeft => "You do not have any Edge left to spend.",
            EdgeError::AtMaximum => "You cannot exceed your Edge Attribute.",
        }
    }
}

/// `cmdEdgeSpent_Click`: spend one point.
pub fn spend(ch: &mut Character, sheet: &Sheet) -> Result<(), EdgeError> {
    let u = used(ch);
    if u >= total(sheet) {
        return Err(EdgeError::NoneLeft);
    }
    set_used(ch, u + 1);
    Ok(())
}

/// `cmdEdgeGained_Click`: regain one point.
pub fn regain(ch: &mut Character, sheet: &Sheet) -> Result<(), EdgeError> {
    let u = used(ch).min(total(sheet));
    if u <= 0 {
        return Err(EdgeError::AtMaximum);
    }
    set_used(ch, u - 1);
    Ok(())
}

/// Refresh Edge to full (start of a session). Returns whether anything
/// was spent.
pub fn refresh(ch: &mut Character) -> bool {
    if used(ch) == 0 {
        return false;
    }
    set_used(ch, 0);
    true
}

/// Set the number of points spent, e.g. from clicking the Edge boxes.
pub fn set_spent(ch: &mut Character, sheet: &Sheet, spent: i32) -> bool {
    let v = spent.clamp(0, total(sheet));
    if v == used(ch) {
        return false;
    }
    set_used(ch, v);
    true
}
