//! Custom spells: Chummer's `CreateSpell` form (Street Grimoire p. 159).
//!
//! A design holds the form's controls. [`set_modifier`] applies the
//! form's checkbox rules (`chkModifier_CheckedChanged`), [`drain`] is
//! `CalculateDrain`, [`problems`] and [`element`] are `AcceptForm`.

use crate::career::{self, CareerError};
use crate::character::Character;
use crate::engine::Engine;
use crate::improvement::bool_str;
use crate::items::new_guid;
use crate::xml::Element;

pub const MODIFIER_SLOTS: usize = 14;

/// The form's state.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellDesign {
    pub name: String,
    /// A `spells.xml` category, e.g. "Combat".
    pub category: String,
    /// "P" (physical) or "M" (mana).
    pub kind: String,
    /// "T" (touch) or "LOS".
    pub range: String,
    pub area: bool,
    /// "I", "P" or "S".
    pub duration: String,
    pub restricted: bool,
    pub very_restricted: bool,
    /// What the restriction is; becomes the spell's `<extra>`.
    pub restriction: String,
    pub limited: bool,
    /// `chkModifier1` .. `chkModifier14`.
    pub mods: [bool; MODIFIER_SLOTS],
    /// Disabled checkboxes (`Enabled = false`), by the same index.
    pub disabled: [bool; MODIFIER_SLOTS],
    /// `nudNumberOfEffects`, for Combat element effects and Manipulation
    /// elemental effects.
    pub effects: i32,
    /// `cboType.Enabled`: false when Indirect forces a physical spell.
    pub kind_locked: bool,
}

impl Default for SpellDesign {
    fn default() -> Self {
        SpellDesign {
            name: String::new(),
            category: "Combat".into(),
            kind: "P".into(),
            range: "T".into(),
            area: false,
            duration: "I".into(),
            restricted: false,
            very_restricted: false,
            restriction: String::new(),
            limited: false,
            mods: [false; MODIFIER_SLOTS],
            disabled: [false; MODIFIER_SLOTS],
            effects: 1,
            kind_locked: false,
        }
    }
}

/// Spell types, ranges and durations as (value, English label).
pub const TYPES: &[(&str, &str)] = &[("P", "Physical"), ("M", "Mana")];
pub const RANGES: &[(&str, &str)] = &[("T", "Touch"), ("LOS", "LOS")];
pub const DURATIONS: &[(&str, &str)] = &[("I", "Instant"), ("P", "Permanent"), ("S", "Sustained")];

/// `ChangeModifiers`: the category's checkboxes as (English label, DV).
/// Every category but Detection, Health, Illusion and Manipulation uses
/// the Combat set (the form's `default` branch).
pub fn modifier_table(category: &str) -> &'static [(&'static str, i32)] {
    match category {
        "Detection" => &[
            ("Directional", 0),
            ("Area", 0),
            ("Psychic", 0),
            ("Active", 0),
            ("Passive", 0),
            ("Basic Detection (ex: Detect Life)", 0),
            ("Complex Detection (ex: Detect Enemies)", 1),
            ("Basic Analyze (ex: Analyze Device)", 1),
            ("Complex Analyze (ex: Analyze Truth)", 2),
            ("Invasive Analyze (ex: Mind Probe)", 4),
            ("Improved Sense", 1),
            ("New Sense", 2),
            ("Psychic Sense (e.g. telepathy, precognition)", 4),
            ("Extended Area", 2),
        ],
        "Health" => &[("Curative", 0), ("Increases Initiative Passes", 4), ("Cosmetic Effect", -2), ("Negative Health Spell", 2), ("Restricted Effect (e.g. Symptoms Only)", -2)],
        "Illusion" => &[("Obvious", -1), ("Realistic", 0), ("Single-Sense", -2), ("Multi-Sense", 0), ("Illusion Hides or Conceals", 2)],
        "Manipulation" => &[("Environmental", -2), ("Mental", 0), ("Physical", 0), ("Minor Change", 0), ("Major Change", 2), ("Elemental effect", 2)],
        _ => &[("Direct", 0), ("Indirect", 0), ("Element effects", 2), ("Physical damage", 0), ("Stun damage", -1)],
    }
}

