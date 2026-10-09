//! Crash safety: crash logs, emergency saves, autosave to a recovery
//! folder, and the "closed unexpectedly" dialog on the next start.
//!
//! Everything lives in the state root ([`data_dir`], see
//! [`chummer_core::paths`]):
//!
//! * `sessions/<pid>` — written at start, removed at a clean exit. A
//!   marker whose process is gone is a session that did not exit
//!   cleanly (two windows at once each have their own).
//! * `recovery/<pid>/<view>.chum5` (+ `.ini`: name, original file, time)
//!   — copies of unsaved characters: every few minutes and when the
//!   window loses focus ([`crate::prefs::Prefs::autosave_minutes`]),
//!   and once more when the app panics. Never written over the user's
//!   file; removed when the character is saved or closed, and at a
//!   clean exit.
//! * `crashes/crash-<time>-<pid>.log` — the panic message, backtrace,
//!   version, OS and the last actions.
//!
//! The panic hook cannot reach the app, so the app keeps copies of the
//! unsaved characters in a shared store (refreshed at most every few
//! seconds) that the hook writes out. A panic in a frame is also caught
//! around [`crate::App::frame`]: the app then copies every unsaved
//! character fresh, writes them and exits.
//!
//! The recovery files are written on other threads ([`crate::bg`]).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chummer_core::character::Character;
use eframe::egui;

use crate::theme::{self, Layout};
use crate::view::CharacterView;
use crate::App;

/// Snapshots of changed characters are refreshed at most this often.
const SNAPSHOT_EVERY: Duration = Duration::from_secs(3);
/// Actions kept for the crash log.
const MAX_ACTIONS: usize = 40;

// ----- folders and time stamps -----

/// The state root ([`chummer_core::paths::state_root`]):
/// `~/.local/share/chummer-rs`, `~/Library/Application Support/chummer-rs`,
/// `%LOCALAPPDATA%\chummer-rs`.
pub fn data_dir() -> Option<PathBuf> {
    chummer_core::paths::state_root()
}

pub fn crashes_dir(root: &Path) -> PathBuf {
    root.join("crashes")
}

pub fn recovery_root(root: &Path) -> PathBuf {
    root.join("recovery")
}

fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

fn recovery_dir(root: &Path, pid: u32) -> PathBuf {
    recovery_root(root).join(pid.to_string())
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's
/// `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn parts(t: SystemTime) -> (i64, u32, u32, u64, u64, u64, u32) {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (y, m, day) = civil((secs / 86_400) as i64);
    let rem = secs % 86_400;
    (y, m, day, rem / 3600, rem / 60 % 60, rem % 60, d.subsec_millis())
}

/// A sortable UTC time stamp for file names: `20261008-140322-123`.
pub fn stamp(t: SystemTime) -> String {
    let (y, m, d, h, min, s, ms) = parts(t);
    format!("{y:04}{m:02}{d:02}-{h:02}{min:02}{s:02}-{ms:03}")
}

/// `2026-10-08 14:03:22 UTC`.
pub fn readable_time(t: SystemTime) -> String {
    let (y, m, d, h, min, s, _) = parts(t);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02} UTC")
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ----- the session -----

struct Session {
    root: PathBuf,
    pid: u32,
    started: Instant,
}

static SESSION: OnceLock<Session> = OnceLock::new();

/// Start crash safety for this process (from `main`, not in tests):
/// the session marker and the panic hook.
pub fn start() {
    let Some(root) = data_dir() else { return };
    let pid = std::process::id();
    let dir = sessions_dir(&root);
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(pid.to_string()), format!("started={}\nversion={}\n", unix(SystemTime::now()), env!("CARGO_PKG_VERSION")))) {
        eprintln!("chummer-rs: crash recovery is off: {e}");
        return;
    }
    if SESSION.set(Session { root, pid, started: Instant::now() }).is_ok() {
        install_panic_hook();
    }
}

fn session() -> Option<&'static Session> {
    SESSION.get()
}

/// A clean exit: the marker and this session's recovery files go.
fn end_session() {
    let Some(s) = session() else { return };
    let _ = std::fs::remove_dir_all(recovery_dir(&s.root, s.pid));
    let _ = std::fs::remove_file(sessions_dir(&s.root).join(s.pid.to_string()));
}

