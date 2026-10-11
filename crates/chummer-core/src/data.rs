//! Access to Chummer's XML game data (`data/*.xml`).
//!
//! The C# code queries these files with XPath; this module keeps the same
//! "query the document" model rather than modelling all ~40 schemas as
//! structs. Each file is parsed once, on first use, and shared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::custom_data;
use crate::xml::{self, Element};

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("cannot parse {path}: {source}")]
    Xml { path: PathBuf, source: xml::XmlError },
}

/// A data record: one `<skill>`, `<quality>`, `<gear>`, and so on.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a>(pub &'a Element);

impl<'a> Record<'a> {
    pub fn el(&self) -> &'a Element {
        self.0
    }
    pub fn id(&self) -> String {
        self.0.get("id")
    }
    pub fn name(&self) -> String {
        self.0.get("name")
    }
    pub fn category(&self) -> String {
        self.0.get("category")
    }
    pub fn source(&self) -> String {
        self.0.get("source")
    }
    pub fn page(&self) -> String {
        self.0.get("page")
    }
    pub fn get(&self, field: &str) -> String {
        self.0.get(field)
    }
    /// `<hide />` marks records that must not be offered for selection.
    pub fn hidden(&self) -> bool {
        self.0.child("hide").is_some()
    }
}

/// Parsed documents by file name. Each file has its own slot, locked
/// while it is parsed: a thread that asks for a file another thread is
/// parsing waits for that parse instead of doing it again.
type DocCache = Mutex<HashMap<String, Arc<Mutex<Option<Arc<Element>>>>>>;

/// The document in `file`'s slot of `cache`, from `load` the first time
/// (a failed load is not kept).
fn cached(cache: &DocCache, file: &str, load: impl FnOnce() -> Result<Arc<Element>, DataError>) -> Result<Arc<Element>, DataError> {
    let slot = cache.lock().unwrap().entry(file.to_owned()).or_default().clone();
    let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(d) = &*slot {
        return Ok(d.clone());
    }
    let d = load()?;
    *slot = Some(d.clone());
    Ok(d)
}

/// The records of one list of a data document (`<container><item/>…`),
/// indexed by id and name: [`find`] without the scan. Built from the
/// store's document for that file, which never changes once loaded.
#[derive(Debug)]
pub struct RecordIndex {
    doc: Arc<Element>,
    /// The container's position in the root's children.
    container: Option<usize>,
    /// Lowercase `<id>` → the first record with it (positions in the
    /// container's children).
    by_id: HashMap<String, usize>,
    /// `<name>` → every record with it, in document order.
    by_name: HashMap<String, Vec<usize>>,
}

impl RecordIndex {
    fn new(doc: Arc<Element>, container: &str, item: &str) -> Self {
        // `Element::child`: the first child of that name.
        let pos = doc.children.iter().position(|n| matches!(n, xml::Node::Element(e) if e.name == container));
        let (mut by_id, mut by_name) = (HashMap::new(), HashMap::<String, Vec<usize>>::new());
        if let Some(xml::Node::Element(c)) = pos.and_then(|p| doc.children.get(p)) {
            for (i, n) in c.children.iter().enumerate() {
                if let xml::Node::Element(e) = n {
                    if e.name == item {
                        by_id.entry(e.get("id").to_ascii_lowercase()).or_insert(i);
                        by_name.entry(e.get("name")).or_default().push(i);
                    }
                }
            }
        }
        RecordIndex { doc, container: pos, by_id, by_name }
    }

    fn container_el(&self) -> Option<&Element> {
        match self.doc.children.get(self.container?) {
            Some(xml::Node::Element(e)) => Some(e),
            _ => None,
        }
    }

    fn at(&self, i: usize) -> Option<Record<'_>> {
        match self.container_el()?.children.get(i) {
            Some(xml::Node::Element(e)) => Some(Record(e)),
            _ => None,
        }
    }

    /// The document the records are in.
    pub fn doc(&self) -> &Element {
        &self.doc
    }

    /// The first record whose `<id>` is `id` (case-insensitive).
    pub fn by_id(&self, id: &str) -> Option<Record<'_>> {
        self.at(*self.by_id.get(&id.to_ascii_lowercase())?)
    }

    /// The records named `name`, in document order.
    pub fn named<'s>(&'s self, name: &str) -> impl Iterator<Item = Record<'s>> + 's {
        self.by_name.get(name).into_iter().flatten().filter_map(|&i| self.at(i))
    }

    /// As [`find`]: by `<id>`, else the first by `<name>`.
    pub fn find(&self, key: &str) -> Option<Record<'_>> {
        self.by_id(key).or_else(|| self.named(key).next())
    }
}

