//! Custom data directories (`customdata/<name>/`): optional rule sets that
//! modify the game data, enabled per settings preset.
//!
//! A directory holds `override_*.xml`, `custom_*.xml` and `amend_*.xml`
//! files plus an optional `manifest.xml`. For a data file such as
//! `qualities.xml`, every file named `<prefix>_*qualities.xml` (anything
//! ending in `_qualities.xml`) applies to it, in three passes per
//! directory, as `XmlManager.DoProcessCustomDataFiles` does:
//!
//! 1. `override_`: replace the contents of the record with the same id/name;
//! 2. `custom_`: append new records, skipping ones that already exist;
//! 3. `amend_`: `AmendNodeChildren` with its `amendoperation`s.
//!
//! Directories apply in the order the settings preset lists them.

mod amend;
mod regex;
pub mod xpath;

use std::path::{Path, PathBuf};

use crate::xml::{self, Element, Node};

// ---------------------------------------------------------------- versions

/// A `System.Version`-like dotted version (`1`, `1.2`, `1.2.3.4`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub [u32; 4]);

impl Version {
    /// Parse `"1"`, `"1.0"`, ...; `None` for anything non-numeric.
    pub fn parse(s: &str) -> Option<Version> {
        let mut parts = [0u32; 4];
        let mut n = 0;
        for (i, p) in s.trim().split('.').enumerate() {
            if i >= 4 {
                return None;
            }
            parts[i] = p.trim().parse().ok()?;
            n += 1;
        }
        (n > 0).then_some(Version(parts))
    }
}

impl std::fmt::Display for Version {
    /// `ValueVersion.ToString()`: major.minor, plus build and revision when set.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.0;
        match (c, d) {
            (0, 0) => write!(f, "{a}.{b}"),
            (_, 0) => write!(f, "{a}.{b}.{c}"),
            _ => write!(f, "{a}.{b}.{c}.{d}"),
        }
    }
}

// ---------------------------------------------------------------- manifest

/// A `<dependency>` or `<incompatibility>` (`DirectoryDependency`).
#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryDependency {
    pub name: String,
    pub guid: String,
    pub min_version: Option<Version>,
    pub max_version: Option<Version>,
}

impl DirectoryDependency {
    /// Whether `v` falls inside `[min_version, max_version]`.
    pub fn accepts(&self, v: Version) -> bool {
        self.min_version.is_none_or(|m| v >= m) && self.max_version.is_none_or(|m| v <= m)
    }
}

/// `manifest.xml` contents (`CustomDataDirectoryInfo.LoadConstructorData`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    pub guid: String,
    pub version: Version,
    pub update_location: String,
    /// `(language, text)` pairs.
    pub descriptions: Vec<(String, String)>,
    /// `(name, is_main_author)` pairs.
    pub authors: Vec<(String, bool)>,
    pub dependencies: Vec<DirectoryDependency>,
    pub incompatibilities: Vec<DirectoryDependency>,
}

impl Manifest {
    /// Parse a `<manifest>` document.
    pub fn parse(src: &str) -> Result<Manifest, xml::XmlError> {
        let root = xml::parse(src)?;
        let get = |k: &str| root.child_text(k).map(|s| s.trim().to_owned()).unwrap_or_default();
        Ok(Manifest {
            guid: get("guid").to_ascii_lowercase(),
            version: Version::parse(&get("version")).unwrap_or(Version([1, 0, 0, 0])),
            update_location: get("updatelocation"),
            descriptions: pairs(&root, "descriptions/description", |d| {
                let lang = d.get("lang");
                (!lang.is_empty() && d.child("text").is_some()).then(|| (lang, d.get("text")))
            }),
            authors: pairs(&root, "authors/author", |a| {
                let name = a.get("name");
                (!name.is_empty()).then(|| (name, a.get_bool("main").unwrap_or(false)))
            }),
            dependencies: pairs(&root, "dependencies/dependency", dependency),
            incompatibilities: pairs(&root, "incompatibilities/incompatibility", dependency),
        })
    }

    /// Description in `lang` (e.g. `en-us`), falling back to the first one.
    pub fn description(&self, lang: &str) -> Option<&str> {
        self.descriptions
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(lang))
            .or_else(|| self.descriptions.first())
            .map(|(_, t)| t.as_str())
    }
}

