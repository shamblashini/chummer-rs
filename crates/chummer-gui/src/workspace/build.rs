//! The Workspace's own character pages (Main.dc.html, A-career.dc.html):
//! Attributes & Qualities with the derived values, and the item tables
//! the other pages use (the guide's hint line is `view::guide_ui`).
//! Skills are in `skills.rs`, magic and the other build pages in
//! `magic.rs`, the story and record pages in `story.rs`, the career
//! ledger and the palette's advances in `career.rs`, and the inspector
//! for a selected attribute or skill in `stats.rs`.
//!
//! Descendants of `view` (through `character.rs`), so they use the
//! view's state; every change goes through the same commands and helpers
//! as the Classic tabs (`Doc::set`, `CareerAction`, `open_select`).

use std::collections::HashMap;
use std::sync::Arc;

use chummer_core::attributes;
use chummer_core::chargen::issues::{Area, Severity};
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sections::Section as Sec;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::tree::Entry;
use chummer_core::{calc, career, format};
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;
use crate::theme;
use crate::view::issues_ui::message;
use crate::view::{CareerAction, CharacterView, Tab};
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

#[path = "career.rs"]
pub(crate) mod career_ws;
#[path = "magic.rs"]
mod magic;
#[path = "skills.rs"]
mod skills;
#[path = "stats.rs"]
mod stats;
#[path = "story.rs"]
mod story;

/// What the inspector shows besides an item: an attribute or a skill
/// picked on a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    /// An attribute by abbreviation ("AGI").
    Attribute(String),
    /// An active or knowledge skill by guid.
    Skill(String),
    /// A skill group by name.
    Group(String),
}

/// The Workspace pages' state in a character view.
#[derive(Default)]
pub struct State {
    pub sel: Option<Selected>,
    /// The ledger's filter: 0 all, 1 karma, 2 nuyen.
    pub ledger: usize,
    /// The ledger's manual entry row is open.
    pub adding: bool,
}

/// An attribute's long name in the UI language.
pub(crate) fn attr_long(lang: &Language, name: &str) -> String {
    let key = format!("String_Attribute{name}Long");
    if lang.has(&key) {
        lang.s(&key)
    } else {
        lang.tr(attributes::long_name(name))
    }
}

/// "14 k": a karma cost in a button.
pub(crate) fn k(cost: i32) -> String {
    format!("{cost} k")
}

