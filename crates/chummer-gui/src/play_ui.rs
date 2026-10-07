//! In-play tracking at the table: the Edge boxes in the sidebar, and in
//! the item pane a weapon's ammunition (clips, fire, reload, unload), a
//! device's matrix attributes, matrix condition monitor and active
//! commlink, and a vehicle's damage track.
//!
//! All changes go through `chummer_core::play`; this file only draws.

use chummer_core::calc::Sheet;
use chummer_core::command::Command;
use chummer_core::data::DataStore;
use chummer_core::items::vehicle::VehicleRules;
use chummer_core::lang::Language;
use chummer_core::play::{ammo, matrix, vehicle};
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::doc::Doc;
use crate::view::cm_track;


/// Sidebar Edge track (career mode): click a box to mark Edge spent up to
/// it, ⟲ regains it all. Spending and regaining single points and burning
/// Edge are on the Karma & Nuyen tab (`career_ui`).
pub fn edge_track(ui: &mut egui::Ui, ch: &mut Doc, sheet: &Sheet, lang: &Language) -> bool {
    if !ch.created {
        return false;
    }
    let total = sheet.attr("EDG").max(0);
    let mut spent = ch.doc.get_i32("edgeused").unwrap_or(0).clamp(0, total);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.tr("Edge")).strong());
        ui.weak(format!("{} / {}", total - spent, total));
        if ui.small_button(crate::theme::glyph("⟲")).on_hover_text(lang.tr("Reset")).clicked() {
            changed |= ch.set(Command::RefreshEdge);
        }
    });
    if cm_track(ui, "edge", total, 0, &mut spent, crate::theme::palette(ui).edge) {
        changed |= ch.set(Command::SetEdgeUsed { used: spent });
    }
    changed
}

/// State of the item pane's play section between frames.
#[derive(Default)]
pub struct PlayPanel {
    /// Weapon the choices below belong to.
    weapon: String,
    /// Ammunition picked for Reload: a gear guid, or "" for an external
    /// source.
    ammo: Option<String>,
    /// Rounds to load (one of `ammo::reload_counts`).
    count: String,
    /// Charges to set for a weapon without ammunition.
    charges: i32,
    /// A "Not enough Ammunition" question waiting for Yes/No.
    confirm: Option<String>,
}

