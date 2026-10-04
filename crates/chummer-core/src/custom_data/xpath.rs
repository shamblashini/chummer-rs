//! An XPath 1.0 subset over [`Element`] trees.
//!
//! Chummer's amend files carry `xpathfilter` attributes, which
//! `XmlManager.AmendNodeChildren` splices into an XPath query and hands to
//! .NET's `XmlDocument.SelectNodes`. This module parses and evaluates those
//! filter expressions: `or`/`and`, comparisons with XPath's existential
//! node-set semantics, arithmetic, relative and absolute location paths
//! (child, attribute, self, parent and descendant axes, `*`, `text()`,
//! `node()`), nested predicates and the core string/number functions.
//!
//! Anything else is a [`XPathError`], never a panic.

use std::fmt;

use crate::xml::{Element, Node};

/// Why an expression could not be parsed or evaluated.
#[derive(Debug, Clone, PartialEq)]
pub struct XPathError(pub String);

impl fmt::Display for XPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XPathError {}

fn err<T>(msg: impl Into<String>) -> Result<T, XPathError> {
    Err(XPathError(msg.into()))
}

// ---------------------------------------------------------------- AST

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// A parsed XPath expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    Arith(ArithOp, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    Literal(String),
    Number(f64),
    Call(String, Vec<Expr>),
    Path(LocationPath),
}

impl Expr {
    /// `name = 'value'`: the identifier filters `AmendNodeChildren` builds
    /// from `<id>`, `<name>` and `isidnode` children.
    pub fn child_equals(child: &str, value: &str) -> Expr {
        let path = LocationPath { start: PathStart::Context, steps: vec![Step::child(child)] };
        Expr::Cmp(CmpOp::Eq, Box::new(Expr::Path(path)), Box::new(Expr::Literal(value.to_owned())))
    }

