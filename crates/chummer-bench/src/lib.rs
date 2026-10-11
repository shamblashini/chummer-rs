//! The performance suite's runner and its check (docs/testing.md,
//! "Performance tests").
//!
//! A benchmark is a name, a unit and samples. [`Runner`] times closures:
//! one warm-up run, then a number of samples (fewer in quick mode), and
//! keeps every sample. A [`Report`] is the JSON a run writes; the GUI
//! frame benchmarks (in chummer-gui's tests) write one too, with this
//! crate as a dev-dependency.
//!
//! [`check`] merges reports, compares each median with its budget in
//! `perf/budgets.toml` and with the previous run, and gives the table for
//! the terminal and the Markdown for the workflow summary.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Version of the JSON layout.
pub const SCHEMA: u32 = 1;

// ----- results -----

/// One benchmark's result. Times are in milliseconds; other units say so.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub unit: String,
    pub n: usize,
    pub median: f64,
    pub mean: f64,
    pub p95: f64,
    pub min: f64,
    pub max: f64,
    /// Median absolute deviation, the spread that ignores outliers.
    pub mad: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why it did not run (no xsltproc, not Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    // Filled in by the check:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
    /// What this run was compared with: the previous median, or, when the
    /// previous run was already flagged, the value that run was compared
    /// with (so a lasting slowdown is not taken as the new normal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    /// Change against `baseline`, in percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// "ok", "new", "warn", "slow" (over the fail threshold once), "fail".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl Record {
    pub fn from_samples(name: &str, unit: &str, samples: Vec<f64>) -> Record {
        let mut s = samples.clone();
        s.sort_by(f64::total_cmp);
        let n = s.len();
        let median = quantile(&s, 0.5);
        let mut dev: Vec<f64> = s.iter().map(|v| (v - median).abs()).collect();
        dev.sort_by(f64::total_cmp);
        Record {
            name: name.to_owned(),
            unit: unit.to_owned(),
            n,
            median,
            mean: if n == 0 { 0.0 } else { s.iter().sum::<f64>() / n as f64 },
            p95: quantile(&s, 0.95),
            min: s.first().copied().unwrap_or(0.0),
            max: s.last().copied().unwrap_or(0.0),
            mad: quantile(&dev, 0.5),
            samples,
            ..Default::default()
        }
    }

    pub fn skipped(name: &str, why: &str) -> Record {
        Record { name: name.to_owned(), unit: "ms".into(), skipped: Some(why.to_owned()), ..Default::default() }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Record {
        self.note = Some(note.into());
        self
    }
}

/// Linear interpolation between the closest ranks of sorted `s`.
fn quantile(s: &[f64], q: f64) -> f64 {
    match s.len() {
        0 => 0.0,
        1 => s[0],
        n => {
            let pos = q * (n - 1) as f64;
            let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
            s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
        }
    }
}

/// What a run writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    /// The commit measured (`GITHUB_SHA`, else `git rev-parse HEAD`).
    pub commit: String,
    /// Unix time of the run.
    pub created: u64,
    pub quick: bool,
    /// CPU model and core count, to tell runners apart.
    pub machine: String,
    pub benches: Vec<Record>,
}

impl Report {
    pub fn new(quick: bool) -> Report {
        Report { schema: SCHEMA, commit: commit(), created: now(), quick, machine: machine(), benches: Vec::new() }
    }

    pub fn read(path: &Path) -> anyhow::Result<Report> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Record> {
        self.benches.iter().find(|r| r.name == name)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn commit() -> String {
    if let Some(s) = std::env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty()) {
        return s;
    }
    std::process::Command::new("git").args(["rev-parse", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default()
}

fn machine() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|t| t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_owned())).unwrap_or_else(|| std::env::consts::ARCH.to_owned());
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    format!("{cpu}, {cores} threads, {}", std::env::consts::OS)
}

/// Peak resident memory of this process in MiB (Linux: VmHWM).
pub fn peak_rss_mib() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

// ----- the runner -----

/// Times benchmarks and collects their records.
pub struct Runner {
    pub quick: bool,
    /// Name prefixes to run (empty: all).
    pub only: Vec<String>,
    pub report: Report,
}

impl Runner {
    pub fn new(quick: bool, only: Vec<String>) -> Runner {
        Runner { quick, only, report: Report::new(quick) }
    }

    /// Whether a benchmark (or a group, by its prefix) is selected.
    pub fn wants(&self, name: &str) -> bool {
        self.only.is_empty() || self.only.iter().any(|o| name.starts_with(o.as_str()) || o.starts_with(name))
    }

    /// Samples for a benchmark that takes `full` in a full run.
    pub fn samples(&self, full: usize) -> usize {
        if self.quick { full.min(3) } else { full }
    }