/// Whether process `pid` is a running chummer-rs (Linux: `/proc`;
/// elsewhere every other session counts as gone).
pub fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if cfg!(target_os = "linux") {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim().starts_with("chummer"))
    } else {
        false
    }
}

// ----- last actions -----

fn actions() -> &'static Mutex<VecDeque<(Instant, String)>> {
    static A: OnceLock<Mutex<VecDeque<(Instant, String)>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// Remember an action for the crash log.
pub fn note(what: impl Into<String>) {
    if let Ok(mut a) = actions().lock() {
        if a.len() == MAX_ACTIONS {
            a.pop_front();
        }
        a.push_back((Instant::now(), what.into()));
    }
}

fn recent_actions() -> Vec<String> {
    let start = session().map(|s| s.started);
    let Ok(a) = actions().try_lock() else { return vec!["(busy)".into()] };
    a.iter().map(|(t, w)| match start {
        Some(s) => format!("[+{:.1}s] {w}", t.duration_since(s).as_secs_f64()),
        None => w.clone(),
    }).collect()
}

// ----- crash logs -----

/// What a crash log says.
#[derive(Debug, Clone)]
pub struct CrashInfo {
    pub message: String,
    pub location: String,
    pub thread: String,
    pub backtrace: String,
    pub actions: Vec<String>,
    pub time: SystemTime,
}

/// The operating system, for reports.
fn os_name() -> String {
    let pretty = std::fs::read_to_string("/etc/os-release").ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_owned())));
    match pretty {
        Some(p) => format!("{} {} ({p})", std::env::consts::OS, std::env::consts::ARCH),
        None => format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

/// The text of a crash log.
pub fn crash_report(c: &CrashInfo) -> String {
    let mut s = String::new();
    s.push_str("chummer-rs crash report\n");
    s.push_str(&format!("Version: {}\n", env!("CARGO_PKG_VERSION")));
    s.push_str(&format!("Time: {}\n", readable_time(c.time)));
    s.push_str(&format!("OS: {}\n", os_name()));
    s.push_str(&format!("Thread: {}\n", c.thread));
    s.push_str(&format!("Panic: {}\n", c.message));
    s.push_str(&format!("Location: {}\n", c.location));
    s.push_str("\nRecent actions:\n");
    if c.actions.is_empty() {
        s.push_str("  (none recorded)\n");
    }
    for a in &c.actions {
        s.push_str(&format!("  {a}\n"));
    }
    s.push_str("\nBacktrace:\n");
    s.push_str(&c.backtrace);
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Write a crash log into `dir`. Returns its path.
pub fn write_crash_log(dir: &Path, pid: u32, c: &CrashInfo) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("crash-{}-{pid}.log", stamp(c.time)));
    std::fs::write(&path, crash_report(c))?;
    Ok(path)
}

/// The crash log written by this process, for adding the emergency saves.
static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

fn append_to_log(text: &str) {
    let path = LOG.try_lock().ok().and_then(|l| l.clone());
    if let Some(p) = path {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(p) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

/// Only a panic on the main (UI) thread ends the app; others (background
/// jobs, network, scans) are caught or end just their thread.
fn ends_the_app(thread: &str) -> bool {
    thread == "main"
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        if !ends_the_app(&thread) {
            return;
        }
        let Some(s) = session() else { return };
        let message = info.payload().downcast_ref::<&str>().map(|m| (*m).to_owned()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        let c = CrashInfo {
            message,
            location: info.location().map(|l| l.to_string()).unwrap_or_default(),
            thread,
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
            actions: recent_actions(),
            time: SystemTime::now(),
        };
        match write_crash_log(&crashes_dir(&s.root), s.pid, &c) {
            Ok(p) => {
                eprintln!("chummer-rs: crash log written to {}", p.display());
                if let Ok(mut l) = LOG.try_lock() {
                    *l = Some(p);
                }
            }
            Err(e) => eprintln!("chummer-rs: could not write the crash log: {e}"),
        }
        // The copies the app kept; a panic in a frame writes fresher
        // ones once it is caught (`App::safety_crashed`).
        emergency_save();
    }));
}

