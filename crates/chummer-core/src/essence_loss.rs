//! Essence loss: the MAG/MAGAdept/RES/DEP reductions that follow lost
//! essence (`Character.RefreshEssenceLossImprovements`).
//!
//! Ported: the RAW creation-mode branch, the RAW career-mode branch, the
//! `SpecialKarmaCostBasedOnShownValue` house rule (both modes), and the
//! Cyberzombie attribute adjustment. In career mode under RAW (SR5 p. 95:
//! every point or fraction of Essence lost lowers Magic/Resonance and its
//! maximum by 1) the reduction beyond the one from creation becomes
//! `EssenceLoss` improvements; once a minimum cannot drop further, karma
//! levels (and a mystic adept's power points) are burnt, against the
//! improvements from the previous call, so a repeated call burns nothing.
//!
//! Call [`refresh`] after anything that changes essence (adding or
//! removing ware, changing its grade, rating or essence discount,
//! qualities with `EssencePenalty*`), and after MAG/RES/DEP become
//! enabled or disabled. The trigger is `<essenceatspecialstart>`:
//! whoever enables a special attribute must set it to the current essence,
//! and reset it to the sentinel when none is enabled.

use crate::attributes::Attribute;
use crate::calc::{self, Rules};
use crate::character::Character;
use crate::data::DataStore;
use crate::expr::{round_away, standard_round};
use crate::improvement::Improvement;

/// `ImprovementSource` values written by this module.
pub const CHARGEN: &str = "EssenceLossChargen";
pub const CAREER: &str = "EssenceLoss";

/// `EssenceAtSpecialStart`, or `None` when it holds `decimal.MinValue`
/// (no special attribute was ever enabled) or is missing.
pub fn essence_at_special_start(ch: &Character) -> Option<f64> {
    ch.doc.get_f64("essenceatspecialstart").filter(|v| *v > -1e20)
}

/// `StandardRound` of a difference of rounded essences, ignoring binary
/// noise below 1e-9 (6 - 4.9999999999 must not round up to 2).
fn round_reduction(x: f64) -> i32 {
    standard_round((x * 1e9).round() / 1e9)
}

/// The special-attribute burn multiplier: `SpecialAttBurn` (additive) times
/// `SpecialAttTotalBurnMultiplier` (multiplicative).
fn burn_multiplier(ch: &Character) -> f64 {
    let imps = &ch.improvements;
    let add = imps.of_kind("SpecialAttBurn").fold(1.0, |m, i| m - (1.0 - i.val / 100.0));
    let mult = imps.of_kind("SpecialAttTotalBurnMultiplier").fold(1.0, |m, i| m * i.val / 100.0);
    add * mult
}

/// `Character.GetAllAttributeSpecificEssence`, rounded unless the settings
/// say not to: essence for (MAG, RES, DEP).
fn attribute_essences(ch: &Character, essence: f64, rules: &Rules) -> [f64; 3] {
    let imps = &ch.improvements;
    let fixed = imps.has("CyborgEssence");
    let one = |kind: &str| {
        let e = if fixed { 0.1 } else { essence + imps.val(kind, None) / 100.0 };
        if rules.dont_round_essence {
            e
        } else {
            round_away(e, rules.essence_decimals)
        }
    };
    [one("EssencePenaltyMAGOnlyT100"), one("EssencePenaltyRESOnlyT100"), one("EssencePenaltyDEPOnlyT100")]
}

/// An `Attribute` improvement as `ImprovementManager.CreateImprovement`
/// makes it, with no source object.
fn attribute_improvement(name: &str, source: &str, min: i32, max: i32, aug: f64) -> Improvement {
    Improvement {
        improved_name: name.into(),
        source: source.into(),
        kind: "Attribute".into(),
        rating: 1,
        min: f64::from(min),
        max: f64::from(max),
        aug,
        enabled: true,
        ..Default::default()
    }
}

