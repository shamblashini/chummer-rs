//! Armor, armor mods, weapons and accessories: data-included children,
//! adding to a character, derived combat values and cost.

use std::path::PathBuf;

use chummer_core::bonus::{self, BonusSource};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::{self, armor, weapon, Purchase};
use chummer_core::xml::{self, Element};

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn named<'a>(ch: &'a Character, container: &str, item: &'a str, name: &str) -> &'a Element {
    ch.items(container, item).into_iter().find(|e| e.get("name") == name).unwrap()
}

fn by_guid<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if e.get("guid") == guid {
        return Some(e);
    }
    e.elements().find_map(|c| by_guid(c, guid))
}

#[test]
fn armor_comes_with_its_mods() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("armor.xml").unwrap();
    let rec = data::find(&doc, "armors", "armor", "Vashon Island: Sleeping Tiger").unwrap();
    let a = armor::armor_element(rec, "g", 0, "", None);
    assert_eq!(a.get("armor"), "13");
    assert_eq!(a.get("maxrating"), "");
    assert!(a.child("wirelessbonus").unwrap().child("limitmodifier").is_some());
    let mods = armor::included_mod_elements(&doc, rec);
    let names: Vec<String> = mods.iter().map(|m| m.get("name")).collect();
    assert_eq!(names, ["Custom Fit", "Newest Model", "Ruthenium Polymer Coating"]);
    for m in &mods {
        assert_eq!(m.get("included"), "True");
        assert_eq!(m.get("armorcapacity"), "[0]");
        assert_eq!(m.get("cost"), "0");
    }
    // `<name rating="3">`: rated mod, its max rating fixed at that rating.
    assert_eq!(mods[2].get("rating"), "3");
    assert_eq!(mods[2].get("maxrating"), "3");
    assert_eq!(mods[0].get("rating"), "0");
}

#[test]
fn armormod_rating_is_clamped_and_costed() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("armor.xml").unwrap();
    let rec = data::find(&doc, "mods", "mod", "Ruthenium Polymer Coating").unwrap();
    let m = armor::armormod_element(rec, "g", 9, "", None);
    assert_eq!(m.get("rating"), "4", "clamped to maxrating 4");
    assert_eq!(m.get("included"), "False");
    assert_eq!(armor::cost(&m), 20000.0);
}

#[test]
fn add_armor_then_a_mod() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Barrett.chum5");
    let doc = engine.store.doc("armor.xml").unwrap();
    let before = ch.items("armors", "armor").len();
    let rec = data::find(&doc, "armors", "armor", "Armor Jacket").unwrap();
    let guid = items::add("armor", &mut ch, &engine.store, rec, &Purchase::default()).unwrap();
    assert_eq!(ch.items("armors", "armor").len(), before + 1);
    let m = data::find(&doc, "mods", "mod", "Fire Resistance").unwrap();
    let p = Purchase { rating: 3, parent: Some(guid.clone()), ..Default::default() };
    let mg = items::add("armormod", &mut ch, &engine.store, m, &p).unwrap();
    let jacket = by_guid(&ch.doc, &guid).unwrap();
    let saved = by_guid(jacket, &mg).unwrap();
    assert_eq!(saved.get("rating"), "3");
    // Fire Resistance: Rating × 250¥.
    assert_eq!(armor::cost(jacket), 1000.0 + 750.0);
    // A mod needs an armor.
    assert!(items::add("armormod", &mut ch, &engine.store, m, &Purchase::default()).is_err());
}

#[test]
fn armor_bonus_becomes_improvements() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Barrett.chum5");
    let doc = engine.store.doc("armor.xml").unwrap();
    let rec = data::records(&doc, "armors", "armor").into_iter().find(|r| r.el().path("bonus/limitmodifier").is_some()).unwrap();
    let n = ch.improvements.list.len();
    let guid = items::add("armor", &mut ch, &engine.store, rec, &Purchase::default()).unwrap();
    assert!(ch.improvements.list.len() > n);
    assert!(ch.improvements.list.iter().any(|i| i.source_name == guid && i.source == "Armor"));
}

