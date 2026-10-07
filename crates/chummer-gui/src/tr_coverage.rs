//! Coverage of the GUI's `lang.tr("...")` labels by Chummer's language files.
//!
//! `cargo test -p chummer-gui tr_coverage -- --nocapture` lists the labels
//! that have no en-us key, and those de-de does not translate.

use std::collections::BTreeSet;

use chummer_core::data;
use chummer_core::lang::Language;

/// Every string literal passed to `.tr(`, `.tr_fmt(` or `.tr_all([` in the
/// GUI sources.
fn gui_labels() -> BTreeSet<String> {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut out = BTreeSet::new();
    let files = [src.to_owned(), format!("{src}/workspace")].into_iter().flat_map(|d| std::fs::read_dir(d).unwrap().flatten());
    for entry in files {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") || path.ends_with("tr_coverage.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        for pat in [".tr(\"", ".tr_fmt(\""] {
            for (i, _) in src.match_indices(pat) {
                out.insert(literal(&src[i + pat.len()..]));
            }
        }
        for (i, _) in src.match_indices(".tr_all([") {
            let list = &src[i + 9..];
            let list = &list[..list.find(']').unwrap()];
            for (j, _) in list.match_indices('"').step_by(2) {
                out.insert(literal(&list[j + 1..]));
            }
        }
    }
    out.remove("");
    out
}

/// Labels from tables the GUI passes through `lang.tr` at run time.
fn table_labels() -> BTreeSet<String> {
    use chummer_core::{attributes, chargen, character, items, sections};
    let mut out: BTreeSet<String> = BTreeSet::new();
    let secs = sections::MAGIC.iter().chain(sections::EQUIPMENT).chain([&sections::QUALITIES, &sections::CONTACTS, &sections::COMPLEX_FORMS, &sections::MARTIAL_ARTS]);
    for s in secs {
        out.insert(s.label.into());
        out.extend(s.columns.iter().map(|c| c.header.to_owned()));
    }
    out.extend(character::INFO_FIELDS.iter().chain(character::TEXT_FIELDS).map(|(_, l)| (*l).to_owned()));
    out.extend(items::KINDS.iter().map(|k| k.label.to_owned()));
    out.extend(data::BROWSABLE.iter().map(|b| b.0.to_owned()));
    out.extend(chargen::CATEGORIES.iter().map(|c| (*c).to_owned()));
    out.extend(attributes::PHYSICAL.iter().chain(attributes::MENTAL).chain(attributes::SPECIAL).map(|a| attributes::long_name(a).to_owned()));
    out.extend(crate::view::TABS.iter().map(|(_, l)| (*l).to_owned()));
    out.extend(crate::workspace::palette::Cmd::ALL.iter().flat_map(|c| [c.label(), c.menu()]).map(str::to_owned));
    out.extend(crate::theme::ThemeKind::ALL.iter().map(|k| k.label().to_owned()).chain(crate::theme::Layout::ALL.iter().map(|l| l.label().to_owned())));
    out.extend(crate::settings_ui::LABELS.iter().map(|(_, l)| (*l).to_owned()));
    out.extend(chargen::issues::templates().into_iter().map(str::to_owned));
    out.extend(chargen::guide::ALL.iter().flat_map(|s| [s.title(), s.explanation("Priority"), s.explanation("Karma"), s.explanation("LifeModule"), s.explanation("SumtoTen"), s.prompt("Priority"), s.prompt("Karma")]).map(str::to_owned));
    out.remove("");
    out
}

/// The body of a string literal starting right after its opening quote.
fn literal(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some(e) => out.push(e),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

#[test]
fn tr_coverage() {
    let dir = data::resource_dir("lang").unwrap();
    let en = Language::load(&dir, "en-us");
    let de = Language::load(&dir, "de-de");
    let literals = gui_labels();
    assert!(!literals.is_empty());
    let labels: BTreeSet<String> = literals.iter().cloned().chain(table_labels()).collect();
    // A matched label must already use Chummer's exact English wording, so
    // wrapping it never changes what an English user sees.
    let changed: Vec<_> = literals.iter().filter(|l| en.tr(l) != **l).map(|l| format!("{l:?} -> {:?}", en.tr(l))).collect();
    assert!(changed.is_empty(), "labels differing from Chummer's English text:\n{}", changed.join("\n"));

    let unknown: Vec<_> = labels.iter().filter(|l| en.key_for(l).is_none()).collect();
    let untranslated: Vec<_> = labels.iter().filter(|l| !de.translates(l)).collect();
    let in_source = |set: &[&String]| set.iter().filter(|l| literals.contains(**l)).count();
    println!(
        "GUI source literals: {}; {} match an en-us key; {} translated by de-de",
        literals.len(),
        literals.len() - in_source(&unknown),
        literals.len() - in_source(&untranslated)
    );
    println!("{} GUI labels incl. core tables; {} match an en-us key; {} translated by de-de", labels.len(), labels.len() - unknown.len(), labels.len() - untranslated.len());
    println!("No en-us key (needs a Chummer string or stays English):");
    for l in &unknown {
        println!("  {l:?}");
    }
    println!("en-us key but no de-de text:");
    for l in untranslated.iter().filter(|l| en.key_for(l).is_some()) {
        println!("  {l:?}");
    }
    println!("de-de translations (check the key picked fits the meaning):");
    for l in labels.iter().filter(|l| de.tr(l) != **l) {
        println!("  {l:?} -> {:?} [{}]", de.tr(l), de.key_for(l).unwrap_or_default());
    }
    println!("Not in de-de:");
    for l in &untranslated {
        println!("  {l:?}");
    }
}
