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

/// Change a spirit's force, services, bound or fettered state. A
/// fettering that [`check_fetter`] refuses is left as it was.
pub fn set_state(ch: &mut Character, guid: &str, force: i32, services: i32, bound: bool, fettered: bool) -> bool {
    let Some(s) = super::super::find_by_guid_mut(ch.items_mut("spirits"), guid) else { return false };
    s.set_child_text("force", force.max(1).to_string());
    s.set_child_text("services", services.max(0).to_string());
    s.set_child_text("bound", crate::improvement::bool_str(bound));
    let was = s.get_bool("fettered").unwrap_or(false);
    let s = s.clone();
    if fettered != was && (!fettered || check_fetter(ch, &s).is_ok()) {
        let _ = set_fettered(ch, guid, fettered);
    }
    true
}

/// `ImprovementSource` of the MAG penalty a fettered spirit costs.
pub const FETTERING_SOURCE: &str = "SpiritFettering";

/// Whether the character may fetter this spirit (`Spirit.Fettered`
/// setter): only one spirit or sprite at a time, and sprites only with
/// an `AllowSpriteFettering` improvement (the Sprite Pet complex form).
pub fn check_fetter(ch: &Character, spirit: &Element) -> Result<(), String> {
    if spirit.get("type") == "Sprite" && !ch.improvements.has("AllowSpriteFettering") {
        return Err("sprites can only be fettered with the Sprite Pet complex form".into());
    }
    if ch.items("spirits", "spirit").iter().any(|s| s.get_bool("fettered").unwrap_or(false)) {
        return Err("only one spirit or sprite can be fettered".into());
    }
    Ok(())
}

/// Whether a fettered spirit may be released in career mode: an unbound
/// one that still owes services would become a second unbound spirit
/// with services, which is not allowed.
pub fn check_release(ch: &Character, spirit: &Element) -> Result<(), String> {
    let kind = spirit.get("type");
    let owes = |s: &Element| s.get_i32("services").unwrap_or(0) > 0;
    let flag = |s: &Element, k: &str| s.get_bool(k).unwrap_or(false);
    if ch.created && !flag(spirit, "bound") && owes(spirit) {
        let other = ch.items("spirits", "spirit").into_iter().any(|x| {
            !x.get("guid").eq_ignore_ascii_case(&spirit.get("guid")) && x.get("type") == kind && owes(x) && !flag(x, "bound") && !flag(x, "fettered")
        });
        if other {
            return Err(if kind == "Sprite" { "only one unregistered sprite with tasks is allowed".into() } else { "only one unbound spirit with services is allowed".into() });
        }
    }
    Ok(())
}

/// The power a fettered spirit gains (SG p. 192).
pub const FETTERED_POWER: &str = "Banishing Resistance";

/// Whether the spirit has [`FETTERED_POWER`] from being fettered: a
/// fettered spirit; sprites gain nothing (Sprite Pet, KC p. 91).
pub fn gains_banishing_resistance(spirit: &Element) -> bool {
    spirit.get_bool("fettered").unwrap_or(false) && spirit.get("type") != "Sprite"
}

/// The spirit's powers as (name, `select`): the critter record's
/// `<powers>`, plus [`FETTERED_POWER`] for a fettered spirit.
// chummer-rs deviates from Chummer here (LB-32): Chummer never adds
// Banishing Resistance, but SG p. 192 says a fettered spirit gains it.
// It follows `<fettered>`, so nothing extra is saved.
pub fn powers(record: Option<&Element>, spirit: &Element) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> =
        record.and_then(|r| r.child("powers")).map(|p| p.children_named("power").map(|x| (x.text(), x.attr("select").unwrap_or_default().to_owned())).collect()).unwrap_or_default();
    if gains_banishing_resistance(spirit) && !v.iter().any(|(n, _)| n == FETTERED_POWER) {
        v.push((FETTERED_POWER.into(), String::new()));
    }
    v
}

/// Set `<fettered>` and the MAG −1 augment a fettered spirit (not a
/// sprite) costs, as an `Attribute` improvement from `SpiritFettering`.
/// Releasing removes every `SpiritFettering` improvement. The power it
/// gains follows the flag ([`powers`]).
pub fn set_fettered(ch: &mut Character, guid: &str, fettered: bool) -> Result<(), String> {
    let s = super::super::find_by_guid_mut(ch.items_mut("spirits"), guid).ok_or_else(|| format!("spirit {guid} not found"))?;
    s.set_child_text("fettered", crate::improvement::bool_str(fettered));
    let sprite = s.get("type") == "Sprite";
    if fettered {
        if !sprite {
            ch.improvements.list.push(crate::improvement::Improvement {
                improved_name: "MAG".into(),
                kind: "Attribute".into(),
                source: FETTERING_SOURCE.into(),
                aug: -1.0,
                rating: 1,
                enabled: true,
                ..Default::default()
            });
        }
    } else {
        ch.improvements.list.retain(|i| i.source != FETTERING_SOURCE);
    }
    ch.dirty = true;
    Ok(())
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