/// Which checkbox the number of effects multiplies (0-based), if any.
pub fn effects_slot(category: &str) -> Option<usize> {
    match category {
        "Manipulation" => Some(5),
        "Detection" | "Health" | "Illusion" => None,
        _ => Some(2),
    }
}

/// `cboCategory_SelectedIndexChanged`: a new category clears every
/// checkbox; Health spells cannot be area spells.
pub fn set_category(d: &mut SpellDesign, category: &str) {
    d.category = category.to_owned();
    d.mods = [false; MODIFIER_SLOTS];
    d.disabled = [false; MODIFIER_SLOTS];
    d.kind_locked = false;
    if category == "Health" {
        d.area = false;
    }
}

/// Whether the Area checkbox is available (not for Health spells).
pub fn area_allowed(d: &SpellDesign) -> bool {
    d.category != "Health"
}

/// Check or uncheck modifier `i` (0-based), then apply the form's rules.
pub fn set_modifier(d: &mut SpellDesign, i: usize, on: bool) {
    if i >= MODIFIER_SLOTS || i >= modifier_table(&d.category).len() || (on && d.disabled[i]) {
        return;
    }
    d.mods[i] = on;
    apply_rules(d);
}

/// `chkModifier_CheckedChanged`. Mutually exclusive boxes uncheck and
/// disable each other; unchecking re-enables them.
fn apply_rules(d: &mut SpellDesign) {
    d.kind_locked = false;
    let m = |d: &SpellDesign, i: usize| d.mods[i - 1];
    // Uncheck and disable `targets` when `src` is checked, else enable them.
    fn exclusive(d: &mut SpellDesign, src: usize, targets: &[usize]) {
        let on = d.mods[src - 1];
        for &t in targets {
            if on {
                d.mods[t - 1] = false;
            }
            d.disabled[t - 1] = on;
        }
    }
    match d.category.as_str() {
        "Detection" => {
            // Directional and Area cannot be selected at the same time.
            exclusive(d, 1, &[2]);
            if !m(d, 1) {
                exclusive(d, 2, &[1]);
            }
            // Active and Passive cannot be selected at the same time.
            exclusive(d, 4, &[5]);
            if !m(d, 4) {
                exclusive(d, 5, &[4]);
            }
            // If Extended Area is selected, Area must also be selected. (The
            // form tests the Active box here, a slip; this follows its comment.)
            if m(d, 14) {
                d.mods[0] = false;
                d.mods[2] = false;
                d.mods[1] = true;
                d.disabled[0] = true;
            }
        }
        "Health" => {}
        "Illusion" => {
            exclusive(d, 1, &[2]);
            if !m(d, 1) {
                exclusive(d, 2, &[1]);
            }
            exclusive(d, 3, &[4]);
            if !m(d, 3) {
                exclusive(d, 4, &[3]);
            }
        }
        "Manipulation" => {
            // Environmental, Mental and Physical cannot be selected together.
            let picked = [1, 2, 3].into_iter().find(|&i| m(d, i));
            for i in [1, 2, 3] {
                match picked {
                    Some(p) if p != i => {
                        d.mods[i - 1] = false;
                        d.disabled[i - 1] = true;
                    }
                    _ => d.disabled[i - 1] = false,
                }
            }
            exclusive(d, 4, &[5]);
            if !m(d, 4) {
                exclusive(d, 5, &[4]);
            }
        }
        _ => {
            // Elemental effect spells must be Indirect (and so physical).
            if m(d, 3) {
                d.mods[1] = true;
            }
            // Direct and Indirect cannot be selected at the same time.
            if m(d, 1) {
                for t in [2, 3] {
                    d.mods[t - 1] = false;
                    d.disabled[t - 1] = true;
                }
            } else {
                d.disabled[1] = false;
                d.disabled[2] = false;
            }
            // Indirect combat spells are always physical.
            if m(d, 2) {
                d.mods[0] = false;
                d.disabled[0] = true;
                d.kind = "P".into();
                d.kind_locked = true;
            } else {
                d.disabled[0] = false;
            }
            // Physical and Stun damage cannot be selected at the same time.
            exclusive(d, 4, &[5]);
            if !m(d, 4) {
                exclusive(d, 5, &[4]);
            }
        }
    }
}

