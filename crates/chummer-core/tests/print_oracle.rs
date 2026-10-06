//! Consistency oracle for the print XML. There is no print XML saved by
//! Chummer5a itself, so the printed values are checked against values
//! known independently:
//!
//! - what Chummer5a saved into each fixture (`<totalvalue>` per
//!   attribute, `<totaless>`, `<karma>`, `<nuyen>`);
//! - the engine calculations the print XML must agree with
//!   (`weapon::stats_with`, `vehicle::stats_with`, the item cost
//!   functions, `lifestyle::total_cost`, `career::reputation_for`).
//!
//! Run with PRINT_ORACLE_VERBOSE=1 to list the saved-value differences.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::{armor, cyberware, gear, lifestyle, vehicle, weapon};
use chummer_core::lang::Language;
use chummer_core::print;
use chummer_core::xml::Element;

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "chum5")).collect();
    v.sort();
    v
}

fn nuyen(v: f64) -> String {
    chummer_core::format::nuyen(v).trim_end_matches('¥').to_owned()
}

/// Every element named `tag` under `e`, depth first.
fn all<'a>(e: &'a Element, tag: &'a str) -> Vec<&'a Element> {
    let mut v = Vec::new();
    e.descendants(tag, &mut v);
    v
}

/// guid -> saved element, for every element named `tag` in the save.
fn saved_by_guid<'a>(ch: &'a Character, tag: &'a str) -> HashMap<String, &'a Element> {
    all(&ch.doc, tag).into_iter().map(|e| (e.get("guid").to_ascii_lowercase(), e)).collect()
}

/// The `<attribute>` elements (the character also has a scalar
/// `<attributes>` with the attribute points, as in Chummer).
fn printed_attributes(pc: &Element) -> Vec<&Element> {
    pc.children_named("attributes").flat_map(|a| a.children_named("attribute")).collect()
}

#[derive(Default)]
struct Tally {
    checked: usize,
    bad: Vec<String>,
}

impl Tally {
    fn eq(&mut self, label: impl FnOnce() -> String, got: &str, want: &str) {
        self.checked += 1;
        if got != want {
            self.bad.push(format!("{}: printed {got:?}, expected {want:?}", label()));
        }
    }
}

