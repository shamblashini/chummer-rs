//! Where an item may go: inside another item, at the top level of its
//! list, or in a location. One rule for buying into a place (the
//! Workspace catalog's fit check) and for moving an owned item there
//! ([`move_item`], `Command::MoveItem`).
//!
//! The rules are the ones the selection dialogs apply when an item is
//! bought into a parent:
//!
//! - the kinds an item takes ([`edit::child_kinds`]); gear only into gear
//!   that is a container ([`edit::is_gear_container`]);
//! - gear: the parent's `addoncategory` list (`SelectGear`);
//! - ware: `requireparent` or a `[n]` capacity and no `mountsto` to go
//!   into ware, no `requireparent` at the top level, the parent's
//!   `allowsubsystems` categories (`SelectCyberware`);
//! - `required/parentdetails` and `forbidden/parentdetails` of the item,
//!   checked against the parent (`ProcessFilterOperationNode`);
//! - accessories: a free mount on the weapon (`SelectWeaponAccessory`);
//!   underbarrel weapons from "Underbarrel Weapons"; weapons in a mount
//!   from its `weaponmountcategories`;
//! - free capacity, with the item's capacity evaluated at its rating,
//!   when the settings enforce capacity (`EnforceCapacity`).
//!
//! Chummer itself only re-parents gear by drag and drop
//! (`MoveGearParent`, `MoveVehicleGearParent`, which check cycles only)
//! and moves top-level armor, weapons and vehicles between locations;
//! chummer-rs moves every kind under the purchase rules
//! (docs/deviations.md).

use serde::{Deserialize, Serialize};

use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::{Element, Node};

use super::edit::{self, find, tag_of};
use super::weapon;

/// Where an item goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dest {
    /// Inside the item with this guid.
    Item(String),
    /// At the top level of its list, in no location.
    Top,
    /// At the top level, in the location with this guid.
    Location(String),
}

/// Why an item cannot go somewhere. [`Misfit::template`] gives an
/// English sentence with `{0}` = the item and `{1}` = the place, for
/// translation; [`Misfit::message`] fills it in.
#[derive(Debug, Clone, PartialEq)]
pub enum Misfit {
    /// The item or the place is not there.
    Missing,
    /// Into itself or something inside it.
    Itself,
    /// Data-included in its parent (`IncludedInParent`).
    Included,
    /// Into another section (gear of a vehicle into an armor…).
    OtherList,
    /// The place does not take this kind of item.
    Kind,
    /// The place takes only these categories.
    Category(Vec<String>),
    /// The item's `parentdetails` rule.
    Required,
    /// Kinds that only exist inside another item.
    NeedsParent,
    /// No free mount on the weapon.
    NoMount,
    /// Not enough free capacity: used, total, needed.
    Full { used: f64, total: f64, need: f64 },
    /// The kind has no locations, or that location is gone.
    NoLocation,
}

impl Misfit {
    /// The sentence and its extra arguments (after the item and place).
    pub fn template(&self) -> (&'static str, Vec<String>) {
        let n = crate::improvement::fmt_num;
        match self {
            Misfit::Missing => ("That item is no longer there.", vec![]),
            Misfit::Itself => ("{0} can't go inside itself.", vec![]),
            Misfit::Included => ("{0} comes with its parent item and can't be moved on its own.", vec![]),
            Misfit::OtherList => ("{0} can only be moved within its own list.", vec![]),
            Misfit::Kind | Misfit::Required => ("{0} can't be installed in {1}.", vec![]),
            Misfit::Category(c) => ("{1} only takes {2}.", vec![c.join(", ")]),
            Misfit::NeedsParent => ("{0} has to be installed in another item.", vec![]),
            Misfit::NoMount => ("{1} has no free mount for {0}.", vec![]),
            Misfit::Full { used, total, need } => ("{1} is full ({2}/{3} capacity used; {0} needs {4}).", vec![n(*used), n(*total), n(*need)]),
            Misfit::NoLocation => ("{0} can't be put in a location.", vec![]),
        }
    }

