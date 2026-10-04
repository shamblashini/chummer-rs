//! Creation-mode fixtures save the nuyen left after shopping. Starting
//! nuyen (+ karma converted) minus what chummer-rs computes for everything
//! owned must give the same amount.
//!
//! The fixtures were saved by Chummer 5.183-5.202, whose `CalculateNuyen`
//! wrote `TotalStartingNuyen - deductions` into `<nuyen>`. Every fixture
//! that still differs does so only because of lifestyles: current Chummer
//! prices the same saved `<lifestyle>` differently from 5.202 (drift, not a
//! bug we can fix without disagreeing with Chummer today):
//!
//! - 5.202 kept free grid subscriptions in a separate `<freegrids>` list that
//!   `TotalMonthlyCost` never read. Current `Lifestyle.Load` moves them into
//!   `LifestyleQualities` with their saved `Selected` source, so they cost
//!   50 a month when the base lifestyle is not in their `<allowed>` list.
//! - 5.202 summed every percentage (area/comforts/security points, quality
//!   multipliers, `LifestyleCost` improvements) into one `CostMultiplier`,
//!   used in `BaseCost = Cost * max(cm + bcm, 1)` and again as `* (cm + 1)`,
//!   with flat quality costs added before it. Current Chummer (HT 139)
//!   multiplies base multipliers, asset multipliers and other multipliers in
//!   turn, adds flat costs after each step, and applies improvements
//!   (dependents, metatype, the rest) multiplicatively.
//!
//! Lifestyle delta (current minus 5.202) per fixture, which is exactly its
//! difference from the saved nuyen:
//!
//! | fixture | delta | cause |
//! |---|---|---|
//! | Apex Predator | +40.05 | grid on Squatter; -10% improvements multiplied |
//! | BLUE | +1050 | grid on Bolt Hole; base/other multipliers multiplied |
//! | Bastion | +73 | grid on Squatter; +10% quality improvement |
//! | Gangerbean | +480 | Extra Secure x comforts/security points |
//! | Gentle Earthquake | -125 | Troll +100% applied multiplicatively |
//! | Mittens Chargen | -160 | Indoor Arboretum before -10% |
//! | Monomax (approved) 3 | +128 | Troll +100%, grid, multipliers |
//! | Munin | +45 | grid on Low, x0.9 |
//! | Ocelot2.0 | -29 | +20% quality improvement |
//! | Popstar | -20 | multiplier order |
//! | Rez0luti0n2.0 | -51.68 | Dwarf +20% and +10% quality multiplied |
//! | SCSi, Tenshi, prime, resub | +50 | grid on Low |
//! | Skink | -110 | +10% quality improvement |
//! | Soma | +55 | grid on Low, x1.1 |
//! | Ushi Resub | +45 | grid on Low, x0.9 (Cramped) |
//! | Yeti-#ffffff2 | +23.6 | grid on Low; multipliers multiplied |
//!
//! [`creation_nuyen_left_matches_chummer`] counts exact matches.
//! [`only_lifestyle_drift_remains`] re-prices lifestyles with the 5.202
//! formula ([`lifestyle_5202`], test-only) and requires every fixture to
//! match, so a gear, ware, armor, weapon or vehicle regression in any of the
//! drifting fixtures still fails.

use std::path::PathBuf;

use chummer_core::chargen;
use chummer_core::character::Character;
use chummer_core::engine::Engine;
use chummer_core::items::lifestyle;
use chummer_core::xml::Element;

/// Exact matches must not fall below this.
const BASELINE: usize = 8;

/// Creation-mode fixtures with a saved `<nuyen>`: (file name, character,
/// saved nuyen, nuyen left by our costs).
fn creation_fixtures() -> Vec<(String, Character, f64, f64)> {
    let engine = Engine::load().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let ch = Character::load(&f).unwrap();
        if ch.created {
            continue;
        }
        let Some(saved) = ch.doc.get_f64("nuyen") else { continue };
        let store = engine.store_for_character(&ch);
        let start = ch.doc.get_f64("startingnuyen").unwrap_or(0.0) + f64::from(ch.doc.get_i32("nuyenbp").unwrap_or(0)) * 2000.0;
        let left = start - chargen::nuyen_spent(&ch, Some(&store));
        out.push((f.file_name().unwrap().to_string_lossy().into_owned(), ch, saved, left));
    }
    out
}

