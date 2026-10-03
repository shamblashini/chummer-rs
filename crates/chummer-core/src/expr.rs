//! Arithmetic in data files: costs, availabilities, ratings, bonus values.
//!
//! Chummer5a evaluates these strings as XPath 1.0 over an empty document
//! (`CommonFunctions.EvaluateInvariantXPath`). This module re-implements the
//! numeric subset of XPath that the data actually uses, including the text
//! rewrites and character whitelist done before evaluation, so strings that
//! fail in Chummer also fail here.

use std::fmt;

/// Result of an XPath evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
}

impl Value {
    /// XPath `number()` conversion.
    pub fn num(self) -> f64 {
        match self {
            Value::Num(n) => n,
            Value::Bool(b) => f64::from(u8::from(b)),
        }
    }
    /// XPath `boolean()` conversion.
    pub fn truthy(self) -> bool {
        match self {
            Value::Num(n) => n != 0.0 && !n.is_nan(),
            Value::Bool(b) => b,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot evaluate expression: {}", self.0)
    }
}

impl std::error::Error for ExprError {}

/// Chummer's `StandardRound`: ceiling away from zero (2.1 -> 3, -2.1 -> -3).
pub fn standard_round(d: f64) -> i32 {
    let r = if d >= 0.0 { d.ceil() } else { d.floor() };
    r.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// `decimal.Round(x, places, MidpointRounding.AwayFromZero)`, used for essence.
pub fn round_away(x: f64, places: u32) -> f64 {
    let m = 10f64.powi(places as i32);
    // Nudge by an ulp-scale epsilon so 0.15 (stored as 0.1499..) rounds like decimal does.
    let scaled = x * m;
    let eps = scaled.abs() * 1e-12;
    let r = if scaled >= 0.0 { (scaled + eps + 0.5).floor() } else { (scaled - eps - 0.5).ceil() };
    r / m
}

const WHITELIST: &str = "1234567890+-*abcdefghilmnorstuvw()[]{}!=<>&;,. ";

/// The text rewrites Chummer applies before handing a string to XPath.
fn rewrite_math(s: &str) -> String {
    let s = s
        .replace(['/', '\\', '÷'], " div ")
        .replace(" x ", " * ")
        .replace(['∙', '×'], "*")
        .replace('[', "(")
        .replace(']', ")");
    s.trim_start_matches('+').to_owned()
}

/// True when the string is not a plain number and needs XPath evaluation
/// (`DoesNeedXPathProcessingToBeConvertedToNumber`).
pub fn needs_evaluation(s: &str) -> bool {
    let t = s.trim();
    const SPECIAL: &str = "abcdfghijklmnopqrstuvwxyzABCDFGHIJKLMNOPQRSTUVWXYZ()[]{}!=<>&;+*/\\÷×∙";
    if t.chars().any(|c| SPECIAL.contains(c)) {
        return true;
    }
    t.char_indices().any(|(i, c)| c == '-' && i != 0)
}

/// Parse a plain number leniently, like `decimal.TryParse(NumberStyles.Any)`.
pub fn parse_plain(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| *c != ',' && !c.is_whitespace()).collect();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

/// Evaluate an expression string. Mirrors `EvaluateInvariantXPath` with
/// `blnIsMathExpression = true`.
pub fn evaluate(src: &str) -> Result<Value, ExprError> {
    if src.trim().is_empty() {
        return Err(ExprError("empty expression".into()));
    }
    let s = rewrite_math(src);
    if let Some(bad) = s.chars().find(|c| !WHITELIST.contains(*c)) {
        return Err(ExprError(format!("{src:?}: character {bad:?} not allowed")));
    }
    if s == "-" {
        return Ok(Value::Num(0.0));
    }
    let tokens = lex(&s).map_err(|e| ExprError(format!("{src:?}: {e}")))?;
    let mut p = Parser { toks: &tokens, pos: 0 };
    let v = p.or_expr().map_err(|e| ExprError(format!("{src:?}: {e}")))?;
    if p.pos != tokens.len() {
        return Err(ExprError(format!("{src:?}: unexpected trailing input")));
    }
    Ok(v)
}

/// Evaluate to a number, the way callers cast the result to `double`.
pub fn evaluate_num(src: &str) -> Result<f64, ExprError> {
    if !needs_evaluation(src) {
        return parse_plain(src).ok_or_else(|| ExprError(format!("{src:?}: not a number")));
    }
    let v = evaluate(src)?;
    match v {
        Value::Num(n) if n.is_finite() => Ok(n),
        Value::Num(_) => Err(ExprError(format!("{src:?}: result is not finite"))),
        // In C# a bare boolean result throws on the `(double)` cast.
        Value::Bool(_) => Err(ExprError(format!("{src:?}: boolean result"))),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Star,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            ' ' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                    i += 1;
                }
                let lit: String = b[start..i].iter().collect();
                out.push(Tok::Num(lit.parse().map_err(|_| format!("bad number {lit:?}"))?));
            }
            'a'..='z' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_lowercase() || b[i] == '-' && i + 1 < b.len() && b[i + 1].is_ascii_lowercase()) {
                    i += 1;
                }
                out.push(Tok::Name(b[start..i].iter().collect()));
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            '!' if b.get(i + 1) == Some(&'=') => {
                out.push(Tok::Ne);
                i += 2;
            }
            '<' | '>' => {
                let eq = b.get(i + 1) == Some(&'=');
                out.push(match (c, eq) {
                    ('<', false) => Tok::Lt,
                    ('<', true) => Tok::Le,
                    ('>', false) => Tok::Gt,
                    _ => Tok::Ge,
                });
                i += if eq { 2 } else { 1 };
            }
            _ => return Err(format!("unexpected {c:?}")),
        }
    }
    Ok(out)
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn bump(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        self.pos += 1;
        t
    }
    fn peek_name(&self, n: &str) -> bool {
        matches!(self.peek(), Some(Tok::Name(x)) if x == n)
    }

    fn or_expr(&mut self) -> Result<Value, String> {
        let mut v = self.and_expr()?;
        while self.peek_name("or") {
            self.pos += 1;
            let r = self.and_expr()?;
            v = Value::Bool(v.truthy() || r.truthy());
        }
        Ok(v)
    }

    fn and_expr(&mut self) -> Result<Value, String> {
        let mut v = self.eq_expr()?;
        while self.peek_name("and") {
            self.pos += 1;
            let r = self.eq_expr()?;
            v = Value::Bool(v.truthy() && r.truthy());
        }
        Ok(v)
    }

    fn eq_expr(&mut self) -> Result<Value, String> {
        let mut v = self.rel_expr()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => true,
                Some(Tok::Ne) => false,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.rel_expr()?;
            // XPath: if either side is boolean, compare as booleans.
            let equal = match (v, r) {
                (Value::Bool(_), _) | (_, Value::Bool(_)) => v.truthy() == r.truthy(),
                _ => v.num() == r.num(),
            };
            v = Value::Bool(equal == op);
        }
    }

    fn rel_expr(&mut self) -> Result<Value, String> {
        let mut v = self.add_expr()?;
        loop {
            let op = match self.peek() {
                Some(t @ (Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge)) => t.clone(),
                _ => return Ok(v),
            };
            self.pos += 1;
            let (a, b) = (v.num(), self.add_expr()?.num());
            v = Value::Bool(match op {
                Tok::Lt => a < b,
                Tok::Le => a <= b,
                Tok::Gt => a > b,
                _ => a >= b,
            });
        }
    }

    fn add_expr(&mut self) -> Result<Value, String> {
        let mut v = self.mul_expr()?;
        loop {
            let plus = match self.peek() {
                Some(Tok::Plus) => true,
                Some(Tok::Minus) => false,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.mul_expr()?.num();
            v = Value::Num(if plus { v.num() + r } else { v.num() - r });
        }
    }

    fn mul_expr(&mut self) -> Result<Value, String> {
        let mut v = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => 0,
                Some(Tok::Name(n)) if n == "div" => 1,
                Some(Tok::Name(n)) if n == "mod" => 2,
                _ => return Ok(v),
            };
            self.pos += 1;
            let (a, b) = (v.num(), self.unary()?.num());
            v = Value::Num(match op {
                0 => a * b,
                1 => a / b,
                _ => a % b, // truncating remainder, sign of dividend
            });
        }
    }

    fn unary(&mut self) -> Result<Value, String> {
        if matches!(self.peek(), Some(Tok::Minus)) {
            self.pos += 1;
            return Ok(Value::Num(-self.unary()?.num()));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Value, String> {
        match self.bump().cloned() {
            Some(Tok::Num(n)) => Ok(Value::Num(n)),
            Some(Tok::LParen) => {
                let v = self.or_expr()?;
                match self.bump() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err("missing ')'".into()),
                }
            }
            Some(Tok::Name(name)) => {
                if !matches!(self.bump(), Some(Tok::LParen)) {
                    // A bare name is a location path; over an empty document
                    // it is an empty node-set, whose number() is NaN.
                    self.pos -= 1;
                    return Ok(Value::Num(f64::NAN));
                }
                let mut args = Vec::new();
                if !matches!(self.peek(), Some(Tok::RParen)) {
                    loop {
                        args.push(self.or_expr()?);
                        match self.peek() {
                            Some(Tok::Comma) => self.pos += 1,
                            _ => break,
                        }
                    }
                }
                if !matches!(self.bump(), Some(Tok::RParen)) {
                    return Err(format!("missing ')' after {name}("));
                }
                call(&name, &args)
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }
}

fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let one = || -> Result<f64, String> {
        match args {
            [a] => Ok(a.num()),
            _ => Err(format!("{name}() takes one argument")),
        }
    };
    Ok(match name {
        "number" => Value::Num(one()?),
        "floor" => Value::Num(one()?.floor()),
        "ceiling" => Value::Num(one()?.ceil()),
        // XPath round: half toward +infinity
        "round" => Value::Num((one()? + 0.5).floor()),
        "not" => match args {
            [a] => Value::Bool(!a.truthy()),
            _ => return Err("not() takes one argument".into()),
        },
        "boolean" => match args {
            [a] => Value::Bool(a.truthy()),
            _ => return Err("boolean() takes one argument".into()),
        },
        "true" if args.is_empty() => Value::Bool(true),
        "false" if args.is_empty() => Value::Bool(false),
        _ => return Err(format!("unsupported function {name}()")),
    })
}

/// Split `FixedValues(a,b,c)` on top-level commas and pick by rating
/// (1-based, clamped). Strings without a leading `FixedValues(` are returned
/// unchanged. Elements are not trimmed, matching Chummer.
pub fn fixed_values(s: &str, rating: i32) -> String {
    let Some(inner) = s.strip_prefix("FixedValues(").and_then(|r| r.strip_suffix(')')) else {
        return s.to_owned();
    };
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    let idx = (rating.max(1) as usize).min(parts.len()) - 1;
    parts[idx].to_owned()
}

/// Something that can substitute `{STR}`-style attribute tokens.
pub trait AttributeSource {
    /// Value for a token without braces, e.g. `"STR"`, `"AGIUnaug"`, `"MAGMaximum"`.
    fn attribute_token(&self, token: &str) -> Option<i32>;
}

/// No character context: every `{...}` token is left in place (and fails).
pub struct NoAttributes;

impl AttributeSource for NoAttributes {
    fn attribute_token(&self, _: &str) -> Option<i32> {
        None
    }
}

pub const ATTRIBUTE_NAMES: &[&str] =
    &["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "MAGAdept", "RES", "ESS", "DEP"];

/// `AttributeSection.ProcessAttributesInXPath`: replace `{X}`, `{XUnaug}`,
/// `{XBase}`, `{XMinimum}` and `{XMaximum}`.
pub fn substitute_attributes(s: &str, src: &dyn AttributeSource) -> String {
    if !s.contains('{') {
        return s.to_owned();
    }
    let mut out = s.to_owned();
    // Longer names first so {MAGAdept} is not eaten by {MAG...}: braces make
    // each token exact, so plain replace is enough.
    for name in ATTRIBUTE_NAMES {
        for suffix in ["", "Unaug", "Base", "Minimum", "Maximum"] {
            let token = format!("{name}{suffix}");
            let braced = format!("{{{token}}}");
            if out.contains(&braced) {
                if let Some(v) = src.attribute_token(&token) {
                    out = out.replace(&braced, &v.to_string());
                }
            }
        }
    }
    out
}

/// `ImprovementManager.ValueToDec`: FixedValues, then `Rating`, then
/// attributes, then evaluate. Failures yield 0, as in Chummer.
pub fn value_to_dec(s: &str, rating: i32, attrs: &dyn AttributeSource) -> f64 {
    let r = rating.to_string();
    let s = fixed_values(s, rating).replace("{Rating}", &r).replace("Rating", &r);
    if !needs_evaluation(&s) {
        return parse_plain(&s).unwrap_or(0.0);
    }
    evaluate_num(&substitute_attributes(&s, attrs)).unwrap_or(0.0)
}

/// `ImprovementManager.ValueToInt`: as [`value_to_dec`], rounded with
/// [`standard_round`].
pub fn value_to_int(s: &str, rating: i32, attrs: &dyn AttributeSource) -> i32 {
    standard_round(value_to_dec(s, rating, attrs))
}

/// Legality suffix of an availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Legality {
    #[default]
    Legal,
    Restricted,
    Forbidden,
}

impl Legality {
    pub fn suffix(self) -> &'static str {
        match self {
            Legality::Legal => "",
            Legality::Restricted => "R",
            Legality::Forbidden => "F",
        }
    }
}