/// `CharacterAttrib.MinimumMaximumNoEssenceLoss`: metatype limits plus
/// every non-essence-loss modifier of the attribute, plus the essence-loss
/// modifiers where they raise it. With `use_start` the essence-loss part
/// is at least `EssenceAtSpecialStart` − metatype maximum ESS.
pub fn minimum_maximum_no_essence_loss(ch: &Character, abbrev: &str, use_start: Option<i32>) -> (i32, i32) {
    if ch.doc.get("metatypecategory") == "Cyberzombie" && abbrev.starts_with("MAG") {
        return (1, 1);
    }
    let a = ch.attribute(abbrev).cloned().unwrap_or(Attribute { metatype_min: 0, metatype_max: 0, ..Default::default() });
    let base = format!("{abbrev}Base");
    let (mut min, mut max) = (a.metatype_min, a.metatype_max);
    let (mut min_loss, mut max_loss) = (0, 0);
    for i in ch.improvements.of_kind("Attribute") {
        if i.improved_name == abbrev || i.improved_name == base {
            if matches!(i.source.as_str(), CHARGEN | CAREER | "CyberadeptDaemon") {
                min_loss += i.min as i32 * i.rating;
                max_loss += i.max as i32 * i.rating;
            } else {
                min += i.min as i32 * i.rating;
                max += i.max as i32 * i.rating;
            }
        }
    }
    let floor = use_start.unwrap_or(0);
    min += min_loss.max(floor);
    max += max_loss.max(floor);
    if min < 1 {
        let zero = ch.flag("iscritter") || a.metatype_max == 0 || matches!(abbrev, "EDG" | "MAG" | "MAGAdept" | "RES" | "DEP");
        min = if zero { 0 } else { 1 };
    }
    (min, max.max(min))
}

fn remove_sources(ch: &mut Character, sources: &[&str]) {
    ch.improvements.list.retain(|i| !sources.contains(&i.source.as_str()));
}

/// Regenerate the essence-loss improvements (`RefreshEssenceLossImprovements`).
pub fn refresh(ch: &mut Character, store: &DataStore, rules: &Rules) {
    let before = ch.improvements.list.clone();
    let sheet = calc::compute(ch, rules, Some(store), None);
    match essence_at_special_start(ch) {
        None => remove_sources(ch, &[CHARGEN, CAREER]),
        Some(start) => {
            let ess_max = f64::from(sheet.attr_values("ESS").map_or(6, |a| a.metatype_max));
            let ess = attribute_essences(ch, sheet.essence, rules);
            let burn = burn_multiplier(ch);
            let max_red = ess.map(|e| round_reduction((ess_max - e) * burn));
            let min_red = ess.map(|e| round_reduction((start - e) * burn));
            if rules.special_karma_cost_based_on_shown_value {
                shown_value(ch, max_red, &sheet);
            } else if ch.created {
                let start_floor = standard_round(start) - ess_max as i32;
                raw_career(ch, store, rules, max_red, min_red, start_floor);
            } else {
                raw_create(ch, rules, max_red, min_red);
            }
        }
    }
    cyberzombie(ch, sheet.essence);
    // Regenerating the same improvements keeps the saved order and does not
    // mark the character modified.
    if same_items(&before, &ch.improvements.list) {
        ch.improvements.list = before;
    } else {
        ch.dirty = true;
    }
}

/// Equal as multisets.
fn same_items(a: &[Improvement], b: &[Improvement]) -> bool {
    let count = |l: &[Improvement], x: &Improvement| l.iter().filter(|y| *y == x).count();
    a.len() == b.len() && a.iter().all(|x| count(a, x) == count(b, x))
}

