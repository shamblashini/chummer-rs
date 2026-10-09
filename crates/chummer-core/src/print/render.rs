//! Turning a print XML into HTML with Chummer's XSLT sheets.
//!
//! Chummer5a runs the sheets through .NET's `XslCompiledTransform`
//! (`CharacterSheetViewer.AsyncGenerateOutput`). Here the system
//! `xsltproc` (libxslt) does the same work. Two differences need a fix-up,
//! done on a private copy of the stylesheets so `resources/sheets` stays
//! byte-for-byte Chummer's:
//!
//! - libxslt rejects `xsl:import href="Shadowrun 5 set.xslt"`: a space is
//!   not valid in a URI reference. The copy gives every file a plain name.
//! - libxslt has no `msxsl:node-set()`. It has the same function as
//!   `exsl:node-set()`, so the copy maps the `msxsl` prefix to EXSLT.
//!
//! Windows has no xsltproc, so the Windows packages carry one (MSYS2's
//! build, with its DLLs) in an `xsltproc` folder next to `chummer-rs.exe`;
//! [`xsltproc_path`] finds it before looking on `PATH`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::xml::{self, Element};

/// Sheet Chummer5a selects when none is set
/// (`GlobalSettings.DefaultCharacterSheetDefaultValue`).
pub const DEFAULT_SHEET: &str = "Shadowrun 5 (Skills grouped by Rating greater 0)";

const MSXSL_NS: &str = "urn:schemas-microsoft-com:xslt";
const EXSL_NS: &str = "http://exslt.org/common";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("sheet {0} not found")]
    NoSheet(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("cannot run xsltproc: {0}. {hint}", hint = XSLTPROC_HINT)]
    NoXsltproc(std::io::Error),
    #[error("no HTML-to-PDF converter found (tried chromium, google-chrome, wkhtmltopdf, weasyprint); open the HTML in a browser and print it to PDF")]
    NoPdfConverter,
    #[error("{0} failed to write the PDF:\n{1}")]
    Pdf(String, String),
    #[error("xsltproc failed on {sheet} (exit {code:?}):\n{stderr}")]
    Xslt { sheet: PathBuf, code: Option<i32>, stderr: String },
}

/// What a successful transform printed on stderr (libxslt warnings and
/// `xsl:message` output). Empty for a clean run.
#[derive(Debug, Default, Clone)]
pub struct RenderReport {
    pub warnings: String,
}

/// Transform `xml` (from [`super::print_xml`]) with the sheet at
/// `xsl_path` and write the HTML to `out`.
pub fn render(xml: &Element, xsl_path: &Path, out: &Path) -> Result<(), RenderError> {
    render_report(xml, xsl_path, out).map(|_| ())
}

/// As [`render`], and return the transform's warnings.
pub fn render_report(xml: &Element, xsl_path: &Path, out: &Path) -> Result<RenderReport, RenderError> {
    if !xsl_path.is_file() {
        return Err(RenderError::NoSheet(xsl_path.to_owned()));
    }
    let dir = TempDir::new()?;
    let fallback = bundled_twin(xsl_path, crate::paths::user_dir(crate::paths::UserDir::Sheets).as_deref(), crate::data::resource_dir("sheets").as_deref());
    let main_xsl = copy_stylesheet(xsl_path, dir.path(), fallback.as_deref())?;
    debug_assert_eq!(main_xsl, dir.path().join("sheet0.xslt"));
    write(&dir.path().join("print.xml"), xml.to_xml_string().as_bytes())?;
    let report = run_xsltproc(dir.path(), xsl_path)?;
    let html = dir.path().join("out.html");
    std::fs::copy(&html, out).map_err(|e| RenderError::Io(out.to_owned(), e))?;
    Ok(report)
}

