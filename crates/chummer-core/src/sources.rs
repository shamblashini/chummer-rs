//! Sourcebook PDFs: link each book code (`SR5`, `CF`, ...) to a PDF file
//! and open it at a rule's page in a native viewer.
//!
//! Chummer5a's model, ported from `CommonFunctions.OpenPdf`: a source like
//! `"CF 54"` names a book code and a printed page. The PDF page is the
//! printed page plus the book's offset, substituted into a viewer command.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::data::{self, DataStore};
use crate::xml::{self, Element};

/// A reference to a printed page, e.g. `SR5 p. 143`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub book: String,
    pub page: i32,
}

impl SourceRef {
    /// From separate `<source>` and `<page>` fields.
    pub fn new(book: &str, page: &str) -> Option<SourceRef> {
        let book = book.trim();
        let page: i32 = page.trim().parse().ok()?;
        (!book.is_empty() && page >= 1).then(|| SourceRef { book: book.to_owned(), page })
    }

    /// From a combined string like `"SR5 143"` or `"CF p. 54"`.
    pub fn parse(s: &str) -> Option<SourceRef> {
        let mut it = s.split_whitespace();
        let book = it.next()?;
        let page = it.find(|t| t.chars().all(|c| c.is_ascii_digit()))?;
        SourceRef::new(book, page)
    }

    /// Source of a data record or saved item (`<source>` + `<page>`).
    pub fn of(e: &Element) -> Option<SourceRef> {
        SourceRef::new(&e.get("source"), &e.get("page"))
    }
}

impl std::fmt::Display for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} p. {}", self.book, self.page)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sourcebook {
    pub path: Option<PathBuf>,
    /// Added to the printed page to get the PDF page.
    pub offset: i32,
}

/// A PDF viewer command. `{page}` and `{path}` are substituted (Chummer's
/// `{localpath}` and `{absolutepath}` are accepted too).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerPreset {
    pub name: &'static str,
    pub binary: &'static str,
    pub template: &'static str,
}

pub const VIEWERS: &[ViewerPreset] = &[
    ViewerPreset { name: "Zathura", binary: "zathura", template: "zathura --page={page} \"{path}\"" },
    ViewerPreset { name: "Sioyek", binary: "sioyek", template: "sioyek --page {page} \"{path}\"" },
    ViewerPreset { name: "Okular", binary: "okular", template: "okular --page {page} \"{path}\"" },
    ViewerPreset { name: "Evince (GNOME Document Viewer)", binary: "evince", template: "evince --page-index={page} \"{path}\"" },
    ViewerPreset { name: "Papers", binary: "papers", template: "papers --page-index={page} \"{path}\"" },
    ViewerPreset { name: "Atril", binary: "atril", template: "atril --page-index={page} \"{path}\"" },
    ViewerPreset { name: "qpdfview", binary: "qpdfview", template: "qpdfview --unique \"{path}#{page}\"" },
    ViewerPreset { name: "MuPDF", binary: "mupdf", template: "mupdf \"{path}\" {page}" },
    ViewerPreset { name: "Firefox", binary: "firefox", template: "firefox \"file://{path}#page={page}\"" },
    ViewerPreset { name: "Chromium", binary: "chromium", template: "chromium \"file://{path}#page={page}\"" },
    ViewerPreset { name: "System default (no page jump)", binary: "xdg-open", template: "xdg-open \"{path}\"" },
];

pub fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(binary)).find(|p| p.is_file())
}

/// Installed viewers, best first.
pub fn installed_viewers() -> Vec<ViewerPreset> {
    VIEWERS.iter().copied().filter(|v| which(v.binary).is_some()).collect()
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("no PDF is linked for {0}; set it in Tools → Sourcebooks")]
    NotLinked(String),
    #[error("PDF for {book} not found: {path}")]
    Missing { book: String, path: PathBuf },
    #[error("no PDF viewer is configured")]
    NoViewer,
    #[error("cannot start the PDF viewer: {0}")]
    Spawn(std::io::Error),
}

/// Linked PDFs and the viewer to use. Saved as XML in the config dir.
#[derive(Debug, Clone, Default)]
pub struct SourcebookLibrary {
    pub books: BTreeMap<String, Sourcebook>,
    pub viewer: String,
}