fn pairs<T>(root: &Element, path: &str, f: impl Fn(&Element) -> Option<T>) -> Vec<T> {
    let (container, item) = path.split_once('/').unwrap_or((path, ""));
    root.child(container).map(|c| c.children_named(item).filter_map(&f).collect()).unwrap_or_default()
}

/// `ConstructorGetDependencies`: entries without a name or guid are dropped.
fn dependency(e: &Element) -> Option<DirectoryDependency> {
    let name = e.get("name");
    let guid = e.get("guid").trim().to_ascii_lowercase();
    if name.is_empty() || guid.is_empty() || guid.chars().all(|c| c == '0' || c == '-') {
        return None;
    }
    Some(DirectoryDependency {
        name,
        guid,
        min_version: Version::parse(&e.get("minversion")),
        max_version: Version::parse(&e.get("maxversion")),
    })
}

// ---------------------------------------------------------------- directories

/// One custom data directory (`CustomDataDirectoryInfo`).
#[derive(Debug, Clone)]
pub struct CustomDataDirectory {
    /// Display name and settings key: the folder name.
    pub name: String,
    pub path: PathBuf,
    pub manifest: Option<Manifest>,
    /// Why `manifest.xml` could not be read, if it exists but is broken.
    pub manifest_error: Option<String>,
}

impl CustomDataDirectory {
    /// Read a directory and its manifest (looked up case-insensitively).
    pub fn load(path: impl Into<PathBuf>) -> CustomDataDirectory {
        let path = path.into();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut dir = CustomDataDirectory { name, path, manifest: None, manifest_error: None };
        if let Some(file) = find_file_ci(&dir.path, "manifest.xml") {
            match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|s| Manifest::parse(&s).map_err(|e| e.to_string())) {
                Ok(m) => dir.manifest = Some(m),
                Err(e) => dir.manifest_error = Some(e),
            }
        }
        dir
    }

    pub fn guid(&self) -> Option<&str> {
        self.manifest.as_ref().map(|m| m.guid.as_str()).filter(|g| !g.is_empty())
    }

    pub fn version(&self) -> Version {
        self.manifest.as_ref().map(|m| m.version).unwrap_or(Version([1, 0, 0, 0]))
    }

    /// `CharacterSettingsSaveKey`: `guid>version` with a manifest, else the name.
    pub fn save_key(&self) -> String {
        match &self.manifest {
            Some(m) => format!("{}>{}", m.guid, m.version),
            None => self.name.clone(),
        }
    }

    /// Data files (`qualities.xml`, ...) this directory changes.
    pub fn affected_files(&self) -> Vec<String> {
        let mut out: Vec<String> = list_xml_files(&self.path)
            .iter()
            .filter_map(|p| {
                let f = p.file_name()?.to_string_lossy().to_ascii_lowercase();
                let rest = PREFIXES.iter().find_map(|pre| f.strip_prefix(pre))?;
                // `amend_orktusk_metatypes.xml` targets `metatypes.xml`.
                Some(rest.rsplit_once('_').map_or(rest, |(_, base)| base).to_owned())
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

fn find_file_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_file() && p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

/// Every subdirectory of `root`, sorted by name
/// (`GlobalSettings` enumerating `Utils.GetCustomDataFolderPath`).
pub fn discover(root: &Path) -> Vec<CustomDataDirectory> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by_key(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase()));
    dirs.into_iter().map(CustomDataDirectory::load).collect()
}

/// Folders searched for custom data: the bundled `customdata` resource
/// directory, then `$XDG_DATA_HOME/chummer-rs/customdata` for the user's own.
pub fn default_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = crate::data::resource_dir("customdata").into_iter().collect();
    let user = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|b| b.join("chummer-rs").join("customdata"));
    if let Some(u) = user.filter(|u| u.is_dir() && !roots.iter().any(|r| same_dir(r, u))) {
        roots.push(u);
    }
    roots
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

// ---------------------------------------------------------------- settings

/// One `<customdatadirectoryname>` entry of a settings preset.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryEntry {
    /// A directory name or a `guid>version` save key.
    pub key: String,
    pub order: Option<i32>,
    pub enabled: bool,
}

