//! The `chummer-cli` binary on odd input: missing, empty, binary, deeply
//! nested and oddly named files, bad arguments. Every case must end with a
//! normal exit (no signal), and errors must say so on stderr without a
//! panic.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures").join(name)
}

/// A scratch directory that also serves as `HOME` / `XDG_CONFIG_HOME`, so
/// nothing touches the real configuration.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let d = std::env::temp_dir().join(format!("chummer-cli-robust-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
    fn run<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        Command::new(env!("CARGO_BIN_EXE_chummer-cli"))
            .args(&args)
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn describe(args: &str, o: &Output) -> String {
    format!("`chummer-cli {args}`: {:?}\nstdout: {}\nstderr: {}", o.status, String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Exited normally (not killed by a signal, no panic).
fn assert_no_crash(args: &str, o: &Output) {
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.code().is_some(), "killed by a signal: {}", describe(args, o));
    assert!(!err.contains("panicked"), "panicked: {}", describe(args, o));
    assert_ne!(o.status.code(), Some(101), "panic exit code: {}", describe(args, o));
}

/// Failed with an error message.
fn assert_error(args: &str, o: &Output) {
    assert_no_crash(args, o);
    assert!(!o.status.success(), "succeeded: {}", describe(args, o));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error"), "no error message: {}", describe(args, o));
}

#[test]
fn bad_character_files() {
    let s = Scratch::new("files");
    let empty = s.write("empty.chum5", b"");
    let garbage = s.write("garbage.chum5", &(0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<_>>());
    let garbage_lz = s.write("garbage.chum5lz", b"\x5d\x00\x00\x00\x01\xff\xff\xff\xff\xff\xff\xff\xffnot lzma at all");
    let forged_lz = s.write("forged.chum5lz", b"\x5d\xf0\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\x00\x00\x00\x00\x00\x00\x00\x00");
    let not_char = s.write("settings.chum5", b"<settings><name>x</name></settings>");
    let truncated = {
        let src = std::fs::read(fixture("Davis Jones.chum5")).unwrap();
        s.write("truncated.chum5", &src[..src.len() / 2])
    };
    let deep = s.write("deep.chum5", format!("<character>{}{}</character>", "<a>".repeat(300_000), "</a>".repeat(300_000)).as_bytes());
    let utf16 = s.write("utf16.chum5", &"<character/>".encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<_>>());
    let missing = s.path("missing.chum5");
    let dir = s.path("adir.chum5");
    std::fs::create_dir_all(&dir).unwrap();
    let long = s.path(&format!("{}.chum5", "x".repeat(5000)));
    for p in [&empty, &garbage, &garbage_lz, &forged_lz, &not_char, &truncated, &deep, &utf16, &missing, &dir, &long] {
        for cmd in ["hash", "items"] {
            let o = s.run([OsString::from(cmd), p.clone().into_os_string()]);
            assert_error(&format!("{cmd} {}", p.display()), &o);
        }
    }
    // One command that loads the game data too.
    let o = s.run([OsString::from("info"), deep.into_os_string()]);
    assert_error("info deep.chum5", &o);
    // `check` reports bad files and carries on.
    let o = s.run(["check", s.0.to_str().unwrap()]);
    assert_no_crash("check <dir>", &o);
    assert!(String::from_utf8_lossy(&o.stdout).contains("FAIL"), "{}", describe("check <dir>", &o));
}

#[cfg(unix)]
#[test]
fn non_utf8_arguments() {
    use std::os::unix::ffi::OsStringExt;
    let s = Scratch::new("utf8");
    let name = OsString::from_vec(b"\xff\xfe.chum5".to_vec());
    let o = s.run([OsString::from("hash"), name.clone()]);
    assert_error("hash <non-UTF-8 name>", &o);
    let o = s.run([name.clone()]);
    assert_error("<non-UTF-8 command>", &o);
    let o = s.run([OsString::from("check"), name]);
    assert_error("check <non-UTF-8 name>", &o);
}

#[test]
fn bad_arguments() {
    let s = Scratch::new("args");
    let davis = fixture("Davis Jones.chum5");
    let davis = davis.to_str().unwrap();
    let out = s.path("out.chum5");
    let out = out.to_str().unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec!["frobnicate"],
        vec!["--frobnicate"],
        vec!["-"],
        vec![""],
        vec!["info"],
        vec!["info", davis, davis],
        vec!["hash", "--bogus"],
        vec!["apply"],
        vec!["apply", davis],
        vec!["apply", davis, "missing-log.json"],
        vec!["apply", davis, davis, "-x", "y"],
        vec!["export", davis],
        vec!["export", davis, "XML"],
        vec!["export", davis, "NoSuchFormat", "-o", out],
        vec!["sheet"],
        vec!["sheet", davis, "--sheet", "../../../../etc/passwd", "-o", out],
        vec!["campaign"],
        vec!["campaign", "frob"],
        vec!["campaign", "list", davis],
        vec!["campaign", "add", "missing.chummercampaign", davis],
        vec!["settings", "frob"],
        vec!["settings", "export", "No Such Preset", "-o", out],
        vec!["settings", "import", "missing.xml"],
        vec!["settings", "import", davis, "--overwrite", "--keep-both"],
        vec!["new"],
        vec!["new", out, "--priorities", "ZZZZZ"],
        vec!["new", out, "--priorities", "AAAA"],
        vec!["new", out, "--settings", "No Such Preset"],
        vec!["new", out, "--metatype", "Dragon"],
        vec!["search"],
    ];
    for args in &cases {
        let o = s.run(args.iter().copied());
        assert_error(&args.join(" "), &o);
    }
    // Odd but accepted: no crash either way.
    for args in [vec!["new", out, "--priorities", "ééééé"], vec!["new", out, "--talent", "Dragon"], vec!["search", ""], vec!["sheets", "../../.."], vec!["roster", "/nonexistent"]] {
        let o = s.run(args.iter().copied());
        assert_no_crash(&args.join(" "), &o);
    }
}

#[test]
fn apply_with_odd_logs() {
    let s = Scratch::new("apply");
    let davis = fixture("Davis Jones.chum5");
    let logs: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("not json", "nope".into()),
        ("empty array", "[]".into()),
        ("unknown command", r#"[{"Frobnicate": {}}]"#.into()),
        ("wrong field type", r#"[{"SetKarma": {"value": "lots"}}]"#.into()),
        ("out of range", r#"[{"SetKarma": {"value": 99999999999}}]"#.into()),
        ("deep", format!("{}{}", "[".repeat(100_000), "]".repeat(100_000))),
        (
            "extreme times",
            r#"[{"cmd": {"SetField": {"key": "alias", "value": "a"}}, "seed": 1, "at": -9223372036854775808, "author": ""},
                {"cmd": {"SetField": {"key": "alias", "value": "b"}}, "seed": 2, "at": 9223372036854775807, "author": ""}]"#
                .into(),
        ),
        ("revert garbage", r#"[{"Revert": {"snapshot": [93, 0, 0, 0, 1, 255], "what": "x", "from": 0, "to": 1}}]"#.into()),
        ("kit garbage", r#"[{"ApplyKit": {"kit": "<pack><"}}]"#.into()),
    ];
    for (name, log) in logs {
        let p = s.write("log.json", log.as_bytes());
        let o = s.run([OsString::from("apply"), davis.clone().into_os_string(), p.into_os_string()]);
        assert_no_crash(&format!("apply <{name}>"), &o);
    }
}
