//! A small owned XML tree.
//!
//! Chummer's data files and `.chum5` saves are plain XML. Both the data
//! layer and the character loader work on this tree, so a character can be
//! saved back without losing elements this port does not model yet.

use std::fmt::Write as _;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

#[derive(Debug, thiserror::Error)]
pub enum XmlError {
    #[error("XML parse error at byte {pos}: {msg}")]
    Parse { pos: u64, msg: String },
    #[error("document has no root element")]
    NoRoot,
    #[error("unbalanced end tag </{0}>")]
    Unbalanced(String),
    #[error("elements nested more than {MAX_DEPTH} deep")]
    TooDeep,
}

/// Deepest element nesting [`parse`] accepts. Chummer's files nest a few
/// dozen levels; the tree is written, cloned and dropped recursively, so a
/// hostile file nested a hundred thousand deep would overflow the stack
/// (an abort, not an error).
pub const MAX_DEPTH: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), attrs: Vec::new(), children: Vec::new() }
    }

    /// Element with a single text child.
    pub fn with_text(name: impl Into<String>, text: impl Into<String>) -> Self {
        let mut e = Self::new(name);
        e.set_text(text);
        e
    }

    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((key.to_owned(), value)),
        }
    }

    /// Iterator over direct child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// First direct child element with this name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name)
    }

    /// First child with this name, created (appended) if missing.
    pub fn child_or_insert(&mut self, name: &str) -> &mut Element {
        let idx = self.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == name));
        let idx = match idx {
            Some(i) => i,
            None => {
                self.children.push(Node::Element(Element::new(name)));
                self.children.len() - 1
            }
        };
        match &mut self.children[idx] {
            Node::Element(e) => e,
            _ => unreachable!(),
        }
    }

    /// All direct child elements with this name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.name == name)
    }

    /// Walk a `/`-separated path of child names, e.g. `"bonus/specificattribute"`.
    pub fn path(&self, path: &str) -> Option<&Element> {
        let mut cur = self;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = cur.child(part)?;
        }
        Some(cur)
    }

    /// Concatenated text and CDATA of this element's direct children.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) | Node::CData(t) => out.push_str(t),
                _ => {}
            }
        }
        out
    }

    /// Text of a child element, `None` when the child is absent.
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name).map(Element::text)
    }

    /// Text of a child element, empty string when absent.
    pub fn get(&self, name: &str) -> String {
        self.child_text(name).unwrap_or_default()
    }

    pub fn get_i32(&self, name: &str) -> Option<i32> {
        self.child_text(name).and_then(|t| parse_int(&t))
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.child_text(name).and_then(|t| parse_f64(&t))
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.child_text(name).map(|t| parse_bool(&t))
    }

    /// Replace all children with a single text node.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.children.clear();
        if !text.is_empty() {
            self.children.push(Node::Text(text));
        }
    }

    /// Set the text of a named child, creating the child if needed.
    pub fn set_child_text(&mut self, name: &str, text: impl Into<String>) {
        self.child_or_insert(name).set_text(text);
    }

    pub fn push(&mut self, child: Element) {
        self.children.push(Node::Element(child));
    }

    /// Remove direct child elements with this name. Returns how many went.
    pub fn remove_children(&mut self, name: &str) -> usize {
        let before = self.children.len();
        self.children.retain(|n| !matches!(n, Node::Element(e) if e.name == name));
        before - self.children.len()
    }

    /// Depth-first search for descendants (not self) with this name.
    pub fn descendants<'a>(&'a self, name: &'a str, out: &mut Vec<&'a Element>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            }
            e.descendants(name, out);
        }
    }

    pub fn to_xml_string(&self) -> String {
        let mut s = String::with_capacity(4096);
        s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        self.write_into(&mut s, 0);
        s.push('\n');
        s
    }

    fn write_into(&self, out: &mut String, depth: usize) {
        indent(out, depth);
        out.push('<');
        out.push_str(&self.name);
        for (k, v) in &self.attrs {
            let _ = write!(out, " {k}=\"{}\"", escape(v, true));
        }
        if self.children.is_empty() {
            out.push_str(" />");
            return;
        }
        out.push('>');
        let has_elements = self.children.iter().any(|n| matches!(n, Node::Element(_)));
        if has_elements {
            for n in &self.children {
                match n {
                    Node::Element(e) => {
                        out.push('\n');
                        e.write_into(out, depth + 1);
                    }
                    Node::Text(t) => {
                        // Mixed content is rare in Chummer files; keep the text.
                        out.push_str(&escape(t, false));
                    }
                    Node::CData(t) => {
                        let _ = write!(out, "<![CDATA[{t}]]>");
                    }
                    Node::Comment(c) => {
                        out.push('\n');
                        indent(out, depth + 1);
                        let _ = write!(out, "<!--{c}-->");
                    }
                }
            }
            out.push('\n');
            indent(out, depth);
        } else {
            for n in &self.children {
                match n {
                    Node::Text(t) => out.push_str(&escape(t, false)),
                    Node::CData(t) => {
                        let _ = write!(out, "<![CDATA[{t}]]>");
                    }
                    Node::Comment(c) => {
                        let _ = write!(out, "<!--{c}-->");
                    }
                    Node::Element(_) => unreachable!(),
                }
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

pub(crate) fn escape(s: &str, attr: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#xA;"),
            _ => out.push(c),
        }
    }
    out
}

/// Whether `s` can be an element name: a letter or `_`, then letters,
/// digits, `-`, `_` and `.` (no spaces, no markup). Commands that name
/// a field check this, or a save would not load again.
pub fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The largest magnitude an integer read from a file, or one the rules
/// math rounds out of a decimal, may have. Such integers are ratings,
/// levels, karma and counts (the largest in the data is 1,000,000,
/// Chummer's "no limit" rating); prices stay decimals. A file with
/// `<base>2147483647</base>` is clamped here, so the rules math, which
/// adds these values as `i32`, cannot overflow (LB-44). Commands are held
/// to the same range ([`crate::command::Command::check_numbers`]).
pub const NUM_LIMIT: i32 = 1_000_000;

/// Parse an integer the way .NET's lenient invariant parsing would accept it:
/// surrounding whitespace and a leading `+` are fine. Values are clamped
/// to ±[`NUM_LIMIT`].
pub fn parse_int(s: &str) -> Option<i32> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    let v: i64 = t.parse().ok()?;
    Some(v.clamp(-i64::from(NUM_LIMIT), i64::from(NUM_LIMIT)) as i32)
}

