//! chummer-cli: inspect and check `.chum5` characters from a terminal.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use chummer_core::attributes;
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::sections;

const USAGE: &str = "\
chummer-cli — Shadowrun 5e character tools (chummer-rs)

USAGE:
    chummer-cli info <file.chum5>          Show a character sheet summary
    chummer-cli skills <file.chum5>        List skills with dice pools
    chummer-cli items <file.chum5>         List gear, ware, weapons and more
    chummer-cli check <file|dir>...        Load and recompute; report problems
    chummer-cli search <text> [kind]       Search game data (kind e.g. Gear)
    chummer-cli kinds                      List searchable data kinds
";

fn main() -> ExitCode {
    // Exit quietly when piped into `head` and the like.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "info" => info(&Engine::load()?, one_file(rest)?),
        "skills" => skills(&Engine::load()?, one_file(rest)?),
        "items" => items(one_file(rest)?),
        "check" => check(&Engine::load()?, rest),
        "search" => search(&Engine::load()?, rest),
        "kinds" => {
            for (label, file, ..) in data::BROWSABLE {
                println!("{label:<20} {file}");
            }
            Ok(())
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn one_file(rest: &[String]) -> Result<&Path> {
    match rest {
        [f] => Ok(Path::new(f)),
        _ => bail!("expected one .chum5 file"),
    }
}

fn load(path: &Path) -> Result<Character> {
    Character::load(path).with_context(|| format!("loading {}", path.display()))
}

fn info(engine: &Engine, path: &Path) -> Result<()> {
    let ch = load(path)?;
    let s = engine.sheet(&ch);
    println!("{}", ch.display_name());
    let meta = [ch.field("metatype"), ch.field("metavariant"), ch.field("sex"), ch.field("age")];
    println!("{}", meta.iter().filter(|m| !m.is_empty()).cloned().collect::<Vec<_>>().join(" · "));
    println!(
        "{} · {} · settings: {}",
        if ch.created { "Career mode" } else { "Creation mode" },
        ch.field("buildmethod"),
        engine.settings.resolve(&ch.field("settings")).map(|p| p.name()).unwrap_or_default()
    );
    println!();
    for group in [attributes::PHYSICAL, attributes::MENTAL, attributes::SPECIAL] {
        for name in group {
            let shown = match *name {
                "ESS" => false,
                "MAG" => ch.mag_enabled(),
                "MAGAdept" => ch.mag_enabled() && ch.is_adept() && ch.is_magician(),
                "RES" => ch.res_enabled(),
                "DEP" => ch.dep_enabled(),
                _ => true,
            };
            if !shown {
                continue;
            }
            if let Some(a) = s.attr_values(name) {
                let aug = if a.total != a.value { format!(" ({})", a.total) } else { String::new() };
                println!("  {:<14} {:>2}{:<5} max {}/{}", attributes::long_name(name), a.value, aug, a.total_max, a.total_aug_max);
            }
        }
        println!();
    }
    println!("  Essence           {:.2}", s.essence);
    println!("  Initiative        {} + {}d6", s.initiative, s.initiative_dice);
    println!("  Astral init.      {} + {}d6", s.astral_initiative, s.astral_initiative_dice);
    println!("  Matrix (cold/hot) {} + {}d6 / {} + {}d6", s.matrix_cold_initiative, s.matrix_cold_dice, s.matrix_hot_initiative, s.matrix_hot_dice);
    println!("  Condition monitor physical {} (filled {}), stun {} (filled {}), overflow {}", s.physical_cm, ch.physical_cm_filled, s.stun_cm, ch.stun_cm_filled, s.cm_overflow);
    println!("  Limits            physical {}, mental {}, social {}, astral {}", s.limit_physical, s.limit_mental, s.limit_social, s.limit_astral);
    println!("  Composure {}, Judge Intentions {}, Memory {}, Lift/Carry {}", s.composure, s.judge_intentions, s.memory, s.lift_carry);
    println!("  Armor             {}", s.armor);
    println!("  Karma {} · Nuyen {}", ch.karma, chummer_core::format::nuyen(ch.nuyen));
    Ok(())
}

fn skills(engine: &Engine, path: &Path) -> Result<()> {
    let ch = load(path)?;
    let s = engine.sheet(&ch);
    println!("{:<34} {:<4} {:>6} {:>5}  specializations", "Active skill", "attr", "rating", "pool");
    for sk in s.skills.iter().filter(|k| k.rating > 0) {
        let spec = if sk.specs.is_empty() { String::new() } else { format!("{} (+{})", sk.specs.join(", "), sk.spec_bonus) };
        println!("{:<34} {:<4} {:>6} {:>5}  {spec}", sk.name, sk.attribute, sk.rating, sk.pool);
    }
    println!();
    println!("{:<34} {:<12} {:>6} {:>5}", "Knowledge skill", "type", "rating", "pool");
    for k in &s.knowledge_skills {
        let pool = if k.native { "N".into() } else { k.pool.to_string() };
        println!("{:<34} {:<12} {:>6} {:>5}", k.name, k.category, if k.native { "N".into() } else { k.rating.to_string() }, pool);
    }
    Ok(())
}

fn items(path: &Path) -> Result<()> {
    let ch = load(path)?;
    let all = [sections::QUALITIES, sections::CONTACTS]
        .iter()
        .chain(sections::MAGIC)
        .chain(sections::EQUIPMENT)
        .copied()
        .collect::<Vec<_>>();
    for sec in all {
        let list = ch.items(sec.container, sec.item);
        if list.is_empty() {
            continue;
        }
        println!("{} ({})", sec.label, list.len());
        for it in list {
            print_item(&sec, it, 1);
        }
        println!();
    }
    Ok(())
}

fn print_item(sec: &sections::Section, e: &chummer_core::xml::Element, depth: usize) {
    let extra: Vec<String> = sec.columns[1..]
        .iter()
        .filter_map(|c| {
            let v = e.get(c.field);
            (!v.is_empty() && v != "0").then(|| format!("{} {v}", c.header))
        })
        .collect();
    println!("{}{}  {}", "  ".repeat(depth), e.get("name"), extra.join(" · "));
    for (container, item) in sec.child_containers {
        if let Some(c) = e.child(container) {
            for child in c.children_named(item) {
                print_item(sec, child, depth + 1);
            }
        }
    }
}

fn check(engine: &Engine, targets: &[String]) -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    for t in targets {
        let p = PathBuf::from(t);
        if p.is_dir() {
            for e in std::fs::read_dir(&p)? {
                let f = e?.path();
                if f.extension().is_some_and(|x| x == "chum5") {
                    files.push(f);
                }
            }
        } else {
            files.push(p);
        }
    }
    if files.is_empty() {
        bail!("no .chum5 files given");
    }
    files.sort();
    let mut problems = 0;
    for f in &files {
        match Character::load(f) {
            Ok(ch) => {
                let s = engine.sheet(&ch);
                let diffs: Vec<String> = ch
                    .attributes
                    .iter()
                    .filter(|a| a.name != "ESS" && a.category != "Shapeshifter")
                    .filter_map(|a| {
                        let saved = a.saved_total?;
                        let got = s.attr(&a.name);
                        (got != saved).then(|| format!("{} {got} (file says {saved})", a.name))
                    })
                    .collect();
                if diffs.is_empty() {
                    println!("ok    {}", f.display());
                } else {
                    problems += 1;
                    println!("DIFF  {}: {}", f.display(), diffs.join(", "));
                }
            }
            Err(e) => {
                problems += 1;
                println!("FAIL  {e}");
            }
        }
    }
    println!("{} files, {problems} with problems", files.len());
    Ok(())
}

fn search(engine: &Engine, rest: &[String]) -> Result<()> {
    let Some(text) = rest.first() else { bail!("expected search text") };
    let kind = rest.get(1).map(|k| k.to_lowercase());
    let needle = text.to_lowercase();
    for (label, file, container, item) in data::BROWSABLE {
        if kind.as_ref().is_some_and(|k| !label.to_lowercase().contains(k.as_str())) {
            continue;
        }
        let doc = engine.store.doc(file)?;
        for r in data::records(&doc, container, item) {
            if r.name().to_lowercase().contains(&needle) {
                println!("{:<18} {:<45} {:<28} {} p.{}", label, r.name(), r.category(), r.source(), r.page());
            }
        }
    }
    Ok(())
}