    /// Times `f` after one warm-up call.
    pub fn bench(&mut self, name: &str, full: usize, mut f: impl FnMut()) {
        self.bench_setup(name, full, || (), |()| f());
    }

    /// Times `f` on a fresh value from `setup` (not timed) each sample.
    pub fn bench_setup<S>(&mut self, name: &str, full: usize, mut setup: impl FnMut() -> S, mut f: impl FnMut(S)) {
        if !self.wants(name) {
            return;
        }
        f(setup());
        let n = self.samples(full);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let s = setup();
            let t = Instant::now();
            f(s);
            out.push(ms(t));
        }
        self.push(Record::from_samples(name, "ms", out));
    }

    /// As [`Runner::bench`], for a closure that does its own set-up and
    /// returns the milliseconds of the part that counts.
    pub fn bench_manual(&mut self, name: &str, full: usize, mut f: impl FnMut() -> f64) -> Option<&Record> {
        if !self.wants(name) {
            return None;
        }
        f();
        let out: Vec<f64> = (0..self.samples(full)).map(|_| f()).collect();
        self.push(Record::from_samples(name, "ms", out));
        self.report.benches.last()
    }

    /// Adds a record measured some other way.
    pub fn push(&mut self, r: Record) {
        if !self.wants(&r.name) {
            return;
        }
        if let Some(why) = &r.skipped {
            eprintln!("  {:<44} skipped: {why}", r.name);
        } else {
            eprintln!("  {:<44} {:>10} {} (p95 {}, n={})", r.name, fmt(r.median), r.unit, fmt(r.p95), r.n);
        }
        self.report.benches.push(r);
    }
}

/// Milliseconds since `t`.
pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn fmt(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    }
}

fn fmt_unit(v: f64, unit: &str) -> String {
    format!("{} {}", fmt(v), unit)
}

// ----- budgets and the regression check -----

/// `perf/budgets.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Budgets {
    #[serde(default)]
    pub policy: Policy,
    /// Name, or a prefix ending in `*`, to the largest median allowed.
    /// The longest matching key wins.
    #[serde(default)]
    pub budgets: BTreeMap<String, f64>,
    /// Name or prefix to the smallest change (in the unit) that counts
    /// as a regression; below it a change is noise whatever its percent.
    #[serde(default)]
    pub min_delta: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Policy {
    /// Slower than the baseline by this many percent: a warning.
    pub warn_pct: f64,
    /// By this many percent in two runs in a row: a failure.
    pub fail_pct: f64,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy { warn_pct: 25.0, fail_pct: 50.0 }
    }
}

impl Budgets {
    pub fn read(path: &Path) -> anyhow::Result<Budgets> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    pub fn budget(&self, name: &str) -> Option<f64> {
        lookup(&self.budgets, name)
    }

    pub fn min_delta(&self, name: &str) -> f64 {
        lookup(&self.min_delta, name).unwrap_or(0.0)
    }
}

fn lookup(map: &BTreeMap<String, f64>, name: &str) -> Option<f64> {
    if let Some(v) = map.get(name) {
        return Some(*v);
    }
    map.iter().filter_map(|(k, v)| k.strip_suffix('*').filter(|p| name.starts_with(p)).map(|p| (p.len(), *v))).max_by_key(|(len, _)| *len).map(|(_, v)| v)
}

