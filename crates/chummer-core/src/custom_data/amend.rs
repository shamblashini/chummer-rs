//! Port of `XmlManager.AmendNodeChildren` and the document paths it builds.
//!
//! The C# builds XPath strings such as
//! `/chummer/qualities/quality[name = 'X']/bonus` one step per recursion
//! level and re-runs them with `SelectNodes`. Here a path is a list of
//! [`DocStep`]s (element name plus optional predicate) that is re-evaluated
//! against the current document whenever the C# would select.

use super::regex::Regex;
use super::xpath::{self, Expr};
use crate::xml::{Element, Node};

/// One `name[predicate]` step of an absolute document path.
#[derive(Debug, Clone)]
pub(crate) struct DocStep {
    pub name: String,
    pub filter: Option<Expr>,
}

/// An absolute path, starting at the root element (`/chummer`).
pub(crate) type DocPath = Vec<DocStep>;

/// The path `/<root>`.
pub(crate) fn root_path(root: &str) -> DocPath {
    vec![DocStep { name: root.to_owned(), filter: None }]
}

fn extend(path: &DocPath, name: &str, filter: Option<&Expr>) -> DocPath {
    let mut p = path.clone();
    p.push(DocStep { name: name.to_owned(), filter: filter.cloned() });
    p
}

/// Index path from the root element down to a node: positions in each
/// element's `children`.
pub(crate) type NodePos = Vec<usize>;

/// `XmlDocument.SelectNodes(path)`: every element on `path`, in document order.
pub(crate) fn select(root: &Element, path: &[DocStep]) -> Result<Vec<NodePos>, xpath::XPathError> {
    let Some((first, rest)) = path.split_first() else { return Ok(Vec::new()) };
    if root.name != first.name || !step_filter_holds(first, root, root, 1, 1)? {
        return Ok(Vec::new());
    }
    let mut current: Vec<NodePos> = vec![Vec::new()];
    for step in rest {
        let mut next = Vec::new();
        for pos in &current {
            next.extend(select_children(root, pos, step)?);
        }
        current = next;
    }
    Ok(current)
}

/// Children of the node at `pos` that satisfy one step.
fn select_children(root: &Element, pos: &NodePos, step: &DocStep) -> Result<Vec<NodePos>, xpath::XPathError> {
    let parent = at(root, pos);
    let candidates: Vec<(usize, &Element)> = parent
        .children
        .iter()
        .enumerate()
        .filter_map(|(i, n)| match n {
            Node::Element(e) if e.name == step.name => Some((i, e)),
            _ => None,
        })
        .collect();
    let size = candidates.len();
    let mut out = Vec::new();
    for (k, (i, e)) in candidates.into_iter().enumerate() {
        if step_filter_holds(step, root, e, k + 1, size)? {
            let mut p = pos.clone();
            p.push(i);
            out.push(p);
        }
    }
    Ok(out)
}

fn step_filter_holds(step: &DocStep, root: &Element, e: &Element, pos: usize, size: usize) -> Result<bool, xpath::XPathError> {
    match &step.filter {
        None => Ok(true),
        Some(f) => xpath::matches_predicate(f, root, e, pos, size),
    }
}

/// `XmlNode.SelectSingleNode(name + filter)` relative to `parent`.
fn has_matching_child(root: &Element, parent: &NodePos, name: &str, filter: Option<&Expr>) -> Result<bool, xpath::XPathError> {
    let step = DocStep { name: name.to_owned(), filter: filter.cloned() };
    Ok(!select_children(root, parent, &step)?.is_empty())
}

pub(crate) fn at<'a>(root: &'a Element, pos: &[usize]) -> &'a Element {
    let mut cur = root;
    for &i in pos {
        cur = match &cur.children[i] {
            Node::Element(e) => e,
            _ => unreachable!("node positions always point at elements"),
        };
    }
    cur
}

pub(crate) fn at_mut<'a>(root: &'a mut Element, pos: &[usize]) -> &'a mut Element {
    let mut cur = root;
    for &i in pos {
        cur = match &mut cur.children[i] {
            Node::Element(e) => e,
            _ => unreachable!("node positions always point at elements"),
        };
    }
    cur
}

/// Attributes that only steer the amend system.
const AMEND_ATTRIBUTES: &[&str] = &["isidnode", "xpathfilter", "amendoperation", "addifnotfound", "regexpattern"];

fn take_attr(e: &mut Element, key: &str) -> Option<String> {
    let i = e.attrs.iter().position(|(k, _)| k == key)?;
    Some(e.attrs.remove(i).1)
}

/// Port of `XmlManager.StripAmendAttributesRecursively`.
pub(crate) fn strip_amend_attributes(e: &mut Element) {
    e.attrs.retain(|(k, _)| !AMEND_ATTRIBUTES.contains(&k.as_str()));
    for c in e.elements_mut() {
        strip_amend_attributes(c);
    }
}

