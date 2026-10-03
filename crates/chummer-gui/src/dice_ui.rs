//! Dice roller window.

use chummer_core::dice::{self, Glitch, Rng};
use eframe::egui;

pub struct DiceRoller {
    pool: u32,
    limit: u32,
    use_limit: bool,
    rule_of_six: bool,
    rng: Rng,
    history: Vec<String>,
    last: Option<dice::Roll>,
}

impl Default for DiceRoller {
    fn default() -> Self {
        Self { pool: 6, limit: 6, use_limit: false, rule_of_six: false, rng: Rng::from_time(), history: Vec::new(), last: None }
    }
}

impl DiceRoller {
    pub fn set_pool(&mut self, pool: u32) {
        self.pool = pool;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Dice");
            ui.add(egui::DragValue::new(&mut self.pool).range(1..=100));
            ui.checkbox(&mut self.use_limit, "Limit");
            ui.add_enabled(self.use_limit, egui::DragValue::new(&mut self.limit).range(0..=50));
            ui.checkbox(&mut self.rule_of_six, "Rule of Six (Edge)");
        });
        if ui.button("🎲 Roll").clicked() {
            let r = dice::roll(&mut self.rng, self.pool, self.rule_of_six, self.use_limit.then_some(self.limit));
            self.history.insert(0, format!("{}d6 → {} hits{}", self.pool, r.hits, glitch_text(r.glitch)));
            self.history.truncate(30);
            self.last = Some(r);
        }
        if let Some(r) = &self.last {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for d in &r.dice {
                    let color = match d {
                        5 | 6 => egui::Color32::from_rgb(80, 200, 120),
                        1 => egui::Color32::from_rgb(220, 80, 80),
                        _ => ui.visuals().weak_text_color(),
                    };
                    ui.label(egui::RichText::new(d.to_string()).monospace().size(18.0).color(color));
                }
            });
            ui.heading(format!("{} hits{}", r.hits, glitch_text(r.glitch)));
        }
        if !self.history.is_empty() {
            ui.separator();
            ui.weak("History");
            for h in &self.history {
                ui.label(h);
            }
        }
    }
}

fn glitch_text(g: Glitch) -> &'static str {
    match g {
        Glitch::None => "",
        Glitch::Glitch => " — GLITCH",
        Glitch::Critical => " — CRITICAL GLITCH",
    }
}