#[test]
fn weapon_comes_with_accessories_and_underbarrel() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("weapons.xml").unwrap();
    let rec = data::find(&doc, "weapons", "weapon", "Ares Alpha").unwrap();
    let spec = weapon::WeaponSpec { guid: "alpha".into(), ..Default::default() };
    let w = weapon::weapon_tree(&doc, rec, spec);
    assert_eq!(w.get("weaponslots"), "Stock/Side/Barrel/Top/Under");
    assert_eq!(w.get("weapontype"), "gun");
    let acc: Vec<&Element> = w.child("accessories").unwrap().children_named("accessory").collect();
    assert_eq!(acc.len(), 1);
    assert_eq!(acc[0].get("name"), "Smartgun System, Internal");
    assert_eq!(acc[0].get("mount"), "Internal");
    assert_eq!(acc[0].get("extramount"), "None");
    assert_eq!(acc[0].get("included"), "True");
    let ub = w.child("underbarrel").unwrap().child("weapon").unwrap();
    assert_eq!(ub.get("name"), "Ares Alpha Grenade Launcher");
    assert_eq!(ub.get("cost"), "0");
    assert_eq!(ub.get("included"), "True");
    assert_eq!(ub.get("parentid"), "alpha");
    // The included smartgun and the underbarrel are free.
    assert_eq!(weapon::cost(&w), 2650.0);
}

#[test]
fn throwing_weapons_hold_one() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("weapons.xml").unwrap();
    for rec in data::records(&doc, "weapons", "weapon").into_iter().filter(|r| r.get("useskill") == "Throwing Weapons") {
        let w = weapon::weapon_element(&doc, rec, weapon::WeaponSpec::default());
        assert_eq!(w.get("ammo"), "1", "{}", rec.name());
    }
}

#[test]
fn add_weapon_and_accessory_with_mount() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Barrett.chum5");
    let doc = engine.store.doc("weapons.xml").unwrap();
    let rec = data::find(&doc, "weapons", "weapon", "Ares Predator V").unwrap();
    let guid = items::add("weapon", &mut ch, &engine.store, rec, &Purchase::default()).unwrap();
    let silencer = data::find(&doc, "accessories", "accessory", "Silencer/Suppressor").unwrap();
    let p = Purchase { parent: Some(guid.clone()), ..Default::default() };
    let choices = items::choices("accessory", &ch, &engine.store, silencer, &p);
    assert!(choices.is_empty(), "only the barrel mount fits: {choices:?}");
    let ag = items::add("accessory", &mut ch, &engine.store, silencer, &p).unwrap();
    let w = by_guid(&ch.doc, &guid).unwrap();
    let a = by_guid(w, &ag).unwrap();
    assert_eq!(a.get("mount"), "Barrel");
    assert_eq!(a.get("included"), "False");
    assert_eq!(weapon::cost(w), 725.0 + 500.0);
    // The barrel is now taken.
    assert!(weapon::mount_options(w, silencer).is_empty());
    let p = Purchase { parent: Some(guid.clone()), ..Default::default() };
    assert!(items::add("accessory", &mut ch, &engine.store, silencer, &p).is_err(), "no second barrel accessory");
    // An accessory needs a weapon.
    assert!(items::add("accessory", &mut ch, &engine.store, silencer, &Purchase::default()).is_err());
}

#[test]
fn stats_of_a_fixture_rifle() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Fuzzy-chargen.chum5");
    let sheet = engine.sheet(&ch);
    let w = named(&ch, "weapons", "weapon", "Ares Alpha");
    let s = weapon::stats(&ch, &sheet, w);
    assert_eq!(s.damage, "11P");
    assert_eq!(s.ap, "-2");
    assert_eq!(s.skill, "Automatics");
    // 5 + smartgun 2 (best of the non-stacking) + personalized grip 1.
    assert_eq!(s.accuracy, 8);
    // 2 + gas vent 3 + foregrip 1 + STR 2 / 3 ⇒ 1, + 1; the folding stock is deployable.
    assert_eq!(s.rc, "8 (9)");
    let automatics = sheet.skills.iter().find(|k| k.name == "Automatics").unwrap();
    // Automatics + the Assault Rifles specialization; the smartgun is not
    // wireless-on in this save, so no Smartlink dice.
    assert_eq!(automatics.specs, ["Assault Rifles"]);
    assert_eq!(s.dice_pool, automatics.pool + automatics.spec_bonus);
    assert_eq!(s.ranges.short, "0-25");
    assert_eq!(s.ranges.extreme, "351-550");
    // 2650 + grip 100 + stock 30 + gas vent 600 + sling 15 + foregrip 100 + custom look 300.
    assert_eq!(weapon::cost(w), 3795.0);
}