/// RAW creation mode: maxima drop by (metatype max ESS − ESS), minima by
/// (ESS at special start − ESS). Indexes of `max_red`/`min_red`: MAG, RES, DEP.
fn raw_create(ch: &mut Character, rules: &Rules, max_red: [i32; 3], min_red: [i32; 3]) {
    let [mag_max, res_max, dep_max] = max_red;
    let [mag_min, res_min, dep_min] = min_red;
    let min_for = |abbrev: &str, red: i32| {
        if rules.ess_loss_reduces_maximum_only {
            let (lo, hi) = minimum_maximum_no_essence_loss(ch, abbrev, None);
            (red + lo - hi).max(0)
        } else {
            red
        }
    };
    let (mag, mag_adept, res, dep) = (min_for("MAG", mag_min), min_for("MAGAdept", mag_min), min_for("RES", res_min), min_for("DEP", dep_min));
    remove_sources(ch, &[CHARGEN, CAREER]);
    let mut add = Vec::new();
    if mag_max != 0 || mag != 0 || mag_adept != 0 {
        add.push(attribute_improvement("MAG", CHARGEN, -mag, -mag_max, 0.0));
        add.push(attribute_improvement("MAGAdept", CHARGEN, -mag_adept, -mag_max, 0.0));
    }
    if res_max != 0 || res != 0 {
        add.push(attribute_improvement("RES", CHARGEN, -res, -res_max, 0.0));
    }
    if dep_max != 0 || dep != 0 {
        add.push(attribute_improvement("DEP", CHARGEN, -dep, -dep_max, 0.0));
    }
    ch.improvements.list.extend(add);
}

/// One attribute's state for [`raw_career`], after the old `EssenceLoss`
/// improvements are removed.
struct CareerAttr {
    total_max: i32,
    total: i32,
    /// `Base + FreeBase + RawMinimum + AttributeValueModifiers`.
    floor: i32,
    karma: i32,
    /// `MaximumNoEssenceLoss()` and `MaximumNoEssenceLoss(true)`.
    max_no_loss: i32,
    max_no_loss_start: i32,
}

/// The legacy-shim burn: karma levels above the total maximum, measured
/// before the old `EssenceLoss` improvements are removed.
fn extra_burn(ch: &Character, store: &DataStore, rules: &Rules, abbrev: &str) -> i32 {
    let v = calc::attribute_values_with(ch, abbrev, rules, Some(store));
    ((v.base + v.free_base + v.raw_min + v.value_mods).max(v.total_min) + v.karma - v.total_max).max(0)
}

impl CareerAttr {
    fn new(ch: &Character, store: &DataStore, rules: &Rules, abbrev: &str, start_floor: i32) -> Self {
        let v = calc::attribute_values_with(ch, abbrev, rules, Some(store));
        let floor = v.base + v.free_base + v.raw_min + v.value_mods;
        CareerAttr {
            total_max: v.total_max,
            total: v.total,
            floor,
            karma: v.karma,
            max_no_loss: minimum_maximum_no_essence_loss(ch, abbrev, None).1,
            max_no_loss_start: minimum_maximum_no_essence_loss(ch, abbrev, Some(start_floor)).1,
        }
    }

    /// The career minimum reduction before comparing with the old one.
    fn min_reduction(&self, base: i32, max_only: bool) -> i32 {
        if max_only {
            (base + self.total - self.max_no_loss_start).max(0)
        } else {
            base + self.total_max - self.max_no_loss_start
        }
    }
}

