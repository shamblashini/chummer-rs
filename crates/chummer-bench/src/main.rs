//! `chummer-bench`: the performance suite (docs/testing.md, "Performance
//! tests"). Not shipped.
//!
//! - `run [--quick] [--only a,b] [--out file]`: the engine, file, command,
//!   save, print and sync benchmarks; writes a JSON report.
//! - `check --budgets perf/budgets.toml [--previous file]... [--out file]
//!   [--summary file] report...`: merges reports (this one and the GUI's),
//!   checks budgets and the change since the previous run, prints the
//!   table, appends Markdown to `--summary`. Exits 1 on a failure.
//! - `survey`: size, items and compute time of every fixture, to choose
//!   the heavy ones.
//! - `probe cold|memory ...`: run in a child process by `run`, so that
//!   "cold" is a fresh process and the peak memory is only what it loads.
//!
//! Settings and data go to `$CHUMMER_BENCH_DIR` (else a temporary
//! directory): the XDG variables are pointed there before the engine
//! loads, so the user's presets neither change the numbers nor get
//! written to.

mod core_benches;
mod sync_benches;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chummer_bench::{check, markdown, ms, peak_rss_mib, table, Budgets, Record, Report, Runner};
use chummer_core::character::Character;
use chummer_core::engine::Engine;

/// The character with the most items (gear trees, cyberware, vehicles).
pub const HEAVY: &str = "Serpent";
/// The biggest file (mostly its mugshots).
pub const BIG: &str = "Ghile Mear";

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures").join(format!("{name}.chum5"))
}

pub fn slug(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// Where the benchmarks may write.
pub fn work_dir() -> PathBuf {
    std::env::var_os("CHUMMER_BENCH_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("chummer-bench-{}", std::process::id())))
}

/// Where saves and sheets are written: a tmpfs when there is one, so
/// that the numbers are the work of saving and not the disk's latency
/// (each save syncs the file and its folder). `core.save.disk_*` saves
/// to the work directory instead.
pub fn files_dir() -> PathBuf {
    let shm = Path::new("/dev/shm");
    let base = if shm.is_dir() && std::fs::create_dir_all(shm.join(format!("chummer-bench-{}", std::process::id()))).is_ok() { shm.join(format!("chummer-bench-{}", std::process::id())) } else { work_dir().join("files") };
    let _ = std::fs::create_dir_all(&base);
    base
}

/// Whether [`files_dir`] is a tmpfs.
pub fn files_in_memory() -> bool {
    files_dir().starts_with("/dev/shm")
}

fn isolate() {
    let dir = work_dir();
    for (var, sub) in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache")] {
        let d = dir.join(sub);
        let _ = std::fs::create_dir_all(&d);
        // Edition 2021, before any thread: safe.
        std::env::set_var(var, d);
    }
}

pub fn engine() -> Result<Engine> {
    Engine::load().context("game data not found (run from the repository, or set CHUMMER_RESOURCES)")
}

pub fn load(name: &str) -> Result<Character> {
    let p = fixture(name);
    Character::load(&p).with_context(|| format!("loading {}", p.display()))
}

fn main() -> Result<()> {
    isolate();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { bail!("usage: chummer-bench run|check|survey|probe ... (see the source's header)") };
    let rest = &args[1..];
    match cmd.as_str() {
        "run" => run(rest),
        "check" => check_cmd(rest),
        "survey" => survey(),
        "probe" => probe(rest),
        other => bail!("unknown command {other:?}"),
    }
}

/// `--flag value` (repeatable) and the free arguments.
struct Flags {
    pairs: Vec<(String, String)>,
    bools: Vec<String>,
    free: Vec<String>,
}

impl Flags {
    fn parse(args: &[String], bools: &[&str]) -> Result<Flags> {
        let mut f = Flags { pairs: Vec::new(), bools: Vec::new(), free: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if bools.contains(&a.as_str()) {
                f.bools.push(a.clone());
            } else if let Some(name) = a.strip_prefix("--") {
                let v = it.next().with_context(|| format!("--{name} needs a value"))?;
                f.pairs.push((name.to_owned(), v.clone()));
            } else {
                f.free.push(a.clone());
            }
        }
        Ok(f)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.pairs.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.pairs.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str()).collect()
    }

    fn has(&self, name: &str) -> bool {
        self.bools.iter().any(|b| b == name)
    }
}

