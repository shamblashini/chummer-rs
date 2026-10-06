//! The History side tab: this session's changes to the character, newest
//! first, with Undo and Redo. Undone changes stay listed (greyed) until a
//! new change replaces them.

use chummer_core::command::{LogEntry, Session};
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

/// A button the user pressed.
pub enum Action {
    Undo,
    Redo,
}

/// The time of day an entry was made (UTC), "14:05:09".
fn time_of(e: &LogEntry) -> String {
    let iso = e.envelope.at_iso();
    iso.split_once('T').map_or(iso.clone(), |(_, t)| t.to_owned())
}

pub fn panel(ui: &mut egui::Ui, session: &Session, lang: &Language) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.add_enabled(session.can_undo(), egui::Button::new(lang.tr("Undo"))).on_hover_text(session.undo_label().unwrap_or_default()).clicked() {
            action = Some(Action::Undo);
        }
        if ui.add_enabled(session.can_redo(), egui::Button::new(lang.tr("Redo"))).on_hover_text(session.redo_label().unwrap_or_default()).clicked() {
            action = Some(Action::Redo);
        }
    });
    ui.weak(lang.tr_fmt("{0} changes this session", &[&session.version()]));
    ui.separator();
    if session.log().is_empty() && !session.can_redo() {
        ui.weak(lang.tr("No changes yet."));
        return action;
    }
    egui::Grid::new("history").num_columns(2).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
        // Undone entries first (the next Redo nearest the live ones).
        for e in session.redo_entries() {
            ui.weak(time_of(e));
            ui.label(RichText::new(&e.description).weak().strikethrough()).on_hover_text(lang.tr("Undone"));
            ui.end_row();
        }
        for e in session.log().iter().rev() {
            ui.weak(time_of(e));
            let r = ui.label(&e.description);
            if !e.envelope.author.is_empty() {
                r.on_hover_text(&e.envelope.author);
            }
            ui.end_row();
        }
    });
    action
}
