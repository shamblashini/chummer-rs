//! Contacts, enemies and pets (Chummer's `Contact` class), and the
//! characters they can be linked to.
//!
//! All three live in `<contacts>/<contact>` and differ by `<type>`
//! (`Contact.EntityType`). Any of them can be linked to another `.chum5`
//! (`Contact.FileName` / `RelativeFileName`); while the linked file loads,
//! Chummer shows its name, metatype, gender, age and mugshots in place of
//! the contact's own, but still saves the contact's own fields.

use std::path::{Component, Path, PathBuf};

use crate::character::Character;
use crate::data::DataStore;
use crate::items::new_guid;
use crate::xml::Element;

/// `ContactType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactType {
    Contact,
    Enemy,
    Pet,
}

impl ContactType {
    /// `Contact.ConvertToContactType`: empty is a contact, and anything
    /// that is neither "Contact" nor "Pet" is an enemy.
    pub fn parse(s: &str) -> ContactType {
        if s.is_empty() || s.eq_ignore_ascii_case("contact") {
            ContactType::Contact
        } else if s.eq_ignore_ascii_case("pet") {
            ContactType::Pet
        } else {
            ContactType::Enemy
        }
    }

    pub fn of(c: &Element) -> ContactType {
        ContactType::parse(&c.get("type"))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ContactType::Contact => "Contact",
            ContactType::Enemy => "Enemy",
            ContactType::Pet => "Pet",
        }
    }
}

/// A new `<contact>` as `Contact.Save` writes it in 5.226 (a fresh
/// `Contact`: connection and loyalty 1, group enabled).
pub fn new_element(kind: ContactType, name: &str, role: &str, connection: i32, loyalty: i32) -> Element {
    let mut c = Element::new("contact");
    let mut put = |k: &str, v: &str| c.push(Element::with_text(k, v));
    put("name", name);
    put("role", role);
    put("location", "");
    put("connection", &connection.to_string());
    put("loyalty", &loyalty.to_string());
    for k in ["metatype", "gender", "age", "contacttype", "preferredpayment", "hobbiesvice", "personallife"] {
        put(k, "");
    }
    put("type", kind.as_str());
    for k in ["file", "relative", "notes"] {
        put(k, "");
    }
    put("notesColor", "Chocolate");
    put("groupname", "");
    put("colour", "-986896");
    for k in ["group", "family", "blackmail", "free"] {
        put(k, "False");
    }
    put("groupenabled", "True");
    put("guid", &new_guid());
    put("mainmugshotindex", "-1");
    c.push(Element::new("mugshots"));
    c
}

/// Chummer's Add Contact / Add Enemy / Add Pet: a blank entry of that
/// type. Returns its guid.
pub fn add(ch: &mut Character, kind: ContactType) -> String {
    let c = new_element(kind, "", "", 1, 1);
    let guid = c.get("guid");
    ch.items_mut("contacts").push(c);
    guid
}

/// The entries of one type, in file order.
pub fn of_type(ch: &Character, kind: ContactType) -> Vec<&Element> {
    ch.items("contacts", "contact").into_iter().filter(|c| ContactType::of(c) == kind).collect()
}

pub fn find_mut<'a>(ch: &'a mut Character, guid: &str) -> Option<&'a mut Element> {
    ch.doc.child_mut("contacts")?.elements_mut().find(|c| c.name == "contact" && c.get("guid").eq_ignore_ascii_case(guid))
}

/// Set one saved field of a contact. Returns whether it changed.
pub fn set_field(ch: &mut Character, guid: &str, key: &str, value: &str) -> bool {
    let Some(c) = find_mut(ch, guid) else { return false };
    if c.get(key) == value {
        return false;
    }
    c.set_child_text(key, value);
    ch.dirty = true;
    true
}

/// Remove a contact (and improvements it was the source of).
pub fn remove(ch: &mut Character, guid: &str) -> bool {
    ch.remove_item("contacts", guid)
}

/// The drop-down lists of `ContactControl` from `contacts.xml`, by saved
/// field: role (`contacts/contact`), gender, age, personal life, type,
/// preferred payment and hobbies/vice. Values are the English names.
pub const CHOICE_LISTS: &[(&str, &str, &str)] = &[
    ("role", "contacts", "contact"),
    ("gender", "genders", "gender"),
    ("age", "ages", "age"),
    ("personallife", "personallives", "personallife"),
    ("contacttype", "types", "type"),
    ("preferredpayment", "preferredpayments", "preferredpayment"),
    ("hobbiesvice", "hobbiesvices", "hobbyvice"),
];

