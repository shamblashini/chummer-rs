//! GM tools: critters, PACKS kits and custom spells.

use chummer_core::calc;
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::character::Character;
use chummer_core::engine::Engine;
use chummer_core::gm::critter::{self, ForceKind, NewCritter};
use chummer_core::gm::custom_spell::{self, SpellDesign};
use chummer_core::gm::packs;

const STANDARD: &str = "223a11ff-80e0-428b-89a9-6ef1c243b8b6";

fn sheet(engine: &Engine, ch: &Character) -> calc::Sheet {
    let rules = engine.rules_for(ch);
    let store = engine.store_for_character(ch);
    calc::compute(ch, &rules, Some(&store), Some(&engine.catalog))
}

fn critter(engine: &Engine, name: &str, force: i32, picks: &[&str]) -> Character {
    let spec = NewCritter {
        settings_id: STANDARD.into(),
        metatype: name.into(),
        force,
        optional_powers: picks.iter().map(|s| s.to_string()).collect(),
        name: format!("{name} F{force}"),
        ..Default::default()
    };
    critter::create(engine, &spec).unwrap()
}

fn powers(ch: &Character) -> Vec<String> {
    ch.items("critterpowers", "critterpower").iter().map(|p| p.get("name")).collect()
}

#[test]
fn spirit_of_air_at_force_six() {
    let engine = Engine::load().unwrap();
    let opts = critter::critter_options(&engine.store, &[]);
    let air = opts.iter().find(|o| o.name == "Spirit of Air").unwrap();
    assert_eq!(air.force, ForceKind::Force { levels: false });
    assert!(air.offers_possession());
    let (count, options) = critter::optional_power_slots(&engine.store, "Spirit of Air", None, 6).unwrap();
    assert_eq!(count, 2, "floor(6 / 3) optional powers");
    assert!(options.contains(&"Fear".to_owned()));

    let ch = critter(&engine, "Spirit of Air", 6, &["Fear", "Guard"]);
    assert!(ch.created, "critters open in career mode");
    assert!(ch.flag("iscritter") && ch.flag("ignorerules"));
    assert_eq!(ch.field("metatypecategory"), "Spirits");
    assert_eq!(ch.field("metatypeid"), air.id);
    let s = sheet(&engine, &ch);
    for (a, v) in [("BOD", 4), ("AGI", 9), ("REA", 10), ("STR", 3), ("CHA", 6), ("WIL", 6), ("EDG", 3), ("MAG", 6)] {
        assert_eq!(s.attr(a), v, "{a}");
    }
    assert!(ch.mag_enabled(), "enableattribute MAG from the metatype bonus");
    let p = powers(&ch);
    for n in ["Accident", "Astral Form", "Materialization", "Fear", "Guard"] {
        assert!(p.contains(&n.to_owned()), "{n} in {p:?}");
    }
    let metatype_power = ch.items("critterpowers", "critterpower").into_iter().find(|p| p.get("name") == "Accident").unwrap();
    assert_eq!(metatype_power.get("counttowardslimit"), "False");
    let fear = ch.items("critterpowers", "critterpower").into_iter().find(|p| p.get("name") == "Fear").unwrap();
    assert_eq!(fear.get("grade"), "-1", "optional powers come from a bonus");
    // Skills at F.
    let perception = s.skills.iter().find(|x| x.name == "Perception").unwrap();
    assert_eq!(perception.rating, 6);
    assert!(ch.skills.iter().any(|x| x.specific == "Elemental Attack"), "exotic ranged weapon skill");
    // The file reloads.
    let again = Character::from_str(&ch.to_xml_string()).unwrap();
    assert_eq!(sheet(&engine, &again).attr("AGI"), 9);
    assert!(again.flag("iscritter"));
}

#[test]
fn low_force_clamps_and_possession() {
    let engine = Engine::load().unwrap();
    let spec = NewCritter {
        settings_id: STANDARD.into(),
        metatype: "Spirit of Air".into(),
        force: 1,
        possession: Some("Possession".into()),
        ..Default::default()
    };
    let ch = critter::create(&engine, &spec).unwrap();
    let s = sheet(&engine, &ch);
    assert_eq!(s.attr("STR"), 1, "F-3 is raised to 1");
    assert_eq!(s.attr("EDG"), 1, "F/2 = 0.5 rounds to 1");
    let p = powers(&ch);
    assert!(!p.contains(&"Materialization".to_owned()));
    assert!(p.contains(&"Possession".to_owned()));
    assert!(!p.iter().any(|n| n == "Fear" || n == "Guard"), "no optional powers below Force 3");
}