/// A vertical scroll area for a page, by tab.
fn page<R>(ui: &mut egui::Ui, tab: Tab, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .id_salt(("ws_page", tab as u8))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

impl CharacterView {
    /// The selected attribute or skill, for the inspector.
    pub fn ws_selected(&self) -> Option<&Selected> {
        self.ws_build.sel.as_ref()
    }

    pub fn ws_clear_selected(&mut self) {
        self.ws_build.sel = None;
    }

    /// Select an attribute or skill (the item pane closes: the inspector
    /// shows one thing).
    pub(crate) fn ws_select(&mut self, s: Selected) {
        self.item_editor = None;
        self.ws_build.sel = Some(s);
    }

    /// Open an item in the inspector (and drop the attribute or skill).
    pub(crate) fn ws_open_item(&mut self, guid: String) {
        self.ws_build.sel = None;
        self.item_editor = Some((guid, crate::item_editor::ItemEditor::default()));
    }

    /// A tab's page: the Workspace's own for the build, story and record
    /// pages, the Classic one for gear (which gets its own screens).
    /// Returns true if the character changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ws_tab_page(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        match tab {
            Tab::Common => page(ui, tab, |ui| self.ws_common(ui, engine, lang, pdfs, status)),
            Tab::Skills => page(ui, tab, |ui| self.ws_skills(ui, engine, lang, pdfs, status, roll)),
            Tab::Limits => page(ui, tab, |ui| self.ws_limits(ui, lang)),
            Tab::MartialArts | Tab::Magician | Tab::Adept | Tab::Technomancer | Tab::AdvancedPrograms | Tab::Critter | Tab::Initiation => page(ui, tab, |ui| self.ws_magic(ui, tab, engine, lang, pdfs, status)),
            Tab::CharacterInfo => page(ui, tab, |ui| self.ws_info(ui, lang)),
            Tab::Notes => page(ui, tab, |ui| self.ws_notes(ui, lang)),
            Tab::Calendar => page(ui, tab, |ui| self.ws_calendar(ui, lang)),
            Tab::Improvements => page(ui, tab, |ui| self.ws_improvements(ui, lang)),
            Tab::Relationships => self.ws_relationships(ui, lang, status),
            Tab::Karma => page(ui, tab, |ui| ui.push_id("ws_karma_page", |ui| self.ws_karma_page(ui, engine, lang)).inner),
            Tab::Cyberware | Tab::StreetGear | Tab::Vehicles => self.tab_page(ui, tab, engine, lang, pdfs, status, roll),
        }
    }

    // ----- Attributes & Qualities -----

    fn ws_common(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        let ws = theme::ws(ui);
        changed |= self.ws_identity(ui, lang);
        if !self.doc.created && self.doc.field("buildmethod") == "LifeModule" {
            widgets::card_frame(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(widgets::title(&lang.tr("Life Modules"), &ws));
                changed |= self.life_module_picker(ui, engine, lang, status);
            });
        }
        self.ws_priorities(ui, lang);
        changed |= self.ws_attribute_table(ui, engine, lang);
        ui.add_space(4.0);
        self.ws_derived(ui, lang);
        ui.add_space(4.0);
        let count = self.doc.items("qualities", "quality").len();
        let mut add = false;
        widgets::heading(ui, &lang.tr("Qualities"), &count.to_string(), 13.0, |ui| {
            add = widgets::button(ui, Some(icons::PLUS), &lang.tr("Add Quality…"), Look::Secondary, 24.0).clicked();
        });
        if add {
            self.open_select("quality", engine);
        }
        changed |= self.ws_item_table(ui, &chummer_core::sections::QUALITIES, lang, pdfs, status);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(icons::icon(icons::INFO, 13.0, ws.muted));
            let hint = if self.doc.created {
                lang.tr("Career mode: Raise spends karma and records it in the Karma & Nuyen log, where it can be undone.")
            } else {
                lang.tr("Creation mode: base uses attribute points (priority builds). Changing levels does not deduct karma automatically; the Karma cost column shows what they are worth.")
            };
            ui.label(RichText::new(hint).size(11.5).color(ws.muted));
        });
        changed
    }

    /// Alias, metatype and build; in creation the karma spent on nuyen.
    fn ws_identity(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(lang.tr("Alias:")).size(12.0).color(ws.muted));
            let mut alias = self.doc.field("alias");
            if widgets::text_field(ui, &mut alias, "", 180.0).changed() {
                changed |= self.doc.set(Command::SetField { key: "alias".into(), value: alias });
            }
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr("Metatype")).font(widgets::bold(13.0)).color(ws.text));
            ui.label(RichText::new(self.doc.field("metatype")).size(12.5).color(ws.accent));
            let variant = self.doc.field("metavariant");
            if !variant.is_empty() {
                widgets::tag(ui, &variant, ws.text, ws.divider);
            }
            let build = match self.doc.field("buildmethod").as_str() {
                "SumtoTen" => lang.tr("Sum-to-Ten"),
                "" => lang.tr("Priority"),
                b => lang.tr(b),
            };
            widgets::tag(ui, &format!("{} · {build}", if self.doc.created { lang.tr("Career") } else { lang.tr("Creation") }), ws.muted, ws.divider);
            if let Some(b) = &self.budget {
                let nuyen = b.nuyen.0;
                ui.add_space(6.0);
                ui.label(RichText::new(lang.tr("Nuyen:")).size(12.0).color(ws.muted));
                let mut bp = self.doc.doc.get_i32("nuyenbp").unwrap_or(0);
                let max = self.settings.as_ref().map_or(10, |s| s.int("nuyenmaxbp", 10));
                if widgets::stepper(ui, "nuyenbp", &mut bp, 0, max, &lang.tr("Lower"), &lang.tr("Raise")) {
                    changed |= self.doc.set(Command::SetField { key: "nuyenbp".into(), value: bp.to_string() });
                }
                ui.label(RichText::new(format!("{} = {}", lang.tr("karma"), format::nuyen(nuyen))).size(12.0).color(ws.muted)).on_hover_text(lang.tr("2,000¥ per karma"));
            }
        });
        changed
    }

    /// Priority builds in creation: the five letters as cards (chosen in
    /// the New Character wizard; read only here).
    fn ws_priorities(&self, ui: &mut egui::Ui, lang: &Language) {
        let Some(b) = &self.budget else { return };
        if !chummer_core::character::uses_priority_tables(&self.doc.field("buildmethod")) {
            return;
        }
        let ws = theme::ws(ui);
        let talent = self.doc.field("prioritytalent");
        let rows: [(&str, &str, String); 5] = [
            ("priorityresources", "Resources", format::nuyen(b.nuyen.0)),
            ("priorityattributes", "Attributes", lang.tr_fmt("{0} points", &[&b.attribute_points.0])),
            ("prioritymetatype", "Metatype", format!("{} · {}", self.doc.field("metatype"), lang.tr_fmt("{0} special", &[&b.special_points.0]))),
            ("priorityskills", "Skills", format!("{} / {}", b.skill_points.0, lang.tr_fmt("{0} groups", &[&b.skill_group_points.0]))),
            ("priorityspecial", "Magic or Resonance", if talent.is_empty() { lang.tr("Mundane") } else { lang.tr(&talent) }),
        ];
        let mut rows: Vec<(String, String, String)> = rows.into_iter().map(|(key, l, v)| (self.doc.field(key), lang.tr(l), v)).filter(|(letter, _, _)| letter.len() == 1).collect();
        if rows.is_empty() {
            return;
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        widgets::heading(ui, &lang.tr("Priorities"), &lang.tr("chosen when the character was created"), 13.0, |_| {});
        let gap = 6.0;
        let w = ((ui.available_width() - gap * (rows.len() - 1) as f32) / rows.len() as f32).max(110.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            for (letter, label, value) in rows {
                egui::Frame::new().fill(ws.raised).stroke(egui::Stroke::new(1.0_f32, ws.divider)).corner_radius(5).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width(w - 18.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(letter).size(14.0).color(ws.accent));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(RichText::new(label).size(12.5).color(ws.text)).truncate());
                            ui.add(egui::Label::new(RichText::new(value).size(11.5).color(ws.muted)).truncate());
                        });
                    });
                });
            }
        });
    }

    /// Improvements that change an attribute's value, as "+1 Wired
    /// Reflexes".
    pub(crate) fn attribute_sources(&self, name: &str) -> Vec<String> {
        let imps = &self.doc.improvements;
        imps.list
            .iter()
            .filter(|i| i.kind == "Attribute" && i.improved_name == name && imps.applies(i) && (i.aug != 0.0 || i.val != 0.0))
            .map(|i| {
                let v = if i.aug != 0.0 { i.aug } else { i.val };
                let from = chummer_core::items::edit::find(&self.doc, &i.source_name).map(|e| e.get("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| i.source.clone());
                format!("{}{} {from}", if v > 0.0 { "+" } else { "" }, chummer_core::improvement::fmt_num(v))
            })
            .collect()
    }

    /// The attribute table: steppers for points and karma in creation,
    /// "+1 · cost" in career; a click selects the row for the inspector.
    fn ws_attribute_table(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let career_mode = self.doc.created;
        let priority = chummer_core::character::uses_priority_tables(&self.doc.field("buildmethod"));
        let attrs = self.shown_attributes();
        let marks: Vec<(String, bool)> = self.issues.iter().filter(|i| matches!(i.area, Area::Attributes | Area::SpecialAttributes) && i.severity != Severity::Info).map(|i| (message(lang, i), i.is_error())).collect();
        let note = if career_mode { lang.tr("career · every button shows its karma cost") } else { String::new() };
        widgets::heading(ui, &lang.tr("Attributes"), &note, 13.0, |ui| {
            for (msg, err) in &marks {
                widgets::issue_mark(ui, *err, msg);
            }
        });
        let mut select = None;
        let (captions, spec): (Vec<String>, Vec<f32>) = if career_mode {
            (lang.tr_all(["Attribute", "Rating", "Total", "Range", "Source", "Advance"]).into_iter().collect(), vec![150.0, 48.0, 48.0, 130.0, 0.0, 118.0])
        } else if priority {
            (lang.tr_all(["Attribute", "Points", "Karma", "Total", "Range", "Source", "Karma +1"]).into_iter().collect(), vec![150.0, 84.0, 84.0, 48.0, 120.0, 0.0, 64.0])
        } else {
            (lang.tr_all(["Attribute", "Karma", "Total", "Range", "Source", "Karma +1"]).into_iter().collect(), vec![150.0, 84.0, 48.0, 120.0, 0.0, 64.0])
        };
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &spec);
            let caps: Vec<&str> = captions.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            for name in attrs {
                let Some(v) = self.sheet.attr_values(name).cloned() else { continue };
                let Some((base, karma)) = self.doc.attribute(name).map(|a| (a.base, a.karma)) else { continue };
                let selected = self.ws_build.sel.as_ref() == Some(&Selected::Attribute(name.to_owned()));
                let long = attr_long(lang, name);
                let sources = self.attribute_sources(name).join(", ");
                let career_cost = if career_mode { career::attribute_upgrade_karma_cost(engine, &self.doc, name) } else { None };
                let karma_left = self.doc.karma;
                let mut w = widths.iter().copied();
                let mut next = || w.next().unwrap_or(40.0);
                let mut set = None;
                let mut raise = false;
                let row = widgets::table_row(ui, name, selected, 34.0, |ui| {
                    widgets::cell(ui, next(), 34.0, |ui| {
                        ui.label(RichText::new(&long).size(13.0).color(ws.text));
                        ui.label(RichText::new(name).size(11.0).color(ws.muted));
                    });
                    if career_mode {
                        widgets::cell(ui, next(), 34.0, |ui| ui.label(widgets::mono(v.value.to_string(), 13.0, ws.text)));
                    } else {
                        if priority {
                            widgets::cell(ui, next(), 34.0, |ui| {
                                let mut b = base;
                                let max = crate::view::attribute_base_max(&v, base, karma);
                                if widgets::stepper(ui, ("base", name), &mut b, 0, max, &lang.tr_fmt("Lower {0}", &[&long]), &lang.tr_fmt("Raise {0}", &[&long])) {
                                    set = Some(Command::SetAttributeBase { attribute: name.to_owned(), value: b });
                                }
                            });
                        }
                        widgets::cell(ui, next(), 34.0, |ui| {
                            let mut kv = karma;
                            let max = crate::view::attribute_karma_max(&v, karma);
                            if widgets::stepper(ui, ("karma", name), &mut kv, 0, max, &lang.tr_fmt("Lower {0}", &[&long]), &lang.tr_fmt("Raise {0}", &[&long])) {
                                set = Some(Command::SetAttributeKarma { attribute: name.to_owned(), value: kv });
                            }
                        });
                    }
                    widgets::cell(ui, next(), 34.0, |ui| {
                        let r = ui.label(widgets::mono(v.total.to_string(), 13.5, if v.total != v.value { ws.accent } else { ws.text }));
                        if v.total != v.value {
                            r.on_hover_text(lang.tr_fmt("{0} natural, {1} augmented", &[&v.value, &v.total]));
                        }
                    });
                    widgets::cell(ui, next(), 34.0, |ui| {
                        widgets::pips(ui, v.value, v.total_max);
                        ui.label(RichText::new(format!("{}–{}", v.total_min, v.total_max)).size(11.0).color(ws.muted)).on_hover_text(lang.tr_fmt("Augmented maximum {0}", &[&v.total_aug_max]));
                    });
                    widgets::cell(ui, next(), 34.0, |ui| {
                        ui.add(egui::Label::new(RichText::new(&sources).size(12.0).color(ws.muted)).truncate());
                    });
                    widgets::cell(ui, next(), 34.0, |ui| {
                        if career_mode {
                            match career_cost {
                                Some(c) => {
                                    let tip = lang.tr_fmt("Raise {0} to {1} for {2} karma", &[&long, &(v.value + 1), &c]);
                                    raise = widgets::cost_button(ui, &format!("+1 · {}", k(c)), karma_left >= c, &tip, &lang.tr("Not enough karma")).clicked();
                                }
                                None => {
                                    ui.label(RichText::new(lang.tr("at maximum")).size(12.0).color(ws.muted));
                                }
                            }
                        } else {
                            let next_cost = calc::attribute_upgrade_cost(&v, &self.rules).map_or_else(|| "—".to_owned(), k);
                            ui.label(RichText::new(next_cost).size(12.0).color(ws.muted)).on_hover_text(lang.tr_fmt("Karma cost so far: {0}", &[&calc::attribute_karma_cost(&v, &self.rules)]));
                        }
                    });
                });
                if let Some(cmd) = set {
                    changed |= self.doc.set(cmd);
                }
                if raise {
                    self.action = Some(CareerAction::RaiseAttribute(name.to_owned()));
                }
                if row.clicked() {
                    select = Some(Selected::Attribute(name.to_owned()));
                }
            }
            ui.add_space(4.0);
        });
        if let Some(s) = select {
            self.ws_select(s);
        }
        ui.label(RichText::new(format!("{} {}", lang.tr("Karma spent on attributes:"), self.sheet.attribute_karma_spent)).size(11.5).color(ws.muted));
        changed
    }

    /// The derived values under the attributes, recomputed as they change.
    fn ws_derived(&self, ui: &mut egui::Ui, lang: &Language) {
        let s = &self.sheet;
        widgets::heading(ui, &lang.tr("Derived"), &lang.tr("recomputed as you edit"), 13.0, |_| {});
        let row = |l: &str, v: String, accent: bool| (lang.tr(l), v, accent);
        let mut limits = vec![row("Physical", s.limit_physical.to_string(), false), row("Mental", s.limit_mental.to_string(), false), row("Social", s.limit_social.to_string(), false)];
        if self.doc.mag_enabled() {
            limits.push(row("Astral", s.limit_astral.to_string(), false));
        }
        let mut init = vec![row("Physical", format!("{} + {}d6", s.initiative, s.initiative_dice), true)];
        if self.doc.mag_enabled() {
            init.push(row("Astral", format!("{} + {}d6", s.astral_initiative, s.astral_initiative_dice), false));
        }
        init.push(row("Matrix cold-sim", format!("{} + {}d6", s.matrix_cold_initiative, s.matrix_cold_dice), false));
        init.push(row("Matrix hot-sim", format!("{} + {}d6", s.matrix_hot_initiative, s.matrix_hot_dice), false));
        let condition = vec![row("Physical CM", s.physical_cm.to_string(), false), row("Stun CM", s.stun_cm.to_string(), false), row("Overflow", s.cm_overflow.to_string(), false)];
        let pools = vec![row("Composure", s.composure.to_string(), false), row("Judge Intentions", s.judge_intentions.to_string(), false), row("Memory", s.memory.to_string(), false), row("Lift and Carry", s.lift_carry.to_string(), false)];
        let other = vec![
            row("Armor", s.armor.to_string(), false),
            row("Essence", format::essence(s.essence, self.rules.essence_decimals), false),
            row("CM Penalty:", s.wound_modifier.to_string(), s.wound_modifier != 0),
        ];
        let cards = [(lang.tr("Limits"), limits), (lang.tr("Initiative"), init), (lang.tr("Condition Monitor"), condition), (lang.tr("Pools"), pools), (lang.tr("Other"), other)];
        let gap = 8.0;
        let total = ui.available_width();
        // As many columns as fit at 170px or more.
        let per_row = ((total + gap) / (170.0 + gap)).floor().clamp(1.0, cards.len() as f32) as usize;
        let w = (total - gap * (per_row - 1) as f32) / per_row as f32;
        for chunk in cards.chunks(per_row) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (caption, rows) in chunk {
                    widgets::mini_card(ui, caption, rows, w);
                }
            });
        }
    }

    // ----- item tables -----

    /// A section's items as a Workspace table: groups as captions,
    /// nested items indented, issue marks, the source as a link; a click
    /// opens the item in the inspector, the bin removes a top-level item
    /// (after the same question as Classic).
    pub(crate) fn ws_item_table(&mut self, ui: &mut egui::Ui, sec: &Sec, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let marks: HashMap<String, (String, bool)> = self.item_marks(lang);
        let tree = chummer_core::tree::section_tree(&self.doc.doc, sec);
        if tree.is_empty() {
            widgets::card_frame(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
            });
            return false;
        }
        // Name, up to three more columns, the source and the bin.
        let extra: Vec<usize> = sec.columns.iter().enumerate().skip(1).filter(|(_, c)| !matches!(c.field, "source" | "page")).map(|(i, _)| i).take(3).collect();
        let mut spec = vec![0.0];
        spec.extend(extra.iter().map(|_| 92.0));
        spec.extend([104.0, 24.0]);
        let selected = self.item_editor.as_ref().map(|(g, _)| g.clone());
        let mut open = None;
        let mut remove = None;
        let mut book = None;
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, &spec);
            let mut caps = vec![lang.tr("Name")];
            caps.extend(extra.iter().map(|i| lang.tr(sec.columns[*i].header)));
            caps.extend([lang.tr("Source"), String::new()]);
            let caps: Vec<&str> = caps.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            let mut stack: Vec<(&chummer_core::tree::ItemNode, usize)> = tree.iter().rev().map(|n| (n, 0)).collect();
            while let Some((n, depth)) = stack.pop() {
                let view = crate::view::tree_row(sec, n, lang, &marks);
                match &n.value {
                    Entry::Group(_) => {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0 + depth as f32 * 14.0);
                            widgets::cell(ui, widths[0], 24.0, |ui| ui.label(widgets::overline(&view.cells[0], &ws)));
                        });
                    }
                    Entry::Item { el, top } => {
                        let guid = el.get("guid");
                        let is_sel = selected.as_deref() == Some(guid.as_str());
                        let src = SourceRef::of(el);
                        let mut w = widths.iter().copied();
                        let row = widgets::table_row(ui, &n.key, is_sel, 30.0, |ui| {
                            widgets::cell(ui, w.next().unwrap_or(80.0), 30.0, |ui| {
                                ui.add_space(depth as f32 * 14.0);
                                if let Some((msg, err)) = &view.warning {
                                    widgets::issue_mark(ui, *err, msg);
                                }
                                let r = ui.add(egui::Label::new(RichText::new(&view.cells[0]).size(12.5).color(ws.text)).truncate());
                                if !view.hover.is_empty() {
                                    r.on_hover_text(&view.hover);
                                }
                            });
                            for i in &extra {
                                let text = view.cells.get(*i).cloned().unwrap_or_default();
                                widgets::cell(ui, w.next().unwrap_or(80.0), 30.0, |ui| ui.add(egui::Label::new(RichText::new(text).size(12.0).color(ws.muted)).truncate()));
                            }
                            widgets::cell(ui, w.next().unwrap_or(80.0), 30.0, |ui| {
                                if let Some(r) = &src {
                                    if source_button(ui, pdfs, lang, r) {
                                        book = Some(r.clone());
                                    }
                                }
                            });
                            widgets::cell(ui, w.next().unwrap_or(24.0), 30.0, |ui| {
                                if *top && widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove (also removes its improvements)")).clicked() {
                                    remove = Some((sec.container.to_owned(), guid.clone(), view.cells[0].clone()));
                                }
                            });
                        });
                        if row.clicked() && view.clickable {
                            open = Some(guid);
                        }
                    }
                }
                for c in n.children.iter().rev() {
                    stack.push((c, depth + 1));
                }
            }
            ui.add_space(4.0);
        });
        if let Some(r) = book {
            crate::pdf_ui::open(pdfs, &r, status);
        }
        if remove.is_some() {
            self.confirm_remove = remove;
        }
        if let Some(g) = open {
            self.ws_open_item(g);
        }
        false
    }

    /// "Add …" buttons for a section's kinds, in the Workspace style.
    pub(crate) fn ws_add_buttons(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, container: &str) {
        for t in crate::view::add_tags(container) {
            let label = chummer_core::items::kind(t).map_or(*t, |k| k.label);
            if widgets::button(ui, Some(icons::PLUS), &lang.tr_fmt("Add {0}…", &[&crate::view::kind_noun(lang, label)]), Look::Secondary, 24.0).clicked() {
                self.open_select(t, engine);
            }
        }
    }
}

/// A sourcebook reference as a small ghost button ("SR5 p. 65"), dimmed
/// when no PDF is linked. True when clicked.
pub(crate) fn source_button(ui: &mut egui::Ui, pdfs: &SourcebookLibrary, lang: &Language, r: &SourceRef) -> bool {
    let linked = pdfs.is_linked(&r.book);
    let hover = if linked { lang.tr("Open the sourcebook at this page") } else { lang.tr("No PDF linked for this book — Tools → Sourcebooks") };
    widgets::button(ui, Some(icons::BOOK_OPEN), &r.to_string(), Look::Ghost, 22.0).on_hover_text(hover).clicked()
}