fn stripped(e: &Element) -> Element {
    let mut c = e.clone();
    strip_amend_attributes(&mut c);
    c
}

/// Identifier text for a filter, with the C#'s `Replace("&amp;", "&")`
/// (entities are already decoded once by the parser).
pub(crate) fn id_text(e: &Element) -> String {
    xpath::string_value(e).replace("&amp;", "&")
}

/// `bool.TrueString` comparison as done by `InnerTextIsTrueString`.
fn is_true(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("true")
}

/// Ancestors recorded by a `recurse` that found no target, to be recreated
/// (shallowly) if a descendant ends up appending
/// (`lstExtraNodesToAddIfNotFound`).
pub(crate) struct ExtraNode {
    id: usize,
    node: Element,
    parent_path: DocPath,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    None,
    AddNode,
    Remove,
    Replace,
    Append,
    RegexReplace,
    Recurse,
    Unknown,
}

fn parse_op(s: &str) -> Op {
    match s.to_ascii_uppercase().as_str() {
        "" => Op::None,
        "ADDNODE" => Op::AddNode,
        "REMOVE" => Op::Remove,
        "REPLACE" => Op::Replace,
        "APPEND" => Op::Append,
        "REGEXREPLACE" => Op::RegexReplace,
        "RECURSE" => Op::Recurse,
        _ => Op::Unknown,
    }
}

/// What the amending node's attributes asked for, after
/// `AmendNodeChildren` has read and removed them.
struct Directives {
    op: Op,
    filter: Option<Expr>,
    add_if_not_found_present: bool,
    add_if_not_found: bool,
    regex_pattern: String,
}

/// State shared by one amend pass: warnings and a counter for extra-node ids.
pub(crate) struct Amender<'w> {
    pub warnings: &'w mut Vec<String>,
    pub context: String,
    /// Number of edits made to the document (inserted, removed, replaced or
    /// rewritten nodes), for reporting and tests.
    pub mutations: usize,
    next_extra_id: usize,
}

impl<'w> Amender<'w> {
    pub fn new(warnings: &'w mut Vec<String>, context: String) -> Self {
        Self { warnings, context, mutations: 0, next_extra_id: 0 }
    }

    fn warn(&mut self, msg: impl std::fmt::Display) {
        self.warnings.push(format!("{}: {msg}", self.context));
    }

    /// `SelectNodes`, logging (and treating as empty) evaluation errors.
    fn select(&mut self, doc: &Element, path: &[DocStep]) -> Vec<NodePos> {
        select(doc, path).unwrap_or_else(|e| {
            self.warn(e);
            Vec::new()
        })
    }

    /// The identifier filter `AmendNodeChildren` builds when there is no
    /// `xpathfilter`: `id`, else `name` (only for removes or when other
    /// children exist), plus every `isidnode="True"` child.
    fn default_filter(amending: &Element, op: Op) -> Option<Expr> {
        let mut filter = if let Some(id) = amending.child("id") {
            Some(Expr::child_equals("id", &id_text(id)))
        } else {
            let name = amending.child("name");
            let other_children = amending.elements().any(|e| e.name != "name");
            name.filter(|_| op == Op::Remove || other_children).map(|n| Expr::child_equals("name", &id_text(n)))
        };
        // `child::*[@isidnode = 'True']`: XPath equality is case-sensitive.
        for extra in amending.elements().filter(|e| e.attr("isidnode") == Some("True")) {
            let cond = Expr::child_equals(&extra.name, &id_text(extra));
            filter = Some(match filter {
                Some(f) => f.and(cond),
                None => cond,
            });
        }
        filter
    }

    /// First block of `AmendNodeChildren`: read and remove the directive
    /// attributes from the amending node.
    fn read_directives(&mut self, amending: &mut Element) -> Option<Directives> {
        // The id filter must see `isidnode` children before they are stripped,
        // but the node's own `isidnode` attribute goes first.
        take_attr(amending, "isidnode");
        let op_text = take_attr(amending, "amendoperation").unwrap_or_default();
        let op = parse_op(&op_text);
        if op == Op::Unknown {
            self.warn(format!("unknown amendoperation {op_text:?} on <{}>, using the default", amending.name));
        }
        let filter = match take_attr(amending, "xpathfilter") {
            Some(f) if f.trim().is_empty() => None,
            Some(f) => match xpath::parse(&f) {
                Ok(e) => Some(e),
                Err(e) => {
                    self.warn(format!("skipping <{}>: cannot parse xpathfilter {f:?}: {e}", amending.name));
                    return None;
                }
            },
            None => Self::default_filter(amending, op),
        };
        let add = take_attr(amending, "addifnotfound");
        let regex_pattern = take_attr(amending, "regexpattern").unwrap_or_default();
        Some(Directives {
            op,
            filter,
            add_if_not_found_present: add.is_some(),
            add_if_not_found: add.as_deref().is_some_and(is_true),
            regex_pattern,
        })
    }