    /// The English message for item `item` and place `place`.
    pub fn message(&self, item: &str, place: &str) -> String {
        let (t, extra) = self.template();
        let mut s = t.replace("{0}", item).replace("{1}", place);
        for (i, a) in extra.iter().enumerate() {
            s = s.replace(&format!("{{{}}}", i + 2), a);
        }
        s
    }
}

/// What is being placed: an owned item, or a data record being bought.
#[derive(Debug, Clone, Copy)]
pub enum Candidate<'a> {
    Owned(&'a str),
    /// A record of kind `tag` (`items::KINDS`) at `rating`.
    Record { tag: &'a str, rec: &'a Element, rating: i32 },
}

/// The top-level container items of kind `tag` live in.
pub fn top_container(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "gear" => "gears",
        "cyberware" | "bioware" => "cyberwares",
        "armor" => "armors",
        "weapon" => "weapons",
        "vehicle" => "vehicles",
        _ => return None,
    })
}

/// Containers whose items [`root_container`] looks through.
const ROOTS: &[&str] = &["gears", "cyberwares", "armors", "weapons", "vehicles"];

/// The top-level container (of [`ROOTS`]) the item is in, at any depth.
pub fn root_container(ch: &Character, guid: &str) -> Option<&'static str> {
    fn holds(e: &Element, guid: &str) -> bool {
        e.get("guid").eq_ignore_ascii_case(guid) || e.elements().any(|c| holds(c, guid))
    }
    ROOTS.iter().copied().find(|c| ch.doc.child(c).is_some_and(|c| c.elements().any(|e| holds(e, guid))))
}

/// The child list an item of kind `tag` goes into inside `parent`.
fn child_list(parent: &Element, tag: &str) -> Option<&'static str> {
    Some(match (parent.name.as_str(), tag) {
        ("gear", "gear") => "children",
        ("cyberware", "cyberware" | "bioware") => "children",
        ("cyberware" | "armor" | "armormod" | "accessory" | "vehicle", "gear") => "gears",
        ("armor", "armormod") => "armormods",
        ("weapon", "accessory") => "accessories",
        ("weapon", "weapon") => "underbarrel",
        ("weaponmount" | "mod", "weapon") => "weapons",
        ("vehicle", "mod") => "mods",
        ("vehicle", "weaponmount") => "weaponmounts",
        _ => return None,
    })
}

/// A comma list of a saved field, trimmed, without empties.
fn list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect()
}

/// The data element of an owned item (else the saved one).
fn data_of(store: &DataStore, e: &Element) -> Element {
    edit::with_record(store, e, |r| r.el().clone()).unwrap_or_else(|| e.clone())
}

/// `ProcessFilterOperationNode` for `parentdetails`: children are ANDed
/// (or ORed); `OR`/`AND`/`NOR`/`NAND` nest, `NONE` holds when there is no
/// parent, `@NOT` inverts; a field compares with `==` (default),
/// `contains` or `exists` against the parent's field of that name. That
/// covers what the data uses (`name`, `category`, `ammoforweapontype`).
pub fn filter_matches(parent: Option<&Element>, op: &Element, or: bool) -> bool {
    let mut any = false;
    for c in op.elements() {
        let invert = c.attr("NOT").is_some();
        let r = match c.name.to_ascii_uppercase().as_str() {
            "OR" => filter_matches(parent, c, true) != invert,
            "NOR" => filter_matches(parent, c, true) == invert,
            "AND" => filter_matches(parent, c, false) != invert,
            "NAND" => filter_matches(parent, c, false) == invert,
            "NONE" => parent.is_none() != invert,
            _ => match parent {
                None => invert,
                Some(p) => {
                    let want = c.text().trim().to_owned();
                    let targets: Vec<&Element> = p.children_named(&c.name).collect();
                    let hit = match c.attr("operation").unwrap_or("==") {
                        "exists" => !targets.is_empty(),
                        "contains" => targets.iter().any(|t| t.text().contains(&want)),
                        "!=" => targets.iter().any(|t| t.text().trim() != want),
                        _ => targets.iter().any(|t| t.text().trim().eq_ignore_ascii_case(&want)),
                    };
                    hit != invert
                }
            },
        };
        if or && r {
            return true;
        }
        if !or && !r {
            return false;
        }
        any = true;
    }
    !or || !any
}