    /// `self and other`.
    pub fn and(self, other: Expr) -> Expr {
        Expr::And(Box::new(self), Box::new(other))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathStart {
    /// Relative to the context node.
    Context,
    /// `/...`: from the document root.
    Root,
    /// `(expr)[pred]/...` or `f(x)/...`.
    Filter(Box<Expr>, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationPath {
    pub start: PathStart,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Axis {
    Child,
    Attribute,
    SelfAxis,
    Parent,
    Descendant,
    DescendantOrSelf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeTest {
    Name(String),
    /// `*`
    AnyName,
    /// `text()`
    Text,
    /// `node()`
    AnyNode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub axis: Axis,
    pub test: NodeTest,
    pub predicates: Vec<Expr>,
}

impl Step {
    fn child(name: &str) -> Step {
        Step { axis: Axis::Child, test: NodeTest::Name(name.to_owned()), predicates: Vec::new() }
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    Num(f64),
    /// A name used as a node test, function name or axis name.
    Name(String),
    /// `and`, `or`, `div`, `mod` in operator position.
    OpWord(&'static str),
    /// `*` in operator position (multiplication).
    Mul,
    /// `*` as a name test.
    Star,
    Sym(&'static str),
}

const SYMBOLS: &[&str] = &["!=", "<=", ">=", "//", "::", "..", "=", "<", ">", "+", "-", "/", "|", "(", ")", "[", "]", "@", ",", "."];

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// XPath 1.0 §3.7: `*` and operator names are operators unless the
/// previous token is `@`, `::`, `(`, `[`, `,` or an operator.
fn operator_position(prev: Option<&Tok>) -> bool {
    match prev {
        None => false,
        Some(Tok::Sym(s)) => matches!(*s, ")" | "]" | "." | ".."),
        Some(Tok::OpWord(_)) | Some(Tok::Mul) => false,
        Some(_) => true,
    }
}

fn tokenize(src: &str) -> Result<Vec<Tok>, XPathError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let end = chars[i + 1..].iter().position(|&x| x == c).ok_or_else(|| XPathError("unterminated string literal".into()))?;
            out.push(Tok::Lit(chars[i + 1..i + 1 + end].iter().collect()));
            i += end + 2;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let n = text.parse().map_err(|_| XPathError(format!("bad number {text}")))?;
            out.push(Tok::Num(n));
            continue;
        }
        if c == '*' {
            out.push(if operator_position(out.last()) { Tok::Mul } else { Tok::Star });
            i += 1;
            continue;
        }
        if is_name_start(c) {
            let start = i;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            // A trailing '.' is never part of a name in practice; keep it as a symbol.
            while i > start + 1 && chars[i - 1] == '.' {
                i -= 1;
            }
            let name: String = chars[start..i].iter().collect();
            let op = ["and", "or", "div", "mod"].into_iter().find(|w| *w == name);
            match op {
                Some(w) if operator_position(out.last()) => out.push(Tok::OpWord(w)),
                _ => out.push(Tok::Name(name)),
            }
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let sym = SYMBOLS.iter().find(|s| rest.starts_with(**s)).ok_or_else(|| XPathError(format!("unexpected character {c:?}")))?;
        out.push(Tok::Sym(sym));
        i += sym.chars().count();
    }
    Ok(out)
}

// ---------------------------------------------------------------- parser

/// Functions the evaluator implements, with their (min, max) arity.
const FUNCTIONS: &[(&str, usize, usize)] = &[
    ("last", 0, 0),
    ("position", 0, 0),
    ("count", 1, 1),
    ("name", 0, 1),
    ("local-name", 0, 1),
    ("string", 0, 1),
    ("concat", 2, usize::MAX),
    ("starts-with", 2, 2),
    ("ends-with", 2, 2),
    ("contains", 2, 2),
    ("substring-before", 2, 2),
    ("substring-after", 2, 2),
    ("substring", 2, 3),
    ("string-length", 0, 1),
    ("normalize-space", 0, 1),
    ("translate", 3, 3),
    ("boolean", 1, 1),
    ("not", 1, 1),
    ("true", 0, 0),
    ("false", 0, 0),
    ("number", 0, 1),
    ("sum", 1, 1),
    ("floor", 1, 1),
    ("ceiling", 1, 1),
    ("round", 1, 1),
];

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

/// Parse an XPath expression, e.g. an `xpathfilter` attribute value.
pub fn parse(src: &str) -> Result<Expr, XPathError> {
    let mut p = Parser { toks: tokenize(src)?, pos: 0 };
    let e = p.or_expr()?;
    match p.peek() {
        None => Ok(e),
        Some(t) => err(format!("unexpected token {t:?} in {src:?}")),
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn peek_at(&self, off: usize) -> Option<&Tok> {
        self.toks.get(self.pos + off)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if matches!(self.peek(), Some(Tok::OpWord(x)) if *x == w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> Result<(), XPathError> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            err(format!("expected {s:?}, found {:?}", self.peek()))
        }
    }

    fn or_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.and_expr()?;
        while self.eat_word("or") {
            e = Expr::Or(Box::new(e), Box::new(self.and_expr()?));
        }
        Ok(e)
    }

    fn and_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.eq_expr()?;
        while self.eat_word("and") {
            e = Expr::And(Box::new(e), Box::new(self.eq_expr()?));
        }
        Ok(e)
    }

    fn eq_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.rel_expr()?;
        loop {
            let op = if self.eat_sym("=") {
                CmpOp::Eq
            } else if self.eat_sym("!=") {
                CmpOp::Ne
            } else {
                return Ok(e);
            };
            e = Expr::Cmp(op, Box::new(e), Box::new(self.rel_expr()?));
        }
    }

    fn rel_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.add_expr()?;
        loop {
            let op = if self.eat_sym("<=") {
                CmpOp::Le
            } else if self.eat_sym(">=") {
                CmpOp::Ge
            } else if self.eat_sym("<") {
                CmpOp::Lt
            } else if self.eat_sym(">") {
                CmpOp::Gt
            } else {
                return Ok(e);
            };
            e = Expr::Cmp(op, Box::new(e), Box::new(self.add_expr()?));
        }
    }

    fn add_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.mul_expr()?;
        loop {
            let op = if self.eat_sym("+") {
                ArithOp::Add
            } else if self.eat_sym("-") {
                ArithOp::Sub
            } else {
                return Ok(e);
            };
            e = Expr::Arith(op, Box::new(e), Box::new(self.mul_expr()?));
        }
    }

    fn mul_expr(&mut self) -> Result<Expr, XPathError> {
        let mut e = self.unary_expr()?;
        loop {
            let op = if matches!(self.peek(), Some(Tok::Mul)) {
                self.pos += 1;
                ArithOp::Mul
            } else if self.eat_word("div") {
                ArithOp::Div
            } else if self.eat_word("mod") {
                ArithOp::Mod
            } else {
                return Ok(e);
            };
            e = Expr::Arith(op, Box::new(e), Box::new(self.unary_expr()?));
        }
    }

    fn unary_expr(&mut self) -> Result<Expr, XPathError> {
        if self.eat_sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary_expr()?)));
        }
        let mut e = self.path_expr()?;
        while self.eat_sym("|") {
            e = Expr::Union(Box::new(e), Box::new(self.path_expr()?));
        }
        Ok(e)
    }

    /// Does the next token start a primary expression (literal, number,
    /// parenthesis or function call) rather than a location path?
    fn at_primary(&self) -> bool {
        match self.peek() {
            Some(Tok::Lit(_)) | Some(Tok::Num(_)) => true,
            Some(Tok::Sym("(")) => true,
            Some(Tok::Name(n)) => {
                matches!(self.peek_at(1), Some(Tok::Sym("("))) && !matches!(n.as_str(), "text" | "node" | "comment" | "processing-instruction")
            }
            _ => false,
        }
    }

    fn path_expr(&mut self) -> Result<Expr, XPathError> {
        if !self.at_primary() {
            return self.location_path().map(Expr::Path);
        }
        let primary = self.primary()?;
        let preds = self.predicates()?;
        let has_slash = matches!(self.peek(), Some(Tok::Sym("/")) | Some(Tok::Sym("//")));
        if preds.is_empty() && !has_slash {
            return Ok(primary);
        }
        let mut path = LocationPath { start: PathStart::Filter(Box::new(primary), preds), steps: Vec::new() };
        if has_slash {
            self.relative_steps(&mut path.steps, true)?;
        }
        Ok(Expr::Path(path))
    }

    fn primary(&mut self) -> Result<Expr, XPathError> {
        match self.next() {
            Some(Tok::Lit(s)) => Ok(Expr::Literal(s)),
            Some(Tok::Num(n)) => Ok(Expr::Number(n)),
            Some(Tok::Sym("(")) => {
                let e = self.or_expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Some(Tok::Name(name)) => self.call(name),
            t => err(format!("unexpected token {t:?}")),
        }
    }

    fn call(&mut self, name: String) -> Result<Expr, XPathError> {
        self.expect_sym("(")?;
        let mut args = Vec::new();
        if !self.eat_sym(")") {
            loop {
                args.push(self.or_expr()?);
                if self.eat_sym(")") {
                    break;
                }
                self.expect_sym(",")?;
            }
        }
        let Some(&(_, min, max)) = FUNCTIONS.iter().find(|(n, _, _)| *n == name) else {
            return err(format!("unsupported function {name}()"));
        };
        if args.len() < min || args.len() > max {
            return err(format!("{name}() takes {min}..{max} arguments, got {}", args.len()));
        }
        Ok(Expr::Call(name, args))
    }

    fn predicates(&mut self) -> Result<Vec<Expr>, XPathError> {
        let mut preds = Vec::new();
        while self.eat_sym("[") {
            preds.push(self.or_expr()?);
            self.expect_sym("]")?;
        }
        Ok(preds)
    }

    fn location_path(&mut self) -> Result<LocationPath, XPathError> {
        let mut path = LocationPath { start: PathStart::Context, steps: Vec::new() };
        if matches!(self.peek(), Some(Tok::Sym("/")) | Some(Tok::Sym("//"))) {
            path.start = PathStart::Root;
            let lone_root = matches!(self.peek(), Some(Tok::Sym("/"))) && !self.step_follows(1);
            if lone_root {
                self.pos += 1;
                return Ok(path);
            }
            self.relative_steps(&mut path.steps, true)?;
            return Ok(path);
        }
        self.relative_steps(&mut path.steps, false)?;
        Ok(path)
    }

    /// Could the token at `off` start a step?
    fn step_follows(&self, off: usize) -> bool {
        matches!(
            self.peek_at(off),
            Some(Tok::Name(_)) | Some(Tok::Star) | Some(Tok::Sym("@")) | Some(Tok::Sym(".")) | Some(Tok::Sym(".."))
        )
    }

    /// `step ('/' step | '//' step)*`, optionally starting with a separator.
    fn relative_steps(&mut self, steps: &mut Vec<Step>, leading_sep: bool) -> Result<(), XPathError> {
        let mut need_sep = leading_sep;
        loop {
            if need_sep {
                if self.eat_sym("//") {
                    steps.push(Step { axis: Axis::DescendantOrSelf, test: NodeTest::AnyNode, predicates: Vec::new() });
                } else if !self.eat_sym("/") {
                    return Ok(());
                }
            }
            steps.push(self.step()?);
            need_sep = true;
        }
    }

    fn step(&mut self) -> Result<Step, XPathError> {
        if self.eat_sym(".") {
            return Ok(Step { axis: Axis::SelfAxis, test: NodeTest::AnyNode, predicates: Vec::new() });
        }
        if self.eat_sym("..") {
            return Ok(Step { axis: Axis::Parent, test: NodeTest::AnyNode, predicates: Vec::new() });
        }
        let axis = if self.eat_sym("@") { Axis::Attribute } else { self.axis_specifier()? };
        let test = self.node_test()?;
        let predicates = self.predicates()?;
        Ok(Step { axis, test, predicates })
    }

    fn axis_specifier(&mut self) -> Result<Axis, XPathError> {
        let Some(Tok::Name(n)) = self.peek().cloned() else { return Ok(Axis::Child) };
        if !matches!(self.peek_at(1), Some(Tok::Sym("::"))) {
            return Ok(Axis::Child);
        }
        let axis = match n.as_str() {
            "child" => Axis::Child,
            "attribute" => Axis::Attribute,
            "self" => Axis::SelfAxis,
            "parent" => Axis::Parent,
            "descendant" => Axis::Descendant,
            "descendant-or-self" => Axis::DescendantOrSelf,
            _ => return err(format!("unsupported axis {n}::")),
        };
        self.pos += 2;
        Ok(axis)
    }

    fn node_test(&mut self) -> Result<NodeTest, XPathError> {
        match self.next() {
            Some(Tok::Star) | Some(Tok::Mul) => Ok(NodeTest::AnyName),
            Some(Tok::Name(n)) => {
                if self.eat_sym("(") {
                    self.expect_sym(")")?;
                    return match n.as_str() {
                        "text" => Ok(NodeTest::Text),
                        "node" => Ok(NodeTest::AnyNode),
                        _ => err(format!("unsupported node test {n}()")),
                    };
                }
                Ok(NodeTest::Name(n))
            }
            t => err(format!("expected a node test, found {t:?}")),
        }
    }
}

