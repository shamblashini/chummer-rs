//! Spending nuyen on items in career mode, and selling them.

use super::ledger::{book_nuyen, ExpenseUndo, NuyenExpenseType};
use super::{require_career, require_nuyen, CareerError};
use crate::character::Character;
use crate::xml::{Element, Node};

/// Containers that hold items bought with nuyen.
pub const ITEM_CONTAINERS: &[&str] = &["cyberwares", "gears", "armors", "weapons", "vehicles", "lifestyles", "drugs"];

/// Pay for an item in career mode (the `Purchased …` entries of
/// `CharacterCareer.cs`). `cost` is positive; `qty` is the quantity bought
/// for gear (0 otherwise). Returns the expense guid.
pub fn spend_nuyen(
    ch: &mut Character,
    cost: f64,
    reason: &str,
    undo_type: NuyenExpenseType,
    object_guid: &str,
    qty: f64,
) -> Result<String, CareerError> {
    require_career(ch)?;
    if cost < 0.0 || !cost.is_finite() {
        return Err(CareerError::Refused("a cost cannot be negative".into()));
    }
    require_nuyen(ch, cost)?;
    let undo = ExpenseUndo::nuyen(undo_type, object_guid, qty);
    Ok(book_nuyen(ch, -cost, reason, Some(undo)))
}

/// The undo type and the "Purchased …"/"Sold …" label of an item kind
/// (the `String_ExpensePurchase*`/`String_ExpenseSold*` strings).
pub fn item_expense_kind(tag: &str, parent_tag: Option<&str>) -> (NuyenExpenseType, &'static str) {
    use NuyenExpenseType as N;
    match (tag, parent_tag) {
        ("gear", Some("armor" | "armormod")) => (N::AddArmorGear, "Armor Gear"),
        ("gear", Some("weapon" | "accessory")) => (N::AddWeaponGear, "Weapon Gear"),
        ("gear", Some("cyberware")) => (N::AddCyberwareGear, "Cyberware Gear"),
        ("gear", Some("vehicle" | "mod" | "weaponmount")) => (N::AddVehicleGear, "Vehicle Gear"),
        ("gear", _) => (N::AddGear, "Gear"),
        ("cyberware", Some("vehicle" | "mod")) => (N::AddVehicleModCyberware, "Vehicle Cyberware"),
        ("cyberware", _) => (N::AddCyberware, "Cyberware"),
        ("bioware", _) => (N::AddCyberware, "Bioware"),
        ("armor", _) => (N::AddArmor, "Armor"),
        ("armormod", _) => (N::AddArmorMod, "Armor Mod"),
        ("weapon", Some("vehicle" | "mod" | "weaponmount")) => (N::AddVehicleWeapon, "Vehicle Weapon"),
        ("weapon", _) => (N::AddWeapon, "Weapon"),
        ("accessory", Some("vehicle" | "mod" | "weaponmount")) => (N::AddVehicleWeaponAccessory, "Vehicle Weapon Accessory"),
        ("accessory", _) => (N::AddWeaponAccessory, "Weapon Accessory"),
        ("vehicle", _) => (N::AddVehicle, "Vehicle"),
        ("mod", _) => (N::AddVehicleMod, "Vehicle Mod"),
        ("weaponmount", _) => (N::AddVehicleWeaponMount, "Vehicle Weapon Mount"),
        ("lifestyle", _) => (N::IncreaseLifestyle, "Lifestyle"),
        ("drug", _) => (N::AddGear, "Drug"),
        _ => (N::AddGear, "Gear"),
    }
}

/// Pay for an item that was just added: `Purchased <Kind> <name>`, with
/// the item's guid as undo id. Returns the expense guid.
pub fn pay_for_item(ch: &mut Character, tag: &str, parent_tag: Option<&str>, guid: &str, cost: f64) -> Result<String, CareerError> {
    let item = find_item_deep(&ch.doc, guid).ok_or_else(|| CareerError::NotFound(format!("item {guid}")))?;
    let (undo, label) = item_expense_kind(tag, parent_tag);
    let qty = if tag == "gear" { item.get_f64("qty").unwrap_or(1.0) } else { 0.0 };
    let reason = format!("Purchased {label} {}", item.get("name"));
    spend_nuyen(ch, cost, &reason, undo, guid, qty)
}

/// Sell an item at a percentage of its cost (`ICanSell.Sell`; `fraction`
/// is `SellItem.SellPercent`, e.g. 0.5). The item and its improvements are
/// removed and the proceeds logged as `Sold <Kind> <name>` without undo.
/// Returns the nuyen received.
pub fn sell_item(ch: &mut Character, guid: &str, fraction: f64) -> Result<f64, CareerError> {
    require_career(ch)?;
    let (item, parent) = take_item(ch, guid).ok_or_else(|| CareerError::NotFound(format!("item {guid}")))?;
    let amount = crate::chargen::item_cost(&item) * fraction;
    let tag = if item.name == "cyberware" && item.get("improvementsource") == "Bioware" { "bioware" } else { item.name.as_str() };
    let (_, label) = item_expense_kind(tag, parent.as_deref());
    book_nuyen(ch, amount, format!("Sold {label} {}", item.get("name")), None);
    Ok(amount)
}

/// Find an item with this guid anywhere under the item containers.
pub(super) fn find_item_deep<'a>(doc: &'a Element, guid: &str) -> Option<&'a Element> {
    fn walk<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
        e.elements().find_map(|c| if c.get("guid").eq_ignore_ascii_case(guid) { Some(c) } else { walk(c, guid) })
    }
    ITEM_CONTAINERS.iter().filter_map(|c| doc.child(c)).find_map(|c| walk(c, guid))
}

pub(super) fn find_item_deep_mut<'a>(doc: &'a mut Element, guid: &str) -> Option<&'a mut Element> {
    for c in doc.elements_mut().filter(|e| ITEM_CONTAINERS.contains(&e.name.as_str())) {
        if let Some(found) = c.elements_mut().find_map(|i| crate::items::find_by_guid_mut(i, guid)) {
            return Some(found);
        }
    }
    None
}

/// Remove an item (at any depth) and the improvements it and its children
/// made. Returns the item and the tag of its parent item, if nested.
pub(super) fn take_item(ch: &mut Character, guid: &str) -> Option<(Element, Option<String>)> {
    fn walk(e: &mut Element, guid: &str, parent: Option<&str>) -> Option<(Element, Option<String>)> {
        let pos = e.children.iter().position(|n| matches!(n, Node::Element(c) if c.get("guid").eq_ignore_ascii_case(guid)));
        if let Some(pos) = pos {
            let Node::Element(item) = e.children.remove(pos) else { return None };
            return Some((item, parent.map(str::to_owned)));
        }
        // An item's own element name is its tag; plain lists (children,
        // gears, mods) pass the enclosing item's tag down.
        let here = if e.child("guid").is_some() { Some(e.name.clone()) } else { parent.map(str::to_owned) };
        e.elements_mut().find_map(|c| walk(c, guid, here.as_deref()))
    }
    let found = ch.doc.elements_mut().filter(|e| ITEM_CONTAINERS.contains(&e.name.as_str())).find_map(|c| walk(c, guid, None))?;
    let mut guids = Vec::new();
    collect_guids(&found.0, &mut guids);
    for g in guids {
        ch.improvements.remove_from_source(&g);
    }
    ch.dirty = true;
    Some(found)
}

fn collect_guids(e: &Element, out: &mut Vec<String>) {
    let g = e.get("guid");
    if !g.is_empty() {
        out.push(g);
    }
    for c in e.elements() {
        collect_guids(c, out);
    }
}