impl PlayPanel {
    /// The play section of the item pane for item `guid`, if it has one.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, guid: &str, status: &mut Option<(String, bool)>) -> bool {
        let Some(e) = chummer_core::items::edit::find(ch, guid).cloned() else { return false };
        let mut changed = false;
        if e.name == "vehicle" {
            changed |= vehicle_track(ui, ch, lang, &e);
        }
        if e.name == "weapon" && ch.created && ammo::uses_ammo(&e) {
            changed |= self.weapon_ui(ui, ch, store, lang, &e, status);
        }
        if matrix::has_matrix(&e) {
            changed |= device_ui(ui, ch, lang, &e);
        }
        changed
    }

    fn weapon_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, w: &Element, status: &mut Option<(String, bool)>) -> bool {
        let guid = w.get("guid");
        self.for_weapon(&guid);
        let mut changed = false;
        let clips = ammo::clips(w);
        if clips.is_empty() {
            return false;
        }
        let slot = ammo::active_slot(w);
        let left = ammo::remaining(w);
        let loaded = ammo::loaded(ch, w).map(|g| g.get("name"));
        let names: Vec<String> = clips
            .iter()
            .map(|c| {
                let gear = c.ammo.as_deref().and_then(|id| chummer_core::items::edit::find(ch, id)).map(|g| g.get("name"));
                gear.unwrap_or_else(|| lang.tr(if c.count > 0 { if ammo::requires_ammo(w) { "External Source" } else { "Internal" } } else { "None" }))
            })
            .collect();

        ui.add_space(6.0);
        ui.label(RichText::new(lang.tr("Ammo Remaining:")).strong());
        egui::Grid::new(("ammo", &guid)).num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            if clips.len() > 1 {
                ui.label(lang.tr("Ammo:"));
                let text = |i: usize, c: &ammo::Clip| format!("{} · {} ({})", lang.tr_fmt("Slot {0}", &[&(i + 1)]), names[i], c.count);
                let mut pick = slot;
                crate::combo::Combo::from_id_salt(("clip", &guid)).selected_text(text(slot - 1, &clips[slot - 1])).show_ui(ui, |ui| {
                    for (i, c) in clips.iter().enumerate() {
                        let mut t = text(i, c);
                        if let Some(o) = &c.owner {
                            t = format!("{t} – {o}");
                        }
                        crate::combo::selectable_value(ui, &mut pick, i + 1, t);
                    }
                });
                if pick != slot {
                    changed |= ch.set(Command::SetActiveClip { weapon: guid.clone(), slot: pick as u32 });
                }
                ui.end_row();
            }
            ui.label(lang.tr("Current Ammo:"));
            ui.label(format!("{} × {}", left, loaded.unwrap_or_else(|| names[slot - 1].clone())));
            ui.end_row();
        });

        // Fire buttons (`cmsAmmoExpense`).
        ui.horizontal_wrapped(|ui| {
            for mode in ammo::FireMode::ALL {
                if !ammo::allows(ch, w, mode) {
                    continue;
                }
                let n = ammo::rounds(w, mode);
                let unit = lang.tr(if n == 1 { "Bullet" } else { "Bullets" });
                let b = ui.add_enabled(left > 0 && self.confirm.is_none(), egui::Button::new(lang.tr_fmt(mode.label(), &[&n, &unit])));
                if b.clicked() {
                    changed |= self.fire(ch, lang, &guid, mode, status);
                }
            }
        });
        changed |= self.confirm_ui(ui, ch, lang, &guid);

        if ammo::requires_ammo(w) {
            changed |= self.reload_ui(ui, ch, store, lang, w, status);
        } else {
            changed |= self.charges_ui(ui, ch, lang, w);
        }
        changed
    }

    /// Start on weapon `guid`: the choices of another weapon are dropped.
    pub(crate) fn for_weapon(&mut self, guid: &str) {
        if self.weapon != guid {
            *self = PlayPanel { weapon: guid.to_owned(), ..Default::default() };
        }
    }

    /// A fire button (`cmsAmmoExpense`): fire, or ask when there are not
    /// enough rounds ([`PlayPanel::confirm_ui`]). Returns true if the
    /// character changed.
    pub(crate) fn fire(&mut self, ch: &mut Doc, lang: &Language, guid: &str, mode: ammo::FireMode, status: &mut Option<(String, bool)>) -> bool {
        match ch.apply(Command::Fire { weapon: guid.to_owned(), mode }) {
            Ok(_) => true,
            Err(e) if e.confirm => {
                self.confirm = Some(e.reason);
                false
            }
            Err(e) => {
                *status = Some((lang.tr(&e.reason), true));
                false
            }
        }
    }

    /// Whether a fire question waits for an answer.
    pub(crate) fn asking(&self) -> bool {
        self.confirm.is_some()
    }

    /// Whether the choices are for weapon `guid`.
    pub(crate) fn is_for(&self, guid: &str) -> bool {
        self.weapon == guid
    }

    /// The "Not enough Ammunition" question: OK empties the clip.
    pub(crate) fn confirm_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, lang: &Language, guid: &str) -> bool {
        let mut changed = false;
        if let Some(m) = self.confirm.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(crate::theme::warn(ui), lang.tr(&m));
                if ui.button(lang.tr("OK")).clicked() {
                    changed |= ch.set(Command::SetAmmoRemaining { weapon: guid.to_owned(), count: 0 });
                    self.confirm = None;
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    self.confirm = None;
                }
            });
        }
        changed
    }

    /// Reload of a weapon without ammunition: the new number of charges.
    pub(crate) fn charges_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, lang: &Language, w: &Element) -> bool {
        let guid = w.get("guid");
        let left = ammo::remaining(w);
        let max = ammo::capacity(ch, w);
        if self.charges < left {
            self.charges = left;
        }
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.charges).range(left..=max.max(left)));
            if ui.add_enabled(self.charges != left, egui::Button::new(lang.tr("Reload"))).on_hover_text(lang.tr_fmt("Select the new number of charges/ammo that {0} should have.", &[&w.get("name")])).clicked() {
                changed |= ch.set(Command::SetCharges { weapon: guid.clone(), count: self.charges });
            }
        });
        changed
    }

    /// `Weapon.Reload` / `Unload` for a weapon that needs ammunition.
    pub(crate) fn reload_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, w: &Element, status: &mut Option<(String, bool)>) -> bool {
        let guid = w.get("guid");
        let mut changed = false;
        let choices = ammo::reloadable(ch, Some(store), &guid);
        let all_counts = ammo::reload_counts(ch, w);
        let external = all_counts.iter().any(|c| c == "External Source");
        let counts: Vec<String> = all_counts.into_iter().filter(|c| c.trim().parse::<i32>().is_ok()).collect();
        if self.ammo.as_deref().is_some_and(|a| !a.is_empty() && !choices.iter().any(|c| c.0 == a)) {
            self.ammo = None;
        }
        if self.ammo.is_none() {
            self.ammo = choices.first().map(|c| c.0.clone()).or_else(|| external.then(String::new));
        }
        if !counts.contains(&self.count) {
            self.count = counts.first().cloned().unwrap_or_default();
        }
        let label = |a: &str| -> String {
            if a.is_empty() {
                return lang.tr("External Source");
            }
            choices.iter().find(|c| c.0 == a).map(|c| format!("{} ({})", c.1, chummer_core::improvement::fmt_num(c.2))).unwrap_or_default()
        };
        ui.horizontal_wrapped(|ui| {
            if choices.is_empty() && !external {
                ui.weak(lang.tr_fmt("You do not have any Ammunition for {0} remaining!", &[&w.get("name")]));
            } else {
                let cur = self.ammo.clone().unwrap_or_default();
                crate::combo::Combo::from_id_salt(("reload_ammo", &guid)).selected_text(label(&cur)).show_ui(ui, |ui| {
                    for c in &choices {
                        crate::combo::selectable_value(ui, &mut self.ammo, Some(c.0.clone()), label(&c.0));
                    }
                    if external {
                        crate::combo::selectable_value(ui, &mut self.ammo, Some(String::new()), label(""));
                    }
                });
                if counts.len() > 1 {
                    crate::combo::Combo::from_id_salt(("reload_count", &guid)).selected_text(self.count.clone()).width(60.0).show_ui(ui, |ui| {
                        for c in &counts {
                            crate::combo::selectable_value(ui, &mut self.count, c.clone(), c);
                        }
                    });
                }
                let n: i32 = self.count.trim().parse().unwrap_or(0);
                if ui.add_enabled(self.ammo.is_some() && n > 0, egui::Button::new(lang.tr("Reload"))).clicked() {
                    let a = self.ammo.clone().unwrap_or_default();
                    changed |= ch.run(Command::Reload { weapon: guid.clone(), ammo: (!a.is_empty()).then_some(a), count: n }, status).is_some();
                }
            }
            let can_unload = ammo::loaded(ch, w).is_some() && ammo::remaining(w) != 0;
            if ui.add_enabled(can_unload, egui::Button::new(lang.tr("Unload"))).clicked() {
                changed |= ch.set(Command::Unload { weapon: guid.clone() });
            }
        });
        changed
    }
}

