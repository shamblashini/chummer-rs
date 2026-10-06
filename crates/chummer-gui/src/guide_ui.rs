//! Guided creation: a bar over the character's tabs that walks through
//! the build one step at a time (see `chummer_core::chargen::guide`).
//!
//! A child module of `view`. The current step is remembered per file in
//! `$XDG_CONFIG_HOME/chummer-rs/guide.ini`, never in the .chum5.

use std::path::{Path, PathBuf};

use chummer_core::chargen::guide::{self, State, Step};
use chummer_core::chargen::issues::{Area, Issue, Severity};
use chummer_core::lang::Language;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use eframe::egui::{self, RichText};

use super::issues_ui::{message, tab_of};
use super::{CharacterView, Tab};
use crate::pdf_ui::Status;

/// The guide of one character view.
pub(super) struct Guide {
    steps: Vec<Step>,
    current: usize,
    /// The furthest step reached; steps before it count as done.
    reached: usize,
}

/// `guide.ini`: one `step<TAB>path` line per character file.
fn steps_path() -> Option<PathBuf> {
    crate::theme::config_path().map(|p| p.with_file_name("guide.ini"))
}

fn load_step(file: &Path) -> Option<Step> {
    let text = std::fs::read_to_string(steps_path()?).ok()?;
    let key = file.display().to_string();
    text.lines().filter_map(|l| l.split_once('\t')).find(|(_, p)| *p == key).and_then(|(s, _)| Step::parse(s))
}

fn save_step(file: &Path, step: Step) {
    let Some(path) = steps_path() else { return };
    let key = file.display().to_string();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().filter(|l| l.split_once('\t').is_none_or(|(_, p)| p != key)).map(str::to_owned).collect();
    lines.push(format!("{}\t{key}", step.id()));
    // Keep the file small: the most recent 200 characters.
    let skip = lines.len().saturating_sub(200);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, lines[skip..].join("\n") + "\n");
}

/// The "Guided creation" preference (`guided=` in gui.ini); off unless
/// turned on.
pub fn guided_preference() -> bool {
    crate::theme::load_value("guided").is_some_and(|v| v == "true")
}

/// Whether the New Character wizard offers the guide checked: unless it
/// was turned off.
pub fn guided_offer() -> bool {
    crate::theme::load_value("guided").is_none_or(|v| v != "false")
}

pub fn save_guided_preference(on: bool) {
    let _ = crate::theme::save_value("guided", if on { "true" } else { "false" });
}

impl CharacterView {
    /// Turn the guide on or off. It only shows in creation mode.
    pub fn set_guided(&mut self, on: bool) {
        if !on {
            self.guide = None;
            return;
        }
        if self.guide.is_some() || self.doc.created {
            return;
        }
        let steps = guide::steps_for(&self.build_method(), &self.doc);
        let remembered = self.doc.file.as_deref().and_then(load_step).and_then(|s| steps.iter().position(|x| *x == s));
        let current = remembered.unwrap_or_else(|| guide::suggested(&steps, &self.issues));
        self.guide = Some(Guide { steps, current, reached: current });
        self.go_to_step(current);
    }

    fn build_method(&self) -> String {
        match self.doc.field("buildmethod") {
            b if b.is_empty() => "Priority".into(),
            b => b,
        }
    }

    /// The tab of the current step, while the guide shows.
    pub(super) fn guide_tab(&self) -> Option<Tab> {
        let g = self.guide.as_ref().filter(|_| self.budget.is_some())?;
        g.steps.get(g.current).map(|s| tab_of(s.tab()))
    }

    /// Steps follow the character (a new Adept quality adds the powers
    /// step); keep the current step where it was.
    fn refresh_steps(&mut self) {
        let method = self.build_method();
        let Some(g) = self.guide.as_mut() else { return };
        let steps = guide::steps_for(&method, &self.doc);
        if steps != g.steps {
            let cur = g.steps.get(g.current).copied();
            let reached = g.steps.get(g.reached).copied();
            g.current = cur.and_then(|c| steps.iter().position(|s| *s == c)).unwrap_or(0).min(steps.len() - 1);
            g.reached = reached.and_then(|c| steps.iter().position(|s| *s == c)).unwrap_or(g.current);
            g.steps = steps;
        }
    }

    fn go_to_step(&mut self, index: usize) {
        let Some(g) = self.guide.as_mut() else { return };
        g.current = index.min(g.steps.len().saturating_sub(1));
        g.reached = g.reached.max(g.current);
        let step = g.steps[g.current];
        self.tab = tab_of(step.tab());
        if step == Step::Gear {
            self.gear_tab = 0;
        }
        if step == Step::KnowledgeSkills || step == Step::ActiveSkills {
            self.skill_filter.clear();
        }
        if let Some(f) = self.doc.file.as_deref() {
            save_step(f, step);
        }
    }

