//! The engine, character files, commands, saving and printing.

use std::time::Instant;

use anyhow::{Context, Result};
use chummer_bench::{ms, Record, Runner};
use chummer_core::calc;
use chummer_core::character::Character;
use chummer_core::chumrs::{self, Extras};
use chummer_core::command::{Command, RecordRef, Session};
use chummer_core::engine::Engine;
use chummer_core::items::Purchase;
use chummer_core::lang::Language;
use chummer_core::{data, print};

use crate::{child, child_record, engine, files_dir, files_in_memory, fixture, load, slug, work_dir, BIG, HEAVY};

/// A second career character (Serpent is one too): smaller, but its GUI
/// pages are among the slowest (ui_tests.rs `frame_times`).
const CAREER: &str = "Munin_Career";

pub fn run(r: &mut Runner) -> Result<()> {
    cold(r)?;
    let engine = engine()?;
    let files = files_dir();
    for name in [HEAVY, BIG] {
        let s = slug(name);
        let path = fixture(name);
        r.bench(&format!("core.load.chum5.{s}"), 10, || {
            Character::load(&path).expect("load");
        });
        let ch = load(name)?;
        let rs = files.join(format!("{s}.chumrs"));
        chumrs::write(&rs, &ch, &Extras::default())?;
        r.bench(&format!("core.load.chumrs.{s}"), 10, || {
            chumrs::load_any(&rs).expect("load .chumrs");
        });
        // The first compute builds the data stores; the samples are the
        // steady state the GUI sees after each edit.
        r.bench(&format!("core.compute.{s}"), 15, || {
            let _ = engine.sheet(&ch);
        });
    }
    large(r, &engine)?;
    commands(r, &engine)?;
    save(r, &engine)?;
    printing(r, &engine)?;
    memory(r)?;
    Ok(())
}

/// Engine start in a fresh process, and the first sheet after it.
fn cold(r: &mut Runner) -> Result<()> {
    if !r.wants("core.engine.") {
        return Ok(());
    }
    let heavy = fixture(HEAVY);
    let heavy = heavy.to_str().context("fixture path")?;
    child(&["cold", heavy])?;
    let runs: Vec<serde_json::Value> = (0..r.samples(7)).map(|_| child(&["cold", heavy])).collect::<Result<_>>()?;
    r.push(child_record("core.engine.load_cold", "engine_ms", "ms", &runs).with_note("Engine::load in a fresh process (disk cache warm)"));
    r.push(child_record(&format!("core.engine.first_sheet_cold.{}", slug(HEAVY)), "first_sheet_ms", "ms", &runs).with_note("fresh process: Engine::load, load the character, compute its sheet"));
    Ok(())
}

fn commands(r: &mut Runner, engine: &Engine) -> Result<()> {
    if !r.wants("core.command.") {
        return Ok(());
    }
    for name in [HEAVY, CAREER] {
        let s = slug(name);
        let mut base = load(name)?;
        base.karma = 10_000;
        base.nuyen = 10_000_000.0;
        // A fresh session per sample (not timed), so each sample does the
        // same work.
        let fresh = || Session::new(base.clone());
        let mut i = 0;
        r.bench_setup(&format!("core.command.set_field.{s}"), 20, fresh, |mut ses| {
            i += 1;
            ses.apply(engine, Command::SetField { key: "alias".into(), value: format!("Bench {i}") }).expect("SetField");
        });
        let medkit = || Command::AddItem { tag: "gear".into(), record: RecordRef { id: String::new(), name: "Medkit".into() }, purchase: Purchase { rating: 3, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } };
        r.bench_setup(&format!("core.command.add_gear.{s}"), 20, fresh, |mut ses| {
            ses.apply(engine, medkit()).expect("AddItem");
        });
        r.bench_setup(
            &format!("core.command.undo.{s}"),
            20,
            || {
                let mut ses = fresh();
                ses.apply(engine, medkit()).expect("AddItem");
                ses
            },
            |mut ses| {
                ses.undo().expect("undo");
            },
        );
        if base.created {
            let attr = "AGI".to_string();
            r.bench_setup(&format!("core.command.raise_attribute.{s}"), 20, fresh, |mut ses| {
                ses.apply(engine, Command::RaiseAttribute { attribute: attr.clone() }).expect("RaiseAttribute");
            });
            let skill = base.skills.iter().find(|k| k.base + k.karma > 0 && k.base + k.karma < 6).map(|k| k.guid.clone()).context("a skill to raise")?;
            r.bench_setup(&format!("core.command.raise_skill.{s}"), 20, fresh, |mut ses| {
                ses.apply(engine, Command::RaiseSkill { skill: skill.clone() }).expect("RaiseSkill");
            });
        }
    }
    Ok(())
}