// ---------------------------------------------------------------- values

/// A node in an XPath node-set.
#[derive(Debug, Clone, Copy)]
pub enum NodeRef<'a> {
    Elem(&'a Element),
    Attr(&'a str, &'a str),
    Text(&'a str),
}

impl<'a> NodeRef<'a> {
    fn same(&self, other: &NodeRef<'a>) -> bool {
        match (self, other) {
            (NodeRef::Elem(a), NodeRef::Elem(b)) => std::ptr::eq(*a, *b),
            (NodeRef::Attr(_, a), NodeRef::Attr(_, b)) | (NodeRef::Text(a), NodeRef::Text(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }

    fn string_value(&self) -> String {
        match self {
            NodeRef::Elem(e) => string_value(e),
            NodeRef::Attr(_, v) | NodeRef::Text(v) => (*v).to_owned(),
        }
    }

    fn name(&self) -> &'a str {
        match self {
            NodeRef::Elem(e) => &e.name,
            NodeRef::Attr(n, _) => n,
            NodeRef::Text(_) => "",
        }
    }
}

/// XPath string-value of an element: all descendant text, in order
/// (.NET's `InnerText`).
pub fn string_value(e: &Element) -> String {
    if let [Node::Text(t)] = e.children.as_slice() {
        return t.clone();
    }
    let mut out = String::new();
    collect_text(e, &mut out);
    out
}

fn collect_text(e: &Element, out: &mut String) {
    for n in &e.children {
        match n {
            Node::Text(t) | Node::CData(t) => out.push_str(t),
            Node::Element(c) => collect_text(c, out),
            Node::Comment(_) => {}
        }
    }
}

#[derive(Debug, Clone)]
enum Value<'a> {
    Nodes(Vec<NodeRef<'a>>),
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Value<'_> {
    fn to_bool(&self) -> bool {
        match self {
            Value::Nodes(n) => !n.is_empty(),
            Value::Str(s) => !s.is_empty(),
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Bool(b) => *b,
        }
    }
    fn to_str(&self) -> String {
        match self {
            Value::Nodes(n) => n.first().map(NodeRef::string_value).unwrap_or_default(),
            Value::Str(s) => s.clone(),
            Value::Num(n) => number_to_string(*n),
            Value::Bool(b) => b.to_string(),
        }
    }
    fn to_num(&self) -> f64 {
        match self {
            Value::Nodes(_) | Value::Str(_) => str_to_number(&self.to_str()),
            Value::Num(n) => *n,
            Value::Bool(b) => f64::from(u8::from(*b)),
        }
    }
}

/// XPath `number()` of a string: optional `-`, digits with an optional
/// fraction, surrounded by whitespace; anything else is NaN.
fn str_to_number(s: &str) -> f64 {
    let t = s.trim();
    let body = t.strip_prefix('-').unwrap_or(t);
    let valid = !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && body.chars().filter(|&c| c == '.').count() <= 1
        && body.chars().any(|c| c.is_ascii_digit());
    if valid {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

// ---------------------------------------------------------------- evaluator

/// Evaluation context: the document root (for `/` and `..`), the context
/// node, and its position within the current node list.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub root: &'a Element,
    pub node: NodeRef<'a>,
    pub pos: usize,
    pub size: usize,
}

/// Evaluate `expr` as a step predicate on `el`: a number selects by
/// position, anything else by its boolean value.
pub fn matches_predicate(expr: &Expr, root: &Element, el: &Element, pos: usize, size: usize) -> Result<bool, XPathError> {
    let ctx = Ctx { root, node: NodeRef::Elem(el), pos, size };
    predicate_holds(expr, &ctx)
}

fn predicate_holds(expr: &Expr, ctx: &Ctx<'_>) -> Result<bool, XPathError> {
    Ok(match eval(expr, ctx)? {
        Value::Num(n) => n == ctx.pos as f64,
        v => v.to_bool(),
    })
}

fn eval<'a>(expr: &Expr, ctx: &Ctx<'a>) -> Result<Value<'a>, XPathError> {
    Ok(match expr {
        Expr::Or(a, b) => Value::Bool(eval(a, ctx)?.to_bool() || eval(b, ctx)?.to_bool()),
        Expr::And(a, b) => Value::Bool(eval(a, ctx)?.to_bool() && eval(b, ctx)?.to_bool()),
        Expr::Cmp(op, a, b) => Value::Bool(compare(*op, &eval(a, ctx)?, &eval(b, ctx)?)),
        Expr::Arith(op, a, b) => Value::Num(arith(*op, eval(a, ctx)?.to_num(), eval(b, ctx)?.to_num())),
        Expr::Neg(a) => Value::Num(-eval(a, ctx)?.to_num()),
        Expr::Union(a, b) => {
            let (Value::Nodes(mut x), Value::Nodes(y)) = (eval(a, ctx)?, eval(b, ctx)?) else {
                return err("union of non-node-sets");
            };
            for n in y {
                push_unique(&mut x, n);
            }
            Value::Nodes(x)
        }
        Expr::Literal(s) => Value::Str(s.clone()),
        Expr::Number(n) => Value::Num(*n),
        Expr::Call(name, args) => call(name, args, ctx)?,
        Expr::Path(p) => Value::Nodes(eval_path(p, ctx)?),
    })
}

fn arith(op: ArithOp, a: f64, b: f64) -> f64 {
    match op {
        ArithOp::Add => a + b,
        ArithOp::Sub => a - b,
        ArithOp::Mul => a * b,
        ArithOp::Div => a / b,
        ArithOp::Mod => a % b,
    }
}

fn cmp_num(op: CmpOp, a: f64, b: f64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        CmpOp::Lt => a < b,
        CmpOp::Le => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::Ge => a >= b,
    }
}

/// XPath 1.0 §3.4 comparisons, including the existential rule for node-sets.
fn compare(op: CmpOp, a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Nodes(x), Value::Nodes(y)) => {
            let ys: Vec<String> = y.iter().map(NodeRef::string_value).collect();
            x.iter().any(|n| {
                let s = n.string_value();
                ys.iter().any(|t| cmp_atoms(op, &Value::Str(s.clone()), &Value::Str(t.clone())))
            })
        }
        (Value::Nodes(x), Value::Bool(_)) | (Value::Bool(_), Value::Nodes(x)) => {
            let nb = Value::Bool(!x.is_empty());
            if matches!(a, Value::Nodes(_)) { cmp_atoms(op, &nb, b) } else { cmp_atoms(op, a, &nb) }
        }
        (Value::Nodes(x), other) => x.iter().any(|n| cmp_atoms(op, &Value::Str(n.string_value()), other)),
        (other, Value::Nodes(y)) => y.iter().any(|n| cmp_atoms(op, other, &Value::Str(n.string_value()))),
        _ => cmp_atoms(op, a, b),
    }
}

