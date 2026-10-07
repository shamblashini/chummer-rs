//! Guided creation without a guide bar: the tabs (Classic) or the
//! sidebar (Workspace) are the checklist, ticked off per step
//! (`chummer_core::chargen::guide`), and one slim hint line at the top
//! of the page says what is left on it and offers the next step:
//!
//! `Attributes ⓘ · ⚠ 4 Attribute points left to spend  +1   Next: Special Attributes →  ✕`
//!
//! The rule explanation and its 📖 page are on demand behind ⓘ; the
//! review step lists everything left and has Finish creation. With the
//! guide off the same line shows the page's issues (✕ hides them until
//! they change), so there is one strip, not two.
//!
//! A child module of `view`. Where the guide is (the current step and
//! the steps visited) is remembered per file in
//! `$XDG_CONFIG_HOME/chummer-rs/guide.ini`, never in the .chum5.

use std::path::{Path, PathBuf};

use chummer_core::chargen::guide::{self, Step, StepStatus};
use chummer_core::chargen::issues::{Issue, Severity};
use chummer_core::lang::Language;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use eframe::egui::{self, Color32, RichText};

use super::issues_ui::{gear_sub_tab, issue_tab, message, tab_of};
use super::{CharacterView, Tab};
use crate::pdf_ui::Status;
use crate::theme::{self, Layout};
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// The guide of one character view.
pub(super) struct Guide {
    pub(super) steps: Vec<Step>,
    /// The step the hint is about.
    pub(super) current: usize,
    /// Steps the player has been on; a visited step without problems is
    /// ticked off.
    pub(super) visited: Vec<Step>,
    /// Workspace: the "Review & Finish" page is open.
    pub(super) reviewing: bool,
}

/// `guide.ini`: one `step;visited,visited<TAB>path` line per character
/// file (older lines have only the step).
fn steps_path() -> Option<PathBuf> {
    crate::theme::config_path().map(|p| p.with_file_name("guide.ini"))
}

fn parse_state(s: &str) -> Option<(Step, Vec<Step>)> {
    let (cur, visited) = s.split_once(';').unwrap_or((s, ""));
    Some((Step::parse(cur)?, visited.split(',').filter_map(Step::parse).collect()))
}

fn load_state(file: &Path) -> Option<(Step, Vec<Step>)> {
    let text = std::fs::read_to_string(steps_path()?).ok()?;
    let key = file.display().to_string();
    text.lines().filter_map(|l| l.split_once('\t')).find(|(_, p)| *p == key).and_then(|(s, _)| parse_state(s))
}

fn save_state(file: &Path, step: Step, visited: &[Step]) {
    let Some(path) = steps_path() else { return };
    let key = file.display().to_string();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().filter(|l| l.split_once('\t').is_none_or(|(_, p)| p != key)).map(str::to_owned).collect();
    let visited: Vec<&str> = visited.iter().map(|s| s.id()).collect();
    lines.push(format!("{};{}\t{key}", step.id(), visited.join(",")));
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

/// Paint a checklist mark centred in `rect`: a tick when the step is
/// done, an empty ring when it still waits for a visit. Used by the
/// Classic tabs and the Workspace sidebar.
pub fn paint_step_mark(painter: &egui::Painter, rect: egui::Rect, done: bool) {
    let t = theme::current(painter.ctx());
    let ws = t.kind.layout() == Layout::Workspace;
    let c = rect.center();
    if done {
        let s = egui::Stroke::new(1.8_f32, if ws { t.ws.accent } else { t.palette.good });
        painter.line_segment([egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x - 1.0, c.y + 3.0)], s);
        painter.line_segment([egui::pos2(c.x - 1.0, c.y + 3.0), egui::pos2(c.x + 4.5, c.y - 3.5)], s);
    } else {
        let ink = if ws { t.ws.muted } else { t.palette.weak };
        painter.circle_stroke(c, 4.0, egui::Stroke::new(1.2_f32, ink));
    }
}

