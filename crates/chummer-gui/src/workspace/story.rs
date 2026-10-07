//! The Limits page and the story and record pages: Character Info,
//! Game Notes, Calendar, Relationships, Improvements and Karma & Nuyen,
//! in Workspace cards. The same commands as the Classic tabs.

use chummer_core::character::INFO_FIELDS;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;
use crate::theme;
use crate::view::CharacterView;
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// A multi-line text field filling the card.
fn text_area(ui: &mut egui::Ui, id: &str, text: &mut String, rows: usize) -> egui::Response {
    let ws = theme::ws(ui);
    let mut r = None;
    egui::Frame::new().fill(ws.well).stroke(egui::Stroke::new(1.0_f32, ws.control)).corner_radius(5).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        r = Some(ui.add(egui::TextEdit::multiline(text).id_salt(id).frame(false).desired_width(f32::INFINITY).desired_rows(rows)));
    });
    r.expect("drawn")
}

impl CharacterView {
    /// Limits and the improvements that modify them.
    pub(super) fn ws_limits(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let s = &self.sheet;
        let mut cards = vec![(lang.tr("Physical"), s.limit_physical), (lang.tr("Mental"), s.limit_mental), (lang.tr("Social"), s.limit_social)];
        if self.doc.mag_enabled() {
            cards.push((lang.tr("Astral"), s.limit_astral));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (label, v) in &cards {
                widgets::tile(ui, label, &v.to_string(), true, 130.0);
            }
        });
        ui.add_space(4.0);
        widgets::heading(ui, &lang.tr("Limit Modifiers"), "", 13.0, |_| {});
        let imps = &self.doc.improvements;
        let mods: Vec<_> = imps.list.iter().filter(|i| i.kind.contains("Limit")).collect();
        if mods.is_empty() {
            ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
            return false;
        }
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &[140.0, 0.0, 60.0, 110.0, 140.0]);
            let caps = lang.tr_all(["Type", "Target", "Value", "Source", "Condition"]);
            let caps: Vec<&str> = caps.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for (n, i) in mods.into_iter().enumerate() {
                let ink = if imps.applies(i) { ws.text } else { ws.muted };
                let cells = [(i.kind.clone(), ink), (i.improved_name.clone(), ink), (chummer_core::improvement::fmt_num(i.val), ws.accent), (i.source.clone(), ws.muted), (i.condition.clone(), ws.muted)];
                widgets::table_row(ui, ("limit", n), false, 26.0, |ui| {
                    for ((t, c), w) in cells.into_iter().zip(&widths) {
                        widgets::cell(ui, *w, 26.0, |ui| ui.add(egui::Label::new(RichText::new(t).size(12.0).color(c)).truncate()));
                    }
                });
            }
            ui.add_space(4.0);
        });
        false
    }

    /// Character Info: personal details and reputation, then the long
    /// texts.
    pub(super) fn ws_info(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(widgets::title(&lang.tr("Personal details"), &ws));
            ui.add_space(4.0);
            let gap = 16.0;
            let per_row = ((ui.available_width() + gap) / (280.0 + gap)).floor().clamp(1.0, 4.0) as usize;
            let col = (ui.available_width() - gap * (per_row - 1) as f32) / per_row as f32;
            for chunk in INFO_FIELDS.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (key, label) in chunk {
                        widgets::cell(ui, col, 30.0, |ui| {
                            widgets::cell(ui, 90.0, 30.0, |ui| ui.label(RichText::new(lang.tr(label)).size(12.0).color(ws.muted)));
                            let mut v = self.doc.field(key);
                            let editable = !matches!(*key, "metatype" | "metavariant");
                            let r = ui.add_enabled_ui(editable, |ui| widgets::text_field(ui, &mut v, "", col - 100.0)).inner;
                            if r.changed() {
                                changed |= self.doc.set(Command::SetField { key: (*key).to_owned(), value: v });
                            }
                        });
                    }
                });
            }
        });
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(widgets::title(&lang.tr("Reputation"), &ws));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (key, label) in [("streetcred", lang.tr("Street Cred")), ("notoriety", lang.tr("Notoriety")), ("publicawareness", lang.tr("Public Awareness"))] {
                    ui.label(RichText::new(&label).size(12.0).color(ws.muted));
                    let mut v = self.doc.doc.get_i32(key).unwrap_or(0);
                    if widgets::stepper(ui, key, &mut v, 0, 100, &lang.tr_fmt("Lower {0}", &[&label]), &lang.tr_fmt("Raise {0}", &[&label])) {
                        changed |= self.doc.set(Command::SetField { key: key.to_owned(), value: v.to_string() });
                    }
                    ui.add_space(10.0);
                }
            });
        });
        let mut subs: Vec<(&'static str, String)> = vec![("description", lang.tr("Description")), ("background", lang.tr("Background")), ("concept", lang.tr("Concept")), ("notes", lang.tr("Character Notes"))];
        if !self.doc.created {
            // Career mode has its own Game Notes page.
            subs.push(("gamenotes", lang.tr("Game Notes")));
        }
        if !subs.iter().any(|(k, _)| *k == self.info_text) {
            self.info_text = "description";
        }
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let items: Vec<(&str, &str)> = subs.iter().map(|(_, l)| (l.as_str(), l.as_str())).collect();
            let current = subs.iter().position(|(k, _)| *k == self.info_text).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &items, current, 26.0) {
                self.info_text = subs[i].0;
            }
            ui.add_space(6.0);
            let key = self.info_text;
            let mut v = self.doc.field(key);
            if text_area(ui, key, &mut v, 16).changed() {
                changed |= self.doc.set(Command::SetField { key: key.to_owned(), value: v });
            }
        });
        changed
    }

    /// Game Notes (career).
    pub(super) fn ws_notes(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(widgets::title(&lang.tr("Game Notes"), &ws));
            ui.add_space(4.0);
            let mut v = self.doc.field("gamenotes");
            if text_area(ui, "gamenotes", &mut v, 24).changed() {
                changed |= self.doc.set(Command::SetField { key: "gamenotes".into(), value: v });
            }
        });
        changed
    }

    /// The career calendar: in-game weeks with notes, newest first.
    pub(super) fn ws_calendar(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        use chummer_core::calendar;
        let ws = theme::ws(ui);
        let mut changed = false;
        let mut weeks = calendar::weeks(&self.doc);
        weeks.sort_by_key(|w| std::cmp::Reverse((w.year, w.week)));
        let mut add = false;
        widgets::heading(ui, &lang.tr("Weeks"), &weeks.len().to_string(), 13.0, |ui| {
            add = widgets::button(ui, Some(icons::PLUS), &lang.tr("Add Week"), Look::Secondary, 24.0).clicked();
        });
        if add {
            changed |= self.doc.set(Command::AddWeek);
        }
        if weeks.is_empty() {
            ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
            return changed;
        }
        let mut edits = Vec::new();
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &[200.0, 0.0, 24.0]);
            let caps = [lang.tr("Week"), lang.tr("Notes"), String::new()];
            let caps: Vec<&str> = caps.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for w in &weeks {
                widgets::table_row(ui, &w.guid, false, 32.0, |ui| {
                    widgets::cell(ui, widths[0], 32.0, |ui| ui.label(RichText::new(w.label()).size(12.5).color(ws.text)));
                    widgets::cell(ui, widths[1], 32.0, |ui| {
                        let mut notes = w.notes.clone();
                        if widgets::text_field(ui, &mut notes, "", widths[1] - 4.0).changed() {
                            edits.push(Command::SetWeekNotes { week: w.guid.clone(), notes });
                        }
                    });
                    widgets::cell(ui, widths[2], 32.0, |ui| {
                        if widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove")).clicked() {
                            edits.push(Command::RemoveWeek { week: w.guid.clone() });
                        }
                    });
                });
            }
            ui.add_space(4.0);
        });
        for c in edits {
            changed |= self.doc.set(c);
        }
        changed
    }

    /// Custom improvements (their editor) and every improvement the
    /// character has.
    pub(super) fn ws_improvements(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        widgets::heading(ui, &lang.tr("Custom improvements"), "", 13.0, |_| {});
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            changed |= self.custom_improvements.tab(ui, &mut self.doc, &self.store, lang);
        });
        let imps = &self.doc.improvements;
        widgets::heading(ui, &lang.tr("Improvements"), &lang.tr_fmt("{0} improvements ({1} active). These modifiers come from qualities, ware, powers and gear.", &[&imps.list.len(), &imps.active().count()]), 13.0, |_| {});
        if imps.list.is_empty() {
            return changed;
        }
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &[150.0, 0.0, 56.0, 56.0, 70.0, 100.0, 120.0]);
            let caps = lang.tr_all(["Type", "Target", "Value", "Aug", "Min/Max", "Source", "Condition"]);
            let caps: Vec<&str> = caps.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            let num = |v: f64| if v == 0.0 { String::new() } else { chummer_core::improvement::fmt_num(v) };
            for (n, i) in imps.list.iter().enumerate() {
                let ink = if imps.applies(i) { ws.text } else { ws.muted };
                let minmax = if i.min != 0.0 || i.max != 0.0 { format!("{}/{}", i.min, i.max) } else { String::new() };
                let cells = [(i.kind.clone(), ink), (i.improved_name.clone(), ink), (num(i.val), ws.accent), (num(i.aug), ws.accent), (minmax, ws.muted), (i.source.clone(), ws.muted), (i.condition.clone(), ws.muted)];
                widgets::table_row(ui, ("imp", n), false, 24.0, |ui| {
                    for ((t, c), w) in cells.into_iter().zip(&widths) {
                        widgets::cell(ui, *w, 24.0, |ui| ui.add(egui::Label::new(RichText::new(t).size(12.0).color(c)).truncate()));
                    }
                });
            }
            ui.add_space(4.0);
        });
        changed
    }

    /// Relationships: the contacts editor in a card.
    pub(super) fn ws_relationships(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let height = ui.available_height();
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(height - 26.0);
            changed = self.relationships.ui(ui, &mut self.doc, &self.store, lang, status);
        });
        changed
    }

    /// Karma & Nuyen: the totals, the career actions (Edge, street cred,
    /// group), a manual entry and the whole ledger.
    pub(super) fn ws_karma_page(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        self.ws_karma_tiles(ui, engine, lang, 4);
        if self.doc.created {
            widgets::card_frame(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                changed |= crate::career_ui::actions_ui(ui, &mut self.doc, engine, lang);
            });
            changed |= self.ws_manual_entry(ui, lang);
        }
        ui.add_space(4.0);
        changed |= self.ws_ledger(ui, lang, false);
        changed
    }
}
