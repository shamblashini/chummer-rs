//! chummer-cli: inspect and check `.chum5` characters from a terminal.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use chummer_core::attributes;
use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::print;
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

    chummer-cli new <out.chum5> [options]  Create a character (Priority / Sum-to-Ten)
        --settings <name|id>   preset (default Standard)
        --metatype <name>      e.g. Human, Elf (default Human)
        --metavariant <name>
        --priorities <HTASR>   letters for Heritage, Talent, Attributes,
                               Skills, Resources (default DEABC)
        --talent <value>       e.g. Mundane, Magician, Adept (default Mundane)
        --skills <a,b>         free talent skills
        --name <text>

    chummer-cli sheet <file.chum5> [options]  Print a character sheet to HTML
        -o <out.html>          output file (default: next to the character)
        --sheet <name>         sheet name or file (default: Shadowrun 5
                               (Skills grouped by Rating greater 0))
        --lang <code>          print language, e.g. de-de (default en-us)
        --notes                include item and skill notes
        --expenses             include the karma and nuyen log
        --xml <out.xml>        also write the print XML
        --pdf                  convert to PDF (needs chromium, wkhtmltopdf
                               or weasyprint; else print the HTML from a browser)
    chummer-cli sheets [lang]              List available sheets

    chummer-cli sources list               Show linked sourcebook PDFs
    chummer-cli sources import-wine [pfx]  Import PDF links from Chummer5a under Wine
    chummer-cli sources scan <dir>         Link PDFs in a folder by title
    chummer-cli sources detect             Find page offsets with pdftotext
    chummer-cli sources open <BOOK> <page> Open a book at a page, e.g. SR5 143
    chummer-cli sources viewer [template]  Show or set the PDF viewer command
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
        "sources" => sources_cmd(&Engine::load()?, rest),
        "new" => new_cmd(&Engine::load()?, rest),
        "sheet" => sheet_cmd(&Engine::load()?, rest),
        "sheets" => {
            for (name, path) in print::available_sheets(rest.first().map_or("en-us", String::as_str)) {
                println!("{name:<55} {}", path.display());
            }
            Ok(())
        }
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

fn sources_cmd(engine: &Engine, rest: &[String]) -> Result<()> {
    use chummer_core::sources::{self, SourceRef, SourcebookLibrary};
    let mut lib = SourcebookLibrary::load();
    let books = sources::book_list(&engine.store);
    let name_of = |code: &str| books.iter().find(|b| b.code == code).map(|b| b.name.clone()).unwrap_or_default();
    match rest.first().map(String::as_str) {
        None | Some("list") => {
            println!("viewer: {}", if lib.viewer.is_empty() { "(none)" } else { &lib.viewer });
            for b in &books {
                let link = lib.books.get(&b.code).cloned().unwrap_or_default();
                match &link.path {
                    Some(p) => {
                        let warn = if p.is_file() { "" } else { "  [missing]" };
                        println!("{:<8} {:<45} {:+} {}{warn}", b.code, b.name, link.offset, p.display());
                    }
                    None => println!("{:<8} {:<45}    -", b.code, b.name),
                }
            }
            println!("{} of {} books linked", lib.linked_count(), books.len());
        }
        Some("import-wine") => {
            let prefixes: Vec<PathBuf> = match rest.get(1) {
                Some(p) => vec![PathBuf::from(p)],
                None => sources::find_wine_prefixes(),
            };
            if prefixes.is_empty() {
                bail!("no Wine prefix with Chummer5a sourcebook settings found; pass one explicitly");
            }
            for pfx in prefixes {
                let found = sources::import_from_wine(&pfx)?;
                println!("{}: {} linked books", pfx.display(), found.len());
                for (code, path, offset) in found {
                    let ok = if path.is_file() { "" } else { "  [file missing]" };
                    println!("  {code:<8} {offset:+} {}{ok}", path.display());
                    lib.books.insert(code, sources::Sourcebook { path: Some(path), offset });
                }
            }
            lib.save()?;
        }
        Some("scan") => {
            let Some(dir) = rest.get(1) else { bail!("expected a folder") };
            let found = sources::scan_folder(Path::new(dir), &books);
            for (code, path) in &found {
                println!("  {code:<8} {:<40} {}", name_of(code), path.display());
                let e = lib.books.entry(code.clone()).or_default();
                e.path = Some(path.clone());
            }
            println!("{} books matched", found.len());
            lib.save()?;
        }
        Some("detect") => {
            if sources::which("pdftotext").is_none() {
                bail!("pdftotext not found; install poppler");
            }
            for b in &books {
                let Some(link) = lib.books.get(&b.code).cloned() else { continue };
                let Some(path) = link.path.filter(|p| p.is_file()) else { continue };
                match sources::detect_offset(&path, b) {
                    Some(off) => {
                        let note = if off != link.offset { format!("  (was {:+})", link.offset) } else { String::new() };
                        println!("  {:<8} {off:+}{note}", b.code);
                        lib.books.get_mut(&b.code).unwrap().offset = off;
                    }
                    None => println!("  {:<8} not found, keeping {:+}", b.code, link.offset),
                }
            }
            lib.save()?;
        }
        Some("open") => {
            let (Some(book), Some(page)) = (rest.get(1), rest.get(2)) else { bail!("expected BOOK PAGE") };
            let r = SourceRef::new(book, page).context("invalid page")?;
            println!("{}", lib.command_for(&r)?.join(" "));
            lib.open(&r)?;
        }
        Some("viewer") => match rest.get(1) {
            Some(t) => {
                lib.viewer = rest[1..].join(" ");
                lib.save()?;
                println!("viewer set to: {}", lib.viewer);
                let _ = t;
            }
            None => {
                println!("current: {}", lib.viewer);
                for v in sources::installed_viewers() {
                    println!("installed: {:<32} {}", v.name, v.template);
                }
            }
        },
        Some(other) => bail!("unknown sources command {other:?}"),
    }
    Ok(())
}

