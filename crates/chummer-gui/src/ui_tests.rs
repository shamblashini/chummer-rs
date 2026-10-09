//! Headless UI tests: the whole `App` driven through `egui::Context::run`
//! with synthesized input, no window or display. They look for panics
//! (index, unwrap, egui's debug asserts) and check a few interactions:
//!
//! * the render matrix: every page of both layouts (Classic tabs, Street
//!   Gear sub-tabs and side panel tabs; Workspace sections including At
//!   the table and History) for the fixtures, at two sizes in both
//!   themes, then every item in the item inspector;
//! * Home, the Master Index and a GM screen with members, the tool
//!   windows and dialogs, the New Character wizard for each preset;
//! * the command palette, the inline catalog and the selection dialog,
//!   undo and redo, tab strip clicks, closing a changed document,
//!   switching documents;
//! * seeded random clicks on every page ("monkey"), and windows far
//!   below the minimum size.
//!
//! A panic is caught per (fixture, layout, page) and every one is listed
//! before the test fails. Known GUI bugs are skipped where they would
//! fail a sweep and have an `#[ignore = "GUI BUG: ..."]` repro test.
//!
//! By default a spread of fixtures runs (about a minute in debug). Set
//! `CHUMMER_UI_FULL=1` for every fixture, both themes at each size plus
//! the app's minimum size, more random clicks (also at the narrow size)
//! and every wizard preset: about 15 minutes.
//!
//! Config and data go to a scratch directory (XDG_CONFIG_HOME,
//! XDG_DATA_HOME) and the fixtures are opened from copies, so nothing the
//! app saves touches the user's files or the repository. DISPLAY,
//! WAYLAND_DISPLAY and the session bus are cleared, so a file dialog
//! cannot open on the desktop.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chummer_core::engine::Engine;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

use crate::theme::{self, Layout, ThemeKind};
use crate::view::{Tab, TABS};
use crate::workspace::Section;
use crate::{App, Home};

// ----- environment -----

/// The scratch directory, set up once: config and data dirs, fixture
/// copies.
fn scratch() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let root = base.join("chummer-ui-tests");
        // Leftovers of earlier runs whose process is gone.
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let gone = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()).is_some_and(|pid| cfg!(target_os = "linux") && !Path::new(&format!("/proc/{pid}")).exists());
            if gone {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        let dir = root.join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["config", "data", "fixtures"] {
            std::fs::create_dir_all(dir.join(sub)).expect("scratch dir");
        }
        // Edition 2021: set_var is safe; std's env lock covers std readers.
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        // No display and no session bus: a file dialog (rfd: the portal,
        // then zenity) or xdg-open reached by a click fails at once
        // instead of showing up on the desktop.
        std::env::remove_var("DISPLAY");
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/chummer-ui-tests");
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests");
        for src in [core.join("fixtures"), core.join("chum5lz")] {
            for e in std::fs::read_dir(&src).into_iter().flatten().flatten() {
                let p = e.path();
                if chummer_core::chum5lz::is_character_file(&p) {
                    std::fs::copy(&p, dir.join("fixtures").join(e.file_name())).expect("copy fixture");
                }
            }
        }
        install_panic_hook();
        dir
    })
}

/// Every fixture copy, sorted by name.
fn all_fixtures() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(scratch().join("fixtures")).unwrap().flatten().map(|e| e.path()).collect();
    v.sort();
    assert!(v.len() >= 30, "expected the chummer-core fixtures, found {}", v.len());
    v
}

/// A copy of `path` for this thread only: background jobs are keyed by
/// file path across the process, so two tests opening the same file at
/// once would take each other's results.
fn own_copy(path: &Path) -> PathBuf {
    let tid = format!("{:?}", std::thread::current().id()).replace(['(', ')'], "");
    let dir = scratch().join(format!("own-{tid}"));
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(path.file_name().expect("a file"));
    if !p.exists() {
        std::fs::copy(path, &p).expect("copy fixture");
    }
    p
}

fn fixture(name: &str) -> PathBuf {
    let p = scratch().join("fixtures").join(name);
    assert!(p.exists(), "no fixture {name}");
    p
}

/// A spread of fixtures for the default run: creation and career,
/// magician, adept, technomancer, AI, critter, vehicles, compressed.
/// The two career characters (slow pages) land in different shards.
const DEFAULT_FIXTURES: &[&str] = &[
    "Munin_Career.chum5",
    "Munin.chum5",
    "Soma (Career).chum5",
    "Apex Predator.chum5",
    "Rez0luti0n2.0.chum5",
    "Spirit_Warden.chum5",
    "Mittens Chargen.chum5",
    "SCSi.chum5",
    "fixer-chummer.chum5lz",
];

fn full() -> bool {
    std::env::var("CHUMMER_UI_FULL").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn matrix_fixtures() -> Vec<PathBuf> {
    if full() {
        all_fixtures()
    } else {
        DEFAULT_FIXTURES.iter().map(|n| fixture(n)).collect()
    }
}

// ----- panic capture -----

thread_local! {
    /// While set, panics on this thread are recorded here instead of
    /// printed: "message at file:line" plus the chummer frames.
    static CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let capturing = CAPTURE.with(|c| c.borrow().is_some());
        if !capturing {
            previous(info);
            return;
        }
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "<non-string panic>".into());
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        // The app's own frames, innermost first, to name the caller of a
        // panic inside std or egui.
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        if std::env::var_os("UI_TESTS_BT").is_some() {
            eprintln!("{bt}");
        }
        let frames: Vec<String> = bt
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("at ") && (l.starts_with("at ./") || l.contains("chummer-gui/src")) && !l.contains("ui_tests.rs"))
            .take(4)
            .map(|l| l.trim_start_matches("at ").to_owned())
            .collect();
        let text = if frames.is_empty() { format!("{msg} at {loc}") } else { format!("{msg} at {loc} (via {})", frames.join(" <- ")) };
        CAPTURE.with(|c| {
            if let Some(v) = c.borrow_mut().as_mut() {
                v.push(text);
            }
        });
    }));
}

/// Runs `f`; on a panic returns the recorded message.
fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let r = panic::catch_unwind(AssertUnwindSafe(f));
    let msgs = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
    r.map_err(|_| msgs.first().cloned().unwrap_or_else(|| "panic (no message)".into()))
}

// ----- the harness -----

const WIDE: Vec2 = Vec2::new(1600.0, 1000.0);
const NARROW: Vec2 = Vec2::new(700.0, 900.0);
/// The smallest window the app asks for.
const MIN: Vec2 = Vec2::new(800.0, 500.0);

/// An `App` and its egui context, drawn frame by frame.
struct Harness {
    ctx: egui::Context,
    app: App,
    size: Vec2,
    time: f64,
    /// The last frame's text: (text, rectangle).
    texts: Vec<(String, Rect)>,
}

impl Harness {
    fn new(kind: ThemeKind) -> Harness {
        scratch();
        let engine = Engine::load().expect("game data (resources/data) must be present for the UI tests");
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let app = App::new(&cc, engine, Vec::new(), None, (Some(kind), Some(kind.layout())), None);
        assert_eq!(app.appearance.layout, kind.layout());
        let mut h = Harness { ctx, app, size: WIDE, time: 0.0, texts: Vec::new() };
        h.frame(Vec::new());
        h
    }

    /// Switch theme (and layout) without saving it.
    fn set_kind(&mut self, kind: ThemeKind) {
        self.app.appearance = self.app.appearance.with_kind(kind);
        theme::apply(&self.ctx, &theme::Theme::of(kind));
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            predicted_dt: 1.0 / 30.0,
            focused: true,
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let out = self.ctx.run(input, |ctx| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::update(app, ctx, &mut frame);
        });
        self.texts.clear();
        for cs in &out.shapes {
            collect_text(&cs.shape, cs.clip_rect, &mut self.texts);
        }
        // Tessellation catches bad geometry (NaN rectangles and the like).
        let _ = self.ctx.tessellate(out.shapes, out.pixels_per_point);
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(Vec::new());
        }
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        self.frame(vec![ev(true), ev(false)]);
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn click_at(&mut self, pos: Pos2) {
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
    }

    /// Where the last frame drew `text` (exactly, ignoring surrounding
    /// spaces); the last one drawn, which is the topmost.
    fn find_text(&self, text: &str) -> Option<Rect> {
        self.texts.iter().rev().find(|(t, _)| t.trim() == text.trim()).map(|(_, r)| *r)
    }

    /// The topmost text drawn last frame that satisfies `pred`.
    fn find_where(&self, pred: impl Fn(&str) -> bool) -> Option<Rect> {
        self.texts.iter().rev().find(|(t, _)| pred(t)).map(|(_, r)| *r)
    }

    fn count(&self, text: &str) -> usize {
        self.texts.iter().filter(|(t, _)| t.trim() == text.trim()).count()
    }

    fn on_screen(&self) -> Vec<&str> {
        self.texts.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// Click the text `label` (e.g. a button or tab caption).
    fn click_text(&mut self, label: &str) {
        let r = self.find_text(label).unwrap_or_else(|| panic!("no text {label:?} on screen; shown: {:?}", self.on_screen()));
        self.click_at(r.center());
    }

    /// Click the topmost text that satisfies `pred`.
    fn click_where(&mut self, what: &str, pred: impl Fn(&str) -> bool) {
        let r = self.find_where(pred).unwrap_or_else(|| panic!("no text like {what} on screen; shown: {:?}", self.on_screen()));
        self.click_at(r.center());
    }

    /// Opens a character (it loads on a background thread) and waits for
    /// its tab.
    fn open(&mut self, path: &Path) -> usize {
        let path = own_copy(path);
        let before = self.app.views.len();
        self.app.open(&path);
        self.wait_loaded(before + 1);
        assert_eq!(self.app.views.len(), before + 1, "opening {}: {:?}", path.display(), self.app.status);
        self.app.views.len() - 1
    }

    /// Draws frames until `n` characters are open (loads finish on other
    /// threads) or a minute passed.
    fn wait_loaded(&mut self, n: usize) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while self.app.views.len() < n && std::time::Instant::now() < end {
            self.frame(Vec::new());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Close everything a click may have opened, and every character.
    fn reset(&mut self) {
        // A random click may have started loading a file (Recent, Open):
        // let those loads land before closing, or the character appears
        // after the reset and the next open finds it already open.
        crate::bg::wait("load:", std::time::Duration::from_secs(60));
        self.frame(Vec::new());
        self.close_all();
        let a = &mut self.app;
        (a.show_dice, a.show_initiative, a.show_print, a.show_export, a.show_about, a.show_sources, a.show_settings) = (false, false, false, false, false, false, false);
        a.wizard = None;
        a.critter = None;
        a.home = None;
        a.online.show_settings = false;
        a.online.join = None;
        a.io.ask_format = None;
        a.ws.palette.close();
        egui::Popup::close_all(&self.ctx);
        self.frame(Vec::new());
    }

    /// Close every character tab without asking.
    fn close_all(&mut self) {
        while !self.app.views.is_empty() {
            self.app.close_tab(0, true);
        }
        self.app.pending = None;
    }

    fn palette_pick(&mut self, query: &str) {
        self.key(Key::K, Modifiers::COMMAND);
        assert!(self.app.ws.palette.open, "Ctrl+K opens the palette");
        self.frame(Vec::new());
        self.type_text(query);
        self.frames(2);
        // The game-data entries are built on another thread.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while self.app.ws.palette.needs_records() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.frame(Vec::new());
        }
        self.frame(Vec::new());
        self.key(Key::Enter, Modifiers::NONE);
        assert!(!self.app.ws.palette.open, "Enter runs the entry and closes the palette");
    }
}