/// The outcome of [`check`].
#[derive(Debug, Clone)]
pub struct Checked {
    /// The merged report with budget, baseline, change and status filled in.
    pub report: Report,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

/// Merges `parts` (later ones replace earlier records of the same name),
/// checks them against `budgets` and the newest of `history` (earlier
/// runs' merged reports, any order).
pub fn check(parts: Vec<Report>, budgets: &Budgets, history: &[Report]) -> Checked {
    let mut report = Report::new(parts.iter().any(|p| p.quick));
    if let Some(c) = parts.iter().map(|p| p.commit.clone()).find(|c| !c.is_empty()) {
        report.commit = c;
    }
    for p in parts {
        for r in p.benches {
            report.benches.retain(|b| b.name != r.name);
            report.benches.push(r);
        }
    }
    // Newest first: each benchmark is compared with the newest run that
    // has it (a run of a subset leaves the others to older runs).
    let mut newest: Vec<&Report> = history.iter().collect();
    newest.sort_by_key(|h| std::cmp::Reverse(h.created));
    let previous = newest.first().copied();
    let (mut failures, mut warnings) = (Vec::new(), Vec::new());
    for r in &mut report.benches {
        if r.skipped.is_some() {
            r.status = "skipped".into();
            continue;
        }
        r.budget = budgets.budget(&r.name);
        let prev = newest.iter().find_map(|p| p.get(&r.name).filter(|p| p.skipped.is_none() && p.unit == r.unit && p.median > 0.0));
        let mut status = "ok";
        let mut reason = String::new();
        if let Some(p) = prev {
            // A flagged run hands on its baseline: a slowdown that stays is
            // compared with the time before it, not with itself.
            let flagged = matches!(p.status.as_str(), "warn" | "slow" | "fail");
            let base = if flagged { p.baseline.unwrap_or(p.median) } else { p.median };
            let pct = (r.median - base) / base * 100.0;
            r.baseline = Some(base);
            r.change_pct = Some(pct);
            let real = r.median - base > budgets.min_delta(&r.name);
            if real && pct >= budgets.policy.fail_pct {
                if matches!(p.status.as_str(), "slow" | "fail") {
                    status = "fail";
                    reason = format!("{:+.0} % against {} for the second run in a row", pct, fmt_unit(base, &r.unit));
                } else {
                    status = "slow";
                    reason = format!("{:+.0} % against {} (fails if the next run is as slow)", pct, fmt_unit(base, &r.unit));
                }
            } else if real && pct >= budgets.policy.warn_pct {
                status = "warn";
                reason = format!("{:+.0} % against {}", pct, fmt_unit(base, &r.unit));
            }
        } else if previous.is_some() {
            status = "new";
        }
        match r.budget {
            Some(b) if r.median > b => {
                status = "fail";
                reason = format!("median {} is over the budget of {}", fmt_unit(r.median, &r.unit), fmt_unit(b, &r.unit));
            }
            None => warnings.push(format!("{}: no budget in perf/budgets.toml", r.name)),
            _ => {}
        }
        match status {
            "fail" => failures.push(format!("{}: {reason}", r.name)),
            "warn" | "slow" => warnings.push(format!("{}: {reason}", r.name)),
            _ => {}
        }
        r.status = status.into();
        r.reason = reason;
    }
    Checked { report, failures, warnings }
}

fn icon(status: &str) -> &'static str {
    match status {
        "fail" => ":x:",
        "slow" => ":warning: slow",
        "warn" => ":warning:",
        "new" => "new",
        "skipped" => "skipped",
        _ => ":white_check_mark:",
    }
}

/// The last few medians of `name`, oldest first, then this run's.
fn trend(name: &str, history: &[Report], now: f64) -> String {
    let mut h: Vec<&Report> = history.iter().collect();
    h.sort_by_key(|r| r.created);
    let mut v: Vec<String> = h.iter().rev().take(5).rev().filter_map(|r| r.get(name)).filter(|r| r.skipped.is_none()).map(|r| fmt(r.median)).collect();
    if v.is_empty() {
        return String::new();
    }
    v.push(format!("**{}**", fmt(now)));
    v.join(" → ")
}

/// The workflow summary.
pub fn markdown(c: &Checked, history: &[Report]) -> String {
    let r = &c.report;
    let mut out = String::new();
    let verdict = if !c.failures.is_empty() { ":x: failed" } else if !c.warnings.is_empty() { ":warning: passed with warnings" } else { ":white_check_mark: passed" };
    out.push_str(&format!("## Performance: {verdict}\n\n"));
    let commit: String = r.commit.chars().take(10).collect();
    out.push_str(&format!("Commit `{commit}`{}, {}.\n\n", if r.quick { " (quick mode)" } else { "" }, r.machine));
    match history.iter().max_by_key(|h| h.created) {
        Some(p) => out.push_str(&format!("Compared with the run of commit `{}` ({} earlier runs in the trend).\n\n", p.commit.chars().take(10).collect::<String>(), history.len())),
        None => out.push_str("No previous run to compare with: only the budgets are checked. This run is the baseline for the next one.\n\n"),
    }
    if !c.failures.is_empty() {
        out.push_str("**Failures**\n\n");
        for f in &c.failures {
            out.push_str(&format!("- {f}\n"));
        }
        out.push('\n');
    }
    out.push_str("| Benchmark | Median | p95 | Budget | Previous | Change | Trend | |\n|---|--:|--:|--:|--:|--:|---|---|\n");
    for b in &r.benches {
        if let Some(why) = &b.skipped {
            out.push_str(&format!("| `{}` | | | | | | | skipped: {why} |\n", b.name));
            continue;
        }
        let budget = b.budget.map(|v| fmt_unit(v, &b.unit)).unwrap_or_else(|| "none".into());
        let prev = b.baseline.map(|v| fmt_unit(v, &b.unit)).unwrap_or_default();
        let change = b.change_pct.map(|p| format!("{p:+.1} %")).unwrap_or_default();
        out.push_str(&format!("| `{}` | {} | {} | {budget} | {prev} | {change} | {} | {} |\n", b.name, fmt_unit(b.median, &b.unit), fmt_unit(b.p95, &b.unit), trend(&b.name, history, b.median), icon(&b.status)));
    }
    let notes: Vec<String> = r.benches.iter().filter_map(|b| b.note.as_ref().map(|n| format!("- `{}`: {n}", b.name))).collect();
    if !notes.is_empty() {
        out.push_str("\n**Notes**\n\n");
        out.push_str(&notes.join("\n"));
        out.push('\n');
    }
    if !c.warnings.is_empty() {
        out.push_str("\n**Warnings**\n\n");
        for w in &c.warnings {
            out.push_str(&format!("- {w}\n"));
        }
    }
    out
}