/// Comparison of two non-node-set values.
fn cmp_atoms(op: CmpOp, a: &Value<'_>, b: &Value<'_>) -> bool {
    if matches!(op, CmpOp::Eq | CmpOp::Ne) {
        let eq = if matches!(a, Value::Bool(_)) || matches!(b, Value::Bool(_)) {
            a.to_bool() == b.to_bool()
        } else if matches!(a, Value::Num(_)) || matches!(b, Value::Num(_)) {
            a.to_num() == b.to_num()
        } else {
            a.to_str() == b.to_str()
        };
        return if op == CmpOp::Eq { eq } else { !eq };
    }
    cmp_num(op, a.to_num(), b.to_num())
}

fn push_unique<'a>(set: &mut Vec<NodeRef<'a>>, n: NodeRef<'a>) {
    if !set.iter().any(|m| m.same(&n)) {
        set.push(n);
    }
}

fn eval_path<'a>(p: &LocationPath, ctx: &Ctx<'a>) -> Result<Vec<NodeRef<'a>>, XPathError> {
    let mut current: Vec<NodeRef<'a>> = match &p.start {
        PathStart::Context => vec![ctx.node],
        // The document node's only element child is the root element.
        PathStart::Root => return eval_from_document(p, ctx),
        PathStart::Filter(e, preds) => {
            let Value::Nodes(nodes) = eval(e, ctx)? else {
                return err("path step applied to a non-node-set");
            };
            filter_by_predicates(nodes, preds, ctx)?
        }
    };
    for step in &p.steps {
        current = apply_step(&current, step, ctx)?;
    }
    Ok(current)
}

