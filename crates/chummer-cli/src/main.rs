//! chummer-cli: inspect and check `.chum5` / `.chum5lz` characters from a
//! terminal.

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
    Character files are .chum5 or compressed .chum5lz (by extension).

    chummer-cli info <file.chum5>          Show a character sheet summary
    chummer-cli skills <file.chum5>        List skills with dice pools
    chummer-cli items <file.chum5>         List gear, ware, weapons and more
    chummer-cli check <file|dir>...        Load and recompute; report problems
    chummer-cli search <text> [kind]       Search game data (kind e.g. Gear)
    chummer-cli kinds                      List searchable data kinds

    chummer-cli export <file.chum5> <XML|JSON|stylesheet> -o <out>   Export a character
    chummer-cli roster <dir>...            List characters in folders
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
        --all-sheets <dir>     render every sheet of the language into
                               <dir>/<sheet>.html; fails if any sheet does
        --pdf                  convert to PDF (needs chromium, wkhtmltopdf
                               or weasyprint; else print the HTML from a browser)
    chummer-cli sheets [lang]              List available sheets

    chummer-cli settings list              List settings presets (house rules)
    chummer-cli settings export <name|key> -o <file.xml>
                                           Save a preset as a Chummer settings file
    chummer-cli settings import <file.xml> [--overwrite | --keep-both]
                                           Install a shared settings file

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
        "settings" => settings_cmd(&Engine::load()?, rest),
        "new" => new_cmd(&Engine::load()?, rest),
        "export" => export_cmd(&Engine::load()?, rest),
        "roster" => {
            let dirs: Vec<PathBuf> = rest.iter().map(PathBuf::from).collect();
            for e in chummer_core::roster::scan(&dirs) {
                let state = if e.career { "career" } else { "creation" };
                match &e.error {
                    Some(err) => println!("{:<28} ERROR {err}  {}", e.display_name(), e.path.display()),
                    None => println!("{:<28} {:<14} {:<9} karma {:<5} {}", e.display_name(), e.metatype, state, e.karma, e.path.display()),
                }
            }
            Ok(())
        }
        "sheet" => sheet_cmd(&Engine::load()?, rest),
        "sheets" => {
            for (name, path) in print::available_sheets(rest.first().map_or("en-us", String::as_str)) {
                println!("{name}\t{}", path.display());
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
        _ => bail!("expected one .chum5 or .chum5lz file"),
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
    if let Some(missing) = engine.settings.missing_preset(&ch.field("settings")) {
        let fallback = engine.settings.fallback().map(|p| p.name()).unwrap_or_default();
        println!("warning: settings file {missing:?} is not installed; costs and budgets use {fallback}");
    }
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
                if chummer_core::chum5lz::is_character_file(&f) {
                    files.push(f);
                }
            }
        } else {
            files.push(p);
        }
    }
    if files.is_empty() {
        bail!("no .chum5 or .chum5lz files given");
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

fn settings_cmd(engine: &Engine, rest: &[String]) -> Result<()> {
    use chummer_core::settings::{self, ImportMode};
    let lib = &engine.settings;
    match rest.first().map(String::as_str) {
        None | Some("list") => {
            for p in &lib.presets {
                let kind = if p.file.is_some() { "yours" } else { "built-in" };
                println!("{:<40} {:<9} {:<11} {}", p.name(), kind, p.build_method(), p.key());
            }
            if let Some(d) = settings::user_settings_dir() {
                println!("user settings: {}", d.display());
            }
        }
        Some("export") => {
            let name = rest.get(1).filter(|a| *a != "-o").context("expected a preset name or key")?;
            let preset = lib.find(name).with_context(|| format!("no settings preset {name:?} (see `settings list`)"))?;
            let out = rest.iter().position(|a| a == "-o").and_then(|i| rest.get(i + 1)).context("missing -o OUT")?;
            settings::export(preset, Path::new(out)).with_context(|| format!("writing {out}"))?;
            println!("{} -> {out}", preset.name());
        }
        Some("import") => {
            let file = rest.get(1).context("expected a settings file")?;
            let mode = match (rest.iter().any(|a| a == "--overwrite"), rest.iter().any(|a| a == "--keep-both")) {
                (true, true) => bail!("--overwrite and --keep-both exclude each other"),
                (true, false) => ImportMode::Overwrite,
                (false, true) => ImportMode::KeepBoth,
                _ => ImportMode::New,
            };
            let dir = settings::user_settings_dir().context("no settings directory")?;
            let plan = lib.plan_import(Path::new(file), &dir).map_err(anyhow::Error::msg)?;
            let done = settings::import(&plan, lib, &dir, &mode).map_err(|e| anyhow::anyhow!("{e}; pass --overwrite or --keep-both"))?;
            println!("{} installed as {} (characters refer to it as {})", done.name, done.path.display(), done.key);
        }
        Some(other) => bail!("unknown settings command {other:?}"),
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
            let res = sources::scan(Path::new(dir), &books, sources::pdf_pages);
            for f in &res.found {
                let how = if f.by_text { " (by text)" } else { "" };
                println!("  {:<8} {:<40} {}{how}", f.code, name_of(&f.code), f.path.display());
                let e = lib.books.entry(f.code.clone()).or_default();
                e.path = Some(f.path.clone());
                if let Some(off) = f.offset {
                    e.offset = off;
                }
            }
            println!("{} books matched", res.found.len());
            if !res.unmatched.is_empty() {
                println!("Not linked:");
                for (path, why) in &res.unmatched {
                    let why = match why {
                        sources::Unmatched::OtherEdition => "another edition".to_owned(),
                        sources::Unmatched::Errata => "errata or FAQ with no book of its own".to_owned(),
                        sources::Unmatched::Duplicate(c) => format!("another file is already linked to {c}"),
                        sources::Unmatched::NoBook => "no book in Chummer's data".to_owned(),
                    };
                    println!("  {}  ({why})", path.file_name().unwrap_or_default().to_string_lossy());
                }
            }
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
    let Some(file) = rest.first() else { bail!("expected a .chum5 or .chum5lz file") };
    let opt = |k: &str| rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned();
    let flag = |k: &str| rest.iter().any(|a| a == k);
    let lang_code = opt("--lang").unwrap_or_else(|| "en-us".into());
    let ch = load(Path::new(file))?;
    let lang_dir = data::resource_dir("lang").context("lang directory not found")?;
    let lang = Language::load(&lang_dir, &lang_code);
    let opts = print::PrintOptions { notes: flag("--notes"), expenses: flag("--expenses"), ..Default::default() };
    let xml = print::print_xml_with(&ch, engine, &lang, opts);
    if let Some(x) = opt("--xml") {
        std::fs::write(&x, xml.to_xml_string()).with_context(|| format!("writing {x}"))?;
    }
    if let Some(dir) = opt("--all-sheets") {
        return all_sheets(&xml, &lang_code, Path::new(&dir));
    }
    let sheet_name = opt("--sheet").unwrap_or_else(|| print::DEFAULT_SHEET.into());
    let xsl = print::find_sheet(&lang_code, &sheet_name)
        .with_context(|| format!("no sheet {sheet_name:?} for {lang_code}; see `chummer-cli sheets {lang_code}`"))?;
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

fn export_cmd(engine: &Engine, rest: &[String]) -> Result<()> {
    let (Some(file), Some(format)) = (rest.first(), rest.get(1)) else {
        let names: Vec<String> = chummer_core::export::BUILT_IN.iter().map(|s| s.to_string()).chain(chummer_core::export::stylesheets().into_iter().map(|(n, _)| n)).collect();
        bail!("expected FILE FORMAT -o OUT; formats: {}", names.join(", "));
    };
    let out = rest.iter().position(|a| a == "-o").and_then(|i| rest.get(i + 1)).context("missing -o OUT")?;
    let ch = load(Path::new(file))?;
    let lang = chummer_core::lang::Language::load(&data::resource_dir("lang").context("lang dir")?, "en-us");
    chummer_core::export::export(&ch, engine, &lang, format, Path::new(out))?;
    println!("wrote {out}");
    Ok(())
}

/// `sheet --all-sheets <dir>`: render the print XML with every sheet of
/// the language, report each, and fail if any sheet errors or warns.
fn all_sheets(xml: &chummer_core::xml::Element, lang_code: &str, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let sheets = print::available_sheets(lang_code);
    if sheets.is_empty() {
        bail!("no sheets for {lang_code}");
    }
    let mut failed = 0;
    for (name, xsl) in &sheets {
        let file_name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
        let out = dir.join(format!("{file_name}.html"));
        match print::render_report(xml, xsl, &out) {
            Ok(r) if r.warnings.trim().is_empty() => println!("OK    {name}"),
            Ok(r) => {
                failed += 1;
                println!("WARN  {name}: {}", r.warnings.trim().replace('\n', " | "));
            }
            Err(e) => {
                failed += 1;
                println!("FAIL  {name}: {e}");
            }
        }
    }
    println!("{} of {} sheets rendered into {}", sheets.len() - failed, sheets.len(), dir.display());
    if failed > 0 {
        bail!("{failed} sheet(s) failed");
    }
    Ok(())
}