/// RAW career mode (the `else if (Created)` branch of
/// `RefreshEssenceLossImprovements`). Indexes of `max_red`/`min_red`:
/// MAG, RES, DEP.
fn raw_career(ch: &mut Character, store: &DataStore, rules: &Rules, max_red: [i32; 3], min_red: [i32; 3], start_floor: i32) {
    let [mag_max, res_max, dep_max] = max_red;
    let [mag_min, res_min, dep_min] = min_red;
    // The old reductions: negative modifier = positive reduction; the
    // augmented part counts for a switch from the shown-value house rule.
    let old = |name: &str| -> i32 {
        ch.improvements.of_kind("Attribute").filter(|i| i.source == CAREER && i.improved_name.eq_ignore_ascii_case(name)).map(|i| -(i.min as i32 + standard_round(i.aug))).sum()
    };
    let (old_mag, old_mag_adept, old_res, old_dep) = (old("MAG"), old("MAGAdept"), old("RES"), old("DEP"));
    let mystic = ch.is_adept() && ch.is_magician();
    // `MAGAdept` is MAG itself unless the second-MAG house rule applies.
    let separate_adept = rules.mys_adept_second_mag_attribute && mystic;
    let extra = |name: &str| extra_burn(ch, store, rules, name);
    let (extra_mag, extra_mag_adept, extra_res, extra_dep) = (extra("MAG"), extra(if separate_adept { "MAGAdept" } else { "MAG" }), extra("RES"), extra("DEP"));
    remove_sources(ch, &[CAREER]);
    let use_mystic_pps = mystic && !rules.mys_adept_second_mag_attribute;
    let max_only = rules.ess_loss_reduces_maximum_only;
    let mag = CareerAttr::new(ch, store, rules, "MAG", start_floor);
    let mag_adept = if separate_adept { CareerAttr::new(ch, store, rules, "MAGAdept", start_floor) } else { CareerAttr::new(ch, store, rules, "MAG", start_floor) };
    let res = CareerAttr::new(ch, store, rules, "RES", start_floor);
    let dep = CareerAttr::new(ch, store, rules, "DEP", start_floor);
    let mut add = Vec::new();

    let mag_max_red = mag_max + mag.total_max - mag.max_no_loss;
    let mag_adept_max_red = mag_max + mag_adept.total_max - mag_adept.max_no_loss;
    if mag_max > 0 || mag_min > 0 || mag_max_red != 0 || mag_adept_max_red != 0 {
        let mut mag_red = mag.min_reduction(mag_min, max_only);
        let mut mag_adept_red = mag_adept.min_reduction(mag_min, max_only);
        let delta = mag_red - old_mag;
        if delta > 0 {
            // A minimum that cannot drop further burns karma levels. The
            // reduction itself stays, so a repeated call burns nothing.
            if mag_red > mag.floor {
                burn_karma(ch, "MAG", extra_mag + delta.min(mag.karma));
            }
            if use_mystic_pps {
                // Power points bought in creation first, then those from
                // initiation (as an `EssenceLossChargen` improvement, so
                // the next career refresh keeps it).
                let pp = ch.doc.get_i32("magsplitadept").unwrap_or(0);
                let chargen_burn = pp.min(delta);
                let mag_total = calc::attribute_values_with(ch, "MAG", rules, Some(store)).total;
                ch.doc.set_child_text("magsplitadept", (pp - chargen_burn).min(mag_total).to_string());
                let pp_burn = f64::from(delta - chargen_burn).min(ch.improvements.val("AdeptPowerPoints", None));
                if pp_burn != 0.0 {
                    add.push(Improvement { source: CHARGEN.into(), kind: "AdeptPowerPoints".into(), rating: 1, val: -pp_burn, enabled: true, ..Default::default() });
                }
            }
        } else {
            mag_red = old_mag;
        }
        if separate_adept {
            let delta = mag_adept_red - old_mag_adept;
            if delta > 0 {
                if mag_adept_red > mag_adept.floor {
                    burn_karma(ch, "MAGAdept", extra_mag_adept + delta.min(mag_adept.karma));
                }
            } else {
                mag_adept_red = old_mag_adept;
            }
        } else if mag_adept_red < old_mag_adept {
            mag_adept_red = old_mag_adept;
        }
        if mag_red != 0 || mag_max_red != 0 {
            add.push(attribute_improvement("MAG", CAREER, -mag_red, -mag_max_red, 0.0));
        }
        if mag_adept_red != 0 || mag_adept_max_red != 0 {
            add.push(attribute_improvement("MAGAdept", CAREER, -mag_adept_red, -mag_adept_max_red, 0.0));
        }
    }
    for (name, a, max, min, old, extra) in [("RES", &res, res_max, res_min, old_res, extra_res), ("DEP", &dep, dep_max, dep_min, old_dep, extra_dep)] {
        let max_red = max + a.total_max - a.max_no_loss;
        if max > 0 || min > 0 || max_red != 0 {
            let mut red = a.min_reduction(min, max_only);
            if red - old > 0 {
                if red > a.floor {
                    burn_karma(ch, name, extra + (red - old).min(a.karma));
                }
            } else {
                red = old;
            }
            if red != 0 || max_red != 0 {
                add.push(attribute_improvement(name, CAREER, -red, -max_red, 0.0));
            }
        }
    }
    ch.improvements.list.extend(add);
}

