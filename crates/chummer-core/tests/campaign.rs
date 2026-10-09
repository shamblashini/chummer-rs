//! Campaigns: the file, members, copies, the feed and GM awards.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chummer_core::campaign::{self, Campaign, Combatant, Encounter, FeedCursor, Member, MemberCharacter, MemberKind};
use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command, Session};
use chummer_core::engine::Engine;
use chummer_core::xml::Element;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.chum5"))
}

fn load(name: &str) -> Character {
    Character::load(&fixture(name)).unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-rs-campaign-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn texts_of(e: &Element, name: &str, out: &mut Vec<String>) {
    for c in e.elements() {
        if c.name == name && !c.text().trim().is_empty() {
            out.push(c.text().trim().to_owned());
        }
        texts_of(c, name, out);
    }
}

fn values(ch: &Character, name: &str) -> Vec<String> {
    let mut v = Vec::new();
    texts_of(&ch.to_document(), name, &mut v);
    v
}

#[test]
fn file_round_trip_keeps_hashes() {
    let dir = tmp("roundtrip");
    let mut c = Campaign::new("Seattle Nights");
    c.gm_notes = "Act 1: the Renraku job".into();
    let mut hashes = Vec::new();
    for (name, kind) in [("Munin_Career", MemberKind::Player), ("Barrett", MemberKind::Npc), ("Apex Predator", MemberKind::Critter)] {
        let ch = load(name);
        hashes.push(command::state_hash(&ch));
        let mut m = Member::embedded(kind, &ch);
        m.player = "Anna".into();
        m.group = "Runners".into();
        c.add(m);
    }
    let mut enc = Encounter::new("Docks");
    enc.combatants.push(Combatant::for_member(c.members[1].id, "Barrett"));
    enc.combatants.push(Combatant::ad_hoc("Guard", 8, 1, 10, 10));
    c.encounters.push(enc);
    let path = dir.join("seattle.chummercampaign");
    c.save(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_ne!(bytes.first(), Some(&b'{'), "the file is compressed");

    let back = Campaign::load(&path).unwrap();
    assert_eq!(back, c);
    assert_eq!(back.id.to_string().len(), 32);
    for (m, h) in back.members.iter().zip(&hashes) {
        let ch = m.load_character(Some(&dir)).unwrap();
        assert_eq!(&command::state_hash(&ch), h, "{}", m.name);
    }
    // Plain JSON loads too, and so do files with fields we do not know.
    let mut json: serde_json::Value = serde_json::from_str(&c.to_json()).unwrap();
    json["added_later"] = serde_json::json!({"anything": [1, 2]});
    json["members"][0]["new_field"] = serde_json::json!(true);
    let back = Campaign::from_bytes(json.to_string().as_bytes()).unwrap();
    assert_eq!(back.members.len(), 3);
    assert!(matches!(Campaign::from_bytes(b"garbage"), Err(campaign::CampaignError::NotACampaign)));
}

#[test]
fn copies_get_fresh_guids() {
    let Ok(engine) = Engine::load() else { return };
    let ch = load("Barrett");
    let mut c = Campaign::new("x");
    let mut template = Member::embedded(MemberKind::Enemy, &ch);
    template.name = "Halloweener Ganger".into();
    template.group = "Halloweeners".into();
    c.add(template.clone());
    let ids = c.add_copies(&engine, &template, &ch, 4);
    assert_eq!(ids.len(), 4);
    let orig: HashSet<String> = values(&ch, "guid").into_iter().collect();
    let mut all = orig.clone();
    for (n, id) in ids.iter().enumerate() {
        let m = c.member(*id).unwrap();
        // The template, named just "Halloweener Ganger", counts as 1.
        assert_eq!(m.name, format!("Halloweener Ganger {}", n + 2));
        assert_eq!(m.group, "Halloweeners");
        assert_eq!(m.kind, MemberKind::Enemy);
        let copy = m.load_character(None).unwrap();
        assert_eq!(copy.display_name(), m.name);
        let guids = values(&copy, "guid");
        assert_eq!(guids.len(), orig.len());
        for g in &guids {
            assert!(all.insert(g.clone()), "guid {g} reused");
        }
        // Data ids are untouched; links point at the copy's own items.
        assert_eq!(values(&copy, "sourceid"), values(&ch, "sourceid"));
        assert_eq!(values(&copy, "suid"), values(&ch, "suid"));
        let own: HashSet<String> = guids.into_iter().collect();
        for p in values(&copy, "parentid") {
            assert!(!orig.contains(&p), "parent link {p} still points at the original");
            let _ = own.contains(&p);
        }
        for s in values(&copy, "sourcename") {
            assert!(!orig.contains(&s.to_ascii_lowercase()), "improvement source {s} still points at the original");
        }
        assert_eq!(engine.sheet(&copy).essence, engine.sheet(&ch).essence);
    }
    // Copying a copy continues the numbers.
    let fifth = c.member(ids[3]).unwrap().clone();
    let more = c.add_copies(&engine, &fifth, &fifth.load_character(None).unwrap(), 1);
    assert_eq!(c.member(more[0]).unwrap().name, "Halloweener Ganger 6");
}

#[test]
fn copies_keep_loaded_ammunition_linked() {
    use chummer_core::play::ammo;
    let Ok(engine) = Engine::load() else { return };
    // A career character with a weapon and ammunition for it: load it.
    let mut found = None;
    for name in ["Munin_Career", "Soma (Career)", "Barrett", "Gangerbean", "Blindfire", "Draught"] {
        let ch = load(name);
        let store = engine.store_for_character(&ch);
        for w in ch.items("weapons", "weapon") {
            let g = w.get("guid");
            if let Some((ammo, _, _)) = ammo::reloadable(&ch, Some(&store), &g).into_iter().next() {
                found = Some((ch.clone(), g, ammo));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some((ch, weapon, ammo_guid)) = found else { panic!("no fixture with reloadable ammunition") };
    let mut s = Session::with_seed(ch, 1);
    s.apply(&engine, Command::Reload { weapon: weapon.clone(), ammo: Some(ammo_guid.clone()), count: 1 }).unwrap();
    let loaded = s.ch().clone();
    let w = chummer_core::items::edit::find(&loaded, &weapon).expect("weapon");
    assert!(ammo::loaded(&loaded, w).is_some(), "the weapon is loaded");
    let copy = campaign::fresh_copy(&loaded);
    let orig: HashSet<String> = values(&loaded, "guid").into_iter().collect();
    for g in values(&copy, "guid") {
        assert!(!orig.contains(&g), "guid {g} shared with the original");
    }
    for id in values(&copy, "id") {
        assert!(!orig.contains(&id.to_ascii_lowercase()), "clip id {id} still points at the original's ammunition");
    }
    let at = loaded.items("weapons", "weapon").iter().position(|x| x.get("guid") == weapon).expect("top-level weapon");
    let new_weapon = copy.items("weapons", "weapon")[at];
    let ammo_name = ammo::loaded(&loaded, w).unwrap().get("name");
    assert_eq!(ammo::loaded(&copy, new_weapon).map(|g| g.get("name")), Some(ammo_name), "the copy's weapon is loaded with the copy's ammunition");
}

#[test]
fn gm_awards_reach_the_feed() {
    let Ok(engine) = Engine::load() else { return };
    let ch = load("Munin_Career");
    let karma = ch.karma;
    let mut c = Campaign::new("x");
    let id = c.add(Member::embedded(MemberKind::Player, &ch));
    let name = c.member(id).unwrap().name.clone();
    let mut s = Session::with_seed(ch, 9).with_author("GM");
    let mut cursor = FeedCursor::default();
    let award = Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount: 100.0, reason: "Great run".into(), ..Default::default() } };
    s.apply(&engine, award).unwrap();
    assert_eq!(s.ch().karma, karma + 100);
    c.absorb(id, s.log(), &mut cursor);
    let last = c.log.last().unwrap();
    assert_eq!(last.description, format!("GM gave {name} 100 karma: Great run"));
    assert_eq!((last.author.as_str(), last.member), ("GM", Some(id)));
    let nuyen = Command::ManualExpense { karma: false, gain: true, expense: ManualExpense { amount: 2500.0, reason: String::new(), ..Default::default() } };
    s.apply(&engine, nuyen).unwrap();
    c.absorb(id, s.log(), &mut cursor);
    assert_eq!(c.log.last().unwrap().description, format!("GM gave {name} 2500¥"));
    // Nothing new: nothing added.
    let n = c.log.len();
    c.absorb(id, s.log(), &mut cursor);
    assert_eq!(c.log.len(), n);
    // Undo shows in the feed.
    s.undo().unwrap();
    c.absorb(id, s.log(), &mut cursor);
    assert_eq!(c.log.last().unwrap().description, format!("Undone: GM gave {name} 2500¥"));
    // A quick custom improvement, as "the GM allows it".
    let form = chummer_core::custom_improvement::Form { type_id: "specificattribute".into(), name: "GM: blessing".into(), val: 1.0, select: "BOD".into(), ..Default::default() };
    s.apply(&engine, Command::CreateImprovement { form, group: "GM".into(), edit: None }).unwrap();
    c.absorb(id, s.log(), &mut cursor);
    assert_eq!(c.log.last().unwrap().description, "Added improvement GM: blessing");
    // The member's stored character follows the session.
    c.member_mut(id).unwrap().store_character(s.ch());
    let back = c.member(id).unwrap().load_character(None).unwrap();
    assert_eq!(command::state_hash(&back), s.state_hash());
}

#[test]
fn merged_edits_are_one_feed_line() {
    let Ok(engine) = Engine::load() else { return };
    let mut c = Campaign::new("x");
    let ch = load("Barrett");
    let id = c.add(Member::embedded(MemberKind::Npc, &ch));
    fn clock() -> i64 {
        1_700_000_000_000
    }
    let mut s = Session::with_seed(ch, 3).with_clock(clock);
    let mut cursor = FeedCursor::default();
    let n = c.log.len();
    for v in ["G", "Gh", "Gho"] {
        s.apply(&engine, Command::SetField { key: "alias".into(), value: v.into() }).unwrap();
        c.absorb(id, s.log(), &mut cursor);
    }
    assert_eq!(s.log().len(), 1);
    assert_eq!(c.log.len(), n + 1);
    assert!(c.log.last().unwrap().description.contains("Gho"), "{}", c.log.last().unwrap().description);
}

#[test]
fn linked_members_save_to_their_file() {
    let Ok(engine) = Engine::load() else { return };
    let dir = tmp("linked");
    let file = dir.join("ghost.chum5");
    std::fs::copy(fixture("Munin_Career"), &file).unwrap();
    let ch = Character::load(&file).unwrap();
    let mut c = Campaign::new("x");
    let mut m = Member::linked(MemberKind::Player, Path::new("ghost.chum5"), &ch);
    m.player = "Ben".into();
    let id = c.add(m);
    let path = dir.join("c.chummercampaign");
    c.save(&path).unwrap();

    let back = Campaign::load(&path).unwrap();
    let m = back.member(id).unwrap();
    assert!(matches!(&m.character, MemberCharacter::Linked { path } if path == Path::new("ghost.chum5")));
    assert_eq!(m.linked_path(Some(&dir)).unwrap(), file);
    let ch = m.load_character(Some(&dir)).unwrap();
    let mut s = Session::new(ch);
    s.apply(&engine, Command::SetPhysicalDamage { filled: 3 }).unwrap();
    s.save(&engine, &m.linked_path(Some(&dir)).unwrap()).unwrap();
    let again = back.member(id).unwrap().load_character(Some(&dir)).unwrap();
    assert_eq!(again.physical_cm_filled, 3);
    assert_eq!(command::state_hash(&again), s.state_hash());
}

#[test]
fn npc_from_a_kit() {
    let Ok(engine) = Engine::load() else { return };
    let doc = chummer_core::gm::packs::load(&engine.store, None);
    let kit = chummer_core::gm::packs::find_kit(&doc, "Intro Runner Pack", "Core Packs").unwrap();
    let ch = campaign::kit_npc(&engine, "Human", &kit.to_xml_string(), "Street Sam").unwrap();
    assert_eq!(ch.display_name(), "Street Sam");
    assert!(!ch.created);
    assert!(!ch.items("gears", "gear").is_empty() || !ch.items("weapons", "weapon").is_empty());
}