/// The entries of one `contacts.xml` list (see [`CHOICE_LISTS`]), sorted.
pub fn choices(store: &DataStore, field: &str) -> Vec<String> {
    let Some((_, container, item)) = CHOICE_LISTS.iter().find(|(f, _, _)| *f == field) else { return Vec::new() };
    let Ok(doc) = store.doc("contacts.xml") else { return Vec::new() };
    let mut out: Vec<String> = doc.child(container).map(|c| c.children_named(item).map(Element::text).collect()).unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// Metatype choices: every metatype and "Metatype (Metavariant)" pair of
/// `metatypes.xml` (contacts, `ContactControl.LoadStatBlockLists`) or
/// `critters.xml` (pets, `PetControl.LoadContactList`), as (saved value,
/// metatype, metavariant). Like Chummer, a metavariant whose name is
/// already listed is skipped. Sorted by display name. Chummer saves the
/// combo box text, so the saved value is "Metatype (Metavariant)".
pub fn metatype_choices(store: &DataStore, file: &str) -> Vec<(String, String, String)> {
    let Ok(doc) = store.doc(file) else { return Vec::new() };
    let mut out: Vec<(String, String, String)> = Vec::new();
    // ListItem values so far: metatype and metavariant names.
    let mut seen: Vec<String> = Vec::new();
    for m in doc.child("metatypes").map(|c| c.children_named("metatype").collect::<Vec<_>>()).unwrap_or_default() {
        let name = m.get("name");
        if name.is_empty() {
            continue;
        }
        out.push((name.clone(), name.clone(), String::new()));
        seen.push(name.clone());
        for v in m.child("metavariants").map(|c| c.children_named("metavariant").collect::<Vec<_>>()).unwrap_or_default() {
            let vn = v.get("name");
            if !vn.is_empty() && !seen.iter().any(|s| s.eq_ignore_ascii_case(&vn)) {
                seen.push(vn.clone());
                out.push((display_pair(&name, &vn), name.clone(), vn));
            }
        }
    }
    out.sort();
    out
}

fn display_pair(metatype: &str, metavariant: &str) -> String {
    if metavariant.is_empty() {
        metatype.to_owned()
    } else {
        format!("{metatype} ({metavariant})")
    }
}

// ---------------------------------------------------------------------------
// Linked characters
// ---------------------------------------------------------------------------

/// The directory Chummer's `Utils.GetStartupPath` stands for: where the
/// program lives, the base of `<relative>` paths.
pub fn startup_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_owned)).unwrap_or_else(|| PathBuf::from("."))
}

/// "Attach Character": `<file>` is the path as picked, `<relative>` the
/// same file relative to the startup directory, written like Chummer's
/// `"../" + new Uri(startup).MakeRelativeUri(new Uri(file))`. Chummer's
/// URI treats the startup directory as a file, so the relative path starts
/// from its parent and gets "../" in front; the result resolves against
/// the startup directory either way.
pub fn link(ch: &mut Character, guid: &str, file: &Path, startup: &Path) -> bool {
    let rel = relative_uri(startup, file);
    let a = set_field(ch, guid, "file", &file.to_string_lossy());
    let b = set_field(ch, guid, "relative", &rel);
    a || b
}

/// "Remove Character": forget the linked file (the file stays on disk).
pub fn unlink(ch: &mut Character, guid: &str) -> bool {
    let a = set_field(ch, guid, "file", "");
    let b = set_field(ch, guid, "relative", "");
    a || b
}

/// Whether a contact names a linked file at all (Chummer shows "Open
/// Character" / "Remove Character" instead of "Attach Character").
pub fn is_linked(c: &Element) -> bool {
    !c.get("file").is_empty()
}

/// `"../" + MakeRelativeUri`, with `/` separators.
pub fn relative_uri(startup: &Path, file: &Path) -> String {
    let base_dir = startup.parent().map(normalize).unwrap_or_default();
    let file = normalize(file);
    let base: Vec<Component> = base_dir.components().collect();
    let target: Vec<Component> = file.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into()];
    parts.extend(std::iter::repeat_n("..".to_owned(), base.len() - common));
    parts.extend(target[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

/// Lexical `Path.GetFullPath`: drop `.` and fold `..`.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where a linked file was found, or why it was not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedPath {
    Found(PathBuf),
    /// Neither `<file>` nor `<relative>` exists (Chummer's
    /// `Message_FileNotFound`, with the `<file>` path).
    Missing(String),
    /// A `.chum5lz` (compressed) save, which chummer-rs cannot read.
    Unsupported(PathBuf),
}

/// The file of a linked contact, like `Contact.RefreshLinkedCharacter`:
/// `<file>` if it exists, else `<relative>` from the startup directory
/// (and, as Chummer's `Path.GetFullPath` does, from the working
/// directory). chummer-rs also looks for the file next to the owner's
/// save, so links made on Windows (`C:\...`) work once the files are
/// copied over together. `None` when the contact is not linked.
pub fn resolve(c: &Element, startup: &Path, owner: Option<&Path>) -> Option<LinkedPath> {
    let file = c.get("file");
    if file.is_empty() {
        return None;
    }
    let rel = c.get("relative").replace('\\', "/");
    let mut candidates = vec![PathBuf::from(&file)];
    if !rel.is_empty() {
        candidates.push(normalize(&startup.join(&rel)));
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(normalize(&cwd.join(&rel)));
        }
    }
    if let Some(dir) = owner.and_then(Path::parent) {
        let name = file.rsplit(['/', '\\']).next().unwrap_or(&file);
        candidates.push(dir.join(name));
    }
    let found = candidates.into_iter().find(|p| p.is_file());
    Some(match found {
        Some(p) if is_chum5lz(&p) => LinkedPath::Unsupported(p),
        Some(p) => LinkedPath::Found(p),
        None => LinkedPath::Missing(file),
    })
}

