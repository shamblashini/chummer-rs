//! `.chumrs` round trips: every fixture `.chum5` (and the `.chum5lz` from
//! Chummer) saved as `.chumrs` and back as `.chum5` is the same character
//! (equal `state_hash`), mugshots exactly as they were; history and guide
//! state survive; damage and newer versions fail with a clear message.

mod common;

use std::path::{Path, PathBuf};

use chummer_core::campaign::{Campaign, Member, MemberKind};
use chummer_core::character::Character;
use chummer_core::chumrs::{self, Extras, GuideState, HistoryItem};
use chummer_core::command;
use chummer_core::container;

fn every_character_file() -> Vec<PathBuf> {
    let mut v = common::fixtures();
    v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chum5lz/fixer-chummer.chum5lz"));
    v
}

fn mugshot_texts(ch: &Character) -> Vec<String> {
    ch.doc.child("mugshots").map(|m| m.children_named("mugshot").map(|e| e.text()).collect()).unwrap_or_default()
}

#[test]
fn every_fixture_round_trips_through_chumrs() {
    let dir = common::temp_dir("chumrs-roundtrip");
    let mut with_shots = 0;
    for f in every_character_file() {
        let name = common::fixture_name(&f);
        let mut orig = Character::load(&f).unwrap();
        let hash = command::state_hash(&orig);
        let shots = mugshot_texts(&orig);

        let rs = dir.join(format!("{name}.chumrs"));
        orig.save(&rs).unwrap();
        let bytes = std::fs::read(&rs).unwrap();
        assert!(container::is_container(&bytes), "{name}");
        let back = Character::load(&rs).unwrap();
        assert_eq!(command::state_hash(&back), hash, "{name}: .chum5 -> .chumrs");
        assert_eq!(mugshot_texts(&back), shots, "{name}: mugshots");

        // The image entries are the decoded bytes, not base64.
        if !shots.is_empty() && shots.iter().any(|s| !s.is_empty()) {
            with_shots += 1;
            let xml_size = std::fs::metadata(&f).unwrap().len();
            assert!((bytes.len() as u64) < xml_size, "{name}: {} >= {xml_size}", bytes.len());
            let a = container::decode(&chumrs::KIND, &bytes).unwrap();
            let entry = a.entries.keys().find(|k| k.starts_with("mugshots/")).expect("an image entry").clone();
            let inner = a.text(chumrs::CHARACTER).unwrap().unwrap();
            assert!(inner.contains(&format!("entry=\"{entry}\"")), "{name}");
            assert!(!inner.contains(&shots[0]), "{name}: base64 left in the XML");
        }

        let mut back = back;
        let c5 = dir.join(format!("{name}.chum5"));
        back.save(&c5).unwrap();
        let again = Character::load(&c5).unwrap();
        assert_eq!(command::state_hash(&again), hash, "{name}: .chumrs -> .chum5");
        assert_eq!(mugshot_texts(&again), shots, "{name}: mugshots after export");
        let lz = dir.join(format!("{name}.chum5lz"));
        back.save(&lz).unwrap();
        assert_eq!(command::state_hash(&Character::load(&lz).unwrap()), hash, "{name}: .chumrs -> .chum5lz");
    }
    assert!(with_shots >= 10, "{with_shots} fixtures with mugshots");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn engine_save_keeps_essence_for_listings() {
    let dir = common::temp_dir("chumrs-engine");
    let f = common::fixtures_dir().join("Munin_Career.chum5");
    let mut ch = Character::load(&f).unwrap();
    let hash = command::state_hash(&ch);
    let rs = dir.join("munin.chumrs");
    common::engine().save(&mut ch, &rs).unwrap();
    let e = chummer_core::roster::summarize(&rs);
    assert!(e.error.is_none(), "{:?}", e.error);
    assert_eq!(e.essence, ch.doc.get("totaless"));
    assert!(!e.essence.is_empty());
    assert_eq!(command::state_hash(&Character::load(&rs).unwrap()), hash);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extras_survive_and_are_optional() {
    let dir = common::temp_dir("chumrs-extras");
    let ch = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let extras = Extras {
        history: (0..3).map(|i| HistoryItem { at: 1_700_000_000_000 + i, author: if i == 1 { "GM".into() } else { String::new() }, description: format!("Change {i}") }).collect(),
        guide: Some(GuideState { step: "attributes".into(), visited: vec!["concept".into(), "metatype".into()] }),
        created: Some("2026-01-02T03:04:05Z".into()),
    };
    let p = dir.join("davis.chumrs");
    chumrs::write(&p, &ch, &extras).unwrap();
    let (back, ex) = chumrs::load_any(&p).unwrap();
    assert_eq!(ex, extras);
    assert_eq!(back.file.as_deref(), Some(p.as_path()));
    let a = container::decode(&chumrs::KIND, &std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(a.manifest.created, "2026-01-02T03:04:05Z");
    assert_eq!(a.manifest.format, chumrs::FORMAT);
    assert_eq!(a.manifest.schema_version, chumrs::SCHEMA_VERSION);
    assert!(!a.manifest.modified.is_empty());

    // Without extras there are no entries for them.
    chumrs::write(&p, &ch, &Extras::default()).unwrap();
    let a = container::decode(&chumrs::KIND, &std::fs::read(&p).unwrap()).unwrap();
    assert!(a.get(chumrs::HISTORY).is_none() && a.get(chumrs::GUIDE).is_none());
    assert_eq!(chumrs::load_any(&p).unwrap().1.history, vec![]);

    // A .chum5 has none.
    assert_eq!(chumrs::load_any(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap().1, Extras::default());

    // A damaged extra is dropped, the character still loads.
    let mut a = container::decode(&chumrs::KIND, &chumrs::to_bytes(&ch, &extras)).unwrap();
    a.put(chumrs::HISTORY, b"not json".to_vec());
    let (_, ex) = chumrs::from_bytes(&container::encode(&chumrs::KIND, &a)).unwrap();
    assert!(ex.history.is_empty());
    assert_eq!(ex.guide, extras.guide);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn damage_and_newer_versions_fail_clearly() {
    let ch = Character::load(&common::fixtures_dir().join("Barrett.chum5")).unwrap();
    let bytes = chumrs::to_bytes(&ch, &Extras::default());

    // A changed image (stored, so its bytes are in the file as they are).
    let a = container::decode(&chumrs::KIND, &bytes).unwrap();
    let (name, img) = a.entries.iter().find(|(k, _)| k.starts_with("mugshots/")).unwrap();
    let at = bytes.windows(32).position(|w| w == &img[100..132]).unwrap() + 5;
    let mut bad = bytes.clone();
    bad[at] ^= 0x40;
    let e = chumrs::from_bytes(&bad).unwrap_err().to_string();
    assert!(e.contains("damaged"), "{name}: {e}");

    // A manifest whose hash does not match (the archive itself intact).
    let a = container::decode(&chumrs::KIND, &bytes).unwrap();
    let mut m = a.manifest.clone();
    m.entries.get_mut(chumrs::CHARACTER).unwrap().blake3 = "0".repeat(64);
    let e = chumrs::from_bytes(&raw_zip(&m, &a)).unwrap_err().to_string();
    assert!(e.contains("checksum"), "{e}");
    // An entry the manifest lists but the archive lacks.
    let mut short = a.clone();
    short.entries.remove(name);
    let e = chumrs::from_bytes(&raw_zip(&a.manifest, &short)).unwrap_err().to_string();
    assert!(e.contains("missing"), "{e}");

    // A newer schema version.
    let mut m2 = container::decode(&chumrs::KIND, &bytes).unwrap().manifest;
    m2.schema_version = chumrs::SCHEMA_VERSION + 1;
    m2.app_version = "9.9.9".into();
    let e = chumrs::from_bytes(&raw_zip(&m2, &a)).unwrap_err().to_string();
    assert!(e.contains("newer chummer-rs") && e.contains("9.9.9"), "{e}");

    // A campaign is not a character, and the other way round.
    let c = Campaign::new("Seattle");
    let e = chumrs::from_bytes(&c.to_bytes()).unwrap_err().to_string();
    assert!(e.contains("campaign"), "{e}");
    assert!(Campaign::from_bytes(&bytes).is_err());
}

/// A ZIP with `manifest` as given (no recomputed hashes) and `a`'s
/// entries.
fn raw_zip(manifest: &container::Manifest, a: &container::Archive) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    w.start_file(container::MANIFEST, o).unwrap();
    w.write_all(&serde_json::to_vec(manifest).unwrap()).unwrap();
    for (n, b) in &a.entries {
        w.start_file(n, o).unwrap();
        w.write_all(b).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn campaign_files_use_the_container() {
    let dir = common::temp_dir("chumrs-campaign");
    let mut c = Campaign::new("Seattle");
    let mut hashes = Vec::new();
    for f in ["Barrett.chum5", "Ghile Mear.chum5"] {
        let ch = Character::load(&common::fixtures_dir().join(f)).unwrap();
        hashes.push(command::state_hash(&ch));
        c.add(Member::embedded(MemberKind::Npc, &ch));
    }
    // A linked .chumrs member.
    let mut linked = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let lp = dir.join("davis.chumrs");
    linked.save(&lp).unwrap();
    hashes.push(command::state_hash(&linked));
    c.add(Member::linked(MemberKind::Player, Path::new("davis.chumrs"), &linked));

    let p = dir.join("seattle.chummercampaign");
    c.save(&p).unwrap();
    let bytes = std::fs::read(&p).unwrap();
    let a = container::decode(&chummer_core::campaign::KIND, &bytes).unwrap();
    let id = c.members[0].id.to_string();
    assert!(a.get(&format!("members/{id}.xml")).is_some());
    assert!(a.entries.keys().any(|k| k.starts_with(&format!("members/{id}/mugshots/"))), "{:?}", a.entries.keys().collect::<Vec<_>>());
    let json = a.text(chummer_core::campaign::CAMPAIGN_ENTRY).unwrap().unwrap();
    assert!(!json.contains("<character"), "characters are their own entries");

    let back = Campaign::load(&p).unwrap();
    assert_eq!(back, c);
    for (m, h) in back.members.iter().zip(&hashes) {
        assert_eq!(&command::state_hash(&m.load_character(Some(&dir)).unwrap()), h, "{}", m.name);
    }

    // The older single-LZMA-stream file still loads.
    let legacy = dir.join("old.chummercampaign");
    std::fs::write(&legacy, chummer_core::chum5lz::compress(c.to_json().as_bytes()).unwrap()).unwrap();
    assert_eq!(Campaign::load(&legacy).unwrap(), c);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn chummer_files_are_still_written_as_chummer_files() {
    let dir = common::temp_dir("chumrs-export");
    let mut ch = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let p = dir.join("davis.chum5");
    ch.save(&p).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("<?xml"), "plain XML");
    let lz = dir.join("davis.chum5lz");
    ch.save(&lz).unwrap();
    assert_eq!(std::fs::read(&lz).unwrap()[0], 0x5D, "LZMA");
    // Content decides how a file is read, not its name.
    let renamed = dir.join("davis-really-chumrs.chum5");
    std::fs::write(&renamed, chumrs::to_bytes(&ch, &Extras::default())).unwrap();
    assert_eq!(command::state_hash(&Character::load(&renamed).unwrap()), command::state_hash(&ch));
    std::fs::remove_dir_all(&dir).unwrap();
}