// ----- recovery files -----

/// A character copy in a recovery folder.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveredDoc {
    /// The `.chum5` copy.
    pub file: PathBuf,
    pub name: String,
    /// The file it was opened from; `None`: never saved.
    pub original: Option<PathBuf>,
    pub saved: SystemTime,
}

/// Write a copy of `ch` as `<key>.chum5` (and its `.ini`) into `dir`.
pub fn write_recovery(dir: &Path, key: u64, name: &str, original: Option<&Path>, ch: &Character) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let file = dir.join(format!("{key}.chum5"));
    chummer_core::chum5lz::write_text(&file, &ch.to_xml_string())?;
    let mut ini = theme::config_set("", "name", &name.replace('\n', " "));
    ini = theme::config_set(&ini, "original", &original.map(|p| p.display().to_string()).unwrap_or_default());
    ini = theme::config_set(&ini, "saved", &unix(SystemTime::now()).to_string());
    std::fs::write(dir.join(format!("{key}.ini")), ini)?;
    Ok(file)
}

/// Remove `<key>`'s copy from `dir`.
pub fn remove_recovery(dir: &Path, key: u64) {
    let _ = std::fs::remove_file(dir.join(format!("{key}.chum5")));
    let _ = std::fs::remove_file(dir.join(format!("{key}.ini")));
}

/// The copies in a recovery folder, oldest first.
pub fn read_recovery(dir: &Path) -> Vec<RecoveredDoc> {
    let mut v: Vec<RecoveredDoc> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "chum5"))
        .map(|file| {
            let ini = std::fs::read_to_string(file.with_extension("ini")).unwrap_or_default();
            let get = |k: &str| theme::config_get(&ini, k).filter(|v| !v.is_empty());
            RecoveredDoc {
                name: get("name").unwrap_or_else(|| file.file_stem().unwrap_or_default().to_string_lossy().into_owned()),
                original: get("original").map(PathBuf::from),
                saved: get("saved").and_then(|s| s.parse().ok()).map_or(SystemTime::UNIX_EPOCH, |s| UNIX_EPOCH + Duration::from_secs(s)),
                file,
            }
        })
        .collect();
    v.sort_by_key(|d| d.saved);
    v
}

/// A session that did not exit cleanly.
#[derive(Debug, Clone)]
pub struct Crashed {
    pub pid: u32,
    pub docs: Vec<RecoveredDoc>,
    /// Its crash log and the log's text, when it panicked.
    pub crash_log: Option<(PathBuf, String)>,
}

/// The sessions under `root` that did not exit cleanly, other than this
/// one: markers (or recovery folders) whose process is gone.
pub fn find_crashed(root: &Path, alive: impl Fn(u32) -> bool) -> Vec<Crashed> {
    let pids = |dir: PathBuf| -> Vec<(u32, Option<SystemTime>)> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| Some((e.file_name().to_str()?.parse().ok()?, e.metadata().ok().and_then(|m| m.modified().ok())))).collect()
    };
    let mut found: BTreeMap<u32, Option<SystemTime>> = BTreeMap::new();
    for (pid, t) in pids(sessions_dir(root)) {
        found.insert(pid, t);
    }
    for (pid, _) in pids(recovery_root(root)) {
        found.entry(pid).or_insert(None);
    }
    let logs: Vec<PathBuf> = std::fs::read_dir(crashes_dir(root)).into_iter().flatten().flatten().map(|e| e.path()).collect();
    found
        .into_iter()
        .filter(|(pid, _)| !alive(*pid))
        .map(|(pid, started)| {
            let suffix = format!("-{pid}.log");
            let mut mine: Vec<&PathBuf> = logs
                .iter()
                .filter(|p| p.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with("crash-") && f.ends_with(&suffix)))
                // A log from an earlier process with the same id is older
                // than the marker.
                .filter(|p| started.is_none_or(|s| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|m| m >= s)))
                .collect();
            mine.sort();
            let crash_log = mine.last().and_then(|p| std::fs::read_to_string(p).ok().map(|t| ((*p).clone(), t)));
            Crashed { pid, docs: read_recovery(&recovery_dir(root, pid)), crash_log }
        })
        .collect()
}