/// `<customdatadirectorynames>` of a settings preset (`raw` element of a
/// `CharacterSettings`), in file order.
pub fn settings_entries(settings: &Element) -> Vec<DirectoryEntry> {
    let Some(c) = settings.child("customdatadirectorynames") else { return Vec::new() };
    c.children_named("customdatadirectoryname")
        .filter_map(|e| {
            let key = e.get("directoryname");
            (!key.is_empty()).then(|| DirectoryEntry {
                key,
                order: xml::parse_int(&e.get("order")),
                enabled: e.get("enabled") == "True",
            })
        })
        .collect()
}

/// `customdatadirectorynames/directoryname` of a saved character: the
/// directories it was last saved with (informational; the settings
/// preset decides what is loaded).
pub fn character_directory_names(character: &Element) -> Vec<String> {
    character
        .child("customdatadirectorynames")
        .map(|c| c.children_named("directoryname").map(Element::text).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// `GetIdFromCharacterSettingsSaveKey`: the guid and version of a
/// `guid>version` key, `None` for a plain directory name.
pub fn split_save_key(key: &str) -> Option<(String, Version)> {
    let (id, ver) = key.split_once('>')?;
    if ver.is_empty() || !is_guid(id) {
        return None;
    }
    Some((id.to_ascii_lowercase(), Version::parse(ver).unwrap_or_default()))
}

fn is_guid(s: &str) -> bool {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The directory a settings key refers to (`RecalculateEnabledCustomDataDirectories`):
/// by name (highest version wins) or by guid (closest version wins).
pub fn resolve_key<'a>(key: &str, available: &'a [CustomDataDirectory]) -> Option<&'a CustomDataDirectory> {
    match split_save_key(key) {
        // `rev()`: on ties the C# keeps the first candidate, `max_by_key` the last.
        None => available.iter().rev().filter(|d| d.name.eq_ignore_ascii_case(key)).max_by_key(|d| d.version()),
        Some((guid, want)) => {
            available.iter().rev().filter(|d| d.guid() == Some(guid.as_str())).max_by_key(|d| version_score(want, d.version()))
        }
    }
}

/// `VersionMatchScore`: higher is closer to the preferred version.
fn version_score(want: Version, have: Version) -> i64 {
    let d = |i: usize| (i64::from(want.0[i]) - i64::from(have.0[i])).pow(2);
    i64::from(i32::MAX) - d(2) * 16_777_216 - d(0) * 65_536 - d(1) * 256 - d(3)
}

/// Enabled directories of a settings preset, in load order: entries
/// sorted by `<order>`, duplicates of one guid collapsed to the highest
/// requested version, disabled and unknown ones dropped.
pub fn enabled_directories<'a>(settings: &Element, available: &'a [CustomDataDirectory]) -> Vec<&'a CustomDataDirectory> {
    let mut entries = settings_entries(settings);
    // Entries without an order keep file order after the ordered ones.
    entries.sort_by_key(|e| e.order.map_or((1, 0), |o| (0, o)));
    let mut picked: Vec<(String, bool, Version)> = Vec::new();
    for e in entries {
        let dedupe_key = match split_save_key(&e.key) {
            Some((guid, v)) => {
                if let Some(slot) = picked.iter_mut().find(|(k, _, _)| split_save_key(k).is_some_and(|(g, _)| g == guid)) {
                    if v > slot.2 {
                        *slot = (e.key.clone(), e.enabled, v);
                    }
                    continue;
                }
                (e.key.clone(), v)
            }
            None => {
                if picked.iter().any(|(k, _, _)| k.eq_ignore_ascii_case(&e.key)) {
                    continue;
                }
                (e.key.clone(), Version::default())
            }
        };
        picked.push((dedupe_key.0, e.enabled, dedupe_key.1));
    }
    let mut out: Vec<&CustomDataDirectory> = Vec::new();
    for (key, enabled, _) in picked {
        if let Some(d) = resolve_key(&key, available).filter(|_| enabled) {
            if !out.iter().any(|o| o.path == d.path) {
                out.push(d);
            }
        }
    }
    out
}

