//! Adept powers (`Power.Create` / `Power.Save` and the power point maths
//! in `Power.PowerPoints`, `FreeLevels`, `FreePoints`, `TotalMaximumLevels`).

use super::{apply_bonus, commit, data_bool, data_dec, data_notes, find_saved, inner_copy, source, Out};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::xml::Element;

/// Per-instance state of a power: what the player bought and toggled.
#[derive(Debug, Clone, Default)]
pub struct PowerState {
    pub rating: i32,
    pub extra: String,
    /// Adept Way discount applied (`DiscountedAdeptWay`).
    pub discounted: bool,
    pub discounted_geas: bool,
}

/// Build a `<power>` (`Power.Create` + `Power.Save`). `bonus_override`
/// replaces the data bonus (`<bonusoverride>` of `specificpower`).
pub fn element(rec: Record<'_>, guid: &str, st: &PowerState, bonus_override: Option<&Element>) -> Element {
    let e = rec.el();
    let mut p = Out::new("power");
    p.put("sourceid", rec.id());
    p.put("guid", guid);
    p.put("name", rec.name());
    p.put("extra", st.extra.clone());
    // Strings kept as in the data; "0" when absent (the field defaults).
    p.put("pointsperlevel", e.child_text("points").unwrap_or_else(|| "0".into()));
    p.put("adeptway", e.child_text("adeptway").unwrap_or_else(|| "0".into()));
    p.put("action", e.get("action"));
    p.put("rating", st.rating.to_string());
    p.put("extrapointcost", data_dec(e, "extrapointcost"));
    p.flag("levels", data_bool(e, "levels", false));
    p.put("maxlevels", max_levels(e).to_string());
    p.flag("discounted", st.discounted || data_bool(e, "discounted", false));
    p.flag("discountedgeas", st.discounted_geas || data_bool(e, "discountedgeas", false));
    p.put("bonussource", e.get("bonussource"));
    p.put("freepoints", data_dec(e, "freepoints"));
    p.put("source", rec.source());
    p.put("page", rec.page());
    p.push(inner_copy(bonus_override.or(e.child("bonus")), "bonus"));
    // Written whenever the data has the node, even with no text in it.
    p.push(e.child("adeptwayrequires").cloned().unwrap_or_else(|| Element::new("adeptwayrequires")));
    p.push(Element::new("enhancements"));
    p.put("notes", data_notes(e));
    p.0
}

/// `<maxlevel>` or `<maxlevels>`, 0 when absent.
fn max_levels(e: &Element) -> i32 {
    e.get_i32("maxlevel").or_else(|| e.get_i32("maxlevels")).unwrap_or(0)
}

/// Add an adept power at `rating` levels. `extra` answers its bonus
/// selection (e.g. the skill of Improved Ability).
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, rating: i32, extra: Option<&str>) -> String {
    let guid = super::super::new_guid();
    let rating = if data_bool(rec.el(), "levels", false) { rating.max(1) } else { 1 };
    let src = source("Power", &guid, &rec.name(), rating);
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, extra);
    let extra = out.selected.clone().or(extra.map(str::to_owned)).unwrap_or_default();
    let st = PowerState { rating, extra, ..Default::default() };
    commit(ch, "powers", element(rec, &guid, &st, None), &out);
    guid
}

/// Oracle: rebuild a saved `<power>` from powers.xml and its saved state.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("powers.xml").ok()?;
    let rec = find_saved(&doc, "powers", "power", saved)?;
    let st = PowerState {
        rating: saved.get_i32("rating").unwrap_or(1),
        extra: saved.get("extra"),
        discounted: saved.get_bool("discounted").unwrap_or(false),
        discounted_geas: saved.get_bool("discountedgeas").unwrap_or(false),
    };
    let mut e = element(rec, &saved.get("guid"), &st, None);
    super::add_legacy_aliases(&mut e, saved, &[("id", "sourceid"), ("maxlevel", "maxlevels")]);
    // Older data spelled decimals ".5"; the value is what matters.
    super::keep_numeric_spelling(&mut e, saved, &["pointsperlevel", "adeptway", "extrapointcost", "freepoints"]);
    Some(e)
}

