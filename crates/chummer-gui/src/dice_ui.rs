//! Dice roller window, and the roller and roll log of the Workspace's
//! Play screen (`workspace/play.rs`), which keeps one per character (and,
//! for a character of an online campaign, reports each roll to the GM:
//! [`DiceRoller::take_new`]).

use chummer_core::dice::{self, Glitch, Rng};
use chummer_core::lang::Language;
use eframe::egui;

/// What a logged roll came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A pool of d6 counted for hits.
    Hits(dice::Roll),
    /// Initiative: base + the dice rolled.
    Initiative { score: i32, dice: Vec<u8> },
    /// Something else for the log (damage taken).
    Note(String),
}

/// One line of the roll history, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// What was rolled ("Pistols · Ares Predator V"); empty for a plain
    /// roll of the window.
    pub label: String,
    pub pool: u32,
    pub limit: Option<u32>,
    /// Rolled with the Rule of Six (Edge).
    pub rule_of_six: bool,
    pub outcome: Outcome,
    /// Unix ms.
    pub at: i64,
}

impl Entry {
    /// The roll as the campaign logs it; `None` for a note.
    pub fn record(&self) -> Option<dice::RollRecord> {
        let base = dice::RollRecord { at: self.at, label: self.label.clone(), pool: self.pool, limit: self.limit, rule_of_six: self.rule_of_six, ..Default::default() };
        match &self.outcome {
            Outcome::Hits(r) => Some(dice::RollRecord { dice: r.dice.clone(), ..base }),
            Outcome::Initiative { score, dice } => {
                let sum: i32 = dice.iter().map(|&d| i32::from(d)).sum();
                Some(dice::RollRecord { pool: dice.len() as u32, initiative: Some(score - sum), dice: dice.clone(), limit: None, rule_of_six: false, ..base })
            }
            Outcome::Note(_) => None,
        }
    }

    /// The result: "3 hits", "1 hits — GLITCH", the initiative score.
    pub fn result(&self, lang: &Language) -> String {
        match &self.outcome {
            Outcome::Hits(r) => hits_text(lang, r.hits, r.glitch),
            Outcome::Initiative { score, .. } => score.to_string(),
            // "took 6P …: 2 Physical" → "2 Physical".
            Outcome::Note(t) => t.rsplit(": ").next().unwrap_or(t).to_owned(),
        }
    }

    /// The history line of the dice roller window.
    pub fn line(&self, lang: &Language) -> String {
        let what = match &self.outcome {
            Outcome::Hits(_) => format!("{}d6 → {}", self.pool, self.result(lang)),
            Outcome::Initiative { dice, .. } => {
                let d: Vec<String> = dice.iter().map(u8::to_string).collect();
                format!("{} [{}]", self.result(lang), d.join(" "))
            }
            Outcome::Note(t) => t.clone(),
        };
        if self.label.is_empty() {
            what
        } else {
            format!("{}: {what}", self.label)
        }
    }
}

/// Rolls kept in the history.
const HISTORY: usize = 30;

pub struct DiceRoller {
    pub pool: u32,
    pub limit: u32,
    pub use_limit: bool,
    pub rule_of_six: bool,
    rng: Rng,
    history: Vec<Entry>,
    /// Rolls made since [`DiceRoller::take_new`].
    new: Vec<Entry>,
}

impl Default for DiceRoller {
    fn default() -> Self {
        Self { pool: 6, limit: 6, use_limit: false, rule_of_six: false, rng: Rng::from_time(), history: Vec::new(), new: Vec::new() }
    }
}

impl DiceRoller {
    pub fn set_pool(&mut self, pool: u32) {
        self.pool = pool;
    }

    /// Newest first.
    pub fn history(&self) -> &[Entry] {
        &self.history
    }

    pub fn last(&self) -> Option<&Entry> {
        self.history.first()
    }

    fn push(&mut self, e: Entry) {
        if !matches!(e.outcome, Outcome::Note(_)) {
            self.new.push(e.clone());
        }
        self.history.insert(0, e);
        self.history.truncate(HISTORY);
    }

    /// The rolls made since the last call, oldest first (notes are not
    /// rolls).
    pub fn take_new(&mut self) -> Vec<Entry> {
        std::mem::take(&mut self.new)
    }

    /// Log a roll made another way (a soak roll).
    pub fn logged(&mut self, label: &str, pool: u32, r: dice::Roll) {
        self.push(Entry { label: label.to_owned(), pool, limit: None, rule_of_six: false, outcome: Outcome::Hits(r), at: chummer_core::campaign::now_ms() });
    }

    /// Roll the pool, limit and Rule of Six set in the roller.
    pub fn roll(&mut self) {
        let label = self.last().filter(|e| e.pool == self.pool && matches!(e.outcome, Outcome::Hits(_))).map(|e| e.label.clone()).unwrap_or_default();
        self.roll_as(&label, self.pool, self.use_limit.then_some(self.limit));
    }

    /// Roll `pool` dice for `label` (a quick roll); the roller keeps the
    /// pool and limit for rolling again.
    pub fn roll_as(&mut self, label: &str, pool: u32, limit: Option<u32>) {
        self.pool = pool;
        self.use_limit = limit.is_some();
        if let Some(l) = limit {
            self.limit = l;
        }
        let r = dice::roll(&mut self.rng, pool, self.rule_of_six, limit);
        self.push(Entry { label: label.to_owned(), pool, limit, rule_of_six: self.rule_of_six, outcome: Outcome::Hits(r), at: chummer_core::campaign::now_ms() });
    }