/// Printed values equal the engine's for every weapon, vehicle and item
/// cost, in every fixture. These must all agree exactly.
#[test]
fn printed_values_equal_engine_values() {
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let mut t = Tally::default();
    let mut weapons = 0;
    let mut vehicles = 0;
    for f in fixtures() {
        let ch = Character::load(&f).unwrap();
        let fname = f.file_name().unwrap().to_string_lossy().to_string();
        let root = print::print_xml(&ch, &engine, &lang);
        let pc = root.child("character").unwrap();
        let sheet = engine.sheet(&ch);
        let store = engine.store_for_character(&ch);
        let settings = engine.settings.resolve(&ch.field("settings"));
        let wrules = settings.map(weapon::WeaponRules::from_settings).unwrap_or_default();
        let vrules = settings.map(|s| vehicle::VehicleRules::from_settings(s, ch.flag("ignorerules"))).unwrap_or_default();

        // Weapons, wherever they are (character, vehicles, underbarrels).
        let saved_weapons = saved_by_guid(&ch, "weapon");
        for pw in all(pc, "weapon") {
            let guid = pw.get("guid").to_ascii_lowercase();
            let Some(sw) = saved_weapons.get(&guid) else { continue };
            weapons += 1;
            let s = weapon::stats_with(&ch, &sheet, Some(&store), sw, &wrules);
            let l = |k: &str| format!("{fname} weapon {} {k}", sw.get("name"));
            t.eq(|| l("damage"), &pw.get("damage"), &s.damage);
            t.eq(|| l("ap"), &pw.get("ap"), &s.ap);
            t.eq(|| l("rc"), &pw.get("rc"), &s.rc);
            t.eq(|| l("reach"), &pw.get("reach"), &s.reach.to_string());
            t.eq(|| l("dicepool"), &pw.get("dicepool"), &s.dice_pool.to_string());
            let acc = pw.get("accuracy");
            t.eq(|| l("accuracy"), acc.rsplit('(').next().unwrap().trim_end_matches(')'), &s.accuracy.to_string());
            let r = pw.child("ranges").unwrap();
            t.eq(|| l("short"), &r.get("short"), &s.ranges.short);
            t.eq(|| l("extreme"), &r.get("extreme"), &s.ranges.extreme);
            if sw.child("clips").is_none_or(|c| c.elements().next().is_none()) {
                t.eq(|| l("dicepool_noammo"), &pw.get("dicepool_noammo"), &pw.get("dicepool"));
                t.eq(|| l("damage_noammo"), &pw.get("damage_noammo"), &pw.get("damage"));
            }
            // Weapons made by gear print the gear's cost.
            let pid = sw.get("parentid");
            let from_gear = !pid.is_empty() && all(&ch.doc, "gear").iter().any(|g| g.get("guid").eq_ignore_ascii_case(&pid));
            if !from_gear && !sw.get_bool("included").unwrap_or(false) {
                t.eq(|| l("cost"), &pw.get("cost"), &nuyen(weapon::cost(sw)));
            }
        }

        // Vehicles: totals after mods and cost.
        let saved_vehicles = saved_by_guid(&ch, "vehicle");
        for pv in pc.child("vehicles").into_iter().flat_map(|v| v.children_named("vehicle")) {
            let Some(sv) = saved_vehicles.get(&pv.get("guid").to_ascii_lowercase()) else { continue };
            vehicles += 1;
            let st = vehicle::stats_with(sv, &vrules);
            let l = |k: &str| format!("{fname} vehicle {} {k}", sv.get("name"));
            t.eq(|| l("handling"), &pv.get("handling"), &st.handling_text);
            t.eq(|| l("speed"), &pv.get("speed"), &st.speed_text);
            t.eq(|| l("accel"), &pv.get("accel"), &st.accel_text);
            t.eq(|| l("body"), &pv.get("body"), &st.body.to_string());
            t.eq(|| l("armor"), &pv.get("armor"), &st.armor.to_string());
            t.eq(|| l("pilot"), &pv.get("pilot"), &st.pilot.to_string());
            t.eq(|| l("sensor"), &pv.get("sensor"), &st.sensor.to_string());
            t.eq(|| l("devicerating"), &pv.get("devicerating"), &st.device_rating.to_string());
            t.eq(|| l("cost"), &pv.get("cost"), &nuyen(vehicle::cost(sv)));
        }

        // Top-level gear, armor, cyberware, lifestyles.
        let pairs = |container: &str, tag: &'static str| -> Vec<(Element, Element)> {
            let printed: Vec<Element> = pc.child(container).map(|c| c.children_named(tag).cloned().collect()).unwrap_or_default();
            let saved = ch.items(container, tag);
            printed.into_iter().filter_map(|p| saved.iter().find(|s| s.get("guid").eq_ignore_ascii_case(&p.get("guid"))).map(|s| (p, (*s).clone()))).collect()
        };
        for (p, s) in pairs("gears", "gear") {
            t.eq(|| format!("{fname} gear {} cost", s.get("name")), &p.get("cost"), &nuyen(gear::cost(&s)));
        }
        for (p, s) in pairs("armors", "armor") {
            t.eq(|| format!("{fname} armor {} cost", s.get("name")), &p.get("cost"), &nuyen(armor::cost(&s)));
        }
        for (p, s) in pairs("cyberwares", "cyberware") {
            t.eq(|| format!("{fname} ware {} cost", s.get("name")), &p.get("cost"), &nuyen(cyberware::cost(&ch, &store, &s)));
            t.eq(|| format!("{fname} ware {} ess", s.get("name")), &p.get("ess"), &chummer_core::format::essence(cyberware::essence(&ch, &store, &engine.rules_for(&ch), &s), engine.rules_for(&ch).essence_decimals));
        }
        let lifestyles = ch.items("lifestyles", "lifestyle");
        for p in pc.child("lifestyles").into_iter().flat_map(|l| l.children_named("lifestyle")) {
            let Some(s) = lifestyles.iter().find(|s| s.get("guid") == p.get("guid")) else { continue };
            t.eq(|| format!("{fname} lifestyle {} total", s.get("name")), &p.get("totalcost"), &nuyen(lifestyle::total_cost(&ch, s)));
            t.eq(|| format!("{fname} lifestyle {} monthly", s.get("name")), &p.get("totalmonthlycost"), &nuyen(lifestyle::monthly_cost(&ch, s)));
        }

        // Reputation and karma.
        let rep = chummer_core::career::reputation_for(&engine, &ch);
        t.eq(|| format!("{fname} totalstreetcred"), &pc.get("totalstreetcred"), &rep.street_cred.to_string());
        t.eq(|| format!("{fname} totalnotoriety"), &pc.get("totalnotoriety"), &rep.notoriety.to_string());
        t.eq(|| format!("{fname} totalpublicawareness"), &pc.get("totalpublicawareness"), &rep.public_awareness.to_string());
        t.eq(|| format!("{fname} totalkarma"), &pc.get("totalkarma"), &chummer_core::career::career_karma(&ch).to_string());

        // Attributes against the engine sheet.
        for a in printed_attributes(pc) {
            let name = a.get("name_english");
            if name == "ESS" {
                continue;
            }
            t.eq(|| format!("{fname} attribute {name}"), &a.get("total"), &sheet.attr(&name).to_string());
        }
    }
    eprintln!("{} engine comparisons ({weapons} weapons, {vehicles} vehicles), {} differ", t.checked, t.bad.len());
    for b in &t.bad {
        eprintln!("  {b}");
    }
    assert!(weapons > 100 && vehicles > 10 && t.checked > 1500, "too few comparisons: {} ({weapons} weapons, {vehicles} vehicles)", t.checked);
    assert!(t.bad.is_empty(), "{} of {} printed values differ from the engine", t.bad.len(), t.checked);
}