/// Absolute paths: the first step is evaluated against the (implicit)
/// document node, whose only child is the root element.
fn eval_from_document<'a>(p: &LocationPath, ctx: &Ctx<'a>) -> Result<Vec<NodeRef<'a>>, XPathError> {
    let Some((first, rest)) = p.steps.split_first() else {
        return Ok(vec![NodeRef::Elem(ctx.root)]);
    };
    let mut current = match first.axis {
        Axis::Child => {
            let cand = if test_matches(&first.test, &NodeRef::Elem(ctx.root)) { vec![NodeRef::Elem(ctx.root)] } else { Vec::new() };
            filter_by_predicates(cand, &first.predicates, ctx)?
        }
        Axis::DescendantOrSelf | Axis::Descendant => {
            let mut all = vec![NodeRef::Elem(ctx.root)];
            descendants(ctx.root, &mut all);
            let cand = all.into_iter().filter(|n| test_matches(&first.test, n)).collect();
            filter_by_predicates(cand, &first.predicates, ctx)?
        }
        _ => return err("unsupported axis at the document root"),
    };
    for step in rest {
        current = apply_step(&current, step, ctx)?;
    }
    Ok(current)
}

fn apply_step<'a>(input: &[NodeRef<'a>], step: &Step, ctx: &Ctx<'a>) -> Result<Vec<NodeRef<'a>>, XPathError> {
    let mut out = Vec::new();
    for n in input {
        let cand: Vec<NodeRef<'a>> = match (n, step.axis, &step.test) {
            // Fast path for the common `name` step.
            (NodeRef::Elem(e), Axis::Child, NodeTest::Name(name)) => e.elements().filter(|c| c.name == *name).map(NodeRef::Elem).collect(),
            _ => axis_nodes(n, step.axis, ctx.root).into_iter().filter(|c| test_matches_on(&step.test, c, step.axis)).collect(),
        };
        let kept = filter_by_predicates(cand, &step.predicates, ctx)?;
        if input.len() == 1 {
            out = kept;
        } else {
            for m in kept {
                push_unique(&mut out, m);
            }
        }
    }
    Ok(out)
}