/// Forget a crashed session: its marker and recovery files.
pub fn resolve(root: &Path, pid: u32) {
    let _ = std::fs::remove_dir_all(recovery_dir(root, pid));
    let _ = std::fs::remove_file(sessions_dir(root).join(pid.to_string()));
}

// ----- the shared store of unsaved characters -----

struct Snap {
    name: String,
    original: Option<PathBuf>,
    revision: u64,
    ch: Character,
}

static SNAPS: Mutex<BTreeMap<u64, Snap>> = Mutex::new(BTreeMap::new());

/// Write every stored copy to this session's recovery folder (from the
/// panic hook: never waits for the store). Returns what was written.
fn emergency_save() -> Vec<String> {
    let Some(s) = session() else { return Vec::new() };
    // A panic while the store was locked poisons it; the copies are still good.
    let snaps = match SNAPS.try_lock() {
        Ok(s) => s,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Vec::new(),
    };
    let dir = recovery_dir(&s.root, s.pid);
    let mut out = Vec::new();
    for (key, snap) in snaps.iter() {
        match write_recovery(&dir, *key, &snap.name, snap.original.as_deref(), &snap.ch) {
            Ok(p) => out.push(format!("  {} -> {}\n", snap.name, p.display())),
            Err(e) => out.push(format!("  {}: FAILED ({e})\n", snap.name)),
        }
    }
    out
}

/// Whether a view's character is kept for recovery: a local character
/// with unsaved changes (campaign members are saved with the campaign,
/// online ones by the GM).
fn eligible(v: &CharacterView) -> bool {
    v.ch().dirty && v.campaign_member.is_none() && !v.doc().is_online()
}

fn snap_of(v: &CharacterView) -> Snap {
    Snap { name: v.ch().display_name(), original: v.path(), revision: v.doc().revision(), ch: v.ch().clone() }
}

// ----- the app's side -----

const AUTOSAVE: &str = "autosave:";
const SCAN_JOB: &str = "ux:crashed-scan";
const RECOVER_JOB: &str = "ux:recover";
const DISCARD_JOB: &str = "ux:recover-discard";

/// Recovery state kept by the app.
#[derive(Default)]
pub struct Safety {
    last_snapshot: Option<Instant>,
    last_autosave: Option<Instant>,
    focused: bool,
    /// Copies written this session: view → revision.
    autosaved: HashMap<u64, u64>,
    /// Copies to delete (their view was saved or closed).
    remove: HashSet<u64>,
    last_status: Option<String>,
    last_undo: Option<String>,
    /// Sessions to offer, and which documents are ticked.
    pub crashed: Vec<Crashed>,
    ticks: Vec<Vec<bool>>,
}

impl App {
    /// At start: look for sessions that did not exit cleanly (on another
    /// thread).
    pub(crate) fn safety_start(&mut self) {
        let Some(s) = session() else { return };
        let root = s.root.clone();
        crate::bg::spawn(&self.ctx, SCAN_JOB, self.lang.tr("Looking for recovered files…"), move || find_crashed(&root, pid_alive));
    }