    /// The guide bar, above the tabs. Creation mode only.
    pub(super) fn guide_bar(&mut self, ctx: &egui::Context, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) {
        if self.budget.is_none() || self.guide.is_none() {
            return;
        }
        self.refresh_steps();
        let method = self.build_method();
        let Some(g) = self.guide.as_ref() else { return };
        let statuses = guide::status(&g.steps, g.current, g.reached, &self.issues);
        let cur = statuses[g.current];
        let step = cur.step;
        let last = g.current + 1 == g.steps.len();
        let mine: Vec<Issue> = self.issues.iter().filter(|i| step.owns(i)).cloned().collect();
        let any_error = self.issues.iter().any(Issue::is_error);
        let mut go: Option<usize> = None;
        let mut jump: Option<Issue> = None;
        let mut finish = false;
        let mut hide = false;
        egui::TopBottomPanel::top("guide_bar").show(ctx, |ui| {
            let p = crate::theme::palette(ui);
            ui.add_space(4.0);
            // The steps, as a row of numbered chips.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 4.0);
                ui.label(crate::theme::strong(ui, lang.tr("Guided creation")));
                ui.add_space(6.0);
                for (i, s) in statuses.iter().enumerate() {
                    let mark = match s.state {
                        State::Done if s.errors == 0 => "✔ ",
                        _ => "",
                    };
                    let text = format!("{mark}{}. {}", i + 1, lang.tr(s.step.title()));
                    let rich = match s.state {
                        State::Current => crate::theme::strong(ui, text),
                        State::Done => RichText::new(text).small().color(p.good),
                        State::Upcoming => RichText::new(text).small().color(p.weak),
                    };
                    let selected = s.state == State::Current;
                    let r = ui.add(egui::Button::new(rich).selected(selected).frame(selected));
                    if s.errors + s.warnings > 0 {
                        crate::theme::warning_mark(ui, s.errors > 0).on_hover_text(lang.tr_fmt("{0} error(s), {1} warning(s)", &[&s.errors, &s.warnings]));
                    }
                    if r.clicked() && i != g.current {
                        go = Some(i);
                    }
                    if i + 1 < statuses.len() {
                        ui.label(RichText::new("›").color(p.weak));
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading(lang.tr(step.title()));
                let (book, page) = step.source(&method);
                crate::pdf_ui::source_link(ui, pdfs, lang, SourceRef::new(book, page), status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    hide = ui.button(lang.tr("Hide guide")).on_hover_text(lang.tr("View → Guided creation turns it back on")).clicked();
                    ui.add_space(8.0);
                    if last {
                        let r = ui.add_enabled(!any_error, crate::theme::primary_button(ui, lang.tr("Finish creation")));
                        if r.on_disabled_hover_text(lang.tr("Fix the problems above first")).clicked() {
                            finish = true;
                        }
                    } else {
                        let r = ui.add_enabled(cur.can_advance(), crate::theme::primary_button(ui, format!("{} ▶", lang.tr("Next"))));
                        if r.on_disabled_hover_text(lang.tr("Fix the errors in this step first")).clicked() {
                            go = Some(g.current + 1);
                        }
                    }
                    if ui.add_enabled(g.current > 0, egui::Button::new(format!("◀ {}", lang.tr("Back")))).clicked() {
                        go = Some(g.current - 1);
                    }
                });
            });
            ui.add(egui::Label::new(lang.tr(step.explanation(&method))).wrap());
            ui.add_space(2.0);
            let todo: Vec<&Issue> = mine.iter().filter(|i| step == Step::Review || i.severity != Severity::Info || i.area == Area::CharacterInfo).collect();
            if todo.is_empty() {
                ui.colored_label(p.good, lang.tr("Nothing left to do in this step."));
            } else {
                // A few lines at most; the Karma Summary has the full list.
                const SHOWN: usize = 4;
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::theme::strong(ui, lang.tr("Left to do:")));
                    for i in todo.iter().take(SHOWN) {
                        if i.severity == Severity::Info {
                            ui.label(RichText::new("ℹ").weak());
                        } else {
                            crate::theme::warning_mark(ui, i.is_error());
                        }
                        let t = RichText::new(message(lang, i));
                        let t = if i.is_error() { t.color(ui.visuals().error_fg_color) } else { t };
                        if ui.add(egui::Label::new(t).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            jump = Some((*i).clone());
                        }
                        ui.add_space(10.0);
                    }
                    if todo.len() > SHOWN {
                        ui.weak(lang.tr_fmt("and {0} more (Karma Summary)", &[&(todo.len() - SHOWN)]));
                    }
                });
            }
            ui.add_space(4.0);
        });
        if let Some(i) = go {
            self.go_to_step(i);
        }
        if let Some(i) = jump {
            self.jump_to(&i);
        }
        if finish {
            self.confirm_finish = true;
        }
        if hide {
            self.guide = None;
            self.guide_hidden = true;
        }
    }
}
