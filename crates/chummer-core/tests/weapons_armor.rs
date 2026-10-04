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
    assert!(s.dice_pool >= automatics.pool, "{} < {}", s.dice_pool, automatics.pool);
    assert_eq!(s.ranges.short, "0-25");
    assert_eq!(s.ranges.extreme, "351-550");
    // 2650 + grip 100 + stock 30 + gas vent 600 + sling 15 + foregrip 100 + custom look 300.
    assert_eq!(weapon::cost(w), 3795.0);
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