/// Lazily loaded, shared view of the data directory, optionally with a set
/// of enabled custom data directories applied on top.
pub struct DataStore {
    data_dir: PathBuf,
    /// Base (unmodified) documents; shared between stores made with
    /// [`DataStore::with_enabled_custom_data`].
    cache: Arc<DocCache>,
    /// Enabled custom data directories, in load order.
    custom_dirs: Vec<PathBuf>,
    /// Documents with custom data applied.
    merged: DocCache,
    warnings: Mutex<Vec<String>>,
    /// Record indexes of this store's documents: (file, container, item).
    /// A few dozen at most: a list, so a lookup allocates nothing.
    indexes: Mutex<Vec<((String, String, String), Arc<RecordIndex>)>>,
    /// The thread of [`DataStore::warm`]: `None` before it, `Some(None)`
    /// once joined.
    warm: Mutex<Option<Option<std::thread::JoinHandle<()>>>>,
}

impl DataStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_custom_data(data_dir, Vec::new())
    }

    /// A store whose documents have `dirs` (custom data directories, in
    /// load order) applied, as `XmlManager.Load` does with a settings
    /// preset's enabled custom data paths.
    pub fn with_custom_data(data_dir: impl Into<PathBuf>, dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            cache: Arc::new(Mutex::new(HashMap::new())),
            custom_dirs: dirs,
            merged: Mutex::new(HashMap::new()),
            warnings: Mutex::new(Vec::new()),
            indexes: Mutex::new(Vec::new()),
            warm: Mutex::new(None),
        }
    }

    /// Same data directory and base-document cache, different custom data.
    pub fn with_enabled_custom_data(&self, dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dir: self.data_dir.clone(),
            cache: self.cache.clone(),
            custom_dirs: dirs,
            merged: Mutex::new(HashMap::new()),
            warnings: Mutex::new(Vec::new()),
            indexes: Mutex::new(Vec::new()),
            warm: Mutex::new(None),
        }
    }

    /// Locate the bundled `resources/data` directory.
    ///
    /// Search order: `$CHUMMER_DATA`, next to the executable, the system
    /// install dir, then the source tree (for `cargo run`).
    pub fn discover() -> Option<Self> {
        resource_dir("data").map(Self::new)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The enabled custom data directories, in load order.
    pub fn custom_data_dirs(&self) -> &[PathBuf] {
        &self.custom_dirs
    }

    /// Problems met while applying custom data (unreadable files,
    /// unsupported XPath or regex constructs). Those parts were skipped.
    pub fn custom_data_warnings(&self) -> Vec<String> {
        self.warnings.lock().unwrap().clone()
    }

    /// Parsed root element of a data file, e.g. `doc("skills.xml")`, with
    /// the enabled custom data applied.
    pub fn doc(&self, file: &str) -> Result<Arc<Element>, DataError> {
        if self.custom_dirs.is_empty() {
            return self.base_doc(file);
        }
        cached(&self.merged, file, || {
            let relevant = custom_data::relevant_directories(file, &self.custom_dirs);
            Ok(if relevant.is_empty() {
                self.base_doc(file)?
            } else {
                let mut root = (*self.base_doc(file)?).clone();
                let report = custom_data::apply(&mut root, file, &relevant);
                self.warnings.lock().unwrap().extend(report.warnings);
                Arc::new(root)
            })
        })
    }

    /// The records of `<container><item>` in `file` (as [`DataStore::doc`]
    /// gives it), indexed by id and name; built on first use.
    pub fn index(&self, file: &str, container: &str, item: &str) -> Result<Arc<RecordIndex>, DataError> {
        let get = |v: &[((String, String, String), Arc<RecordIndex>)]| v.iter().find(|((f, c, i), _)| f == file && c == container && i == item).map(|(_, ix)| ix.clone());
        if let Some(ix) = get(&self.indexes.lock().unwrap()) {
            return Ok(ix);
        }
        let ix = Arc::new(RecordIndex::new(self.doc(file)?, container, item));
        let mut v = self.indexes.lock().unwrap();
        // Another thread may have built it meanwhile: keep the first.
        if let Some(first) = get(&v) {
            return Ok(first);
        }
        v.push(((file.to_owned(), container.to_owned(), item.to_owned()), ix.clone()));
        Ok(ix)
    }

    /// Parses `docs` and builds `indexes` (file, container, item) on
    /// another thread, once per store, so the first page that reads them
    /// does not (an app warms what its pages read on their first visit).
    /// A file asked for meanwhile waits for that parse (see [`DocCache`]).
    pub fn warm(self: &Arc<Self>, docs: &'static [&'static str], indexes: &'static [(&'static str, &'static str, &'static str)]) {
        let mut w = self.warm.lock().unwrap();
        if w.is_some() {
            return;
        }
        let me = self.clone();
        let job = move || {
            for (f, c, i) in indexes {
                let _ = me.index(f, c, i);
            }
            for f in docs {
                let _ = me.doc(f);
            }
        };
        *w = Some(std::thread::Builder::new().name("warm game data".into()).spawn(job).ok());
    }

    /// Waits for [`DataStore::warm`]'s thread, if one runs.
    pub fn wait_warm(&self) {
        let handle = self.warm.lock().unwrap().as_mut().and_then(Option::take);
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    /// A data file exactly as shipped, without custom data.
    pub fn base_doc(&self, file: &str) -> Result<Arc<Element>, DataError> {
        cached(&self.cache, file, || {
            let path = self.data_dir.join(file);
            let src = std::fs::read_to_string(&path).map_err(|source| DataError::Io { path: path.clone(), source })?;
            Ok(Arc::new(xml::parse(&src).map_err(|source| DataError::Xml { path, source })?))
        })
    }
}