/// The `required` / `forbidden` `parentdetails` of a data element against
/// a parent (its data element), as `SelectCyberware` / `SelectGear` test
/// them.
fn parent_rules_ok(data: &Element, parent: Option<&Element>) -> bool {
    if let Some(f) = data.child("forbidden").and_then(|f| f.child("parentdetails")) {
        if filter_matches(parent, f, false) {
            return false;
        }
    }
    if let Some(r) = data.child("required").and_then(|r| r.child("parentdetails")) {
        if !filter_matches(parent, r, false) {
            return false;
        }
    }
    true
}

/// What an item uses of its parent's capacity: `[n]` of `capacity`
/// (`armorcapacity` in armor and armor mods) at `rating`; vehicle mods
/// their slots.
pub fn need_in(parent: &Element, item: &Element, tag: &str, rating: i32) -> f64 {
    match (parent.name.as_str(), tag) {
        ("vehicle", "mod" | "weaponmount") => super::armor::rating_value(&item.get("slots"), rating).max(0.0),
        ("armor" | "armormod", _) => edit::parse_capacity(&item.get("armorcapacity"), rating).1,
        _ => {
            if item.get_bool("addtoparentcapacity").unwrap_or(false) || item.child("addtoparentcapacity").is_some_and(|c| c.text().trim().is_empty()) {
                return 0.0;
            }
            edit::parse_capacity(&item.get("capacity"), rating).1
        }
    }
}

/// The settings' "Enforce capacity" (`EnforceCapacity`, on unless the
/// settings say otherwise).
pub fn enforces_capacity(ch: &Character, engine: &crate::engine::Engine) -> bool {
    engine.settings.resolve(&ch.field("settings")).is_none_or(|s| s.flag("enforcecapacity"))
}

/// Whether `cand` can go to `dest`. `enforce`: refuse what does not fit
/// the free capacity ([`enforces_capacity`]).
pub fn check(ch: &Character, store: &DataStore, cand: Candidate<'_>, dest: &Dest, enforce: bool) -> Result<(), Misfit> {
    let (tag, data, item, rating): (String, Element, Option<&Element>, i32) = match cand {
        Candidate::Owned(g) => {
            let e = find(ch, g).ok_or(Misfit::Missing)?;
            if edit::is_included(ch, g) {
                return Err(Misfit::Included);
            }
            (tag_of(e).to_owned(), data_of(store, e), Some(e), e.get_i32("rating").unwrap_or(0))
        }
        Candidate::Record { tag, rec, rating } => (tag.to_owned(), rec.clone(), None, rating),
    };
    let owned = match cand {
        Candidate::Owned(g) => Some(g),
        Candidate::Record { .. } => None,
    };
    let needs_parent = matches!(tag.as_str(), "armormod" | "accessory" | "mod" | "weaponmount") || data.child("requireparent").is_some();
    match dest {
        Dest::Top | Dest::Location(_) => {
            let Some(container) = top_container(&tag) else { return Err(Misfit::NeedsParent) };
            if needs_parent {
                return Err(Misfit::NeedsParent);
            }
            if let Some(g) = owned {
                if root_container(ch, g) != Some(container) {
                    return Err(Misfit::OtherList);
                }
            }
            if let Dest::Location(l) = dest {
                let lc = match tag.as_str() {
                    "gear" => "gearlocations",
                    "armor" => "armorlocations",
                    "weapon" => "weaponlocations",
                    "vehicle" => "vehiclelocations",
                    _ => return Err(Misfit::NoLocation),
                };
                let known = ch.doc.child(lc).is_some_and(|c| c.children_named("location").any(|e| e.get("guid").eq_ignore_ascii_case(l)));
                if !known {
                    return Err(Misfit::NoLocation);
                }
            }
            Ok(())
        }
        Dest::Item(pg) => {
            let host = Host::of(ch, store, pg).ok_or(Misfit::Missing)?;
            let mut here = false;
            if let Some(g) = owned {
                let mut inside = Vec::new();
                edit::subtree_guids(item.expect("owned"), &mut inside);
                if inside.iter().any(|x| x.eq_ignore_ascii_case(pg)) {
                    return Err(Misfit::Itself);
                }
                if root_container(ch, g) != host.root {
                    return Err(Misfit::OtherList);
                }
                here = edit::parent(ch, g).is_some_and(|p| p.get("guid").eq_ignore_ascii_case(pg));
            }
            host.takes(&tag, &data, item, rating, enforce, here)
        }
    }
}