impl SourcebookLibrary {
    pub fn config_path() -> Option<PathBuf> {
        crate::settings::user_settings_dir().and_then(|d| d.parent().map(|p| p.join("sourcebooks.xml")))
    }

    /// Load the user's library. Picks an installed viewer if none is set.
    pub fn load() -> SourcebookLibrary {
        let mut lib = Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| xml::parse(&s).ok())
            .map(|root| Self::from_xml(&root))
            .unwrap_or_default();
        if lib.viewer.trim().is_empty() {
            lib.viewer = installed_viewers().first().map(|v| v.template.to_owned()).unwrap_or_default();
        }
        lib
    }

    pub fn from_xml(root: &Element) -> SourcebookLibrary {
        let mut lib = SourcebookLibrary { viewer: root.get("viewer"), ..Default::default() };
        if let Some(books) = root.child("books") {
            for b in books.children_named("book") {
                let path = b.child_text("path").filter(|p| !p.trim().is_empty()).map(PathBuf::from);
                lib.books.insert(b.get("code"), Sourcebook { path, offset: b.get_i32("offset").unwrap_or(0) });
            }
        }
        lib
    }

    pub fn to_xml(&self) -> Element {
        let mut root = Element::new("sourcebooks");
        root.push(Element::with_text("viewer", self.viewer.clone()));
        let mut books = Element::new("books");
        for (code, b) in &self.books {
            if b.path.is_none() && b.offset == 0 {
                continue;
            }
            let mut e = Element::new("book");
            e.push(Element::with_text("code", code.clone()));
            e.push(Element::with_text("path", b.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
            e.push(Element::with_text("offset", b.offset.to_string()));
            books.push(e);
        }
        root.push(books);
        root
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::config_path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_xml().to_xml_string())
    }

    pub fn is_linked(&self, book: &str) -> bool {
        self.books.get(book).is_some_and(|b| b.path.is_some())
    }

    pub fn linked_count(&self) -> usize {
        self.books.values().filter(|b| b.path.is_some()).count()
    }

    /// The argv that would open `r`.
    pub fn command_for(&self, r: &SourceRef) -> Result<Vec<String>, OpenError> {
        let book = self.books.get(&r.book).and_then(|b| b.path.as_ref().map(|p| (p, b.offset)));
        let Some((path, offset)) = book else { return Err(OpenError::NotLinked(r.book.clone())) };
        if !path.is_file() {
            return Err(OpenError::Missing { book: r.book.clone(), path: path.clone() });
        }
        if self.viewer.trim().is_empty() {
            return Err(OpenError::NoViewer);
        }
        let page = (r.page + offset).max(1).to_string();
        let path = path.display().to_string();
        let argv: Vec<String> = split_command(&self.viewer)
            .into_iter()
            .map(|a| a.replace("{page}", &page).replace("{path}", &path).replace("{localpath}", &path).replace("{absolutepath}", &path))
            .collect();
        if argv.is_empty() {
            return Err(OpenError::NoViewer);
        }
        Ok(argv)
    }

    /// Open the PDF at the referenced page. The viewer runs detached.
    pub fn open(&self, r: &SourceRef) -> Result<(), OpenError> {
        let argv = self.command_for(r)?;
        Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(OpenError::Spawn)
    }
}

/// Split a command line on whitespace, honouring double and single quotes.
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---------------------------------------------------------------------------
// Book list, import and discovery
// ---------------------------------------------------------------------------

/// A book from `books.xml`.
#[derive(Debug, Clone)]
pub struct BookInfo {
    pub code: String,
    pub name: String,
    /// (printed page, English text on that page) used to find the offset.
    pub match_text: Option<(i32, String)>,
}

pub fn book_list(store: &DataStore) -> Vec<BookInfo> {
    let Ok(doc) = store.doc("books.xml") else { return Vec::new() };
    data::records(&doc, "books", "book")
        .into_iter()
        .map(|r| {
            let match_text = r.el().child("matches").and_then(|m| {
                m.children_named("match")
                    .find(|x| x.get("language") == "en-us")
                    .and_then(|x| Some((x.get_i32("page")?, x.get("text"))))
            });
            BookInfo { code: r.get("code"), name: r.name(), match_text }
        })
        .collect()
}