/// Whether the number-of-effects field is usable (Direct disables it).
pub fn effects_enabled(d: &SpellDesign) -> bool {
    match effects_slot(&d.category) {
        Some(2) => !d.mods[0],
        Some(_) => true,
        None => false,
    }
}

/// `CalculateDrain`: the spell's drain value, e.g. `(F/2)+1`. Health
/// spells with Curative use `(Damage Value)` as their base.
pub fn drain(d: &SpellDesign) -> String {
    let mut dv = 0;
    if d.kind != "M" {
        dv += 1;
    }
    if d.range == "T" {
        dv -= 2;
    }
    if d.area {
        dv += 2;
    }
    if d.restricted {
        dv -= 1;
    }
    if d.very_restricted {
        dv -= 2;
    }
    // Curative Health spells have no modifier for Permanent duration.
    if d.duration == "P" && (d.category != "Health" || !d.mods[0]) {
        dv += 2;
    }
    let multiplied = effects_slot(&d.category);
    for (i, (_, mod_dv)) in modifier_table(&d.category).iter().enumerate() {
        if d.mods[i] {
            dv += if Some(i) == multiplied { mod_dv * d.effects } else { *mod_dv };
        }
    }
    let base = if d.category == "Health" && d.mods[0] { "(Damage Value)" } else { "(F/2)" };
    match dv {
        0 => base.to_owned(),
        n if n > 0 => format!("{base}+{n}"),
        n => format!("{base}{n}"),
    }
}

/// `AcceptForm` checks, as Chummer's English messages.
pub fn problems(d: &SpellDesign) -> Vec<&'static str> {
    let mut v = Vec::new();
    if d.name.trim().is_empty() {
        v.push("You must enter a name for your new Spell.");
    }
    if (d.restricted || d.very_restricted) && d.restriction.trim().is_empty() {
        v.push("You must specify how your Spell is restricted.");
    }
    let m = |i: usize| d.mods[i - 1];
    match d.category.as_str() {
        "Detection" => {
            if !m(1) && !m(2) && !m(3) {
                v.push("You must select at least 1 of Directional, Area, or Psychic from the Spell Options.");
            }
            if !m(4) && !m(5) {
                v.push("You must select either Active or Passive from the Spell Options.");
            }
        }
        "Health" => {}
        "Illusion" => {
            if !m(1) && !m(2) {
                v.push("You must select either Obvious or Realistic from the Spell Options.");
            }
            if !m(3) && !m(4) {
                v.push("You must select either Single-Sense or Multi-Sense from the Spell Options.");
            }
        }
        "Manipulation" => {
            if !m(1) && !m(2) && !m(3) {
                v.push("You must select either Environmental, Mental, or Physical from the Spell Options.");
            }
            if !m(4) && !m(5) {
                v.push("You must select either Minor Change or Major Change from the Spell Options.");
            }
        }
        _ => {
            if !m(1) && !m(2) {
                v.push("You must select either Direct or Indirect from the Spell Options.");
            }
            if !m(4) && !m(5) {
                v.push("You must select either Physical damage or Stun damage from the Spell Options.");
            }
        }
    }
    v
}

/// The saved `range`: "T" or "LOS", plus "(A)" for area spells.
pub fn range(d: &SpellDesign) -> String {
    if d.area { format!("{}(A)", d.range) } else { d.range.clone() }
}

