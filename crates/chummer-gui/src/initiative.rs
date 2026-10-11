//! Initiative tracker: combatants, rolls, passes (-10 per pass), turn
//! order and initiative adjustments, on a core [`Encounter`] (the GM
//! screen's rules, without a campaign).

use chummer_core::campaign::boosts::Suggestion;
use chummer_core::campaign::{Combatant, Encounter, InitAdjust};
use chummer_core::dice::Rng;
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

use crate::init_adjust::{self, AdjustForm};

/// An open character, for the tracker's quick add.
pub struct OpenCharacter {
    pub name: String,
    pub base: i32,
    pub dice: u32,
    /// Its conditional initiative bonuses.
    pub boosts: Vec<Suggestion>,
}

pub struct Tracker {
    enc: Encounter,
    /// The suggested bonuses of each combatant (by position), taken when
    /// it was added.
    boosts: Vec<Vec<Suggestion>>,
    new_name: String,
    new_base: i32,
    new_dice: u32,
    form: AdjustForm,
    rng: Rng,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker { enc: Encounter::new(""), boosts: Vec::new(), new_name: String::new(), new_base: 8, new_dice: 1, form: AdjustForm::default(), rng: Rng::from_time() }
    }
}

impl Tracker {
    /// Add a combatant with its initiative (base + dice) and suggested
    /// bonuses; during a round it rolls at once.
    pub fn add(&mut self, name: String, base: i32, dice: u32, boosts: Vec<Suggestion>) {
        let c = Combatant { track: None, ..Combatant::ad_hoc(name, base, dice, 0, 0) };
        self.enc.join(c, &mut self.rng, None);
        self.boosts.push(boosts);
    }

    fn remove(&mut self, i: usize) {
        self.enc.combatants.remove(i);
        self.boosts.remove(i);
    }

    pub fn adjust(&mut self, i: usize, a: InitAdjust) {
        self.enc.adjust(i, a, &mut self.rng);
    }

    #[cfg(test)]
    pub fn encounter(&self) -> &Encounter {
        &self.enc
    }

    #[cfg(test)]
    pub fn roll_all(&mut self) {
        self.enc.new_round(&mut self.rng, |_| None);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language, open_characters: &[OpenCharacter]) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text(lang.tr("Name")).desired_width(140.0));
            ui.label(lang.tr("Base"));
            ui.add(egui::DragValue::new(&mut self.new_base).range(0..=40));
            ui.label(lang.tr("Dice"));
            ui.add(egui::DragValue::new(&mut self.new_dice).range(1..=5));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                self.add(self.new_name.trim().to_owned(), self.new_base, self.new_dice, Vec::new());
                self.new_name.clear();
            }
        });
        if !open_characters.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.weak(lang.tr("Open characters:"));
                for c in open_characters {
                    if ui.small_button(format!("+ {}", c.name)).clicked() {
                        self.add(c.name.clone(), c.base, c.dice, c.boosts.clone());
                    }
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Roll initiative"))).clicked() {
                self.enc.new_round(&mut self.rng, |_| None);
            }
            if ui.add_enabled(self.enc.pass > 0 && self.enc.has_next_pass(), egui::Button::new(lang.tr("Next pass"))).clicked() {
                self.enc.next_pass();
            }
            if ui.button(lang.tr("Clear")).clicked() {
                self.enc = Encounter::new("");
                self.boosts.clear();
            }
            if self.enc.pass > 0 {
                ui.label(RichText::new(format!("{}  ·  {}", lang.tr_fmt("Round {0}", &[&self.enc.round]), lang.tr_fmt("Pass {0}", &[&self.enc.pass]))).strong());
            }
        });
        let mut remove = None;
        let mut adjust = None;
        let mut unadjust = None;
        let pass = self.enc.pass;
        let order = self.enc.order();
        egui::Grid::new("init").striped(true).num_columns(6).show(ui, |ui| {
            for h in lang.tr_all(["Name", "Initiative", "Score", "Acted", "", ""]) {
                ui.strong(h);
            }
            ui.end_row();
            for i in order {
                let c = &mut self.enc.combatants[i];
                let active = pass > 0 && c.score > 0;
                let name = RichText::new(&c.name);
                ui.label(if active && !c.acted { name.color(crate::theme::accent(ui)).strong() } else if active { name } else { name.weak() });
                ui.label(format!("{} + {}d6", c.base, c.dice));
                ui.add(egui::DragValue::new(&mut c.score));
                ui.checkbox(&mut c.acted, "");
                let c = &self.enc.combatants[i];
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
                    if let Some(k) = init_adjust::chips(ui, lang, &c.adjust) {
                        unadjust = Some((i, k));
                    }
                    ui.push_id(("tracker_adjust", c.id), |ui| {
                        if let Some(a) = init_adjust::button(ui, lang, &mut self.form, &self.boosts[i], &c.adjust, format!("± {}", lang.tr("Adjust"))) {
                            adjust = Some((i, a));
                        }
                    });
                });
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some((i, a)) = adjust {
            self.adjust(i, a);
        }
        if let Some((i, k)) = unadjust {
            self.enc.unadjust(i, k);
        }
        if let Some(i) = remove {
            self.remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chummer_core::campaign::Lasts;

    #[test]
    fn tracker_adjusts_and_reorders() {
        let mut t = Tracker::default();
        t.add("a".into(), 10, 1, Vec::new());
        t.add("b".into(), 9, 1, Vec::new());
        t.roll_all();
        let e = t.encounter();
        let (sa, sb) = (e.combatants[0].score, e.combatants[1].score);
        t.adjust(1, InitAdjust::new("Boost", sa - sb + 10, 0, Lasts::Turn));
        let e = t.encounter();
        assert_eq!(e.order(), [1, 0], "b goes first after its boost");
        assert_eq!(e.combatants[1].score, sa + 10);
    }
}
