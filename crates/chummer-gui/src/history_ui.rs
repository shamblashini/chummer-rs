//! The History side tab.
//!
//! A local character: this session's changes, newest first, with Undo and
//! Redo. Undone changes stay listed (greyed) until a new change replaces
//! them. Below them, the changes of earlier sessions a `.chumrs` kept
//! (read-only).
//!
//! An online character: its campaign log instead. A player sees their
//! character's log (their own changes and the GM's, "GM gave you 100
//! karma: note"), how it is synced and the changes the GM's app refused;
//! the GM sees every change to the character with its author, and can
//! revert one.

use chummer_core::command::{LogEntry, Session};
use chummer_core::lang::Language;
use chummer_sync::SyncMode;
use eframe::egui::{self, RichText};

use crate::doc::{Doc, ONLINE_UNDO};

/// The time of day an entry was made (UTC), "14:05:09".
fn time_of(e: &LogEntry) -> String {
    let iso = e.envelope.at_iso();
    iso.split_once('T').map_or(iso.clone(), |(_, t)| t.to_owned())
}

/// "10-06 14:05" for Unix milliseconds (UTC).
pub fn short_time(at_ms: i64) -> String {
    let iso = chummer_core::chargen::iso_from_unix(at_ms.div_euclid(1000));
    iso.get(5..16).unwrap_or("").replace('T', " ")
}

/// Draws the panel; returns whether the character changed.
pub fn panel(ui: &mut egui::Ui, doc: &mut Doc, lang: &Language) -> bool {
    match doc.session() {
        Some(_) => local(ui, doc, lang),
        None => online(ui, doc, lang),
    }
}

fn local(ui: &mut egui::Ui, doc: &mut Doc, lang: &Language) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.add_enabled(doc.can_undo(), egui::Button::new(lang.tr("Undo"))).on_hover_text(doc.undo_label().unwrap_or_default()).clicked() {
            changed |= doc.undo().is_some();
        }
        if ui.add_enabled(doc.can_redo(), egui::Button::new(lang.tr("Redo"))).on_hover_text(doc.redo_label().unwrap_or_default()).clicked() {
            changed |= doc.redo().is_some();
        }
    });
    let Some(session): Option<&Session> = doc.session() else { return changed };
    ui.weak(lang.tr_fmt("Changes this session: {0}", &[&session.version()]));
    ui.separator();
    if session.log().is_empty() && !session.can_redo() {
        ui.weak(lang.tr("No changes yet."));
        earlier(ui, doc, lang);
        return changed;
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
    earlier(ui, doc, lang);
    changed
}

/// The history saved in the file, newest first.
fn earlier(ui: &mut egui::Ui, doc: &Doc, lang: &Language) {
    let items = doc.earlier_history();
    if items.is_empty() {
        return;
    }
    ui.separator();
    ui.strong(lang.tr("Earlier sessions"));
    egui::Grid::new("history_earlier").num_columns(2).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
        for e in items.iter().rev() {
            ui.weak(short_time(e.at));
            let r = ui.label(&e.description);
            if !e.author.is_empty() {
                r.on_hover_text(&e.author);
            }
            ui.end_row();
        }
    });
}

fn online(ui: &mut egui::Ui, doc: &mut Doc, lang: &Language) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new(lang.tr("Undo"))).on_disabled_hover_text(lang.tr(ONLINE_UNDO));
        ui.add_enabled(false, egui::Button::new(lang.tr("Redo"))).on_disabled_hover_text(lang.tr(ONLINE_UNDO));
    });
    if let Some(s) = doc.sync_state() {
        let mode = match s.mode {
            Some(SyncMode::Online) => lang.tr("Connected to the GM"),
            Some(SyncMode::Mailbox) => lang.tr("The GM is offline: changes go through the mailbox"),
            Some(SyncMode::Offline) => lang.tr("Offline: changes are kept until the GM or the mailbox can be reached"),
            None => lang.tr("Connecting…"),
        };
        ui.label(mode);
        if s.pending > 0 {
            ui.weak(lang.tr_fmt("{0} changes not confirmed by the GM yet", &[&s.pending]));
        } else if s.mode == Some(SyncMode::Online) {
            ui.weak(lang.tr("All changes are synced."));
        }
        let refused = doc.refused();
        if !refused.is_empty() {
            ui.add_space(4.0);
            ui.colored_label(ui.visuals().error_fg_color, lang.tr("The GM's app refused these changes:"));
            for r in &refused {
                ui.label(format!("• {}: {}", r.description, r.reason));
            }
            if ui.button(lang.tr("Dismiss")).clicked() {
                doc.dismiss_refused();
            }
        }
    } else {
        ui.weak(lang.tr("Every change to this character is logged in the campaign. Revert takes a change back (later changes that needed it are dropped)."));
    }
    ui.separator();
    let log = doc.log();
    if log.is_empty() {
        ui.weak(lang.tr("No changes yet."));
        return changed;
    }
    let err_id = egui::Id::new("history_revert_error");
    if let Some(e) = ui.data(|d| d.get_temp::<String>(err_id)) {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    egui::Grid::new("online_history").num_columns(3).striped(true).spacing([8.0, 3.0]).show(ui, |ui| {
        for l in log.iter().take(500) {
            ui.weak(short_time(l.at));
            if l.refused {
                ui.label(RichText::new(&l.text).color(ui.visuals().error_fg_color));
            } else {
                ui.label(&l.text);
            }
            match l.revert {
                Some(v) => {
                    if ui.small_button(lang.tr("Revert")).on_hover_text(lang.tr("Take this change back")).clicked() {
                        match doc.revert(v) {
                            Ok(_) => {
                                changed = true;
                                ui.data_mut(|d| d.remove::<String>(err_id));
                            }
                            Err(e) => ui.data_mut(|d| d.insert_temp(err_id, e)),
                        }
                    }
                }
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
    changed
}