fn filter_by_predicates<'a>(mut nodes: Vec<NodeRef<'a>>, preds: &[Expr], ctx: &Ctx<'a>) -> Result<Vec<NodeRef<'a>>, XPathError> {
    for pred in preds {
        let size = nodes.len();
        let mut kept = Vec::with_capacity(size);
        for (i, n) in nodes.into_iter().enumerate() {
            let inner = Ctx { root: ctx.root, node: n, pos: i + 1, size };
            if predicate_holds(pred, &inner)? {
                kept.push(n);
            }
        }
        nodes = kept;
    }
    Ok(nodes)
}

fn axis_nodes<'a>(n: &NodeRef<'a>, axis: Axis, root: &'a Element) -> Vec<NodeRef<'a>> {
    match axis {
        Axis::SelfAxis => vec![*n],
        Axis::Parent => parent_of(n, root).into_iter().collect(),
        Axis::Child => match n {
            NodeRef::Elem(e) => children(e),
            _ => Vec::new(),
        },
        Axis::Attribute => match n {
            NodeRef::Elem(e) => e.attrs.iter().map(|(k, v)| NodeRef::Attr(k.as_str(), v.as_str())).collect(),
            _ => Vec::new(),
        },
        Axis::Descendant | Axis::DescendantOrSelf => {
            let mut out = if axis == Axis::DescendantOrSelf { vec![*n] } else { Vec::new() };
            if let NodeRef::Elem(e) = n {
                descendants(e, &mut out);
            }
            out
        }
    }
}

