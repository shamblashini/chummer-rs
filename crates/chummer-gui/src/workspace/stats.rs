//! The inspector for an attribute, skill or skill group picked on a
//! page: its value and range, where it comes from, what uses it, and the
//! same raise buttons as the row (creation steppers' next step, career
//! "+1 · cost").

use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::{calc, career};
use eframe::egui::{self, RichText};

use super::{attr_long, k, source_button, Selected};
use crate::pdf_ui::Status;
use crate::theme;
use crate::view::{CareerAction, CharacterView};
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// The limits an attribute counts towards (SR5 p. 101).
fn limits_of(attr: &str) -> &'static [&'static str] {
    match attr {
        "STR" | "BOD" | "REA" => &["Physical limit"],
        "LOG" | "INT" => &["Mental limit"],
        "WIL" => &["Mental limit", "Social limit"],
        "CHA" => &["Social limit"],
        _ => &[],
    }
}

impl CharacterView {
    /// The inspector section's title: what is selected.
    pub fn ws_selected_title(&self, lang: &Language) -> String {
        match &self.ws_build.sel {
            Some(Selected::Attribute(a)) => attr_long(lang, a),
            Some(Selected::Skill(g)) => self.sheet.skills.iter().chain(&self.sheet.knowledge_skills).find(|s| &s.guid == g).map(|s| s.name.clone()).unwrap_or_default(),
            Some(Selected::Group(n)) => lang.data_name("skills.xml", "", n),
            None => String::new(),
        }
    }

    /// The inspector section's contents. Returns true if the character
    /// changed.
    pub fn ws_selected_ui(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        match self.ws_build.sel.clone() {
            Some(Selected::Attribute(a)) => self.ws_attribute_info(ui, engine, lang, &a),
            Some(Selected::Skill(g)) => self.ws_skill_info(ui, engine, lang, pdfs, status, roll, &g),
            Some(Selected::Group(n)) => {
                self.ws_group_info(ui, engine, lang, &n);
                false
            }
            None => false,
        }
    }

