//! Spirits and sprites (`Spirit.Save`). Everything but the name is player
//! input: force, services owed, bound/fettered, the critter's own name.

use super::Out;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Per-instance state of a spirit or sprite.
#[derive(Debug, Clone, Default)]
pub struct SpiritState {
    pub crittername: String,
    pub services: i32,
    pub force: i32,
    pub bound: bool,
    pub fettered: bool,
    /// `SpiritType`: "Spirit" or "Sprite".
    pub kind: String,
    pub file: String,
    pub relative: String,
}

/// Build a `<spirit>` (`Spirit.Save`).
pub fn element(name: &str, guid: &str, st: &SpiritState) -> Element {
    let mut s = Out::new("spirit");
    s.put("guid", guid);
    s.put("name", name);
    s.put("crittername", st.crittername.clone());
    s.put("services", st.services.to_string());
    s.put("force", st.force.to_string());
    s.flag("bound", st.bound);
    s.flag("fettered", st.fettered);
    s.put("type", if st.kind.is_empty() { "Spirit".to_owned() } else { st.kind.clone() });
    s.put("file", st.file.clone());
    s.put("relative", st.relative.clone());
    s.put("notes", "");
    s.put("mainmugshotindex", "-1");
    s.push(Element::new("mugshots"));
    s.0
}

/// Whether a name is a spirit or a sprite, from where it is defined:
/// spirits in traditions.xml, sprites in streams.xml, other summoned
/// critters (ally spirits, watchers...) in critters.xml.
pub fn find_kind(store: &DataStore, name: &str) -> Option<&'static str> {
    let has = |file: &str, container: &str, item: &str| {
        store.doc(file).ok().is_some_and(|d| crate::data::find(&d, container, item, name).is_some())
    };
    if has("traditions.xml", "spirits", "spirit") {
        Some("Spirit")
    } else if has("streams.xml", "spirits", "spirit") {
        Some("Sprite")
    } else if has("critters.xml", "metatypes", "metatype") {
        Some(if name.contains("Sprite") { "Sprite" } else { "Spirit" })
    } else {
        None
    }
}

/// Add a spirit or sprite of `force` owing `services`. Sprites are
/// "registered" and spirits "bound" when `bound` is set.
pub fn add(ch: &mut Character, rec: Record<'_>, force: i32, services: i32, bound: bool) -> String {
    let guid = super::super::new_guid();
    let kind = if rec.category().contains("Sprite") || rec.name().contains("Sprite") { "Sprite" } else { "Spirit" };
    let st = SpiritState { services: services.max(0), force: force.max(1), bound, kind: kind.into(), ..Default::default() };
    ch.items_mut("spirits").push(element(&rec.name(), &guid, &st));
    guid
}

/// Change a spirit's force, services, bound or fettered state.
pub fn set_state(ch: &mut Character, guid: &str, force: i32, services: i32, bound: bool, fettered: bool) -> bool {
    let Some(s) = super::super::find_by_guid_mut(ch.items_mut("spirits"), guid) else { return false };
    s.set_child_text("force", force.max(1).to_string());
    s.set_child_text("services", services.max(0).to_string());
    s.set_child_text("bound", crate::improvement::bool_str(bound));
    s.set_child_text("fettered", crate::improvement::bool_str(fettered));
    true
}

/// Oracle: rebuild a saved `<spirit>`: the name must exist in the data;
/// the rest is the saved player input.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let name = saved.get("name");
    let kind = find_kind(store, &name)?;
    let st = SpiritState {
        crittername: saved.get("crittername"),
        services: saved.get_i32("services").unwrap_or(0),
        force: saved.get_i32("force").unwrap_or(1),
        bound: saved.get_bool("bound").unwrap_or(false),
        fettered: saved.get_bool("fettered").unwrap_or(false),
        kind: saved.child_text("type").unwrap_or_else(|| kind.into()),
        file: saved.get("file"),
        relative: saved.get("relative"),
    };
    let mut e = element(&name, &saved.get("guid"), &st);
    e.set_child_text("notes", saved.get("notes"));
    Some(e)
}
