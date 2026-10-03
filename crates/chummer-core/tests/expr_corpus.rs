//! Evaluate every cost/availability/essence string in the data at rating 1.
//! Strings that depend on parent items, vehicles or user choice are skipped;
//! everything else must evaluate.

use chummer_core::data::DataStore;
use chummer_core::expr::{self, NoAttributes};
use chummer_core::xml::Element;

const CONTEXT_TOKENS: &[&str] = &[
    "Parent", "Gear", "Children", "Weapon", "Armor", "Vehicle", "Body", "Speed", "Handling", "Accel",
    "Sensor", "Pilot", "Slots", "Seats", "Capacity", "Variable", "{", "Cost", "Level", "Weight",
];

fn walk(e: &Element, tags: &[&str], out: &mut Vec<String>) {
    for c in e.elements() {
        if tags.contains(&c.name.as_str()) && c.elements().next().is_none() {
            out.push(c.text());
        }
        walk(c, tags, out);
    }
}

#[test]
fn data_expressions_evaluate() {
    let store = DataStore::discover().unwrap();
    let mut total = 0;
    let mut failures = Vec::new();
    for file in ["armor.xml", "bioware.xml", "cyberware.xml", "gear.xml", "weapons.xml", "vehicles.xml", "drugcomponents.xml", "lifestyles.xml"] {
        let doc = store.doc(file).unwrap();
        let mut vals = Vec::new();
        walk(&doc, &["cost", "ess"], &mut vals);
        for v in vals {
            let v = v.trim();
            if v.is_empty() || CONTEXT_TOKENS.iter().any(|t| v.contains(t)) {
                continue;
            }
            total += 1;
            let s = expr::fixed_values(v, 1).replace("MinRating", "1").replace("Rating", "1");
            if expr::evaluate_num(&s).is_err() {
                failures.push(format!("{file}: {v:?}"));
            }
        }
    }
    assert!(total > 3000, "only {total} expressions checked");
    assert!(failures.is_empty(), "{} of {total} failed:\n{}", failures.len(), failures.join("\n"));
    let _ = NoAttributes;
}
