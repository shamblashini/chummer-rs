//! A.I. programs and Advanced Programs (`AIProgram.Create` /
//! `AIProgram.Save`, `programs.xml`), kept in `<aiprograms>`.
//!
//! Programs have no rating. A program granted by a bonus (Inherent
//! Program, a critter's programs) has `candelete` False: it costs nothing
//! and cannot be removed on its own. Creation karma follows
//! `CharacterCreate.CalculateBP`; career purchases are in
//! [`crate::career::learn_ai_program`].

use crate::bonus::{self, BonusSource, Choice};
use crate::calc::Rules;
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr::standard_round;
use crate::improvement::{bool_str, Field, Query};
use crate::xml::Element;

use super::magic::{apply_bonus, commit, data_notes, source};
use super::Purchase;

/// Fields the oracle does not compare.
pub const IGNORE: &[&str] = &[];

/// The data category of Advanced Programs.
pub const ADVANCED: &str = "Advanced Programs";

/// `GlobalSettings.DefaultHasNotesColor`.
const NOTES_COLOR: &str = "Chocolate";

/// Build an `<aiprogram>` in `AIProgram.Save` order.
pub fn element(rec: Record<'_>, guid: &str, extra: &str, can_delete: bool) -> Element {
    let e = rec.el();
    let mut p = Element::new("aiprogram");
    let notes_color = e.child_text("notesColor").unwrap_or_else(|| NOTES_COLOR.into());
    for (k, v) in [
        ("sourceid", rec.id()),
        ("guid", guid.to_owned()),
        ("name", rec.name()),
        ("candelete", bool_str(can_delete)),
        ("isadvancedprogram", bool_str(rec.category() == ADVANCED)),
        ("requiresprogram", e.get("require")),
        ("extra", extra.to_owned()),
        ("source", rec.source()),
        ("page", rec.page()),
        ("notes", data_notes(e)),
        ("notesColor", notes_color),
    ] {
        p.push(Element::with_text(k, v));
    }
    p
}

/// Selections the program's bonus needs (`selecttext` for "[Vehicle]
/// Autosoft" and the like, `selectskill`).
pub fn choices(ch: &Character, store: &DataStore, rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    let src = BonusSource { kind: "AIProgram".into(), guid: String::new(), name: rec.name(), rating: 1 };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Add a program (`AIProgram.Create`): its bonus runs with `extra` as the
/// forced answer, and the selected value becomes `<extra>`.
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, extra: Option<&str>, can_delete: bool) -> String {
    let guid = super::new_guid();
    let src = source("AIProgram", &guid, &rec.name(), 1);
    let out = apply_bonus(ch, store, rec.el().child("bonus"), &src, extra);
    let extra = out.selected.clone().filter(|s| !s.is_empty()).or(extra.map(str::to_owned)).unwrap_or_default();
    commit(ch, store, "aiprograms", element(rec, &guid, &extra, can_delete), &out);
    guid
}

/// `items::add` entry point: a program bought by the player.
pub fn add_purchase(ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    Ok(add(ch, store, rec, p.answer.as_deref(), true))
}

/// Oracle: rebuild a saved `<aiprogram>`.
pub fn rebuild(store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc("programs.xml").ok()?;
    let rec = super::magic::find_saved(&doc, "programs", "program", saved)?;
    let mut e = element(rec, &saved.get("guid"), &saved.get("extra"), saved.get_bool("candelete").unwrap_or(true));
    e.set_child_text("notesColor", saved.get("notesColor"));
    Some(e)
}

/// Remove a program and its improvements (`AIProgram.Remove`). Programs
/// granted by a bonus cannot be removed on their own.
pub fn remove(ch: &mut Character, guid: &str) -> Result<(), String> {
    let p = ch.items("aiprograms", "aiprogram").into_iter().find(|p| p.get("guid").eq_ignore_ascii_case(guid)).cloned();
    let Some(p) = p else { return Err(format!("no program {guid}")) };
    if !p.get_bool("candelete").unwrap_or(true) {
        return Err(format!("{} was granted by an improvement and cannot be removed", p.get("name")));
    }
    ch.remove_item("aiprograms", guid);
    Ok(())
}