/// Problems with a set of enabled directories: `(directory, message)` for
/// each missing dependency and each enabled incompatibility
/// (`CustomDataDirectoryInfo.CheckDependency` / `CheckIncompatibility`).
pub fn check_dependencies(enabled: &[&CustomDataDirectory]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for d in enabled {
        let Some(m) = &d.manifest else { continue };
        for dep in &m.dependencies {
            let ok = enabled.iter().any(|o| o.guid() == Some(dep.guid.as_str()) && dep.accepts(o.version()));
            if !ok {
                out.push((d.name.clone(), format!("requires {}", dep.name)));
            }
        }
        for inc in &m.incompatibilities {
            if enabled.iter().any(|o| o.guid() == Some(inc.guid.as_str()) && inc.accepts(o.version())) {
                out.push((d.name.clone(), format!("is incompatible with {}", inc.name)));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- merging

const PREFIXES: [&str; 3] = ["override_", "custom_", "amend_"];

/// What applying custom data to one document did.
#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    /// Files that were read and applied, in order.
    pub files: Vec<PathBuf>,
    /// Edits made per file (same order as `files`).
    pub mutations: Vec<usize>,
    /// Unreadable files and unsupported constructs, which were skipped.
    pub warnings: Vec<String>,
}

/// All `*.xml` files under `dir`, recursively (`SearchOption.AllDirectories`).
fn list_xml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for p in rd.flatten().map(|e| e.path()) {
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
                out.push(p);
            }
        }
    }
    out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    out
}

/// Files in `dir` with `prefix` that apply to `file_name` (`*_<file_name>`).
fn files_for(dir: &Path, file_name: &str, prefix: &str) -> Vec<PathBuf> {
    let suffix = format!("_{}", file_name.to_ascii_lowercase());
    list_xml_files(dir)
        .into_iter()
        .filter(|p| {
            let f = p.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            f.starts_with(prefix) && f.ends_with(&suffix)
        })
        .collect()
}

/// `CompileRelevantCustomDataPaths`: the directories with files for `file_name`.
pub fn relevant_directories(file_name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    if file_name == "improvements.xml" {
        return Vec::new();
    }
    dirs.iter().filter(|d| PREFIXES.iter().any(|p| !files_for(d, file_name, p).is_empty())).cloned().collect()
}

/// Apply every enabled directory to `doc` (the parsed `file_name`), in
/// order (`XmlManager.LoadCoreAsync` calling `DoProcessCustomDataFiles`).
pub fn apply(doc: &mut Element, file_name: &str, dirs: &[PathBuf]) -> MergeReport {
    let mut report = MergeReport::default();
    for dir in relevant_directories(file_name, dirs) {
        apply_directory(doc, file_name, &dir, &mut report);
    }
    report
}

/// `DoProcessCustomDataFiles` for one directory: overrides, then customs,
/// then amends.
pub fn apply_directory(doc: &mut Element, file_name: &str, dir: &Path, report: &mut MergeReport) {
    type Pass = fn(&mut Element, Element, &mut Vec<String>, &str) -> usize;
    let passes: [(&str, Pass); 3] = [("override_", apply_override), ("custom_", apply_custom), ("amend_", apply_amend)];
    for (prefix, pass) in passes {
        for file in files_for(dir, file_name, prefix) {
            let label = file.display().to_string();
            let parsed = std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|s| xml::parse(&s).map_err(|e| e.to_string()));
            match parsed {
                Ok(custom) => {
                    let n = pass(doc, custom, &mut report.warnings, &label);
                    report.files.push(file);
                    report.mutations.push(n);
                }
                Err(e) => report.warnings.push(format!("{label}: skipped: {e}")),
            }
        }
    }
}

/// `Replace("&amp;", "&")` on identifier text, as the C# does before
/// building its filter (our parser has already decoded entities once).
fn id_text(e: &Element) -> String {
    xpath::string_value(e).replace("&amp;", "&")
}

/// The id/name filter of `override_` and `custom_` records.
fn record_filter(record: &Element, with_isidnode: bool) -> Option<xpath::Expr> {
    let mut f = match (record.child("id"), record.child("name")) {
        (Some(id), _) => Some(xpath::Expr::child_equals("id", &id_text(id))),
        (None, Some(name)) => Some(xpath::Expr::child_equals("name", &id_text(name))),
        _ => None,
    };
    if with_isidnode {
        for extra in record.elements().filter(|e| e.attr("isidnode") == Some("True")) {
            let cond = xpath::Expr::child_equals(&extra.name, &id_text(extra));
            f = Some(match f {
                Some(x) => x.and(cond),
                None => cond,
            });
        }
    }
    f
}

