//! GUI frame times for the performance suite (docs/testing.md,
//! "Performance tests"): every page of the heavy character in both
//! layouts (steady frames, and the first frame after switching to the
//! page), the catalog with a search typed, the GM screen with a campaign
//! of six members, and the item tables of a large synthetic character
//! (`chummer_bench::synth`). Headless, as the other UI tests.
//!
//! Each frame is timed on its own (`Context::run` plus tessellation, as
//! in [`Harness::frame`]; no GPU), after a few warm-up frames, and a page
//! reports the median, mean and p95 of its frames. Ignored in the normal
//! run; `scripts/perf.sh` runs it in release:
//!
//!     cargo test --release -p chummer-gui perf_frames -- --ignored
//!
//! `CHUMMER_PERF_OUT` is the JSON file to write (else
//! `target/perf/gui.json`), `CHUMMER_PERF_QUICK=1` draws fewer frames,
//! `CHUMMER_PERF_ONLY` takes name prefixes as `chummer-bench run --only`.

use std::path::PathBuf;
use std::time::Instant;

use chummer_bench::{Record, Runner};
use eframe::egui::{Key, Modifiers};

use super::{catalog_search, fixture, go_special, pages, scratch, Harness, WIDE};
use crate::theme::{Layout, ThemeKind};
use crate::view::Tab;
use crate::workspace::Section;

/// The character with the most items (as chummer-bench's `HEAVY`).
const HEAVY: &str = "Serpent.chum5";

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_owned()
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::Workspace => "workspace",
        Layout::Classic => "classic",
    }
}