fn children(e: &Element) -> Vec<NodeRef<'_>> {
    e.children
        .iter()
        .filter_map(|c| match c {
            Node::Element(x) => Some(NodeRef::Elem(x)),
            Node::Text(t) | Node::CData(t) => Some(NodeRef::Text(t.as_str())),
            Node::Comment(_) => None,
        })
        .collect()
}

fn descendants<'a>(e: &'a Element, out: &mut Vec<NodeRef<'a>>) {
    for c in children(e) {
        out.push(c);
        if let NodeRef::Elem(x) = c {
            descendants(x, out);
        }
    }
}

/// Parent of a node, found by walking down from the root (the tree has no
/// parent links).
fn parent_of<'a>(n: &NodeRef<'a>, root: &'a Element) -> Option<NodeRef<'a>> {
    fn walk<'a>(e: &'a Element, n: &NodeRef<'a>) -> Option<&'a Element> {
        let direct = children(e).iter().any(|c| c.same(n))
            || matches!(n, NodeRef::Attr(_, v) if e.attrs.iter().any(|(_, x)| std::ptr::eq(x.as_str(), *v)));
        if direct {
            return Some(e);
        }
        e.elements().find_map(|c| walk(c, n))
    }
    walk(root, n).map(NodeRef::Elem)
}

fn test_matches(test: &NodeTest, n: &NodeRef<'_>) -> bool {
    test_matches_on(test, n, Axis::Child)
}

/// Node test against the axis' principal node type (attributes on the
/// attribute axis, elements elsewhere).
fn test_matches_on(test: &NodeTest, n: &NodeRef<'_>, axis: Axis) -> bool {
    let principal = match axis {
        Axis::Attribute => matches!(n, NodeRef::Attr(..)),
        _ => matches!(n, NodeRef::Elem(_)),
    };
    match test {
        NodeTest::AnyNode => true,
        NodeTest::Text => matches!(n, NodeRef::Text(_)),
        NodeTest::AnyName => principal,
        NodeTest::Name(name) => principal && n.name() == name,
    }
}

// ---------------------------------------------------------------- functions

fn arg_str(args: &[Expr], i: usize, ctx: &Ctx<'_>) -> Result<String, XPathError> {
    match args.get(i) {
        Some(e) => Ok(eval(e, ctx)?.to_str()),
        None => Ok(ctx.node.string_value()),
    }
}

fn arg_num(args: &[Expr], i: usize, ctx: &Ctx<'_>) -> Result<f64, XPathError> {
    match args.get(i) {
        Some(e) => Ok(eval(e, ctx)?.to_num()),
        None => Ok(str_to_number(&ctx.node.string_value())),
    }
}

fn arg_nodes<'a>(args: &[Expr], i: usize, ctx: &Ctx<'a>) -> Result<Vec<NodeRef<'a>>, XPathError> {
    match args.get(i) {
        Some(e) => match eval(e, ctx)? {
            Value::Nodes(n) => Ok(n),
            _ => err("expected a node-set argument"),
        },
        None => Ok(vec![ctx.node]),
    }
}

fn xpath_round(n: f64) -> f64 {
    (n + 0.5).floor()
}

/// XPath `substring(s, start, len?)` with its rounding rules (1-based).
fn substring(s: &str, start: f64, len: Option<f64>) -> String {
    let first = xpath_round(start);
    let last = len.map(|l| first + xpath_round(l));
    s.chars()
        .enumerate()
        .filter(|(i, _)| {
            let p = (*i + 1) as f64;
            p >= first && last.is_none_or(|l| p < l)
        })
        .map(|(_, c)| c)
        .collect()
}