/// A parent item with what the checks need from it, looked up once (the
/// catalog checks every record it lists against its target).
#[derive(Debug, Clone)]
pub struct Host {
    pub el: Element,
    tag: String,
    kinds: Vec<&'static str>,
    cats: Vec<String>,
    data: Element,
    /// (used, total).
    pub capacity: Option<(f64, f64)>,
    root: Option<&'static str>,
}

impl Host {
    pub fn of(ch: &Character, store: &DataStore, guid: &str) -> Option<Host> {
        let el = find(ch, guid)?.clone();
        let tag = tag_of(&el).to_owned();
        let mut kinds: Vec<&'static str> = edit::child_kinds(ch, guid).iter().map(|k| k.tag).collect();
        if tag == "vehicle" {
            kinds.push("weaponmount");
        }
        if tag == "gear" && !edit::is_gear_container(ch, store, guid) {
            kinds.retain(|k| *k != "gear");
        }
        Some(Host { tag, kinds, cats: edit::addon_categories(ch, store, guid), data: data_of(store, &el), capacity: edit::capacity(ch, guid), root: root_container(ch, guid), el })
    }

    /// Whether it takes an item of kind `tag` with data element `data`
    /// (and saved element `item`, when owned) at `rating`. `here`: the
    /// item is in it already (no capacity check).
    pub fn takes(&self, tag: &str, data: &Element, item: Option<&Element>, rating: i32, enforce: bool, here: bool) -> Result<(), Misfit> {
        let ptag = self.tag.as_str();
        // A full weapon mount no longer lists weapons; the one already in
        // it may stay.
        if !self.kinds.contains(&tag) && !(here && ptag == "weaponmount") {
            return Err(Misfit::Kind);
        }
        let category = data.get("category");
        match tag {
            "gear" => {
                if !self.cats.is_empty() && !self.cats.iter().any(|c| c.eq_ignore_ascii_case(&category)) {
                    return Err(Misfit::Category(self.cats.clone()));
                }
            }
            "cyberware" | "bioware" => {
                if !(data.child("requireparent").is_some() || data.get("capacity").contains('[')) || data.child("mountsto").is_some() {
                    return Err(Misfit::Kind);
                }
                let subs = list(&self.el.get("subsystems"));
                if !subs.is_empty() && !subs.iter().any(|c| c.eq_ignore_ascii_case(&category)) {
                    return Err(Misfit::Category(subs));
                }
            }
            "accessory" => {
                if weapon::mount_options(&self.el, Record(data)).is_empty() {
                    return Err(Misfit::NoMount);
                }
            }
            "weapon" if ptag == "weapon" => {
                if !category.eq_ignore_ascii_case("Underbarrel Weapons") {
                    return Err(Misfit::Category(vec!["Underbarrel Weapons".into()]));
                }
            }
            "weapon" => {
                let cats = list(&self.el.get("weaponmountcategories"));
                if !cats.is_empty() && !cats.iter().any(|c| c.eq_ignore_ascii_case(&category)) {
                    return Err(Misfit::Category(cats));
                }
            }
            _ => {}
        }
        if !parent_rules_ok(data, Some(&self.data)) {
            return Err(Misfit::Required);
        }
        if here {
            return Ok(());
        }
        if enforce {
            if let Some((used, total)) = self.capacity {
                let need = need_in(&self.el, item.unwrap_or(data), tag, rating);
                if need > 0.0 && used + need > total + 1e-9 {
                    return Err(Misfit::Full { used, total, need });
                }
            }
        }
        Ok(())
    }
}