/// Name of another program on the character whose data requires the
/// program `guid` (`<required>` … `<program>Name</program>`), if any.
pub fn required_by(ch: &Character, store: &DataStore, guid: &str) -> Option<String> {
    let programs = ch.items("aiprograms", "aiprogram");
    let name = programs.iter().find(|p| p.get("guid").eq_ignore_ascii_case(guid))?.get("name");
    let doc = store.doc("programs.xml").ok()?;
    programs.iter().filter(|p| !p.get("guid").eq_ignore_ascii_case(guid)).find_map(|p| {
        let rec = super::magic::find_saved(&doc, "programs", "program", p)?;
        let mut found = Vec::new();
        rec.el().child("required")?.descendants("program", &mut found);
        found.iter().any(|e| e.text() == name).then(|| p.get("name"))
    })
}

/// `AIProgram.IsAdvancedProgram`.
pub fn is_advanced(p: &Element) -> bool {
    p.get_bool("isadvancedprogram").unwrap_or(false)
}

fn karma_cost(ch: &Character, base: i32, kind: &str) -> i32 {
    let imps = &ch.improvements;
    let mut cost = f64::from(base) + imps.val(kind, None);
    let mult: f64 = imps.winners(Query::new(&format!("{kind}Multiplier")), Field::Val).iter().map(|i| i.val / 100.0).product();
    if mult != 1.0 {
        cost *= mult;
    }
    standard_round(cost).max(0)
}

/// `Character.AIProgramKarmaCost`: `KarmaNewAIProgram` plus
/// `NewAIProgramKarmaCost` improvements, times the multipliers.
pub fn program_karma_cost(ch: &Character, rules: &Rules) -> i32 {
    karma_cost(ch, rules.karma_new_ai_program, "NewAIProgramKarmaCost")
}

/// `Character.AIAdvancedProgramKarmaCost`.
pub fn advanced_program_karma_cost(ch: &Character, rules: &Rules) -> i32 {
    karma_cost(ch, rules.karma_new_ai_advanced_program, "NewAIAdvancedProgramKarmaCost")
}

/// Programs bought at creation against the talent's free programs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProgramCounts {
    /// Deletable programs that are not Advanced Programs.
    pub normal: i32,
    pub advanced: i32,
    /// `AINormalProgramLimit` and `AIAdvancedProgramLimit` (saved as
    /// `<ainormalprogramlimit>` and `<aiadvancedprogramlimit>`).
    pub normal_limit: i32,
    pub advanced_limit: i32,
}

pub fn counts(ch: &Character) -> ProgramCounts {
    let mut c = ProgramCounts {
        normal_limit: ch.doc.get_i32("ainormalprogramlimit").unwrap_or(0),
        advanced_limit: ch.doc.get_i32("aiadvancedprogramlimit").unwrap_or(0),
        ..Default::default()
    };
    for p in ch.items("aiprograms", "aiprogram").into_iter().filter(|p| p.get_bool("candelete").unwrap_or(true)) {
        if is_advanced(p) {
            c.advanced += 1;
        } else {
            c.normal += 1;
        }
    }
    c
}

/// Karma for programs at creation (`CalculateBP`): normal programs beyond
/// the free ones first use spare Advanced Program slots, then cost
/// `AIProgramKarmaCost` each; Advanced Programs beyond theirs cost
/// `AIAdvancedProgramKarmaCost`. 0 in career mode.
pub fn creation_karma(ch: &Character, rules: &Rules) -> i32 {
    if ch.created {
        return 0;
    }
    let c = counts(ch);
    let mut normal = c.normal;
    let mut karma = 0;
    if normal > c.normal_limit {
        if c.advanced < c.advanced_limit {
            normal -= (normal - c.normal_limit).min(c.advanced_limit - c.advanced);
        }
        if normal > c.normal_limit {
            karma += (normal - c.normal_limit) * program_karma_cost(ch, rules);
        }
    }
    if c.advanced > c.advanced_limit {
        karma += (c.advanced - c.advanced_limit) * advanced_program_karma_cost(ch, rules);
    }
    karma
}