/// Convert a Windows path as Wine sees it to a Linux path.
pub fn wine_path_to_unix(win: &str, prefix: &Path) -> PathBuf {
    let p = win.replace('\\', "/");
    let lower = p.get(..2).map(str::to_ascii_lowercase);
    match lower.as_deref() {
        Some("z:") => PathBuf::from(&p[2..]),
        Some("c:") => prefix.join("drive_c").join(p[2..].trim_start_matches('/')),
        _ => PathBuf::from(p),
    }
}

/// Unescape a value from a Wine `.reg` file (`\\` and `\"`).
fn reg_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Read the sourcebook links Chummer5a saved in a Wine prefix's registry
/// (`HKCU\Software\Chummer5\Sourcebook`, values `"CODE"="path|offset"`).
/// Returns `(code, path, offset)` for every book with a path.
pub fn import_from_wine(prefix: &Path) -> std::io::Result<Vec<(String, PathBuf, i32)>> {
    let reg = std::fs::read_to_string(prefix.join("user.reg"))?;
    let mut out = Vec::new();
    let mut in_section = false;
    for line in reg.lines() {
        if line.starts_with('[') {
            in_section = line.starts_with("[Software\\\\Chummer5\\\\Sourcebook]");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = line.split_once("\"=\"") else { continue };
        let code = reg_unescape(k.trim_start_matches('"'));
        let value = reg_unescape(v.trim_end_matches('"'));
        let (path, offset) = value.rsplit_once('|').unwrap_or((&value, "0"));
        if path.trim().is_empty() {
            continue;
        }
        out.push((code, wine_path_to_unix(path, prefix), offset.trim().parse().unwrap_or(0)));
    }
    Ok(out)
}

/// Wine prefixes that have Chummer5a settings, for the import button.
pub fn find_wine_prefixes() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut candidates = vec![home.join(".wine")];
    if let Ok(rd) = std::fs::read_dir(home.join(".local/share/wineprefixes")) {
        candidates.extend(rd.flatten().map(|e| e.path()));
    }
    // Steam Proton prefixes
    if let Ok(rd) = std::fs::read_dir(home.join(".local/share/Steam/steamapps/compatdata")) {
        candidates.extend(rd.flatten().map(|e| e.path().join("pfx")));
    }
    if let Ok(env) = std::env::var("WINEPREFIX") {
        candidates.insert(0, PathBuf::from(env));
    }
    candidates
        .into_iter()
        .filter(|p| std::fs::read_to_string(p.join("user.reg")).is_ok_and(|r| r.contains("[Software\\\\Chummer5\\\\Sourcebook]")))
        .collect()
}

fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Match PDF files in a folder to books by title. Returns `(code, path)`.
/// A file matches the book with the longest title contained in its name;
/// the core rulebook also matches "core rulebook"/"core rules".
pub fn scan_folder(dir: &Path, books: &[BookInfo]) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    collect_pdfs(dir, 0, &mut files);
    let titles: Vec<(String, &BookInfo)> = books
        .iter()
        .map(|b| (normalize(b.name.trim_start_matches("The ").trim_start_matches("Shadowrun 5th Edition")), b))
        .filter(|(t, _)| t.len() >= 4)
        .collect();
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for f in files {
        let stem = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // Skip other editions; data codes are all SR5.
        let lower = stem.to_lowercase();
        if ["3e", "4e", "6e", "sr6", "sr4", "anarchy", "errata"].iter().any(|e| lower.contains(e)) {
            continue;
        }
        let n = normalize(&stem);
        let hit = if n.contains("corerulebook") || n.contains("corerules") {
            books.iter().find(|b| b.code == "SR5")
        } else {
            titles.iter().filter(|(t, _)| n.contains(t.as_str())).max_by_key(|(t, _)| t.len()).map(|(_, b)| *b)
        };
        if let Some(b) = hit {
            if !out.iter().any(|(c, _)| *c == b.code) {
                out.push((b.code.clone(), f));
            }
        }
    }
    out
}

fn collect_pdfs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() && depth < 3 {
            collect_pdfs(&p, depth + 1, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            out.push(p);
        }
    }
}

