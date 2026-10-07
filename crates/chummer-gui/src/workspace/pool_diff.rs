//! The inline catalog's dice-pool preview: what a purchase changes in
//! the pools rolled at the table.
//!
//! [`diff`] compares the character before and after (a copy with the
//! purchase applied, see `catalog.rs`) and keeps only what changed: the
//! fixed pools (Defense, Damage Resistance, Composure, Judge Intentions,
//! Memory, Lift/Carry, the astral and Matrix initiative), the active
//! skills (pool, with the specialization pool), and every weapon (dice
//! pool, damage, AP, accuracy). Values come from the engine's own
//! functions (`calc::Sheet`, `calc::soak_body`, `items::weapon::stats`),
//! the same ones the Play screen shows.

use chummer_core::calc::{self, Sheet};
use chummer_core::character::Character;
use chummer_core::items::weapon;

/// What a [`Line`] is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A pool of the sheet (English label; goes through `lang.tr`).
    Fixed(&'static str),
    /// An active skill (its data name).
    Skill(String),
    /// A weapon: its saved name and whether the purchase adds it.
    Weapon { name: String, new: bool },
}

/// Which value of a weapon changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Pool,
    Damage,
    Ap,
    Accuracy,
}

impl Field {
    /// The label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            Field::Pool => "Dice Pool",
            Field::Damage => "DV",
            Field::Ap => "AP",
            Field::Accuracy => "Accuracy",
        }
    }
}

/// One value that changed: before (`None`: did not exist) and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub field: Field,
    pub before: Option<String>,
    pub after: String,
}

/// A pool, skill or weapon that changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub subject: Subject,
    pub parts: Vec<Part>,
}

impl Line {
    /// A compact English form, e.g. "Pistols 12 → 14" or
    /// "Ares Predator V: Dice Pool 12 → 14, Accuracy 5 → 7" (tests).
    #[cfg(test)]
    pub fn text(&self) -> String {
        let part = |p: &Part| match &p.before {
            Some(b) => format!("{b} → {}", p.after),
            None => p.after.clone(),
        };
        match &self.subject {
            Subject::Fixed(l) => format!("{l} {}", self.parts.first().map(part).unwrap_or_default()),
            Subject::Skill(n) => format!("{n} {}", self.parts.first().map(part).unwrap_or_default()),
            Subject::Weapon { name, .. } => {
                let parts: Vec<String> = self.parts.iter().map(|p| format!("{} {}", p.field.label(), part(p))).collect();
                format!("{name}: {}", parts.join(", "))
            }
        }
    }
}

