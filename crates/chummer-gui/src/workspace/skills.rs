//! The Skills page: active skills, skill groups and knowledge skills as
//! Workspace tables. Creation edits points and karma with steppers;
//! career shows a "+1 · cost" button on every rating (the same
//! `CareerAction`s as Classic, costed by `chummer_core::career`), and
//! the "Other advances" karma can buy.

use chummer_core::calc::SkillValues;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::career;
use eframe::egui::{self, RichText};

use super::{attr_long, k, Selected};
use crate::pdf_ui::Status;
use crate::theme;
use crate::view::{CareerAction, CharacterView};
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// What a row asked for while the table was drawn.
enum Edit {
    Set(Command),
    Action(CareerAction),
    Select(Selected),
    Roll(i32),
    Book(SourceRef),
}

const ROW: f32 = 32.0;

impl CharacterView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ws_skills(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        let ws = theme::ws(ui);
        let career_mode = self.doc.created;
        let mut edits: Vec<Edit> = Vec::new();
        // Filter and totals.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(icons::icon(icons::MAGNIFYING_GLASS, 14.0, ws.muted));
            widgets::text_field(ui, &mut self.skill_filter, &lang.tr("Filter skills"), 200.0);
            widgets::check(ui, &mut self.only_rated, &lang.tr("Only skills with a rating"));
        });
        let note = if career_mode {
            format!("{} · {} {}", lang.tr("career · every button shows its karma cost"), lang.tr("Karma value of skills:"), self.sheet.skill_karma_spent)
        } else {
            format!("{} {} / {} · {} {}", lang.tr("Knowledge Points:"), self.sheet.knowledge_points_used, self.sheet.knowledge_points, lang.tr("karma spent on skills:"), self.sheet.skill_karma_spent)
        };
        widgets::heading(ui, &lang.tr("Active Skills"), &note, 13.0, |_| {});
        let rows: Vec<SkillValues> = self.sheet.skills.iter().filter(|s| crate::view::skill_matches(&self.skill_filter, self.only_rated, &s.name, s.rating)).cloned().collect();
        self.ws_skill_table(ui, engine, lang, &rows, false, &mut edits);
        if !self.doc.skill_groups.is_empty() {
            ui.add_space(4.0);
            widgets::heading(ui, &lang.tr("Skill Groups"), "", 13.0, |_| {});
            self.ws_group_table(ui, engine, lang, &mut edits);
        }
        ui.add_space(4.0);
        let mut add = false;
        widgets::heading(ui, &lang.tr("Knowledge Skills"), &format!("{} + {}", lang.tr("INT"), lang.tr("LOG")), 13.0, |ui| {
            let ok = !self.new_kno.0.trim().is_empty();
            let label = lang.tr("Add");
            add = ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::PLUS), &label, Look::Secondary, 24.0)).inner.clicked();
            if self.new_kno.1 == "Language" {
                widgets::check(ui, &mut self.new_kno.2, &lang.tr("Native"));
            }
            crate::combo::Combo::from_id_salt("ws_kno_type").selected_text(lang.data_name("skills.xml", "", &self.new_kno.1)).width(120.0).show_ui(ui, |ui| {
                for t in ["Academic", "Interest", "Language", "Professional", "Street"] {
                    crate::combo::selectable_value(ui, &mut self.new_kno.1, t.to_owned(), lang.data_name("skills.xml", "", t));
                }
            });
            knowledge_name_input(ui, &self.store, lang, &mut self.new_kno, 220.0);
        });
        if add {
            let name = self.new_kno.0.trim().to_owned();
            if career_mode {
                edits.push(Edit::Action(CareerAction::LearnKnowledge(name, self.new_kno.1.clone())));
            } else {
                let native = self.new_kno.1 == "Language" && self.new_kno.2;
                edits.push(Edit::Set(Command::AddKnowledgeSkill { name, kind: self.new_kno.1.clone(), native }));
            }
            self.new_kno.0.clear();
        }
        let kno: Vec<SkillValues> = self.sheet.knowledge_skills.iter().filter(|s| crate::view::skill_matches(&self.skill_filter, self.only_rated, &s.name, s.rating)).cloned().collect();
        if kno.is_empty() {
            ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
        } else {
            self.ws_skill_table(ui, engine, lang, &kno, true, &mut edits);
        }
        if career_mode {
            ui.add_space(4.0);
            self.ws_other_advances(ui, engine, lang, &mut edits);
        }
        let mut changed = false;
        for e in edits {
            match e {
                Edit::Set(c) => changed |= self.doc.set(c),
                Edit::Action(a) => self.action = Some(a),
                Edit::Select(s) => self.ws_select(s),
                Edit::Roll(p) => *roll = Some(p.max(1) as u32),
                Edit::Book(r) => crate::pdf_ui::open(pdfs, &r, status),
            }
        }
        changed
    }

    /// Active or knowledge skills as a table.
    fn ws_skill_table(&self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, rows: &[SkillValues], knowledge: bool, edits: &mut Vec<Edit>) {
        let ws = theme::ws(ui);
        let career_mode = self.doc.created;
        let cap = if career_mode { self.rules.max_skill_rating_career } else { self.rules.max_skill_rating_create };
        let marks = self.item_marks(lang);
        let karma = self.doc.karma;
        let first = if knowledge { lang.tr("Knowledge skill") } else { lang.tr("Active skill") };
        let second = if knowledge { lang.tr("Type") } else { lang.tr("Attr") };
        let (captions, spec): (Vec<String>, Vec<f32>) = if career_mode {
            (vec![first, second, lang.tr("Rating"), String::new(), lang.tr("Pool"), lang.tr("Advance"), String::new()], vec![0.0, if knowledge { 76.0 } else { 44.0 }, 44.0, 96.0, 56.0, 118.0, 24.0])
        } else {
            (vec![first, second, lang.tr("Points"), lang.tr("Karma"), lang.tr("Rating"), String::new(), lang.tr("Pool"), String::new()], vec![0.0, if knowledge { 76.0 } else { 44.0 }, 84.0, 84.0, 44.0, 96.0, 50.0, 24.0])
        };
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &spec);
            let caps: Vec<&str> = captions.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for s in rows {
                let selected = self.ws_build.sel.as_ref() == Some(&Selected::Skill(s.guid.clone()));
                let (base, karma_levels) = if knowledge {
                    self.doc.knowledge_skills.iter().find(|x| x.guid == s.guid).map_or((0, 0), |x| (x.base, x.karma))
                } else {
                    self.doc.skills.iter().find(|x| x.guid == s.guid).map_or((0, 0), |x| (x.base, x.karma))
                };
                let cost = if career_mode && !s.disabled && !s.native { self.career_costs(engine).skill(&self.doc, &s.guid) } else { None };
                let mut w = widths.iter().copied();
                let mut next = || w.next().unwrap_or(40.0);
                let ink = if s.disabled { ws.muted } else { ws.text };
                let row = widgets::table_row(ui, &s.guid, selected, ROW, |ui| {
                    widgets::cell(ui, next(), ROW, |ui| {
                        if let Some((msg, err)) = marks.get(&s.guid) {
                            widgets::issue_mark(ui, *err, msg);
                        }
                        let r = ui.label(RichText::new(&s.name).size(12.5).color(ink));
                        if let Some(src) = SourceRef::new(&s.source, &s.page) {
                            r.on_hover_text(format!("{src} · {}", s.group));
                        }
                        if !s.specs.is_empty() {
                            ui.add(egui::Label::new(RichText::new(format!("{} +{}", s.specs.join(", "), s.spec_bonus)).size(11.0).color(ws.muted)).truncate());
                        }
                    });
                    widgets::cell(ui, next(), ROW, |ui| {
                        let t = if knowledge { lang.data_name("skills.xml", "", &s.category) } else { s.attribute.clone() };
                        ui.add(egui::Label::new(RichText::new(t).size(11.5).color(ws.muted)).truncate());
                    });
                    if !career_mode {
                        for (karma_col, value) in [(false, base), (true, karma_levels)] {
                            widgets::cell(ui, next(), ROW, |ui| {
                                if s.native {
                                    ui.label(RichText::new(lang.tr("native")).size(11.5).color(ws.muted));
                                    return;
                                }
                                let mut v = value;
                                let tips = (lang.tr_fmt("Lower {0}", &[&s.name]), lang.tr_fmt("Raise {0}", &[&s.name]));
                                let r = ui.add_enabled_ui(!s.disabled, |ui| widgets::stepper(ui, (karma_col, &s.guid), &mut v, 0, cap, &tips.0, &tips.1)).inner;
                                if r {
                                    let skill = s.guid.clone();
                                    edits.push(Edit::Set(match (knowledge, karma_col) {
                                        (false, false) => Command::SetSkillBase { skill, value: v },
                                        (false, true) => Command::SetSkillKarma { skill, value: v },
                                        (true, false) => Command::SetKnowledgeBase { skill, value: v },
                                        (true, true) => Command::SetKnowledgeKarma { skill, value: v },
                                    }));
                                }
                            });
                        }
                    }
                    widgets::cell(ui, next(), ROW, |ui| ui.label(widgets::mono(if s.native { "N".to_owned() } else { s.rating.to_string() }, 13.0, ink)));
                    widgets::cell(ui, next(), ROW, |ui| {
                        if !s.native {
                            widgets::pips(ui, s.rating, cap);
                        }
                    });
                    widgets::cell(ui, next(), ROW, |ui| {
                        let shown = if s.native || (s.rating == 0 && !s.default) { "—".to_owned() } else { s.pool.to_string() };
                        let r = ui.add(egui::Label::new(widgets::mono(shown, 12.5, ws.accent)).sense(egui::Sense::click())).on_hover_text(lang.tr("Roll this pool")).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() {
                            edits.push(Edit::Roll(s.pool));
                        }
                    });
                    if career_mode {
                        widgets::cell(ui, next(), ROW, |ui| match cost {
                            Some(c) => {
                                let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&s.name, &(s.rating + 1), &c]);
                                if widgets::cost_button(ui, &format!("+1 · {}", k(c)), karma >= c, &tip, &lang.tr("Not enough karma")).clicked() {
                                    edits.push(Edit::Action(CareerAction::RaiseSkill(s.guid.clone())));
                                }
                            }
                            None if s.native => {
                                ui.label(RichText::new(lang.tr("native")).size(11.5).color(ws.muted));
                            }
                            None if !s.disabled => {
                                ui.label(RichText::new(lang.tr("at maximum")).size(11.5).color(ws.muted));
                            }
                            None => {}
                        });
                    }
                    widgets::cell(ui, next(), ROW, |ui| {
                        if knowledge {
                            if widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove")).clicked() {
                                edits.push(Edit::Set(Command::RemoveKnowledgeSkill { skill: s.guid.clone() }));
                            }
                        } else if !s.disabled {
                            self.spec_menu(ui, engine, lang, s, edits);
                        }
                    });
                });
                if row.clicked() {
                    edits.push(Edit::Select(Selected::Skill(s.guid.clone())));
                }
                if row.double_clicked() {
                    if let Some(r) = SourceRef::new(&s.source, &s.page) {
                        edits.push(Edit::Book(r));
                    }
                }
            }
            ui.add_space(4.0);
        });
    }

    /// The + menu of an active skill: its specializations not taken yet
    /// (career: with their karma cost).
    fn spec_menu(&self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, s: &SkillValues, edits: &mut Vec<Edit>) {
        let career_mode = self.doc.created;
        let r = widgets::icon_button(ui, icons::PLUS, 22.0).on_hover_text(lang.tr("Add a specialization"));
        let suid = self.doc.skills.iter().find(|x| x.guid == s.guid).map(|x| x.suid.clone()).unwrap_or_default();
        let cost = if career_mode { self.career_costs(engine).specialization(&self.doc, &s.guid) } else { None };
        egui::Popup::menu(&r).show(|ui| {
            let opts = engine.catalog.get(&suid).map(|d| d.specs.clone()).unwrap_or_default();
            let opts: Vec<String> = opts.into_iter().filter(|o| !s.specs.contains(o)).collect();
            if opts.is_empty() {
                ui.label(lang.tr("None."));
            }
            for o in opts {
                let label = match cost {
                    Some(c) => format!("{o}  ·  {}", k(c)),
                    None => o.clone(),
                };
                let ok = cost.is_none_or(|c| self.doc.karma >= c);
                if ui.add_enabled(ok, egui::Button::new(label)).clicked() {
                    edits.push(if career_mode { Edit::Action(CareerAction::Specialize(s.guid.clone(), o)) } else { Edit::Set(Command::AddSpecialization { skill: s.guid.clone(), name: o }) });
                    ui.close();
                }
            }
        });
    }

    /// Skill groups: points and karma in creation, "+1 · cost" in career.
    fn ws_group_table(&self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, edits: &mut Vec<Edit>) {
        let ws = theme::ws(ui);
        let career_mode = self.doc.created;
        let cap = if career_mode { self.rules.max_skill_rating_career } else { self.rules.max_skill_rating_create };
        let karma = self.doc.karma;
        let (captions, spec): (Vec<String>, Vec<f32>) = if career_mode {
            (vec![lang.tr("Group"), lang.tr("Rating"), String::new(), lang.tr("Advance")], vec![0.0, 44.0, 96.0, 118.0])
        } else {
            (vec![lang.tr("Group"), lang.tr("Points"), lang.tr("Karma"), lang.tr("Rating"), String::new()], vec![0.0, 84.0, 84.0, 44.0, 96.0])
        };
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &spec);
            let caps: Vec<&str> = captions.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for g in &self.doc.skill_groups {
                let selected = self.ws_build.sel.as_ref() == Some(&Selected::Group(g.name.clone()));
                let cost = if career_mode { self.career_costs(engine).group(&self.doc, &g.name) } else { None };
                let mut w = widths.iter().copied();
                let mut next = || w.next().unwrap_or(40.0);
                let row = widgets::table_row(ui, ("group", &g.name), selected, ROW, |ui| {
                    widgets::cell(ui, next(), ROW, |ui| ui.label(RichText::new(lang.data_name("skills.xml", "", &g.name)).size(12.5).color(ws.text)));
                    if !career_mode {
                        for (karma_col, value) in [(false, g.base), (true, g.karma)] {
                            widgets::cell(ui, next(), ROW, |ui| {
                                let mut v = value;
                                if widgets::stepper(ui, ("group", karma_col, &g.name), &mut v, 0, cap, &lang.tr_fmt("Lower {0}", &[&g.name]), &lang.tr_fmt("Raise {0}", &[&g.name])) {
                                    let group = g.name.clone();
                                    edits.push(Edit::Set(if karma_col { Command::SetGroupKarma { group, value: v } } else { Command::SetGroupBase { group, value: v } }));
                                }
                            });
                        }
                    }
                    widgets::cell(ui, next(), ROW, |ui| ui.label(widgets::mono(g.rating().to_string(), 13.0, ws.text)));
                    widgets::cell(ui, next(), ROW, |ui| widgets::pips(ui, g.rating(), cap));
                    if career_mode {
                        widgets::cell(ui, next(), ROW, |ui| {
                            if let Some(c) = cost {
                                let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&g.name, &(g.rating() + 1), &c]);
                                if widgets::cost_button(ui, &format!("+1 · {}", k(c)), karma >= c, &tip, &lang.tr("Not enough karma")).clicked() {
                                    edits.push(Edit::Action(CareerAction::RaiseGroup(g.name.clone())));
                                }
                            }
                        });
                    }
                });
                if row.clicked() {
                    edits.push(Edit::Select(Selected::Group(g.name.clone())));
                }
            }
            ui.add_space(4.0);
        });
    }

    /// Career: other things karma buys, as cards with their cost (dim
    /// when the karma is not there): the cheapest attribute raises,
    /// initiation or submersion, and new qualities and martial arts
    /// (their cost shows in the picker).
    fn ws_other_advances(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, edits: &mut Vec<Edit>) {
        let karma = self.doc.karma;
        widgets::heading(ui, &lang.tr("Other advances"), &lang.tr_fmt("greyed: costs more than {0} karma", &[&karma]), 13.0, |_| {});
        // (icon, title, detail, value, enabled, what a click does)
        enum Click {
            Action(CareerAction),
            Select(&'static str),
        }
        let mut cards: Vec<(&str, String, String, String, bool, Click)> = Vec::new();
        let mut raises: Vec<(i32, &str, i32)> = self
            .shown_attributes()
            .into_iter()
            .filter_map(|a| Some((self.career_costs(engine).attribute(&self.doc, a)?, a, self.sheet.attr_values(a)?.value)))
            .collect();
        raises.sort();
        for (c, a, v) in raises.into_iter().take(3) {
            cards.push((icons::ARROW_FAT_UP, format!("{} {v} → {}", attr_long(lang, a), v + 1), lang.tr("Attribute"), k(c), karma >= c, Click::Action(CareerAction::RaiseAttribute(a.to_owned()))));
        }
        if self.doc.mag_enabled() || self.doc.res_enabled() {
            let techno = self.doc.res_enabled() && !self.doc.mag_enabled();
            let grade = self.doc.doc.get_i32(if techno { "submersiongrade" } else { "initiategrade" }).unwrap_or(0);
            let c = career::initiation_karma_cost(engine, &self.doc, self.initiation);
            let title = if techno { lang.tr("Submerge") } else { lang.tr("Initiate") };
            let detail = format!("{} {} → {}", if techno { lang.tr("Submersion Grade") } else { lang.tr("Initiate Grade") }, grade, grade + 1);
            cards.push((icons::EYE, title, detail, k(c), karma >= c, Click::Action(CareerAction::Initiate(self.initiation))));
        }
        cards.push((icons::USER_CIRCLE, lang.tr("Add Quality…"), lang.tr("Positive or negative quality"), "…".into(), true, Click::Select("quality")));
        cards.push((icons::HAND_FIST, lang.tr_fmt("Add {0}…", &[&crate::view::kind_noun(lang, "Martial Art")]), lang.tr("Style and techniques"), "…".into(), true, Click::Select("martialart")));
        let gap = 6.0;
        let total = ui.available_width();
        let per_row = ((total + gap) / (220.0 + gap)).floor().clamp(1.0, 3.0) as usize;
        let w = (total - gap * (per_row - 1) as f32) / per_row as f32;
        let mut picked = None;
        for chunk in cards.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (glyph, title, detail, value, enabled, click) in chunk {
                    if widgets::advance_card(ui, glyph, title, detail, value, *enabled, w).clicked() {
                        picked = Some(match click {
                            Click::Action(a) => Click::Action(a.clone()),
                            Click::Select(t) => Click::Select(t),
                        });
                    }
                }
            });
        }
        match picked {
            Some(Click::Action(a)) => edits.push(Edit::Action(a)),
            Some(Click::Select(t)) => self.open_select(t, engine),
            None => {}
        }
    }
}

/// The name of a new knowledge skill: free text with the knowledge skills
/// of `skills.xml` as presets; picking one also sets its type.
pub(crate) fn knowledge_name_input(ui: &mut egui::Ui, store: &chummer_core::data::DataStore, lang: &Language, kno: &mut (String, String, bool), width: f32) {
    let Ok(doc) = store.doc("skills.xml") else { return };
    let mut recs: Vec<(String, String, String)> =
        chummer_core::data::records(&doc, "knowledgeskills", "skill").into_iter().filter(|r| !r.hidden()).map(|r| (r.name(), lang.data_name("skills.xml", &r.id(), &r.name()), r.category())).collect();
    recs.sort_by_cached_key(|r| r.1.to_lowercase());
    let presets: Vec<(String, String)> = recs.iter().map(|(n, shown, _)| (n.clone(), shown.clone())).collect();
    if widgets::preset_input(ui, "new_knowledge", &mut kno.0, &presets, &lang.tr("New Knowledge Skill"), width).changed() {
        if let Some((_, _, cat)) = recs.iter().find(|(n, _, _)| *n == kno.0) {
            if ["Academic", "Interest", "Language", "Professional", "Street"].contains(&cat.as_str()) {
                kno.1 = cat.clone();
            }
        }
    }
}
