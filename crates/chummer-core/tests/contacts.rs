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

    // Compressed saves resolve and read like plain ones.
    let lz = dir.join("Lz.chum5lz");
    std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/chum5lz/fixer-chummer.chum5lz"), &lz).unwrap();
    contacts::set_field(&mut ch, &guid, "file", &lz.to_string_lossy());
    let c = contacts::of_type(&ch, ContactType::Contact).into_iter().find(|c| c.get("guid") == guid).unwrap();
    assert_eq!(contacts::resolve(c, Path::new("/opt"), None), Some(LinkedPath::Found(lz.clone())));
    let l = LinkedCharacter::load(&lz).unwrap();
    assert_eq!((l.name.as_str(), l.metatype.as_str()), ("Lz Fixer", "Elf"));
    // A broken compressed file is an error, not a panic.
    std::fs::write(dir.join("Bad.chum5lz"), b"\x00").unwrap();
    assert!(LinkedCharacter::load(&dir.join("Bad.chum5lz")).is_err());
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

fn guids(ch: &Character, kind: ContactType) -> Vec<String> {
    contacts::of_type(ch, kind).iter().map(|c| c.get("guid")).collect()
}

#[test]
fn reordering_is_saved_as_element_order() {
    let dir = scratch("order");
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let start = guids(&ch, ContactType::Contact);
    let a = contacts::add(&mut ch, ContactType::Contact);
    let enemy = contacts::add(&mut ch, ContactType::Enemy);
    let b = contacts::add(&mut ch, ContactType::Contact);
    let c = contacts::add(&mut ch, ContactType::Contact);
    let mut want = start.clone();
    want.extend([a.clone(), b.clone(), c.clone()]);
    assert_eq!(guids(&ch, ContactType::Contact), want);

    // Drag c onto a (dropped above it), then a below b; up/down buttons.
    assert!(contacts::move_contact(&mut ch, &c, &a, false));
    assert!(contacts::move_contact(&mut ch, &a, &b, true));
    assert!(!contacts::move_contact(&mut ch, &a, &b, true), "already there");
    assert!(!contacts::move_contact(&mut ch, &a, &a, false));
    let mut want = start.clone();
    want.extend([c.clone(), b.clone(), a.clone()]);
    assert_eq!(guids(&ch, ContactType::Contact), want);
    assert!(contacts::move_step(&mut ch, &a, true));
    assert!(!contacts::move_step(&mut ch, &b, false), "b is last now");
    let mut want = start.clone();
    want.extend([c.clone(), a.clone(), b.clone()]);
    assert_eq!(guids(&ch, ContactType::Contact), want);
    // Steps stay within one type: the enemy never moves among contacts.
    assert!(!contacts::move_step(&mut ch, &enemy, true));
    assert!(!contacts::move_step(&mut ch, &enemy, false));

    let order = guids(&ch, ContactType::Contact);
    let path = dir.join("ordered.chum5");
    ch.dirty = false;
    let first = order[0].clone();
    assert!(!contacts::move_step(&mut ch, &first, true));
    assert!(!ch.dirty, "a no-op move leaves the character clean");
    ch.save(&path).unwrap();
    let again = Character::load(&path).unwrap();
    assert_eq!(guids(&again, ContactType::Contact), order);
    assert_eq!(guids(&again, ContactType::Enemy), [enemy]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn notes_colour_round_trips_like_color_translator() {
    let dir = scratch("colour");
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let g = contacts::add(&mut ch, ContactType::Contact);
    let find = |ch: &Character| contacts::of_type(ch, ContactType::Contact).into_iter().find(|c| c.get("guid") == g).unwrap().clone();
    assert_eq!(find(&ch).get("notesColor"), "Chocolate");
    assert_eq!(contacts::notes_color(&find(&ch)), [0xD2, 0x69, 0x1E]);
    assert!(contacts::preferred_color(&find(&ch)).is_none());
    // Picking the colour it already has changes nothing.
    assert!(!contacts::set_notes_color(&mut ch, &g, [0xD2, 0x69, 0x1E]));
    assert!(contacts::set_notes_color(&mut ch, &g, [0x12, 0xAB, 0x0F]));
    contacts::set_field(&mut ch, &g, "colour", "-65536");
    let path = dir.join("c.chum5");
    ch.save(&path).unwrap();
    let again = Character::load(&path).unwrap();
    let c = find(&again);
    assert_eq!(c.get("notesColor"), "#12AB0F");
    assert_eq!(contacts::notes_color(&c), [0x12, 0xAB, 0x0F]);
    assert_eq!(contacts::preferred_color(&c), Some([0xFF, 0xFF, 0, 0]));
    // Unreadable text falls back to the default, as Chummer's default color.
    let mut ch2 = again.clone();
    contacts::set_field(&mut ch2, &g, "notesColor", "nonsense");
    assert_eq!(contacts::notes_color(&find(&ch2)), [0xD2, 0x69, 0x1E]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn printed_contact_uses_the_linked_character() {
    use chummer_core::{data, engine::Engine, lang::Language, print};
    let dir = scratch("print");
    let linked = dir.join("Glessner.chum5lz");
    // A compressed link, to cover .chum5lz on the way.
    Character::load(&fixture("Glessner.chum5")).unwrap().save(&linked).unwrap();
    let mut ch = Character::load(&fixture("Barrett.chum5")).unwrap();
    let g = contacts::add(&mut ch, ContactType::Contact);
    for (k, v) in [("name", "Own Name"), ("metatype", "Dwarf"), ("gender", "Female ♀"), ("age", "Young"), ("role", "Fixer")] {
        contacts::set_field(&mut ch, &g, k, v);
    }
    let mut unlinked = ch.clone();
    assert!(contacts::link(&mut ch, &g, &linked, Path::new("/opt/chummer-rs")));
    let owner = dir.join("owner.chum5");
    ch.save(&owner).unwrap();

    let engine = Engine::load().unwrap();
    let lang = Language::load(&data::resource_dir("lang").unwrap(), "en-us");
    let printed = |ch: &Character| -> chummer_core::xml::Element {
        let root = print::print_xml(ch, &engine, &lang);
        let c = root.child("character").unwrap().child("contacts").unwrap().children_named("contact").find(|c| c.get("guid") == g).unwrap().clone();
        c
    };
    let p = printed(&Character::load(&owner).unwrap());
    assert_eq!(p.get("name"), "Glessner");
    assert_eq!(p.get("metatype"), "Ork (Satyr)");
    assert_eq!(p.get("gender"), "Male ♂");
    let glessner = Character::load(&fixture("Glessner.chum5")).unwrap();
    assert_eq!(p.get("age"), glessner.field("age"));
    // The contact's own fields still print where Chummer uses them.
    assert_eq!(p.get("role"), "Fixer");
    // Mugshots are the linked character's.
    let main = contacts::main_mugshot(&glessner.doc).unwrap();
    assert_eq!(p.get("mainmugshotbase64"), main);
    assert!(p.child("othermugshots").is_some());

    // Unlinked: the contact's own fields and no mugshots.
    unlinked.file = Some(owner.clone());
    let p = printed(&unlinked);
    assert_eq!((p.get("name").as_str(), p.get("metatype").as_str(), p.get("gender").as_str(), p.get("age").as_str()), ("Own Name", "Dwarf", "Female ♀", "Young"));
    assert!(p.child("mainmugshotbase64").is_none());
    std::fs::remove_dir_all(dir).ok();
}