fn new_cmd(engine: &Engine, rest: &[String]) -> Result<()> {
    use chummer_core::chargen::{self, NewCharacter, Priorities};
    let Some(out) = rest.first() else { bail!("expected an output file") };
    let opt = |k: &str, default: &str| -> String {
        rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned().unwrap_or_else(|| default.to_owned())
    };
    let settings = opt("--settings", "Standard");
    let preset = engine.settings.presets.iter().find(|p| p.name() == settings || p.id() == settings).context("unknown settings preset")?;
    let letters: Vec<char> = opt("--priorities", "DEABC").to_uppercase().chars().collect();
    if letters.len() != 5 {
        bail!("--priorities needs five letters");
    }
    let spec = NewCharacter {
        settings_id: preset.id(),
        metatype: opt("--metatype", "Human"),
        metavariant: Some(opt("--metavariant", "")).filter(|v| !v.is_empty()),
        priorities: Priorities([letters[0], letters[1], letters[2], letters[3], letters[4]]),
        talent: opt("--talent", "Mundane"),
        talent_skills: opt("--skills", "").split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
        name: opt("--name", "New Runner"),
    };
    let mut ch = chargen::create(engine, &spec).map_err(anyhow::Error::msg)?;
    engine.save(&mut ch, Path::new(out))?;
    println!("wrote {out}");
    Ok(())
}

/// `sheet`: render a character with an XSLT sheet (Chummer's
/// CharacterSheetViewer, as a command).
fn sheet_cmd(engine: &Engine, rest: &[String]) -> Result<()> {
    let Some(file) = rest.first() else { bail!("expected a .chum5 file") };
    let opt = |k: &str| rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned();
    let flag = |k: &str| rest.iter().any(|a| a == k);
    let lang_code = opt("--lang").unwrap_or_else(|| "en-us".into());
    let sheet_name = opt("--sheet").unwrap_or_else(|| print::DEFAULT_SHEET.into());
    let xsl = print::find_sheet(&lang_code, &sheet_name)
        .with_context(|| format!("no sheet {sheet_name:?} for {lang_code}; see `chummer-cli sheets {lang_code}`"))?;
    let ch = load(Path::new(file))?;
    let lang_dir = data::resource_dir("lang").context("lang directory not found")?;
    let lang = Language::load(&lang_dir, &lang_code);
    let opts = print::PrintOptions { notes: flag("--notes"), expenses: flag("--expenses"), ..Default::default() };
    let xml = print::print_xml_with(&ch, engine, &lang, opts);
    if let Some(x) = opt("--xml") {
        std::fs::write(&x, xml.to_xml_string()).with_context(|| format!("writing {x}"))?;
    }
    let out = opt("-o").map(PathBuf::from).unwrap_or_else(|| Path::new(file).with_extension("html"));
    let report = print::render_report(&xml, &xsl, &out)?;
    if !report.warnings.trim().is_empty() {
        eprintln!("{}", report.warnings.trim_end());
    }
    println!("wrote {}", out.display());
    if flag("--pdf") {
        let pdf = out.with_extension("pdf");
        print::html_to_pdf(&out, &pdf)?;
        println!("wrote {}", pdf.display());
    }
    Ok(())
}
