//! Dice roller window.

use chummer_core::dice::{self, Glitch, Rng};
use chummer_core::lang::Language;
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

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language) {
        ui.horizontal(|ui| {
            ui.label(lang.tr("Dice"));
            ui.add(egui::DragValue::new(&mut self.pool).range(1..=100));
            ui.checkbox(&mut self.use_limit, lang.tr("Limit"));
            ui.add_enabled(self.use_limit, egui::DragValue::new(&mut self.limit).range(0..=50));
            ui.checkbox(&mut self.rule_of_six, lang.tr("Rule of Six (Edge)"));
        });
        if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Roll"))).clicked() {
            let r = dice::roll(&mut self.rng, self.pool, self.rule_of_six, self.use_limit.then_some(self.limit));
            self.history.insert(0, format!("{}d6 → {}", self.pool, hits_text(lang, r.hits, r.glitch)));
            self.history.truncate(30);
            self.last = Some(r);
        }
        if let Some(r) = &self.last {
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
                ui.label(h);
            }
        }
    }
}

fn hits_text(lang: &Language, hits: u32, g: Glitch) -> String {
    let glitch = match g {
        Glitch::None => String::new(),
        Glitch::Glitch => format!(" — {}", lang.tr("GLITCH")),
        Glitch::Critical => format!(" — {}", lang.tr("CRITICAL GLITCH")),
    };
    format!("{}{glitch}", lang.tr_fmt("{0} hits", &[&hits]))
}
