//! Damaged `.chum5` files must load or fail with an error, never panic.
//!
//! Every fixture is mutated with a seeded generator (truncation, flipped
//! bytes, deleted / duplicated / renamed elements, extreme numbers, BOMs,
//! bad UTF-8, odd entities) and fed through `Character::from_str` or
//! `Character::load`. Characters that load are then computed, printed
//! and saved, which is where bad numbers would bite.
//!
//! Failures name the fixture, the seed and the mutation. Rerun one with
//! `CHUMMER_FUZZ_SEED=<seed>`; `CHUMMER_FUZZ_ITERS=<n>` multiplies the
//! number of rounds.

mod common;

use std::path::Path;

use chummer_core::character::Character;
use chummer_core::command;
use chummer_core::xml;
use common::{engine, fixture_name, iters, no_panic, Prng};

/// Values that once broke parsers and arithmetic.
const NUMBERS: &[&str] = &[
    "",
    " ",
    "-1",
    "0",
    "-0",
    "2147483647",
    "-2147483648",
    "2147483648",
    "9223372036854775807",
    "-9223372036854775808",
    "18446744073709551616",
    "1e308",
    "-1e308",
    "1e-308",
    "NaN",
    "-NaN",
    "inf",
    "-inf",
    "Infinity",
    "1,5",
    "1.5",
    "+7",
    "0x10",
    "１２",
    "99999999999999999999999999999999",
    "True",
    "-",
    ".",
    "1e",
];

/// Start of element tags (`<name`) in `s`, as byte offsets.
fn tag_starts(s: &str) -> Vec<usize> {
    s.match_indices('<').map(|(i, _)| i).filter(|&i| s[i + 1..].starts_with(|c: char| c.is_ascii_alphabetic())).collect()
}

fn tag_name_at(s: &str, i: usize) -> &str {
    let rest = &s[i + 1..];
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':')).unwrap_or(rest.len());
    &rest[..end]
}

/// The byte range of the element starting at `i` (up to its matching end
/// tag, ignoring nesting of the same name), if found.
fn element_range(s: &str, i: usize) -> Option<std::ops::Range<usize>> {
    let name = tag_name_at(s, i);
    let open_end = i + s[i..].find('>')? + 1;
    if s[..open_end].ends_with("/>") {
        return Some(i..open_end);
    }
    let close = format!("</{name}>");
    let j = open_end + s[open_end..].find(&close)? + close.len();
    Some(i..j)
}