/// `AcceptForm` descriptors, in Chummer's order.
pub fn descriptors(d: &SpellDesign) -> String {
    let m = |i: usize| d.mods[i - 1];
    let mut v: Vec<&str> = Vec::new();
    match d.category.as_str() {
        "Detection" => {
            if m(4) {
                v.push("Active");
            }
            if m(5) {
                v.push("Passive");
            }
            if m(1) {
                v.push("Directional");
            }
            if m(3) {
                v.push("Psychic");
            }
            if m(2) {
                v.push(if m(14) { "Extended Area" } else { "Area" });
            }
        }
        "Health" => {
            if m(4) {
                v.push("Negative");
            }
        }
        "Illusion" => {
            for (i, s) in [(1, "Obvious"), (2, "Realistic"), (3, "Single-Sense"), (4, "Multi-Sense")] {
                if m(i) {
                    v.push(s);
                }
            }
            if d.area {
                v.push("Area");
            }
        }
        "Manipulation" => {
            for (i, s) in [(1, "Environmental"), (2, "Mental"), (3, "Physical")] {
                if m(i) {
                    v.push(s);
                }
            }
            if d.area {
                v.push("Area");
            }
        }
        _ => {
            if m(1) {
                v.push("Direct");
            }
            if m(2) {
                v.push("Indirect");
            }
            // Chummer tests the range combo's value ("T"/"LOS"), which never
            // contains "(A)", so combat spells never get an Area descriptor.
            if d.range.contains("(A)") {
                v.push("Area");
            }
            if m(3) {
                v.push("Elemental");
            }
        }
    }
    v.join(", ")
}

/// The `<spell>` Chummer saves for the design (`Spell.Save` of the spell
/// `AcceptForm` builds: no source id, Street Grimoire p. 159).
pub fn element(d: &SpellDesign, guid: &str) -> Element {
    let mut e = Element::new("spell");
    let mut put = |k: &str, v: String| e.push(Element::with_text(k, v));
    put("sourceid", "00000000-0000-0000-0000-000000000000".into());
    put("guid", guid.to_owned());
    put("name", d.name.clone());
    put("descriptors", descriptors(d));
    put("category", d.category.clone());
    put("type", d.kind.clone());
    put("range", range(d));
    put("damage", if d.category == "Combat" { if d.mods[3] { "P" } else { "S" }.into() } else { String::new() });
    put("duration", d.duration.clone());
    put("dv", drain(d));
    put("useskill", String::new());
    put("limited", bool_str(d.limited));
    for k in ["extended", "customextended", "alchemical"] {
        put(k, bool_str(false));
    }
    put("source", "SM".into());
    put("page", "159".into());
    put("extra", d.restriction.clone());
    put("notes", String::new());
    put("notesColor", "#000000".into());
    put("freebonus", bool_str(false));
    put("barehandedadept", bool_str(false));
    put("improvementsource", "Spell".into());
    put("grade", "0".into());
    e
}

/// `tsCreateSpell_Click` in creation mode: at most 2 × the best of
/// Spellcasting and Ritual Spellcasting (plus `SpellLimit`) spells, unless
/// the character ignores rules.
pub fn creation_limit_reached(ch: &Character, sheet: &crate::calc::Sheet) -> bool {
    if ch.flag("ignorerules") {
        return false;
    }
    let rating = |name: &str| sheet.skills.iter().find(|s| s.name == name).map_or(0, |s| s.rating);
    let limit = 2 * rating("Spellcasting").max(rating("Ritual Spellcasting")) + ch.improvements.val_int("SpellLimit", None);
    ch.items("spells", "spell").len() as i32 >= limit
}

/// Add the designed spell. In career mode it is paid like any learned
/// spell (`tsCreateSpell_Click` in `CharacterCareer`); without enough
/// karma nothing is added. Returns the spell's guid.
pub fn add(ch: &mut Character, engine: &Engine, d: &SpellDesign) -> Result<String, CareerError> {
    if let Some(p) = problems(d).first() {
        return Err(CareerError::Refused((*p).to_owned()));
    }
    let guid = new_guid();
    ch.items_mut("spells").push(element(d, &guid));
    if ch.created {
        if let Err(e) = career::pay_for_spell(ch, engine, &guid) {
            ch.remove_item("spells", &guid);
            return Err(e);
        }
    }
    Ok(guid)
}
