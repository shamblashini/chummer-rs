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
    #[error("cannot run xsltproc (install libxslt): {0}")]
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
    let main_xsl = copy_stylesheet(xsl_path, dir.path())?;
    let input = dir.path().join("print.xml");
    write(&input, xml.to_xml_string().as_bytes())?;
    run_xsltproc(&main_xsl, &input, out, xsl_path)
}

/// `xsltproc` with arguments passed directly (no shell).
fn run_xsltproc(xsl: &Path, input: &Path, out: &Path, shown: &Path) -> Result<RenderReport, RenderError> {
    let output = Command::new("xsltproc")
        .arg("--nonet")
        .arg("--novalid")
        .arg("--output")
        .arg(out)
        .arg(xsl)
        .arg(input)
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

/// Copy `xsl` and everything it imports or includes into `dir` as
/// `sheet0.xslt`, `sheet1.xslt`, ..., rewriting the hrefs to match.
/// Returns the path of the copied main sheet.
fn copy_stylesheet(xsl: &Path, dir: &Path) -> Result<PathBuf, RenderError> {
    let mut names: HashMap<PathBuf, String> = HashMap::new();
    let mut queue = vec![canonical(xsl)?];
    names.insert(queue[0].clone(), "sheet0.xslt".into());
    while let Some(src) = queue.pop() {
        let text = std::fs::read_to_string(&src).map_err(|e| RenderError::Io(src.clone(), e))?;
        let base = src.parent().unwrap_or(Path::new("."));
        let rewritten = rewrite_hrefs(&text, |href| {
            let target = canonical(&base.join(href)).ok()?;
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
        c.arg(flag).arg(&html);
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
/// `data/sheets.xml` (`XmlManager.GetXslFilesFromLocalDirectoryAsync`).
/// English sheets live in `sheets/`; others in `sheets/<lang>/`.
pub fn available_sheets(lang_code: &str) -> Vec<(String, PathBuf)> {
    match (crate::data::resource_dir("data"), crate::data::resource_dir("sheets")) {
        (Some(data), Some(sheets)) => available_sheets_in(&data, &sheets, lang_code),
        _ => Vec::new(),
    }
}

/// As [`available_sheets`], with explicit `data` and `sheets` directories.
pub fn available_sheets_in(data_dir: &Path, sheets_dir: &Path, lang_code: &str) -> Vec<(String, PathBuf)> {
    let Some(root) = std::fs::read_to_string(data_dir.join("sheets.xml")).ok().and_then(|s| xml::parse(&s).ok()) else {
        return Vec::new();
    };
    let Some(list) = root.children_named("sheets").find(|s| s.attr("lang").is_some_and(|l| l.eq_ignore_ascii_case(lang_code))) else {
        return Vec::new();
    };
    let dir = if lang_code.eq_ignore_ascii_case("en-us") { sheets_dir.to_owned() } else { sheets_dir.join(lang_code) };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for sheet in list.children_named("sheet").filter(|s| s.child("hide").is_none()) {
        let file = sheet.get("filename");
        if file.is_empty() || seen.contains(&file) {
            continue;
        }
        let name = sheet.child_text("name").unwrap_or_else(|| file.clone());
        out.push((name, dir.join(format!("{file}.xsl"))));
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
}
