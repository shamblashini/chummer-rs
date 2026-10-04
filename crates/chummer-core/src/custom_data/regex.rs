//! A small backtracking regular-expression engine for `regexreplace`
//! amends (`XmlManager.AmendNodeChildren` calls .NET's `Regex.Replace`).
//!
//! Supported: literals, `.`, classes `[a-z]`/`[^...]`, `\d \w \s` and their
//! negations, escapes, groups `( )` and `(?: )`, alternation `|`, greedy and
//! lazy `* + ? {n} {n,} {n,m}`, anchors `^ $`. Replacement strings use .NET
//! syntax: `$1`, `${1}`, `$0`, `$&`, `$$`. Other constructs are rejected
//! with an error rather than guessed at.

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<ClassItem>, bool),
    Start,
    End,
    Group(Box<Node>, Option<usize>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: usize, max: usize, greedy: bool },
}

#[derive(Debug, Clone)]
enum ClassItem {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl ClassItem {
    fn matches(&self, c: char) -> bool {
        match *self {
            ClassItem::Range(a, b) => a <= c && c <= b,
            ClassItem::Digit(neg) => c.is_ascii_digit() != neg,
            ClassItem::Word(neg) => (c.is_alphanumeric() || c == '_') != neg,
            ClassItem::Space(neg) => c.is_whitespace() != neg,
        }
    }
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Regex {
    root: Node,
    groups: usize,
}

type Caps = Vec<Option<(usize, usize)>>;

struct Parser<'s> {
    chars: Vec<char>,
    pos: usize,
    groups: usize,
    src: &'s str,
}

impl Regex {
    /// Compile a pattern; unsupported syntax is an error.
    pub fn new(pattern: &str) -> Result<Regex, String> {
        let mut p = Parser { chars: pattern.chars().collect(), pos: 0, groups: 0, src: pattern };
        let root = p.alternation()?;
        if p.pos < p.chars.len() {
            return Err(format!("unbalanced ')' in regex {pattern:?}"));
        }
        Ok(Regex { root, groups: p.groups })
    }

    /// `Regex.Replace(input, pattern, replacement)`: every non-overlapping
    /// match, left to right.
    pub fn replace_all(&self, input: &str, replacement: &str) -> String {
        let s: Vec<char> = input.chars().collect();
        let mut out = String::new();
        let mut pos = 0;
        while pos <= s.len() {
            match self.match_at(&s, pos) {
                Some(caps) => {
                    let (st, en) = caps[0].unwrap_or((pos, pos));
                    expand(replacement, &s, &caps, &mut out);
                    if en == st {
                        if let Some(c) = s.get(pos) {
                            out.push(*c);
                        }
                        pos += 1;
                    } else {
                        pos = en;
                    }
                }
                None => {
                    if let Some(c) = s.get(pos) {
                        out.push(*c);
                    }
                    pos += 1;
                }
            }
        }
        out
    }