fn run(args: &[String]) -> Result<()> {
    let f = Flags::parse(args, &["--quick"])?;
    let quick = f.has("--quick") || std::env::var("CHUMMER_PERF_QUICK").is_ok_and(|v| !v.is_empty() && v != "0");
    let only: Vec<String> = f.get("only").map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "all").map(str::to_owned).collect()).unwrap_or_default();
    let out = PathBuf::from(f.get("out").unwrap_or("target/perf/core.json"));
    let mut r = Runner::new(quick, only);
    eprintln!("chummer-bench{}: {}", if quick { " (quick)" } else { "" }, r.report.machine);
    let t = Instant::now();
    core_benches::run(&mut r)?;
    sync_benches::run(&mut r)?;
    eprintln!("done in {:.0} s", t.elapsed().as_secs_f64());
    r.report.write(&out)?;
    eprintln!("wrote {}", out.display());
    let _ = std::fs::remove_dir_all(files_dir());
    Ok(())
}

fn check_cmd(args: &[String]) -> Result<()> {
    let f = Flags::parse(args, &[])?;
    let budgets = Budgets::read(Path::new(f.get("budgets").unwrap_or("perf/budgets.toml")))?;
    let mut parts = Vec::new();
    for p in &f.free {
        parts.push(Report::read(Path::new(p))?);
    }
    if parts.is_empty() {
        bail!("no report to check");
    }
    let mut history = Vec::new();
    for p in f.all("previous") {
        match Report::read(Path::new(p)) {
            Ok(r) => history.push(r),
            Err(e) => eprintln!("ignoring previous report: {e}"),
        }
    }
    let c = check(parts, &budgets, &history);
    print!("{}", table(&c));
    for w in &c.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(out) = f.get("out") {
        c.report.write(Path::new(out))?;
    }
    if let Some(s) = f.get("summary") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(s).with_context(|| s.to_owned())?;
        file.write_all(markdown(&c, &history).as_bytes())?;
    }
    if !c.failures.is_empty() {
        eprintln!("{} benchmark(s) failed", c.failures.len());
        std::process::exit(1);
    }
    Ok(())
}

/// Every fixture: bytes, items, career or not, load and compute time.
fn survey() -> Result<()> {
    let engine = engine()?;
    let dir = fixture("x").parent().unwrap().to_owned();
    let mut rows = Vec::new();
    for e in std::fs::read_dir(&dir)?.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "chum5") {
            continue;
        }
        let bytes = std::fs::metadata(&p)?.len();
        let t = Instant::now();
        let ch = Character::load(&p)?;
        let load = ms(t);
        let _ = engine.sheet(&ch);
        let t = Instant::now();
        let _ = engine.sheet(&ch);
        let compute = ms(t);
        let t = Instant::now();
        let _ = chummer_core::command::state_hash(&ch);
        let hash = ms(t);
        let mut items = 0usize;
        fn count(e: &chummer_core::xml::Element, n: &mut usize) {
            if e.child("guid").is_some() {
                *n += 1;
            }
            for c in e.elements() {
                count(c, n);
            }
        }
        count(&ch.doc, &mut items);
        rows.push((compute, format!("{:<32} {:>9} B {:>5} items {:<8} load {:>7.1} ms  compute {:>7.1} ms  hash {:>6.1} ms", p.file_stem().unwrap().to_string_lossy(), bytes, items, if ch.created { "career" } else { "creation" }, load, compute, hash)));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, r) in rows {
        println!("{r}");
    }
    Ok(())
}

/// The child-process measurements.
fn probe(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("cold") => {
            let t = Instant::now();
            let engine = engine()?;
            let load = ms(t);
            let ch = load_path(args.get(1).context("probe cold <character>")?)?;
            let _ = engine.sheet(&ch);
            let first = ms(t);
            println!("{}", serde_json::json!({ "engine_ms": load, "first_sheet_ms": first }));
        }
        Some("memory") => {
            let before = peak_rss_mib().unwrap_or(0.0);
            let engine = engine()?;
            let after_engine = peak_rss_mib().unwrap_or(0.0);
            let mut kept = Vec::new();
            for p in &args[1..] {
                let ch = load_path(p)?;
                let sheet = engine.sheet(&ch);
                kept.push((ch, sheet));
            }
            let peak = peak_rss_mib().unwrap_or(0.0);
            println!("{}", serde_json::json!({ "start_mib": before, "engine_mib": after_engine, "peak_mib": peak, "characters": kept.len() }));
        }
        _ => bail!("probe cold|memory"),
    }
    Ok(())
}

fn load_path(p: &str) -> Result<Character> {
    Character::load(Path::new(p)).with_context(|| p.to_owned())
}

/// Runs `chummer-bench probe ...` in a child process and parses its JSON.
pub fn child(args: &[&str]) -> Result<serde_json::Value> {
    let exe = std::env::current_exe()?;
    let out = std::process::Command::new(exe).arg("probe").args(args).output()?;
    if !out.status.success() {
        bail!("probe {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// A record of child-process samples of `key`.
pub fn child_record(name: &str, key: &str, unit: &str, runs: &[serde_json::Value]) -> Record {
    Record::from_samples(name, unit, runs.iter().filter_map(|v| v[key].as_f64()).collect())
}
