//! Chummer's "Advanced Programs" tab: an A.I.'s programs and Advanced
//! Programs. Programs are added through the item selection dialog (kind
//! `aiprogram`); this lists them with their costs and lets the player
//! remove the ones they bought.

use chummer_core::career;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::items::aiprogram;
use chummer_core::lang::Language;
use chummer_core::play::matrix;
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;

/// The program list with its summary. Returns whether the character
/// changed.
pub fn tab(ui: &mut egui::Ui, ch: &mut crate::doc::Doc, engine: &Engine, lang: &Language, status: &mut Status) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        if ch.created {
            ui.label(lang.tr_fmt("New program: {0} karma", &[&career::ai_program_karma_cost(engine, ch, false)]));
            ui.label(lang.tr_fmt("New Advanced Program: {0} karma", &[&career::ai_program_karma_cost(engine, ch, true)]));
        } else {
            let rules = engine.rules_for(ch);
            let c = aiprogram::counts(ch);
            ui.label(lang.tr_fmt("Free programs used: {0} / {1}", &[&c.normal, &c.normal_limit]));
            ui.label(lang.tr_fmt("Free Advanced Programs used: {0} / {1}", &[&c.advanced, &c.advanced_limit]));
            ui.label(lang.tr_fmt("Programs cost {0} karma", &[&aiprogram::creation_karma(ch, &rules)]));
        }
    });
    ui.add_space(4.0);
    let programs: Vec<_> = ch.items("aiprograms", "aiprogram").into_iter().cloned().collect();
    if programs.is_empty() {
        ui.weak(lang.tr("No programs yet."));
        return false;
    }
    let mut remove: Option<String> = None;
    egui::Grid::new("ai_programs").striped(true).num_columns(5).spacing([14.0, 4.0]).show(ui, |ui| {
        for h in [lang.tr("Name"), lang.tr("Advanced Programs"), lang.tr("Requires:"), lang.tr("Source"), String::new()] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        for p in &programs {
            let id = p.get("sourceid");
            let mut name = lang.data_name("programs.xml", &id, &p.get("name"));
            let extra = p.get("extra");
            if !extra.is_empty() {
                name = format!("{name} ({extra})");
            }
            let can_delete = p.get_bool("candelete").unwrap_or(true);
            // Chummer greys out programs granted by an improvement.
            ui.label(if can_delete { RichText::new(name) } else { RichText::new(name).weak() });
            ui.label(if aiprogram::is_advanced(p) { "✔" } else { "" });
            let req = p.get("requiresprogram");
            ui.label(if req.is_empty() { lang.tr("None") } else { req });
            ui.label(format!("{} {}", p.get("source"), p.get("page")));
            if can_delete {
                if ui.small_button("✖").on_hover_text(lang.tr("Delete")).clicked() {
                    remove = Some(p.get("guid"));
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    if let Some(g) = remove {
        changed = ch.run(Command::RemoveAiProgram { guid: g }, status).is_some();
    }
    changed
}

/// Condition monitor headings (`PhysicalCMLabelText`, `StunCMLabelText`):
/// an A.I. has a Core track unless it lives in a vehicle, and a Matrix
/// track in its home node.
pub fn cm_labels(ch: &Character, lang: &Language) -> (String, String) {
    if !ch.is_ai() {
        return (lang.tr("Physical"), lang.tr("Stun"));
    }
    let key = |k: &str| lang.s(k).trim_end_matches(':').to_owned();
    let home = matrix::home_node(ch);
    let physical = if home.is_some_and(|h| h.name == "vehicle") { lang.tr("Physical") } else { key("Label_OtherCoreCM") };
    let stun = if home.is_some() { key("Label_OtherMatrixCM") } else { String::new() };
    (physical, stun)
}