/// Find a book's page offset by locating its known text with `pdftotext`
/// (poppler). Searches PDF pages around the printed page. `None` when
/// `pdftotext` is missing or the text is not found.
pub fn detect_offset(pdf: &Path, book: &BookInfo) -> Option<i32> {
    let (page, text) = book.match_text.as_ref()?;
    which("pdftotext")?;
    let needle = normalize(text);
    if needle.len() < 8 {
        return None;
    }
    // Most offsets are small; try those first.
    let mut tries: Vec<i32> = (0..=12).flat_map(|d| [d, -d]).collect();
    tries.dedup();
    for off in tries {
        let p = page + off;
        if p < 1 {
            continue;
        }
        let out = Command::new("pdftotext")
            .args(["-q", "-f", &p.to_string(), "-l", &p.to_string()])
            .arg(pdf)
            .arg("-")
            .output()
            .ok()?;
        if normalize(&String::from_utf8_lossy(&out.stdout)).contains(&needle) {
            return Some(off);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refs() {
        assert_eq!(SourceRef::parse("SR5 143"), Some(SourceRef { book: "SR5".into(), page: 143 }));
        assert_eq!(SourceRef::parse("CF p. 54").unwrap().page, 54);
        assert_eq!(SourceRef::new("SR5", "abc"), None);
        assert_eq!(SourceRef::new("", "3"), None);
        assert_eq!(SourceRef::new("RF", "0"), None);
    }

    #[test]
    fn command_splitting() {
        assert_eq!(split_command(r#"evince --page-index={page} "{path}""#), vec!["evince", "--page-index={page}", "{path}"]);
        assert_eq!(split_command(r#"a "b c" 'd e' """#), vec!["a", "b c", "d e", ""]);
    }

    #[test]
    fn command_substitution_and_offset() {
        let dir = std::env::temp_dir().join(format!("chummer-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("My Book.pdf");
        std::fs::write(&pdf, b"%PDF-1.4").unwrap();
        let mut lib = SourcebookLibrary { viewer: "viewer --page={page} \"{path}\"".into(), ..Default::default() };
        lib.books.insert("CF".into(), Sourcebook { path: Some(pdf.clone()), offset: 1 });
        let argv = lib.command_for(&SourceRef { book: "CF".into(), page: 54 }).unwrap();
        assert_eq!(argv, vec!["viewer".to_owned(), "--page=55".into(), pdf.display().to_string()]);
        assert!(matches!(lib.command_for(&SourceRef { book: "SR5".into(), page: 1 }), Err(OpenError::NotLinked(_))));
        let back = SourcebookLibrary::from_xml(&lib.to_xml());
        assert_eq!(back.books, lib.books);
        assert_eq!(back.viewer, lib.viewer);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wine_registry_import() {
        let dir = std::env::temp_dir().join(format!("chummer-reg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("user.reg"),
            "[Software\\\\Chummer5] 1\n\"pdfapppath\"=\"x\"\n\n[Software\\\\Chummer5\\\\Sourcebook] 1\n#time=1\n\"2050\"=\"|0\"\n\"CF\"=\"Z:\\\\home\\\\me\\\\Chrome Flesh.pdf|1\"\n\"RG\"=\"C:\\\\books\\\\rg.pdf|-2\"\n\n[Software\\\\Other] 1\n\"X\"=\"Z:\\\\no.pdf|0\"\n",
        )
        .unwrap();
        let got = import_from_wine(&dir).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("CF".into(), PathBuf::from("/home/me/Chrome Flesh.pdf"), 1));
        assert_eq!(got[1], ("RG".into(), dir.join("drive_c/books/rg.pdf"), -2));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn folder_scan_matches_titles() {
        let dir = std::env::temp_dir().join(format!("chummer-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["Shadowrun 5e - Chrome Flesh.pdf", "Shadowrun 5e - Core Rulebook (2nd Printing).pdf", "Shadowrun 4e - Vice.pdf", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let books = vec![
            BookInfo { code: "SR5".into(), name: "Shadowrun 5th Edition".into(), match_text: None },
            BookInfo { code: "CF".into(), name: "Chrome Flesh".into(), match_text: None },
            BookInfo { code: "V".into(), name: "Vice".into(), match_text: None },
        ];
        let mut got: Vec<String> = scan_folder(&dir, &books).into_iter().map(|(c, _)| c).collect();
        got.sort();
        assert_eq!(got, vec!["CF", "SR5"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