/// Times `n` frames one by one, after `warm` untimed ones.
fn timed_frames(h: &mut Harness, warm: usize, n: usize) -> Vec<f64> {
    h.frames(warm);
    (0..n)
        .map(|_| {
            let t = Instant::now();
            h.frame(Vec::new());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

fn go(h: &mut Harness, i: usize, s: Section) {
    match s {
        Section::Play | Section::History => go_special(h, s),
        _ => h.app.views[i].ws_go(s),
    }
}

/// A campaign of three players and three NPCs, from the biggest fixtures.
fn big_campaign() -> PathBuf {
    use chummer_core::campaign::{Campaign, Encounter, Member, MemberKind};
    use chummer_core::character::Character;
    let path = scratch().join("perf.chummercampaign");
    let mut c = Campaign::new("Perf");
    c.encounters.push(Encounter::new("Encounter 1"));
    let members = [("Serpent", MemberKind::Player), ("Munin_Career", MemberKind::Player), ("Soma (Career)", MemberKind::Player), ("Ghile Mear", MemberKind::Npc), ("Barrett", MemberKind::Npc), ("Apex Predator", MemberKind::Npc)];
    for (f, kind) in members {
        let ch = Character::load(&fixture(&format!("{f}.chum5"))).unwrap();
        c.add(Member::embedded(kind, &ch));
    }
    c.save(&path).unwrap();
    path
}

#[test]
#[ignore = "performance suite (scripts/perf.sh), run in release"]
fn perf_frames() {
    let quick = std::env::var("CHUMMER_PERF_QUICK").is_ok_and(|v| !v.is_empty() && v != "0");
    let only: Vec<String> = std::env::var("CHUMMER_PERF_ONLY").unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "all").map(str::to_owned).collect();
    let out = std::env::var_os("CHUMMER_PERF_OUT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/perf/gui.json"));
    let (warm, n) = if quick { (2, 5) } else { (5, 30) };
    // The frame cap (frame_cap.rs) sleeps the rest of 16 ms between
    // frames; with "vsync" it leaves pacing to the driver, so frames run
    // back to back. Edition 2021: set_var is safe; std's lock covers it.
    std::env::set_var("CHUMMER_VSYNC", "1");
    // CHUMMER_TRACE_FRAMES=1 (and CHUMMER_TRACE_SLOW_MS) logs slow frames
    // with their spans, as in the app.
    crate::trace::init();
    let mut r = Runner::new(quick, only);
    let heavy = slug(HEAVY.trim_end_matches(".chum5"));
    for kind in [ThemeKind::WorkspaceDark, ThemeKind::Graphite] {
        let layout = kind.layout();
        let ln = layout_name(layout);
        let prefix = format!("gui.frame.{ln}.");
        let switch_prefix = format!("gui.switch.{ln}.");
        if !(r.wants(&prefix) || r.wants(&switch_prefix) || (layout == Layout::Workspace && r.wants("gui.catalog."))) {
            continue;
        }
        let mut h = Harness::new(kind);
        h.size = WIDE;
        let i = h.open(&fixture(HEAVY));
        h.frames(2);
        let (mut all, mut first) = (Vec::new(), Vec::new());
        let switches = if quick { 2 } else { 5 };
        for s in pages(&h, i) {
            let page = slug(s.label());
            let (name, switch) = (format!("{prefix}{heavy}.{page}"), format!("{switch_prefix}{heavy}.{page}"));
            if !r.wants(&name) && !r.wants(&switch) {
                continue;
            }
            h.size = WIDE;
            // Play and History are reached by clicks over several frames:
            // no switch times for them.
            let direct = !matches!(s, Section::Play | Section::History);
            // The first visit (caches being built), then switches from a
            // light page: the frame a click on the page's tab costs.
            let t = Instant::now();
            go(&mut h, i, s);
            h.frame(Vec::new());
            if direct {
                first.push(t.elapsed().as_secs_f64() * 1000.0);
                let rest = if s == Section::Page(Tab::Limits) { Section::Page(Tab::Common) } else { Section::Page(Tab::Limits) };
                let mut samples = Vec::new();
                for _ in 0..switches {
                    go(&mut h, i, rest);
                    h.frames(2);
                    let t = Instant::now();
                    go(&mut h, i, s);
                    h.frame(Vec::new());
                    samples.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                r.push(Record::from_samples(&switch, "ms", samples));
            }
            if r.wants(&name) {
                let samples = timed_frames(&mut h, warm, n);
                all.extend_from_slice(&samples);
                r.push(Record::from_samples(&name, "ms", samples));
            }
        }
        if !all.is_empty() {
            r.push(Record::from_samples(&format!("{prefix}{heavy}.all_pages"), "ms", all).with_note("every frame of every page above"));
        }
        if !first.is_empty() {
            r.push(Record::from_samples(&format!("{switch_prefix}{heavy}.first_visits"), "ms", first).with_note("the first frame of each page on its first visit, one sample per page (max = the worst page)"));
        }
        if layout == Layout::Workspace && r.wants("gui.catalog.") {
            catalog(&mut r, &mut h, i, &heavy, warm, n);
        }
        // The GM screen with six members in the encounter, initiative rolled.
        let name = format!("{prefix}gm_screen");
        if r.wants(&name) {
            h.close_all();
            h.app.open_campaign(&big_campaign());
            assert!(h.app.gm.is_some(), "{:?}", h.app.status);
            {
                let crate::App { gm, views, .. } = &mut h.app;
                let gm = gm.as_mut().unwrap();
                gm.add_all_players(views);
                gm.roll_initiative(views);
            }
            let samples = timed_frames(&mut h, warm, n);
            r.push(Record::from_samples(&name, "ms", samples).with_note("a campaign of 3 players and 3 NPCs (the biggest fixtures), initiative rolled"));
            assert!(h.app.close_campaign(true));
        }
    }
    if r.wants("gui.large.") {
        large(&mut r, warm, n);
    }
    r.report.write(&out).expect("write the report");
    eprintln!("wrote {}", out.display());
}

/// Workspace: the gear page with its catalog: frames with a search
/// shown, and the frame that takes a typed search (filtering).
fn catalog(r: &mut Runner, h: &mut Harness, i: usize, heavy: &str, warm: usize, n: usize) {
    h.app.views[i].ws_go(Section::Gear(0));
    h.frames(2);
    if h.find_text(&h.app.lang.tr("Catalog")).is_none() {
        let add = h.app.lang.tr_fmt("Add {0}", &[&crate::view::kind_noun(&h.app.lang, "Gear")]);
        h.click_text(&add);
        h.frames(2);
    }
    catalog_search(h, i, "Medkit");
    let samples = timed_frames(h, warm, n);
    r.push(Record::from_samples(&format!("gui.catalog.{heavy}.frame_with_search"), "ms", samples).with_note("Workspace gear page with the catalog open, \"Medkit\" typed"));
    let mut typed = Vec::new();
    for k in 0..n.min(15) {
        h.ctx.memory_mut(|m| m.request_focus(eframe::egui::Id::new("ws_catalog_search")));
        h.frame(Vec::new());
        h.key(Key::A, Modifiers::COMMAND);
        h.key(Key::Backspace, Modifiers::NONE);
        h.frames(1);
        let query = ["comm", "armor", "medkit", "ammo"][k % 4];
        let t = Instant::now();
        h.type_text(query);
        typed.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    r.push(Record::from_samples(&format!("gui.catalog.{heavy}.type_search"), "ms", typed).with_note("the frame in which a search is typed into the catalog (it filters the list)"));
    // Back to a plain page for what follows.
    h.app.views[i].ws_go(Section::Page(Tab::Common));
    h.frames(2);
}

/// The large synthetic character: building the Workspace Gear, Vehicles
/// and Cyberware tables cold (as after each edit), the first frame of
/// each page with its table to build, and steady frames.
fn large(r: &mut Runner, warm: usize, n: usize) {
    let engine = chummer_core::engine::Engine::load().expect("game data");
    let ch = chummer_core::character::Character::load(&fixture(HEAVY)).unwrap();
    let (mut ch, added) = chummer_bench::synth::large(&engine, ch);
    let path = scratch().join("perf-large.chum5");
    engine.save(&mut ch, &path).expect("save the large character");
    let mut h = Harness::new(ThemeKind::WorkspaceDark);
    h.size = WIDE;
    let i = h.open(&path);
    h.frames(2);
    let about = format!("Serpent with {added} items added (chummer_bench::synth)");
    let rows_n = if r.quick { 3 } else { 10 };
    for (tab, page) in [(Tab::StreetGear, "gear"), (Tab::Vehicles, "vehicles"), (Tab::Cyberware, "cyberware")] {
        let s = if tab == Tab::StreetGear { Section::Gear(0) } else { Section::Page(tab) };
        h.app.views[i].ws_go(s);
        h.frames(warm);
        let rows = format!("gui.large.rows.{page}");
        if r.wants(&rows) {
            let lang = h.app.lang.clone();
            let mut count = 0;
            let samples: Vec<f64> = (0..rows_n)
                .map(|_| {
                    let t = Instant::now();
                    count = h.app.views[i].ws_inventory_rows_cold(tab, &lang);
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            r.push(Record::from_samples(&rows, "ms", samples).with_note(format!("{about}: the Workspace table's {count} top rows built cold (ws_inventory_rows, as on a new revision)")));
        }
        let first = format!("gui.large.first_frame.{page}");
        if r.wants(&first) {
            let samples: Vec<f64> = (0..rows_n)
                .map(|_| {
                    h.app.views[i].ws_inventory_forget();
                    let t = Instant::now();
                    h.frame(Vec::new());
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            r.push(Record::from_samples(&first, "ms", samples).with_note(format!("{about}: a frame of the page whose table is rebuilt (as after an edit)")));
        }
        let frame = format!("gui.large.frame.{page}");
        if r.wants(&frame) {
            let samples = timed_frames(&mut h, warm, n);
            r.push(Record::from_samples(&frame, "ms", samples).with_note(about.clone()));
        }
    }
}
