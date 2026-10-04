//! Per-category creation karma for every creation-mode fixture (debugging
//! aid for tests/karma_oracle.rs). Pass a name fragment to filter and
//! print per-item details.
use chummer_core::{calc, chargen, character::Character, engine::Engine};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let engine = Engine::load().unwrap();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "chum5")).collect();
    files.sort();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if !name.contains(&filter) {
            continue;
        }
        let ch = Character::load(&f).unwrap();
        if ch.created {
            continue;
        }
        let st = engine.store_for_character(&ch);
        let rules = engine.rules_for(&ch);
        let sheet = calc::compute(&ch, &rules, Some(&st), Some(&engine.catalog));
        let Some(set) = engine.settings.resolve(&ch.field("settings")) else { continue };
        // KARMA_FLAGS=a,b turns on settings flags (house-rule experiments).
        let mut set = set.clone();
        for flag in std::env::var("KARMA_FLAGS").unwrap_or_default().split(',').filter(|f| !f.is_empty()) {
            set.raw.set_child_text(flag, "True");
        }
        let set = &set;
        let b = chargen::budget_with(&ch, &sheet, &rules, set, Some(&st));
        let k = chargen::karma_breakdown(&ch, &sheet, &rules, set, Some(&st));
        let saved = ch.doc.get_i32("karma").unwrap_or(0);
        println!(
            "{} {name:<28} want-spent {:>4} ours {:>4} (off {:>4}) | {}",
            if b.karma_left() == saved { "ok  " } else { "DIFF" },
            b.karma.0 - saved,
            b.karma.1,
            b.karma.1 - (b.karma.0 - saved),
            k.iter().filter(|(_, v)| *v != 0).map(|(n, v)| format!("{n} {v}")).collect::<Vec<_>>().join(", ")
        );
        if filter.is_empty() {
            continue;
        }
        println!("  settings {} build {}", ch.field("settings"), ch.field("buildmethod"));
        for a in &sheet.attributes {
            let c = calc::attribute_karma_cost(a, &rules);
            if c != 0 {
                println!("  attr {} base {} karma {} total_base {} -> {c}", a.name, a.base, a.karma, a.total_base);
            }
        }
        for s in sheet.skills.iter().chain(&sheet.knowledge_skills) {
            if s.karma_cost != 0 || s.karma != 0 {
                println!("  skill {} base {} karma {} rating {} specs {:?} -> {}", s.name, s.base, s.karma, s.rating, s.specs, s.karma_cost);
            }
        }
        for g in &ch.skill_groups {
            if g.karma != 0 || g.base != 0 {
                println!("  group {} base {} karma {}", g.name, g.base, g.karma);
            }
        }
        for q in ch.items("qualities", "quality") {
            println!(
                "  quality {} [{}] bp {} type {} src {} ctbp {} ctl {} doubled {}",
                q.get("name"),
                q.get("extra"),
                q.get("bp"),
                q.get("qualitytype"),
                q.get("qualitysource"),
                q.get("contributetobp"),
                q.get("contributetolimit"),
                q.get("doublecareer")
            );
        }
        for c in ch.items("contacts", "contact") {
            println!(
                "  contact {} type {} C{} L{} free {} group {} family {} blackmail {}",
                c.get("name"),
                c.get("type"),
                c.get("connection"),
                c.get("loyalty"),
                c.get("free"),
                c.get("group"),
                c.get("family"),
                c.get("blackmail")
            );
        }
        for i in ch.improvements.list.iter() {
            if i.kind.contains("Karma") || i.kind.contains("Quality") || i.kind.contains("Contact") {
                println!("  imp {} {} val {} src {} {}", i.kind, i.improved_name, i.val, i.source, i.source_name);
            }
        }
    }
}
