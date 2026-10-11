//! Suggested initiative adjustments: the conditional or activated
//! bonuses a character has that its sheet's initiative does not count
//! ([`suggestions`]), offered on the initiative boards as quick buttons.
//!
//! What the sheet counts already is never suggested: anything with an
//! `<initiative>`, `<initiativedice>` or `<initiativepass>` bonus in the
//! data (Wired Reflexes, Synaptic Booster, Synaptic Acceleration,
//! Improved Reflexes, Lightning Reflexes, …) is always on, and a drug
//! whose effect improvements the player switched on is in the sheet.
//!
//! Values come from the data where the data has them (the drugs'
//! `<bonus>` in drugcomponents.xml, a power's level, a bioware's rating).
//! Adrenaline Boost, the Adrenaline Pump and Increase Reflexes have no
//! bonus in the data at all; their numbers below come from the rules
//! text and are marked TODO.

use super::initiative::{InitAdjust, Lasts};
use crate::calc::Sheet;
use crate::character::Character;
use crate::data::DataStore;
use crate::items::drug;
use crate::xml::Element;

/// TODO(rules text, not in data): Adrenaline Boost adds 2 to the
/// Initiative Score per level, in the first combat turn (SR5 p. 309).
pub const ADRENALINE_BOOST_PER_LEVEL: i32 = 2;

/// TODO(rules text, not in data): an activated Adrenaline Pump adds its
/// rating to Reaction (among other attributes), so to the Initiative
/// Score, while it runs (SR5 p. 459).
pub const ADRENALINE_PUMP_PER_RATING: i32 = 1;

/// A bonus the board offers for a character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The adjustment it adds (`source` set, so it is offered once).
    pub adjust: InitAdjust,
    /// What it is and where the rule is ("SR5 p. 309"), for the tooltip.
    pub note: String,
    /// The numbers depend on the moment (a spell's hits): open the
    /// adjustment form with it filled in instead of adding it.
    pub ask: bool,
}

/// The conditional initiative bonuses `ch` has; `sheet` is its computed
/// sheet (for a power's level), `store` its game data (for drugs).
pub fn suggestions(ch: &Character, sheet: &Sheet, store: &DataStore) -> Vec<Suggestion> {
    let mut out = Vec::new();
    // Adept power: Adrenaline Boost, its level.
    let mag = crate::items::magic::account::adept_mag(ch, sheet, false);
    for p in ch.items("powers", "power").into_iter().filter(|p| p.get("name") == "Adrenaline Boost") {
        let level = crate::items::magic::power::total_rating(ch, p, mag);
        if level <= 0 {
            continue;
        }
        let a = InitAdjust::new(format!("Adrenaline Boost {level}"), level * ADRENALINE_BOOST_PER_LEVEL, 0, Lasts::Turn).from_source(format!("power:{}", p.get("guid")));
        out.push(Suggestion { adjust: a, note: format!("{} p. {}: first combat turn; Drain afterwards", p.get("source"), p.get("page")), ask: false });
    }
    // Bioware: an activated Adrenaline Pump, its rating.
    walk(ch.doc.child("cyberwares"), "cyberware", &mut |w| {
        let name = w.get("name");
        if name != "Adrenaline Pump" && name != "Adrenaline Pump (2050)" {
            return;
        }
        let rating = w.get_i32("rating").unwrap_or(0);
        if rating <= 0 {
            return;
        }
        let a = InitAdjust::new(format!("{name} {rating}"), rating * ADRENALINE_PUMP_PER_RATING, 0, Lasts::Removed).from_source(format!("ware:{}", w.get("guid")));
        out.push(Suggestion { adjust: a, note: format!("{} p. {}: while active (Reaction +{rating})", w.get("source"), w.get("page")), ask: false });
    });
    // Drugs bought as gear: their record in drugcomponents.xml.
    let drugs = store.doc("drugcomponents.xml").ok();
    walk(ch.doc.child("gears"), "gear", &mut |g| {
        if g.get("category") != "Drugs" || g.get_f64("qty").is_some_and(|q| q <= 0.0) {
            return;
        }
        let Some(doc) = drugs.as_deref() else { return };
        let Some(rec) = crate::data::find(doc, "drugs", "drug", &g.get("id")).or_else(|| crate::data::find(doc, "drugs", "drug", &g.get("name"))) else { return };
        let fx = drug::effects(&drug::element(rec, &Default::default(), ""));
        if let Some(s) = drug_suggestion(&g.get("name"), &format!("gear:{}", g.get("guid")), &fx, &format!("{} p. {}", rec.source(), rec.page())) {
            out.push(s);
        }
    });
    // Drugs (built ones): unless their effects are switched on already.
    for d in ch.items("drugs", "drug") {
        let guid = d.get("guid");
        let on = ch.improvements.list.iter().any(|i| i.source == "Drug" && i.source_name == guid && i.enabled);
        if on || d.get_f64("qty").is_some_and(|q| q <= 0.0) {
            continue;
        }
        if let Some(s) = drug_suggestion(&d.get("name"), &format!("drug:{guid}"), &drug::effects(d), &format!("{} p. {}", d.get("source"), d.get("page"))) {
            out.push(s);
        }
    }
    // A sustained Increase Reflexes: its dice depend on the hits.
    for s in ch.items("spells", "spell").into_iter().filter(|s| s.get("name") == "Increase Reflexes") {
        // TODO(rules text, not in data): the bonus per hit (SR5 p. 288).
        let a = InitAdjust::new("Increase Reflexes", 0, 1, Lasts::Removed).from_source(format!("spell:{}", s.get("guid")));
        out.push(Suggestion { adjust: a, note: format!("{} p. {}: sustained; enter the bonus its hits give", s.get("source"), s.get("page")), ask: true });
    }
    out
}