/// A parsed availability such as `12F` or `+2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Availability {
    pub value: i32,
    pub legality: Legality,
    /// Leading `+`/`-`: the value adds to the parent item's availability.
    pub add_to_parent: bool,
}

impl fmt::Display for Availability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.add_to_parent && self.value >= 0 {
            f.write_str("+")?;
        }
        write!(f, "{}{}", self.value, self.legality.suffix())
    }
}

impl Availability {
    /// Parse an availability string for an item at `rating`. Follows
    /// `Gear.TotalAvailTuple`: FixedValues first, then strip the R/F suffix,
    /// then evaluate and round. Negative totals clamp to 0 unless the value
    /// is a modifier to the parent.
    pub fn parse(s: &str, rating: i32, min_rating: i32, attrs: &dyn AttributeSource) -> Availability {
        let mut s = fixed_values(s.trim(), rating).trim().to_owned();
        if let Some(rest) = s.strip_suffix(" or Gear") {
            s = rest.to_owned();
        }
        let legality = match s.chars().last() {
            Some('F') => Legality::Forbidden,
            Some('R') => Legality::Restricted,
            _ => Legality::Legal,
        };
        if legality != Legality::Legal {
            s.pop();
        }
        let add_to_parent = s.starts_with('+') || s.starts_with('-');
        let r = rating.to_string();
        let s = s
            .replace("{MinRating}", &min_rating.to_string())
            .replace("MinRating", &min_rating.to_string())
            .replace("{Rating}", &r)
            .replace("Rating", &r);
        let n = if s.trim().is_empty() {
            0.0
        } else if needs_evaluation(&s) {
            evaluate_num(&substitute_attributes(&s, attrs)).unwrap_or(0.0)
        } else {
            parse_plain(&s).unwrap_or(0.0)
        };
        let mut value = standard_round(n);
        if !add_to_parent {
            value = value.max(0);
        }
        Availability { value, legality, add_to_parent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Attrs;
    impl AttributeSource for Attrs {
        fn attribute_token(&self, t: &str) -> Option<i32> {
            Some(match t {
                "STR" => 5,
                "STRUnaug" => 4,
                "INTUnaug" => 3,
                "LOGUnaug" => 4,
                "MAG" => 7,
                _ => return None,
            })
        }
    }

    fn num(s: &str) -> f64 {
        evaluate_num(s).unwrap()
    }

    #[test]
    fn standard_round_is_ceiling_away_from_zero() {
        assert_eq!(standard_round(2.1), 3);
        assert_eq!(standard_round(2.0), 2);
        assert_eq!(standard_round(-2.1), -3);
        assert_eq!(standard_round(-0.5), -1);
        assert_eq!(standard_round(0.0), 0);
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(num("2 + 3 * 4"), 14.0);
        assert_eq!(num("(2 + 3) * 4"), 20.0);
        assert_eq!(num("10 div 4"), 2.5);
        assert_eq!(num("10/4"), 2.5);
        assert_eq!(num("[3]"), 3.0);
        assert_eq!(num("-7 mod 3"), -1.0);
        assert_eq!(num("7 mod 3"), 1.0);
        assert_eq!(num("+5"), 5.0);
        assert_eq!(num("3 x 4"), 12.0);
        assert_eq!(num("-(8 div 4)"), -2.0);
        assert_eq!(num("2 - -1"), 3.0);
    }

    #[test]
    fn xpath_functions() {
        assert_eq!(num("floor(7 div 3)"), 2.0);
        assert_eq!(num("ceiling(7 div 3)"), 3.0);
        assert_eq!(num("round(2.5)"), 3.0);
        assert_eq!(num("round(-2.5)"), -2.0);
        assert_eq!(num("number(3 > 2) * 250 + 3 * 500"), 1750.0);
        assert_eq!(num("number(0 = 0)"), 1.0);
        assert_eq!(num("-number(1 >= 2)"), 0.0);
    }

    #[test]
    fn rejects_like_chummer() {
        // Uppercase letters left over from an unknown token fail the whitelist.
        assert!(evaluate("Rating * 2").is_err());
        assert!(evaluate("{STR} + 1").is_err() || evaluate_num("{STR} + 1").is_err());
        assert!(evaluate_num("3 > 2").is_err(), "bare boolean must not cast to number");
        assert!(evaluate_num("1 div 0").is_err());
        assert_eq!(evaluate("-"), Ok(Value::Num(0.0)));
    }

    #[test]
    fn plain_numbers_take_fast_path() {
        assert!(!needs_evaluation("-12"));
        assert!(!needs_evaluation("1.5"));
        assert!(needs_evaluation("2-1"));
        assert_eq!(num("1,000"), 1000.0);
    }

    #[test]
    fn fixed_values_selection() {
        assert_eq!(fixed_values("FixedValues(1,2,3)", 2), "2");
        assert_eq!(fixed_values("FixedValues(1,2,3)", 0), "1");
        assert_eq!(fixed_values("FixedValues(1,2,3)", 9), "3");
        assert_eq!(fixed_values("FixedValues(1, 2)", 2), " 2");
        assert_eq!(fixed_values("FixedValues((Rating*2),8)", 1), "(Rating*2)");
        assert_eq!(fixed_values("Rating * 2", 3), "Rating * 2");
    }

    #[test]
    fn value_to_int_with_rating_and_attributes() {
        assert_eq!(value_to_int("Rating * 3", 4, &Attrs), 12);
        assert_eq!(value_to_int("-Rating", 2, &Attrs), -2);
        assert_eq!(value_to_int("{STR}*2", 0, &Attrs), 10);
        assert_eq!(value_to_int("({INTUnaug} + {LOGUnaug}) * 2", 0, &Attrs), 14);
        assert_eq!(value_to_int("floor({MAG} div 3)", 0, &Attrs), 2);
        assert_eq!(value_to_int("Rating div 4", 3, &Attrs), 1); // 0.75 ceils to 1
        assert_eq!(value_to_int("FixedValues(2,4,6)", 3, &Attrs), 6);
        assert_eq!(value_to_int("{UNKNOWN}", 0, &Attrs), 0);
    }

    #[test]
    fn availability_parsing() {
        let a = |s: &str, r: i32| Availability::parse(s, r, 1, &NoAttributes);
        assert_eq!(a("12F", 0).to_string(), "12F");
        assert_eq!(a("4R", 0).to_string(), "4R");
        assert_eq!(a("+2", 0).to_string(), "+2");
        assert_eq!(a("(Rating * 3)R", 4).to_string(), "12R");
        assert_eq!(a("Rating*4R", 2).to_string(), "8R");
        assert_eq!(a("RatingR", 5).to_string(), "5R");
        assert_eq!(a("(Rating*2)+4F", 3).to_string(), "10F");
        assert_eq!(a("FixedValues(8R,12R,20R)", 2).to_string(), "12R");
        assert_eq!(a("+Rating - MinRating + 1", 4).to_string(), "+4");
        assert_eq!(a("12R or Gear", 0).to_string(), "12R");
        assert_eq!(a("0", 0).to_string(), "0");
        let neg = a("-4", 0);
        assert!(neg.add_to_parent);
        assert_eq!(neg.value, -4);
        assert!(Legality::Forbidden > Legality::Restricted);
    }

    #[test]
    fn essence_rounding() {
        assert_eq!(round_away(0.15, 1), 0.2);
        assert_eq!(round_away(5.555, 2), 5.56);
        assert_eq!(round_away(-0.25, 1), -0.3);
    }
}