fn collect_text(shape: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(v) => v.iter().for_each(|s| collect_text(s, clip, out)),
        egui::Shape::Text(t) => {
            let r = t.galley.rect.translate(t.pos.to_vec2());
            if r.intersects(clip) {
                out.push((t.galley.text().to_owned(), r.intersect(clip)));
            }
        }
        _ => {}
    }
}


// ----- the render matrix -----

/// How one matrix test draws each page.
#[derive(Clone, Copy)]
struct Matrix {
    /// (size, theme) passes per page.
    passes: &'static [(Vec2, ThemeKind)],
}

const FRAMES: usize = 3;
/// The fixtures are split over this many tests per layout, so they run
/// in parallel.
const SHARDS: usize = 4;

const CLASSIC_PASSES: &[(Vec2, ThemeKind)] = &[(WIDE, ThemeKind::Graphite), (NARROW, ThemeKind::Classic)];
const WORKSPACE_PASSES: &[(Vec2, ThemeKind)] = &[(WIDE, ThemeKind::WorkspaceDark), (NARROW, ThemeKind::WorkspaceLight)];
const CLASSIC_FULL: &[(Vec2, ThemeKind)] = &[(WIDE, ThemeKind::Graphite), (NARROW, ThemeKind::Graphite), (MIN, ThemeKind::Graphite), (WIDE, ThemeKind::Classic), (NARROW, ThemeKind::Classic)];
const WORKSPACE_FULL: &[(Vec2, ThemeKind)] = &[(WIDE, ThemeKind::WorkspaceDark), (NARROW, ThemeKind::WorkspaceDark), (MIN, ThemeKind::WorkspaceDark), (WIDE, ThemeKind::WorkspaceLight), (NARROW, ThemeKind::WorkspaceLight)];

impl Matrix {
    fn of(layout: Layout) -> Matrix {
        let passes = match (layout, full()) {
            (Layout::Classic, false) => CLASSIC_PASSES,
            (Layout::Classic, true) => CLASSIC_FULL,
            (Layout::Workspace, false) => WORKSPACE_PASSES,
            (Layout::Workspace, true) => WORKSPACE_FULL,
        };
        Matrix { passes }
    }
}

/// Every page the layout has for character `i`.
fn pages(h: &Harness, i: usize) -> Vec<Section> {
    match h.app.appearance.layout {
        Layout::Classic => {
            let v = &h.app.views[i];
            let mut out = Vec::new();
            for (t, _) in TABS {
                if !v.visible(*t) {
                    continue;
                }
                if *t == Tab::StreetGear {
                    out.extend((0..5).map(Section::Gear));
                } else {
                    out.push(Section::Page(*t));
                }
            }
            out
        }
        Layout::Workspace => h.app.views[i].ws_nav(&h.app.lang).into_iter().flat_map(|g| g.items.into_iter().map(|it| it.section)).collect(),
    }
}

/// Workspace: go to Play or History, which only the sidebar and the
/// palette reach. Clicks the sidebar entry when it is on screen.
fn go_special(h: &mut Harness, s: Section) {
    let label = h.app.lang.tr(s.label());
    h.size = WIDE;
    h.frame(Vec::new());
    if let Some(r) = h.find_text(&label) {
        h.click_at(r.center());
        h.frame(Vec::new());
    }
    if h.count(&label) < 2 {
        // Not on screen, or covered by a window.
        h.palette_pick(&label);
        h.frame(Vec::new());
    }
    // The sidebar entry and the page's heading.
    assert!(h.count(&label) >= 2, "went to {label}; shown: {:?}", h.on_screen());
}

/// Go to `s` on character `i` and draw it in every pass.
fn draw_page(h: &mut Harness, i: usize, s: Section, m: Matrix) {
    match s {
        Section::Page(_) | Section::Gear(_) | Section::Review => h.app.views[i].ws_go(s),
        Section::Play | Section::History => go_special(h, s),
        _ => unreachable!("not a character section: {s:?}"),
    }
    for &(size, kind) in m.passes {
        h.size = size;
        h.set_kind(kind);
        h.frames(FRAMES);
    }
    let want = match s {
        Section::Page(t) => Some(t),
        Section::Gear(_) => Some(Tab::StreetGear),
        _ => None,
    };
    if let Some(want) = want {
        assert_eq!(h.app.views[i].ws_tab(), want, "the page fell back to another tab");
    }
}

/// Classic: click through the right-hand panel's tabs.
fn draw_side_tabs(h: &mut Harness) {
    h.size = WIDE;
    h.app.views[h.app.active].ws_go(Section::Page(Tab::Common));
    h.frames(2);
    let labels = [h.app.lang.tr("Other Info"), h.app.lang.s("String_SpellDefense"), h.app.lang.tr("History"), h.app.lang.tr("Condition Monitor"), h.app.lang.tr("Karma Summary")];
    for l in labels {
        if let Some(r) = h.find_text(&l) {
            h.click_at(r.center());
            h.frames(2);
        }
    }
}

/// Known GUI bugs the matrix skips: (fixture or "*", layout, section
/// label, why). Each has an `#[ignore]` repro test below.
const KNOWN: &[(&str, Option<Layout>, &str, &str)] = &[];

fn known(fixture: &str, layout: Layout, s: Section) -> Option<&'static str> {
    KNOWN.iter().find(|(f, l, sec, _)| (*f == "*" || *f == fixture) && l.is_none_or(|l| l == layout) && *sec == s.label()).map(|k| k.3)
}

/// Draws every page of every fixture in shard `shard`; one report of
/// all the (fixture, page) pairs that panicked.
fn run_matrix(layout: Layout, shard: usize) {
    let m = Matrix::of(layout);
    let fixtures: Vec<PathBuf> = matrix_fixtures().into_iter().enumerate().filter(|(n, _)| n % SHARDS == shard).map(|(_, p)| p).collect();
    let first = m.passes[0].1;
    let mut failures: Vec<String> = Vec::new();
    let mut drawn = 0usize;
    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // A fresh app per fixture (the engine loads in well under a second).
        let mut h = Harness::new(first);
        let opened = guarded(|| {
            let i = h.open(path);
            h.frames(2);
            i
        });
        let i = match opened {
            Ok(i) => i,
            Err(e) => {
                failures.push(format!("{name} [{layout:?}] opening: {e}"));
                continue;
            }
        };
        let creating = !h.app.views[i].ch().created;
        for s in pages(&h, i) {
            if let Some(why) = known(&name, layout, s) {
                eprintln!("skipping known GUI bug: {name} [{layout:?}] {}: {why}", s.label());
                continue;
            }
            let r = guarded(|| {
                draw_page(&mut h, i, s, m);
                // The guide shows while creating (Classic and Workspace
                // draw it the same way).
                if creating && s == Section::Page(Tab::Common) {
                    h.app.views[i].set_guided(true);
                    h.frames(FRAMES);
                    h.app.views[i].set_guided(false);
                    h.frames(1);
                }
            });
            drawn += 1;
            if let Err(e) = r {
                failures.push(format!("{name} [{layout:?}] {}: {e}", s.label()));
                // The app (and the engine's mutexes) may be poisoned.
                h = Harness::new(first);
                if guarded(|| h.open(path)).is_err() {
                    break;
                }
            }
        }
        // Every item in the item inspector/editor.
        if h.app.views.len() == 1 {
            let items = h.app.views[0].ws_items(&h.app.lang);
            for it in items {
                if it.section == Section::Page(Tab::Skills) && h.app.views[0].ch().created {
                    continue;
                }
                let r = guarded(|| {
                    h.size = WIDE;
                    h.app.views[0].ws_show_item(it.section, &it.guid);
                    h.frames(2);
                });
                if let Err(e) = r {
                    failures.push(format!("{name} [{layout:?}] item {:?} ({}) in {}: {e}", it.name, it.list, it.section.label()));
                    h = Harness::new(first);
                    if guarded(|| h.open(path)).is_err() {
                        break;
                    }
                }
            }
        }
        if layout == Layout::Classic {
            if let Err(e) = guarded(|| draw_side_tabs(&mut h)) {
                failures.push(format!("{name} [{layout:?}] side panel tabs: {e}"));
                h = Harness::new(first);
            }
        }
        if let Err(e) = guarded(|| {
            h.close_all();
            h.frames(1);
        }) {
            failures.push(format!("{name} [{layout:?}] closing: {e}"));
        }
    }
    eprintln!("{layout:?} shard {shard}: drew {drawn} pages of {} fixtures", fixtures.len());
    assert!(failures.is_empty(), "{} page(s) panicked:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn classic_pages_shard_0() {
    run_matrix(Layout::Classic, 0);
}
#[test]
fn classic_pages_shard_1() {
    run_matrix(Layout::Classic, 1);
}
#[test]
fn classic_pages_shard_2() {
    run_matrix(Layout::Classic, 2);
}
#[test]
fn classic_pages_shard_3() {
    run_matrix(Layout::Classic, 3);
}
#[test]
fn workspace_pages_shard_0() {
    run_matrix(Layout::Workspace, 0);
}
#[test]
fn workspace_pages_shard_1() {
    run_matrix(Layout::Workspace, 1);
}
#[test]
fn workspace_pages_shard_2() {
    run_matrix(Layout::Workspace, 2);
}
#[test]
fn workspace_pages_shard_3() {
    run_matrix(Layout::Workspace, 3);
}

// ----- home, campaign, windows -----

/// A campaign file with two players and an NPC from the fixtures.
fn campaign_file() -> PathBuf {
    use chummer_core::campaign::{Campaign, Encounter, Member, MemberKind};
    use chummer_core::character::Character;
    let path = scratch().join(format!("ui-{:?}.chummercampaign", std::thread::current().id()).replace(['(', ')'], ""));
    let mut c = Campaign::new("UI test");
    c.encounters.push(Encounter::new("Encounter 1"));
    for (f, kind) in [("Munin_Career.chum5", MemberKind::Player), ("Soma (Career).chum5", MemberKind::Player), ("Ghile Mear.chum5", MemberKind::Npc)] {
        let ch = Character::load(&fixture(f)).unwrap();
        c.add(Member::embedded(kind, &ch));
    }
    c.save(&path).unwrap();
    path
}

fn sizes(h: &mut Harness, n: usize) {
    for size in [WIDE, NARROW] {
        h.size = size;
        h.frames(n);
    }
}