/// The pools of a character (`ch` with its computed `sheet`), keyed for
/// comparison.
struct Pools {
    fixed: Vec<(&'static str, String)>,
    /// (guid or name, name, rated, shown pool).
    skills: Vec<(String, String, bool, String)>,
    /// (guid, name, [pool, damage, AP, accuracy]).
    weapons: Vec<(String, String, [String; 4])>,
}

fn pools(ch: &Character, s: &Sheet) -> Pools {
    let init = |base: i32, dice: i32| format!("{base} + {dice}d6");
    let fixed = vec![
        ("Defense", (s.attr("REA") + s.attr("INT") + s.wound_modifier).to_string()),
        ("Damage Resistance", (calc::soak_body(ch, s) + s.armor).to_string()),
        ("Composure", s.composure.to_string()),
        ("Judge Intentions", s.judge_intentions.to_string()),
        ("Memory", s.memory.to_string()),
        ("Lift/Carry", s.lift_carry.to_string()),
        ("Astral Initiative", init(s.astral_initiative, s.astral_initiative_dice)),
        ("Matrix AR Initiative", init(s.matrix_cold_initiative, s.matrix_cold_dice)),
        ("Matrix VR Initiative (Hot Sim)", init(s.matrix_hot_initiative, s.matrix_hot_dice)),
    ];
    let skills = s
        .skills
        .iter()
        .filter(|k| !k.disabled)
        .map(|k| {
            let key = if k.guid.is_empty() { k.name.clone() } else { k.guid.clone() };
            let shown = if !k.specs.is_empty() && k.spec_bonus > 0 { format!("{} ({})", k.pool, k.pool + k.spec_bonus) } else { k.pool.to_string() };
            (key, k.name.clone(), k.rating > 0, shown)
        })
        .collect();
    let weapons = ch
        .items("weapons", "weapon")
        .into_iter()
        .map(|w| {
            let st = weapon::stats(ch, s, w);
            (w.get("guid"), w.get("name"), [st.dice_pool.to_string(), st.damage, st.ap, st.accuracy.to_string()])
        })
        .collect();
    Pools { fixed, skills, weapons }
}

/// What changed between `before` and `after` (each a character with
/// its sheet): the fixed pools, then the active skills (rated before or
/// after), then the weapons, in the character's order.
pub fn diff(before: (&Character, &Sheet), after: (&Character, &Sheet)) -> Vec<Line> {
    let (b, a) = (pools(before.0, before.1), pools(after.0, after.1));
    let mut out = Vec::new();
    for ((label, x), (_, y)) in b.fixed.iter().zip(&a.fixed) {
        if x != y {
            out.push(Line { subject: Subject::Fixed(label), parts: vec![Part { field: Field::Pool, before: Some(x.clone()), after: y.clone() }] });
        }
    }
    for (key, name, rated, y) in &a.skills {
        let old = b.skills.iter().find(|(k, ..)| k == key);
        let was_rated = old.is_some_and(|(_, _, r, _)| *r);
        if !rated && !was_rated {
            continue;
        }
        let x = old.map(|(.., v)| v.clone());
        if x.as_ref() != Some(y) {
            out.push(Line { subject: Subject::Skill(name.clone()), parts: vec![Part { field: Field::Pool, before: x, after: y.clone() }] });
        }
    }
    const FIELDS: [Field; 4] = [Field::Pool, Field::Damage, Field::Ap, Field::Accuracy];
    for (guid, name, y) in &a.weapons {
        let old = b.weapons.iter().find(|(g, ..)| g == guid).map(|(.., v)| v);
        let parts: Vec<Part> = FIELDS
            .iter()
            .zip(y)
            .enumerate()
            .filter(|(i, (_, v))| old.is_none_or(|o| o[*i] != **v))
            .map(|(i, (f, v))| Part { field: *f, before: old.map(|o| o[i].clone()), after: v.clone() })
            .collect();
        if !parts.is_empty() {
            out.push(Line { subject: Subject::Weapon { name: name.clone(), new: old.is_none() }, parts });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chummer_core::command::{self, Command, Envelope, RecordRef};
    use chummer_core::data::{self, Record};
    use chummer_core::engine::Engine;
    use chummer_core::items::Purchase;

    fn munin() -> Option<(Engine, Character)> {
        let engine = Engine::load().ok()?;
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        Some((engine, Character::load(&p).unwrap()))
    }

    /// `ch` with the `tag` record `name` added for free.
    fn with(engine: &Engine, ch: &Character, tag: &str, name: &str, p: Purchase) -> Character {
        let kind = chummer_core::items::kind(tag).unwrap();
        let store = engine.store_for_character(ch);
        let doc = store.doc(kind.file).unwrap();
        let rec = data::records(&doc, kind.data_container, kind.data_item).into_iter().find(|r| r.name() == name).unwrap_or_else(|| panic!("{name}"));
        let mut copy = ch.clone();
        let cmd = Command::AddItem { tag: tag.to_owned(), record: RecordRef::of(Record(rec.el())), purchase: Purchase { qty: 1.0, cost_multiplier: 1.0, free: true, ..p } };
        command::apply(&mut copy, engine, &Envelope::new(cmd, 0, 0, "")).unwrap();
        copy
    }

    #[test]
    fn nothing_changes_nothing_shows() {
        let Some((engine, ch)) = munin() else { return };
        let s = engine.sheet(&ch);
        assert!(diff((&ch, &s), (&ch, &s)).is_empty());
    }

    #[test]
    fn agility_raises_skills_and_weapons() {
        let Some((engine, ch)) = munin() else { return };
        let before = engine.sheet(&ch);
        let after_ch = with(&engine, &ch, "bioware", "Muscle Toner", Purchase { rating: 2, ..Default::default() });
        let after = engine.sheet(&after_ch);
        assert!(after.attr("AGI") > before.attr("AGI"), "the toner raises Agility");
        let lines = diff((&ch, &before), (&after_ch, &after));
        let texts: Vec<String> = lines.iter().map(Line::text).collect();
        // Every rated Agility skill shows, with its old and new pool.
        for k in before.skills.iter().filter(|k| k.attribute == "AGI" && k.rating > 0 && !k.disabled) {
            let new = after.skills.iter().find(|x| x.guid == k.guid).unwrap();
            let line = lines.iter().find(|l| l.subject == Subject::Skill(k.name.clone())).unwrap_or_else(|| panic!("{} missing in {texts:?}", k.name));
            assert_eq!(line.parts[0].before.as_deref().map(|s| s.split(' ').next().unwrap().parse::<i32>().unwrap()), Some(k.pool));
            assert!(line.parts[0].after.starts_with(&new.pool.to_string()));
        }
        // Skills on other attributes do not.
        assert!(!lines.iter().any(|l| matches!(&l.subject, Subject::Skill(n) if before.skills.iter().any(|k| &k.name == n && k.attribute == "LOG"))), "{texts:?}");
        // Ranged weapons using an Agility skill: only the pool changes.
        let weapons: Vec<&Line> = lines.iter().filter(|l| matches!(l.subject, Subject::Weapon { .. })).collect();
        if ch.items("weapons", "weapon").iter().any(|w| w.get("type") == "Ranged") {
            assert!(!weapons.is_empty(), "{texts:?}");
        }
        for w in &weapons {
            assert!(matches!(w.subject, Subject::Weapon { new: false, .. }));
            assert!(w.parts.iter().any(|p| p.field == Field::Pool), "{}", w.text());
        }
        // Defense is REA + INT: unchanged.
        assert!(!lines.iter().any(|l| l.subject == Subject::Fixed("Defense")));
    }

    #[test]
    fn a_new_weapon_shows_its_values() {
        let Some((engine, ch)) = munin() else { return };
        let before = engine.sheet(&ch);
        let after_ch = with(&engine, &ch, "weapon", "Ares Predator V", Purchase::default());
        let after = engine.sheet(&after_ch);
        let lines = diff((&ch, &before), (&after_ch, &after));
        let w = lines.iter().find(|l| matches!(&l.subject, Subject::Weapon { name, new: true } if name == "Ares Predator V")).expect("the new weapon");
        assert_eq!(w.parts.len(), 4, "all of its values");
        assert!(w.parts.iter().all(|p| p.before.is_none()));
        assert!(w.text().starts_with("Ares Predator V: Dice Pool "), "{}", w.text());
    }

    #[test]
    fn line_text() {
        let l = Line { subject: Subject::Skill("Pistols".into()), parts: vec![Part { field: Field::Pool, before: Some("12".into()), after: "14".into() }] };
        assert_eq!(l.text(), "Pistols 12 → 14");
        let w = Line { subject: Subject::Weapon { name: "Ares Predator V".into(), new: false }, parts: vec![Part { field: Field::Pool, before: Some("12".into()), after: "14".into() }, Part { field: Field::Accuracy, before: Some("5".into()), after: "7".into() }] };
        assert_eq!(w.text(), "Ares Predator V: Dice Pool 12 → 14, Accuracy 5 → 7");
    }
}
