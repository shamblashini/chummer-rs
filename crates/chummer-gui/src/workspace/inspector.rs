//! The Workspace inspector's Item panel: the selected item in the
//! Workspace style (what `item_editor` shows in Classic: rating,
//! quantity, equipped, wireless, custom name, location, cost and the
//! other values, the play panels, notes, contents with their "Add …"
//! commands, Sell or Delete), or, while the inline catalog has a record
//! selected, that record (`ws_catalog`).
//!
//! Every change is the same `Command` as in `item_editor`; Classic keeps
//! its editor unchanged.

use std::sync::Arc;

use chummer_core::command::Command;
use chummer_core::data::DataStore;
use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::items::{self, edit};
use chummer_core::lang::Language;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use eframe::egui::{self, RichText};

use super::CharacterView;
use crate::doc::Doc;
use crate::item_editor::EditorResult;
use crate::pdf_ui::{self, Status};
use crate::theme;
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// What the Workspace item panel keeps between frames, for the item
/// `guid`; it starts over when another item is shown.
pub struct WsItemEditor {
    guid: String,
    /// `SellItem.SellPercent`, in percent.
    sell_percent: f64,
    /// Delete was clicked once; the second click confirms.
    confirm_remove: bool,
    /// Ammunition, matrix and damage tracking (`play_ui`).
    play: crate::play_ui::PlayPanel,
    /// Name typed for a new location.
    new_location: String,
}

impl Default for WsItemEditor {
    fn default() -> Self {
        WsItemEditor { guid: String::new(), sell_percent: 50.0, confirm_remove: false, play: Default::default(), new_location: String::new() }
    }
}

impl CharacterView {
    /// Whether the inspector's Item panel has something to show: an item,
    /// or the catalog's selected record.
    pub(crate) fn ws_inspector_has_item(&self) -> bool {
        self.item_editor.is_some() || self.ws_catalog_inspecting()
    }

    /// Close the Item panel: the item, or the catalog's selection.
    pub(crate) fn ws_inspector_close(&mut self) {
        if self.ws_catalog_inspecting() {
            self.ws_catalog_deselect();
        } else {
            self.item_editor = None;
        }
    }

    /// The Item panel's contents. Returns true if the character changed.
    pub(crate) fn ws_inspector(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        if self.ws_catalog_inspecting() {
            return self.ws_catalog_inspector(ui, engine, lang, pdfs, status);
        }
        let Some(guid) = self.item_editor.as_ref().map(|(g, _)| g.clone()) else { return false };
        let store = self.store.clone();
        let mut ed = std::mem::take(&mut self.ws_gear.editor);
        if ed.guid != guid {
            ed = WsItemEditor { guid: guid.clone(), ..Default::default() };
        }
        let mut res = ed.ui(ui, &mut self.doc, &store, engine, lang, pdfs, status, &guid);
        self.ws_gear.editor = ed;
        if let Some(s) = res.status.take() {
            *status = Some(s);
        }
        if let Some((tag, parent)) = res.add_child.take() {
            // The catalog adds into the item, showing that kind.
            if let (Some(page), Some(k)) = (self.ws_item_page(self.tab), items::kind(&tag)) {
                self.ws_add_into(page, &parent, Some(k.tag));
            }
        }
        if let Some(g) = res.select.take() {
            // As a click on its row: the catalog adds into it when it can.
            match self.ws_item_page(self.tab) {
                Some(page) => self.ws_select_item(page, &g),
                None => self.item_editor = Some((g, crate::item_editor::ItemEditor::default())),
            }
        } else if res.removed {
            self.item_editor = None;
        }
        res.changed
    }
}