    /// Each frame: keep copies of unsaved characters, autosave them when
    /// due or when the window loses focus, and show the recovery dialog.
    pub(crate) fn safety_frame(&mut self, ctx: &egui::Context) {
        if let Some(found) = crate::bg::take::<Vec<Crashed>>(SCAN_JOB) {
            // Nothing to offer (killed, or the copies were saved): forget it.
            let (found, empty): (Vec<Crashed>, Vec<Crashed>) = found.into_iter().partition(|c| !c.docs.is_empty() || c.crash_log.is_some());
            if let (false, Some(root)) = (empty.is_empty(), session().map(|s| s.root.clone())) {
                crate::bg::spawn(ctx, DISCARD_JOB, self.lang.tr("Discarding recovered characters…"), move || {
                    for c in empty {
                        resolve(&root, c.pid);
                    }
                });
            }
            self.ux.safety.ticks = found.iter().map(|c| vec![true; c.docs.len()]).collect();
            self.ux.safety.crashed = found;
        }
        self.recovered_arrived();
        for id in crate::bg::finished_with(DISCARD_JOB) {
            let _ = crate::bg::take::<()>(&id);
        }
        if !self.ux.safety.crashed.is_empty() {
            self.recovery_dialog(ctx);
        }
        let Some(s) = session() else { return };
        // Debug builds: `CHUMMER_CRASH_AFTER=<seconds>` panics in a frame,
        // to try the recovery.
        if cfg!(debug_assertions) && std::env::var("CHUMMER_CRASH_AFTER").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|after| s.started.elapsed().as_secs() >= after) {
            panic!("crash test (CHUMMER_CRASH_AFTER)");
        }
        self.note_actions();
        let now = Instant::now();
        let dir = recovery_dir(&s.root, s.pid);
        // Views no longer kept: drop their copies.
        let keep: HashSet<u64> = self.views.iter().filter(|v| eligible(v)).map(|v| v.ws_id()).collect();
        {
            let mut snaps = SNAPS.lock().unwrap_or_else(|e| e.into_inner());
            snaps.retain(|k, _| keep.contains(k));
        }
        let gone: Vec<u64> = self.ux.safety.autosaved.keys().filter(|k| !keep.contains(k)).copied().collect();
        for k in gone {
            self.ux.safety.autosaved.remove(&k);
            self.ux.safety.remove.insert(k);
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(i.focused));
        let lost_focus = std::mem::replace(&mut self.ux.safety.focused, focused) && !focused;
        let last = *self.ux.safety.last_autosave.get_or_insert(now);
        let due = self.ux.prefs.autosave_every().is_some_and(|every| now.duration_since(last) >= every);
        let refresh = due || lost_focus || self.ux.safety.last_snapshot.is_none_or(|t| now.duration_since(t) >= SNAPSHOT_EVERY);
        if refresh && !keep.is_empty() {
            self.refresh_snapshots(false);
            self.ux.safety.last_snapshot = Some(now);
        }
        if due || lost_focus {
            self.ux.safety.last_autosave = Some(now);
            let snaps = SNAPS.lock().unwrap_or_else(|e| e.into_inner());
            let todo: Vec<(u64, String, Option<PathBuf>, u64, Character)> = snaps
                .iter()
                .filter(|(k, s)| self.ux.safety.autosaved.get(k) != Some(&s.revision))
                .map(|(k, s)| (*k, s.name.clone(), s.original.clone(), s.revision, s.ch.clone()))
                .collect();
            drop(snaps);
            for (key, name, original, revision, ch) in todo {
                let d = dir.clone();
                let started = crate::bg::spawn(ctx, format!("{AUTOSAVE}{key}"), self.lang.tr("Autosaving…"), move || -> Result<Option<(u64, u64)>, String> {
                    write_recovery(&d, key, &name, original.as_deref(), &ch).map(|_| Some((key, revision))).map_err(|e| e.to_string())
                });
                if started {
                    self.ux.safety.autosaved.insert(key, revision);
                }
            }
        }
        // Deletions wait for a write of the same copy to finish.
        for key in std::mem::take(&mut self.ux.safety.remove) {
            let d = dir.clone();
            if !crate::bg::spawn(ctx, format!("{AUTOSAVE}{key}"), self.lang.tr("Autosaving…"), move || -> Result<Option<(u64, u64)>, String> {
                remove_recovery(&d, key);
                Ok(None)
            }) {
                self.ux.safety.remove.insert(key);
            }
        }
        for id in crate::bg::finished_with(AUTOSAVE) {
            if let Some(Err(e)) = crate::bg::take::<Result<Option<(u64, u64)>, String>>(&id) {
                if let Some(key) = id.strip_prefix(AUTOSAVE).and_then(|k| k.parse::<u64>().ok()) {
                    self.ux.safety.autosaved.remove(&key);
                }
                self.status = Some((self.lang.tr_fmt("Could not autosave: {0}", &[&e]), true));
            }
        }
    }

    /// Copy the unsaved characters into the store (`all`: even those
    /// whose copy is current).
    fn refresh_snapshots(&self, all: bool) {
        let mut snaps = SNAPS.lock().unwrap_or_else(|e| e.into_inner());
        for v in self.views.iter().filter(|v| eligible(v)) {
            if all || snaps.get(&v.ws_id()).is_none_or(|s| s.revision != v.doc().revision()) {
                snaps.insert(v.ws_id(), snap_of(v));
            }
        }
    }

    /// Status messages and changes, for the crash log.
    fn note_actions(&mut self) {
        let status = self.status.as_ref().map(|(m, _)| m.clone());
        if status.is_some() && status != self.ux.safety.last_status {
            note(status.clone().unwrap_or_default());
        }
        self.ux.safety.last_status = status;
        let undo = self.current().and_then(|i| self.views[i].doc().undo_label().map(|l| format!("{}: {l}", self.views[i].ch().display_name())));
        if undo.is_some() && undo != self.ux.safety.last_undo {
            note(undo.clone().unwrap_or_default());
        }
        self.ux.safety.last_undo = undo;
    }

    /// A clean exit: wait for copies being written, then remove them.
    pub(crate) fn safety_exit(&mut self) {
        crate::bg::wait(AUTOSAVE, Duration::from_secs(10));
        end_session();
    }

    /// A frame panicked: write fresh copies of every unsaved character,
    /// note them in the crash log and exit. The next start offers them.
    /// Without crash safety (tests) the panic goes on.
    pub(crate) fn safety_crashed(&mut self, panic: Box<dyn std::any::Any + Send>) -> ! {
        if session().is_none() {
            std::panic::resume_unwind(panic);
        }
        let fresh = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.refresh_snapshots(true))).is_ok();
        let saved = emergency_save();
        let mut text = String::from("\nEmergency saves");
        if !fresh {
            text.push_str(" (copies from a few seconds before the crash)");
        }
        text.push_str(":\n");
        if saved.is_empty() {
            text.push_str("  (no unsaved characters)\n");
        }
        for s in saved {
            text.push_str(&s);
        }
        append_to_log(&text);
        std::process::exit(101);
    }

    /// Open a recovered or backed-up character as a modified document
    /// of `original` (saving writes there; `None`: Save asks for a file).
    pub(crate) fn open_recovered(&mut self, mut ch: Character, original: Option<PathBuf>) {
        ch.file = original;
        ch.dirty = true;
        let v = CharacterView::new(ch, &self.engine);
        self.views.push(v);
        self.active = self.views.len() - 1;
        self.home = None;
    }

    fn recovered_arrived(&mut self) {
        type Loaded = Vec<(Result<Character, String>, Option<PathBuf>)>;
        let Some(docs) = crate::bg::take::<Loaded>(RECOVER_JOB) else { return };
        let mut n = 0;
        for (r, original) in docs {
            match r {
                Ok(ch) => {
                    self.open_recovered(ch, original);
                    n += 1;
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        if n > 0 {
            self.status = Some((self.lang.tr_fmt("Reopened {0} recovered characters; save them to keep the changes", &[&n]), false));
        }
    }

    /// "chummer-rs closed unexpectedly": reopen or discard the copies.
    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let workspace = theme::current(ctx).kind.layout() == Layout::Workspace;
        let crashed = self.ux.safety.crashed.clone();
        let log = crashed.iter().find_map(|c| c.crash_log.clone());
        let any_docs = crashed.iter().any(|c| !c.docs.is_empty());
        let now = SystemTime::now();
        #[derive(PartialEq)]
        enum Choice {
            Reopen,
            Discard,
            Later,
        }
        let mut choice = None;
        let mut ticks = std::mem::take(&mut self.ux.safety.ticks);
        let lang = &self.lang;
        egui::Modal::new(egui::Id::new("crash_recovery")).show(ctx, |ui| {
            ui.set_width(520.0);
            if workspace {
                let ws = theme::ws(ui);
                ui.label(egui::RichText::new(lang.tr("chummer-rs closed unexpectedly")).font(crate::workspace::widgets::bold(17.0)).color(ws.text));
            } else {
                ui.heading(lang.tr("chummer-rs closed unexpectedly"));
            }
            ui.add_space(4.0);
            ui.label(if log.is_some() { lang.tr("It ran into an error and had to close. A crash log was written.") } else { lang.tr("It did not shut down normally last time.") });
            ui.add_space(8.0);
            if any_docs {
                ui.label(crate::theme::strong(ui, lang.tr("Recovered characters")));
                ui.weak(lang.tr("Copies of unsaved changes. Reopened characters stay unsaved until you save them."));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (c, t) in crashed.iter().zip(ticks.iter_mut()) {
                        for (d, on) in c.docs.iter().zip(t.iter_mut()) {
                            ui.horizontal(|ui| {
                                if workspace {
                                    crate::workspace::widgets::check(ui, on, &d.name);
                                } else {
                                    ui.checkbox(on, &d.name);
                                }
                                ui.weak(crate::backups::age(d.saved, now, lang));
                            });
                            let where_ = d.original.as_ref().map_or_else(|| lang.tr("never saved"), |p| p.display().to_string());
                            ui.indent(("recovered", &d.file), |ui| {
                                ui.weak(where_);
                            });
                        }
                    }
                });
            } else {
                ui.weak(lang.tr("No unsaved characters were open."));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let primary = if any_docs { lang.tr("Reopen selected") } else { lang.tr("OK") };
                let reopen = if workspace { crate::workspace::widgets::button(ui, None, &primary, crate::workspace::widgets::Look::Primary, 28.0) } else { ui.add(crate::theme::primary_button(ui, primary)) };
                if reopen.clicked() {
                    choice = Some(Choice::Reopen);
                }
                if any_docs {
                    let discard = if workspace { crate::workspace::widgets::button(ui, None, &lang.tr("Discard"), crate::workspace::widgets::Look::Secondary, 28.0) } else { ui.button(lang.tr("Discard")) };
                    if discard.clicked() {
                        choice = Some(Choice::Discard);
                    }
                    let later = if workspace { crate::workspace::widgets::button(ui, None, &lang.tr("Decide later"), crate::workspace::widgets::Look::Ghost, 28.0) } else { ui.button(lang.tr("Decide later")) };
                    if later.on_hover_text(lang.tr("Keep the copies and ask again next time")).clicked() {
                        choice = Some(Choice::Later);
                    }
                }
            });
            if let Some((path, text)) = &log {
                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.link(lang.tr("Open the crash log folder")).on_hover_text(path.display().to_string()).clicked() {
                        if let Some(dir) = path.parent() {
                            let _ = crate::prefs::open_folder(dir);
                        }
                    }
                    if ui.link(lang.tr("Copy the report")).on_hover_text(lang.tr("Copies the crash log, to paste into a bug report")).clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                });
            }
        });
        self.ux.safety.ticks = ticks;
        let Some(choice) = choice else { return };
        let Some(root) = session().map(|s| s.root.clone()) else {
            self.ux.safety.crashed.clear();
            return;
        };
        let pids: Vec<u32> = crashed.iter().map(|c| c.pid).collect();
        match choice {
            Choice::Reopen => {
                let picked: Vec<RecoveredDoc> = crashed.iter().zip(&self.ux.safety.ticks).flat_map(|(c, t)| c.docs.iter().zip(t).filter(|(_, on)| **on).map(|(d, _)| d.clone())).collect();
                // Read the copies, then forget the sessions.
                crate::bg::spawn(ctx, RECOVER_JOB, self.lang.tr("Opening recovered characters…"), move || {
                    let out: Vec<(Result<Character, String>, Option<PathBuf>)> = picked.into_iter().map(|d| (Character::load(&d.file).map_err(|e| e.to_string()), d.original)).collect();
                    for pid in pids {
                        resolve(&root, pid);
                    }
                    out
                });
            }
            Choice::Discard => {
                crate::bg::spawn(ctx, DISCARD_JOB, self.lang.tr("Discarding recovered characters…"), move || {
                    for pid in pids {
                        resolve(&root, pid);
                    }
                });
            }
            Choice::Later => {}
        }
        self.ux.safety.crashed.clear();
        self.ux.safety.ticks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chummer-safety-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture() -> Character {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin.chum5");
        Character::load(&p).expect("fixture")
    }

    #[test]
    fn time_stamps() {
        let t = UNIX_EPOCH + Duration::from_millis(1_791_468_202_123);
        assert_eq!(readable_time(t), "2026-10-08 14:03:22 UTC");
        assert_eq!(stamp(t), "20261008-140322-123");
        assert_eq!(stamp(UNIX_EPOCH), "19700101-000000-000");
        // Leap day.
        assert_eq!(readable_time(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn crash_log_is_written() {
        let d = scratch("log");
        note("Opened Munin.chum5");
        let c = CrashInfo {
            message: "index out of bounds".into(),
            location: "src/view.rs:10:5".into(),
            thread: "main".into(),
            backtrace: "   0: chummer_rs::main".into(),
            actions: vec!["[+1.0s] Opened Munin.chum5".into()],
            time: UNIX_EPOCH + Duration::from_secs(1_791_468_202),
        };
        let p = write_crash_log(&crashes_dir(&d), 4242, &c).unwrap();
        assert_eq!(p.file_name().unwrap(), "crash-20261008-140322-000-4242.log");
        let text = std::fs::read_to_string(&p).unwrap();
        for want in ["chummer-rs crash report", &format!("Version: {}", env!("CARGO_PKG_VERSION")), "OS: ", "Thread: main", "Panic: index out of bounds", "Location: src/view.rs:10:5", "Opened Munin.chum5", "Backtrace:\n   0: chummer_rs::main"] {
            assert!(text.contains(want), "{want:?} missing from\n{text}");
        }
        assert!(recent_actions().iter().any(|a| a.contains("Opened Munin.chum5")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recovery_round_trip() {
        let d = scratch("recovery");
        let ch = fixture();
        let original = PathBuf::from("/home/someone/Runners/Munin.chum5");
        let dir = recovery_dir(&d, 777);
        write_recovery(&dir, 3, "Munin", Some(&original), &ch).unwrap();
        write_recovery(&dir, 9, "New runner", None, &ch).unwrap();
        // A session that crashed (marker, process gone) and a running one.
        std::fs::create_dir_all(sessions_dir(&d)).unwrap();
        std::fs::write(sessions_dir(&d).join("777"), "started=0\n").unwrap();
        std::fs::write(sessions_dir(&d).join("778"), "started=0\n").unwrap();
        write_recovery(&recovery_dir(&d, 778), 1, "Busy", None, &ch).unwrap();
        let c = CrashInfo { message: "boom".into(), location: String::new(), thread: "main".into(), backtrace: String::new(), actions: Vec::new(), time: SystemTime::now() };
        write_crash_log(&crashes_dir(&d), 777, &c).unwrap();
        let found = find_crashed(&d, |pid| pid == 778);
        assert_eq!(found.len(), 1, "{found:?}");
        let s = &found[0];
        assert_eq!(s.pid, 777);
        assert!(s.crash_log.as_ref().is_some_and(|(_, t)| t.contains("Panic: boom")));
        let names: Vec<&str> = s.docs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Munin") && names.contains(&"New runner"));
        let munin = s.docs.iter().find(|d| d.name == "Munin").unwrap();
        assert_eq!(munin.original.as_deref(), Some(original.as_path()));
        assert!(s.docs.iter().find(|d| d.name == "New runner").unwrap().original.is_none());
        // The copy loads back as the same character.
        let back = Character::load(&munin.file).unwrap();
        assert_eq!(back.to_xml_string(), ch.to_xml_string());
        // Removing one copy, then resolving the session.
        remove_recovery(&dir, 9);
        assert_eq!(read_recovery(&dir).len(), 1);
        resolve(&d, 777);
        assert!(find_crashed(&d, |pid| pid == 778).is_empty());
        // A crash with no marker left but copies on disk is still offered.
        write_recovery(&recovery_dir(&d, 900), 1, "Orphan", None, &ch).unwrap();
        assert_eq!(find_crashed(&d, |_| false).iter().map(|c| c.pid).collect::<Vec<_>>(), vec![778, 900]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_crashing_threads_are_logged() {
        assert!(!ends_the_app("bg save:3"));
        assert!(!ends_the_app("chummer-net"));
        assert!(ends_the_app("main"));
        assert!(pid_alive(std::process::id()));
    }
}