    /// Port of `XmlManager.AmendNodeChildren`: apply one amending node to
    /// every node at `xpath/<amending.name>[filter]`. Returns whether the
    /// document changed (the C# return value, quirks included).
    pub fn amend_node_children(&mut self, doc: &mut Element, amending: &mut Element, xpath: &DocPath, extra: &mut Vec<ExtraNode>) -> bool {
        let Some(mut d) = self.read_directives(amending) else { return false };
        if d.op == Op::AddNode {
            return self.add_node(doc, amending, xpath);
        }
        let new_path = extend(xpath, &amending.name, d.filter.as_ref());
        let targets = self.select(doc, &new_path);
        let has_element_children = amending.elements().next().is_some();
        if !self.resolve_op(&mut d, has_element_children, targets.len()) {
            return false;
        }
        if !targets.is_empty() || (d.op == Op::Recurse && !d.add_if_not_found) {
            if d.op == Op::Recurse {
                return self.recurse(doc, amending, xpath, &new_path, !targets.is_empty(), extra);
            }
            self.edit_targets(doc, amending, xpath, &targets, &d);
            if d.add_if_not_found {
                self.add_to_parents_without_target(doc, amending, xpath, targets.len(), &d);
            }
            return true;
        }
        let wants_add = d.op == Op::Append || (d.add_if_not_found && matches!(d.op, Op::Recurse | Op::Replace));
        if wants_add {
            return self.append_when_not_found(doc, amending, xpath, &d, extra);
        }
        false
    }

    /// The operation `switch` of `AmendNodeChildren`: validate the explicit
    /// operation or pick the default. Returns `false` to abort (bad regex).
    fn resolve_op(&mut self, d: &mut Directives, has_element_children: bool, target_count: usize) -> bool {
        match d.op {
            Op::Remove | Op::Replace | Op::Append => return true,
            Op::RegexReplace if d.regex_pattern.trim().is_empty() => {
                d.op = Op::Replace;
                return true;
            }
            Op::RegexReplace => {
                if let Err(e) = Regex::new(&d.regex_pattern) {
                    self.warn(e);
                    return false;
                }
                return true;
            }
            Op::Recurse if has_element_children => return true,
            _ => {}
        }
        if has_element_children {
            d.op = Op::Recurse;
        } else if target_count == 0 {
            d.op = Op::Append;
        } else {
            d.op = Op::Replace;
            if !d.add_if_not_found_present {
                d.add_if_not_found = true;
            }
        }
        true
    }

    /// `addnode`: append the amending node, as is, under every parent.
    fn add_node(&mut self, doc: &mut Element, amending: &Element, xpath: &DocPath) -> bool {
        let parents = self.select(doc, xpath);
        for p in &parents {
            at_mut(doc, p).push(amending.clone());
            self.mutations += 1;
        }
        !parents.is_empty()
    }

    /// `recurse`: apply each element child against the targets' path. With
    /// no targets (or pending ancestors) this node is recorded so a
    /// descendant that appends can recreate it. Returns the last child's
    /// result, as the C# does.
    fn recurse(&mut self, doc: &mut Element, amending: &mut Element, xpath: &DocPath, new_path: &DocPath, found: bool, extra: &mut Vec<ExtraNode>) -> bool {
        let mut result = false;
        if extra.is_empty() && found {
            for child in amending.elements_mut() {
                result = self.amend_node_children(doc, child, new_path, extra);
            }
        } else {
            let id = self.next_extra_id;
            self.next_extra_id += 1;
            let shallow = Element { name: amending.name.clone(), attrs: amending.attrs.clone(), children: Vec::new() };
            extra.push(ExtraNode { id, node: shallow, parent_path: xpath.clone() });
            for child in amending.elements_mut() {
                result = self.amend_node_children(doc, child, new_path, extra);
            }
            if let Some(i) = extra.iter().position(|x| x.id == id) {
                extra.remove(i);
            }
        }
        result
    }

