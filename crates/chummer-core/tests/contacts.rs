//! Contacts, enemies, pets and linked characters (Chummer's `Contact`).

use std::path::{Path, PathBuf};

use chummer_core::character::Character;
use chummer_core::contacts::{self, ContactType, LinkedCharacter, LinkedPath};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-rs-contacts-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn add_pet_writes_chummer_elements() {
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let before = ch.items("contacts", "contact").len();
    let guid = contacts::add(&mut ch, ContactType::Pet);
    assert_eq!(ch.items("contacts", "contact").len(), before + 1);
    let pets = contacts::of_type(&ch, ContactType::Pet);
    assert_eq!(pets.len(), 1);
    let p = pets[0];
    assert_eq!(p.get("type"), "Pet");
    assert_eq!(p.get("guid"), guid);
    // Element order of Contact.Save (5.226).
    let names: Vec<&str> = p.elements().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "name", "role", "location", "connection", "loyalty", "metatype", "gender", "age", "contacttype", "preferredpayment", "hobbiesvice",
            "personallife", "type", "file", "relative", "notes", "notesColor", "groupname", "colour", "group", "family", "blackmail", "free",
            "groupenabled", "guid", "mainmugshotindex", "mugshots"
        ]
    );
    // Pets are neither contacts nor enemies.
    assert!(contacts::of_type(&ch, ContactType::Contact).iter().all(|c| c.get("guid") != guid));
    assert!(contacts::set_field(&mut ch, &guid, "name", "Fluffy"));
    assert!(contacts::set_field(&mut ch, &guid, "metatype", "Hellhound"));
    assert!(!contacts::set_field(&mut ch, &guid, "metatype", "Hellhound"));
    assert!(contacts::remove(&mut ch, &guid));
    assert!(contacts::of_type(&ch, ContactType::Pet).is_empty());
}