/// Find a bundled resource directory (`data`, `lang`, `sheets`, ...) in
/// the [`resource_root`].
pub fn resource_dir(name: &str) -> Option<PathBuf> {
    resource_root().map(|r| r.join(name)).filter(|p| p.is_dir())
}

/// The folder holding the bundled resources: the first of
/// [`resource_root_candidates`] with a `data` folder. One root for every
/// resource, so the user's own `sheets` and `customdata` folders (in the
/// data root, which has no `data` folder) are never taken for bundled ones.
pub fn resource_root() -> Option<PathBuf> {
    find_resource_root(resource_root_candidates())
}

/// The first candidate with a `data` folder.
pub fn find_resource_root(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|p| p.join("data").is_dir())
}

/// Where the bundled resources may be: `$CHUMMER_RESOURCES`, `resources`
/// next to the program (release archives, Windows), `install.sh`'s
/// `<prefix>/share/chummer-rs/resources`, the macOS bundle's
/// `Contents/Resources/resources`, a system `/usr/share/chummer-rs`, and
/// the source tree.
pub fn resource_root_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("CHUMMER_RESOURCES") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resources"));
            candidates.push(dir.join("../share/chummer-rs/resources"));
            // macOS app bundle: Contents/MacOS/chummer-rs, Contents/Resources/resources.
            candidates.push(dir.join("../Resources/resources"));
        }
    }
    candidates.push(PathBuf::from("/usr/share/chummer-rs/resources"));
    candidates.push(PathBuf::from("/usr/share/chummer-rs"));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources"));
    candidates
}

/// Records inside a document: `<root><container><item/>...</container></root>`.
pub fn records<'a>(doc: &'a Element, container: &str, item: &'a str) -> Vec<Record<'a>> {
    doc.child(container)
        .map(|c| c.children_named(item).map(Record).collect())
        .unwrap_or_default()
}

/// Find one record by `<id>` (case-insensitive, as GUIDs are) or `<name>`.
pub fn find<'a>(doc: &'a Element, container: &str, item: &'a str, key: &str) -> Option<Record<'a>> {
    let c = doc.child(container)?;
    c.children_named(item)
        .find(|e| e.get_eq_ignore_case("id", key))
        .or_else(|| c.children_named(item).find(|e| e.get_eq("name", key)))
        .map(Record)
}

/// `<categories><category>..</category></categories>` as plain strings.
pub fn categories(doc: &Element) -> Vec<String> {
    doc.child("categories")
        .map(|c| c.children_named("category").map(Element::text).collect())
        .unwrap_or_default()
}

/// The `(file, container, item)` triple for each record kind the UI browses.
pub const BROWSABLE: &[(&str, &str, &str, &str)] = &[
    ("Armor", "armor.xml", "armors", "armor"),
    ("Bioware", "bioware.xml", "biowares", "bioware"),
    ("Books", "books.xml", "books", "book"),
    ("Complex Forms", "complexforms.xml", "complexforms", "complexform"),
    ("Critter Powers", "critterpowers.xml", "powers", "power"),
    ("Critters", "critters.xml", "metatypes", "metatype"),
    ("Cyberware", "cyberware.xml", "cyberwares", "cyberware"),
    ("Drugs", "drugcomponents.xml", "drugs", "drug"),
    ("Echoes", "echoes.xml", "echoes", "echo"),
    ("Gear", "gear.xml", "gears", "gear"),
    ("Lifestyles", "lifestyles.xml", "lifestyles", "lifestyle"),
    ("Martial Arts", "martialarts.xml", "martialarts", "martialart"),
    ("Mentors", "mentors.xml", "mentors", "mentor"),
    ("Metamagic", "metamagic.xml", "metamagics", "metamagic"),
    ("Metatypes", "metatypes.xml", "metatypes", "metatype"),
    ("Adept Powers", "powers.xml", "powers", "power"),
    ("Programs", "programs.xml", "programs", "program"),
    ("Qualities", "qualities.xml", "qualities", "quality"),
    ("Skills", "skills.xml", "skills", "skill"),
    ("Knowledge Skills", "skills.xml", "knowledgeskills", "skill"),
    ("Spells", "spells.xml", "spells", "spell"),
    ("Traditions", "traditions.xml", "traditions", "tradition"),
    ("Vehicles", "vehicles.xml", "vehicles", "vehicle"),
    ("Vehicle Mods", "vehicles.xml", "mods", "mod"),
    ("Weapons", "weapons.xml", "weapons", "weapon"),
    ("Weapon Accessories", "weapons.xml", "accessories", "accessory"),
];