/// Colours and controls of the hint line in the current theme.
struct Ink {
    workspace: bool,
    fill: Color32,
    stroke: Color32,
    text: Color32,
    muted: Color32,
    error: Color32,
}

impl Ink {
    fn of(ui: &egui::Ui) -> Ink {
        let t = theme::current(ui.ctx());
        if t.kind.layout() == Layout::Workspace {
            Ink { workspace: true, fill: t.ws.raised, stroke: t.ws.divider, text: t.ws.text, muted: t.ws.muted, error: t.ws.error }
        } else {
            Ink { workspace: false, fill: t.palette.window, stroke: t.palette.stroke, text: t.palette.text, muted: t.palette.weak, error: t.palette.bad }
        }
    }

    /// Next (outline) or Finish (primary), with an icon.
    fn button(&self, ui: &mut egui::Ui, glyph: &str, text: &str, primary: bool) -> egui::Response {
        if self.workspace {
            return widgets::button(ui, Some(glyph), text, if primary { Look::Primary } else { Look::Outline }, 24.0);
        }
        if primary {
            ui.add(theme::primary_button(ui, format!("{glyph} {text}")))
        } else {
            ui.button(format!("{text} {glyph}"))
        }
    }

    /// A small icon button; add a tooltip.
    fn icon(&self, ui: &mut egui::Ui, glyph: &str) -> egui::Response {
        if self.workspace {
            widgets::icon_button(ui, glyph, 22.0)
        } else {
            ui.add(egui::Button::new(RichText::new(glyph).color(self.muted)).frame(false))
        }
    }
}

/// One issue as a frameless, focusable button with its mark; true when
/// clicked (go to it).
fn issue_button(ui: &mut egui::Ui, lang: &Language, ink: &Ink, i: &Issue, truncate: bool) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if i.severity == Severity::Info {
            ui.label(RichText::new(icons::INFO).color(ink.muted));
        } else {
            theme::warning_mark(ui, i.is_error());
        }
        let color = match i.severity {
            Severity::Error => ink.error,
            Severity::Warning => ink.text,
            Severity::Info => ink.muted,
        };
        let b = egui::Button::new(RichText::new(message(lang, i)).color(color)).frame(false);
        let b = if truncate { b.truncate() } else { b };
        ui.add(b).on_hover_text(lang.tr("Show")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    })
    .inner
}

/// The step part of a hint line: its status, where Next goes, and the
/// progress (done, total).
#[derive(Clone, Copy)]
struct HintStep {
    status: StepStatus,
    next: Option<(usize, Step)>,
    progress: (usize, usize),
}

