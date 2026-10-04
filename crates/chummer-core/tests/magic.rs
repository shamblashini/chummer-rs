//! Magic and resonance: adding items, bonus hooks, accounting, summary.

use std::path::PathBuf;

use chummer_core::bonus::{self, BonusSource};
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::items::{self, magic, Purchase};
use chummer_core::xml;

fn fixture(name: &str) -> Character {
    Character::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn src(kind: &str) -> BonusSource {
    BonusSource { kind: kind.into(), guid: "11111111-2222-3333-4444-555555555555".into(), name: "Test".into(), rating: 1 }
}

fn bonus_node(s: &str) -> xml::Element {
    xml::parse(s).unwrap()
}

#[test]
fn add_spell_and_rebuild() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Glessner.chum5");
    let before = ch.items("spells", "spell").len();
    let counted = |ch: &Character| {
        let c = magic::spell_counts(ch, &engine.sheet(ch));
        c.spells + c.rituals + c.preparations
    };
    let counted_before = counted(&ch);
    let doc = engine.store.doc("spells.xml").unwrap();
    let rec = data::find(&doc, "spells", "spell", "Increase [Attribute]").unwrap();
    let p = Purchase { answer: Some("AGI".into()), ..Default::default() };
    let guid = items::add("spell", &mut ch, &engine.store, rec, &p).unwrap();
    let spells = ch.items("spells", "spell");
    assert_eq!(spells.len(), before + 1);
    let s = spells.iter().find(|s| s.get("guid") == guid).unwrap();
    assert_eq!(s.get("extra"), "AGI");
    assert_eq!(s.get("descriptors"), "Essence");
    assert_eq!(s.get("improvementsource"), "Spell");
    // The oracle rebuild reproduces what `add` wrote.
    let rebuilt = items::rebuild("spell", &ch, &engine.store, s).unwrap();
    assert_eq!(rebuilt.to_xml_string(), s.to_xml_string());
    // It counts against the free spells.
    assert_eq!(counted(&ch), counted_before + 1);
}

#[test]
fn power_points_and_adding_a_power() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Skink.chum5");
    let (total, used) = magic::power_points(&ch, &engine.sheet(&ch));
    assert_eq!((total, used), (6.0, 6.0));
    let doc = engine.store.doc("powers.xml").unwrap();
    let rec = data::find(&doc, "powers", "power", "Improved Reflexes").unwrap();
    let p = Purchase { rating: 2, ..Default::default() };
    let guid = items::add("power", &mut ch, &engine.store, rec, &p).unwrap();
    let (_, used2) = magic::power_points(&ch, &engine.sheet(&ch));
    // 2 levels at 1 PP plus 0.5 extra point cost.
    assert_eq!(used2 - used, 2.5);
    assert!(ch.improvements.list.iter().any(|i| i.source_name == guid && i.source == "Power" && i.kind == "Attribute"));
}

#[test]
fn specificpower_grants_free_levels() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Skink.chum5");
    let b = bonus_node("<bonus><specificpower><name>Light Body</name><val>2</val></specificpower></bonus>");
    let out = bonus::apply(&ch, &engine.store, &b, &src("MentorSpirit"), None);
    assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);
    let i = out.improvements.iter().find(|i| i.kind == "AdeptPowerFreeLevels").unwrap();
    assert_eq!((i.improved_name.as_str(), i.rating, i.unique_name.as_str()), ("Light Body", 2, ""));
    // Skink does not have Light Body: the bonus adds it with rating 0.
    let (c, p) = out.added.iter().find(|(c, _)| c == "powers").unwrap();
    assert_eq!((c.as_str(), p.get("name"), p.get("rating")), ("powers", "Light Body".into(), "0".into()));
    // A mundane gets nothing.
    let mundane = fixture("Barrett.chum5");
    let out = bonus::apply(&mundane, &engine.store, &b, &src("Quality"), None);
    assert!(out.improvements.is_empty() && out.added.is_empty() && out.unsupported.is_empty());
}

#[test]
fn add_magic_hooks() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Glessner.chum5");
    let b = bonus_node("<bonus><addspell alchemical=\"True\">Fireball</addspell><addmetamagic>Centering</addmetamagic></bonus>");
    let out = bonus::apply(&ch, &engine.store, &b, &src("Quality"), None);
    assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);
    let spell = &out.added.iter().find(|(c, _)| c == "spells").unwrap().1;
    assert_eq!((spell.get("grade"), spell.get("alchemical")), ("-1".into(), "True".into()));
    let link = out.improvements.iter().find(|i| i.kind == "Spell").unwrap();
    assert_eq!(link.improved_name, spell.get("guid"));
    assert_eq!(link.source, "Quality");
    assert!(out.improvements.iter().any(|i| i.kind == "Metamagic"));
}