fn step(name: &str, filter: Option<xpath::Expr>) -> amend::DocStep {
    amend::DocStep { name: name.to_owned(), filter }
}

/// `override_` pass: each record replaces the children (`InnerXml`) of the
/// existing record at `/chummer/<container>/<item>[id or name]`.
fn apply_override(doc: &mut Element, custom: Element, warnings: &mut Vec<String>, label: &str) -> usize {
    let mut n = 0;
    for container in custom.elements() {
        for record in container.elements() {
            let Some(filter) = record_filter(record, true) else { continue };
            let path = vec![step(&doc.name, None), step(&container.name, None), step(&record.name, Some(filter))];
            match amend::select(doc, &path) {
                Ok(hits) => {
                    if let Some(first) = hits.first() {
                        amend::at_mut(doc, first).children = record.children.clone();
                        n += 1;
                    }
                }
                Err(e) => warnings.push(format!("{label}: {e}")),
            }
        }
    }
    n
}

/// Filter on a container's attributes (`@a = 'x' and @b = 'y'`).
fn attribute_filter(container: &Element) -> Option<xpath::Expr> {
    container
        .attrs
        .iter()
        .map(|(k, v)| {
            let attr = xpath::parse(&format!("@{k}")).ok()?;
            Some(xpath::Expr::Cmp(xpath::CmpOp::Eq, Box::new(attr), Box::new(xpath::Expr::Literal(v.replace("&amp;", "&")))))
        })
        .reduce(|a, b| match (a, b) {
            (Some(a), Some(b)) => Some(a.and(b)),
            _ => None,
        })
        .flatten()
}

/// `custom_` pass: drop records whose id/name already exists, then append
/// the rest to the matching container (or append the whole container).
fn apply_custom(doc: &mut Element, mut custom: Element, warnings: &mut Vec<String>, label: &str) -> usize {
    let mut n = 0;
    for container in custom.elements_mut() {
        let parent_step = step(&container.name, attribute_filter(container));
        let mut keep = Vec::with_capacity(container.children.len());
        for child in std::mem::take(&mut container.children) {
            let exists = match &child {
                Node::Element(rec) => record_filter(rec, false).is_some_and(|f| {
                    let path = vec![step(&doc.name, None), parent_step.clone(), step(&rec.name, Some(f))];
                    amend::select(doc, &path).map(|h| !h.is_empty()).unwrap_or_else(|e| {
                        warnings.push(format!("{label}: {e}"));
                        false
                    })
                }),
                _ => false,
            };
            if !exists {
                keep.push(child);
            }
        }
        container.children = keep;
        n += merge_container(doc, container);
    }
    n
}

/// The container merge at the end of the `custom_` loop. Matches the C#
/// quirk: an existing container with the same attribute count but other
/// values receives nothing.
fn merge_container(doc: &mut Element, container: &Element) -> usize {
    let existing = doc.child_mut(&container.name);
    match existing {
        Some(ex) if ex.attrs.len() == container.attrs.len() => {
            let all_match = ex.attrs.iter().all(|(k, v)| container.attr(k) == Some(v.as_str()));
            if !all_match {
                return 0;
            }
            let added = container.elements().count();
            ex.children.extend(container.children.iter().cloned());
            added
        }
        _ => {
            doc.push(container.clone());
            1
        }
    }
}