/// Leaf elements whose text is a number: the range of the text.
fn numeric_leaves(s: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    for i in tag_starts(s) {
        let Some(gt) = s[i..].find('>') else { continue };
        let a = i + gt + 1;
        if s[i..a].ends_with("/>") {
            continue;
        }
        let Some(lt) = s[a..].find('<') else { continue };
        let text = &s[a..a + lt];
        if !text.is_empty() && text.len() < 24 && text.trim().parse::<f64>().is_ok() {
            out.push(a..a + lt);
        }
    }
    out
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// One mutation of `src`: a description and the bytes.
fn mutate(src: &str, rng: &mut Prng) -> (String, Vec<u8>) {
    let tags = tag_starts(src);
    if tags.is_empty() || src.is_empty() {
        return ("no tags left".into(), src.as_bytes().to_vec());
    }
    match rng.below(14) {
        0 => {
            let at = floor_char(src, rng.below(src.len() + 1));
            (format!("truncate at {at}"), src.as_bytes()[..at].to_vec())
        }
        1 => {
            let mut b = src.as_bytes().to_vec();
            let n = 1 + rng.below(8);
            let mut desc = String::from("flip bytes");
            for _ in 0..n {
                let at = rng.below(b.len());
                let v = rng.next_u64() as u8;
                desc.push_str(&format!(" {at}={v:#04x}"));
                b[at] = v;
            }
            (desc, b)
        }
        2 => {
            let i = *rng.pick(&tags);
            match element_range(src, i) {
                Some(r) => (format!("delete element <{}> at {}", tag_name_at(src, i), r.start), [&src[..r.start], &src[r.end..]].concat().into_bytes()),
                None => (format!("cut at tag {i}"), src.as_bytes()[..i].to_vec()),
            }
        }
        3 => {
            let i = *rng.pick(&tags);
            match element_range(src, i) {
                Some(r) => {
                    let times = 1 + rng.below(3);
                    let dup = src[r.clone()].repeat(times);
                    (format!("duplicate element <{}> at {} x{times}", tag_name_at(src, i), r.start), [&src[..r.end], &dup, &src[r.end..]].concat().into_bytes())
                }
                None => (format!("noop at {i}"), src.as_bytes().to_vec()),
            }
        }
        4 => {
            // Rename one start tag only (unbalanced) or a whole element.
            let i = *rng.pick(&tags);
            let j = *rng.pick(&tags);
            let (from, to) = (tag_name_at(src, i), tag_name_at(src, j));
            let whole = rng.chance(1, 2);
            let mut out = String::with_capacity(src.len());
            out.push_str(&src[..i + 1]);
            out.push_str(to);
            let rest = &src[i + 1 + from.len()..];
            match element_range(src, i).filter(|_| whole) {
                Some(r) => {
                    if src[..r.end].ends_with("/>") {
                        out.push_str(rest);
                    } else {
                        let inner_end = r.end - from.len() - 3 - (i + 1 + from.len());
                        out.push_str(&rest[..inner_end]);
                        out.push_str(&format!("</{to}>"));
                        out.push_str(&src[r.end..]);
                    }
                }
                None => out.push_str(rest),
            }
            (format!("rename <{from}> at {i} to <{to}> (whole: {whole})"), out.into_bytes())
        }
        5..=7 => {
            // Extreme values into several numeric leaves.
            let leaves = numeric_leaves(src);
            if leaves.is_empty() {
                return ("no numeric leaves".into(), src.as_bytes().to_vec());
            }
            let mut picks: Vec<(std::ops::Range<usize>, &str)> = (0..1 + rng.below(12)).map(|_| (rng.pick(&leaves).clone(), *rng.pick(NUMBERS))).collect();
            picks.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
            picks.dedup_by_key(|(r, _)| r.start);
            let mut out = src.to_owned();
            let mut desc = String::from("numbers");
            for (r, v) in &picks {
                let at = r.start - src[..r.start].rfind('<').unwrap_or(0);
                desc.push_str(&format!(" <{}>@{}={v:?}", tag_name_at(src, r.start - at), r.start));
                out.replace_range(r.clone(), v);
            }
            (desc, out.into_bytes())
        }
        8 => {
            let bom: &[u8] = rng.pick(&[&b"\xEF\xBB\xBF"[..], b"\xEF\xBB\xBF\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF", b"\x00"]);
            (format!("prefix {bom:02x?}"), [bom, src.as_bytes()].concat())
        }
        9 => {
            let at = floor_char(src, rng.below(src.len()));
            let bad: &[u8] = rng.pick(&[&b"\xFF"[..], b"\xC3", b"\xED\xA0\x80", b"\xF4\x90\x80\x80", b"\xC0\xAF"]);
            (format!("invalid UTF-8 {bad:02x?} at {at}"), [&src.as_bytes()[..at], bad, &src.as_bytes()[at..]].concat())
        }
        10 => {
            let leaves = numeric_leaves(src);
            let ent = *rng.pick(&["&#0;", "&#xD800;", "&#x110000;", "&#xFFFFFFFFFF;", "&#-1;", "&bogus;", "&", "&#;", "&#x;", "&lt", "]]>", "<![CDATA[x", "<!--", "<?pi?>", "<!DOCTYPE x [<!ENTITY a \"b\">]>"]);
            let at = match leaves.first() {
                Some(_) => rng.pick(&leaves).start,
                None => floor_char(src, rng.below(src.len())),
            };
            (format!("insert {ent:?} at {at}"), [&src[..at], ent, &src[at..]].concat().into_bytes())
        }
        11 => {
            // Swap two random lines.
            let mut lines: Vec<&str> = src.lines().collect();
            let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
            lines.swap(a, b);
            (format!("swap lines {a} and {b}"), lines.join("\n").into_bytes())
        }
        12 => {
            // Random junk at a random place.
            let at = floor_char(src, rng.below(src.len()));
            let junk: String = (0..1 + rng.below(16)).map(|_| *rng.pick(&['<', '>', '/', '"', '\'', '=', '&', ';', 'a', ' ', '\n', '\0', 'é', '¥'])).collect();
            (format!("insert junk {junk:?} at {at}"), [&src[..at], &junk, &src[at..]].concat().into_bytes())
        }
        _ => {
            // Several mutations stacked.
            let (d1, b1) = mutate(src, rng);
            let s1 = String::from_utf8_lossy(&b1).into_owned();
            let (d2, b2) = mutate(&s1, rng);
            (format!("{d1}; then {d2}"), b2)
        }
    }
}

/// Load from bytes, then exercise what a loaded character goes through.
fn exercise(bytes: &[u8], deep: bool) -> String {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.to_owned(),
        Err(_) => {
            // `from_str` cannot see bad UTF-8: go through a file.
            let p = std::env::temp_dir().join(format!("chummer-rs-fuzz-load-{}-{:?}.chum5", std::process::id(), std::thread::current().id()));
            std::fs::write(&p, bytes).unwrap();
            let r = Character::load(&p);
            let _ = std::fs::remove_file(&p);
            return match r {
                Ok(_) => "loaded (lossy)".into(),
                Err(e) => format!("error: {e}"),
            };
        }
    };
    let ch = match Character::from_str(&text) {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    let saved = ch.to_xml_string();
    // What we write must load again.
    let again = Character::from_str(&saved).unwrap_or_else(|e| panic!("saved XML does not load: {e}"));
    let _ = command::state_hash(&again);
    let _ = ch.display_name();
    let eng = engine();
    let sheet = eng.sheet(&ch);
    let _ = (sheet.essence, sheet.initiative);
    if deep {
        let lang = chummer_core::lang::Language::load(&chummer_core::data::resource_dir("lang").unwrap(), "en-us");
        let _ = chummer_core::print::print_xml(&ch, eng, &lang);
        let rules = eng.rules_for(&ch);
        let store = eng.store_for_character(&ch);
        if let Some(settings) = eng.settings.resolve(&ch.field("settings")) {
            let b = chummer_core::chargen::budget_with(&ch, &sheet, &rules, settings, Some(&store));
            let _ = chummer_core::chargen::issues::issues(&ch, &b, &sheet, settings, Some(&store));
        }
        let _ = chummer_core::career::entries(&ch);
        let mut copy = ch.clone();
        let p = std::env::temp_dir().join(format!("chummer-rs-fuzz-save-{}-{:?}.chum5", std::process::id(), std::thread::current().id()));
        eng.save(&mut copy, &p).unwrap();
        let _ = std::fs::remove_file(&p);
    }
    "loaded".into()
}

/// Any panic is a failure. Absurd numbers used to overflow the `i32`
/// rules math (LB-44); file integers are clamped now and the cost
/// formulas work in `i64`.
fn report(what: &str, panics: Vec<String>) {
    assert!(panics.is_empty(), "{what}: {} panics:\n{}", panics.len(), panics.join("\n"));
}

#[test]
fn mutated_fixtures_never_panic() {
    let seed0 = common::base_seed();
    let long = std::env::var_os("CHUMMER_FUZZ_ITERS").is_some();
    let mut panics = Vec::new();
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    let started = std::time::Instant::now();
    for (fi, path) in common::fixtures().iter().enumerate() {
        let src = std::fs::read_to_string(path).unwrap();
        // Debug builds parse a few MB per second: the two multi-megabyte
        // fixtures only join long runs.
        let rounds = match src.len() {
            n if n > 1_000_000 => usize::from(long),
            n if n > 200_000 => iters(2),
            _ => iters(6),
        };
        for round in 0..rounds {
            let seed = seed0 ^ ((fi as u64) << 32) ^ round as u64;
            let mut rng = Prng::new(seed);
            let (desc, bytes) = mutate(&src, &mut rng);
            let deep = round % 3 == 0 && src.len() < 200_000;
            match no_panic(|| exercise(&bytes, deep)) {
                Ok(outcome) => *outcomes.entry(outcome.split(':').next().unwrap().to_owned()).or_default() += 1,
                Err(panic) => {
                    let saved = common::save_failure(&format!("load-{seed:x}"), &bytes);
                    panics.push(format!("{} seed {seed:#x} [{desc}] -> {panic} (input saved to {})", fixture_name(path), saved.display()));
                }
            }
        }
    }
    eprintln!("mutated loads: {outcomes:?} in {:?}", started.elapsed());
    report("mutated fixtures", panics);
}

/// Extreme values in the numeric fields of a small career fixture: each
/// of the worst values in every field at once, and a sample of single
/// fields with every value.
fn extreme_number_panics() -> Vec<String> {
    let worst = ["2147483647", "-2147483648", "NaN", "1e308", "-1e308", "9223372036854775807", "inf"];
    let mut groups: Vec<(String, String)> = Vec::new();
    // Every field at once on a few more characters: creation, magic,
    // technomancer, vehicles.
    // Long runs (CHUMMER_FUZZ_ITERS) take every small fixture.
    let mut paths: Vec<std::path::PathBuf> = ["Davis Jones", "Draught", "Soma (Career)", "Skink", "Apex Predator"].iter().map(|n| common::fixtures_dir().join(format!("{n}.chum5"))).collect();
    if std::env::var_os("CHUMMER_FUZZ_ITERS").is_some() {
        paths = common::small_fixtures(200_000);
    }
    for path in &paths {
        let name = fixture_name(path);
        let other = std::fs::read_to_string(path).unwrap();
        let leaves = numeric_leaves(&other);
        for v in worst {
            let mut out = other.clone();
            for r in leaves.iter().rev() {
                out.replace_range(r.clone(), v);
            }
            groups.push((format!("{name}: all numeric fields = {v}"), out));
        }
    }
    let path = common::fixtures_dir().join("Munin_Career.chum5");
    let src = std::fs::read_to_string(&path).unwrap();
    let leaves = numeric_leaves(&src);
    assert!(leaves.len() > 50, "{}", leaves.len());
    let mut rng = Prng::new(common::base_seed());
    for v in worst {
        let mut out = src.clone();
        for r in leaves.iter().rev() {
            out.replace_range(r.clone(), v);
        }
        groups.push((format!("Munin_Career: all numeric fields = {v}"), out));
    }
    for _ in 0..iters(20) {
        let r = rng.pick(&leaves).clone();
        let v = *rng.pick(NUMBERS);
        let name_at = src[..r.start].rfind('<').unwrap();
        groups.push((format!("Munin_Career <{}>@{} = {v:?}", tag_name_at(&src, name_at), r.start), [&src[..r.start], v, &src[r.end..]].concat()));
    }
    groups.into_iter().filter_map(|(desc, text)| no_panic(|| exercise(text.as_bytes(), true)).err().map(|p| format!("[{desc}] -> {p}"))).collect()
}

/// `<base>2147483647</base>`, `1e308` and the like in every numeric field
/// load, compute, print and save without overflowing (LB-44, fixed).
#[test]
fn extreme_numbers_in_numeric_fields() {
    report("extreme numbers", extreme_number_panics());
}

/// The values are clamped where they are read: the save is sane.
#[test]
fn extreme_numbers_are_clamped_on_load() {
    let src = std::fs::read_to_string(common::fixtures_dir().join("Munin_Career.chum5")).unwrap();
    let base = Character::from_str(&src).unwrap();
    let doc = src.replacen(&format!("<karma>{}</karma>", base.karma), "<karma>2147483647</karma>", 1);
    assert_eq!(Character::from_str(&doc).unwrap().karma, chummer_core::xml::NUM_LIMIT);
    let doc = src.replace("<base>", "<base>-92233720368547758").replace("<karma>", "<karma>9");
    let ch = Character::from_str(&doc).unwrap();
    assert!(ch.attributes.iter().all(|a| a.base.abs() <= chummer_core::xml::NUM_LIMIT && a.karma.abs() <= chummer_core::xml::NUM_LIMIT));
    assert_eq!(chummer_core::xml::parse_int(" +99999999999 "), Some(chummer_core::xml::NUM_LIMIT));
    assert_eq!(chummer_core::xml::parse_f64("NaN"), None);
    assert_eq!(chummer_core::xml::parse_f64("-inf"), None);
    assert_eq!(chummer_core::expr::standard_round(1e308), chummer_core::xml::NUM_LIMIT);
    assert_eq!(chummer_core::expr::standard_round(f64::NAN), 0);
}

/// Hand-made odd documents.
#[test]
fn odd_documents() {
    let deep_ok = format!("<character>{}{}</character>", "<a>".repeat(200), "</a>".repeat(200));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", b"".to_vec()),
        ("whitespace", b"  \n\t ".to_vec()),
        ("bom only", b"\xEF\xBB\xBF".to_vec()),
        ("declaration only", b"<?xml version=\"1.0\"?>".to_vec()),
        ("empty character", b"<character/>".to_vec()),
        ("empty character pair", b"<character></character>".to_vec()),
        ("other root", b"<settings><name>x</name></settings>".to_vec()),
        ("two roots", b"<character/><character/>".to_vec()),
        ("text root", b"hello".to_vec()),
        ("stray end", b"</character>".to_vec()),
        ("unclosed", b"<character><karma>5</karma>".to_vec()),
        ("mismatched", b"<character><karma>5</nuyen></character>".to_vec()),
        ("attributes only", b"<character><attributes><attribute/><attribute><name/></attribute></attributes></character>".to_vec()),
        ("empty sections", b"<character><newskills><skills><skill/></skills><knoskills><skill/></knoskills><groups><group/></groups></newskills><improvements><improvement/></improvements></character>".to_vec()),
        ("utf-16", "<character/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect()),
        ("nul", b"<character>\0</character>".to_vec()),
        ("200 deep", deep_ok.into_bytes()),
        ("990 deep", format!("<character>{}{}</character>", "<a>".repeat(990), "</a>".repeat(990)).into_bytes()),
    ];
    let mut failures = Vec::new();
    for (name, bytes) in &cases {
        if let Err(p) = no_panic(|| exercise(bytes, true)) {
            failures.push(format!("{name} -> {p}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Nesting deep enough to overflow a recursive walk is refused by the
/// parser instead of crashing the process (a stack overflow aborts; it
/// cannot be caught).
#[test]
fn absurd_nesting_is_an_error() {
    for depth in [10_000usize, 200_000] {
        let doc = format!("<character>{}{}</character>", "<a>".repeat(depth), "</a>".repeat(depth));
        assert!(xml::parse(&doc).is_err(), "depth {depth} parsed");
        assert!(Character::from_str(&doc).is_err(), "depth {depth} loaded");
        // Unclosed, too.
        let open = format!("<character>{}", "<a>".repeat(depth));
        assert!(xml::parse(&open).is_err());
    }
    // Real files are far shallower than the limit.
    for p in common::small_fixtures(200_000) {
        assert!(Character::load(&p).is_ok(), "{}", p.display());
    }
}

#[test]
fn missing_and_odd_paths() {
    let dir = common::temp_dir("paths");
    let cases = [dir.join("nope.chum5"), dir.join("nope.chum5lz"), dir.clone(), Path::new("").to_path_buf(), dir.join("x".repeat(300) + ".chum5")];
    for p in &cases {
        let r = no_panic(|| Character::load(p).is_err()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(r, "{} loaded", p.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