/// A drug's initiative effect: Reaction and Intuition and initiative add
/// to the score, initiative dice to the dice; `None` when it has none.
fn drug_suggestion(name: &str, source: &str, fx: &drug::Effects, page: &str) -> Option<Suggestion> {
    let attr = |a: &str| fx.attributes.iter().filter(|(n, _)| n.eq_ignore_ascii_case(a)).map(|(_, v)| *v as i32).sum::<i32>();
    let score = fx.initiative + attr("REA") + attr("INT");
    let dice = fx.initiative_dice;
    if score == 0 && dice == 0 {
        return None;
    }
    let a = InitAdjust::new(name, score, dice, Lasts::Removed).from_source(source);
    Some(Suggestion { adjust: a, note: format!("{page}: while it lasts"), ask: false })
}

/// Every `item` element under `list`, nested ones (`<children>`) too.
fn walk(list: Option<&Element>, item: &str, f: &mut dyn FnMut(&Element)) {
    let Some(list) = list else { return };
    for e in list.children_named(item) {
        f(e);
        walk(e.child("children"), item, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use std::path::Path;

    fn load(name: &str) -> Option<(Engine, Character, Sheet)> {
        let engine = Engine::load().ok()?;
        let ch = Character::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap();
        let sheet = engine.sheet(&ch);
        Some((engine, ch, sheet))
    }

    #[test]
    fn adrenaline_boost_by_level() {
        let Some((engine, ch, sheet)) = load("Ocelot2.0.chum5") else { return };
        let s = suggestions(&ch, &sheet, &engine.store_for_character(&ch));
        let boost = s.iter().find(|s| s.adjust.label.starts_with("Adrenaline Boost")).expect("Ocelot has Adrenaline Boost");
        let p = ch.items("powers", "power").into_iter().find(|p| p.get("name") == "Adrenaline Boost").unwrap();
        let level = crate::items::magic::power::total_rating(&ch, p, crate::items::magic::account::adept_mag(&ch, &sheet, false));
        assert!(level > 0);
        assert_eq!(boost.adjust.label, format!("Adrenaline Boost {level}"));
        assert_eq!((boost.adjust.score, boost.adjust.dice, boost.adjust.lasts), (level * 2, 0, Lasts::Turn));
        assert!(boost.adjust.source.starts_with("power:"));
        assert!(boost.note.contains("SR5 p. 309"), "{}", boost.note);
        // Always-on boosters are in the sheet and never suggested.
        assert!(s.iter().all(|x| !x.adjust.label.contains("Reflexes") || x.ask));
    }

    #[test]
    fn drugs_as_gear_from_data() {
        let Some((engine, ch, sheet)) = load("BLUE.chum5") else { return };
        let s = suggestions(&ch, &sheet, &engine.store_for_character(&ch));
        let jazz = s.iter().find(|s| s.adjust.label == "Jazz").expect("BLUE carries Jazz");
        // drugcomponents.xml: Jazz REA +1, initiativedice 2.
        assert_eq!((jazz.adjust.score, jazz.adjust.dice, jazz.adjust.lasts), (1, 2, Lasts::Removed));
    }

    #[test]
    fn switched_on_drugs_are_in_the_sheet() {
        let Some((engine, mut ch, _)) = load("Munin_Career.chum5") else { return };
        let store = engine.store_for_character(&ch);
        let doc = store.doc("drugcomponents.xml").unwrap();
        let rec = crate::data::find(&doc, "drugs", "drug", "Jazz").unwrap();
        drug::add_element(&mut ch, drug::element(rec, &Default::default(), "11111111-2222-3333-4444-555555555555"));
        let sheet = engine.sheet(&ch);
        let s = suggestions(&ch, &sheet, &store);
        assert!(s.iter().any(|x| x.adjust.source == "drug:11111111-2222-3333-4444-555555555555"), "off: offered");
        let dice_before = sheet.initiative_dice;
        for i in ch.improvements.list.iter_mut().filter(|i| i.source == "Drug") {
            i.enabled = true;
        }
        let sheet = engine.sheet(&ch);
        assert_eq!(sheet.initiative_dice, dice_before + 2, "on: the sheet counts it");
        let s = suggestions(&ch, &sheet, &store);
        assert!(s.iter().all(|x| !x.adjust.source.starts_with("drug:")), "on: not offered again");
    }

    #[test]
    fn increase_reflexes_asks() {
        let Some((engine, ch, sheet)) = load("Draught.chum5") else { return };
        let s = suggestions(&ch, &sheet, &engine.store_for_character(&ch));
        let ir = s.iter().find(|s| s.adjust.label == "Increase Reflexes").expect("Draught knows Increase Reflexes");
        assert!(ir.ask);
    }
}