/// How to get xsltproc, for the "cannot run xsltproc" error.
#[cfg(windows)]
const XSLTPROC_HINT: &str = "Character sheets use the xsltproc.exe that comes with chummer-rs (in its xsltproc folder); reinstall chummer-rs to restore it";
#[cfg(target_os = "macos")]
const XSLTPROC_HINT: &str = "Character sheets need xsltproc, which macOS includes in /usr/bin; if it is missing, install it with `brew install libxslt` or `xcode-select --install`";
#[cfg(not(any(windows, target_os = "macos")))]
const XSLTPROC_HINT: &str = "Character sheets need xsltproc. Install it with your package manager: `sudo apt install xsltproc` (Debian, Ubuntu), `sudo dnf install libxslt` (Fedora), `sudo pacman -S libxslt` (Arch)";

const XSLTPROC_EXE: &str = if cfg!(windows) { "xsltproc.exe" } else { "xsltproc" };

/// The xsltproc to run: `$CHUMMER_XSLTPROC`, else one shipped with
/// chummer-rs (an `xsltproc` folder next to the executable, or next to its
/// resources), else the first on `PATH`. `None` when there is none.
pub fn xsltproc_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CHUMMER_XSLTPROC").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_owned));
    let bundled = exe_dir.into_iter().flat_map(|d| {
        [
            d.join("xsltproc").join(XSLTPROC_EXE),
            d.join(XSLTPROC_EXE),
            d.join("../share/chummer-rs/xsltproc").join(XSLTPROC_EXE),
            d.join("../Resources/xsltproc").join(XSLTPROC_EXE),
        ]
    });
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).map(|d| d.join(XSLTPROC_EXE)).collect::<Vec<_>>());
    bundled.chain(on_path).find(|p| p.is_file())
}

/// `xsltproc` with arguments passed directly (no shell), in `dir`:
/// `sheet0.xslt` + `print.xml` -> `out.html`. Plain relative names, so the
/// user's temp path (perhaps not ASCII, which a Windows build of libxml2
/// may not read from its command line) never reaches xsltproc.
fn run_xsltproc(dir: &Path, shown: &Path) -> Result<RenderReport, RenderError> {
    let exe = xsltproc_path().ok_or_else(|| RenderError::NoXsltproc(std::io::Error::new(std::io::ErrorKind::NotFound, "not found")))?;
    let mut cmd = Command::new(exe);
    // The GUI has no console; without this each run flashes a console window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd
        .current_dir(dir)
        .args(["--nonet", "--novalid", "--output", "out.html", "sheet0.xslt", "print.xml"])
        .output()
        .map_err(RenderError::NoXsltproc)?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(RenderError::Xslt { sheet: shown.to_owned(), code: output.status.code(), stderr });
    }
    Ok(RenderReport { warnings: stderr })
}

// ---------------------------------------------------------------------------
// Stylesheet copy
// ---------------------------------------------------------------------------

/// For a sheet in the user's sheets folder, the matching bundled folder
/// (`<user>/de-de/x.xsl` -> `<bundled>/de-de`): imports not found next to
/// the user's sheet are taken from there.
fn bundled_twin(xsl: &Path, user: Option<&Path>, bundled: Option<&Path>) -> Option<PathBuf> {
    let user = user?.canonicalize().ok()?;
    let parent = xsl.canonicalize().ok()?.parent()?.to_owned();
    let rel = parent.strip_prefix(&user).ok()?;
    Some(bundled?.join(rel))
}

/// Copy `xsl` and everything it imports or includes into `dir` as
/// `sheet0.xslt`, `sheet1.xslt`, ..., rewriting the hrefs to match. An
/// href that does not resolve is looked up in `fallback` (the bundled
/// sheets, for a user sheet). Returns the path of the copied main sheet.
fn copy_stylesheet(xsl: &Path, dir: &Path, fallback: Option<&Path>) -> Result<PathBuf, RenderError> {
    let mut names: HashMap<PathBuf, String> = HashMap::new();
    let mut queue = vec![canonical(xsl)?];
    names.insert(queue[0].clone(), "sheet0.xslt".into());
    while let Some(src) = queue.pop() {
        let text = std::fs::read_to_string(&src).map_err(|e| RenderError::Io(src.clone(), e))?;
        let base = src.parent().unwrap_or(Path::new("."));
        let rewritten = rewrite_hrefs(&text, |href| {
            let target = canonical(&base.join(href)).ok().or_else(|| canonical(&fallback?.join(href)).ok())?;
            let next = names.len();
            let name = names.entry(target.clone()).or_insert_with(|| {
                queue.push(target);
                format!("sheet{next}.xslt")
            });
            Some(name.clone())
        });
        let patched = rewritten.replace(MSXSL_NS, EXSL_NS);
        write(&dir.join(&names[&src]), patched.as_bytes())?;
    }
    Ok(dir.join("sheet0.xslt"))
}