    fn match_at(&self, s: &[char], start: usize) -> Option<Caps> {
        let mut caps: Caps = vec![None; self.groups + 1];
        let mut end = None;
        if m(&self.root, s, start, &mut caps, &mut |e, _| {
            end = Some(e);
            true
        }) {
            caps[0] = end.map(|e| (start, e));
            Some(caps)
        } else {
            None
        }
    }
}

/// Expand a .NET replacement pattern.
fn expand(rep: &str, s: &[char], caps: &Caps, out: &mut String) {
    let r: Vec<char> = rep.chars().collect();
    let group = |n: usize, out: &mut String| {
        if let Some(Some((a, b))) = caps.get(n) {
            out.extend(&s[*a..*b]);
        }
    };
    let mut i = 0;
    while i < r.len() {
        if r[i] != '$' || i + 1 >= r.len() {
            out.push(r[i]);
            i += 1;
            continue;
        }
        let next = r[i + 1];
        if next == '$' {
            out.push('$');
            i += 2;
        } else if next == '&' {
            group(0, out);
            i += 2;
        } else if next == '{' {
            let close = r[i + 2..].iter().position(|&c| c == '}');
            let num = close.and_then(|c| r[i + 2..i + 2 + c].iter().collect::<String>().parse::<usize>().ok());
            match (close, num) {
                (Some(c), Some(n)) if n < caps.len() => {
                    group(n, out);
                    i += c + 3;
                }
                _ => {
                    out.push('$');
                    i += 1;
                }
            }
        } else if next.is_ascii_digit() {
            // .NET takes the longest digit run that names an existing group.
            let digits: Vec<char> = r[i + 1..].iter().take_while(|c| c.is_ascii_digit()).copied().collect();
            let best = (1..=digits.len()).rev().find_map(|l| {
                let n: usize = digits[..l].iter().collect::<String>().parse().ok()?;
                (n < caps.len()).then_some((n, l))
            });
            match best {
                Some((n, l)) => {
                    group(n, out);
                    i += l + 1;
                }
                None => {
                    out.push('$');
                    i += 1;
                }
            }
        } else {
            out.push('$');
            i += 1;
        }
    }
}

/// Backtracking matcher in continuation-passing style.
fn m(n: &Node, s: &[char], pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
    match n {
        Node::Char(c) => s.get(pos) == Some(c) && k(pos + 1, caps),
        Node::Any => s.get(pos).is_some_and(|&c| c != '\n') && k(pos + 1, caps),
        Node::Class(items, neg) => s.get(pos).is_some_and(|&c| items.iter().any(|i| i.matches(c)) != *neg) && k(pos + 1, caps),
        Node::Start => pos == 0 && k(pos, caps),
        Node::End => (pos == s.len() || (pos + 1 == s.len() && s[pos] == '\n')) && k(pos, caps),
        Node::Group(inner, idx) => m(inner, s, pos, caps, &mut |e, c: &mut Caps| {
            let Some(i) = *idx else { return k(e, c) };
            let prev = c[i];
            c[i] = Some((pos, e));
            if k(e, c) {
                return true;
            }
            c[i] = prev;
            false
        }),
        Node::Concat(v) => seq(v, s, pos, caps, k),
        Node::Alt(v) => {
            for alt in v {
                let saved = caps.clone();
                if m(alt, s, pos, caps, k) {
                    return true;
                }
                *caps = saved;
            }
            false
        }
        Node::Repeat { node, min, max, greedy } => rep(node, *min, *max, *greedy, 0, s, pos, caps, k),
    }
}

fn seq(v: &[Node], s: &[char], pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
    match v.split_first() {
        None => k(pos, caps),
        Some((first, rest)) => m(first, s, pos, caps, &mut |p, c: &mut Caps| seq(rest, s, p, c, k)),
    }
}

#[allow(clippy::too_many_arguments)]
fn rep(
    n: &Node,
    min: usize,
    max: usize,
    greedy: bool,
    count: usize,
    s: &[char],
    pos: usize,
    caps: &mut Caps,
    k: &mut dyn FnMut(usize, &mut Caps) -> bool,
) -> bool {
    let more = |caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool| {
        count < max
            && m(n, s, pos, caps, &mut |p, c: &mut Caps| {
                // An empty iteration past the minimum cannot make progress.
                if p == pos && count >= min {
                    return false;
                }
                rep(n, min, max, greedy, count + 1, s, p, c, k)
            })
    };
    if greedy {
        more(caps, k) || (count >= min && k(pos, caps))
    } else {
        (count >= min && k(pos, caps)) || more(caps, k)
    }
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn unsupported<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("unsupported regex construct {what} in {:?}", self.src))
    }

    fn alternation(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.pos += 1;
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap_or(Node::Concat(Vec::new())) } else { Node::Alt(alts) })
    }

    fn concat(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(Node::Concat(items))
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let single = match self.peek() {
            Some('*') => Some((0, usize::MAX)),
            Some('+') => Some((1, usize::MAX)),
            Some('?') => Some((0, 1)),
            _ => None,
        };
        let (min, max) = match single {
            Some(q) => {
                self.pos += 1;
                q
            }
            None if self.peek() == Some('{') => match self.braces() {
                Some(b) => b,
                None => return Ok(atom),
            },
            None => return Ok(atom),
        };
        let greedy = if self.peek() == Some('?') {
            self.pos += 1;
            false
        } else {
            true
        };
        if matches!(self.peek(), Some('*' | '+' | '?' | '{')) {
            return self.unsupported("(stacked quantifier)");
        }
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    /// `{n}`, `{n,}`, `{n,m}`; on success `pos` moves past the closing brace.
    fn braces(&mut self) -> Option<(usize, usize)> {
        let rest: String = self.chars[self.pos + 1..].iter().collect();
        let close = rest.find('}')?;
        let body = &rest[..close];
        let (min, max) = match body.split_once(',') {
            None => {
                let n = body.parse().ok()?;
                (n, n)
            }
            Some((a, "")) => (a.parse().ok()?, usize::MAX),
            Some((a, b)) => (a.parse().ok()?, b.parse().ok()?),
        };
        self.pos += close + 2;
        Some((min, max))
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.peek().ok_or("unexpected end of regex")?;
        self.pos += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => self.group()?,
            '[' => self.class()?,
            '\\' => self.escape(false)?,
            '*' | '+' | '?' => return self.unsupported("(quantifier without operand)"),
            _ => Node::Char(c),
        })
    }

    fn group(&mut self) -> Result<Node, String> {
        let idx = if self.peek() == Some('?') {
            if self.chars.get(self.pos + 1) != Some(&':') {
                return self.unsupported("(?...)");
            }
            self.pos += 2;
            None
        } else {
            self.groups += 1;
            Some(self.groups)
        };
        let inner = self.alternation()?;
        if self.peek() != Some(')') {
            return Err(format!("missing ')' in regex {:?}", self.src));
        }
        self.pos += 1;
        Ok(Node::Group(Box::new(inner), idx))
    }

    fn class(&mut self) -> Result<Node, String> {
        let neg = self.peek() == Some('^');
        if neg {
            self.pos += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or_else(|| format!("missing ']' in regex {:?}", self.src))?;
            self.pos += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                match self.escape(true)? {
                    Node::Char(ch) => ch,
                    Node::Class(mut v, _) => {
                        items.append(&mut v);
                        continue;
                    }
                    _ => return self.unsupported("(escape in class)"),
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|&n| n != ']') {
                self.pos += 1;
                let hi = self.peek().ok_or("bad range")?;
                self.pos += 1;
                if hi == '\\' {
                    return self.unsupported("(escaped range end)");
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
        Ok(Node::Class(items, neg))
    }

    fn escape(&mut self, in_class: bool) -> Result<Node, String> {
        let c = self.peek().ok_or("trailing backslash")?;
        self.pos += 1;
        let class = |item| Node::Class(vec![item], false);
        Ok(match c {
            'd' => class(ClassItem::Digit(false)),
            'D' => class(ClassItem::Digit(true)),
            'w' => class(ClassItem::Word(false)),
            'W' => class(ClassItem::Word(true)),
            's' => class(ClassItem::Space(false)),
            'S' => class(ClassItem::Space(true)),
            't' => Node::Char('\t'),
            'n' => Node::Char('\n'),
            'r' => Node::Char('\r'),
            c if c.is_ascii_alphanumeric() && !in_class => return self.unsupported(&format!("\\{c}")),
            c if c.is_ascii_alphanumeric() => return self.unsupported(&format!("\\{c} in a class")),
            c => Node::Char(c),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Regex;

    #[test]
    fn neotokyo_avail_pattern() {
        let re = Regex::new("([0-9]+)([FR]*)").unwrap();
        assert_eq!(re.replace_all("4R", "2+$1F"), "2+4F");
        assert_eq!(re.replace_all("12", "$1F"), "12F");
        assert_eq!(re.replace_all("+2", "4+$1F"), "+4+2F");
        assert_eq!(re.replace_all("", "$1F"), "");
    }

    #[test]
    fn general_constructs() {
        let re = Regex::new(r"^(?:a|b)+?c\d{2,3}$").unwrap();
        assert_eq!(re.replace_all("abac123", "X"), "X");
        assert_eq!(Regex::new("o").unwrap().replace_all("foo", "0"), "f00");
        assert_eq!(Regex::new("x*").unwrap().replace_all("ab", "-"), "-a-b-");
        assert_eq!(Regex::new("(a)(b)").unwrap().replace_all("ab", "$2$1$$${1}"), "ba$a");
        assert!(Regex::new("(?<n>a)").is_err());
        assert!(Regex::new(r"\bx").is_err());
    }
}