/// The item's place now, as a [`Dest`].
pub fn current(ch: &Character, guid: &str) -> Option<Dest> {
    let e = find(ch, guid)?;
    if let Some(p) = edit::parent(ch, guid) {
        return Some(Dest::Item(p.get("guid")));
    }
    let l = e.get("location");
    Some(if l.is_empty() || !edit::has_location(ch, guid) { Dest::Top } else { Dest::Location(l) })
}

/// Take an item out of the document, keeping its improvements. An
/// `<underbarrel>` left empty goes too.
fn take(doc: &mut Element, guid: &str) -> Option<Element> {
    let at = doc.children.iter().position(|n| matches!(n, Node::Element(c) if edit::is_item(c) && c.get("guid").eq_ignore_ascii_case(guid)));
    if let Some(i) = at {
        if let Node::Element(e) = doc.children.remove(i) {
            return Some(e);
        }
    }
    let mut found = None;
    for c in doc.elements_mut() {
        if let Some(e) = take(c, guid) {
            found = Some(e);
            break;
        }
    }
    // The wrapper the item sat in, if it is an underbarrel left empty.
    if found.is_some() {
        doc.children.retain(|n| !matches!(n, Node::Element(u) if u.name == "underbarrel" && u.elements().next().is_none()));
    }
    found
}

/// Whether every item above `guid` is equipped (so its improvements are
/// on when it is).
fn ancestors_on(ch: &Character, guid: &str) -> bool {
    let mut cur = guid.to_owned();
    for _ in 0..32 {
        match edit::parent(ch, &cur) {
            Some(p) => {
                if !edit::is_equipped(p) {
                    return false;
                }
                cur = p.get("guid");
            }
            None => return true,
        }
    }
    true
}

/// Set a ware and the ware inside it to `grade` and, when it picks a
/// side, `side` (`SelectCyberware` forces the parent's grade and side on
/// what is bought into it).
fn follow_parent_ware(store: &DataStore, w: &mut Element, grade: &str, side: &str) {
    if w.name != "cyberware" {
        return;
    }
    let data = data_of(store, w);
    if data.get("forcegrade").is_empty() && !grade.is_empty() {
        w.set_child_text("grade", grade);
    }
    if !side.is_empty() && data.child("selectside").is_some() {
        w.set_child_text("location", side);
    }
    if let Some(kids) = w.child_mut("children") {
        for k in kids.elements_mut() {
            follow_parent_ware(store, k, grade, side);
        }
    }
}

