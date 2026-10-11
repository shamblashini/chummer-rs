//! Initiative adjustments on the boards (the GM screen's encounter in
//! both layouts, Tools → Initiative tracker, the Play screen): the chips
//! of the adjustments in effect, and the menu that adds one, from the
//! character's suggested bonuses ([`chummer_core::campaign::boosts`]) or
//! typed in (score, dice, how long, label).

use chummer_core::campaign::boosts::Suggestion;
use chummer_core::campaign::{InitAdjust, Lasts};
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

use crate::workspace::icons;

/// The typed-in adjustment, kept between frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustForm {
    pub label: String,
    pub score: i32,
    pub dice: i32,
    pub roll_now: bool,
    pub lasts: Lasts,
    /// The count for [`Lasts::Turns`].
    pub turns: u32,
    /// Filled in from a suggestion (kept as its source).
    pub source: String,
}

impl Default for AdjustForm {
    fn default() -> Self {
        AdjustForm { label: String::new(), score: 0, dice: 0, roll_now: true, lasts: Lasts::Turn, turns: 2, source: String::new() }
    }
}

impl AdjustForm {
    fn take(&mut self, a: &InitAdjust) {
        self.label = a.label.clone();
        self.score = a.score;
        self.dice = a.dice;
        self.roll_now = a.roll_now;
        self.lasts = a.lasts;
        if let Lasts::Turns(n) = a.lasts {
            self.turns = n;
        }
        self.source = a.source.clone();
    }

    /// The adjustment it describes.
    pub fn adjust(&self, lang: &Language) -> InitAdjust {
        let lasts = if let Lasts::Turns(_) = self.lasts { Lasts::Turns(self.turns.max(1)) } else { self.lasts };
        let label = if self.label.trim().is_empty() { lang.tr("Initiative") } else { self.label.trim().to_owned() };
        let mut a = InitAdjust::new(label, self.score, self.dice, lasts).from_source(self.source.clone());
        a.roll_now = self.roll_now;
        a
    }
}

/// How long, in short: "pass", "turn", "3 turns", "until removed".
pub fn lasts_text(a: &InitAdjust, lang: &Language) -> String {
    match a.lasts {
        Lasts::Pass => lang.tr("this pass"),
        Lasts::Turn => lang.tr("this turn"),
        Lasts::Turns(n) => lang.tr_fmt("{0} turns", &[&n]),
        Lasts::Removed => lang.tr("until removed"),
    }
}

/// A chip's text: "Adrenaline Boost 2 +4 · this turn".
pub fn chip_text(a: &InitAdjust, lang: &Language) -> String {
    format!("{} {} · {}", a.label, a.effect(), lasts_text(a, lang))
}

/// The chips of `adjust`; returns the one whose × was clicked.
pub fn chips(ui: &mut egui::Ui, lang: &Language, adjust: &[InitAdjust]) -> Option<usize> {
    let mut out = None;
    let accent = crate::theme::accent(ui);
    for (k, a) in adjust.iter().enumerate() {
        egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.6))).corner_radius(9).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.horizontal(|ui| {
                let mut tip = chip_text(a, lang);
                if !a.rolled.is_empty() {
                    tip.push_str(&format!("\n[{}]", a.rolled.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")));
                }
                if a.dice != 0 && !a.roll_now {
                    tip.push_str(&format!("\n{}", lang.tr("dice from the next roll")));
                }
                ui.label(RichText::new(chip_text(a, lang)).size(11.0).color(accent)).on_hover_text(tip);
                if ui.add(egui::Button::new(RichText::new(icons::X).size(10.0)).frame(false).small()).on_hover_text(lang.tr("Remove")).clicked() {
                    out = Some(k);
                }
            });
        });
    }
    out
}