/// The phases of a save (as tests/save_timing.rs), then whole saves.
fn save(r: &mut Runner, engine: &Engine) -> Result<()> {
    if !r.wants("core.save.") {
        return Ok(());
    }
    let files = files_dir();
    let s = slug(BIG);
    let ch = load(BIG)?;
    r.bench(&format!("core.save.clone.{s}"), 15, || {
        let _ = ch.clone();
    });
    let rules = engine.rules_for(&ch);
    let store = engine.store_for_character(&ch);
    let sheet = calc::compute(&ch, &rules, Some(&store), Some(&engine.catalog));
    r.bench_setup(&format!("core.save.stamp.{s}"), 15, || ch.clone(), |mut c| calc::stamp_totals(&mut c, &sheet, &rules));
    let mut stamped = ch.clone();
    calc::stamp_totals(&mut stamped, &sheet, &rules);
    r.bench(&format!("core.save.to_document.{s}"), 15, || {
        let _ = stamped.to_document();
    });
    let doc = stamped.to_document();
    r.bench(&format!("core.save.to_xml_string.{s}"), 15, || {
        let _ = doc.to_xml_string();
    });
    let text = doc.to_xml_string();
    let out = files.join("save.chum5");
    r.bench(&format!("core.save.write_chum5.{s}"), 15, || chummer_core::chum5lz::write_text(&out, &text).expect("write"));
    let lz = files.join("save.chum5lz");
    r.bench(&format!("core.save.write_chum5lz.{s}"), 7, || chummer_core::chum5lz::write_text(&lz, &text).expect("write lz"));
    r.bench(&format!("core.save.chumrs_bytes.{s}"), 15, || {
        let _ = chumrs::to_bytes(&stamped, &Extras::default());
    });
    // Whole saves, as the GUI does them (Engine::save on a copy).
    for name in [HEAVY, BIG] {
        let s = slug(name);
        let ch = load(name)?;
        for ext in ["chum5", "chumrs"] {
            let path = files.join(format!("{s}.saved.{ext}"));
            r.bench_setup(&format!("core.save.full_{ext}.{s}"), 10, || ch.clone(), |mut c| engine.save(&mut c, &path).expect("save"));
        }
    }
    let place = if files_in_memory() { "to a tmpfs (/dev/shm), so without the disk's sync latency" } else { "to the work directory (no tmpfs here)" };
    for b in r.report.benches.iter_mut().filter(|b| b.name.starts_with("core.save.write_") || b.name.starts_with("core.save.full_")) {
        b.note = Some(format!("written {place}"));
    }
    // The same on the disk, with its sync: for the trend, with a wide budget.
    let disk = work_dir().join("disk");
    std::fs::create_dir_all(&disk)?;
    let path = disk.join("saved.chum5");
    r.bench_setup(&format!("core.save.disk_chum5.{s}"), 7, || ch.clone(), |mut c| engine.save(&mut c, &path).expect("save"));
    if let Some(b) = r.report.benches.last_mut().filter(|b| b.name.starts_with("core.save.disk_")) {
        b.note = Some("a whole .chum5 save to the disk (target/perf/work), synced: depends on the disk and what else uses it".into());
    }
    Ok(())
}