fn home_and_campaign(kind: ThemeKind) {
    let mut h = Harness::new(kind);
    // Home with nothing open, then with recent files.
    for home in [Home::Roster, Home::MasterIndex] {
        h.app.home = Some(home);
        sizes(&mut h, FRAMES);
    }
    h.app.recent = vec![fixture("Munin.chum5"), scratch().join("missing.chum5")];
    h.app.home = Some(Home::Roster);
    sizes(&mut h, FRAMES);
    // A new, empty campaign; then one with members.
    h.app.new_campaign();
    assert_eq!(h.app.home, Some(Home::Campaign));
    sizes(&mut h, FRAMES);
    assert!(h.app.close_campaign(true));
    h.app.open_campaign(&campaign_file());
    assert!(h.app.gm.is_some(), "{:?}", h.app.status);
    sizes(&mut h, FRAMES);
    {
        let App { gm, views, .. } = &mut h.app;
        let gm = gm.as_mut().unwrap();
        gm.add_all_players(views);
        gm.roll_initiative(views);
    }
    sizes(&mut h, FRAMES);
    // A member in its own tab, then back to the GM screen.
    let id = h.app.gm.as_ref().unwrap().campaign.members[0].id;
    h.app.open_member(id);
    assert_eq!(h.app.views.len(), 1);
    sizes(&mut h, FRAMES);
    h.app.close_tab(0, false);
    assert!(h.app.views.is_empty());
    assert_eq!(h.app.home, Some(Home::Campaign));
    sizes(&mut h, FRAMES);
    assert!(h.app.close_campaign(true));
    sizes(&mut h, 1);
}

#[test]
fn classic_home_and_campaign() {
    home_and_campaign(ThemeKind::Graphite);
}

#[test]
fn workspace_home_and_campaign() {
    home_and_campaign(ThemeKind::WorkspaceDark);
}

/// The GM's Players & invites panel, in both layouts: an online campaign
/// (not served), a new invite with a character to give, its row, a new
/// link, a revoke and a new campaign key, each through the panel's own
/// buttons.
#[test]
fn players_and_invites_panel() {
    for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceDark, ThemeKind::WorkspaceLight] {
        let mut h = Harness::new(kind);
        // A fresh campaign each round: the last round's online state
        // (saved on a background thread at close) belongs to another one.
        crate::bg::wait("save:authority", std::time::Duration::from_secs(60));
        let file = campaign_file();
        for ext in ["authority", "authority.journal", "invites"] {
            let _ = std::fs::remove_file(file.with_extension(ext));
        }
        h.app.open_campaign(&file);
        {
            let App { gm, online, engine, views, .. } = &mut h.app;
            gm.as_mut().unwrap().go_online(online, engine, views, false).unwrap();
        }
        crate::bg::wait("gm-go-online", std::time::Duration::from_secs(120));
        for _ in 0..200 {
            h.frames(1);
            if h.app.gm.as_ref().unwrap().is_online() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(h.app.gm.as_ref().unwrap().is_online(), "{:?} {:?}", h.app.status, h.app.gm.as_ref().unwrap().online_error());
        sizes(&mut h, FRAMES);
        // The clicks at the wide size (in the narrow one the inspector
        // reaches past the window's edge).
        h.size = WIDE;
        h.frames(2);
        assert!(h.find_text("No invites yet.").is_some(), "{:?}", h.on_screen());
        h.click_text("New invite…");
        h.frames(2);
        h.click_text("Name, e.g. Anna");
        h.type_text("Anna");
        h.frames(1);
        h.click_text("Create link");
        h.frames(2);
        let invites: Vec<(String, chummer_net::invite::InviteId)> = h.app.gm.as_ref().unwrap().hosted().unwrap().host.authority().invites().values().map(|i| (i.label.clone(), i.id)).collect();
        assert_eq!(invites.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Anna"]);
        assert!(h.find_text("Link for Anna: send it to that player only.").is_some(), "{:?}", h.on_screen());
        assert!(h.find_text("waiting for the player").is_some(), "{:?}", h.on_screen());
        assert!(h.find_text("not used yet").is_some(), "{:?}", h.on_screen());
        // A new link (with its confirmation): the key changes.
        let key = |h: &Harness| h.app.gm.as_ref().unwrap().hosted().unwrap().host.authority().invite(&invites[0].1).unwrap().key();
        let before = key(&h);
        h.click_text("New link");
        // (The question scrolls into view: let the scrolling finish.)
        h.frames(30);
        assert!(h.find_where(|t| t.starts_with("Give Anna a new link?")).is_some(), "{:?}", h.on_screen());
        h.click_text("New link");
        h.frames(2);
        assert_ne!(key(&h), before);
        // Revoke, confirmed.
        h.click_text("Revoke");
        h.frames(30);
        assert!(h.find_where(|t| t.starts_with("Revoke Anna?")).is_some(), "{:?}", h.on_screen());
        h.click_text("Revoke");
        h.frames(2);
        let state = h.app.gm.as_ref().unwrap().hosted().unwrap().host.authority().invite(&invites[0].1).unwrap().state(0);
        assert!(matches!(state, chummer_sync::invites::InviteState::Revoked { .. }), "{state:?}");
        assert!(h.find_text("revoked").is_some(), "{:?}", h.on_screen());
        // A new campaign key, after the explanation.
        h.click_text("New campaign key…");
        h.frames(3);
        assert!(h.find_where(|t| t.starts_with("Make a new campaign key (now generation 0)?")).is_some(), "{:?}", h.on_screen());
        h.click_text("New campaign key");
        h.frames(2);
        assert_eq!(h.app.gm.as_ref().unwrap().hosted().unwrap().host.authority().key_generation(), 1);
        assert!(h.find_where(|t| t.starts_with("The campaign key is now generation 1")).is_some() || h.app.status.as_ref().is_some_and(|(m, _)| m.starts_with("The campaign key is now generation 1")), "{:?}", h.app.status);
        sizes(&mut h, FRAMES);
        assert!(h.app.close_campaign(true));
        h.frames(1);
    }
}

/// The tool windows and dialogs over a character, in both layouts.
#[test]
fn tool_windows_and_dialogs() {
    for kind in [ThemeKind::Classic, ThemeKind::WorkspaceLight] {
        let mut h = Harness::new(kind);
        h.open(&fixture("Munin_Career.chum5"));
        h.frames(2);
        let a = &mut h.app;
        a.show_dice = true;
        a.show_initiative = true;
        a.show_print = true;
        a.show_export = true;
        a.show_about = true;
        a.show_sources = true;
        a.show_settings = true;
        sizes(&mut h, FRAMES);
        let a = &mut h.app;
        (a.show_dice, a.show_initiative, a.show_print, a.show_export, a.show_about, a.show_sources, a.show_settings) = (false, false, false, false, false, false, false);
        a.online.show_settings = true;
        a.online.join = Some((String::new(), "Tester".into()));
        sizes(&mut h, FRAMES);
        h.app.online.show_settings = false;
        h.app.online.join = None;
        h.app.wizard = Some(crate::wizard::Wizard::new());
        sizes(&mut h, FRAMES);
        h.app.wizard = None;
        h.app.critter = Some(crate::gm_ui::CritterWizard::new());
        sizes(&mut h, FRAMES);
        h.app.critter = None;
        // The unsaved-changes question, for a tab and for quitting.
        h.app.pending = Some(crate::Pending::CloseTab(0));
        sizes(&mut h, 2);
        h.app.pending = Some(crate::Pending::Quit);
        sizes(&mut h, 2);
        h.app.pending = None;
        h.frames(1);
    }
}

// ----- interactions -----

fn gear_count(h: &Harness) -> usize {
    h.app.views[h.app.active].ch().items("gears", "gear").len()
}

/// Workspace: the palette opens with Ctrl+K, filters as you type, runs
/// the entry with Enter and closes with Escape.
#[test]
fn workspace_palette() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin.chum5"));
    h.frames(2);
    // Navigate to a section.
    let skills = h.app.lang.tr("Skills");
    h.palette_pick(&skills);
    h.frames(1);
    assert_eq!(h.app.views[i].ws_tab(), Tab::Skills);
    // Escape closes without running anything.
    h.key(Key::K, Modifiers::COMMAND);
    h.frame(Vec::new());
    h.type_text("Limits");
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(Vec::new());
    assert!(!h.app.ws.palette.open);
    assert_eq!(h.app.views[i].ws_tab(), Tab::Skills);
    // Arrow keys move the selection, also past either end.
    h.app.views[i].ws_go(Section::Page(Tab::Limits));
    h.key(Key::K, Modifiers::COMMAND);
    h.frame(Vec::new());
    h.type_text("a");
    for _ in 0..4 {
        h.key(Key::ArrowUp, Modifiers::NONE);
    }
    for _ in 0..6 {
        h.key(Key::ArrowDown, Modifiers::NONE);
    }
    h.type_text("zzzzzz");
    h.key(Key::ArrowDown, Modifiers::NONE);
    h.key(Key::Enter, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(Vec::new());
    assert!(!h.app.ws.palette.open);
    // A game-data record: opens the Master Index.
    h.palette_pick("Ruthenium Polymer");
    h.frames(2);
    // A command: the dice roller.
    h.palette_pick(&h.app.lang.tr("Dice Roller"));
    h.frames(1);
    assert!(h.app.show_dice);
    // Switch layout from the palette with a character open.
    h.palette_pick("Classic layout");
    h.frames(1);
    assert_eq!(h.app.appearance.layout, Layout::Classic, "{:?}", h.app.status);
    sizes(&mut h, 2);
}

/// Workspace, career: the palette offers advances bought with karma.
#[test]
fn workspace_palette_buys_an_advance() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin_Career.chum5"));
    // A cheap page behind the palette (see the report on career frames).
    h.app.views[i].ws_go(Section::Page(Tab::Limits));
    h.app.views[i].doc_mut().apply(chummer_core::command::Command::SetKarma { value: 100 }).unwrap();
    h.frames(1);
    let karma = h.app.views[i].ch().karma;
    let body = h.app.lang.tr("Body");
    h.palette_pick(&format!("Raise {body}"));
    // The purchase runs at the end of the next frame.
    h.frames(1);
    assert!(h.app.views[i].ch().karma < karma, "raising Body costs karma ({karma} before; {:?})", h.app.status);
    h.app.undo();
    assert_eq!(h.app.views[i].ch().karma, karma, "undo gives the karma back");
}

