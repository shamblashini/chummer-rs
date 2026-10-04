//! Character export (`ExportCharacter`): the print XML as XML, as JSON
//! (Newtonsoft's `SerializeXmlNode` conventions) or through an export
//! stylesheet from `resources/export` (e.g. Squad Manager).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::character::Character;
use crate::data;
use crate::engine::Engine;
use crate::lang::Language;
use crate::print;
use crate::xml::{Element, Node};

/// Export formats offered besides the stylesheets.
pub const BUILT_IN: &[&str] = &["XML", "JSON"];

/// Export stylesheets in `resources/export`, as (name, path).
pub fn stylesheets() -> Vec<(String, PathBuf)> {
    let Some(dir) = data::resource_dir("export") else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xsl" || x == "xslt"))
        .map(|p| (p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p))
        .collect();
    v.sort();
    v
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("unknown export format {0}")]
    UnknownFormat(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Render(#[from] print::RenderError),
}

/// Export a character to `out` in `format` ("XML", "JSON" or the name of
/// an export stylesheet).
pub fn export(ch: &Character, engine: &Engine, lang: &Language, format: &str, out: &Path) -> Result<(), ExportError> {
    let opts = print::PrintOptions { notes: true, expenses: true, ..Default::default() };
    let xml = print::print_xml_with(ch, engine, lang, opts);
    match format {
        "XML" => std::fs::write(out, xml.to_xml_string())?,
        "JSON" => std::fs::write(out, to_json(&xml))?,
        name => {
            let (_, path) = stylesheets().into_iter().find(|(n, _)| n == name).ok_or_else(|| ExportError::UnknownFormat(name.to_owned()))?;
            print::render(&xml, &path, out)?;
        }
    }
    Ok(())
}

/// JSON with Newtonsoft's XML conventions: the root becomes the only
/// property; repeated child names become arrays; attributes are `@name`;
/// text next to attributes or elements is `#text`; empty elements `null`.
pub fn to_json(root: &Element) -> String {
    let mut s = String::from("{\n");
    let _ = write!(s, "  {}: ", quote(&root.name));
    write_value(&mut s, root, 1);
    s.push_str("\n}\n");
    s
}

/// Writes one JSON value at an indentation depth.
type FieldWriter<'a> = Box<dyn Fn(&mut String, usize) + 'a>;

fn write_value(s: &mut String, e: &Element, depth: usize) {
    let kids: Vec<&Element> = e.elements().collect();
    let text: String = e
        .children
        .iter()
        .filter_map(|n| match n {
            Node::Text(t) | Node::CData(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    if kids.is_empty() && e.attrs.is_empty() {
        if text.is_empty() {
            s.push_str("null");
        } else {
            s.push_str(&quote(&text));
        }
        return;
    }
    // Object: attributes, then text, then children grouped by name.
    let pad = "  ".repeat(depth + 1);
    let mut fields: Vec<(String, FieldWriter<'_>)> = Vec::new();
    for (k, v) in &e.attrs {
        let v = v.clone();
        fields.push((format!("@{k}"), Box::new(move |s: &mut String, _| s.push_str(&quote(&v)))));
    }
    if !text.trim().is_empty() {
        let t = text.clone();
        fields.push(("#text".into(), Box::new(move |s: &mut String, _| s.push_str(&quote(&t)))));
    }
    let mut names: Vec<&str> = Vec::new();
    for k in &kids {
        if !names.contains(&k.name.as_str()) {
            names.push(&k.name);
        }
    }
    for name in names {
        let group: Vec<&Element> = kids.iter().copied().filter(|k| k.name == name).collect();
        fields.push((
            name.to_owned(),
            Box::new(move |s: &mut String, d: usize| {
                if group.len() == 1 {
                    write_value(s, group[0], d);
                } else {
                    let ipad = "  ".repeat(d + 1);
                    s.push_str("[\n");
                    for (i, g) in group.iter().enumerate() {
                        s.push_str(&ipad);
                        write_value(s, g, d + 1);
                        if i + 1 < group.len() {
                            s.push(',');
                        }
                        s.push('\n');
                    }
                    s.push_str(&"  ".repeat(d));
                    s.push(']');
                }
            }),
        ));
    }
    s.push_str("{\n");
    let n = fields.len();
    for (i, (k, f)) in fields.into_iter().enumerate() {
        s.push_str(&pad);
        s.push_str(&quote(&k));
        s.push_str(": ");
        f(s, depth + 1);
        if i + 1 < n {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str(&"  ".repeat(depth));
    s.push('}');
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml;

    #[test]
    fn newtonsoft_conventions() {
        let e = xml::parse(r#"<characters><character><name>A "B"</name><skill>x</skill><skill>y</skill><empty/><w id="1">t</w></character></characters>"#).unwrap();
        let j = to_json(&e);
        assert!(j.contains(r#""name": "A \"B\"""#), "{j}");
        assert!(j.contains(r#""skill": ["#), "{j}");
        assert!(j.contains(r#""empty": null"#), "{j}");
        assert!(j.contains(r#""@id": "1""#) && j.contains(r##""#text": "t""##), "{j}");
    }
}