    fn ws_attribute_info(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, name: &str) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let Some(v) = self.sheet.attr_values(name).cloned() else {
            self.ws_build.sel = None;
            return false;
        };
        let long = attr_long(lang, name);
        ui.horizontal(|ui| {
            ui.label(RichText::new(name).size(11.5).color(ws.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(v.total.to_string(), 17.0, ws.accent));
                if v.total != v.value {
                    ui.label(RichText::new(lang.tr_fmt("{0} natural", &[&v.value])).size(11.5).color(ws.muted));
                }
            });
        });
        let metatype = self.doc.field("metatype");
        ui.label(RichText::new(lang.tr_fmt("{0} range {1}–{2}, augmented maximum {3}.", &[&metatype, &v.total_min, &v.total_max, &v.total_aug_max])).size(12.0).color(ws.muted));
        if v.value >= v.total_max {
            ui.label(RichText::new(lang.tr("At its natural maximum.")).size(12.0).color(ws.warning));
        }
        let sources = self.attribute_sources(name);
        if !sources.is_empty() {
            ui.add_space(2.0);
            ui.label(widgets::overline(&lang.tr("Modified by"), &ws));
            for s in sources {
                ui.label(RichText::new(s).size(12.5).color(ws.text));
            }
        }
        // Skills on this attribute, best pool first, and the limits.
        let mut used: Vec<(String, String)> = self.sheet.skills.iter().chain(&self.sheet.knowledge_skills).filter(|s| s.attribute == name && s.rating > 0).map(|s| (s.name.clone(), s.pool.to_string())).collect();
        used.sort_by_key(|(_, p)| std::cmp::Reverse(p.parse::<i32>().unwrap_or(0)));
        used.truncate(8);
        for l in limits_of(name) {
            let value = match *l {
                "Physical limit" => self.sheet.limit_physical,
                "Mental limit" => self.sheet.limit_mental,
                _ => self.sheet.limit_social,
            };
            used.push((lang.tr(l), value.to_string()));
        }
        if !used.is_empty() {
            ui.add_space(2.0);
            ui.label(widgets::overline(&lang.tr("Used by"), &ws));
            for (label, value) in used {
                widgets::stat_row(ui, &label, &value, true);
            }
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if self.doc.created {
                match career::attribute_upgrade_karma_cost(engine, &self.doc, name) {
                    Some(c) => {
                        let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&long, &(v.value + 1), &c]);
                        if widgets::cost_button(ui, &format!("{} · {}", lang.tr_fmt("Raise to {0}", &[&(v.value + 1)]), k(c)), self.doc.karma >= c, &tip, &lang.tr("Not enough karma")).clicked() {
                            self.action = Some(CareerAction::RaiseAttribute(name.to_owned()));
                        }
                    }
                    None => {
                        ui.label(RichText::new(lang.tr("at maximum")).size(12.0).color(ws.muted));
                    }
                }
            } else if let Some(a) = self.doc.attribute(name).map(|a| (a.base, a.karma)) {
                let (base, karma) = a;
                let priority = chummer_core::character::uses_priority_tables(&self.doc.field("buildmethod"));
                let points_left = self.budget.as_ref().map_or(0, |b| b.attribute_points.0 - b.attribute_points.1);
                let special = matches!(name, "EDG" | "MAG" | "MAGAdept" | "RES" | "DEP");
                let special_left = self.budget.as_ref().map_or(0, |b| b.special_points.0 - b.special_points.1);
                let left = if special { special_left } else { points_left };
                let can_point = priority && left > 0 && base < crate::view::attribute_base_max(&v, base, karma) && v.value < v.total_max;
                if can_point && widgets::button(ui, Some(icons::ARROW_UP), &format!("{} · 1 {}", lang.tr_fmt("Raise to {0}", &[&(v.value + 1)]), lang.tr("pt")), Look::Secondary, 26.0).clicked() {
                    changed |= self.doc.set(Command::SetAttributeBase { attribute: name.to_owned(), value: base + 1 });
                }
                match calc::attribute_upgrade_cost(&v, &self.rules) {
                    Some(c) if karma < crate::view::attribute_karma_max(&v, karma) && v.value < v.total_max => {
                        let look = if can_point { Look::Ghost } else { Look::Secondary };
                        if widgets::button(ui, Some(icons::ARROW_UP), &format!("{} · {}", lang.tr_fmt("Raise to {0}", &[&(v.value + 1)]), k(c)), look, 26.0).clicked() {
                            changed |= self.doc.set(Command::SetAttributeKarma { attribute: name.to_owned(), value: karma + 1 });
                        }
                    }
                    _ => {
                        ui.label(RichText::new(lang.tr("at maximum")).size(12.0).color(ws.muted));
                    }
                }
            }
        });
        changed
    }

    #[allow(clippy::too_many_arguments)]
    fn ws_skill_info(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>, guid: &str) -> bool {
        let ws = theme::ws(ui);
        let Some(s) = self.sheet.skills.iter().chain(&self.sheet.knowledge_skills).find(|s| s.guid == guid).cloned() else {
            self.ws_build.sel = None;
            return false;
        };
        ui.horizontal(|ui| {
            let kind = if s.knowledge { lang.data_name("skills.xml", "", &s.category) } else { format!("{} · {}", s.attribute, lang.data_name("skills.xml", "", &s.category)) };
            ui.label(RichText::new(kind).size(11.5).color(ws.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(if s.native { "N".to_owned() } else { s.rating.to_string() }, 17.0, ws.accent));
            });
        });
        if !s.group.is_empty() {
            widgets::stat_row(ui, &lang.tr("Group"), &lang.data_name("skills.xml", "", &s.group), false);
        }
        if !s.native {
            widgets::stat_row(ui, &lang.tr("Points"), &s.base.to_string(), false);
            widgets::stat_row(ui, &lang.tr("Karma"), &s.karma.to_string(), false);
        }
        let pool = if s.native || (s.rating == 0 && !s.default) { "—".to_owned() } else { s.pool.to_string() };
        ui.horizontal(|ui| {
            ui.label(RichText::new(lang.tr("Pool")).size(12.0).color(ws.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::DICE_FIVE, 22.0).on_hover_text(lang.tr("Roll this pool")).clicked() {
                    *roll = Some(s.pool.max(1) as u32);
                }
                ui.label(widgets::mono(pool, 12.5, ws.accent));
            });
        });
        if s.rating == 0 && s.default && !s.knowledge {
            ui.label(RichText::new(lang.tr("Unrated: defaults to the attribute − 1.")).size(12.0).color(ws.muted));
        }
        if s.disabled {
            ui.label(RichText::new(lang.tr("Not available to this character.")).size(12.0).color(ws.warning));
        }
        if !s.specs.is_empty() {
            ui.add_space(2.0);
            ui.label(widgets::overline(&lang.tr("Specializations"), &ws));
            for sp in &s.specs {
                widgets::stat_row(ui, sp, &format!("+{}", s.spec_bonus), false);
            }
        }
        ui.add_space(4.0);
        let mut book = None;
        ui.horizontal_wrapped(|ui| {
            if self.doc.created && !s.disabled && !s.native {
                match career::skill_upgrade_karma_cost(engine, &self.doc, &s.guid) {
                    Some(c) => {
                        let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&s.name, &(s.rating + 1), &c]);
                        if widgets::cost_button(ui, &format!("{} · {}", lang.tr_fmt("Raise to {0}", &[&(s.rating + 1)]), k(c)), self.doc.karma >= c, &tip, &lang.tr("Not enough karma")).clicked() {
                            self.action = Some(CareerAction::RaiseSkill(s.guid.clone()));
                        }
                    }
                    None => {
                        ui.label(RichText::new(lang.tr("at maximum")).size(12.0).color(ws.muted));
                    }
                }
            }
            if let Some(r) = SourceRef::new(&s.source, &s.page) {
                if source_button(ui, pdfs, lang, &r) {
                    book = Some(r);
                }
            }
        });
        if let Some(r) = book {
            crate::pdf_ui::open(pdfs, &r, status);
        }
        false
    }

    fn ws_group_info(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, name: &str) {
        let ws = theme::ws(ui);
        let Some(g) = self.doc.skill_groups.iter().find(|g| g.name == name).cloned() else {
            self.ws_build.sel = None;
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(lang.tr("Skill Group")).size(11.5).color(ws.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(g.rating().to_string(), 17.0, ws.accent));
            });
        });
        widgets::stat_row(ui, &lang.tr("Points"), &g.base.to_string(), false);
        widgets::stat_row(ui, &lang.tr("Karma"), &g.karma.to_string(), false);
        ui.add_space(2.0);
        ui.label(widgets::overline(&lang.tr("Skills"), &ws));
        for s in self.sheet.skills.iter().filter(|s| s.group == name) {
            widgets::stat_row(ui, &s.name, &format!("{} · {}", s.rating, s.pool), false);
        }
        if self.doc.created {
            ui.add_space(4.0);
            if let Some(c) = career::skill_group_upgrade_karma_cost(engine, &self.doc, name) {
                let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&name, &(g.rating() + 1), &c]);
                if widgets::cost_button(ui, &format!("{} · {}", lang.tr_fmt("Raise to {0}", &[&(g.rating() + 1)]), k(c)), self.doc.karma >= c, &tip, &lang.tr("Not enough karma")).clicked() {
                    self.action = Some(CareerAction::RaiseGroup(name.to_owned()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn attributes_feed_their_limits() {
        assert_eq!(super::limits_of("STR"), ["Physical limit"]);
        assert_eq!(super::limits_of("WIL"), ["Mental limit", "Social limit"]);
        assert!(super::limits_of("EDG").is_empty());
    }
}