/// Workspace: "Add" on the gear page opens the inline catalog; search,
/// arrow to the first match and Enter adds it. Then undo and redo with
/// the keyboard.
#[test]
fn workspace_catalog_add_then_undo_redo() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    let before = gear_count(&h);
    h.app.views[i].ws_open_catalog((Tab::StreetGear, 0), "gear", None);
    sizes(&mut h, 2);
    h.size = WIDE;
    h.frames(1);
    h.type_text("Flashlight");
    h.frames(2);
    h.key(Key::ArrowDown, Modifiers::NONE);
    h.frames(2);
    assert!(h.app.views[i].ws_catalog_has_selection(), "ArrowDown selects the first match; shown: {:?}", h.on_screen());
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(2);
    assert_eq!(gear_count(&h), before + 1, "Enter adds the item ({:?})", h.app.status);
    assert!(h.app.views[i].doc().undo_label().is_some());
    // The search field may still have the keyboard; Escape gives it up.
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(1);
    h.key(Key::Z, Modifiers::COMMAND);
    h.frames(2);
    assert_eq!(gear_count(&h), before, "Ctrl+Z removes it");
    h.key(Key::Y, Modifiers::COMMAND);
    h.frames(2);
    assert_eq!(gear_count(&h), before + 1, "Ctrl+Y adds it again");
    h.key(Key::Z, Modifiers::COMMAND);
    h.frames(1);
    h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.frames(2);
    assert_eq!(gear_count(&h), before + 1, "Ctrl+Shift+Z redoes too");
    // Undo everything there is, then once more.
    for _ in 0..20 {
        h.app.undo();
        h.frames(1);
    }
    assert!(h.app.views[i].doc().undo_label().is_none());
    h.app.redo();
    h.frames(1);
    // Every gear page with the catalog open.
    for g in 0..5 {
        h.app.views[i].ws_go(Section::Gear(g));
        h.frames(1);
        for tag in ["gear", "armor", "weapon", "drug", "lifestyle"] {
            h.app.views[i].ws_open_catalog((Tab::StreetGear, g), tag, None);
            sizes(&mut h, 2);
        }
    }
    for (tab, tag) in [(Tab::Cyberware, "cyberware"), (Tab::Cyberware, "bioware"), (Tab::Vehicles, "vehicle")] {
        h.app.views[i].ws_go(Section::Page(tab));
        h.app.views[i].ws_open_catalog((tab, 0), tag, None);
        sizes(&mut h, 2);
        // Select the first row and look at the preview in the inspector.
        h.size = WIDE;
        h.key(Key::ArrowDown, Modifiers::NONE);
        h.frames(2);
        h.key(Key::ArrowDown, Modifiers::NONE);
        h.frames(2);
    }
}

/// A window tall enough for Munin's whole gear list.
const TALL: Vec2 = Vec2::new(1600.0, 1500.0);

/// The guid of a top-level gear item by name.
fn gear_guid(h: &Harness, name: &str) -> String {
    h.app.views[h.app.active].ch().items("gears", "gear").iter().find(|g| g.get("name") == name).map(|g| g.get("guid")).unwrap_or_else(|| panic!("no gear {name}"))
}

/// Whether the inline catalog shows (its panel title).
fn catalog_shown(h: &Harness) -> bool {
    h.find_text(&h.app.lang.tr("Catalog")).is_some()
}

/// Type `search` into the catalog's search field (replacing what is
/// there) and select the first match.
fn catalog_search(h: &mut Harness, i: usize, search: &str) {
    h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("ws_catalog_search")));
    h.frames(1);
    h.key(Key::A, Modifiers::COMMAND);
    h.key(Key::Backspace, Modifiers::NONE);
    h.type_text(search);
    h.frames(2);
    h.key(Key::ArrowDown, Modifiers::NONE);
    h.frames(2);
    assert!(h.app.views[i].ws_catalog_has_selection(), "ArrowDown selects the first match for {search:?}; shown: {:?}", h.on_screen());
}

/// Opens the gear page's catalog with `search` typed and the first
/// match selected.
fn gear_catalog_pick(h: &mut Harness, i: usize, search: &str) {
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    if !catalog_shown(h) {
        let add = h.app.lang.tr_fmt("Add {0}", &[&crate::view::kind_noun(&h.app.lang, "Gear")]);
        h.click_text(&add);
        h.frames(2);
    }
    assert!(catalog_shown(h), "Add gear opens the catalog next to the inventory: {:?}", h.on_screen());
    catalog_search(h, i, search);
}

/// Click the inline Undo of the row showing the "Added" chip: left of
/// Remove and the ⋯, at the right end of the actions column (where the
/// footer's note ends).
fn click_inline_undo(h: &mut Harness) {
    let chip = h.find_text(&h.app.lang.tr("Added")).expect("the new row has the Added chip");
    let tail = h.find_text(&h.app.lang.tr("purchase value")).expect("the inventory's footer");
    // ⋯ at the right end, Remove left of it, then Undo.
    h.click_at(Pos2::new(tail.right() - 2.0 * (22.0 + 2.0) - 11.0, chip.center().y));
    h.frames(2);
}

/// Workspace inventory: the catalog opens beside the inventory (no view
/// swap); Enter adds, the item shows highlighted ("Added", in the tray)
/// with an inline Undo, and that Undo takes it back. When other changes
/// came after, the inline Undo removes the item and keeps them.
#[test]
fn workspace_inventory_add_then_inline_undo() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    let before = gear_count(&h);
    gear_catalog_pick(&mut h, i, "Flashlight");
    assert!(h.find_text(&h.app.lang.tr("Your gear")).is_some(), "the inventory shows with the catalog: {:?}", h.on_screen());
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(3);
    assert_eq!(gear_count(&h), before + 1, "Enter adds ({:?})", h.app.status);
    let added = h.app.views[i].ws_last_added().map(str::to_owned).expect("the add is recorded");
    assert!(h.app.views[i].ws_added_is_latest(&added), "the add is the latest change");
    assert!(catalog_shown(&h), "the catalog stays open");
    assert!(h.find_text(&h.app.lang.tr("Added since you opened this page")).is_some(), "the tray lists it");
    click_inline_undo(&mut h);
    assert_eq!(gear_count(&h), before, "the inline Undo removes it");
    assert!(h.app.views[i].doc().can_redo(), "it was an undo");
    assert!(h.app.views[i].ws_last_added().is_none());
    // Not the latest change any more: the inline Undo removes it instead.
    catalog_search(&mut h, i, "Flashlight");
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(2);
    let added = h.app.views[i].ws_last_added().map(str::to_owned).expect("added again");
    let glasses = gear_guid(&h, "Glasses");
    assert!(h.app.views[i].doc_mut().set(chummer_core::command::Command::SetItemText { guid: glasses, field: "notes".into(), value: "later".into() }));
    h.frames(2);
    assert!(!h.app.views[i].ws_added_is_latest(&added));
    click_inline_undo(&mut h);
    assert_eq!(gear_count(&h), before, "the inline Undo removed it");
    let notes = h.app.views[i].ch().items("gears", "gear").iter().find(|g| g.get("name") == "Glasses").map(|g| g.get("notes"));
    assert_eq!(notes.as_deref(), Some("later"), "the later change stays");
}

/// Workspace inventory: a narrow page stacks the catalog above the
/// inventory, with the inspector folded into the details strip; a wide
/// one puts them side by side.
#[test]
fn workspace_inventory_narrow_stacks() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin.chum5"));
    gear_catalog_pick(&mut h, i, "Flashlight");
    let (cat, inv) = (h.app.lang.tr("Catalog"), h.app.lang.tr("Your gear"));
    let c = h.find_text(&cat).unwrap();
    let v = h.find_text(&inv).unwrap();
    assert!(c.right() < v.left() && (c.center().y - v.center().y).abs() < 4.0, "side by side at {:?}: {c:?} {v:?}", h.size);
    assert!(h.find_text(&h.app.lang.tr("Issues")).is_some() || h.find_text(&h.app.lang.tr("Item")).is_some(), "the inspector shows when wide");
    h.size = Vec2::new(1000.0, 900.0);
    h.frames(3);
    let c = h.find_text(&cat).unwrap();
    let v = h.find_text(&inv).unwrap();
    assert!(c.bottom() < v.top(), "stacked at {:?}: {c:?} {v:?}", h.size);
    // Below 1180 the inspector folds into the details strip under the
    // selected row.
    let strip = h.app.lang.tr("Nuyen after").to_uppercase();
    assert!(h.find_text(&strip).is_some(), "details strip: {:?}", h.on_screen());
    assert!(h.find_text(&h.app.lang.tr("Item")).is_none(), "the inspector is folded");
    h.size = NARROW;
    h.frames(3);
    assert!(h.find_text(&cat).unwrap().bottom() < h.find_text(&inv).unwrap().top());
}

/// Workspace inventory: the selected inventory row becomes the
/// catalog's target when it can hold the selected record ("Adding into",
/// the Target chip, its free capacity); when it cannot, the purchase goes
/// to the top level and the bar says why. What does not fit a target is
/// dimmed with the reason; Add puts the item inside; × adds at the top
/// level again. The inspector has no "Install in" choice.
#[test]
fn workspace_inventory_adds_into_the_target() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    gear_catalog_pick(&mut h, i, "Flashlight");
    assert!(h.find_text(&h.app.lang.tr("Install in")).is_none(), "no Install in choice: {:?}", h.on_screen());
    let glasses = gear_guid(&h, "Glasses");
    // Munin's glasses (rating 2) are full: a vision enhancement is
    // refused with the reason.
    let row = inventory_row(&h, "Glasses");
    h.click_at(row.center());
    h.frames(2);
    catalog_search(&mut h, i, "Vision Magnification");
    h.frames(1);
    assert!(h.app.views[i].ws_catalog_target_guid().is_none());
    assert!(h.find_where(|t| t.contains("Glasses is full (2/2 capacity used")).is_some(), "{:?}", h.on_screen());
    // Rating 4 has room.
    assert!(h.app.views[i].doc_mut().set(chummer_core::command::Command::SetItemRating { guid: glasses.clone(), rating: 4 }));
    catalog_search(&mut h, i, "Flashlight");
    // The inventory's "Glasses" (right of the catalog's).
    let row = inventory_row(&h, "Glasses");
    h.click_at(row.center());
    h.frames(2);
    // A flashlight does not go into glasses: top level, and why.
    assert!(h.app.views[i].ws_catalog_target_guid().is_none(), "a flashlight cannot go into the glasses");
    assert!(h.find_text(&h.app.lang.tr("Adding at the top level")).is_some(), "{:?}", h.on_screen());
    assert!(h.find_where(|t| t.contains("only takes")).is_some(), "the bar says why: {:?}", h.on_screen());
    // A vision enhancement does: the selected glasses are the target.
    catalog_search(&mut h, i, "Vision Magnification");
    h.frames(1);
    assert_eq!(h.app.views[i].ws_catalog_target_guid().as_deref(), Some(glasses.as_str()), "the selected container is the target");
    assert!(h.find_text(&h.app.lang.tr("Adding into")).is_some() && h.find_where(|t| t.contains(&h.app.lang.tr("Target"))).is_some(), "{:?}", h.on_screen());
    assert!(h.find_where(|t| t.ends_with(" capacity") && t.contains('/')).is_some(), "the target's free capacity: {:?}", h.on_screen());
    // What does not fit is dimmed with the reason.
    catalog_search(&mut h, i, "Flashlight");
    h.frames(1);
    catalog_search(&mut h, i, "Vision Magnification");
    let kids = |h: &Harness| chummer_core::items::edit::children(h.app.views[i].ch(), &glasses).len();
    let before = kids(&h);
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(2);
    assert_eq!(kids(&h), before + 1, "Add puts it into the glasses ({:?})", h.app.status);
    assert!(h.find_text(&h.app.lang.tr("Install in")).is_none());
    // × (add at the top level instead).
    let bar = h.find_text(&h.app.lang.tr("Adding into")).expect("the target bar");
    let change = h.find_text(&h.app.lang.tr("Change")).expect("Change");
    h.click_at(Pos2::new(change.right() + 4.0 + 13.0 + 8.0, bar.center().y));
    h.frames(2);
    assert!(h.app.views[i].ws_catalog_target_guid().is_none(), "× clears the target: {:?}", h.on_screen());
}