/// A float from a file: NaN and the infinities are not numbers any rule
/// can use, so they read as absent (LB-45). The magnitude is kept
/// (Chummer writes `decimal.MinValue` as a "not set" marker); integers
/// derived from floats are clamped where they are rounded (LB-44).
pub fn parse_f64(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    v.is_finite().then_some(v)
}

pub fn parse_bool(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes")
}

fn resolve_entity(name: &str) -> Option<String> {
    Some(match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)?.to_string()
        }
    })
}

/// Decode `&...;` entity references in raw text.
fn unescape_text(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(';').and_then(|j| resolve_entity(&after[..j]).map(|r| (j, r))) {
            Some((j, r)) => {
                out.push_str(&r);
                rest = &after[j + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// quick-xml 0.42 already hands out `str`; this keeps call sites uniform.
fn lossless(s: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(s)
}

fn start_to_element(e: &BytesStart<'_>) -> Element {
    let name = lossless(e.name().as_ref()).into_owned();
    let mut el = Element::new(name);
    for a in e.attributes().flatten() {
        let k = lossless(a.key.as_ref()).into_owned();
        let v = unescape_text(&lossless(&a.value));
        el.attrs.push((k, v));
    }
    el
}

/// Parse a document and return its root element.
///
/// Whitespace-only text between elements is dropped. The writer re-indents
/// output, so this keeps saves tidy without changing meaning.
pub fn parse(src: &str) -> Result<Element, XmlError> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut reader = Reader::from_str(src);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let mut pending_text = String::new();

    fn flush_text(stack: &mut [Element], text: &mut String) {
        if text.is_empty() {
            return;
        }
        if let Some(top) = stack.last_mut() {
            match top.children.last_mut() {
                Some(Node::Text(prev)) => prev.push_str(text),
                _ => top.children.push(Node::Text(std::mem::take(text))),
            }
        }
        text.clear();
    }

    loop {
        let ev = reader.read_event().map_err(|e| XmlError::Parse {
            pos: reader.buffer_position(),
            msg: e.to_string(),
        })?;
        match ev {
            Event::Start(e) => {
                flush_text(&mut stack, &mut pending_text);
                if stack.len() >= MAX_DEPTH {
                    return Err(XmlError::TooDeep);
                }
                stack.push(start_to_element(&e));
            }
            Event::Empty(e) => {
                flush_text(&mut stack, &mut pending_text);
                let el = start_to_element(&e);
                match stack.last_mut() {
                    Some(top) => top.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::End(e) => {
                flush_text(&mut stack, &mut pending_text);
                let mut el = stack
                    .pop()
                    .ok_or_else(|| XmlError::Unbalanced(lossless(e.name().as_ref()).into()))?;
                drop_layout_whitespace(&mut el);
                match stack.last_mut() {
                    Some(top) => top.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                if !stack.is_empty() {
                    pending_text.push_str(&unescape_text(&lossless(&t)));
                }
            }
            Event::GeneralRef(r) => {
                if !stack.is_empty() {
                    let name = lossless(&r);
                    match resolve_entity(&name) {
                        Some(s) => pending_text.push_str(&s),
                        None => {
                            pending_text.push('&');
                            pending_text.push_str(&name);
                            pending_text.push(';');
                        }
                    }
                }
            }
            Event::CData(t) => {
                flush_text(&mut stack, &mut pending_text);
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::CData(lossless(&t).into_owned()));
                }
            }
            Event::Comment(t) => {
                flush_text(&mut stack, &mut pending_text);
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::Comment(lossless(&t).into_owned()));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or(XmlError::NoRoot)
}

/// In elements that contain child elements, whitespace-only text is layout.
fn drop_layout_whitespace(el: &mut Element) {
    if el.children.iter().any(|n| matches!(n, Node::Element(_))) {
        el.children.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_basic() {
        let src = r#"<?xml version="1.0"?><a x="1 &amp; 2"><b>t &lt;3</b><c /><!--note--><d><![CDATA[raw <x>]]></d></a>"#;
        let el = parse(src).unwrap();
        assert_eq!(el.attr("x"), Some("1 & 2"));
        assert_eq!(el.get("b"), "t <3");
        assert_eq!(el.get("d"), "raw <x>");
        let again = parse(&el.to_xml_string()).unwrap();
        assert_eq!(el, again);
    }

    #[test]
    fn whitespace_text_kept_in_leaf() {
        let el = parse("<a>\n  <b>  spaced  </b>\n</a>").unwrap();
        assert_eq!(el.children.len(), 1);
        assert_eq!(el.get("b"), "  spaced  ");
    }

    #[test]
    fn char_refs() {
        let el = parse("<a>&#233;&#x41;</a>").unwrap();
        assert_eq!(el.text(), "éA");
    }
}