#[test]
fn critter_power_and_spirit_hooks() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Glessner.chum5");
    let b = bonus_node("<bonus><critterpowers><power select=\"Fire\">Elemental Attack</power></critterpowers></bonus>");
    let out = bonus::apply(&ch, &engine.store, &b, &src("Metatype"), None);
    assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);
    let p = &out.added.iter().find(|(c, _)| c == "critterpowers").unwrap().1;
    assert_eq!(p.get("extra"), "Fire");
    assert_eq!(out.improvements.iter().find(|i| i.kind == "CritterPower").unwrap().improved_name, p.get("guid"));

    let b = bonus_node("<bonus><addspirit><spirit>Spirit of Air</spirit><spirit>Spirit of Fire</spirit></addspirit></bonus>");
    let none = bonus::apply(&ch, &engine.store, &b, &src("Quality"), None);
    assert_eq!(none.unsupported, vec!["addspirit".to_owned()], "two options need an answer");
    let c = bonus::choices(&ch, &engine.store, &b, &src("Quality"));
    assert_eq!(c[0].options, vec!["Spirit of Air".to_owned(), "Spirit of Fire".to_owned()]);
    let out = bonus::apply(&ch, &engine.store, &b, &src("Quality"), Some("Spirit of Fire"));
    let i = &out.improvements[0];
    assert_eq!((i.kind.as_str(), i.improved_name.as_str()), ("AddSpirit", "Spirit of Fire"));
}

#[test]
fn mentor_with_choices() {
    let engine = Engine::load().unwrap();
    let mut ch = fixture("Glessner.chum5");
    ch.improvements.list.clear();
    let doc = engine.store.doc("mentors.xml").unwrap();
    let rec = data::find(&doc, "mentors", "mentor", "Raven (Alt)").unwrap();
    let choice = magic::mentor::choice_names(rec).into_iter().find(|c| c.contains("summoning")).unwrap();
    let guid = magic::add_mentor(&mut ch, &engine.store, "MentorSpirit", "Raven (Alt)", Some(&choice), None, None).unwrap();
    let kinds: Vec<&str> = ch.improvements.list.iter().filter(|i| i.source_name == guid).map(|i| i.kind.as_str()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "SkillCategory").count(), 5);
    assert!(kinds.contains(&"Skill"), "choice bonus applied: {kinds:?}");
    let m = ch.items("mentorspirits", "mentorspirit").into_iter().find(|m| m.get("guid") == guid).unwrap().clone();
    assert_eq!(m.get("extrachoice1"), choice);
    let rebuilt = items::rebuild("mentorspirit", &ch, &engine.store, &m).unwrap();
    assert_eq!(rebuilt.to_xml_string(), m.to_xml_string());
}

#[test]
fn initiation_and_martial_arts() {
    let engine = Engine::load().unwrap();
    let ch = fixture("Munin_Career.chum5");
    let rules = engine.rules_for(&ch);
    let settings = engine.settings.resolve(&ch.field("settings")).unwrap();
    let plain = magic::initiation::GradeOptions::default();
    assert_eq!(magic::initiation_karma(&rules, settings, 1, false, plain), 13);
    let o = magic::initiation::GradeOptions { ordeal: true, schooling: true, ..plain };
    assert_eq!(magic::initiation_karma(&rules, settings, 2, false, o), 13, "16 x 0.8 = 12.8");

    let mut ch = fixture("Barrett.chum5");
    let doc = engine.store.doc("martialarts.xml").unwrap();
    let rec = data::find(&doc, "martialarts", "martialart", "Tae Kwon Do").unwrap();
    let p = Purchase { answer: Some("Counterstrike".into()), ..Default::default() };
    let guid = items::add("martialart", &mut ch, &engine.store, rec, &p).unwrap();
    let art = ch.items("martialarts", "martialart").into_iter().find(|a| a.get("guid") == guid).unwrap().clone();
    assert_eq!(art.child("martialarttechniques").unwrap().elements().count(), 1);
    assert_eq!(items::rebuild("martialart", &ch, &engine.store, &art).unwrap().to_xml_string(), art.to_xml_string());
}

#[test]
fn summary_reads_both_tradition_layouts() {
    let engine = Engine::load().unwrap();
    // Old layout: <tradition>Cosmic</tradition> + <traditiondrain>.
    let ch = fixture("Glessner.chum5");
    let sheet = engine.sheet(&ch);
    let m = magic::magic_summary(&ch, &sheet);
    assert_eq!((m.tradition.as_str(), m.drain_expression.as_str()), ("Cosmic", "{WIL} + {LOG}"));
    assert_eq!(m.drain_pool, sheet.attr("WIL") + sheet.attr("LOG"));
    // Current layout, technomancer.
    let ch = fixture("Bastion.chum5");
    let sheet = engine.sheet(&ch);
    let m = magic::magic_summary(&ch, &sheet);
    assert_eq!(m.fading_expression, "{WIL} + {RES}");
    assert_eq!(m.fading_pool, sheet.attr("WIL") + sheet.attr("RES"));
    assert_eq!(m.complex_forms, (3, 2));
    // Adepts resist drain with BOD + WIL.
    let ch = fixture("Skink.chum5");
    let m = magic::magic_summary(&ch, &engine.sheet(&ch));
    assert_eq!(m.drain_expression, "{BOD} + {WIL}");
}