#[test]
fn sprites_have_no_physical_attributes_and_mundane_critters_no_force() {
    let engine = Engine::load().unwrap();
    let opts = critter::critter_options(&engine.store, &[]);
    let sprite = opts.iter().find(|o| o.category == "Sprites").unwrap();
    let ch = critter(&engine, &sprite.name, 4, &[]);
    let s = sheet(&engine, &ch);
    assert_eq!(s.attr("BOD"), 0);
    assert_eq!(s.attr("STR"), 0);
    assert!(ch.res_enabled() || ch.dep_enabled() || s.attr("LOG") > 0);

    let mundane = opts.iter().find(|o| o.category == "Mundane Critters" && o.force == ForceKind::None).unwrap();
    let ch = critter(&engine, &mundane.name, 0, &[]);
    assert!(ch.created && ch.flag("iscritter"));
    assert!(sheet(&engine, &ch).attr("BOD") >= 1);
    assert!(ch.items("weapons", "weapon").iter().any(|w| w.get("name") == "Unarmed Attack"));
}

fn new_runner(engine: &Engine) -> Character {
    let spec = NewCharacter {
        settings_id: STANDARD.into(),
        metatype: "Human".into(),
        metavariant: None,
        priorities: Priorities(['D', 'E', 'A', 'B', 'C']),
        talent: "Mundane".into(),
        talent_skills: vec![],
        name: "Kit Test".into(),
    };
    chargen::create(engine, &spec).unwrap()
}

#[test]
fn intro_runner_pack() {
    let engine = Engine::load().unwrap();
    let store = engine.store_for_character(&new_runner(&engine));
    let doc = packs::load(&store, None);
    assert!(packs::kits(&doc).len() >= 80);
    let kit = packs::find_kit(&doc, "Intro Runner Pack", "Core Packs").unwrap();
    let preview = packs::contents(kit);
    assert!(preview.iter().any(|(s, lines)| *s == "Gear" && lines.iter().any(|l| l.starts_with("Fake SIN"))));

    let mut ch = new_runner(&engine);
    let settings = engine.settings.resolve(STANDARD);
    let report = packs::apply(&mut ch, &store, settings, kit);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert!(ch.items("armors", "armor").iter().any(|a| a.get("name") == "Armor Vest"));
    let gear = ch.items("gears", "gear");
    let sin = gear.iter().find(|g| g.get("name") == "Fake SIN").unwrap();
    assert_eq!(sin.get("rating"), "1");
    assert!(gear.iter().any(|g| g.get("name") == "Meta Link"));
    assert!(gear.iter().any(|g| g.get("name").starts_with("Ammo: Regular Ammo") && g.get("extra") == "Light Pistols"));
    assert_eq!(ch.field("nuyenbp"), "2");
}

#[test]
fn every_builtin_kit_applies() {
    let engine = Engine::load().unwrap();
    let base = new_runner(&engine);
    let store = engine.store_for_character(&base);
    let doc = packs::load(&store, None);
    let settings = engine.settings.resolve(STANDARD);
    for (name, cat) in packs::kits(&doc) {
        let mut ch = base.clone();
        let report = packs::apply(&mut ch, &store, settings, packs::find_kit(&doc, &name, &cat).unwrap());
        assert!(!report.added.is_empty(), "{name}: nothing added ({:?})", report.skipped);
        Character::from_str(&ch.to_xml_string()).unwrap();
    }
}

