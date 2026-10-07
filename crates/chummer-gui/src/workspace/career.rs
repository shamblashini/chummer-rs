//! Career mode in the Workspace (A-career.dc.html): karma and nuyen
//! tiles, the ledger with Undo on the entries that can be undone, the
//! manual entry, and the command palette's advances ("Raise Pistols
//! 6 → 7", with the cost and the karma after it). Costs come from
//! `chummer_core::career`; purchases are the Classic `CareerAction`s.

use chummer_core::career::{self, ExpenseType};
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::{format, improvement};
use eframe::egui::{self, RichText};

use super::{attr_long, k};
use crate::theme;
use crate::view::{CareerAction, CharacterView};
use crate::workspace::icons;
use crate::workspace::palette::{Entry, Kind, Raise, Target};
use crate::workspace::widgets::{self, Look};

/// The date part of a ledger entry's ISO date.
fn day(date: &str) -> &str {
    date.split('T').next().unwrap_or(date)
}

impl CharacterView {
    /// Karma available, career karma, nuyen and street cred as tiles,
    /// `per_row` to a row, with notoriety and public awareness under them.
    pub(crate) fn ws_karma_tiles(&self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, per_row: usize) {
        let ws = theme::ws(ui);
        let rep = career::reputation_for(engine, &self.doc);
        let tiles = [
            (lang.tr("Karma available"), self.doc.karma.to_string(), true),
            (lang.tr("Career Karma"), career::career_karma(&self.doc).to_string(), false),
            (lang.tr("Nuyen"), format::nuyen(self.doc.nuyen), true),
            (lang.tr("Street Cred"), rep.street_cred.to_string(), false),
        ];
        let gap = 6.0;
        let w = (ui.available_width() - gap * (per_row - 1) as f32) / per_row as f32;
        for chunk in tiles.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (caption, value, accent) in chunk {
                    widgets::tile(ui, caption, value, *accent, w);
                }
            });
        }
        ui.label(RichText::new(format!("{} {} · {} {}", lang.tr("Notoriety"), rep.notoriety, lang.tr("Public Awareness"), rep.public_awareness)).size(11.5).color(ws.muted));
    }

    /// The manual karma or nuyen entry: kind, amount, reason, Gain or
    /// Spend. Returns true if the character changed.
    pub(crate) fn ws_manual_entry(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut gain = None;
        widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let karma_tip = lang.tr("Karma");
                let nuyen_tip = lang.tr("Nuyen");
                if let Some(i) = widgets::segmented(ui, &[(&karma_tip, &karma_tip), (&nuyen_tip, &nuyen_tip)], usize::from(!self.manual.0), 24.0) {
                    self.manual.0 = i == 0;
                }
                ui.add(egui::DragValue::new(&mut self.manual.1).range(0.0..=1_000_000.0).max_decimals(2));
                widgets::text_field(ui, &mut self.manual.2, &lang.tr("Reason (e.g. run payout)"), 220.0);
                let ok = self.manual.1 > 0.0;
                if ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::PLUS), &lang.tr("Gain"), Look::Primary, 24.0)).inner.clicked() {
                    gain = Some(true);
                }
                if ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::MINUS), &lang.tr("Spend"), Look::Secondary, 24.0)).inner.clicked() {
                    gain = Some(false);
                }
            });
        });
        let Some(g) = gain else { return false };
        match self.apply_manual(g) {
            Ok(()) => true,
            Err(e) => {
                ui.label(RichText::new(e).size(12.0).color(ws.error));
                false
            }
        }
    }

    /// The ledger, newest first, filtered All / Karma / Nuyen; entries
    /// that can be undone have an Undo button (it refunds by the rules).
    /// `compact` leaves out the date column (the inspector).
    pub(crate) fn ws_ledger(&mut self, ui: &mut egui::Ui, lang: &Language, compact: bool) -> bool {
        let ws = theme::ws(ui);
        let entries = career::entries(&self.doc);
        let all = lang.tr("All");
        let karma = lang.tr("Karma");
        let nuyen = lang.tr("Nuyen");
        widgets::heading(ui, &lang.tr("Ledger"), "", 13.0, |ui| {
            if let Some(i) = widgets::segmented(ui, &[(&all, &all), (&karma, &karma), (&nuyen, &nuyen)], self.ws_build.ledger, 22.0) {
                self.ws_build.ledger = i;
            }
        });
        let shown: Vec<&career::ExpenseEntry> = entries
            .iter()
            .rev()
            .filter(|e| match self.ws_build.ledger {
                1 => e.kind == ExpenseType::Karma,
                2 => e.kind == ExpenseType::Nuyen,
                _ => true,
            })
            .collect();
        if shown.is_empty() {
            ui.label(RichText::new(lang.tr("No entries yet. Career-mode spending and income appear here.")).size(12.0).color(ws.muted));
            return false;
        }
        let can_undo = self.doc.created;
        let mut undo = None;
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let spec: Vec<f32> = if compact { vec![0.0, 70.0, 22.0] } else { vec![84.0, 0.0, 60.0, 100.0, 22.0] };
            let widths = widgets::table_columns(ui, &spec);
            let caps: Vec<String> = if compact { vec![lang.tr("Reason"), lang.tr("Amount"), String::new()] } else { lang.tr_all(["Date", "Reason", "Karma", "Nuyen", ""]).into_iter().collect() };
            let caps: Vec<&str> = caps.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for e in shown {
                let is_karma = e.kind == ExpenseType::Karma;
                let amount = if is_karma { format!("{}{}", if e.amount > 0.0 { "+" } else { "" }, improvement::fmt_num(e.amount).replace('-', "−")) } else { format!("{}{}", if e.amount > 0.0 { "+" } else { "" }, format::nuyen(e.amount).replace('-', "−")) };
                let color = if e.amount < 0.0 { ws.warning } else { ws.accent };
                let mut w = widths.iter().copied();
                let mut next = || w.next().unwrap_or(40.0);
                widgets::table_row(ui, &e.guid, false, 26.0, |ui| {
                    if !compact {
                        widgets::cell(ui, next(), 26.0, |ui| ui.label(RichText::new(day(&e.date)).size(11.5).color(ws.muted)));
                    }
                    widgets::cell(ui, next(), 26.0, |ui| {
                        let r = ui.add(egui::Label::new(RichText::new(&e.reason).size(12.0).color(ws.text)).truncate());
                        if compact {
                            r.on_hover_text(day(&e.date));
                        }
                    });
                    if compact {
                        widgets::cell(ui, next(), 26.0, |ui| ui.label(widgets::mono(amount.clone(), 12.0, color)));
                    } else {
                        widgets::cell(ui, next(), 26.0, |ui| {
                            if is_karma {
                                ui.label(widgets::mono(amount.clone(), 12.0, color));
                            }
                        });
                        widgets::cell(ui, next(), 26.0, |ui| {
                            if !is_karma {
                                ui.label(widgets::mono(amount.clone(), 12.0, color));
                            }
                        });
                    }
                    widgets::cell(ui, next(), 26.0, |ui| {
                        if can_undo && e.undo.is_some() && widgets::icon_button(ui, icons::ARROW_U_UP_LEFT, 22.0).on_hover_text(lang.tr("Reverse this and refund it")).clicked() {
                            undo = Some(e.guid.clone());
                        }
                    });
                });
            }
            ui.add_space(4.0);
        });
        if let Some(g) = undo {
            self.action = Some(CareerAction::Undo(g));
        }
        ui.label(RichText::new(lang.tr("Undo refunds by the rules; it is written to the ledger.")).size(11.0).color(ws.muted));
        false
    }

    /// Inspector, career: the karma and nuyen tiles, Add entry, and the
    /// ledger. Returns true if the character changed.
    pub fn ws_ledger_panel(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        // Its own ids: the Karma & Nuyen page draws the same widgets.
        ui.push_id("ws_ledger_panel", |ui| self.ledger_panel_inner(ui, engine, lang)).inner
    }

    fn ledger_panel_inner(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut changed = false;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        self.ws_karma_tiles(ui, engine, lang, 2);
        ui.add_space(4.0);
        let mut open = self.ws_build.adding;
        if widgets::button(ui, Some(if open { icons::CARET_DOWN } else { icons::PLUS }), &lang.tr("Add entry"), Look::Secondary, 24.0).clicked() {
            open = !open;
        }
        self.ws_build.adding = open;
        if open {
            changed |= self.ws_manual_entry(ui, lang);
        }
        ui.add_space(4.0);
        changed |= self.ws_ledger(ui, lang, true);
        changed
    }

    /// Career: what the palette offers to buy with karma — raising an
    /// attribute, skill, knowledge skill or group by one, and new
    /// specializations of rated skills — with the cost and the karma left.
    pub fn ws_raise_entries(&self, engine: &Engine, lang: &Language) -> Vec<Entry> {
        if !self.doc.created {
            return Vec::new();
        }
        let karma = self.doc.karma;
        let mut out = Vec::new();
        let mut push = |icon: &'static str, title: String, kind: String, what: String, cost: i32, raise: Raise| {
            let ok = karma >= cost;
            let detail = if ok { format!("{kind} · {}", lang.tr_fmt("{0} karma", &[&cost])) } else { format!("{kind} · {} · {}", lang.tr_fmt("{0} karma", &[&cost]), lang.tr_fmt("not affordable ({0} karma)", &[&karma])) };
            let preview = format!("{what} · {} {karma} → {}", lang.tr("karma"), karma - cost);
            out.push(Entry { kind: Kind::Advance, icon, title, detail, hint: k(cost), keywords: lang.tr("raise advance karma"), enabled: ok, target: Target::Raise { raise, preview } });
        };
        for a in self.shown_attributes() {
            let (Some(c), Some(v)) = (career::attribute_upgrade_karma_cost(engine, &self.doc, a), self.sheet.attr_values(a)) else { continue };
            let name = attr_long(lang, a);
            push(icons::ARROW_FAT_UP, lang.tr_fmt("Raise {0} {1} → {2}", &[&name, &v.value, &(v.value + 1)]), lang.tr("Attribute"), format!("{name} {}", v.value + 1), c, Raise::Attribute(a.to_owned()));
        }
        for s in self.sheet.skills.iter().chain(&self.sheet.knowledge_skills) {
            if s.disabled || s.native {
                continue;
            }
            if let Some(c) = career::skill_upgrade_karma_cost(engine, &self.doc, &s.guid) {
                let kind = if s.knowledge { lang.tr("Knowledge skill") } else { lang.tr("Skill") };
                push(icons::LIGHTNING, lang.tr_fmt("Raise {0} {1} → {2}", &[&s.name, &s.rating, &(s.rating + 1)]), kind, format!("{} {}", s.name, s.rating + 1), c, Raise::Skill(s.guid.clone()));
            }
            if s.knowledge || s.rating == 0 {
                continue;
            }
            let Some(c) = career::specialization_karma_cost(engine, &self.doc, &s.guid) else { continue };
            let suid = self.doc.skills.iter().find(|x| x.guid == s.guid).map(|x| x.suid.clone()).unwrap_or_default();
            for o in engine.catalog.get(&suid).map(|d| d.specs.clone()).unwrap_or_default().into_iter().filter(|o| !s.specs.contains(o)) {
                push(icons::PLUS_CIRCLE, lang.tr_fmt("Add specialization: {0} ({1})", &[&s.name, &o]), lang.tr("Skill"), format!("{} ({o}) +{}", s.name, s.spec_bonus.max(2)), c, Raise::Specialize(s.guid.clone(), o));
            }
        }
        for g in &self.doc.skill_groups {
            if let Some(c) = career::skill_group_upgrade_karma_cost(engine, &self.doc, &g.name) {
                push(icons::LIGHTNING, lang.tr_fmt("Raise {0} {1} → {2}", &[&g.name, &g.rating(), &(g.rating() + 1)]), lang.tr("Skill Group"), format!("{} {}", g.name, g.rating() + 1), c, Raise::Group(g.name.clone()));
            }
        }
        out
    }

    /// Buy an advance picked in the palette (run at the end of the
    /// frame, like the page's buttons).
    pub fn ws_raise(&mut self, r: Raise) {
        self.action = Some(match r {
            Raise::Attribute(a) => CareerAction::RaiseAttribute(a),
            Raise::Skill(s) => CareerAction::RaiseSkill(s),
            Raise::Group(g) => CareerAction::RaiseGroup(g),
            Raise::Specialize(s, name) => CareerAction::Specialize(s, name),
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn day_of_an_iso_date() {
        assert_eq!(super::day("2018-10-13T20:58:35"), "2018-10-13");
        assert_eq!(super::day("2018-10-13"), "2018-10-13");
    }
}