fn is_chum5lz(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("chum5lz"))
}

/// What Chummer shows from a linked character.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkedCharacter {
    pub path: PathBuf,
    /// `Character.CharacterName`: alias, else name, else "Unnamed Character".
    pub name: String,
    pub metatype: String,
    pub metavariant: String,
    pub gender: String,
    pub age: String,
    /// The main mugshot, base64 as saved (`Character.MainMugshot`).
    pub mugshot: Option<String>,
}

impl LinkedCharacter {
    pub fn load(path: &Path) -> Result<LinkedCharacter, String> {
        if is_chum5lz(path) {
            return Err(format!("{}: compressed .chum5lz saves are not supported", path.display()));
        }
        let ch = Character::load(path).map_err(|e| e.to_string())?;
        Ok(LinkedCharacter::of(&ch, path))
    }

    pub fn of(ch: &Character, path: &Path) -> LinkedCharacter {
        let gender = if ch.doc.child("gender").is_some() { ch.field("gender") } else { ch.field("sex") };
        LinkedCharacter {
            path: path.to_owned(),
            name: ch.display_name(),
            metatype: ch.field("metatype"),
            metavariant: ch.field("metavariant"),
            gender,
            age: ch.field("age"),
            mugshot: main_mugshot(&ch.doc),
        }
    }

    /// `Contact.Metatype` of a linked contact: "Metatype (Metavariant)".
    pub fn display_metatype(&self) -> String {
        if self.metavariant.is_empty() {
            self.metatype.clone()
        } else {
            format!("{} ({})", self.metatype, self.metavariant)
        }
    }
}

/// `MainMugshot`: the mugshot at `<mainmugshotindex>`, none when the
/// index is out of range.
pub fn main_mugshot(e: &Element) -> Option<String> {
    let idx = usize::try_from(e.get_i32("mainmugshotindex")?).ok()?;
    let shot = e.child("mugshots")?.children_named("mugshot").nth(idx)?.text();
    (!shot.trim().is_empty()).then_some(shot)
}

/// Decode a base64 mugshot (standard alphabet; whitespace ignored).
pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            n |= val(b)? << (18 - 6 * i);
        }
        let take = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        out.extend_from_slice(&n.to_be_bytes()[1..1 + take]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_type_like_chummer() {
        assert_eq!(ContactType::parse(""), ContactType::Contact);
        assert_eq!(ContactType::parse("CONTACT"), ContactType::Contact);
        assert_eq!(ContactType::parse("pet"), ContactType::Pet);
        assert_eq!(ContactType::parse("Enemy"), ContactType::Enemy);
        assert_eq!(ContactType::parse("whatever"), ContactType::Enemy);
    }

    #[test]
    fn relative_uri_like_make_relative_uri() {
        let s = Path::new("/opt/chummer");
        assert_eq!(relative_uri(s, Path::new("/opt/chummer/saves/a.chum5")), "../chummer/saves/a.chum5");
        assert_eq!(relative_uri(s, Path::new("/home/u/a b.chum5")), "../../home/u/a b.chum5");
        // Both resolve back against the startup directory.
        assert_eq!(normalize(&s.join("../../home/u/a b.chum5")), PathBuf::from("/home/u/a b.chum5"));
        assert_eq!(normalize(&s.join("../chummer/saves/a.chum5")), PathBuf::from("/opt/chummer/saves/a.chum5"));
    }

    #[test]
    fn base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8h").unwrap(), b"hello!");
        assert_eq!(decode_base64("aGV\nsbA==").unwrap(), b"hell");
        assert!(decode_base64("a").is_none());
        assert!(decode_base64("a*bc").is_none());
    }
}