/// The inventory's row of `name`: the rightmost text (the catalog's
/// rows are left of the inventory).
fn inventory_row(h: &Harness, name: &str) -> Rect {
    h.texts.iter().filter(|(t, _)| t == name).map(|(_, r)| *r).max_by(|a, b| a.left().total_cmp(&b.left())).unwrap_or_else(|| panic!("{name} in the inventory: {:?}", h.on_screen()))
}

/// The catalog's row of `name`: in the left half, below the search field
/// (the lowest such text).
fn catalog_row(h: &Harness, name: &str) -> Rect {
    let mid = h.size.x / 2.0;
    h.texts.iter().filter(|(t, r)| t == name && r.left() < mid).map(|(_, r)| *r).max_by(|a, b| a.top().total_cmp(&b.top())).unwrap_or_else(|| panic!("{name} in the catalog: {:?}", h.on_screen()))
}

/// Drag with the primary button from `from` to `to` in small steps.
fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.frame(vec![Event::PointerMoved(from)]);
    h.frame(vec![button(from, true)]);
    for k in 1..=8 {
        let t = k as f32 / 8.0;
        h.frame(vec![Event::PointerMoved(from + (to - from) * t)]);
    }
    h.frame(vec![Event::PointerMoved(to)]);
    h.frame(vec![button(to, false)]);
    h.frames(2);
}

/// Buy gear with a command (no catalog); returns its guid.
fn buy_gear(h: &mut Harness, i: usize, name: &str, rating: i32, parent: Option<&str>) -> String {
    use chummer_core::command::{Command, RecordRef};
    let before: Vec<String> = h.app.views[i].ch().items("gears", "gear").iter().map(|g| g.get("guid")).collect();
    let purchase = chummer_core::items::Purchase { rating, qty: 1.0, parent: parent.map(str::to_owned), cost_multiplier: 1.0, ..Default::default() };
    h.app.views[i].doc_mut().apply(Command::AddItem { tag: "gear".into(), record: RecordRef { id: String::new(), name: name.into() }, purchase }).unwrap();
    h.frames(2);
    h.app.views[i].ch().items("gears", "gear").iter().map(|g| g.get("guid")).find(|g| !before.contains(g)).unwrap_or_default()
}

/// Workspace inventory: the ⋯ menu of a row that is not selected opens
/// with a click on the ⋯ (always there, muted), with a right-click on
/// the row and with Shift+F10 on the selected row, and stays open while
/// the pointer goes elsewhere; its entries work.
#[test]
fn workspace_inventory_row_menu_is_reachable() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(3);
    let remove = h.app.lang.tr("Remove (also removes its improvements)");
    let menu_open = |h: &Harness| h.find_where(|t| t.contains(&remove)).is_some();
    assert!(!menu_open(&h));
    // The ⋯ of the Trode Patch row (not selected, not hovered): at the
    // right end of the actions column, where the footer's note ends.
    let row = inventory_row(&h, "Trode Patch");
    let tail = h.find_text(&h.app.lang.tr("purchase value")).expect("the inventory's footer");
    let dots = Pos2::new(tail.right() - 11.0, row.center().y);
    h.click_at(dots);
    h.frames(1);
    assert!(menu_open(&h), "the ⋯ opens the row's menu: {:?}", h.on_screen());
    // The pointer leaves for another row and the empty page: still open.
    let other = inventory_row(&h, "Psyche");
    h.frame(vec![Event::PointerMoved(other.center())]);
    h.frame(vec![Event::PointerMoved(Pos2::new(row.left(), row.center().y + 400.0))]);
    h.frames(2);
    assert!(menu_open(&h), "the menu stays open while the pointer moves away");
    // Escape closes it.
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(1);
    assert!(!menu_open(&h), "Escape closes it");
    // Right-click on the row opens the same menu.
    let pos = row.center();
    let button = |pressed| Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    h.frame(vec![Event::PointerMoved(pos)]);
    h.frame(vec![button(true)]);
    h.frame(vec![button(false)]);
    h.frames(2);
    assert!(menu_open(&h), "right-click opens it: {:?}", h.on_screen());
    // Edit selects the item.
    let edit = h.app.lang.tr("Edit");
    h.click_where("Edit in the menu", |t| t.ends_with(&format!("  {edit}")));
    h.frames(2);
    let trode = gear_guid(&h, "Trode Patch");
    assert_eq!(h.app.views[i].item_editor_guid().as_deref(), Some(trode.as_str()), "Edit selected the item");
    assert!(!menu_open(&h));
    // Shift+F10 opens it for the selected row.
    h.frame(vec![Event::PointerMoved(Pos2::new(5.0, 5.0))]);
    h.key(Key::F10, Modifiers::SHIFT);
    h.frames(2);
    assert!(menu_open(&h), "Shift+F10 opens it: {:?}", h.on_screen());
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(1);
}

/// Workspace inventory: drag an owned row onto a container: it moves in
/// (MoveItem; one undo step); a drop where it cannot go does nothing and
/// the dragged chip says why.
#[test]
fn workspace_inventory_drag_into_a_container() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    let goggles = buy_gear(&mut h, i, "Goggles", 3, None);
    h.frames(2);
    let flare = {
        let ch = h.app.views[i].ch();
        let glasses = ch.items("gears", "gear").into_iter().find(|g| g.get("name") == "Glasses").unwrap();
        glasses.child("children").unwrap().children_named("gear").find(|g| g.get("name") == "Flare Compensation").unwrap().get("guid")
    };
    let from = inventory_row(&h, "Flare Compensation").center();
    let to = inventory_row(&h, "Goggles").center();
    drag(&mut h, from, to);
    let parent = chummer_core::items::edit::parent(h.app.views[i].ch(), &flare).map(|p| p.get("guid"));
    assert_eq!(parent.as_deref(), Some(goggles.as_str()), "dropped into the goggles ({:?})", h.app.status);
    assert!(h.app.views[i].doc().undo_label().unwrap_or_default().contains("Moved Flare Compensation into Goggles"));
    // Onto the drug (no container): refused, it stays.
    let from = inventory_row(&h, "Flare Compensation").center();
    let to = inventory_row(&h, "Psyche").center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.frame(vec![Event::PointerMoved(from)]);
    h.frame(vec![button(from, true)]);
    for k in 1..=8 {
        h.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 8.0))]);
    }
    h.frames(1);
    assert!(h.find_where(|t| t.contains("Psyche only takes")).is_some(), "the chip says why: {:?}", h.on_screen());
    // Escape cancels the drag.
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![button(to, false)]);
    h.frames(2);
    let parent = chummer_core::items::edit::parent(h.app.views[i].ch(), &flare).map(|p| p.get("guid"));
    assert_eq!(parent.as_deref(), Some(goggles.as_str()), "still in the goggles");
}

/// Workspace inventory: drag a catalog record onto a location header:
/// bought into that location; onto a container: bought into it.
#[test]
fn workspace_catalog_drag_onto_a_location_and_a_container() {
    use chummer_core::command::Command;
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    let psyche = gear_guid(&h, "Psyche");
    assert!(h.app.views[i].doc_mut().set(Command::AddItemLocation { guid: psyche, name: "Car".into() }));
    gear_catalog_pick(&mut h, i, "Flashlight");
    let car = chummer_core::items::edit::locations(h.app.views[i].ch(), &gear_guid(&h, "Psyche"))[0].0.clone();
    let count = gear_count(&h);
    let from = catalog_row(&h, "Flashlight").center();
    let to = inventory_row(&h, "Car").center();
    drag(&mut h, from, to);
    assert_eq!(gear_count(&h), count + 1, "bought ({:?})", h.app.status);
    let new = h.app.views[i].ch().items("gears", "gear").into_iter().rev().find(|g| g.get("name") == "Flashlight").unwrap().get("location");
    assert_eq!(new, car, "in the Car location");
    // A vision enhancement dropped on the glasses (with room at rating
    // 4) goes inside.
    let glasses = gear_guid(&h, "Glasses");
    assert!(h.app.views[i].doc_mut().set(Command::SetItemRating { guid: glasses.clone(), rating: 4 }));
    catalog_search(&mut h, i, "Thermographic Vision");
    let kids = |h: &Harness| chummer_core::items::edit::children(h.app.views[i].ch(), &glasses).len();
    let before = kids(&h);
    let from = catalog_row(&h, "Thermographic Vision").center();
    let to = inventory_row(&h, "Glasses").center();
    drag(&mut h, from, to);
    assert_eq!(kids(&h), before + 1, "bought into the glasses ({:?})", h.app.status);
}

/// Workspace inventory: "Group by" category and none; remembered.
#[test]
fn workspace_inventory_group_by() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = TALL;
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    let selected = h.app.lang.tr("Selected Gear");
    assert!(h.find_text(&selected).is_some(), "{:?}", h.on_screen());
    let id = crate::view::ws_inventory::table_id((Tab::StreetGear, 0));
    crate::workspace::table::set_group(&h.ctx, id, crate::workspace::table::GroupBy::Category);
    h.frames(2);
    assert!(h.find_text(&selected).is_none());
    assert!(h.find_text("Vision Devices").is_some() && h.find_text("Commlinks").is_some(), "category groups: {:?}", h.on_screen());
    assert!(h.find_text("Flare Compensation").is_some(), "children stay with their parent");
    crate::workspace::table::set_group(&h.ctx, id, crate::workspace::table::GroupBy::None);
    h.frames(2);
    assert!(h.find_text("Vision Devices").is_none() && h.find_text("Glasses").is_some());
    assert_eq!(crate::workspace::table::group_of(&h.ctx, id), crate::workspace::table::GroupBy::None);
}

/// Workspace: the budget strip never runs off the edge; at a narrow
/// width the less important chips go behind "+N".
#[test]
fn workspace_budget_strip_fits() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    for w in [1024.0, 800.0, 640.0] {
        h.size = Vec2::new(w, 768.0);
        h.frames(3);
        let karma = h.app.lang.tr("Karma").to_uppercase();
        let k = h.find_text(&karma).unwrap_or_else(|| panic!("the Karma chip at {w}: {:?}", h.on_screen()));
        for (t, r) in &h.texts {
            if (r.center().y - k.center().y).abs() < 12.0 {
                assert!(r.right() <= w + 0.5, "{t:?} runs off the edge at {w}: {r:?}");
            }
        }
        if w <= 800.0 {
            assert!(h.find_where(|t| t.starts_with('+') && t[1..].parse::<u32>().is_ok()).is_some(), "+N at {w}: {:?}", h.on_screen());
        }
    }
}

/// Workspace inventory: a header click sorts (high to low first for
/// numbers), again the other way, a third time back to the file's order.
#[test]
fn workspace_inventory_sorts_by_column() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Apex Predator.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(3);
    let id = crate::view::ws_inventory::table_id((Tab::StreetGear, 0));
    let cost = h.app.lang.tr("Cost").to_uppercase();
    let costs = |h: &Harness| -> Vec<String> {
        let mut v: Vec<(f32, String)> = h.texts.iter().filter(|(t, _)| t.ends_with('¥')).map(|(t, r)| (r.top(), t.clone())).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, t)| t).collect()
    };
    let unsorted = costs(&h);
    h.click_text(&cost);
    h.frames(2);
    assert_eq!(crate::workspace::table::sort_of(&h.ctx, id), Some(("cost".into(), false)), "first click: high to low");
    let down = costs(&h);
    assert_ne!(down, unsorted, "the order changed");
    h.click_text(&cost);
    h.frames(2);
    assert_eq!(crate::workspace::table::sort_of(&h.ctx, id), Some(("cost".into(), true)));
    assert_ne!(costs(&h), down);
    h.click_text(&cost);
    h.frames(2);
    assert_eq!(crate::workspace::table::sort_of(&h.ctx, id), None, "third click: the file's order");
    assert_eq!(costs(&h), unsorted);
}