/// Lower an attribute's karma levels by `n` (Chummer does not stop at 0;
/// chummer-rs does).
fn burn_karma(ch: &mut Character, abbrev: &str, n: i32) {
    if let Some(a) = ch.attribute_mut(abbrev) {
        a.karma = (a.karma - n).max(0);
        ch.dirty = true;
    }
}

/// `SpecialKarmaCostBasedOnShownValue`: the reduction is an augmented
/// malus. Cyberadept Daemon offsets RES loss by submersion grade.
fn shown_value(ch: &mut Character, max_red: [i32; 3], sheet: &calc::Sheet) {
    let source = if ch.created { CAREER } else { CHARGEN };
    let [mag, res, dep] = max_red;
    let res = match cyberadept_daemon_bonus(ch, sheet) {
        Some(bonus) if res != 0 => (res - bonus).max(0),
        _ => res,
    };
    remove_sources(ch, &[CHARGEN, CAREER, "CyberadeptDaemon"]);
    let mut add = Vec::new();
    if mag != 0 {
        add.push(attribute_improvement("MAG", source, 0, 0, f64::from(-mag)));
        add.push(attribute_improvement("MAGAdept", source, 0, 0, f64::from(-mag)));
        // Mystic adepts using the single-MAG power point rules lose PPs too
        // (assumes the default `MysAdeptSecondMAGAttribute` = false).
        if ch.is_adept() && ch.is_magician() {
            add.push(Improvement { source: source.into(), kind: "AdeptPowerPoints".into(), rating: 1, val: f64::from(-mag), enabled: true, ..Default::default() });
        }
    }
    if res != 0 {
        add.push(attribute_improvement("RES", source, 0, 0, f64::from(-res)));
    }
    if dep != 0 {
        add.push(attribute_improvement("DEP", source, 0, 0, f64::from(-dep)));
    }
    ch.improvements.list.extend(add);
}

/// RES restored by Cyberadept Daemon: Σ ⌈i/2⌉ over submersion grades,
/// capped by whole points of cyberware essence. `None` when it does not
/// apply (not a submerged technomancer with the daemon).
fn cyberadept_daemon_bonus(ch: &Character, sheet: &calc::Sheet) -> Option<i32> {
    if !ch.is_technomancer() || !ch.improvements.has("CyberadeptDaemon") {
        return None;
    }
    let grades = ch.doc.child("initiationgrades").map_or(0, |g| g.children_named("initiationgrade").filter(|e| e.get_bool("technomancer").unwrap_or(false)).count()) as i32;
    if grades == 0 {
        return None;
    }
    let hole: f64 = ch.items("cyberwares", "cyberware").iter().filter(|w| w.get("name") == "Essence Hole").map(|w| f64::from(w.get_i32("rating").unwrap_or(0)) / 100.0).sum();
    let non_cyber = sheet.bioware_essence + hole;
    let cap = if non_cyber.ceil() == non_cyber.floor() { sheet.cyberware_essence.ceil() } else { sheet.cyberware_essence.floor() } as i32;
    let bonus: i32 = (1..=grades).map(|i| calc::div_away_from_zero(i, 2)).sum();
    Some(bonus.min(cap))
}

/// Cyberzombies' attributes follow their (negative) essence.
fn cyberzombie(ch: &mut Character, essence: f64) {
    if ch.doc.get("metatypecategory") != "Cyberzombie" {
        return;
    }
    ch.improvements.list.retain(|i| !(i.source == "Cyberzombie" && i.kind == "Attribute"));
    let m = standard_round(-essence);
    if m != 0 {
        for a in ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL"] {
            ch.improvements.list.push(attribute_improvement(a, "Cyberzombie", 0, m, 0.0));
        }
    }
}