/// Printed totals against what Chummer5a saved: attribute totals,
/// essence, karma and nuyen.
#[test]
fn printed_values_match_saved_totals() {
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let mut t = Tally::default();
    for f in fixtures() {
        let ch = Character::load(&f).unwrap();
        let fname = f.file_name().unwrap().to_string_lossy().to_string();
        let root = print::print_xml(&ch, &engine, &lang);
        let pc = root.child("character").unwrap();
        for a in printed_attributes(pc) {
            let name = a.get("name_english");
            let Some(saved) = ch.attributes.iter().find(|x| x.name == name && x.category != "Shapeshifter").and_then(|x| x.saved_total) else { continue };
            if name == "ESS" {
                continue;
            }
            t.eq(|| format!("{fname} {name}"), &a.get("total"), &saved.to_string());
        }
        if let Some(saved) = ch.doc.get_f64("totaless") {
            let printed: f64 = pc.get("totaless").parse().unwrap();
            t.checked += 1;
            // Printed with the preset's EssenceFormat (2 decimals by default).
            if (printed - saved).abs() > 0.0051 {
                t.bad.push(format!("{fname} totaless: printed {printed}, saved {saved}"));
            }
        }
        t.eq(|| format!("{fname} karma"), &pc.get("karma"), &ch.doc.get_i32("karma").unwrap_or(0).to_string());
        t.eq(|| format!("{fname} nuyen"), &pc.get("nuyen"), &nuyen(ch.doc.get_f64("nuyen").unwrap_or(0.0)));
    }
    eprintln!("{} saved-value comparisons, {} differ", t.checked, t.bad.len());
    if std::env::var_os("PRINT_ORACLE_VERBOSE").is_some() {
        for b in &t.bad {
            eprintln!("  {b}");
        }
    }
    assert!(t.checked > 400, "{}", t.checked);
    // The fixtures saved with house-rule settings we do not have (limb
    // count 5, 3-decimal essence) differ, as in tests/oracle.rs.
    for b in &t.bad {
        assert!(["Bastion", "Blindfire", "Fuzzy-chargen"].iter().any(|n| b.starts_with(n)), "unexpected difference: {b}");
    }
}