/// The menu's body: the character's suggestions (those in effect
/// already greyed out), then the typed-in form. Returns the adjustment to
/// add.
pub fn menu(ui: &mut egui::Ui, lang: &Language, form: &mut AdjustForm, suggestions: &[Suggestion], active: &[InitAdjust]) -> Option<InitAdjust> {
    let mut out = None;
    ui.set_min_width(260.0);
    if !suggestions.is_empty() {
        ui.label(RichText::new(lang.tr("From the character")).strong());
        for s in suggestions {
            let on = active.iter().any(|a| a.source == s.adjust.source);
            let text = if s.ask { format!("{}…", s.adjust.label) } else { format!("{}: {} {}", s.adjust.label, s.adjust.effect(), lasts_text(&s.adjust, lang)) };
            let r = ui.add_enabled(!on, egui::Button::new(text)).on_hover_text(&s.note).on_disabled_hover_text(lang.tr("In effect"));
            if r.clicked() {
                if s.ask {
                    form.take(&s.adjust);
                } else {
                    out = Some(s.adjust.clone());
                    ui.close();
                }
            }
        }
        ui.separator();
    }
    ui.label(RichText::new(lang.tr("Adjust")).strong());
    egui::Grid::new("init_adjust_form").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label(lang.tr("Name"));
        ui.add(egui::TextEdit::singleline(&mut form.label).hint_text(lang.tr("Initiative")).desired_width(150.0));
        ui.end_row();
        ui.label(lang.tr("Score"));
        let plus = if form.score > 0 { "+" } else { "" };
        ui.add(egui::DragValue::new(&mut form.score).range(-30..=30).prefix(plus));
        ui.end_row();
        ui.label(lang.tr("Dice"));
        ui.horizontal(|ui| {
            let plus = if form.dice > 0 { "+" } else { "" };
            ui.add(egui::DragValue::new(&mut form.dice).range(-4..=4).suffix("d6").prefix(plus));
            ui.add_enabled(form.dice > 0, egui::Checkbox::new(&mut form.roll_now, lang.tr("roll now"))).on_hover_text(lang.tr("Off: from the next roll"));
        });
        ui.end_row();
        ui.label(lang.tr("Duration"));
        ui.horizontal(|ui| {
            let picks = [(Lasts::Pass, lang.tr("this pass")), (Lasts::Turn, lang.tr("this turn")), (Lasts::Turns(form.turns), lang.tr("turns")), (Lasts::Removed, lang.tr("until removed"))];
            let shown = picks.iter().find(|(l, _)| std::mem::discriminant(l) == std::mem::discriminant(&form.lasts)).map(|(_, t)| t.clone()).unwrap_or_default();
            crate::combo::Combo::from_id_salt("init_adjust_lasts").selected_text(shown).width(110.0).show_ui(ui, |ui| {
                for (l, t) in &picks {
                    crate::combo::selectable_value(ui, &mut form.lasts, *l, t.as_str());
                }
            });
            if let Lasts::Turns(_) = form.lasts {
                ui.add(egui::DragValue::new(&mut form.turns).range(1..=99));
                form.lasts = Lasts::Turns(form.turns);
            }
        });
        ui.end_row();
    });
    let empty = form.score == 0 && form.dice == 0;
    if ui.add_enabled(!empty, egui::Button::new(format!("{} {}", icons::PLUS, lang.tr("Add")))).clicked() {
        out = Some(form.adjust(lang));
        *form = AdjustForm { lasts: form.lasts, turns: form.turns, ..AdjustForm::default() };
        ui.close();
    }
    out
}

/// A button that opens [`menu`] in a popup; returns the adjustment to add.
pub fn button(ui: &mut egui::Ui, lang: &Language, form: &mut AdjustForm, suggestions: &[Suggestion], active: &[InitAdjust], text: impl Into<egui::WidgetText>) -> Option<InitAdjust> {
    let r = ui.add(egui::Button::new(text).small()).on_hover_text(lang.tr("Add to the Initiative Score or dice"));
    let mut out = None;
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        out = menu(ui, lang, form, suggestions, active);
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_makes_the_adjustment() {
        let lang = Language::default();
        let mut f = AdjustForm { label: " Jazz ".into(), score: 1, dice: 2, lasts: Lasts::Turns(0), turns: 3, ..Default::default() };
        let a = f.adjust(&lang);
        assert_eq!((a.label.as_str(), a.score, a.dice, a.lasts, a.roll_now), ("Jazz", 1, 2, Lasts::Turns(3), true));
        assert_eq!(chip_text(&a, &lang), "Jazz +1 +2d6 · 3 turns");
        f.take(&InitAdjust::new("Increase Reflexes", 0, 1, Lasts::Removed).from_source("spell:x"));
        assert_eq!((f.dice, f.lasts, f.source.as_str()), (1, Lasts::Removed, "spell:x"));
    }
}