#[test]
fn wireless_smartgun_adds_smartlink_dice() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Fuzzy-chargen.chum5");
    let sheet = engine.sheet(&ch);
    let mut w = named(&ch, "weapons", "weapon", "Ares Alpha").clone();
    let before = weapon::stats(&ch, &sheet, &w).dice_pool;
    for a in w.child_mut("accessories").unwrap().elements_mut() {
        if a.get("name").starts_with("Smartgun") {
            a.set_child_text("wirelesson", "True");
        }
    }
    ch.improvements.list.push(chummer_core::improvement::Improvement { kind: "Smartlink".into(), val: 2.0, enabled: true, rating: 1, ..Default::default() });
    assert_eq!(weapon::stats(&ch, &sheet, &w).dice_pool, before + 2);
    // A weapon switched off wirelessly gets none.
    w.set_child_text("wirelesson", "False");
    assert_eq!(weapon::stats(&ch, &sheet, &w).dice_pool, before);
}

#[test]
fn weapon_specific_bonuses_go_to_their_own_stat() {
    // LB-11: Chummer adds WeaponSpecificDV/AP/Accuracy/Range to the pool.
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Fuzzy-chargen.chum5");
    let sheet = engine.sheet(&ch);
    let w = named(&ch, "weapons", "weapon", "Ares Alpha").clone();
    let before = weapon::stats(&ch, &sheet, &w);
    let guid = w.get("guid");
    for kind in ["WeaponSpecificDV", "WeaponSpecificAP", "WeaponSpecificAccuracy", "WeaponSpecificRange"] {
        let val = if kind == "WeaponSpecificRange" { 100.0 } else { 1.0 };
        ch.improvements.list.push(chummer_core::improvement::Improvement { kind: kind.into(), improved_name: guid.clone(), val, enabled: true, rating: 1, ..Default::default() });
    }
    let after = weapon::stats(&ch, &sheet, &w);
    assert_eq!(after.dice_pool, before.dice_pool);
    assert_eq!((before.damage.as_str(), after.damage.as_str()), ("11P", "12P"));
    assert_eq!((before.ap.as_str(), after.ap.as_str()), ("-2", "-1"));
    assert_eq!(after.accuracy, before.accuracy + 1);
    assert_ne!(after.ranges, before.ranges);
    ch.improvements.list.push(chummer_core::improvement::Improvement { kind: "WeaponSpecificDice".into(), improved_name: guid, val: 2.0, enabled: true, rating: 1, ..Default::default() });
    assert_eq!(weapon::stats(&ch, &sheet, &w).dice_pool, before.dice_pool + 2);
}

#[test]
fn melee_damage_uses_strength() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Apex Predator.chum5");
    let sheet = engine.sheet(&ch);
    let str_ = sheet.attr("STR");
    let knife = named(&ch, "weapons", "weapon", "Knife (Survival Kit)");
    // The save's legacy "(STR+1)P" is read from the data, as Weapon.Load does.
    assert_eq!(weapon::stats(&ch, &sheet, knife).damage, format!("{}P", str_ + 1));
    let unarmed = named(&ch, "weapons", "weapon", "Unarmed Attack");
    let s = weapon::stats(&ch, &sheet, unarmed);
    assert_eq!(s.damage, format!("{str_}S"));
    assert_eq!(s.accuracy, sheet.limit_physical, "Physical accuracy");
    assert_eq!(s.ranges, weapon::Ranges::default());
}

#[test]
fn natural_weapon_bonus_adds_a_weapon() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Barrett.chum5");
    let node = xml::parse("<bonus><naturalweapon><name>Claws</name><damage>({STR}+1)P</damage><ap>-1</ap><useskill>Unarmed Combat</useskill></naturalweapon></bonus>").unwrap();
    let src = BonusSource { kind: "Quality".into(), guid: "q-guid".into(), name: "Some Quality".into(), rating: 1 };
    let out = bonus::apply(&ch, &engine.store, &node, &src, None);
    assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);
    let (container, w) = &out.added[0];
    assert_eq!(container, "weapons");
    assert_eq!(w.get("name"), "Claws");
    assert_eq!(w.get("category"), "Critter Powers");
    assert_eq!(w.get("type"), "Melee");
    assert_eq!(w.get("accuracy"), "Physical");
    assert_eq!(w.get("parentid"), "q-guid");
    let imp = &out.improvements[0];
    assert_eq!(imp.kind, "Weapon");
    assert_eq!(imp.improved_name, w.get("guid"));

    let sheet = engine.sheet(&ch);
    let s = weapon::stats(&ch, &sheet, w);
    assert_eq!(s.damage, format!("{}P", sheet.attr("STR") + 1));
    assert_eq!(s.ap, "-1");
    assert_eq!(s.skill, "Unarmed Combat");
}