/// `amend_` pass: `AmendNodeChildren(doc, node, "/chummer")` for each
/// top-level node of the amend file.
fn apply_amend(doc: &mut Element, mut custom: Element, warnings: &mut Vec<String>, label: &str) -> usize {
    let root = amend::root_path(&doc.name);
    let mut amender = amend::Amender::new(warnings, label.to_owned());
    for node in custom.elements_mut() {
        let mut extra = Vec::new();
        amender.amend_node_children(doc, node, &root, &mut extra);
    }
    amender.mutations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amend(base: &str, amend_src: &str) -> Element {
        let mut doc = xml::parse(base).unwrap();
        let mut warnings = Vec::new();
        apply_amend(&mut doc, xml::parse(amend_src).unwrap(), &mut warnings, "test");
        assert!(warnings.is_empty(), "{warnings:?}");
        doc
    }

    const BASE: &str = "<chummer><qualities>\
        <quality><id>1</id><name>A</name><karma>5</karma><bonus><x>1</x></bonus></quality>\
        <quality><id>2</id><name>B</name><karma>7</karma></quality>\
        </qualities></chummer>";

    fn quality<'a>(doc: &'a Element, name: &str) -> Option<&'a Element> {
        crate::data::find(doc, "qualities", "quality", name).map(|r| r.el())
    }

    #[test]
    fn default_operations() {
        // Leaf with a target -> replace; leaf without -> append; children -> recurse.
        let doc = amend(BASE, "<chummer><qualities><quality><name>A</name><karma>9</karma><limit>1</limit></quality></qualities></chummer>");
        let a = quality(&doc, "A").unwrap();
        assert_eq!(a.get("karma"), "9");
        assert_eq!(a.get("limit"), "1");
        assert_eq!(quality(&doc, "B").unwrap().get("karma"), "7");
    }

    #[test]
    fn remove_replace_addnode_and_filters() {
        let doc = amend(
            BASE,
            r#"<chummer><qualities>
                <quality xpathfilter="karma &gt; 6" amendoperation="remove" />
                <quality><id>1</id><bonus amendoperation="replace"><y>2</y></bonus><note amendoperation="addnode">n</note></quality>
            </qualities></chummer>"#,
        );
        assert!(quality(&doc, "B").is_none());
        let a = quality(&doc, "A").unwrap();
        assert_eq!(a.path("bonus/y").unwrap().text(), "2");
        assert!(a.path("bonus/x").is_none());
        assert_eq!(a.get("note"), "n");
    }

    #[test]
    fn recurse_recreates_missing_parents() {
        // <bonus> does not exist on B: the recurse records it, and the leaf
        // append recreates it before appending.
        let doc = amend(BASE, "<chummer><qualities><quality><name>B</name><bonus><z>3</z></bonus></quality></qualities></chummer>");
        assert_eq!(quality(&doc, "B").unwrap().path("bonus/z").unwrap().text(), "3");
    }

    #[test]
    fn append_and_regexreplace_text() {
        let doc = amend(
            BASE,
            r#"<chummer><qualities><quality><name>A</name><karma amendoperation="append">0</karma></quality>
               <quality xpathfilter="name = 'B'"><karma amendoperation="regexreplace" regexpattern="([0-9]+)">2+$1F</karma></quality></qualities></chummer>"#,
        );
        assert_eq!(quality(&doc, "A").unwrap().get("karma"), "50");
        assert_eq!(quality(&doc, "B").unwrap().get("karma"), "2+7F");
    }

    #[test]
    fn override_and_custom_passes() {
        let mut doc = xml::parse(BASE).unwrap();
        let mut w = Vec::new();
        let ov = xml::parse("<chummer><qualities><quality><id>2</id><name>B2</name></quality></qualities></chummer>").unwrap();
        assert_eq!(apply_override(&mut doc, ov, &mut w, "t"), 1);
        assert_eq!(quality(&doc, "2").unwrap().get("name"), "B2");
        let cu = xml::parse("<chummer><qualities><quality><id>1</id><name>dup</name></quality><quality><id>3</id><name>C</name></quality></qualities><categories><category>New</category></categories></chummer>").unwrap();
        apply_custom(&mut doc, cu, &mut w, "t");
        assert!(quality(&doc, "dup").is_none());
        assert_eq!(quality(&doc, "C").unwrap().get("id"), "3");
        assert_eq!(crate::data::categories(&doc), vec!["New".to_owned()]);
        assert!(w.is_empty());
    }

    #[test]
    fn save_keys_and_versions() {
        assert_eq!(Version::parse("1"), Some(Version([1, 0, 0, 0])));
        assert_eq!(Version([1, 0, 0, 0]).to_string(), "1.0");
        let key = "091a9694-4186-4c2d-96fc-d4dc2ae62505>1.0";
        assert_eq!(split_save_key(key).unwrap().0, "091a9694-4186-4c2d-96fc-d4dc2ae62505");
        assert!(split_save_key("Bone Lacing Adds to Body").is_none());
    }
}