fn call<'a>(name: &str, args: &[Expr], ctx: &Ctx<'a>) -> Result<Value<'a>, XPathError> {
    let s = |i| arg_str(args, i, ctx);
    Ok(match name {
        "last" => Value::Num(ctx.size as f64),
        "position" => Value::Num(ctx.pos as f64),
        "count" => Value::Num(arg_nodes(args, 0, ctx)?.len() as f64),
        "name" | "local-name" => Value::Str(arg_nodes(args, 0, ctx)?.first().map(|n| n.name().to_owned()).unwrap_or_default()),
        "string" => Value::Str(s(0)?),
        "concat" => Value::Str(args.iter().map(|a| eval(a, ctx).map(|v| v.to_str())).collect::<Result<String, _>>()?),
        "starts-with" => Value::Bool(s(0)?.starts_with(&s(1)?)),
        "ends-with" => Value::Bool(s(0)?.ends_with(&s(1)?)),
        "contains" => Value::Bool(s(0)?.contains(&s(1)?)),
        "substring-before" => {
            let (h, n) = (s(0)?, s(1)?);
            Value::Str(h.find(&n).map(|i| h[..i].to_owned()).unwrap_or_default())
        }
        "substring-after" => {
            let (h, n) = (s(0)?, s(1)?);
            Value::Str(h.find(&n).map(|i| h[i + n.len()..].to_owned()).unwrap_or_default())
        }
        "substring" => {
            let len = if args.len() > 2 { Some(arg_num(args, 2, ctx)?) } else { None };
            Value::Str(substring(&s(0)?, arg_num(args, 1, ctx)?, len))
        }
        "string-length" => Value::Num(s(0)?.chars().count() as f64),
        "normalize-space" => Value::Str(s(0)?.split_whitespace().collect::<Vec<_>>().join(" ")),
        "translate" => Value::Str(translate(&s(0)?, &s(1)?, &s(2)?)),
        "boolean" => Value::Bool(eval(&args[0], ctx)?.to_bool()),
        "not" => Value::Bool(!eval(&args[0], ctx)?.to_bool()),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "number" => Value::Num(arg_num(args, 0, ctx)?),
        "sum" => Value::Num(arg_nodes(args, 0, ctx)?.iter().map(|n| str_to_number(&n.string_value())).sum()),
        "floor" => Value::Num(arg_num(args, 0, ctx)?.floor()),
        "ceiling" => Value::Num(arg_num(args, 0, ctx)?.ceil()),
        "round" => Value::Num(xpath_round(arg_num(args, 0, ctx)?)),
        _ => return err(format!("unsupported function {name}()")),
    })
}

fn translate(s: &str, from: &str, to: &str) -> String {
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();
    s.chars()
        .filter_map(|c| match from.iter().position(|&f| f == c) {
            Some(i) => to.get(i).copied(),
            None => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml;

    fn check(doc: &str, filter: &str) -> bool {
        let root = xml::parse(doc).unwrap();
        let e = parse(filter).unwrap_or_else(|e| panic!("{filter}: {e}"));
        matches_predicate(&e, &root, &root, 1, 1).unwrap()
    }

    #[test]
    fn existential_comparisons() {
        let d = "<q><skilldisable>A</skilldisable><skilldisable>B</skilldisable></q>";
        assert!(check(d, "skilldisable = 'A' and skilldisable = 'B'"));
        assert!(check(d, "not(skilldisable = 'C')"));
        assert!(check(d, "skilldisable != 'A'"));
        assert!(!check(d, "missing"));
        assert!(check(d, "count(skilldisable) = 2"));
    }

    #[test]
    fn functions_and_arithmetic() {
        let d = "<w><category>Light Pistols</category><damage>8P</damage><conceal>-1</conceal></w>";
        let ends_s = "not(damage[(substring(., string-length(.) - string-length('S') + 1) = 'S')])";
        assert!(check(d, ends_s));
        assert!(check("<w><damage>6S</damage></w>", "damage[(substring(., string-length(.) - string-length('S') + 1) = 'S')]"));
        assert!(check(d, "category='Light Pistols' and conceal <= 0"));
        assert!(check(d, "contains(category,'Pistol') and starts-with(category, 'Light')"));
        assert!(check(d, "count(*) > 2"));
    }

    #[test]
    fn attributes_dots_and_nested_paths() {
        let d = r#"<b><selectskill limittoskill="Artisan,Performance"><val>1</val></selectskill><skills><skill rating="2">Perception</skill></skills></b>"#;
        assert!(check(d, "selectskill[@limittoskill = 'Artisan,Performance']/val = '1'"));
        assert!(check(d, "skills/skill[. = 'Perception' and @rating]"));
        assert!(!check(d, "skills/skill[. = 'Perception' and not(@rating)]"));
        assert!(check(d, "count(bonus/*/name[. = 'X']) = 0"));
        assert!(check(d, "name = \"The Beast's Way\" or skills"));
    }

    #[test]
    fn unknown_constructs_are_errors() {
        assert!(parse("frobnicate(name)").is_err());
        assert!(parse("following-sibling::x").is_err());
        assert!(parse("name = 'unterminated").is_err());
        assert!(parse("name = ").is_err());
    }
}
