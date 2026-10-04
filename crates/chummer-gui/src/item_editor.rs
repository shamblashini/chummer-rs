//! Detail pane for one saved item: the right-hand item panels of
//! `CharacterCreate.cs` / `CharacterCareer.cs` (rating, quantity, equipped,
//! wireless, custom name, location, notes, cost/availability/essence/
//! capacity, nested items with their "Add …" commands, Sell / Delete).
//!
//! All changes go through `chummer_core::items::edit`; this file only draws.

use chummer_core::character::Character;
use chummer_core::data::DataStore;
use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::items::{self, edit};
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

use crate::view::{ACCENT, WARN};

/// What the editor asks of its caller this frame.
#[derive(Debug, Default)]
pub struct EditorResult {
    /// The character changed: recompute the sheet.
    pub changed: bool,
    /// Open the add dialog for kind `tag` with this parent guid preset.
    pub add_child: Option<(String, String)>,
    /// Show this item (a child clicked in the list) instead.
    pub select: Option<String>,
    /// The item is gone (sold, deleted, or no longer on the character).
    pub removed: bool,
    /// A message for the status bar: (text, is error).
    pub status: Option<(String, bool)>,
}

pub struct ItemEditor {
    /// `SellItem.SellPercent`, in percent.
    sell_percent: f64,
    /// Weapon mount size id picked for "Add weapon mount".
    mount_size: String,
    /// Delete was clicked once; the second click confirms.
    confirm_remove: bool,
    /// Ammunition, matrix and damage tracking (`play_ui`).
    play: crate::play_ui::PlayPanel,
}

impl Default for ItemEditor {
    fn default() -> Self {
        ItemEditor { sell_percent: 50.0, mount_size: String::new(), confirm_remove: false, play: Default::default() }
    }
}