#[test]
fn creation_nuyen_left_matches_chummer() {
    let fixtures = creation_fixtures();
    let mut ok = 0;
    for (name, _, saved, left) in &fixtures {
        let hit = (left - saved).abs() < 0.5;
        if hit {
            ok += 1;
        }
        eprintln!("{} {name:<28} saved {saved:>10} computed {left:>10}", if hit { "ok  " } else { "DIFF" });
    }
    eprintln!("nuyen oracle: {ok} of {} creation-mode characters", fixtures.len());
    assert!(ok >= BASELINE);
}

#[test]
fn only_lifestyle_drift_remains() {
    let mut bad = Vec::new();
    for (name, ch, saved, left) in creation_fixtures() {
        let drift: f64 = ch.items("lifestyles", "lifestyle").into_iter().map(|e| lifestyle::total_cost(&ch, e) - lifestyle_5202(&ch, e)).sum();
        if (left + drift - saved).abs() >= 0.5 {
            bad.push(format!("{name}: saved {saved}, computed {left}, lifestyle drift {drift}"));
        }
    }
    assert!(bad.is_empty(), "differences not explained by lifestyle drift:\n{}", bad.join("\n"));
}

/// The 5.202 `Lifestyle.TotalMonthlyCost` x months, for the drift check
/// only. Free grids (`<freegrids>`) were not counted; qualities were free
/// when built in, `free`, or entertainment/contracts allowed on the base.
fn lifestyle_5202(ch: &Character, e: &Element) -> f64 {
    let n = |el: &Element, k: &str| el.get_f64(k).unwrap_or(0.0);
    let base = e.get("baselifestyle");
    let standard = matches!(e.get("type").as_str(), "" | "Standard");
    let free = |q: &Element| {
        let kind = q.get("lifestylequalitytype");
        q.get("lifestylequalitysource") == "BuiltIn"
            || q.get_bool("free").unwrap_or(false)
            || (kind == "Entertainment" || kind == "Contracts") && q.get("allowed").split(',').any(|a| a == base)
    };
    let qualities: Vec<&Element> = e.child("lifestylequalities").into_iter().flat_map(|l| l.children_named("lifestylequality")).filter(|q| !free(q)).collect();
    let improvements: f64 = ch
        .improvements
        .list
        .iter()
        .filter(|i| i.enabled && (i.kind == "LifestyleCost" || standard && i.kind == "BasicLifestyleCost"))
        .map(|i| i.val)
        .sum();
    let cm = ((n(e, "roommates") + n(e, "area") + n(e, "comforts") + n(e, "security")) * 10.0 + improvements + qualities.iter().map(|q| n(q, "multiplier")).sum::<f64>()) / 100.0;
    let bcm = qualities.iter().map(|q| n(q, "basemultiplier")).sum::<f64>() / 100.0;
    let cost = |q: &Element| chummer_core::expr::evaluate_num(&q.get("cost")).unwrap_or(0.0);
    let contracts: f64 = qualities.iter().filter(|q| q.get("lifestylequalitytype") == "Contracts").map(|q| cost(q)).sum();
    let extras: f64 = qualities.iter().filter(|q| q.get("lifestylequalitytype") != "Contracts").map(|q| cost(q)).sum();
    let mut total = if e.get_bool("trustfund").unwrap_or(false) { 0.0 } else { n(e, "cost") * (cm + bcm).max(1.0) };
    total += n(e, "area") * n(e, "costforearea") + n(e, "comforts") * n(e, "costforcomforts") + n(e, "security") * n(e, "costforsecurity");
    total = (total + extras).max(0.0) * (cm + 1.0);
    if !e.get_bool("primarytenant").unwrap_or(false) {
        total /= n(e, "roommates") + 1.0;
    }
    total *= e.get_f64("percentage").unwrap_or(100.0) / 100.0;
    (total + contracts) * f64::from(e.get_i32("months").unwrap_or(1))
}