    /// Roll initiative (`base` + `dice`d6) for the log; returns the score
    /// and the dice.
    pub fn initiative(&mut self, label: &str, base: i32, dice: u32) -> (i32, Vec<u8>) {
        let (score, rolled) = dice::initiative(&mut self.rng, base, dice);
        self.push(Entry { label: label.to_owned(), pool: dice, limit: None, rule_of_six: false, outcome: Outcome::Initiative { score, dice: rolled.clone() }, at: chummer_core::campaign::now_ms() });
        (score, rolled)
    }

    /// Log something that is not a roll (damage taken).
    pub fn note(&mut self, label: &str, text: String) {
        self.push(Entry { label: label.to_owned(), pool: 0, limit: None, rule_of_six: false, outcome: Outcome::Note(text), at: chummer_core::campaign::now_ms() });
    }

    /// The random source, for rolls logged another way (a soak roll).
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language) {
        ui.horizontal(|ui| {
            ui.label(lang.tr("Dice"));
            ui.add(egui::DragValue::new(&mut self.pool).range(1..=100));
            ui.checkbox(&mut self.use_limit, lang.tr("Limit"));
            ui.add_enabled(self.use_limit, egui::DragValue::new(&mut self.limit).range(0..=50));
            ui.checkbox(&mut self.rule_of_six, lang.tr("Rule of Six (Edge)"));
        });
        if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Roll"))).clicked() {
            self.roll_as("", self.pool, self.use_limit.then_some(self.limit));
        }
        if let Some(Entry { outcome: Outcome::Hits(r), .. }) = self.history.first() {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for d in &r.dice {
                    let color = match d {
                        5 | 6 => crate::theme::palette(ui).good,
                        1 => crate::theme::palette(ui).bad,
                        _ => ui.visuals().weak_text_color(),
                    };
                    ui.label(egui::RichText::new(d.to_string()).monospace().size(18.0).color(color));
                }
            });
            ui.heading(hits_text(lang, r.hits, r.glitch));
        }
        if !self.history.is_empty() {
            ui.separator();
            ui.weak(lang.tr("Roll history"));
            for h in &self.history {
                ui.label(h.line(lang));
            }
        }
    }
}

pub fn hits_text(lang: &Language, hits: u32, g: Glitch) -> String {
    let glitch = match g {
        Glitch::None => String::new(),
        Glitch::Glitch => format!(" — {}", lang.tr("GLITCH")),
        Glitch::Critical => format!(" — {}", lang.tr("CRITICAL GLITCH")),
    };
    format!("{}{glitch}", lang.tr_fmt("{0} hits", &[&hits]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_are_logged_newest_first() {
        let lang = Language::default();
        let mut d = DiceRoller::default();
        d.roll_as("Pistols", 9, Some(5));
        assert_eq!((d.pool, d.limit, d.use_limit), (9, 5, true), "the roller keeps the quick roll's pool and limit");
        let Some(Entry { outcome: Outcome::Hits(r), .. }) = d.last() else { panic!("a roll") };
        assert!(r.hits <= 5, "hits are capped by the limit");
        assert_eq!(r.dice.len(), 9);
        d.roll();
        assert_eq!(d.last().unwrap().label, "Pistols", "rolling again keeps the label");
        let (score, dice) = d.initiative("Initiative", 9, 2);
        assert_eq!(score, 9 + dice.iter().map(|&x| i32::from(x)).sum::<i32>());
        assert_eq!(d.history().len(), 3);
        assert!(d.last().unwrap().line(&lang).starts_with(&format!("Initiative: {score} [")));
        d.set_pool(4);
        d.use_limit = false;
        d.roll();
        assert_eq!(d.last().unwrap().label, "", "a new pool is a plain roll");
        assert!(d.last().unwrap().line(&lang).starts_with("4d6 → "));
        d.note("Damage", "took 6P: 2 Physical".into());
        assert_eq!(d.last().unwrap().line(&lang), "Damage: took 6P: 2 Physical");
        assert_eq!(d.last().unwrap().result(&lang), "2 Physical");
        for _ in 0..40 {
            d.roll();
        }
        assert_eq!(d.history().len(), HISTORY);
    }

    #[test]
    fn new_rolls_are_taken_once_as_records() {
        let mut d = DiceRoller { rule_of_six: true, ..Default::default() };
        d.roll_as("Longarms + Agility", 8, Some(4));
        d.rule_of_six = false;
        let (score, dice) = d.initiative("Initiative", 9, 2);
        d.note("Damage", "took 6P: 2 Physical".into());
        let new = d.take_new();
        assert_eq!(new.len(), 2, "notes are not rolls");
        assert!(d.take_new().is_empty(), "taken once");
        let r = new[0].record().unwrap();
        assert_eq!((r.label.as_str(), r.pool, r.limit, r.rule_of_six), ("Longarms + Agility", 8, Some(4), true));
        assert_eq!(r.check(), Ok(()), "the dice fit the pool: {r:?}");
        let i = new[1].record().unwrap();
        assert_eq!((i.score(), i.dice.clone(), i.check()), (Some(score), dice, Ok(())));
    }
}