#[test]
fn linked_character_name_metatype_and_mugshot() {
    let path = fixture("Glessner.chum5");
    let l = LinkedCharacter::load(&path).unwrap();
    assert_eq!(l.name, "Glessner");
    assert_eq!(l.metatype, "Ork");
    assert_eq!(l.display_metatype(), "Ork (Satyr)");
    assert_eq!(l.gender, "Male ♂");
    let png = contacts::decode_base64(&l.mugshot.unwrap()).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn link_round_trips_and_resolves() {
    let dir = scratch("link");
    let pet_file = dir.join("Glessner.chum5");
    std::fs::copy(fixture("Glessner.chum5"), &pet_file).unwrap();
    let startup = Path::new("/opt/chummer-rs");

    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let guid = contacts::add(&mut ch, ContactType::Pet);
    assert!(contacts::link(&mut ch, &guid, &pet_file, startup));
    let saved = dir.join("owner.chum5");
    ch.save(&saved).unwrap();

    let again = Character::load(&saved).unwrap();
    let pet = contacts::of_type(&again, ContactType::Pet)[0];
    assert_eq!(pet.get("file"), pet_file.to_string_lossy());
    assert_eq!(pet.get("relative"), contacts::relative_uri(startup, &pet_file));
    assert!(pet.get("relative").starts_with("../"));
    assert!(contacts::is_linked(pet));
    assert_eq!(contacts::resolve(pet, startup, Some(&saved)), Some(LinkedPath::Found(pet_file.clone())));
    // The contact's own fields stay as they were; the linked ones are shown.
    assert_eq!(pet.get("name"), "");
    let LinkedPath::Found(p) = contacts::resolve(pet, startup, Some(&saved)).unwrap() else { unreachable!() };
    assert_eq!(LinkedCharacter::load(&p).unwrap().name, "Glessner");

    // The relative path is used when the absolute one is gone.
    let mut ch2 = again.clone();
    let g = pet.get("guid");
    contacts::set_field(&mut ch2, &g, "file", "/nowhere/Glessner.chum5");
    let rel_startup = dir.join("bin");
    contacts::set_field(&mut ch2, &g, "relative", &contacts::relative_uri(&rel_startup, &pet_file));
    let pet2 = contacts::of_type(&ch2, ContactType::Pet)[0].clone();
    assert_eq!(contacts::resolve(&pet2, &rel_startup, None), Some(LinkedPath::Found(pet_file.clone())));

    assert!(contacts::unlink(&mut ch2, &g));
    let pet3 = contacts::of_type(&ch2, ContactType::Pet)[0];
    assert_eq!((pet3.get("file"), pet3.get("relative")), (String::new(), String::new()));
    assert_eq!(contacts::resolve(pet3, startup, None), None);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_linked_file_is_not_an_error() {
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let guid = contacts::add(&mut ch, ContactType::Contact);
    let windows = r"C:\Users\someone\Documents\Chummer\Fixer.chum5";
    contacts::set_field(&mut ch, &guid, "file", windows);
    contacts::set_field(&mut ch, &guid, "relative", "../Documents/Chummer/Fixer.chum5");
    let c = contacts::of_type(&ch, ContactType::Contact).into_iter().find(|c| c.get("guid") == guid).unwrap();
    assert_eq!(contacts::resolve(c, Path::new("/opt/chummer-rs"), None), Some(LinkedPath::Missing(windows.into())));
    assert!(LinkedCharacter::load(Path::new("/nowhere/Fixer.chum5")).is_err());

    // A Windows link finds the file next to the owner's save.
    let dir = scratch("windows");
    std::fs::copy(fixture("Glessner.chum5"), dir.join("Fixer.chum5")).unwrap();
    let owner = dir.join("owner.chum5");
    assert_eq!(contacts::resolve(c, Path::new("/opt/chummer-rs"), Some(&owner)), Some(LinkedPath::Found(dir.join("Fixer.chum5"))));

    // Compressed saves are found but reported as unsupported.
    std::fs::write(dir.join("Lz.chum5lz"), b"\x00").unwrap();
    contacts::set_field(&mut ch, &guid, "file", &dir.join("Lz.chum5lz").to_string_lossy());
    let c = contacts::of_type(&ch, ContactType::Contact).into_iter().find(|c| c.get("guid") == guid).unwrap();
    assert!(matches!(contacts::resolve(c, Path::new("/opt"), None), Some(LinkedPath::Unsupported(_))));
    assert!(LinkedCharacter::load(&dir.join("Lz.chum5lz")).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn enemies_and_old_files() {
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let e = contacts::add(&mut ch, ContactType::Enemy);
    assert_eq!(contacts::of_type(&ch, ContactType::Enemy).len(), 1);
    // Chummer reads any unknown <type> as an enemy.
    contacts::set_field(&mut ch, &e, "type", "Rival");
    assert_eq!(contacts::of_type(&ch, ContactType::Enemy).len(), 1);
    // Fixture contacts are plain contacts.
    assert!(!contacts::of_type(&ch, ContactType::Contact).is_empty());
}

#[test]
fn choice_lists_from_data() {
    let store = chummer_core::data::DataStore::discover().unwrap();
    let roles = contacts::choices(&store, "role");
    assert!(roles.iter().any(|r| r == "Fixer"), "{roles:?}");
    assert!(!contacts::choices(&store, "hobbiesvice").is_empty());
    let metas = contacts::metatype_choices(&store, "metatypes.xml");
    assert!(metas.iter().any(|(v, m, mv)| v == "Ork (Satyr)" && m == "Ork" && mv == "Satyr"));
    assert!(metas.iter().any(|(v, _, mv)| v == "Human" && mv.is_empty()));
    let critters = contacts::metatype_choices(&store, "critters.xml");
    assert!(critters.iter().any(|(v, _, _)| v == "Great Cat"), "{}", critters.len());
}

#[test]
fn add_contacts_from_file() {
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let before = ch.items("contacts", "contact").len();
    let src = "<chummer><contacts><contact><name>Mr. Johnson</name><role>Fixer</role><connection>4</connection><loyalty>2</loyalty><type>Contact</type></contact><contact><name>Rex</name><type>Pet</type></contact></contacts></chummer>";
    assert_eq!(contacts::import(&mut ch, src), Ok(2));
    assert_eq!(ch.items("contacts", "contact").len(), before + 2);
    let rex = contacts::of_type(&ch, ContactType::Pet)[0];
    assert_eq!(rex.get("name"), "Rex");
    assert!(!rex.get("guid").is_empty());
    assert!(contacts::import(&mut ch, "<character/>").is_err());
}