#[test]
fn kit_round_trip_through_the_packs_folder() {
    let engine = Engine::load().unwrap();
    let mut source = new_runner(&engine);
    let store = engine.store_for_character(&source);
    let builtin = packs::load(&store, None);
    let settings = engine.settings.resolve(STANDARD);
    packs::apply(&mut source, &store, settings, packs::find_kit(&builtin, "Intro Runner Pack", "Core Packs").unwrap());
    source.attribute_mut("LOG").unwrap().base = 2;
    source.attribute_mut("CHA").unwrap().karma = 1;
    let s = sheet(&engine, &source);
    let kit = packs::from_character(&source, &s, settings, "My Kit", packs::KitParts::default());
    assert_eq!(kit.get("category"), "Custom");
    assert_eq!(kit.path("attributes/bod").unwrap().text(), "1", "value - (metatype minimum - 1)");
    assert!(kit.path("weapons").unwrap().elements().all(|w| w.get("name") != "Unarmed Attack"));

    let dir = std::env::temp_dir().join(format!("chummer-rs-packs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = packs::save(&dir, "mine", &kit, &builtin).unwrap();
    assert_eq!(path.file_name().unwrap(), "custom_mine_packs.xml");
    let merged = packs::load(&store, Some(&dir));
    assert!(packs::find_kit(&merged, "My Kit", "Custom").is_some());
    assert!(matches!(packs::save(&dir, "mine", &kit, &merged), Err(packs::SaveError::Duplicate(_))));

    let mut fresh = new_runner(&engine);
    let report = packs::apply(&mut fresh, &store, settings, packs::find_kit(&merged, "My Kit", "Custom").unwrap());
    assert!(fresh.items("armors", "armor").iter().any(|a| a.get("name") == "Armor Vest"), "{report:?}");
    assert!(fresh.items("gears", "gear").iter().any(|g| g.get("name") == "Fake SIN" && g.get("rating") == "1"));
    // LB-09: the kit's attributes are applied (Chummer 5.226 lists them only).
    assert!(report.skipped.iter().all(|s| !s.starts_with("Attribute")), "{report:?}");
    let f = sheet(&engine, &fresh);
    for n in ["BOD", "LOG", "CHA", "EDG"] {
        assert_eq!(f.attr_values(n).unwrap().value, s.attr_values(n).unwrap().value, "{n}");
    }

    assert!(packs::delete(&dir, "My Kit").unwrap());
    assert!(packs::find_kit(&packs::load(&store, Some(&dir)), "My Kit", "Custom").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kit_export_writes_each_quality_list_it_has() {
    // LB-06: Chummer drops the negative list when there are no positive
    // qualities, and writes an empty <negative/> when there are no negative.
    let engine = Engine::load().unwrap();
    let base = new_runner(&engine);
    let store = engine.store_for_character(&base);
    let settings = engine.settings.resolve(STANDARD);
    let qdoc = store.doc("qualities.xml").unwrap();
    let kit_with = |quality: &str| {
        let mut ch = base.clone();
        let rec = chummer_core::data::find(&qdoc, "qualities", "quality", quality).unwrap();
        chargen::add_quality(&mut ch, &store, rec, None);
        let s = sheet(&engine, &ch);
        packs::from_character(&ch, &s, settings, "Q", packs::KitParts::default())
    };
    let neg = kit_with("Bad Luck");
    assert!(neg.path("qualities/positive").is_none());
    assert_eq!(neg.path("qualities/negative/quality").map(|q| q.text()), Some("Bad Luck".to_owned()));
    let pos = kit_with("Ambidextrous");
    assert_eq!(pos.path("qualities/positive/quality").map(|q| q.text()), Some("Ambidextrous".to_owned()));
    assert!(pos.path("qualities/negative").is_none());
}

#[test]
fn kit_attributes_and_skills_are_applied() {
    // LB-09: Chummer 5.226 applies none of these.
    let engine = Engine::load().unwrap();
    let mut ch = new_runner(&engine);
    let store = engine.store_for_character(&ch);
    let settings = engine.settings.resolve(STANDARD);
    let kit = chummer_core::xml::parse(
        "<pack><name>T</name><category>Custom</category>\
         <attributes><bod>4</bod><agi>6</agi><rea>6</rea><str>1</str><cha>1</cha><int>3</int><log>1</log><wil>1</wil><edg>2</edg></attributes>\
         <skills><skillgroup><name>Athletics</name><rating>2</rating></skillgroup><skill><name>Pistols</name><rating>9</rating><spec>Revolvers</spec></skill></skills>\
         <knowledgeskills><skill><name>Seattle Gangs</name><rating>2</rating><category>Street</category></skill></knowledgeskills>\
         </pack>",
    )
    .unwrap();
    let report = packs::apply(&mut ch, &store, settings, &kit);
    let s = sheet(&engine, &ch);
    let value = |n: &str| s.attr_values(n).unwrap().value;
    // Kit value + (metatype minimum - 1): a human's minimum is 1, Edge's 2.
    assert_eq!((value("BOD"), value("AGI"), value("INT"), value("EDG")), (4, 6, 3, 3));
    // Only one attribute at the maximum (SR5 p. 66).
    assert_eq!(value("REA"), 5);
    assert!(report.skipped.iter().any(|r| r.starts_with("Attribute: REA lowered")), "{report:?}");
    let pistols = s.skills.iter().find(|x| x.name == "Pistols").unwrap();
    assert_eq!(pistols.total_base, 6, "capped at the creation maximum");
    assert_eq!(pistols.specs, vec!["Revolvers".to_owned()]);
    assert_eq!(ch.skill_groups.iter().find(|g| g.name == "Athletics").unwrap().rating(), 2);
    let gangs = ch.knowledge_skills.iter().find(|k| k.name == "Seattle Gangs").unwrap();
    assert_eq!((gangs.kind.as_str(), gangs.base + gangs.karma), ("Street", 2));
    // Creation points first, the rest with karma: no point pool overspent.
    let rules = engine.rules_for(&ch);
    let b = chargen::budget(&ch, &s, &rules, settings.unwrap());
    for (what, p) in [("attributes", b.attribute_points), ("special", b.special_points), ("skills", b.skill_points), ("groups", b.skill_group_points), ("knowledge", b.knowledge_points)] {
        assert!(p.1 <= p.0, "{what}: {p:?}");
    }
}

#[test]
fn file_names_follow_chummer() {
    assert_eq!(packs::normalize_file_name("foo"), "custom_foo_packs.xml");
    assert_eq!(packs::normalize_file_name("custom_foo_packs.xml"), "custom_foo_packs.xml");
    assert_eq!(packs::normalize_file_name("custom_bar"), "custom_bar_packs.xml");
}

fn combat_design() -> SpellDesign {
    let mut d = SpellDesign { name: "Zap".into(), ..Default::default() };
    custom_spell::set_category(&mut d, "Combat");
    d
}

#[test]
fn custom_spell_drain_and_descriptors() {
    let mut d = combat_design();
    assert!(!custom_spell::problems(&d).is_empty(), "needs direct/indirect and damage type");
    custom_spell::set_modifier(&mut d, 0, true); // Direct
    custom_spell::set_modifier(&mut d, 3, true); // Physical damage
    d.kind = "M".into();
    d.range = "LOS".into();
    assert!(custom_spell::problems(&d).is_empty());
    assert_eq!(custom_spell::drain(&d), "(F/2)");
    assert_eq!(custom_spell::descriptors(&d), "Direct");
    // Direct disables Indirect and element effects.
    custom_spell::set_modifier(&mut d, 1, true);
    assert!(!d.mods[1]);

    // Indirect elemental area stun: Indirect forces physical.
    let mut d = combat_design();
    custom_spell::set_modifier(&mut d, 2, true); // Element effects -> Indirect
    assert!(d.mods[1] && d.kind == "P" && d.kind_locked);
    custom_spell::set_modifier(&mut d, 4, true); // Stun
    d.effects = 2;
    d.area = true;
    d.range = "LOS".into();
    // +1 physical, +2 area, +2×2 element effects, -1 stun
    assert_eq!(custom_spell::drain(&d), "(F/2)+6");
    assert_eq!(custom_spell::descriptors(&d), "Indirect, Elemental", "Chummer never adds Area for combat spells");
    let e = custom_spell::element(&d, "g");
    assert_eq!(e.get("range"), "LOS(A)");
    assert_eq!(e.get("damage"), "S");
    assert_eq!(e.get("source"), "SM");
    assert_eq!(e.get("page"), "159");
    assert_eq!(e.get("sourceid"), "00000000-0000-0000-0000-000000000000");

    // Curative health spells use the damage value and ignore Permanent.
    let mut h = SpellDesign { name: "Mend".into(), ..Default::default() };
    custom_spell::set_category(&mut h, "Health");
    custom_spell::set_modifier(&mut h, 0, true);
    h.duration = "P".into();
    h.kind = "M".into();
    assert_eq!(custom_spell::drain(&h), "(Damage Value)-2");
    assert_eq!(custom_spell::element(&h, "g").get("damage"), "");

    // Detection: extended area implies area.
    let mut det = SpellDesign { name: "Seek".into(), ..Default::default() };
    custom_spell::set_category(&mut det, "Detection");
    custom_spell::set_modifier(&mut det, 0, true); // Directional
    custom_spell::set_modifier(&mut det, 13, true); // Extended Area
    assert!(det.mods[1] && !det.mods[0]);
    custom_spell::set_modifier(&mut det, 3, true); // Active
    assert_eq!(custom_spell::descriptors(&det), "Active, Extended Area");
    det.restricted = true;
    assert!(custom_spell::problems(&det).iter().any(|p| p.contains("restricted")));
}

#[test]
fn custom_spell_on_a_character() {
    let engine = Engine::load().unwrap();
    let mut d = combat_design();
    custom_spell::set_modifier(&mut d, 0, true);
    custom_spell::set_modifier(&mut d, 3, true);
    let mut ch = new_runner(&engine);
    let g = custom_spell::add(&mut ch, &engine, &d).unwrap();
    let saved = ch.items("spells", "spell").into_iter().find(|s| s.get("guid") == g).unwrap().clone();
    assert_eq!(saved.get("dv"), "(F/2)-1");
    assert_eq!(saved.get("descriptors"), "Direct");
    // Career mode pays spell karma.
    ch.created = true;
    ch.karma = 2;
    assert!(custom_spell::add(&mut ch, &engine, &d).is_err(), "not enough karma");
    assert_eq!(ch.items("spells", "spell").len(), 1);
    ch.karma = 20;
    custom_spell::add(&mut ch, &engine, &d).unwrap();
    assert_eq!(ch.karma, 15);
}