impl WsItemEditor {
    #[allow(clippy::too_many_arguments)]
    fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, guid: &str) -> EditorResult {
        let ws = theme::ws(ui);
        let mut res = EditorResult::default();
        let Some(e) = edit::find(ch, guid).cloned() else {
            res.removed = true;
            return res;
        };
        let tag = edit::tag_of(&e).to_owned();
        let career = ch.created;
        let label = items::kind(&tag).map_or_else(|| tag.clone(), |k| lang.tr(k.label));
        ui.spacing_mut().item_spacing.y = 6.0;

        // Header: name, kind and category, the parent, the source.
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(e.get("name")).font(widgets::bold(14.0)).color(ws.text)).wrap());
            if let Some(r) = SourceRef::of(&e) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tip = if pdfs.is_linked(&r.book) { lang.tr("Open the sourcebook at this page") } else { lang.tr("No PDF linked for this book — Tools → Sourcebooks") };
                    if widgets::button(ui, Some(icons::BOOK_OPEN), &r.to_string(), Look::Ghost, 22.0).on_hover_text(tip).clicked() {
                        pdf_ui::open(pdfs, &r, status);
                    }
                });
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            widgets::tag(ui, &label, ws.muted, ws.divider);
            let cat = e.get("category");
            if !cat.is_empty() {
                widgets::tag(ui, &cat, ws.muted, ws.divider);
            }
            if tag.ends_with("ware") && !e.get("grade").is_empty() {
                widgets::tag(ui, &e.get("grade"), ws.accent, ws.primary);
            }
        });
        if let Some(p) = edit::parent(ch, guid) {
            let pg = p.get("guid");
            if widgets::button(ui, Some(icons::ARROW_BEND_LEFT_UP), &lang.tr_fmt("in {0}", &[&p.get("name")]), Look::Ghost, 22.0).on_hover_text(lang.tr("Show the item this is in")).clicked() {
                res.select = Some(pg);
            }
        }
        let included = edit::is_included(ch, guid);
        if included {
            ui.label(RichText::new(lang.tr("Included with its parent item.")).size(11.5).color(ws.muted));
        }

        // Choices: rating, quantity, equipped, wireless, name, location.
        let rating_tip = |up: bool| lang.tr(if up { "Raise Rating" } else { "Lower Rating" });
        egui::Grid::new(("ws_item_edit", guid)).num_columns(2).spacing([12.0, 6.0]).min_row_height(24.0).show(ui, |ui| {
            let caption = |ui: &mut egui::Ui, t: &str| {
                ui.label(RichText::new(t).size(12.0).color(ws.muted));
            };
            // Rating and quantity are bought in career mode, not edited
            // (CharacterCareer has no rating/quantity spinners).
            if let Some((min, max)) = edit::rating_range(ch, store, guid) {
                caption(ui, &e.child_text("ratinglabel").map(|l| if lang.has(&l) { lang.s(&l) } else { l }).filter(|l| !l.starts_with("String_") && !l.starts_with("Label_")).unwrap_or_else(|| lang.tr("Rating")));
                let mut r = e.get_i32("rating").unwrap_or(min);
                let resp = ui.add_enabled_ui(!career && !included, |ui| widgets::rating_stepper(ui, &mut r, min, max, &rating_tip(false), &rating_tip(true))).inner;
                if resp.changed() {
                    res.changed |= ch.run(Command::SetItemRating { guid: guid.to_owned(), rating: r }, &mut res.status).is_some();
                }
                ui.end_row();
            }
            if matches!(e.name.as_str(), "gear" | "drug") {
                caption(ui, &lang.tr("Quantity"));
                let mut q = e.get_f64("qty").unwrap_or(1.0);
                let step = e.get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0);
                let resp = ui.add_enabled_ui(!career && !included, |ui| widgets::qty_stepper(ui, ("ws_item_qty", guid), &mut q, 1.0, 100_000.0, step, 2, &lang.tr("Lower"), &lang.tr("Raise"))).inner;
                if resp.changed() {
                    res.changed |= ch.set(Command::SetItemQuantity { guid: guid.to_owned(), qty: q });
                }
                ui.end_row();
            }
            if edit::can_equip(&e) || e.child("wirelesson").is_some() {
                caption(ui, &lang.tr("State"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if edit::can_equip(&e) {
                        let mut on = e.get_bool("equipped").unwrap_or(true);
                        if widgets::check(ui, &mut on, &lang.tr("Equipped")).changed() {
                            res.changed |= ch.set(Command::SetItemEquipped { guid: guid.to_owned(), on });
                        }
                    }
                    if e.child("wirelesson").is_some() {
                        let mut on = e.get_bool("wirelesson").unwrap_or(false);
                        if widgets::check(ui, &mut on, &lang.tr("Wireless")).changed() {
                            res.changed |= ch.set(Command::SetItemWireless { guid: guid.to_owned(), on });
                        }
                    }
                });
                ui.end_row();
            }
            if let Some(field) = edit::custom_name_field(&tag) {
                caption(ui, &if tag == "lifestyle" { lang.tr("Name") } else { lang.tr("Custom name") });
                let mut v = e.get(field);
                if ui.add(egui::TextEdit::singleline(&mut v).desired_width(ui.available_width().min(220.0))).changed() {
                    res.changed |= ch.set(Command::SetItemText { guid: guid.to_owned(), field: field.to_owned(), value: v });
                }
                ui.end_row();
            }
            if edit::has_location(ch, guid) {
                caption(ui, &lang.tr("Location"));
                // `<location>` holds the location's guid (older files: its name).
                let cur = e.get("location");
                let locations = edit::locations(ch, guid);
                let none = lang.tr("None");
                let shown = locations.iter().find(|(g, n)| g.eq_ignore_ascii_case(&cur) || *n == cur).map_or_else(|| if cur.is_empty() { none.clone() } else { cur.clone() }, |(_, n)| n.clone());
                let mut pick = None;
                crate::combo::Combo::from_id_salt(("ws_location", guid)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
                    if crate::combo::selectable_label(ui, cur.is_empty(), &none).clicked() {
                        pick = Some(String::new());
                    }
                    for (g, n) in &locations {
                        if crate::combo::selectable_label(ui, *g == cur, n).clicked() {
                            pick = Some(g.clone());
                        }
                    }
                });
                if let Some(g) = pick {
                    res.changed |= ch.set(Command::SetItemText { guid: guid.to_owned(), field: "location".into(), value: g });
                }
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add(egui::TextEdit::singleline(&mut self.new_location).hint_text(lang.tr("New location")).desired_width(120.0));
                    let name = self.new_location.trim().to_owned();
                    if ui.add_enabled_ui(!name.is_empty(), |ui| widgets::button(ui, Some(icons::PLUS), &lang.tr("Add"), Look::Secondary, 24.0)).inner.clicked() {
                        res.changed |= ch.set(Command::AddItemLocation { guid: guid.to_owned(), name });
                        self.new_location.clear();
                    }
                });
                ui.end_row();
            }
        });

        // Values.
        widgets::rule(ui);
        let rules = engine.rules_for(ch);
        widgets::value_row(ui, &lang.tr("Cost"), &format::nuyen(edit::total_cost(ch, store, guid)), ws.text, "");
        let avail = edit::availability(ch, store, guid);
        if !avail.is_empty() {
            widgets::value_row(ui, &lang.tr("Availability"), &avail, ws.text, "");
        }
        if let Some(ess) = edit::essence(ch, store, &rules, guid) {
            widgets::value_row(ui, &lang.tr("Essence"), &format::essence(ess, rules.essence_decimals), ws.accent, "");
        }
        if let Some((used, total)) = edit::capacity(ch, guid) {
            let t = format!("{} / {}", chummer_core::improvement::fmt_num(used), chummer_core::improvement::fmt_num(total));
            widgets::value_row(ui, &if tag == "vehicle" { lang.tr("Mod Slots") } else { lang.tr("Capacity") }, &t, if used > total { ws.error } else { ws.text }, "");
        }
        if tag.ends_with("ware") && !e.get("location").is_empty() {
            widgets::value_row(ui, &lang.tr("Side"), &e.get("location"), ws.text, "");
        }
        if tag == "lifestyle" {
            widgets::value_row(ui, &lang.tr("Months"), &e.get("months"), ws.text, "");
        }
        if tag == "weapon" && !e.get("weaponslots").is_empty() {
            let free = edit::free_mounts(ch, guid);
            widgets::value_row(ui, &lang.tr("Free mounts"), &if free.is_empty() { lang.tr("none") } else { free.join(", ") }, ws.text, "");
        }

        // At the table: ammunition, matrix, vehicle damage.
        // In a horizontal scroll area: its tables would widen the inspector.
        let play = &mut self.play;
        let status = &mut res.status;
        res.changed |= egui::ScrollArea::horizontal().id_salt(("ws_item_play", guid)).show(ui, |ui| play.ui(ui, ch, store, lang, guid, status)).inner;

        // Notes.
        ui.add_space(2.0);
        ui.label(widgets::overline(&lang.tr("Notes"), &ws));
        let mut notes = e.get("notes");
        if ui.add(egui::TextEdit::multiline(&mut notes).desired_width(f32::INFINITY).desired_rows(3)).changed() {
            res.changed |= ch.set(Command::SetItemText { guid: guid.to_owned(), field: "notes".into(), value: notes });
        }

        self.contents(ui, ch, store, lang, guid, &mut res);
        widgets::rule(ui);
        self.remove(ui, ch, store, lang, guid, &tag, included, &mut res);
        res
    }

    /// Nested items and the "Add …" commands.
    fn contents(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, guid: &str, res: &mut EditorResult) {
        let ws = theme::ws(ui);
        let kids = edit::children(ch, guid);
        // What it takes: each opens the catalog on that kind, adding into
        // it (as selecting it in the inventory does).
        let kinds = items::place::accepts(ch, store, guid);
        if kids.is_empty() && kinds.is_empty() {
            return;
        }
        ui.add_space(2.0);
        ui.label(widgets::overline(&lang.tr("Contents"), &ws));
        if kids.is_empty() {
            ui.label(RichText::new(lang.tr("Nothing inside.")).size(12.0).color(ws.muted));
        }
        for (g, t, name) in &kids {
            let label = items::kind(t).map_or_else(|| t.clone(), |k| lang.tr(k.label));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icons::icon(icons::CARET_RIGHT, 11.0, ws.muted));
                let r = ui.add(egui::Label::new(RichText::new(name).size(12.5).color(ws.accent)).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.label(RichText::new(label).size(11.0).color(ws.muted));
                if r.clicked() {
                    res.select = Some(g.clone());
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for k in &kinds {
                if widgets::button(ui, Some(icons::PLUS), &format!("{}…", lang.tr(k.label)), Look::Secondary, 24.0).clicked() {
                    res.add_child = Some((k.tag.to_owned(), guid.to_owned()));
                }
            }
        });
    }

    /// Career mode sells (`ICanSell.Sell`), creation mode deletes.
    #[allow(clippy::too_many_arguments)]
    fn remove(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, guid: &str, tag: &str, included: bool, res: &mut EditorResult) {
        if included {
            return;
        }
        let ws = theme::ws(ui);
        if ch.created && tag != "lifestyle" {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(egui::DragValue::new(&mut self.sell_percent).range(0.0..=100.0).suffix(" %"));
                let value = edit::sale_value(ch, store, guid, self.sell_percent / 100.0);
                let r = widgets::button(ui, Some(icons::COINS), &lang.tr_fmt("Sell for {0}", &[&format::nuyen(value)]), Look::Secondary, 26.0);
                if r.on_hover_text(lang.tr("Removes the item and adds the proceeds to the log")).clicked() {
                    if let Some(r) = ch.run(Command::SellItem { guid: guid.to_owned(), fraction: self.sell_percent / 100.0 }, &mut res.status) {
                        res.status = r.message.map(|m| (m, false));
                        res.changed = true;
                        res.removed = true;
                    }
                }
            });
            return;
        }
        let text = if self.confirm_remove { lang.tr("Click again to delete") } else { lang.tr("Delete") };
        let r = ui
            .scope(|ui| {
                if self.confirm_remove {
                    ui.visuals_mut().override_text_color = Some(ws.error);
                }
                widgets::button(ui, Some(icons::TRASH), &text, if self.confirm_remove { Look::Outline } else { Look::Secondary }, 26.0)
            })
            .inner;
        if r.on_hover_text(lang.tr("Also removes its improvements and everything inside it")).clicked() {
            if self.confirm_remove {
                if ch.set(Command::DeleteItem { guid: guid.to_owned() }) {
                    res.changed = true;
                    res.removed = true;
                }
                self.confirm_remove = false;
            } else {
                self.confirm_remove = true;
            }
        }
    }
}