/// The table for the terminal.
pub fn table(c: &Checked) -> String {
    let mut out = format!("{:<46} {:>10} {:>10} {:>10} {:>9}  {}\n", "benchmark", "median", "p95", "budget", "change", "status");
    for b in &c.report.benches {
        if let Some(why) = &b.skipped {
            out.push_str(&format!("{:<46} skipped: {why}\n", b.name));
            continue;
        }
        let budget = b.budget.map(fmt).unwrap_or_else(|| "-".into());
        let change = b.change_pct.map(|p| format!("{p:+.1}%")).unwrap_or_default();
        out.push_str(&format!("{:<46} {:>10} {:>10} {:>10} {:>9}  {} {}\n", b.name, fmt_unit(b.median, &b.unit), fmt(b.p95), budget, change, b.status, b.reason));
    }
    for f in &c.failures {
        out.push_str(&format!("FAIL {f}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(created: u64, v: &[(&str, f64, &str)]) -> Report {
        let mut r = Report { created, ..Default::default() };
        for (n, m, s) in v {
            let mut rec = Record::from_samples(n, "ms", vec![*m]);
            rec.status = (*s).into();
            r.benches.push(rec);
        }
        r
    }

    fn budgets(text: &str) -> Budgets {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn stats() {
        let r = Record::from_samples("x", "ms", vec![5.0, 1.0, 3.0, 2.0, 4.0]);
        assert_eq!((r.median, r.min, r.max, r.mean), (3.0, 1.0, 5.0, 3.0));
        assert!((r.p95 - 4.8).abs() < 1e-9);
        assert_eq!(r.mad, 1.0);
    }

    #[test]
    fn longest_prefix_wins() {
        let b = budgets("[budgets]\n\"gui.*\" = 50\n\"gui.frame.classic.*\" = 80\n\"core.compute\" = 10\n");
        assert_eq!(b.budget("gui.frame.classic.skills"), Some(80.0));
        assert_eq!(b.budget("gui.frame.workspace.skills"), Some(50.0));
        assert_eq!(b.budget("core.compute"), Some(10.0));
        assert_eq!(b.budget("core.compute.x"), None);
    }

    #[test]
    fn regression_policy() {
        let b = budgets("[budgets]\n\"*\" = 1000\n[min_delta]\n\"*\" = 5\n");
        // Small absolute changes never count.
        let c = check(vec![rep(2, &[("a", 3.0, "")])], &b, &[rep(1, &[("a", 1.0, "ok")])]);
        assert_eq!(c.report.benches[0].status, "ok");
        // +30 %: a warning.
        let c = check(vec![rep(2, &[("a", 130.0, "")])], &b, &[rep(1, &[("a", 100.0, "ok")])]);
        assert_eq!(c.report.benches[0].status, "warn");
        assert!(c.failures.is_empty());
        // +60 % once: slow, not failed; twice against the same baseline: failed.
        let c = check(vec![rep(2, &[("a", 160.0, "")])], &b, &[rep(1, &[("a", 100.0, "ok")])]);
        assert_eq!(c.report.benches[0].status, "slow");
        assert!(c.failures.is_empty());
        let prev = c.report.clone();
        let c = check(vec![rep(3, &[("a", 158.0, "")])], &b, std::slice::from_ref(&prev));
        assert_eq!(c.report.benches[0].status, "fail");
        assert_eq!(c.report.benches[0].baseline, Some(100.0));
        // A one-off spike: back to normal clears it.
        let c = check(vec![rep(3, &[("a", 101.0, "")])], &b, &[prev]);
        assert_eq!(c.report.benches[0].status, "ok");
        // A newer run of a subset leaves the others to the older run.
        let c = check(vec![rep(3, &[("a", 100.0, "")])], &b, &[rep(1, &[("a", 99.0, "ok")]), rep(2, &[("b", 5.0, "ok")])]);
        assert_eq!(c.report.benches[0].baseline, Some(99.0));
        // Over budget fails at once.
        let c = check(vec![rep(1, &[("a", 1001.0, "")])], &b, &[]);
        assert_eq!(c.failures.len(), 1);
    }
}