#[test]
fn add_weapon_bonus_is_free_unless_full_cost() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Barrett.chum5");
    let src = BonusSource { kind: "Quality".into(), guid: "q".into(), name: "Q".into(), rating: 1 };
    let node = xml::parse("<bonus><addweapon><name>Ares Predator V</name></addweapon></bonus>").unwrap();
    let out = bonus::apply(&ch, &engine.store, &node, &src, None);
    let (_, w) = out.added.last().unwrap();
    assert_eq!(w.get("cost"), "0");
    assert_eq!(w.get("parentid"), "q");
    assert_eq!(out.improvements[0].kind, "Weapon");
    let node = xml::parse("<bonus><addweapon><name>Ares Predator V</name><fullcost /></addweapon></bonus>").unwrap();
    let out = bonus::apply(&ch, &engine.store, &node, &src, None);
    assert_eq!(out.added.last().unwrap().1.get("cost"), "725");
}

/// High-Fashion Armor Clothing (Run & Gun p. 58): a piece like the
/// Argentum Coat (Armor 12, "+4") is an armor of its own value; its +4 adds
/// only over the armor its Custom Fit (Stack) mod is fitted to, chosen when
/// it is bought, capped at STR like other stacking armor. It used to count
/// as a +4 accessory always: worn alone, min(4, STR) (2 at STR 2).
#[test]
fn custom_fit_stack_armor_counts_alone_and_over_its_fitted_armor() {
    let engine = Engine::load().unwrap();
    let doc = engine.store.doc("armor.xml").unwrap();
    let mut ch = fixture("Soma (Career).chum5");
    for a in ch.items_mut("armors").elements_mut().filter(|e| e.name == "armor") {
        a.set_child_text("equipped", "False");
    }
    let strength = 2;
    assert_eq!(engine.sheet(&ch).attributes.iter().find(|a| a.name == "STR").map(|a| a.total), Some(strength));

    // Buying it asks which armor it is fitted to.
    let coat = data::find(&doc, "armors", "armor", "Mortimer of London: Argentum Coat").unwrap();
    let asks = items::choices("armor", &ch, &engine.store, coat, &Purchase::default());
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert!(asks[0].options.iter().any(|o| o == "Mortimer of London: Berwick Suit"), "{:?}", asks[0].options);
    assert!(!asks[0].options.iter().any(|o| o == "Armor Jacket"), "only armors that can be custom fit");

    let p = Purchase { answer: Some("Mortimer of London: Berwick Suit".into()), ..Purchase::default() };
    let cg = items::add("armor", &mut ch, &engine.store, coat, &p).unwrap();
    let fit = ch.items("armors", "armor").into_iter().find(|a| a.get("guid") == cg).unwrap().child("armormods").unwrap().children_named("armormod").find(|m| m.get("name") == "Custom Fit (Stack)").unwrap().get("extra");
    assert_eq!(fit, "Mortimer of London: Berwick Suit");
    let set = |ch: &mut Character, guid: &str, on: bool| {
        for a in ch.items_mut("armors").elements_mut().filter(|e| e.name == "armor") {
            if a.get("guid") == guid {
                a.set_child_text("equipped", if on { "True" } else { "False" });
            }
        }
    };
    set(&mut ch, &cg, true);
    assert_eq!(engine.sheet(&ch).armor, 12, "worn alone: its own 12");

    // Over another armor it is not fitted to: the higher of the two.
    let jacket = data::find(&doc, "armors", "armor", "Armor Jacket").unwrap();
    let jg = items::add("armor", &mut ch, &engine.store, jacket, &Purchase::default()).unwrap();
    set(&mut ch, &jg, true);
    assert_eq!(engine.sheet(&ch).armor, 12);
    set(&mut ch, &jg, false);

    // Over the suit it is fitted to: 9 + 4, capped at STR 2 = 11, so its
    // own 12 is still the best; at STR 6 the suit with it is 13.
    let suit = data::find(&doc, "armors", "armor", "Mortimer of London: Berwick Suit").unwrap();
    let sg = items::add("armor", &mut ch, &engine.store, suit, &Purchase::default()).unwrap();
    set(&mut ch, &sg, true);
    assert_eq!(engine.sheet(&ch).armor, 12);
    let mut rules = engine.rules_for(&ch);
    rules.uncapped_armor_accessory_bonuses = true;
    assert_eq!(chummer_core::calc::compute(&ch, &rules, Some(&engine.store), None).armor, 13, "uncapped: 9 + 4");
}