    /// The per-target loop for `remove`, `append`, `replace`, `regexreplace`.
    fn edit_targets(&mut self, doc: &mut Element, amending: &Element, xpath: &DocPath, targets: &[NodePos], d: &Directives) -> bool {
        match d.op {
            Op::Remove => {
                // Reverse document order keeps the remaining positions valid.
                for t in targets.iter().rev() {
                    let (last, parent) = t.split_last().expect("targets are below the root");
                    at_mut(doc, parent).children.remove(*last);
                    self.mutations += 1;
                }
            }
            Op::Append => {
                for t in targets {
                    self.append_to_target(doc, amending, xpath, t);
                }
            }
            Op::Replace => {
                let node = stripped(amending);
                for t in targets {
                    let (last, parent) = t.split_last().expect("targets are below the root");
                    at_mut(doc, parent).children[*last] = Node::Element(node.clone());
                    self.mutations += 1;
                }
            }
            Op::RegexReplace => {
                let Ok(re) = Regex::new(&d.regex_pattern) else { return false };
                for t in targets {
                    if regex_replace_target(at_mut(doc, t), amending, &re) {
                        self.mutations += 1;
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// `append` on one target: merge text into the existing text node,
    /// append element children; an empty amending node is appended next
    /// to the target instead.
    fn append_to_target(&mut self, doc: &mut Element, amending: &Element, xpath: &DocPath, target: &NodePos) {
        if !amending.children.is_empty() {
            self.mutations += 1;
            let t = at_mut(doc, target);
            for child in &amending.children {
                match child {
                    Node::Comment(_) => {}
                    Node::Text(s) | Node::CData(s) => append_text(t, child, s),
                    Node::Element(e) => t.push(stripped(e)),
                }
            }
        } else if !at(doc, target).children.is_empty() {
            let node = stripped(amending);
            for gp in self.select(doc, xpath) {
                at_mut(doc, &gp).push(node.clone());
                self.mutations += 1;
            }
        }
    }

    /// The `blnAddIfNotFound` tail of the target loop: add the amending node
    /// to parents that still have no matching child.
    fn add_to_parents_without_target(&mut self, doc: &mut Element, amending: &Element, xpath: &DocPath, target_count: usize, d: &Directives) -> bool {
        let parents = self.select(doc, xpath);
        if parents.len() <= target_count {
            return false;
        }
        let node = stripped(amending);
        let mut changed = false;
        for p in parents {
            let found = has_matching_child(doc, &p, &amending.name, d.filter.as_ref()).unwrap_or_else(|e| {
                self.warn(e);
                true
            });
            if !found {
                at_mut(doc, &p).push(node.clone());
                self.mutations += 1;
                changed = true;
            }
        }
        changed
    }

    /// No target found and the operation adds: first recreate recorded
    /// ancestors (when this node has no filter), then append the node.
    fn append_when_not_found(&mut self, doc: &mut Element, amending: &Element, xpath: &DocPath, d: &Directives, extra: &mut Vec<ExtraNode>) -> bool {
        if !extra.is_empty() && d.filter.is_none() {
            for x in extra.iter() {
                for p in self.select(doc, &x.parent_path) {
                    at_mut(doc, &p).push(x.node.clone());
                    self.mutations += 1;
                }
            }
            extra.clear();
        }
        let parents = self.select(doc, xpath);
        let node = stripped(amending);
        for p in &parents {
            at_mut(doc, p).push(node.clone());
            self.mutations += 1;
        }
        !parents.is_empty()
    }
}

/// `append` of a text/CDATA child: concatenate onto the target's first
/// child of the same kind, or add it as a new child.
fn append_text(target: &mut Element, child: &Node, s: &str) {
    let same_kind = |n: &Node| matches!((n, child), (Node::Text(_), Node::Text(_)) | (Node::CData(_), Node::CData(_)));
    if let Some(Node::Text(t) | Node::CData(t)) = target.children.iter_mut().find(|n| same_kind(n)) {
        t.push_str(s);
        return;
    }
    target.children.push(child.clone());
}

/// `regexreplace` on one target: run the pattern over the target's first
/// text (or CDATA) child, using the amending node's text as replacement.
/// Returns whether any text changed.
fn regex_replace_target(target: &mut Element, amending: &Element, re: &Regex) -> bool {
    let before = target.children.clone();
    let replacements: Vec<&Node> = amending.children.iter().filter(|n| matches!(n, Node::Text(_) | Node::CData(_))).collect();
    if amending.children.is_empty() {
        if let Some(Node::Text(t)) = target.children.iter_mut().find(|n| matches!(n, Node::Text(_))) {
            *t = re.replace_all(t, "");
        }
        return target.children != before;
    }
    for rep in replacements {
        let (Node::Text(r) | Node::CData(r)) = rep else { continue };
        let same_kind = |n: &Node| matches!((n, rep), (Node::Text(_), Node::Text(_)) | (Node::CData(_), Node::CData(_)));
        if let Some(Node::Text(t) | Node::CData(t)) = target.children.iter_mut().find(|n| same_kind(n)) {
            *t = re.replace_all(t, r);
        }
    }
    target.children != before
}