/// A vehicle's physical condition monitor.
fn vehicle_track(ui: &mut egui::Ui, ch: &mut Doc, lang: &Language, v: &Element) -> bool {
    let rules = VehicleRules::default();
    let boxes = vehicle::condition_monitor(v, &rules);
    let mut filled = vehicle::filled(v);
    ui.add_space(6.0);
    ui.label(RichText::new(lang.tr("Condition Monitor")).strong());
    if cm_track(ui, &format!("vcm{}", v.get("guid")), boxes, 0, &mut filled, crate::theme::palette(ui).physical) {
        return ch.set(Command::SetVehicleDamage { vehicle: v.get("guid"), filled });
    }
    false
}

/// Matrix attributes, active commlink and matrix condition monitor of a
/// device.
fn device_ui(ui: &mut egui::Ui, ch: &mut Doc, lang: &Language, e: &Element) -> bool {
    let guid = e.get("guid");
    let mut changed = false;
    ui.add_space(6.0);
    egui::Grid::new(("matrix", &guid)).num_columns(5).spacing([10.0, 2.0]).show(ui, |ui| {
        for l in ["Device Rating", "Attack", "Sleaze", "Data Processing", "Firewall"] {
            ui.weak(lang.tr(l));
        }
        ui.end_row();
        for l in ["Device Rating", "Attack", "Sleaze", "Data Processing", "Firewall"] {
            ui.strong(matrix::total(e, l).to_string());
        }
        ui.end_row();
    });
    if matrix::is_commlink(e) {
        let mut on = e.get_bool("active").unwrap_or(false);
        if ui.checkbox(&mut on, lang.tr("Active Commlink")).changed() {
            changed |= ch.set(Command::SetActiveCommlink { device: guid.clone(), on });
        }
    }
    // `chkGearHomeNode` & co.: only A.I.s have a home node.
    if ch.is_ai() {
        let mut on = e.get_bool("homenode").unwrap_or(false);
        let depth = chummer_core::calc::attribute_values(ch, "DEP", &Default::default()).total;
        let can = on || matrix::can_be_home_node(e, depth);
        if ui.add_enabled(can, egui::Checkbox::new(&mut on, lang.tr("Home Node"))).changed() {
            changed |= ch.set(Command::SetHomeNode { device: guid.clone(), on });
        }
    }
    ui.label(RichText::new(lang.tr("Matrix Condition Monitor")).strong());
    let mut filled = matrix::filled(e);
    if cm_track(ui, &format!("mcm{guid}"), matrix::condition_monitor(e), 0, &mut filled, crate::theme::palette(ui).matrix) {
        changed |= ch.set(Command::SetMatrixDamage { device: guid.clone(), filled });
    }
    changed
}