/// LB-07: top-level gear prints its own cost (Chummer prints 1 / CostFor).
#[test]
fn top_level_gear_prints_its_own_cost() {
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let mut checked = 0;
    for f in fixtures() {
        let ch = Character::load(&f).unwrap();
        let root = print::print_xml(&ch, &engine, &lang);
        let printed = all(&root, "gear");
        for g in ch.items("gears", "gear") {
            let plain = g.child("children").is_none_or(|c| c.elements().next().is_none());
            if !plain || g.get_f64("qty").unwrap_or(1.0) != 1.0 || g.get_f64("costfor").unwrap_or(1.0) != 1.0 {
                continue;
            }
            let Some(p) = printed.iter().find(|p| p.get("guid") == g.get("guid")) else { continue };
            assert_eq!(p.get("owncost"), nuyen(gear::cost(g)), "{} {}", f.display(), g.get("name"));
            checked += 1;
        }
    }
    assert!(checked > 50, "{checked}");
}

/// Spells print the display strings and the recalculated DV.
#[test]
fn spells_translate_codes() {
    let engine = Engine::load().unwrap();
    let dir = data::resource_dir("lang").unwrap();
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Soma (Career).chum5");
    let ch = Character::load(&f).unwrap();
    let en = print::print_xml(&ch, &engine, &Language::load(&dir, "en-us"));
    let de = print::print_xml(&ch, &engine, &Language::load(&dir, "de-de"));
    let de_lang = Language::load(&dir, "de-de");
    let spells = |r: &Element| r.child("character").unwrap().child("spells").unwrap().children_named("spell").cloned().collect::<Vec<_>>();
    let (en, de) = (spells(&en), spells(&de));
    assert!(!en.is_empty());
    for (e, d) in en.iter().zip(&de) {
        // English twins are the same in both languages.
        for k in ["type_english", "range_english", "duration_english", "dv_english", "damage_english", "descriptors_english"] {
            assert_eq!(e.get(k), d.get(k), "{} {k}", e.get("name"));
        }
        let saved = ch.items("spells", "spell").into_iter().find(|s| s.get("guid") == e.get("guid")).unwrap();
        let dur = match saved.get("duration").as_str() {
            "I" => "String_SpellDurationInstant",
            "S" => "String_SpellDurationSustained",
            "P" => "String_SpellDurationPermanent",
            _ => "String_SpellDurationSpecial",
        };
        assert_eq!(d.get("duration"), de_lang.s(dur), "{}", e.get("name"));
        assert_eq!(d.get("type"), de_lang.s(if saved.get("type") == "M" { "String_SpellTypeMana" } else { "String_SpellTypePhysical" }));
        if saved.get("dv").starts_with('F') {
            assert!(d.get("dv").starts_with(&de_lang.s("String_SpellForce")), "{}", d.get("dv"));
        }
        if matches!(saved.get("damage").as_str(), "P" | "S") {
            assert_eq!(e.get("damage"), format!("0{}", saved.get("damage")));
        }
    }
}

/// `Spell.CalculatedDv` sends a non-numeric DV such as `Special` through
/// the XPath evaluator, which rejects the letters, so Chummer appends the
/// failed expression: `Special(Special)`.
#[test]
fn special_dv_follows_chummer() {
    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let mut seen = 0;
    for f in fixtures() {
        let ch = Character::load(&f).unwrap();
        let saved = ch.items("spells", "spell");
        if !saved.iter().any(|s| s.get("dv") == "Special") {
            continue;
        }
        let root = print::print_xml(&ch, &engine, &lang);
        for p in all(&root, "spell") {
            let s = saved.iter().find(|s| s.get("guid") == p.get("guid")).unwrap();
            if s.get("dv") == "Special" && !s.get_bool("limited").unwrap_or(false) {
                seen += 1;
                assert_eq!(p.get("dv"), "Special(Special)");
            }
        }
    }
    assert!(seen > 0);
}