fn canonical(p: &Path) -> Result<PathBuf, RenderError> {
    p.canonicalize().map_err(|e| RenderError::Io(p.to_owned(), e))
}

/// Replace the `href` of every `xsl:import` / `xsl:include` with what
/// `map` returns for it (unchanged when `map` returns `None`).
fn rewrite_hrefs(text: &str, mut map: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = find_import(rest) {
        let tag_end = rest[start..].find('>').map_or(rest.len(), |i| start + i);
        let tag = &rest[start..tag_end];
        out.push_str(&rest[..start]);
        out.push_str(&rewrite_tag(tag, &mut map));
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    out
}

fn find_import(s: &str) -> Option<usize> {
    [s.find("<xsl:import"), s.find("<xsl:include")].into_iter().flatten().min()
}

fn rewrite_tag(tag: &str, map: &mut impl FnMut(&str) -> Option<String>) -> String {
    for quote in ['"', '\''] {
        let key = format!("href={quote}");
        let Some(i) = tag.find(&key) else { continue };
        let vstart = i + key.len();
        let Some(len) = tag[vstart..].find(quote) else { continue };
        let href = xml_unescape(&tag[vstart..vstart + len]);
        if let Some(new) = map(&href) {
            return format!("{}{new}{}", &tag[..vstart], &tag[vstart + len..]);
        }
    }
    tag.to_owned()
}

fn xml_unescape(s: &str) -> String {
    s.replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), RenderError> {
    std::fs::write(path, bytes).map_err(|e| RenderError::Io(path.to_owned(), e))
}

/// A directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir, RenderError> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("chummer-rs-sheet-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).map_err(|e| RenderError::Io(p.clone(), e))?;
        Ok(TempDir(p))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

/// Converters tried by [`html_to_pdf`], with their arguments before and
/// after the input/output paths. Chummer5a prints through its embedded
/// browser; here an installed headless browser or converter does it.
const PDF_CONVERTERS: &[(&str, &[&str])] = &[
    ("chromium", &["--headless", "--disable-gpu", "--no-pdf-header-footer"]),
    ("google-chrome", &["--headless", "--disable-gpu", "--no-pdf-header-footer"]),
    ("wkhtmltopdf", &["--quiet", "--enable-local-file-access"]),
    ("weasyprint", &[]),
];

/// The first converter on `PATH`, if any.
pub fn pdf_converter() -> Option<&'static str> {
    PDF_CONVERTERS.iter().map(|(c, _)| *c).find(|c| on_path(c))
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// Convert a rendered sheet to PDF with [`pdf_converter`].
pub fn html_to_pdf(html: &Path, pdf: &Path) -> Result<(), RenderError> {
    let Some((cmd, args)) = PDF_CONVERTERS.iter().find(|(c, _)| on_path(c)) else {
        return Err(RenderError::NoPdfConverter);
    };
    let html = canonical(html)?;
    let mut c = Command::new(cmd);
    c.args(*args);
    if cmd.contains("chrom") {
        let mut flag = std::ffi::OsString::from("--print-to-pdf=");
        flag.push(pdf);
        c.arg(flag).arg(format!("file://{}", html.display()));
    } else {
        c.arg(&html).arg(pdf);
    }
    let output = c.output().map_err(|e| RenderError::Pdf(cmd.to_string(), e.to_string()))?;
    if !output.status.success() || !pdf.is_file() {
        return Err(RenderError::Pdf(cmd.to_string(), String::from_utf8_lossy(&output.stderr).into_owned()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sheet list
// ---------------------------------------------------------------------------

/// Sheets offered for a language, as `(display name, .xsl path)`, from
/// `data/sheets.xml` (`XmlManager.GetXslFilesFromLocalDirectoryAsync`),
/// then the user's own from the sheets folder in the data root
/// ([`crate::paths`]). English sheets live in `sheets/`; others in
/// `sheets/<lang>/`.
pub fn available_sheets(lang_code: &str) -> Vec<(String, PathBuf)> {
    let user = crate::paths::user_dir(crate::paths::UserDir::Sheets);
    match (crate::data::resource_dir("data"), crate::data::resource_dir("sheets")) {
        (Some(data), Some(sheets)) => available_sheets_with(&data, &sheets, user.as_deref(), lang_code),
        _ => user.map(|u| user_sheets(Vec::new(), &u, lang_code)).unwrap_or_default(),
    }
}

/// As [`available_sheets`], with explicit `data` and `sheets` directories
/// and no user sheets.
pub fn available_sheets_in(data_dir: &Path, sheets_dir: &Path, lang_code: &str) -> Vec<(String, PathBuf)> {
    available_sheets_with(data_dir, sheets_dir, None, lang_code)
}

/// The bundled sheets, then every `*.xsl` in the user's sheets folder
/// (`user_dir`, or its `<lang>` subfolder). A user sheet whose file name
/// matches a bundled one replaces it (keeping its place and name); the
/// others follow, named after their file and sorted.
pub fn available_sheets_with(data_dir: &Path, sheets_dir: &Path, user_dir: Option<&Path>, lang_code: &str) -> Vec<(String, PathBuf)> {
    let bundled = bundled_sheets(data_dir, sheets_dir, lang_code);
    match user_dir {
        // Never list the bundled folder twice (CHUMMER_RESOURCES or
        // XDG_DATA_HOME pointing at it).
        Some(u) if !same_dir(u, sheets_dir) => user_sheets(bundled, u, lang_code),
        _ => bundled,
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn lang_dir(root: &Path, lang_code: &str) -> PathBuf {
    if lang_code.eq_ignore_ascii_case("en-us") {
        root.to_owned()
    } else {
        root.join(lang_code)
    }
}

fn user_sheets(mut out: Vec<(String, PathBuf)>, user_root: &Path, lang_code: &str) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(lang_dir(user_root, lang_code)) else { return out };
    let mut mine: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xsl"))).collect();
    mine.sort_by_key(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()));
    for p in mine {
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
        match out.iter_mut().find(|(_, b)| stem_is(b, &stem)) {
            Some(entry) => entry.1 = p,
            None => out.push((stem, p)),
        }
    }
    out
}

fn bundled_sheets(data_dir: &Path, sheets_dir: &Path, lang_code: &str) -> Vec<(String, PathBuf)> {
    let Some(root) = std::fs::read_to_string(data_dir.join("sheets.xml")).ok().and_then(|s| xml::parse(&s).ok()) else {
        return Vec::new();
    };
    let Some(list) = root.children_named("sheets").find(|s| s.attr("lang").is_some_and(|l| l.eq_ignore_ascii_case(lang_code))) else {
        return Vec::new();
    };
    let dir = lang_dir(sheets_dir, lang_code);
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for sheet in list.children_named("sheet").filter(|s| s.child("hide").is_none()) {
        let file = sheet.get("filename");
        if file.is_empty() || seen.contains(&file) {
            continue;
        }
        // sheets.xml lists a few translated sheets that are not shipped
        // (Chummer then shows "File not found"); leave those out.
        let path = dir.join(format!("{file}.xsl"));
        if !path.is_file() {
            continue;
        }
        let name = sheet.child_text("name").unwrap_or_else(|| file.clone());
        out.push((name, path));
        seen.push(file);
    }
    out
}

/// Find a sheet by display name or file name (case-insensitive).
pub fn find_sheet(lang_code: &str, name: &str) -> Option<PathBuf> {
    available_sheets(lang_code).into_iter().find(|(n, p)| n.eq_ignore_ascii_case(name) || stem_is(p, name)).map(|(_, p)| p)
}

fn stem_is(p: &Path, name: &str) -> bool {
    p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrefs_are_rewritten() {
        let src = r#"<xsl:import href="xz.language.xslt" />
  <xsl:import href="../Shadowrun 5 set.xslt"/><xsl:include href='a &amp; b.xslt'/>"#;
        let mut seen = Vec::new();
        let out = rewrite_hrefs(src, |h| {
            seen.push(h.to_owned());
            Some(format!("s{}.xslt", seen.len()))
        });
        assert_eq!(seen, ["xz.language.xslt", "../Shadowrun 5 set.xslt", "a & b.xslt"]);
        assert!(out.contains(r#"href="s1.xslt" />"#));
        assert!(out.contains(r#"href="s2.xslt"/>"#));
        assert!(out.contains("href='s3.xslt'/>"));
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chummer-sheets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn user_sheets_are_listed_and_override() {
        let res = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
        let (data, sheets) = (res.join("data"), res.join("sheets"));
        let user = tmp("user");
        std::fs::write(user.join("Shadowrun 5 (Core).xsl"), "<x/>").unwrap();
        std::fs::write(user.join("My Sheet.XSL"), "<x/>").unwrap();
        std::fs::write(user.join("helper.xslt"), "<x/>").unwrap();
        std::fs::write(user.join("README.txt"), "").unwrap();
        std::fs::create_dir_all(user.join("de-de")).unwrap();
        std::fs::write(user.join("de-de/Mein Bogen.xsl"), "<x/>").unwrap();

        let bundled = available_sheets_in(&data, &sheets, "en-us");
        let all = available_sheets_with(&data, &sheets, Some(&user), "en-us");
        assert_eq!(all.len(), bundled.len() + 1);
        let core = bundled.iter().position(|(_, p)| stem_is(p, "Shadowrun 5 (Core)")).unwrap();
        assert_eq!(all[core].0, bundled[core].0, "keeps the bundled name and place");
        assert_eq!(all[core].1, user.join("Shadowrun 5 (Core).xsl"));
        assert_eq!(all.last().unwrap(), &("My Sheet".to_owned(), user.join("My Sheet.XSL")));
        // Per-language subfolders.
        let de = available_sheets_with(&data, &sheets, Some(&user), "de-de");
        assert!(de.iter().any(|(n, p)| n == "Mein Bogen" && *p == user.join("de-de/Mein Bogen.xsl")));
        assert!(!de.iter().any(|(n, _)| n == "My Sheet"));
        // The bundled folder as the user folder: listed once.
        assert_eq!(available_sheets_with(&data, &sheets, Some(&sheets), "en-us"), bundled);
        // No user folder: just the bundled ones.
        assert_eq!(available_sheets_with(&data, &sheets, Some(&user.join("missing")), "en-us"), bundled);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn user_sheet_imports_fall_back_to_the_bundled_ones() {
        let t = tmp("fallback");
        let (user, bundled, out) = (t.join("user"), t.join("bundled"), t.join("out"));
        for d in [user.join("de-de"), bundled.join("de-de"), out.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(user.join("de-de/Mine.xsl"), r#"<xsl:import href="Base set.xslt"/><xsl:include href="local.xslt"/>"#).unwrap();
        std::fs::write(user.join("de-de/local.xslt"), "local").unwrap();
        std::fs::write(bundled.join("de-de/Base set.xslt"), "base").unwrap();
        let xsl = user.join("de-de/Mine.xsl");
        let twin = bundled_twin(&xsl, Some(&user), Some(&bundled)).unwrap();
        assert_eq!(twin, bundled.join("de-de"));
        // A bundled sheet has no twin.
        assert_eq!(bundled_twin(&bundled.join("de-de/Base set.xslt"), Some(&user), Some(&bundled)), None);
        copy_stylesheet(&xsl, &out, Some(&twin)).unwrap();
        let main = std::fs::read_to_string(out.join("sheet0.xslt")).unwrap();
        assert!(main.contains(r#"href="sheet1.xslt""#) && main.contains(r#"href="sheet2.xslt""#), "{main}");
        let copied: Vec<String> = (1..=2).map(|i| std::fs::read_to_string(out.join(format!("sheet{i}.xslt"))).unwrap()).collect();
        assert!(copied.contains(&"base".to_owned()) && copied.contains(&"local".to_owned()));
        let _ = std::fs::remove_dir_all(&t);
    }
}
