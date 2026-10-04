//! Initiative tracker: combatants, rolls, passes (-10 per pass), turn order.

use chummer_core::dice::{self, Rng};
use eframe::egui::{self, RichText};

use crate::view::ACCENT;

struct Combatant {
    name: String,
    base: i32,
    dice: u32,
    score: i32,
    acted: bool,
}

pub struct Tracker {
    list: Vec<Combatant>,
    new_name: String,
    new_base: i32,
    new_dice: u32,
    pass: u32,
    rng: Rng,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker { list: Vec::new(), new_name: String::new(), new_base: 8, new_dice: 1, pass: 0, rng: Rng::from_time() }
    }
}

impl Tracker {
    /// Add an open character with its initiative (base + dice).
    pub fn add(&mut self, name: String, base: i32, dice: u32) {
        self.list.push(Combatant { name, base, dice, score: 0, acted: false });
    }

    fn roll_all(&mut self) {
        for c in &mut self.list {
            c.score = dice::initiative(&mut self.rng, c.base, c.dice).0;
            c.acted = false;
        }
        self.pass = 1;
        self.sort();
    }

    fn sort(&mut self) {
        self.list.sort_by(|a, b| b.score.cmp(&a.score).then(b.base.cmp(&a.base)));
    }

    /// Next initiative pass: everyone loses 10; those at 0 or less are out.
    fn next_pass(&mut self) {
        for c in &mut self.list {
            c.score -= 10;
            c.acted = false;
        }
        self.pass += 1;
        self.sort();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, open_characters: &[(String, i32, u32)]) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Name").desired_width(140.0));
            ui.label("Base");
            ui.add(egui::DragValue::new(&mut self.new_base).range(0..=40));
            ui.label("Dice");
            ui.add(egui::DragValue::new(&mut self.new_dice).range(1..=5));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new("Add")).clicked() {
                self.add(self.new_name.trim().to_owned(), self.new_base, self.new_dice);
                self.new_name.clear();
            }
        });
        if !open_characters.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.weak("Open characters:");
                for (n, b, d) in open_characters {
                    if ui.small_button(format!("+ {n}")).clicked() {
                        self.add(n.clone(), *b, *d);
                    }
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("🎲 Roll initiative").clicked() {
                self.roll_all();
            }
            let any_left = self.list.iter().any(|c| c.score - 10 > 0);
            if ui.add_enabled(self.pass > 0 && any_left, egui::Button::new("Next pass")).clicked() {
                self.next_pass();
            }
            if ui.button("Clear").clicked() {
                self.list.clear();
                self.pass = 0;
            }
            if self.pass > 0 {
                ui.label(RichText::new(format!("Pass {}", self.pass)).strong());
            }
        });
        let mut remove = None;
        egui::Grid::new("init").striped(true).num_columns(5).show(ui, |ui| {
            for h in ["Name", "Initiative", "Score", "Acted", ""] {
                ui.strong(h);
            }
            ui.end_row();
            for (i, c) in self.list.iter_mut().enumerate() {
                let active = self.pass > 0 && c.score > 0;
                let name = RichText::new(&c.name);
                ui.label(if active && !c.acted { name.color(ACCENT).strong() } else if active { name } else { name.weak() });
                ui.label(format!("{} + {}d6", c.base, c.dice));
                ui.add(egui::DragValue::new(&mut c.score));
                ui.checkbox(&mut c.acted, "");
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            self.list.remove(i);
        }
    }
}