/// Workspace: the dialogs the magic page opens are Workspace dialogs
/// (their own header, Workspace rows and buttons): "Add Spell…", pick a
/// spell, its options, Add. Escape closes a dialog.
#[test]
fn workspace_spell_dialogs() {
    let mut h = Harness::new(ThemeKind::WorkspaceLight);
    let i = h.open(&fixture("Harmony.chum5"));
    h.app.views[i].ws_go(Section::Page(Tab::Magician));
    h.size = WIDE;
    h.frames(2);
    let spells = |h: &Harness| h.app.views[i].doc().items("spells", "spell").len();
    let before = spells(&h);
    h.click_text("Add Spell…");
    h.frames(2);
    let title = h.app.lang.tr_fmt("Add {0}", &[&crate::view::kind_noun(&h.app.lang, "Spell")]);
    assert!(h.find_text(&title).is_some(), "{title}; shown: {:?}", h.on_screen());
    // Escape closes it.
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(2);
    assert!(h.find_text(&title).is_none(), "{:?}", h.on_screen());
    h.click_text("Add Spell…");
    h.frames(2);
    h.click_text(&h.app.lang.tr("Search"));
    h.type_text("Stunbolt");
    h.frames(2);
    h.click_text("Stunbolt");
    h.frames(2);
    h.click_text(&h.app.lang.tr("Add"));
    h.frames(2);
    assert!(h.find_text(&h.app.lang.tr("Spell Options")).is_some(), "{:?}", h.on_screen());
    sizes(&mut h, 2);
    h.size = WIDE;
    h.frames(2);
    h.click_text(&h.app.lang.tr("Add"));
    h.frames(2);
    assert_eq!(spells(&h), before + 1, "{:?}", h.app.status);
}

/// Classic: the "Add Gear…" button opens the selection dialog; search,
/// pick a row and Add.
#[test]
fn classic_selection_dialog_add() {
    let mut h = Harness::new(ThemeKind::Graphite);
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    let before = gear_count(&h);
    let add = h.app.lang.tr("Add {0}…").replace("{0}", "");
    h.click_where("Add Gear…", |t| t.contains(add.trim_end_matches('…')) && t.to_lowercase().contains("gear"));
    h.frames(2);
    let search = h.app.lang.tr("Search");
    h.click_text(&search);
    h.type_text("Flashlight");
    h.frames(2);
    h.click_where("the Flashlight row", |t| t.starts_with("Flashlight ") && !t.contains(','));
    h.frames(1);
    h.click_text(&h.app.lang.tr("Add"));
    h.frames(2);
    assert_eq!(gear_count(&h), before + 1, "added ({:?}); shown: {:?}", h.app.status, h.on_screen());
    h.key(Key::Z, Modifiers::COMMAND);
    h.frames(2);
    assert_eq!(gear_count(&h), before);
}

/// Classic: clicking each tab of the tab strip shows it.
#[test]
fn classic_tab_strip_clicks() {
    let mut h = Harness::new(ThemeKind::Classic);
    for f in ["Munin_Career.chum5", "Apex Predator.chum5"] {
        let i = h.open(&fixture(f));
        h.frames(2);
        for (t, label) in TABS {
            if !h.app.views[i].visible(*t) {
                continue;
            }
            let label = h.app.lang.tr(label);
            // A tab's caption may carry a badge or count after it.
            h.click_where(&label, |s| s.trim() == label || s.starts_with(&format!("{label} ")));
            h.frames(1);
            assert_eq!(h.app.views[i].ws_tab(), *t, "clicked {label}");
        }
        h.close_all();
    }
}

/// Closing a changed character asks first; Cancel keeps it, Don't save
/// closes it. In both layouts, with Ctrl+W.
#[test]
fn close_changed_document_asks_first() {
    use chummer_core::command::Command;
    for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceDark] {
        let mut h = Harness::new(kind);
        let path = fixture("Munin.chum5");
        let original = std::fs::read(&path).unwrap();
        h.open(&path);
        h.open(&fixture("Soma.chum5"));
        h.app.select(crate::Mdi::Character(0));
        h.frames(1);
        h.app.views[0].doc_mut().apply(Command::SetField { key: "alias".into(), value: "Ghost".into() }).unwrap();
        h.frames(2);
        assert!(h.app.views[0].ch().dirty);
        h.key(Key::W, Modifiers::COMMAND);
        h.frames(1);
        assert!(matches!(h.app.pending, Some(crate::Pending::CloseTab(0))), "Ctrl+W asks first");
        h.click_text(&h.app.lang.tr("Cancel"));
        h.frames(1);
        assert!(h.app.pending.is_none());
        assert_eq!(h.app.views.len(), 2);
        h.key(Key::W, Modifiers::COMMAND);
        h.frames(1);
        h.click_text(&h.app.lang.tr("Don't save"));
        h.frames(2);
        assert_eq!(h.app.views.len(), 1, "closed");
        assert_eq!(std::fs::read(&path).unwrap(), original, "not saved");
        // Close the last one too (unchanged: no question).
        h.key(Key::W, Modifiers::COMMAND);
        h.frames(2);
        assert!(h.app.views.is_empty());
        assert_eq!(h.app.home, Some(Home::Roster));
        sizes(&mut h, 2);
    }
}

/// Draws frames until character `i` is saved (not modified) or 30 s
/// passed.
fn wait_saved(h: &mut Harness, i: usize) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while h.app.views[i].ch().dirty && std::time::Instant::now() < end {
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!h.app.views[i].ch().dirty, "saved: {:?}", h.app.status);
}

/// A Chummer5a file's first Save asks whether to save as .chumrs or keep
/// the .chum5; keeping it saves the .chum5 and does not ask again. A
/// .chumrs saves with its history: the earlier sessions' changes and this
/// one's, which the History panel lists.
#[test]
fn chumrs_and_the_first_save_of_a_chum5() {
    use chummer_core::chumrs::{self, Extras, HistoryItem};
    use chummer_core::command::Command;
    let mut h = Harness::new(ThemeKind::Graphite);
    let i = h.open(&fixture("Munin.chum5"));
    let path = h.app.views[i].path().unwrap();
    h.app.views[i].doc_mut().apply(Command::SetField { key: "alias".into(), value: "Ghost".into() }).unwrap();
    h.frames(1);
    h.key(Key::S, Modifiers::COMMAND);
    h.frames(2);
    assert!(h.app.io.ask_format.is_some(), "the first Save asks");
    assert!(h.find_text(&h.app.lang.tr("Save as .chumrs…")).is_some());
    h.click_text(&h.app.lang.tr("Keep saving as Chummer5a"));
    wait_saved(&mut h, i);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("<?xml") && text.contains("<alias>Ghost</alias>"), "saved as .chum5");
    h.app.views[i].doc_mut().apply(Command::SetField { key: "alias".into(), value: "Ghost 2".into() }).unwrap();
    h.key(Key::S, Modifiers::COMMAND);
    h.frames(1);
    assert!(h.app.io.ask_format.is_none(), "asked once");
    wait_saved(&mut h, i);
    assert!(std::fs::read_to_string(&path).unwrap().contains("<alias>Ghost 2</alias>"));
    h.close_all();

    // A .chumrs with history from an earlier session.
    let ch = chummer_core::character::Character::load(&fixture("Munin.chum5")).unwrap();
    let rs = scratch().join(format!("history-{:?}.chumrs", std::thread::current().id()).replace(['(', ')'], ""));
    let earlier = vec![HistoryItem { at: 1_700_000_000_000, author: String::new(), description: "Raised Pistols to 5".into() }];
    chumrs::write(&rs, &ch, &Extras { history: earlier.clone(), ..Default::default() }).unwrap();
    let before = h.app.views.len();
    h.app.open(&rs);
    h.wait_loaded(before + 1);
    let i = h.app.views.len() - 1;
    assert_eq!(h.app.views[i].doc().earlier_history(), &earlier[..]);
    h.app.views[i].doc_mut().apply(Command::SetField { key: "alias".into(), value: "Ghost".into() }).unwrap();
    h.app.views[i].show_history();
    h.frames(2);
    assert!(h.find_text(&h.app.lang.tr("Earlier sessions")).is_some(), "shown: {:?}", h.on_screen());
    assert!(h.find_text("Raised Pistols to 5").is_some());
    h.key(Key::S, Modifiers::COMMAND);
    h.frames(1);
    assert!(h.app.io.ask_format.is_none(), "a .chumrs saves without asking");
    wait_saved(&mut h, i);
    let (back, extras) = chumrs::load_any(&rs).unwrap();
    assert_eq!(back.field("alias"), "Ghost");
    assert_eq!(extras.history.len(), 2, "{:?}", extras.history);
    assert_eq!(extras.history[0], earlier[0]);
    h.close_all();
    let _ = std::fs::remove_file(&rs);
}

/// Several characters open: switch between them, close the one in
/// front and the ones behind it.
#[test]
fn switching_and_closing_documents() {
    for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceLight] {
        let mut h = Harness::new(kind);
        for f in ["Munin.chum5", "Munin_Career.chum5", "Rez0luti0n2.0.chum5", "fixer-chummer.chum5lz"] {
            h.open(&fixture(f));
            h.frames(1);
        }
        // Opening an open file brings it to the front instead.
        h.app.open(&own_copy(&fixture("Munin.chum5")));
        assert_eq!((h.app.views.len(), h.app.active), (4, 0));
        for k in [3, 1, 2, 0] {
            h.app.select(crate::Mdi::Character(k));
            h.frames(2);
        }
        h.app.select(crate::Mdi::Home(Home::MasterIndex));
        h.frames(2);
        h.app.select(crate::Mdi::Character(3));
        h.frames(1);
        // Close the front tab (the last), then one behind the front.
        h.app.close_tab(3, false);
        h.frames(2);
        h.app.close_tab(0, false);
        h.frames(2);
        assert_eq!(h.app.views.len(), 2);
        assert!(h.app.active < 2);
        h.close_all();
        h.frames(2);
    }
}

/// The harness itself: a panic in a frame is caught and named.
#[test]
fn harness_reports_panics() {
    scratch();
    let r = guarded(|| {
        let v: Vec<u8> = Vec::new();
        v[std::hint::black_box(3)]
    });
    let e = r.unwrap_err();
    assert!(e.contains("index out of bounds") && e.contains("ui_tests.rs"), "{e}");
}