// ---------------------------------------------------------------------------
// Power point maths
// ---------------------------------------------------------------------------

fn dec(e: &Element, k: &str) -> f64 {
    e.get_f64(k).unwrap_or(0.0)
}

/// Sum of `Rating` of improvements of `kind` for this power
/// (ImprovedName = power name, UniqueName = power extra).
fn bonus_levels(ch: &Character, kind: &str, p: &Element) -> i32 {
    let (name, extra) = (p.get("name"), p.get("extra"));
    ch.improvements.active().filter(|i| i.kind == kind && i.improved_name == name && i.unique_name == extra).map(|i| i.rating).sum()
}

/// `Power.FreePoints`: AdeptPowerFreePoints ratings × 0.25 (Qi foci).
pub fn free_points(ch: &Character, p: &Element) -> f64 {
    f64::from(bonus_levels(ch, "AdeptPowerFreePoints", p)) * 0.25
}

/// `Power.TotalMaximumLevels`, without the boosted-skill cap.
pub fn total_maximum_levels(p: &Element, mag: i32, ignore_rules: bool) -> i32 {
    if !p.get_bool("levels").unwrap_or(false) {
        return 1;
    }
    let mut max = p.get_i32("maxlevels").or_else(|| p.get_i32("maxlevel")).unwrap_or(0);
    if max <= 0 {
        max = i32::MAX;
    }
    if !ignore_rules {
        max = max.min(mag);
    }
    max
}

/// `Power.FreeLevels`: levels granted by improvements plus levels the
/// free points pay for, capped at MAG.
pub fn free_levels(ch: &Character, p: &Element, mag: i32) -> i32 {
    let mut levels = bonus_levels(ch, "AdeptPowerFreeLevels", p);
    let mut extra_cost = free_points(ch, p);
    let ppl = dec(p, "pointsperlevel");
    let epc = dec(p, "extrapointcost");
    let rating = p.get_i32("rating").unwrap_or(0);
    if rating + levels == 0 && epc > 0.0 {
        extra_cost -= ppl + epc;
        if extra_cost >= 0.0 {
            levels += 1;
        }
        let mut i = extra_cost;
        while i >= 1.0 {
            levels += 1;
            i -= 1.0;
        }
    } else if ppl != 0.0 {
        let mut i = extra_cost;
        while i >= ppl {
            levels += 1;
            i -= ppl;
        }
    }
    levels.min(mag)
}

/// `Power.PowerPoints`: what this power costs out of the adept's pool.
pub fn power_point_cost(ch: &Character, p: &Element, mag: i32) -> f64 {
    let max = total_maximum_levels(p, mag, ch.flag("ignorerules"));
    let rating = p.get_i32("rating").unwrap_or(0).min(max);
    if rating == 0 {
        return 0.0;
    }
    let free = free_levels(ch, p, mag);
    let levels_enabled = p.get_bool("levels").unwrap_or(false);
    if !levels_enabled && free > 0 {
        return 0.0;
    }
    let discount = if p.get_bool("discounted").unwrap_or(false) { dec(p, "adeptway") } else { 0.0 };
    let ppl = dec(p, "pointsperlevel");
    let fp = free_points(ch, p);
    let mut cost = dec(p, "extrapointcost") - discount;
    if f64::from(free) * ppl >= fp {
        cost += f64::from(rating) * ppl;
    } else {
        let total = (rating + free).min(max);
        cost += f64::from(total) * ppl - fp;
    }
    cost.max(0.0)
}

/// `Power.TotalRating`: bought levels plus free levels, capped.
pub fn total_rating(ch: &Character, p: &Element, mag: i32) -> i32 {
    let max = total_maximum_levels(p, mag, ch.flag("ignorerules"));
    (p.get_i32("rating").unwrap_or(0).min(max) + free_levels(ch, p, mag)).min(max)
}