fn printing(r: &mut Runner, engine: &Engine) -> Result<()> {
    if !r.wants("core.print.") {
        return Ok(());
    }
    let lang = Language::load(&data::resource_dir("lang").context("lang directory")?, "en-us");
    let xsl = print::find_sheet("en-us", print::DEFAULT_SHEET).context("the default sheet")?;
    let xsltproc = print::xsltproc_path().filter(|p| std::process::Command::new(p).arg("--version").output().is_ok());
    let files = files_dir();
    for name in [HEAVY, BIG] {
        let s = slug(name);
        let ch = load(name)?;
        r.bench(&format!("core.print.xml.{s}"), 10, || {
            let _ = print::print_xml(&ch, engine, &lang);
        });
        let name = format!("core.print.xslt.{s}");
        if xsltproc.is_none() {
            r.push(Record::skipped(&name, "xsltproc not found"));
            continue;
        }
        let xml = print::print_xml(&ch, engine, &lang);
        let out = files.join(format!("{s}.html"));
        r.bench(&name, 7, || print::render(&xml, &xsl, &out).expect("render"));
        if let Some(b) = r.report.benches.last_mut().filter(|b| b.name == name) {
            b.note = Some(format!("the default sheet ({}) through xsltproc, a process per sample", print::DEFAULT_SHEET));
        }
    }
    Ok(())
}

/// Peak memory of a process that loads the engine and the ten biggest
/// fixtures and computes their sheets.
fn memory(r: &mut Runner) -> Result<()> {
    let name = "core.memory.engine_plus_10";
    if !r.wants(name) {
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        r.push(Record { unit: "MiB".into(), ..Record::skipped(name, "needs /proc/self/status (Linux)") });
        return Ok(());
    }
    let dir = fixture(HEAVY).parent().context("fixtures")?.to_owned();
    let mut all: Vec<(u64, String)> = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).map(|p| (std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0), p.to_string_lossy().into_owned())).collect();
    all.sort_by_key(|a| std::cmp::Reverse(a.0));
    let paths: Vec<&str> = std::iter::once("memory").chain(all.iter().take(10).map(|(_, p)| p.as_str())).collect();
    let t = Instant::now();
    let v = child(&paths)?;
    let took = ms(t);
    let peak = v["peak_mib"].as_f64().unwrap_or(0.0);
    let engine = v["engine_mib"].as_f64().unwrap_or(0.0);
    r.push(Record::from_samples(name, "MiB", vec![peak]).with_note(format!("peak RSS (VmHWM) of a process with the engine ({engine:.0} MiB after Engine::load) and the 10 biggest fixtures with their sheets; took {took:.0} ms")));
    Ok(())
}

/// Items (elements with a guid) in a character.
pub fn item_count(ch: &Character) -> usize {
    fn walk(e: &chummer_core::xml::Element) -> usize {
        usize::from(e.child("guid").is_some()) + e.elements().map(walk).sum::<usize>()
    }
    walk(&ch.doc)
}

/// The large synthetic character ([`chummer_bench::synth`]): load and
/// compute, and edits.
fn large(r: &mut Runner, engine: &Engine) -> Result<()> {
    if !r.wants("core.large.") {
        return Ok(());
    }
    let t = Instant::now();
    let (ch, added) = chummer_bench::synth::large(engine, load(HEAVY)?);
    let built = ms(t);
    let items = item_count(&ch);
    eprintln!("  large character: {added} items added to {HEAVY} ({items} in all) in {built:.0} ms");
    let path = files_dir().join("large.chum5");
    engine.save(&mut ch.clone(), &path)?;
    let about = format!("{HEAVY} with {added} items added ({items} in all)");
    r.bench("core.large.load_compute", 10, || {
        let c = Character::load(&path).expect("load");
        let _ = engine.sheet(&c);
    });
    r.bench("core.large.compute", 15, || {
        let _ = engine.sheet(&ch);
    });
    let mut base = ch.clone();
    base.karma = 10_000;
    base.nuyen = 10_000_000.0;
    let fresh = || Session::new(base.clone());
    let medkit = Command::AddItem { tag: "gear".into(), record: RecordRef { id: String::new(), name: "Medkit".into() }, purchase: Purchase { rating: 3, qty: 1.0, cost_multiplier: 1.0, ..Default::default() } };
    r.bench_setup("core.large.add_gear", 15, fresh, |mut ses| {
        ses.apply(engine, medkit.clone()).expect("AddItem");
    });
    r.bench_setup("core.large.set_field", 15, fresh, |mut ses| {
        ses.apply(engine, Command::SetField { key: "alias".into(), value: "Bench".into() }).expect("SetField");
    });
    for b in r.report.benches.iter_mut().filter(|b| b.name.starts_with("core.large.")) {
        b.note = Some(about.clone());
    }
    Ok(())
}