impl ItemEditor {
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, engine: &Engine, lang: &Language, guid: &str) -> EditorResult {
        let mut res = EditorResult::default();
        let Some(e) = edit::find(ch, guid).cloned() else {
            res.removed = true;
            return res;
        };
        let tag = edit::tag_of(&e).to_owned();
        let career = ch.created;
        let label = items::kind(&tag).map_or_else(|| tag.clone(), |k| lang.tr(k.label));

        ui.heading(RichText::new(e.get("name")).color(ACCENT));
        let cat = e.get("category");
        ui.weak(if cat.is_empty() { label.clone() } else { format!("{label} · {cat}") });
        if let Some(p) = edit::parent(ch, guid) {
            let pg = p.get("guid");
            if ui.link(lang.tr_fmt("in {0}", &[&p.get("name")])).on_hover_text(lang.tr("Show the item this is in")).clicked() {
                res.select = Some(pg);
            }
        }
        let included = edit::is_included(ch, guid);
        if included {
            ui.weak(lang.tr("Included with its parent item."));
        }
        ui.separator();

        egui::Grid::new(("item_edit", guid)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            // Rating and quantity are bought in career mode, not edited
            // (CharacterCareer has no rating/quantity spinners).
            if let Some((min, max)) = edit::rating_range(ch, store, guid) {
                ui.label(e.child_text("ratinglabel").map(|l| if lang.has(&l) { lang.s(&l) } else { l }).filter(|l| !l.starts_with("String_") && !l.starts_with("Label_")).unwrap_or_else(|| lang.tr("Rating")));
                let mut r = e.get_i32("rating").unwrap_or(min);
                let resp = ui.add_enabled(!career && !included, egui::DragValue::new(&mut r).range(min..=max));
                if resp.changed() {
                    match edit::apply_rating_change(ch, store, guid, r) {
                        Ok(_) => res.changed = true,
                        Err(err) => res.status = Some((err, true)),
                    }
                }
                ui.end_row();
            }
            if matches!(e.name.as_str(), "gear" | "drug") {
                ui.label(lang.tr("Quantity"));
                let mut q = e.get_f64("qty").unwrap_or(1.0);
                let step = e.get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0);
                let resp = ui.add_enabled(!career && !included, egui::DragValue::new(&mut q).range(1.0..=100_000.0).speed(step).max_decimals(2));
                if resp.changed() {
                    res.changed |= edit::set_quantity(ch, guid, q);
                }
                ui.end_row();
            }
            if edit::can_equip(&e) {
                ui.label(lang.tr("Equipped"));
                let mut on = e.get_bool("equipped").unwrap_or(true);
                if ui.checkbox(&mut on, "").changed() {
                    res.changed |= edit::set_equipped(ch, store, guid, on);
                }
                ui.end_row();
            }
            if e.child("wirelesson").is_some() {
                ui.label(lang.tr("Wireless"));
                let mut on = e.get_bool("wirelesson").unwrap_or(false);
                if ui.checkbox(&mut on, "").changed() {
                    res.changed |= edit::set_wireless(ch, store, guid, on);
                }
                ui.end_row();
            }
            if let Some(field) = edit::custom_name_field(&tag) {
                ui.label(if tag == "lifestyle" { lang.tr("Name") } else { lang.tr("Custom name") });
                let mut v = e.get(field);
                if ui.add(egui::TextEdit::singleline(&mut v).desired_width(180.0)).changed() {
                    res.changed |= edit::set_text(ch, guid, field, &v);
                }
                ui.end_row();
            }
            if tag == "lifestyle" {
                ui.label(lang.tr("Months"));
                ui.label(e.get("months"));
                ui.end_row();
            }
            if edit::has_location(ch, guid) {
                ui.label(lang.tr("Location"));
                let mut v = e.get("location");
                if ui.add(egui::TextEdit::singleline(&mut v).desired_width(180.0)).changed() {
                    res.changed |= edit::set_text(ch, guid, "location", &v);
                }
                ui.end_row();
            }
            if tag.ends_with("ware") && !e.get("location").is_empty() {
                ui.label(lang.tr("Side"));
                ui.label(e.get("location"));
                ui.end_row();
            }
            if tag.ends_with("ware") && !e.get("grade").is_empty() {
                ui.label(lang.tr("Grade"));
                ui.label(e.get("grade"));
                ui.end_row();
            }

            ui.label(lang.tr("Cost"));
            ui.strong(format::nuyen(edit::total_cost(ch, store, guid)));
            ui.end_row();
            let avail = edit::availability(ch, store, guid);
            if !avail.is_empty() {
                ui.label(lang.tr("Availability"));
                ui.label(avail);
                ui.end_row();
            }
            let rules = engine.rules_for(ch);
            if let Some(ess) = edit::essence(ch, store, &rules, guid) {
                ui.label(lang.tr("Essence"));
                ui.label(format::essence(ess, rules.essence_decimals));
                ui.end_row();
            }
            if let Some((used, total)) = edit::capacity(ch, guid) {
                ui.label(if tag == "vehicle" { lang.tr("Mod Slots") } else { lang.tr("Capacity") });
                let t = RichText::new(format!("{} / {}", chummer_core::improvement::fmt_num(used), chummer_core::improvement::fmt_num(total)));
                ui.label(if used > total { t.color(ui.visuals().error_fg_color) } else { t });
                ui.end_row();
            }
            if tag == "weapon" {
                let free = edit::free_mounts(ch, guid);
                if !e.get("weaponslots").is_empty() {
                    ui.label(lang.tr("Free mounts"));
                    ui.label(if free.is_empty() { lang.tr("none") } else { free.join(", ") });
                    ui.end_row();
                }
            }
        });

        res.changed |= self.play.ui(ui, ch, store, lang, guid, &mut res.status);

        ui.add_space(6.0);
        ui.label(RichText::new(lang.tr("Notes")).strong());
        let mut notes = e.get("notes");
        if ui.add(egui::TextEdit::multiline(&mut notes).desired_width(f32::INFINITY).desired_rows(3)).changed() {
            res.changed |= edit::set_text(ch, guid, "notes", &notes);
        }

        self.children_ui(ui, ch, store, lang, guid, &tag, &mut res);
        ui.separator();
        self.remove_ui(ui, ch, store, lang, guid, &tag, included, &mut res);
        res
    }

    /// Nested items and the "Add …" commands.
    #[allow(clippy::too_many_arguments)]
    fn children_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language, guid: &str, tag: &str, res: &mut EditorResult) {
        let kids = edit::children(ch, guid);
        let kinds = edit::child_kinds(ch, guid);
        if kids.is_empty() && kinds.is_empty() && tag != "vehicle" {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(lang.tr("Contents")).strong());
        if kids.is_empty() {
            ui.weak(lang.tr("Nothing inside."));
        }
        for (g, t, name) in &kids {
            let label = items::kind(t).map_or_else(|| t.clone(), |k| lang.tr(k.label));
            if ui.link(name).on_hover_text(label).clicked() {
                res.select = Some(g.clone());
            }
        }
        ui.horizontal_wrapped(|ui| {
            for k in &kinds {
                if ui.button(format!("➕ {}…", lang.tr(k.label))).clicked() {
                    res.add_child = Some((k.tag.to_owned(), guid.to_owned()));
                }
            }
        });
        if tag == "vehicle" {
            // Weapon mounts have no selection dialog kind (`CreateWeaponMount`).
            let sizes = edit::weapon_mount_sizes(store);
            ui.horizontal(|ui| {
                let cur = sizes.iter().find(|(id, _)| *id == self.mount_size).map(|(_, n)| n.clone()).unwrap_or_else(|| lang.tr("Mount size…"));
                egui::ComboBox::from_id_salt(("mount_size", guid)).selected_text(cur).show_ui(ui, |ui| {
                    for (id, n) in &sizes {
                        ui.selectable_value(&mut self.mount_size, id.clone(), n);
                    }
                });
                if ui.add_enabled(!self.mount_size.is_empty(), egui::Button::new(format!("➕ {}", lang.tr("Add Weapon Mount")))).clicked() {
                    match edit::add_weapon_mount(ch, store, guid, &self.mount_size) {
                        Ok(_) => res.changed = true,
                        Err(e) => res.status = Some((e, true)),
                    }
                }
            });
        }
    }

    /// Career mode sells (`ICanSell.Sell`), creation mode deletes.
    #[allow(clippy::too_many_arguments)]
    fn remove_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language, guid: &str, tag: &str, included: bool, res: &mut EditorResult) {
        if included {
            return;
        }
        if ch.created && tag != "lifestyle" {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.sell_percent).range(0.0..=100.0).suffix(" %"));
                let value = edit::sale_value(ch, store, guid, self.sell_percent / 100.0);
                if ui.button(lang.tr_fmt("Sell for {0}", &[&format::nuyen(value)])).on_hover_text(lang.tr("Removes the item and adds the proceeds to the log")).clicked() {
                    match edit::sell(ch, store, guid, self.sell_percent / 100.0) {
                        Ok(v) => {
                            res.status = Some((format!("Sold for {}", format::nuyen(v)), false));
                            res.changed = true;
                            res.removed = true;
                        }
                        Err(e) => res.status = Some((e.to_string(), true)),
                    }
                }
            });
            return;
        }
        let text = if self.confirm_remove { lang.tr("Click again to delete") } else { format!("🗑 {}", lang.tr("Delete")) };
        let b = ui.button(RichText::new(text).color(if self.confirm_remove { WARN } else { ui.visuals().text_color() }));
        if b.on_hover_text(lang.tr("Also removes its improvements and everything inside it")).clicked() {
            if self.confirm_remove {
                if edit::remove(ch, guid) {
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