/// What the hint line asked for.
#[derive(Default)]
struct Clicked {
    jump: Option<Issue>,
    go: Option<usize>,
    finish: bool,
    hide: bool,
    dismiss: bool,
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
        let remembered = self.doc.file.as_deref().and_then(load_state).and_then(|(s, v)| steps.iter().position(|x| *x == s).map(|i| (i, v)));
        let (current, visited) = remembered.unwrap_or_else(|| {
            // New: the wizard did the concept; for an older file, the
            // steps before the first one with something to do.
            let start = guide::suggested(&steps, &self.issues);
            (start, guide::visited_before(&steps, start))
        });
        self.guide = Some(Guide { steps, current, visited, reviewing: false });
        self.go_to_step(current);
    }

    /// Whether guided creation shows (creation mode, guide on).
    pub fn guided(&self) -> bool {
        self.guide_shown()
    }

    pub(super) fn build_method(&self) -> String {
        match self.doc.field("buildmethod") {
            b if b.is_empty() => "Priority".into(),
            b => b,
        }
    }

    /// Steps follow the character (a new Adept quality adds the powers
    /// step); keep the current step where it was.
    pub(super) fn refresh_steps(&mut self) {
        let method = self.build_method();
        let Some(g) = self.guide.as_mut() else { return };
        let steps = guide::steps_for(&method, &self.doc);
        if steps != g.steps {
            let cur = g.steps.get(g.current).copied();
            g.current = cur.and_then(|c| steps.iter().position(|s| *s == c)).unwrap_or(0).min(steps.len() - 1);
            g.steps = steps;
        }
    }

    /// The steps' status, while the guide shows.
    pub(super) fn guide_statuses(&self) -> Option<Vec<StepStatus>> {
        let g = self.guide.as_ref().filter(|_| self.budget.is_some())?;
        Some(guide::status(&g.steps, &g.visited, &self.issues))
    }

    /// A tab's checklist mark (see [`paint_step_mark`]): `None` without
    /// the guide or when the tab has no step.
    pub(super) fn tab_check(&self, tab: Tab) -> Option<bool> {
        guide::tab_done(&self.guide_statuses()?, issue_tab(tab)?)
    }

    /// Steps done and in all (without the review), while the guide shows.
    pub(super) fn guide_progress(&self) -> Option<(usize, usize)> {
        self.guide_statuses().map(|s| guide::progress(&s))
    }

    /// Whether nothing is left for the review (no errors or warnings).
    pub(super) fn review_clear(&self) -> bool {
        !self.issues.iter().any(|i| i.severity != Severity::Info)
    }

    /// Make step `index` the current one and remember it as visited.
    fn visit(&mut self, index: usize) {
        let Some(g) = self.guide.as_mut() else { return };
        g.current = index.min(g.steps.len().saturating_sub(1));
        let step = g.steps[g.current];
        if !g.visited.contains(&step) {
            g.visited.push(step);
        }
        if let Some(f) = self.doc.file.as_deref() {
            save_state(f, step, &g.visited);
        }
    }

    /// Going to an issue makes its step the guide's (a knowledge issue
    /// on the Skills page is the knowledge step's).
    pub(super) fn guide_follow(&mut self, issue: &Issue) {
        let Some(g) = self.guide.as_ref() else { return };
        if let Some(i) = g.steps.iter().position(|s| *s != Step::Review && s.owns(issue)) {
            self.visit(i);
        }
    }

    /// Go to a step: its tab (the review is its own Workspace page).
    pub(super) fn go_to_step(&mut self, index: usize) {
        self.visit(index);
        let Some(g) = self.guide.as_mut() else { return };
        let step = g.steps[g.current];
        g.reviewing = step == Step::Review;
        self.tab = tab_of(step.tab());
        if step == Step::Gear {
            self.gear_tab = 0;
        }
        if step == Step::KnowledgeSkills || step == Step::ActiveSkills {
            self.skill_filter.clear();
        }
    }

    /// Whether the guide shows: creation mode with the guide on.
    pub(super) fn guide_shown(&self) -> bool {
        self.budget.is_some() && self.guide.is_some()
    }

    /// Workspace: whether the "Review & Finish" page is open.
    pub(super) fn reviewing(&self) -> bool {
        self.guide_shown() && self.guide.as_ref().is_some_and(|g| g.reviewing)
    }

    pub(super) fn set_reviewing(&mut self, on: bool) {
        if let Some(g) = self.guide.as_mut() {
            g.reviewing = on;
        }
    }

    /// Follow the page: the guide's step becomes the one this page is
    /// about, so the sidebar, the tabs and issue links all move freely.
    /// `review` is the Workspace's review page; `review_here` lets the
    /// review stay current on its tab (Classic shows it on Common).
    fn guide_sync(&mut self, tab: Tab, review: bool, review_here: bool) {
        let Some(st) = self.guide_statuses() else { return };
        let Some(g) = self.guide.as_ref() else { return };
        let cur = st[g.current].step;
        let page = issue_tab(tab);
        let target = if review {
            Some(st.len() - 1)
        } else if page == Some(cur.tab()) && (cur != Step::Review || review_here) {
            Some(g.current)
        } else {
            page.and_then(|t| guide::focus(&st, t))
        };
        if let Some(i) = target {
            if i != g.current || !g.visited.contains(&g.steps[i]) {
                self.visit(i);
            }
        }
    }

    /// The hint line at the top of a page (both layouts). Classic passes
    /// its tab; the Workspace its section's tab, the Street Gear sub-tab
    /// and whether it is the review page.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn page_hint(&mut self, ui: &mut egui::Ui, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, tab: Tab, gear: Option<usize>, review: bool) {
        if self.budget.is_none() {
            return;
        }
        let classic = theme::current(ui.ctx()).kind.layout() != Layout::Workspace;
        if self.guide_shown() {
            self.refresh_steps();
            self.guide_sync(tab, review, classic);
        }
        // Street Gear sub-tabs show their own part of the gear step.
        let on_sub = |i: &&Issue| gear.is_none_or(|g| gear_sub_tab(i.area).is_none_or(|s| s == g));
        let step = self.guide_statuses().and_then(|st| {
            let g = self.guide.as_ref()?;
            let s = st[g.current];
            let here = review || (issue_tab(tab) == Some(s.step.tab()) && (s.step != Step::Review || classic));
            here.then(|| HintStep { status: s, next: guide::next(&st, g.current).map(|n| (n, st[n].step)), progress: guide::progress(&st) })
        });
        let mut c = Clicked::default();
        match step {
            Some(h) => {
                let todo: Vec<Issue> = h.status.step.todo(&self.issues).into_iter().filter(on_sub).cloned().collect();
                self.hint_line(ui, lang, pdfs, status, Some(h), &todo, &mut c);
                if h.status.step == Step::Review && !todo.is_empty() {
                    // Everything Finish creation would list, grouped by tab.
                    let ink = Ink::of(ui);
                    egui::Frame::new()
                        .fill(ink.fill)
                        .stroke(egui::Stroke::new(1.0_f32, ink.stroke))
                        .corner_radius(if ink.workspace { 6 } else { 0 })
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                            egui::ScrollArea::vertical().id_salt("guide_review").max_height(if classic { 180.0 } else { f32::INFINITY }).show(ui, |ui| self.issue_list(ui, lang));
                        });
                    ui.add_space(4.0);
                }
            }
            None => {
                // No step here (or no guide): the page's issues.
                let mine: Vec<Issue> = self.issues.iter().filter(|i| i.tab().map(tab_of) == Some(tab) && i.severity != Severity::Info).filter(on_sub).cloned().collect();
                if mine.is_empty() || self.dismissed.is(tab, &mine) {
                    return;
                }
                self.hint_line(ui, lang, pdfs, status, None, &mine, &mut c);
                if c.dismiss {
                    self.dismissed.dismiss(tab, &mine);
                }
            }
        }
        if let Some(i) = c.go {
            self.go_to_step(i);
        }
        if let Some(i) = c.jump {
            self.set_reviewing(false);
            self.jump_to(&i);
        }
        if c.finish {
            self.confirm_finish = true;
        }
        if c.hide {
            self.guide = None;
            self.guide_hidden = true;
        }
    }

    /// The line itself: the step (ⓘ opens its rule), the first thing left
    /// and how many more, Next or Finish creation, and ✕.
    #[allow(clippy::too_many_arguments)]
    fn hint_line(&self, ui: &mut egui::Ui, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, step: Option<HintStep>, todo: &[Issue], c: &mut Clicked) {
        let ink = Ink::of(ui);
        let method = self.build_method();
        let review = step.is_some_and(|h| h.status.step == Step::Review);
        egui::Frame::new()
            .fill(ink.fill)
            .stroke(egui::Stroke::new(1.0_f32, ink.stroke))
            .corner_radius(if ink.workspace { 6 } else { 0 })
            .inner_margin(egui::Margin { left: 10, right: 4, top: 3, bottom: 3 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        // Right: ✕, then Next or Finish creation.
                        if step.is_some() {
                            c.hide = ink.icon(ui, icons::X).on_hover_text(format!("{} · {}", lang.tr("Hide guide"), lang.tr("View → Guided creation turns it back on"))).clicked();
                        } else {
                            c.dismiss = ink.icon(ui, icons::X).on_hover_text(lang.tr("Hide until something changes")).clicked();
                        }
                        if review {
                            let ok = !self.issues.iter().any(Issue::is_error);
                            let r = ui.add_enabled_ui(ok, |ui| ink.button(ui, icons::CHECK, &lang.tr("Finish creation"), true)).inner;
                            c.finish = r.on_disabled_hover_text(lang.tr("Fix the problems above first")).clicked();
                        } else if let Some((n, ns)) = step.and_then(|h| h.next) {
                            let label = format!("{}: {}", lang.tr("Next"), lang.tr(ns.title()));
                            if ink.button(ui, icons::ARROW_RIGHT, &label, false).on_hover_text(lang.tr("The next step with something left to do")).clicked() {
                                c.go = Some(n);
                            }
                        }
                        // Left, in the rest: the step and what is left.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if let Some(h) = step {
                                let name = lang.tr(h.status.step.title());
                                let title = if ink.workspace { RichText::new(name).font(widgets::bold(12.5)).color(ink.text) } else { theme::strong(ui, name) };
                                ui.label(title).on_hover_text(lang.tr_fmt("{0} of {1} steps done", &[&h.progress.0, &h.progress.1]));
                                let r = ink.icon(ui, icons::INFO).on_hover_text(lang.tr("How this step works"));
                                egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                                    ui.set_max_width(380.0);
                                    ui.add(egui::Label::new(lang.tr(h.status.step.explanation(&method))).wrap());
                                    let (book, page) = h.status.step.source(&method);
                                    crate::pdf_ui::source_link(ui, pdfs, lang, SourceRef::new(book, page), status);
                                });
                                ui.label(RichText::new("·").color(ink.muted));
                            }
                            if review {
                                // The list below has them all.
                                let errors = todo.iter().filter(|i| i.is_error()).count();
                                let text = if todo.is_empty() { lang.tr(Step::Review.prompt(&method)) } else { lang.tr_fmt("{0} error(s), {1} warning(s)", &[&errors, &(todo.len() - errors)]) };
                                ui.add(egui::Label::new(RichText::new(text).color(if errors > 0 { ink.error } else { ink.muted })).truncate());
                                return;
                            }
                            let Some(first) = todo.first() else {
                                if let Some(h) = step {
                                    ui.add(egui::Label::new(RichText::new(lang.tr(h.status.step.prompt(&method))).color(ink.muted)).truncate());
                                }
                                return;
                            };
                            // The issue takes what room is left beside its "+n".
                            let room = ui.available_width() - if todo.len() > 1 { 44.0 } else { 0.0 };
                            if ui.allocate_ui_with_layout(egui::vec2(room.max(40.0), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| issue_button(ui, lang, &ink, first, true)).inner {
                                c.jump = Some(first.clone());
                            }
                            if todo.len() > 1 {
                                let r = ui.add(egui::Button::new(RichText::new(format!("+{}", todo.len() - 1)).color(ink.muted)).small()).on_hover_text(lang.tr("Everything left in this step"));
                                egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                                    for i in &todo[1..] {
                                        if issue_button(ui, lang, &ink, i, false) {
                                            c.jump = Some(i.clone());
                                        }
                                    }
                                });
                            }
                        });
                    });
                });
            });
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_ini_lines_old_and_new() {
        assert_eq!(parse_state("skills"), Some((Step::ActiveSkills, vec![])));
        assert_eq!(parse_state("review;concept,attributes,bogus"), Some((Step::Review, vec![Step::Concept, Step::Attributes])));
        assert_eq!(parse_state("nope;concept"), None);
    }
}
