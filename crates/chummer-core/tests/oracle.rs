//! Compare computed values with the totals Chummer5a wrote into each
//! fixture (`<totalvalue>` per attribute, `<totaless>`).

use std::path::PathBuf;

use chummer_core::calc::{self, Rules, SkillCatalog};
use chummer_core::character::Character;
use chummer_core::data::DataStore;

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "chum5")).collect();
    v.sort();
    v
}

#[test]
fn attributes_and_essence_match_chummer() {
    let store = DataStore::discover().unwrap();
    let cat = SkillCatalog::load(&store).unwrap();
    let mut checked = 0;
    let mut bad = Vec::new();
    for path in fixtures() {
        let ch = Character::load(&path).unwrap();
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        // These were saved with house-rule settings files we do not have:
        // a 5-limb count (head excluded) and 3-decimal essence.
        let mut rules = Rules::default();
        match fname.as_str() {
            "Bastion.chum5" | "Blindfire.chum5" => rules.limb_count = 5,
            "Fuzzy-chargen.chum5" => rules.essence_decimals = 3,
            _ => {}
        }
        let sheet = calc::compute(&ch, &rules, Some(&store), Some(&cat));
        for a in &ch.attributes {
            if a.name == "ESS" || a.category == "Shapeshifter" {
                continue;
            }
            if let Some(saved) = a.saved_total {
                checked += 1;
                let got = sheet.attr(&a.name);
                if got != saved {
                    bad.push(format!("{fname} {}: got {got}, saved {saved} ({:?})", a.name, sheet.attr_values(&a.name)));
                }
            }
        }
        if let Some(saved) = ch.doc.get_f64("totaless") {
            checked += 1;
            if (sheet.essence - saved).abs() > 0.005 {
                bad.push(format!("{fname} ESS: got {:.4}, saved {saved}", sheet.essence));
            }
        }
    }
    eprintln!("{} mismatches of {checked}", bad.len());
    for b in &bad {
        eprintln!("  {b}");
    }
    assert!(checked > 400);
    assert!(bad.is_empty(), "{} of {checked} values differ", bad.len());
}