/// File → New Character with each rules preset: pick it in the wizard,
/// Create, then draw every page of the new (empty) character in both
/// layouts.
#[test]
fn new_character_from_each_preset() {
    let mut h = Harness::new(ThemeKind::Graphite);
    let presets: Vec<(String, String)> = chummer_core::chargen::creation_presets(&h.app.engine).iter().map(|p| (p.name(), format!("{} ({})", p.name(), p.build_method()))).collect();
    assert!(!presets.is_empty());
    let mut failures = Vec::new();
    for (k, (name, item)) in presets.iter().enumerate() {
        // By default: the first few and those with unusual priorities.
        let unusual = ["Street Scum", "High Life", "Improved"].iter().any(|u| name.contains(u));
        if !full() && k > 3 && !unusual {
            continue;
        }
        for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceDark] {
            let r = guarded(|| {
                h.set_kind(kind);
                h.size = WIDE;
                h.key(Key::N, Modifiers::COMMAND);
                h.frames(2);
                assert!(h.app.wizard.is_some(), "Ctrl+N opens the wizard");
                // The preset combo shows the current preset's name; open
                // it, type the preset's name and take the first match.
                let current = presets[0].0.clone();
                h.click_text(&current);
                h.frames(2);
                h.type_text(item);
                h.frames(1);
                h.key(Key::Enter, Modifiers::NONE);
                h.frames(2);
                assert!(h.find_text(&presets[k].0).is_some(), "picked {item}; shown: {:?}", h.on_screen());
                h.click_text(&h.app.lang.tr("Create character"));
                h.frames(2);
                // Every preset's default priorities are valid: Create works.
                assert!(h.app.wizard.is_none(), "preset {item}: Create did nothing; shown: {:?}", h.on_screen());
                let i = h.app.active;
                let passes: &'static [(Vec2, ThemeKind)] = if kind == ThemeKind::Graphite { &[(WIDE, ThemeKind::Graphite)] } else { &[(WIDE, ThemeKind::WorkspaceDark)] };
                for s in pages(&h, i) {
                    draw_page(&mut h, i, s, Matrix { passes });
                }
                h.close_all();
                h.frames(1);
            });
            if let Err(e) = r {
                failures.push(format!("preset {item} [{:?}]: {e}", kind.layout()));
                h = Harness::new(ThemeKind::Graphite);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Frame times of the heaviest pages, for the record (debug builds are
/// much slower than release; compare fixtures, not absolute numbers).
/// Run with `cargo test -p chummer-gui frame_times -- --ignored --nocapture`.
#[test]
#[ignore = "timing report, run by hand"]
fn frame_times() {
    for (f, kind) in [("Munin.chum5", ThemeKind::WorkspaceDark), ("Munin_Career.chum5", ThemeKind::WorkspaceDark), ("Munin_Career.chum5", ThemeKind::Graphite)] {
        let mut h = Harness::new(kind);
        let i = h.open(&fixture(f));
        for s in [Section::Page(Tab::Limits), Section::Page(Tab::Skills), Section::Page(Tab::Common), Section::Gear(0)] {
            h.app.views[i].ws_go(s);
            h.frames(2);
            let t = std::time::Instant::now();
            h.frames(3);
            eprintln!("{f} [{:?}] {}: {:?} per frame", kind.layout(), s.label(), t.elapsed() / 3);
        }
        if kind.layout() == Layout::Workspace {
            h.app.views[i].ws_go(Section::Page(Tab::Limits));
            h.key(Key::K, Modifiers::COMMAND);
            let t = std::time::Instant::now();
            h.frames(3);
            eprintln!("{f} [Workspace] Limits with the palette open: {:?} per frame", t.elapsed() / 3);
        }
    }
    // Every page of the career characters, slowest first.
    let mut all = Vec::new();
    for f in ["Munin_Career.chum5", "Soma (Career).chum5", "Apex Predator.chum5"] {
        let mut h = Harness::new(ThemeKind::WorkspaceDark);
        let i = h.open(&fixture(f));
        let secs: Vec<Section> = h.app.views[i].ws_nav(&h.app.lang).into_iter().flat_map(|g| g.items.into_iter().map(|it| it.section)).collect();
        for s in secs {
            if !matches!(s, Section::Page(_) | Section::Gear(_)) {
                continue;
            }
            h.app.views[i].ws_go(s);
            h.frames(2);
            let t = std::time::Instant::now();
            h.frames(3);
            all.push((t.elapsed() / 3, format!("{f} {}", s.label())));
        }
    }
    all.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (d, what) in all.iter().take(8) {
        eprintln!("slowest: {what}: {d:?} per frame");
    }
}

// ----- monkey: random clicks on every page -----

/// xorshift64*, seeded per (fixture, layout, page) so a failure repeats.
struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Rng {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in seed.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        Rng(h | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n.max(1)
    }
}

/// Texts the monkey never clicks: file dialogs, saving, printing,
/// opening files or URLs, going online.
const NO_CLICK: &[&str] = &["open", "save", "export", "print", "sheet", "pdf", "folder", "browse", "import", "link", "file", "sourcebook", "exit", "quit", "join", "host", "invite", "online", "relay", "http", "www.", "github", "settings…", "load", " p. "];

fn clickable(t: &str) -> bool {
    let t = t.trim();
    let lower = t.to_lowercase();
    // Icon-only buttons are skipped: their meaning is not readable here.
    !t.is_empty() && t.chars().any(|c| !('\u{e000}'..='\u{f8ff}').contains(&c)) && !NO_CLICK.iter().any(|w| lower.contains(w))
}

/// Where the page is (not the menus, top bar, sidebar or status bar).
fn page_region(h: &Harness) -> Rect {
    let s = h.size;
    match h.app.appearance.layout {
        Layout::Classic => Rect::from_min_max(Pos2::new(0.0, 70.0), Pos2::new(s.x, s.y - 28.0)),
        Layout::Workspace => Rect::from_min_max(Pos2::new(210.0, 100.0), Pos2::new(s.x, s.y - 30.0)),
    }
}

/// `steps` random clicks on texts in the page and a few keys; the
/// actions taken, for a failure report.
fn monkey(h: &mut Harness, steps: usize, rng: &mut Rng, log: &mut Vec<String>) {
    let keys = [Key::Enter, Key::Escape, Key::Tab, Key::ArrowDown, Key::Delete];
    for step in 0..steps {
        // Keep off career Skills pages (seconds per debug frame).
        if let Some(i) = h.app.current() {
            if h.app.views[i].ch().created && h.app.views[i].ws_tab() == Tab::Skills {
                h.app.views[i].ws_go(Section::Page(Tab::Limits));
            }
        }
        h.frame(Vec::new());
        if step % 5 == 4 {
            let k = keys[rng.below(keys.len())];
            log.push(format!("key {k:?}"));
            h.key(k, Modifiers::NONE);
            continue;
        }
        let region = page_region(h);
        let candidates: Vec<(String, Rect)> = h.texts.iter().filter(|(t, r)| clickable(t) && region.contains(r.center())).cloned().collect();
        if candidates.is_empty() {
            continue;
        }
        let (t, r) = &candidates[rng.below(candidates.len())];
        log.push(format!("click {t:?}"));
        h.click_at(r.center());
        if step % 3 == 2 {
            log.push("type 2".into());
            h.type_text("2");
        }
    }
    h.frames(1);
}

fn run_monkey(layout: Layout, fixtures: &[&str], steps: usize) {
    let t0 = std::time::Instant::now();
    let kind = if layout == Layout::Classic { ThemeKind::Graphite } else { ThemeKind::WorkspaceDark };
    let mut failures = Vec::new();
    for f in fixtures {
        let path = fixture(f);
        let mut h = Harness::new(kind);
        let i = h.open(&path);
        let career = h.app.views[i].ch().created;
        for (n, s) in pages(&h, i).into_iter().enumerate() {
            // Career Skills pages take seconds per debug frame.
            if career && s == Section::Page(Tab::Skills) {
                continue;
            }
            let mut log = Vec::new();
            let mut rng = Rng::new(&format!("{f}/{layout:?}/{}", s.label()));
            let r = guarded(|| {
                // Each page starts from the file, so a failure repeats
                // from its seed alone.
                h.reset();
                let i = h.open(&path);
                h.size = WIDE;
                draw_page(&mut h, i, s, Matrix { passes: &[] });
                // The full run clicks every other page at the narrow size.
                if full() && n % 2 == 1 {
                    h.size = NARROW;
                }
                monkey(&mut h, steps, &mut rng, &mut log);
            });
            if let Err(e) = r {
                failures.push(format!("{f} [{layout:?}] {} after {}: {e}", s.label(), log.join(", ")));
                h = Harness::new(kind);
            }
        }
    }
    eprintln!("{layout:?} monkey: {} fixtures in {:?}", fixtures.len(), t0.elapsed());
    assert!(failures.is_empty(), "{} page(s) panicked under random clicks:\n{}", failures.len(), failures.join("\n\n"));
}

const MONKEY_FIXTURES: &[&str] = &["Munin.chum5", "Apex Predator.chum5", "Rez0luti0n2.0.chum5", "Soma (Career).chum5"];

/// Fixture names of monkey shard `shard`.
fn monkey_fixtures(shard: usize) -> Vec<String> {
    let all: Vec<String> = if full() { all_fixtures().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect() } else { MONKEY_FIXTURES.iter().map(|s| s.to_string()).collect() };
    all.into_iter().enumerate().filter(|(n, _)| n % SHARDS == shard).map(|(_, f)| f).collect()
}

fn monkey_shard(layout: Layout, shard: usize) {
    let f = monkey_fixtures(shard);
    run_monkey(layout, &f.iter().map(String::as_str).collect::<Vec<_>>(), if full() { 30 } else { 10 });
}

#[test]
fn classic_random_clicks_0() {
    monkey_shard(Layout::Classic, 0);
}
#[test]
fn classic_random_clicks_1() {
    monkey_shard(Layout::Classic, 1);
}
#[test]
fn classic_random_clicks_2() {
    monkey_shard(Layout::Classic, 2);
}
#[test]
fn classic_random_clicks_3() {
    monkey_shard(Layout::Classic, 3);
}
#[test]
fn workspace_random_clicks_0() {
    monkey_shard(Layout::Workspace, 0);
}
#[test]
fn workspace_random_clicks_1() {
    monkey_shard(Layout::Workspace, 1);
}
#[test]
fn workspace_random_clicks_2() {
    monkey_shard(Layout::Workspace, 2);
}
#[test]
fn workspace_random_clicks_3() {
    monkey_shard(Layout::Workspace, 3);
}

/// Random clicks on the Master Index and on a GM screen with members
/// in an encounter, in both layouts.
#[test]
fn home_and_campaign_random_clicks() {
    let steps = if full() { 120 } else { 30 };
    let mut failures = Vec::new();
    for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceDark] {
        let mut h = Harness::new(kind);
        for what in ["Master Index", "GM screen"] {
            let mut log = Vec::new();
            let mut rng = Rng::new(&format!("{what}/{kind:?}"));
            let r = guarded(|| {
                h.reset();
                h.size = WIDE;
                if what == "Master Index" {
                    h.app.home = Some(Home::MasterIndex);
                } else {
                    h.app.open_campaign(&campaign_file());
                    let App { gm, views, .. } = &mut h.app;
                    let gm = gm.as_mut().expect("campaign opened");
                    gm.add_all_players(views);
                    gm.roll_initiative(views);
                }
                h.frames(2);
                monkey(&mut h, steps, &mut rng, &mut log);
                if h.app.gm.is_some() {
                    h.app.close_campaign(true);
                    h.frames(1);
                }
            });
            if let Err(e) = r {
                failures.push(format!("{what} [{:?}] after {}: {e}", kind.layout(), log.join(", ")));
                h = Harness::new(kind);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Windows far below the minimum size the app asks for (tiling window
/// managers may not honour it): every page, the palette and the tool
/// windows still draw.
#[test]
fn tiny_windows() {
    const TINY: &[(Vec2, ThemeKind)] = &[(Vec2::new(420.0, 300.0), ThemeKind::Graphite), (Vec2::new(120.0, 90.0), ThemeKind::Classic)];
    const TINY_WS: &[(Vec2, ThemeKind)] = &[(Vec2::new(420.0, 300.0), ThemeKind::WorkspaceDark), (Vec2::new(120.0, 90.0), ThemeKind::WorkspaceLight)];
    let mut failures = Vec::new();
    for (kind, passes) in [(ThemeKind::Graphite, TINY), (ThemeKind::WorkspaceDark, TINY_WS)] {
        for f in ["Munin.chum5", "Soma (Career).chum5"] {
            let mut h = Harness::new(kind);
            let i = h.open(&fixture(f));
            for s in pages(&h, i) {
                if h.app.views[i].ch().created && s == Section::Page(Tab::Skills) {
                    continue;
                }
                let r = guarded(|| draw_page(&mut h, i, s, Matrix { passes }));
                if let Err(e) = r {
                    failures.push(format!("{f} [{:?}] {}: {e}", kind.layout(), s.label()));
                    h = Harness::new(kind);
                    h.open(&fixture(f));
                }
            }
            let r = guarded(|| {
                for &(size, _) in passes {
                    h.size = size;
                    let a = &mut h.app;
                    (a.show_dice, a.show_initiative, a.show_print, a.show_export, a.show_about) = (true, true, true, true, true);
                    h.frames(2);
                    h.app.wizard = Some(crate::wizard::Wizard::new());
                    h.frames(2);
                    h.reset();
                    if kind.layout() == Layout::Workspace {
                        h.open(&fixture(f));
                        h.key(Key::K, Modifiers::COMMAND);
                        h.type_text("ar");
                        h.frames(2);
                        h.key(Key::Escape, Modifiers::NONE);
                    }
                }
            });
            if let Err(e) = r {
                failures.push(format!("{f} [{:?}] windows: {e}", kind.layout()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}


/// GUI BUG: Workspace → Relationships in a window about 320 px tall or less
/// panics in debug builds: `workspace/story.rs` (`ws_relationships`)
/// calls `ui.set_min_height(available_height - 26.0)`, negative once the
/// page has under 26 px, and egui's `Ui::set_min_height` debug-asserts
/// "Negative height makes no sense". Release builds don't panic. The app
/// asks for a 500 px minimum, which tiling window managers may ignore.
/// Fix: `.max(0.0)`.
#[test]
fn workspace_relationships_short_window() {
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    let i = h.open(&fixture("Munin.chum5"));
    h.app.views[i].ws_go(Section::Page(Tab::Relationships));
    h.size = Vec2::new(1600.0, 300.0);
    h.frames(3);
}

/// Creation, priority build: click two priority cards to swap them
/// (Workspace), undo it; "Change Priority Selection…" opens the dialog
/// in both layouts, and a new metatype there is applied.
#[test]
fn priority_swap_and_change_metatype() {
    for kind in [ThemeKind::WorkspaceLight, ThemeKind::Graphite] {
        let mut h = Harness::new(kind);
        let i = h.open(&fixture("Miko.chum5"));
        h.app.views[i].ws_go(Section::Page(Tab::Common));
        h.size = WIDE;
        h.frames(3);
        let letters = |h: &Harness| {
            let d = h.app.views[i].doc();
            (d.field("priorityattributes").chars().next(), d.field("priorityskills").chars().next())
        };
        if kind == ThemeKind::WorkspaceLight {
            assert_eq!(letters(&h), (Some('B'), Some('C')));
            // The cards' second lines: "20 points", "28 / 2 groups".
            h.click_where("the Attributes card", |t| t.ends_with(" points") && t.chars().next().is_some_and(|c| c.is_ascii_digit()));
            h.frames(2);
            assert!(h.find_text("click another to swap them (Esc: cancel)").is_some(), "{:?}", h.on_screen());
            h.click_where("the Skills card", |t| t.contains(" / ") && t.ends_with(" groups"));
            h.frames(3);
            assert_eq!(letters(&h), (Some('C'), Some('B')), "{:?}", h.app.status);
            h.key(Key::Z, Modifiers::COMMAND);
            h.frames(2);
            assert_eq!(letters(&h), (Some('B'), Some('C')), "one undo step");
        }
        let label = crate::metatype_ui::change_label(&h.app.lang, "Priority");
        h.click_text(&label);
        h.frames(3);
        let title = label.trim_end_matches('…').to_owned();
        assert!(h.find_text(&title).is_some(), "{title}: {:?}", h.on_screen());
        sizes(&mut h, 2);
        h.size = WIDE;
        h.frames(2);
        // Human instead of Elf (both at Heritage D), then OK.
        h.click_where("Human", |t| t == "Human" || t.starts_with("Human "));
        h.frames(2);
        h.click_text(&h.app.lang.tr("OK"));
        h.frames(3);
        assert_eq!(h.app.views[i].doc().field("metatype"), "Human", "{:?}; {:?}", h.app.status, h.on_screen());
        assert!(h.find_text(&title).is_none(), "the dialog closed");
        h.key(Key::Z, Modifiers::COMMAND);
        h.frames(2);
        assert_eq!(h.app.views[i].doc().field("metatype"), "Elf");
    }
}

/// Relationships: Sort → By Name (Workspace) and By Connection (Classic)
/// reorder the contacts with one undoable command.
#[test]
fn relationships_sort() {
    for kind in [ThemeKind::WorkspaceDark, ThemeKind::Graphite] {
        let mut h = Harness::new(kind);
        let i = h.open(&fixture("Munin.chum5"));
        h.app.views[i].ws_go(Section::Page(Tab::Relationships));
        h.size = WIDE;
        h.frames(3);
        let names = |h: &Harness| h.app.views[i].doc().items("contacts", "contact").iter().filter(|c| c.get("type") != "Enemy" && c.get("type") != "Pet").map(|c| (c.get("name").to_lowercase(), c.get_i32("connection").unwrap_or(0))).collect::<Vec<_>>();
        let before = names(&h);
        h.click_text("Sort");
        h.frames(2);
        let by = if kind == ThemeKind::Graphite { "By Connection" } else { "By Name" };
        h.click_text(by);
        h.frames(2);
        let after = names(&h);
        let mut want = before.clone();
        if kind == ThemeKind::Graphite {
            want.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        } else {
            want.sort_by(|a, b| (a.0.is_empty(), &a.0).cmp(&(b.0.is_empty(), &b.0)));
        }
        assert_eq!(after, want, "{:?}", h.app.status);
        if after != before {
            h.key(Key::Z, Modifiers::COMMAND);
            h.frames(2);
            assert_eq!(names(&h), before, "one undo step");
        }
    }
}

/// The command line: `--tab`, `--layout` and `--theme` names, and
/// starting with files on a tab.
#[test]
fn command_line_tab_layout_theme() {
    let names = "common|skills|limits|martial|spells|adept|complex|critter|initiation|cyberware|street|vehicles|character|karma|calendar|game|improvements|relationships";
    for n in names.split('|') {
        assert!(Tab::parse(n).is_some(), "--tab {n}");
    }
    // Full labels parse too, with or without their "&".
    for (t, label) in TABS {
        assert_eq!(Tab::parse(label), Some(*t), "--tab {label:?}");
        let arg = label.replace('&', "");
        assert_eq!(Tab::parse(&arg), Some(*t), "--tab {arg:?}");
    }
    for l in Layout::ALL {
        assert_eq!(Layout::parse(l.as_str()), Some(l));
    }
    for k in ThemeKind::ALL {
        assert_eq!(ThemeKind::parse(k.as_str()), Some(k));
    }
    assert_eq!(ThemeKind::parse("workspace-dark"), Some(ThemeKind::WorkspaceDark));
    scratch();
    for kind in [ThemeKind::Graphite, ThemeKind::WorkspaceDark] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let files = vec![own_copy(&fixture("Munin.chum5")), own_copy(&fixture("fixer-chummer.chum5lz"))];
        let app = App::new(&cc, Engine::load().unwrap(), files, Tab::parse("skills"), (Some(kind), None), None);
        let mut h = Harness { ctx, app, size: WIDE, time: 0.0, texts: Vec::new() };
        h.wait_loaded(2);
        assert_eq!(h.app.appearance.layout, kind.layout(), "--theme picks its layout");
        assert_eq!(h.app.views.len(), 2);
        h.frames(2);
        assert!(h.app.views.iter().all(|v| v.ws_tab() == Tab::Skills));
        assert_eq!(h.app.current(), Some(1), "the last file is in front");
    }
}

/// GUI BUG: closing a tab *before* the one in front changes which
/// character is in front. `App::close_tab` (main.rs) removes the view
/// but only clamps `active` when it runs past the end: with tabs A B C
/// and B in front, closing A leaves `active` = 1, which is now C.
#[test]
fn closing_a_background_tab_keeps_the_front_one() {
    let mut h = Harness::new(ThemeKind::Graphite);
    for f in ["Munin.chum5", "Soma.chum5", "Rez0luti0n2.0.chum5"] {
        h.open(&fixture(f));
    }
    h.app.select(crate::Mdi::Character(1));
    h.frames(1);
    let front = h.app.views[1].ws_id();
    h.app.close_tab(0, false);
    h.frames(1);
    assert_eq!(h.app.views[h.app.active].ws_id(), front, "the character in front stays in front");
}

/// Closing a campaign drops its member tabs; the plain character in front
/// stays in front, and closing a member tab before it does not move it.
#[test]
fn closing_member_tabs_and_the_campaign_keeps_the_front_one() {
    let mut h = Harness::new(ThemeKind::Graphite);
    h.app.open_campaign(&campaign_file());
    let ids: Vec<_> = h.app.gm.as_ref().unwrap().campaign.members.iter().map(|m| m.id).take(2).collect();
    h.app.open_member(ids[0]);
    h.open(&fixture("Munin.chum5"));
    h.app.open_member(ids[1]);
    h.open(&fixture("Soma.chum5"));
    // Tabs: member0, Munin, member1, Soma. Munin in front.
    h.app.select(crate::Mdi::Character(1));
    h.frames(1);
    let munin = h.app.views[1].ws_id();
    h.app.close_tab(0, false);
    assert_eq!(h.app.views[h.app.active].ws_id(), munin, "closing a member tab before it");
    h.app.home = None;
    h.frames(1);
    assert!(h.app.close_campaign(true));
    assert_eq!(h.app.views.len(), 2);
    assert_eq!(h.app.views[h.app.active].ws_id(), munin, "closing the campaign");
    h.frames(1);
}