/// The stock game data, for lookups that only have a saved element at
/// hand (weapon ranges, accessory and drug grade multipliers, cyberware
/// grade device ratings). Custom data is not included.
pub fn shared_store() -> Option<&'static DataStore> {
    static STORE: OnceLock<Option<DataStore>> = OnceLock::new();
    STORE.get_or_init(DataStore::discover).as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index answers exactly as the scan does, for every id and name
    /// (and their case variants) of the lists the item pages look up.
    #[test]
    fn the_record_index_finds_what_find_finds() {
        let store = DataStore::discover().expect("resources/data");
        for (file, container, item) in [("gear.xml", "gears", "gear"), ("vehicles.xml", "vehicles", "vehicle"), ("vehicles.xml", "mods", "mod"), ("weapons.xml", "accessories", "accessory"), ("cyberware.xml", "cyberwares", "cyberware")] {
            let ix = store.index(file, container, item).unwrap();
            let doc = store.doc(file).unwrap();
            let mut keys: Vec<String> = records(&doc, container, item).iter().flat_map(|r| [r.id(), r.id().to_uppercase(), r.name(), r.name().to_lowercase()]).collect();
            keys.extend(["".to_owned(), "no such record".to_owned()]);
            for k in &keys {
                let (a, b) = (ix.find(k).map(|r| r.el() as *const Element), find(&doc, container, item, k).map(|r| r.el() as *const Element));
                assert_eq!(a, b, "{file} {container}/{item}: {k:?}");
            }
            assert!(std::ptr::eq(ix.doc(), &*doc));
        }
    }

    /// Threads asking for a file at once share one parse.
    #[test]
    fn a_file_is_parsed_once_across_threads() {
        let store = Arc::new(DataStore::discover().expect("resources/data"));
        let docs: Vec<Arc<Element>> = (0..4).map(|_| { let s = store.clone(); std::thread::spawn(move || s.doc("lifestyles.xml").unwrap()) }).collect::<Vec<_>>().into_iter().map(|t| t.join().unwrap()).collect();
        assert!(docs.iter().all(|d| Arc::ptr_eq(d, &docs[0])));
        store.warm(&["contacts.xml"], &[("armor.xml", "armors", "armor")]);
        store.wait_warm();
        assert!(Arc::ptr_eq(&store.doc("armor.xml").unwrap(), &store.index("armor.xml", "armors", "armor").unwrap().doc));
        // Once per store.
        store.warm(&["critters.xml"], &[]);
        store.wait_warm();
    }

    #[test]
    fn the_user_root_is_never_a_resource_root() {
        // An install.sh prefix: the user's data root is <prefix>/share/chummer-rs
        // (with its own sheets and customdata), the program's resources are
        // in its `resources` folder.
        let t = std::env::temp_dir().join(format!("chummer-resource-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let user = t.join("share/chummer-rs");
        for d in ["sheets", "customdata", "kits"] {
            std::fs::create_dir_all(user.join(d)).unwrap();
        }
        let bin = t.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cands = || vec![bin.join("resources"), user.clone(), bin.join("../share/chummer-rs/resources")];
        assert_eq!(find_resource_root(cands()), None);
        std::fs::create_dir_all(user.join("resources/data")).unwrap();
        std::fs::create_dir_all(user.join("resources/sheets")).unwrap();
        assert_eq!(find_resource_root(cands()), Some(bin.join("../share/chummer-rs/resources")));
        // The default list looks in that folder, not in the user root.
        let defaults = resource_root_candidates();
        assert!(defaults.iter().any(|p| p.ends_with("share/chummer-rs/resources")));
        assert!(!defaults.iter().any(|p| p.ends_with("share/chummer-rs") && !p.starts_with("/usr")));
        let _ = std::fs::remove_dir_all(&t);
    }
}
