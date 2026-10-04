//! What the Magic & Resonance tab shows: tradition, drain and fading,
//! astral values, grades and power points.

use crate::calc::{Sheet, SheetAttributes};
use crate::character::Character;
use crate::data::DataStore;
use crate::expr;

/// Summary of a character's magic or resonance for the GUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MagicSummary {
    /// Tradition name, empty when none.
    pub tradition: String,
    /// Drain attributes, e.g. "{WIL} + {LOG}" (`Tradition.DrainExpression`).
    pub drain_expression: String,
    /// Drain resistance pool: attributes plus DrainResistance improvements
    /// (`Tradition.DrainValue`). 0 without magic.
    pub drain_pool: i32,
    /// Stream name for technomancers, empty when none.
    pub stream: String,
    /// Fading attributes, e.g. "{RES} + {WIL}".
    pub fading_expression: String,
    /// Fading resistance pool with FadingResistance improvements.
    pub fading_pool: i32,
    /// Spirits of the tradition by type: combat, detection, health,
    /// illusion, manipulation.
    pub spirits: Vec<(String, String)>,
    pub initiate_grade: i32,
    pub submersion_grade: i32,
    pub astral_initiative: i32,
    pub astral_initiative_dice: i32,
    pub astral_limit: i32,
    /// (total, used) for adepts and mystic adepts.
    pub power_points: Option<(f64, f64)>,
    /// (free, paid) spells at creation.
    pub spells: (i32, i32),
    /// (free, bought) complex forms at creation.
    pub complex_forms: (i32, i32),
}

/// Put braces around bare attribute names ("WIL + LOG" -> "{WIL} + {LOG}"),
/// as older saves store the drain without them.
pub fn braced(expr_text: &str) -> String {
    if expr_text.contains('{') {
        return expr_text.to_owned();
    }
    expr_text
        .split_inclusive(|c: char| !c.is_ascii_alphanumeric())
        .map(|part| {
            let word = part.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
            let rest = &part[word.len()..];
            if expr::ATTRIBUTE_NAMES.contains(&word) { format!("{{{word}}}{rest}") } else { part.to_owned() }
        })
        .collect()
}

/// Evaluate an attribute expression with the sheet's totals.
pub fn eval_attributes(sheet: &Sheet, expression: &str) -> f64 {
    let s = expr::substitute_attributes(&braced(expression), &SheetAttributes(&sheet.attributes));
    if expr::needs_evaluation(&s) { expr::evaluate_num(&s).unwrap_or(0.0) } else { expr::parse_plain(&s).unwrap_or(0.0) }
}

/// Tradition (name, drain expression, MAG or RES) from either save
/// layout: a `<tradition>` element (current) or a `<tradition>` text with
/// `<traditiondrain>` (5.18x-5.20x).
fn tradition(ch: &Character) -> Option<(String, String, String)> {
    let t = ch.doc.child("tradition")?;
    if t.elements().next().is_some() {
        return Some((t.get("name"), t.get("drain"), t.child_text("traditiontype").unwrap_or_else(|| "MAG".into())));
    }
    let name = t.text();
    (!name.trim().is_empty()).then(|| (name, ch.doc.get("traditiondrain"), "MAG".to_owned()))
}

/// Technomancer stream (name, fading expression).
fn stream(ch: &Character) -> Option<(String, String)> {
    if let Some((name, drain, _)) = tradition(ch).filter(|t| t.2 == "RES") {
        return Some((name, drain));
    }
    ch.is_technomancer().then(|| (ch.doc.get("stream"), ch.doc.child_text("streamdrain").filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "RES + WIL".into())))
}

/// Summary for the Magic & Resonance tab.
pub fn magic_summary(ch: &Character, sheet: &Sheet) -> MagicSummary {
    magic_summary_with(ch, sheet, None)
}

/// The tradition's `<drain>` from traditions.xml, for saves that only
/// store the tradition name (5.18x).
fn data_drain(store: &DataStore, tradition: &str) -> Option<String> {
    let doc = store.doc("traditions.xml").ok()?;
    crate::data::find(&doc, "traditions", "tradition", tradition).map(|r| r.get("drain"))
}

/// [`magic_summary`], looking up the drain in the data when the save has
/// none.
pub fn magic_summary_with(ch: &Character, sheet: &Sheet, store: Option<&DataStore>) -> MagicSummary {
    let mut m = MagicSummary {
        initiate_grade: ch.doc.get_i32("initiategrade").unwrap_or(0),
        submersion_grade: ch.doc.get_i32("submersiongrade").unwrap_or(0),
        astral_initiative: sheet.astral_initiative,
        astral_initiative_dice: sheet.astral_initiative_dice,
        astral_limit: sheet.limit_astral,
        ..Default::default()
    };
    if ch.mag_enabled() {
        let (name, mut drain) = tradition(ch).filter(|t| t.2 == "MAG").map(|t| (t.0, t.1)).unwrap_or_default();
        if drain.trim().is_empty() {
            drain = store.and_then(|s| data_drain(s, &name)).unwrap_or_default();
        }
        m.tradition = name;
        // Adepts without spellcasting resist drain with BOD + WIL.
        m.drain_expression = if ch.is_adept() && !ch.is_magician() { "{BOD} + {WIL}".into() } else { braced(&drain) };
        if !m.drain_expression.trim().is_empty() {
            m.drain_pool = expr::standard_round(eval_attributes(sheet, &m.drain_expression) + ch.improvements.val("DrainResistance", None));
        }
        m.spirits = spirits(ch);
    }
    if ch.res_enabled() {
        if let Some((name, fading)) = stream(ch) {
            m.stream = name;
            m.fading_expression = braced(&fading);
            m.fading_pool = expr::standard_round(eval_attributes(sheet, &m.fading_expression) + ch.improvements.val("FadingResistance", None));
        }
    }
    if ch.is_adept() {
        m.power_points = Some(super::account::power_points(ch, sheet));
    }
    let sc = super::account::spell_counts(ch, sheet);
    m.spells = (sc.free, sc.spells + sc.rituals + sc.preparations);
    let cf = super::account::complex_form_counts(ch);
    m.complex_forms = (cf.free, cf.forms);
    m
}

/// The tradition's spirit for each type of spell.
fn spirits(ch: &Character) -> Vec<(String, String)> {
    let holder = match ch.doc.child("tradition") {
        Some(t) if t.elements().next().is_some() => t,
        _ => &ch.doc,
    };
    [("Combat", "spiritcombat"), ("Detection", "spiritdetection"), ("Health", "spirithealth"), ("Illusion", "spiritillusion"), ("Manipulation", "spiritmanipulation")]
        .iter()
        .map(|(label, k)| ((*label).to_owned(), holder.get(k)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}