/// Move an owned item to `dest` under the purchase rules ([`check`]).
/// Its guid stays, so the improvements it made stay with it; when the
/// move puts it under an unequipped item (or takes it out of one), its
/// improvements are switched off (or on) as equipping would. Costs,
/// essence and capacity are computed from the tree, so they follow by
/// themselves. Returns false when it is already there.
pub fn move_item(ch: &mut Character, store: &DataStore, guid: &str, dest: &Dest, enforce: bool) -> Result<bool, Misfit> {
    check(ch, store, Candidate::Owned(guid), dest, enforce)?;
    let here = current(ch, guid);
    if here.as_ref() == Some(dest) {
        return Ok(false);
    }
    let was_on = ancestors_on(ch, guid);
    let old_parent = edit::parent(ch, guid).map(|p| p.get("guid"));
    let mut el = take(&mut ch.doc, guid).ok_or(Misfit::Missing)?;
    let tag = tag_of(&el).to_owned();
    // A weapon's `parentid` naming the item it sat in follows it.
    if el.name == "weapon" {
        let pid = el.get("parentid");
        if !pid.is_empty() && old_parent.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&pid)) {
            let new = match dest {
                Dest::Item(p) => p.clone(),
                _ => String::new(),
            };
            el.set_child_text("parentid", new);
        }
    }
    match dest {
        Dest::Item(pg) => {
            let parent = edit::find(ch, pg).cloned().ok_or(Misfit::Missing)?;
            let list = child_list(&parent, &tag).ok_or(Misfit::Kind)?;
            if el.child("location").is_some() && el.name != "cyberware" {
                el.set_child_text("location", "");
            }
            match el.name.as_str() {
                "accessory" => {
                    let data = data_of(store, &el);
                    // Mounts of the new weapon, without this accessory.
                    let mut w = parent.clone();
                    if let Some(a) = w.child_mut("accessories") {
                        a.children.retain(|n| !matches!(n, Node::Element(x) if x.get("guid").eq_ignore_ascii_case(guid)));
                    }
                    let options = weapon::mount_options(&w, Record(&data));
                    let mount = el.get("mount");
                    let mount = if options.contains(&mount) { mount } else { options.into_iter().next().unwrap_or_else(|| "None".into()) };
                    el.set_child_text("mount", &mount);
                    let extra = el.get("extramount");
                    if !extra.is_empty() && extra != "None" {
                        let free: Vec<String> = edit::free_mounts(ch, pg);
                        if !free.contains(&extra) || extra == mount {
                            el.set_child_text("extramount", "None");
                        }
                    }
                }
                "cyberware" if parent.name == "cyberware" => follow_parent_ware(store, &mut el, &parent.get("grade"), &parent.get("location")),
                _ => {}
            }
            let p = super::find_by_guid_mut(&mut ch.doc, pg).ok_or(Misfit::Missing)?;
            p.child_or_insert(list).push(el);
        }
        Dest::Top | Dest::Location(_) => {
            let container = top_container(&tag).ok_or(Misfit::NeedsParent)?;
            let loc = match dest {
                Dest::Location(l) => l.clone(),
                _ => String::new(),
            };
            if matches!(el.name.as_str(), "gear" | "armor" | "weapon" | "vehicle") {
                el.set_child_text("location", loc);
            }
            ch.items_mut(container).push(el);
        }
    }
    ch.dirty = true;
    let now_on = ancestors_on(ch, guid);
    let own_on = find(ch, guid).is_some_and(edit::is_equipped);
    if was_on != now_on && own_on {
        edit::set_tree_enabled(ch, guid, now_on);
        if now_on {
            let mut guids = Vec::new();
            if let Some(e) = find(ch, guid) {
                edit::subtree_guids(e, &mut guids);
            }
            for g in guids {
                edit::refresh_wireless(ch, store, &g);
            }
        }
    }
    Ok(true)
}

/// The display name of the place for messages: the parent's name, the
/// location's name, or "the top level".
pub fn place_name(ch: &Character, dest: &Dest) -> String {
    match dest {
        Dest::Item(g) => find(ch, g).map(|e| e.get("name")).unwrap_or_else(|| "that item".into()),
        Dest::Top => "the top level".into(),
        Dest::Location(l) => ["gearlocations", "armorlocations", "weaponlocations", "vehiclelocations"]
            .iter()
            .filter_map(|c| ch.doc.child(c))
            .flat_map(|c| c.children_named("location"))
            .find(|e| e.get("guid").eq_ignore_ascii_case(l))
            .map(|e| e.get("name"))
            .unwrap_or_else(|| "that location".into()),
    }
}
