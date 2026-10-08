//! The first-start setup: a short dialog on the very first start
//! (no gui.ini yet), skippable at every step and re-opened from Help →
//! First-start setup… or Tools → Preferences. Steps: language, layout,
//! theme, sourcebooks, online name, updates, and a finish page with
//! shortcuts. Choices apply at once, so the dialog itself turns into the
//! picked layout's style.

use eframe::egui::{self, RichText};

use crate::theme::{self, Appearance, Layout, ThemeKind};
use crate::workspace::widgets::{self, Look};
use crate::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Language,
    Layout,
    Theme,
    Sourcebooks,
    Online,
    Updates,
    Finish,
}

impl Step {
    pub const ALL: [Step; 7] = [Step::Language, Step::Layout, Step::Theme, Step::Sourcebooks, Step::Online, Step::Updates, Step::Finish];

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The step's title (English; goes through `lang.tr`).
    pub fn title(self) -> &'static str {
        match self {
            Step::Language => "Language",
            Step::Layout => "Layout",
            Step::Theme => "Theme",
            Step::Sourcebooks => "Sourcebooks",
            Step::Online => "Online campaigns",
            Step::Updates => "Updates",
            Step::Finish => "All set",
        }
    }
}

/// What the finish page's shortcuts ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    NewCharacter,
    OpenFile,
    JoinCampaign,
}

/// How a frame of the setup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Still open.
    Open,
    /// Finished (with a shortcut picked) or skipped.
    Done(Option<Shortcut>),
}

/// The setup's state.
pub struct Setup {
    pub step: Step,
    /// The online name typed so far.
    pub name: String,
    pub check_updates: bool,
    thumbs: Option<[egui::TextureHandle; 2]>,
}

impl Setup {
    pub fn new(name: String, check_updates: bool) -> Setup {
        Setup { step: Step::Language, name, check_updates, thumbs: None }
    }

    /// The next step; false on the last one.
    pub fn next(&mut self) -> bool {
        match Step::ALL.get(self.step.index() + 1) {
            Some(s) => {
                self.step = *s;
                true
            }
            None => false,
        }
    }

    pub fn back(&mut self) {
        if let Some(i) = self.step.index().checked_sub(1) {
            self.step = Step::ALL[i];
        }
    }

    pub fn is_last(&self) -> bool {
        self.step == Step::Finish
    }

    fn thumbs(&mut self, ctx: &egui::Context) -> &[egui::TextureHandle; 2] {
        self.thumbs.get_or_insert_with(|| {
            let load = |name: &str, bytes: &[u8]| {
                let img = image::load_from_memory(bytes).map(|i| i.into_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
                let size = [img.width() as usize, img.height() as usize];
                ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
            };
            [load("setup-workspace", include_bytes!("../assets/setup/layout-workspace.png")), load("setup-classic", include_bytes!("../assets/setup/layout-classic.png"))]
        })
    }
}

/// Whether the setup opens at start: never with `--theme`/`--layout`,
/// else on the very first start (no gui.ini and nothing saved by an
/// earlier run), or when gui.ini says `setup_done=false`.
pub fn should_show(gui_ini: Option<&str>, ran_before: bool, look_from_command_line: bool) -> bool {
    if look_from_command_line {
        return false;
    }
    match gui_ini {
        None => !ran_before,
        Some(t) => theme::config_get(t, crate::prefs::SETUP_DONE).is_some_and(|v| v == "false"),
    }
}

/// The appearance after picking `layout` in the setup: its last theme.
pub fn with_layout(a: Appearance, layout: Layout) -> Appearance {
    a.with_layout(layout)
}

/// A primary, secondary or ghost button in the current layout's style.
fn button(ui: &mut egui::Ui, text: &str, look: Look) -> egui::Response {
    if theme::current(ui.ctx()).kind.layout() == Layout::Workspace {
        widgets::button(ui, None, text, look, 28.0)
    } else if look == Look::Primary {
        ui.add(theme::primary_button(ui, text))
    } else {
        ui.button(text)
    }
}

/// A choice card: Workspace-style card or a Classic group box, with a
/// highlighted border when `selected`.
fn choice(ui: &mut egui::Ui, id: &str, selected: bool, width: f32, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let t = theme::current(ui.ctx());
    let r = if t.kind.layout() == Layout::Workspace {
        widgets::click_card(ui, id, width, add).0
    } else {
        let inner = egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(width - 18.0);
            ui.vertical(add);
        });
        ui.interact(inner.response.rect, ui.id().with(id), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    if selected {
        let color = if t.kind.layout() == Layout::Workspace { t.ws.primary } else { t.palette.accent };
        ui.painter().rect_stroke(r.rect, egui::CornerRadius::same(7), egui::Stroke::new(2.0_f32, color), egui::StrokeKind::Inside);
    }
    r
}

impl App {
    /// Open the setup (first start, Help or Preferences).
    pub(crate) fn open_setup(&mut self) {
        self.ux.setup = Some(Setup::new(self.online.settings.name.clone(), self.ux.prefs.check_updates));
    }

    /// The setup dialog, while open.
    pub(crate) fn setup_window(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.ux.setup.take() else { return };
        let outcome = self.setup_ui(ctx, &mut s);
        match outcome {
            Outcome::Open => self.ux.setup = Some(s),
            Outcome::Done(shortcut) => {
                self.setup_done(&s);
                match shortcut {
                    Some(Shortcut::NewCharacter) => self.wizard = Some(crate::wizard::Wizard::new()),
                    Some(Shortcut::OpenFile) => self.open_dialog(),
                    Some(Shortcut::JoinCampaign) => self.online.join = Some((String::new(), self.online.display_name())),
                    None => {}
                }
            }
        }
    }

    /// Keep what the setup chose: gui.ini (`setup_done`, appearance,
    /// `check_updates`) and the online name.
    fn setup_done(&mut self, s: &Setup) {
        let mut errors = Vec::new();
        if let Err(e) = theme::save_appearance(&self.appearance) {
            errors.push(e.to_string());
        }
        self.ux.prefs.check_updates = s.check_updates;
        // The notice was on the language step.
        if crate::lang_notice::needs_notice(&self.lang.code, &self.ux.prefs.notice_seen) {
            self.ux.prefs.notice_seen.push(self.lang.code.clone());
        }
        self.ux.notice = None;
        if let Err(e) = self.ux.prefs.save() {
            errors.push(e.to_string());
        }
        if let Err(e) = theme::save_value(crate::prefs::SETUP_DONE, "true") {
            errors.push(e.to_string());
        }
        let name = s.name.trim().to_owned();
        if name != self.online.settings.name {
            self.online.settings.name = name;
            if let Err(e) = self.online.settings.save() {
                errors.push(e.to_string());
            }
        }
        if !errors.is_empty() {
            self.status = Some((format!("Could not save the settings: {}", errors.join("; ")), true));
        }
    }

    fn setup_ui(&mut self, ctx: &egui::Context, s: &mut Setup) -> Outcome {
        let t = theme::current(ctx);
        let workspace = t.kind.layout() == Layout::Workspace;
        let mut outcome = Outcome::Open;
        let thumbs = s.thumbs(ctx).clone();
        let mut new_language = None;
        let mut new_look = None;
        egui::Modal::new(egui::Id::new("first_start_setup")).show(ctx, |ui| {
            ui.set_width(640.0);
            // Header: the steps as dots and the step's title.
            ui.horizontal(|ui| {
                for (i, step) in Step::ALL.iter().enumerate() {
                    let done = i <= s.step.index();
                    let color = if workspace { if done { t.ws.primary } else { t.ws.control } } else if done { t.palette.accent } else { t.palette.stroke };
                    let (r, _) = ui.allocate_exact_size(egui::vec2(if *step == s.step { 22.0 } else { 10.0 }, 6.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, egui::CornerRadius::same(3), color);
                }
                ui.add_space(8.0);
                ui.weak(self.lang.tr_fmt("Step {0} of {1}", &[&(s.step.index() + 1), &Step::ALL.len()]));
            });
            ui.add_space(6.0);
            let title = if s.step == Step::Language { self.lang.tr("Welcome to chummer-rs") } else { self.lang.tr(s.step.title()) };
            if workspace {
                ui.label(RichText::new(title).font(widgets::bold(19.0)).color(t.ws.text));
            } else {
                ui.heading(title);
            }
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_min_height(300.0);
                match s.step {
                    Step::Language => {
                        ui.label(self.lang.tr("A few questions to set chummer-rs up. Skip any of them; everything can be changed later."));
                        ui.add_space(8.0);
                        ui.label(theme::strong(ui, self.lang.tr("Language")));
                        egui::Grid::new("setup_languages").num_columns(2).spacing([24.0, 2.0]).show(ui, |ui| {
                            for (i, (code, name)) in self.languages.iter().enumerate() {
                                if crate::combo::selectable_label(ui, self.lang.code == *code, name).clicked() && self.lang.code != *code {
                                    new_language = Some(code.clone());
                                }
                                if i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                        if !self.lang.code.starts_with("en") {
                            ui.add_space(10.0);
                            let frame = if workspace { widgets::card_frame(&t.ws) } else { egui::Frame::group(ui.style()) };
                            frame.show(ui, |ui| {
                                ui.label(theme::strong(ui, self.lang.tr("Translation incomplete")).color(if workspace { t.ws.warning } else { t.palette.warning }));
                                ui.label(crate::lang_notice::notice_text(&self.lang));
                            });
                        }
                    }
                    Step::Layout => {
                        ui.label(self.lang.tr("How should chummer-rs look? You can switch any time in View → Appearance."));
                        ui.add_space(8.0);
                        ui.horizontal_top(|ui| {
                            for (layout, tex, text) in [
                                (Layout::Workspace, &thumbs[0], self.lang.tr("A modern layout: sections in a sidebar, an inspector and a command palette (Ctrl+K).")),
                                (Layout::Classic, &thumbs[1], self.lang.tr("Chummer5a's layout: menus, a toolbar and tabs, as in the Windows app.")),
                            ] {
                                let r = choice(ui, layout.as_str(), self.appearance.layout == layout, 300.0, |ui| {
                                    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(276.0, 172.0)).corner_radius(4));
                                    ui.add_space(4.0);
                                    ui.label(theme::strong(ui, self.lang.tr(layout.label())));
                                    ui.weak(text);
                                });
                                if r.clicked() {
                                    new_look = Some(with_layout(self.appearance, layout));
                                }
                            }
                        });
                    }
                    Step::Theme => {
                        ui.label(self.lang.tr("Pick the colours."));
                        ui.add_space(8.0);
                        let kinds = match self.appearance.layout {
                            Layout::Workspace => ThemeKind::WORKSPACE,
                            Layout::Classic => [ThemeKind::Graphite, ThemeKind::Classic],
                        };
                        ui.horizontal_top(|ui| {
                            for k in kinds {
                                let p = theme::Theme::of(k);
                                let note = match k {
                                    ThemeKind::WorkspaceDark => self.lang.tr("Dark background, easy on the eyes at the table"),
                                    ThemeKind::WorkspaceLight => self.lang.tr("Light background, for bright rooms"),
                                    ThemeKind::Graphite => self.lang.tr("Dark grey, flat"),
                                    ThemeKind::Classic => self.lang.tr("Light, like the Windows app"),
                                };
                                let r = choice(ui, k.as_str(), self.appearance.kind() == k, 220.0, |ui| {
                                    // A swatch of the theme's colours.
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(196.0, 70.0), egui::Sense::hover());
                                    let (ground, card, ink, accent) = if k.layout() == Layout::Workspace { (p.ws.ground, p.ws.raised, p.ws.text, p.ws.primary) } else { (p.palette.panel, p.palette.window, p.palette.text, p.palette.accent) };
                                    ui.painter().rect_filled(rect, egui::CornerRadius::same(5), ground);
                                    let c = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 12.0), egui::vec2(120.0, 46.0));
                                    ui.painter().rect_filled(c, egui::CornerRadius::same(4), card);
                                    ui.painter().rect_filled(egui::Rect::from_min_size(c.min + egui::vec2(10.0, 10.0), egui::vec2(80.0, 6.0)), egui::CornerRadius::same(2), ink);
                                    ui.painter().rect_filled(egui::Rect::from_min_size(c.min + egui::vec2(10.0, 26.0), egui::vec2(50.0, 10.0)), egui::CornerRadius::same(2), accent);
                                    ui.add_space(4.0);
                                    ui.label(theme::strong(ui, self.lang.tr(k.label())));
                                    ui.weak(note);
                                });
                                if r.clicked() {
                                    new_look = Some(self.appearance.with_kind(k));
                                }
                            }
                        });
                    }
                    Step::Sourcebooks => {
                        ui.label(self.lang.tr("Link your sourcebook PDFs to open a rule's page from its reference (\"SR5 p. 143\"). chummer-rs can take the links from Chummer5a or find the books in a folder."));
                        ui.weak(self.lang.tr("Later: Tools → Sourcebooks (PDFs)."));
                        ui.add_space(8.0);
                        if self.sources_window.quick_ui(ui, &mut self.pdfs, &self.lang) {
                            if let Err(e) = self.pdfs.save() {
                                self.status = Some((format!("Could not save sourcebook settings: {e}"), true));
                            }
                        }
                    }
                    Step::Online => {
                        ui.label(self.lang.tr("Play in a GM's online campaign, or run one. Your name is shown to the GM and the other players."));
                        ui.add_space(8.0);
                        ui.label(theme::strong(ui, self.lang.tr("Your name (optional)")));
                        let hint = self.online.display_name();
                        if workspace {
                            widgets::text_field(ui, &mut s.name, &hint, 320.0);
                        } else {
                            ui.add(egui::TextEdit::singleline(&mut s.name).hint_text(hint).desired_width(320.0));
                        }
                        ui.weak(self.lang.tr("Later: Tools → Online Settings."));
                    }
                    Step::Updates => {
                        ui.label(self.lang.tr("chummer-rs can look for a newer version and tell you when there is one."));
                        ui.add_space(8.0);
                        if workspace {
                            widgets::check(ui, &mut s.check_updates, &self.lang.tr("Check for updates"));
                        } else {
                            ui.checkbox(&mut s.check_updates, self.lang.tr("Check for updates"));
                        }
                        ui.weak(self.lang.tr("Later: Tools → Preferences."));
                    }
                    Step::Finish => {
                        ui.label(self.lang.tr("chummer-rs is ready. What would you like to do first?"));
                        ui.add_space(10.0);
                        for (shortcut, glyph, text, note) in [
                            (Shortcut::NewCharacter, "✨", self.lang.tr("New Character…"), self.lang.tr("Build a runner step by step")),
                            (Shortcut::OpenFile, "📂", self.lang.tr("Open…"), self.lang.tr("A .chum5 or .chum5lz file from Chummer5a or chummer-rs")),
                            (Shortcut::JoinCampaign, "🎭", self.lang.tr("Join Campaign…"), self.lang.tr("Play in a GM's online campaign with an invite link")),
                        ] {
                            let r = choice(ui, text.as_str(), false, 600.0, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(theme::glyph(glyph)).size(20.0));
                                    ui.vertical(|ui| {
                                        ui.label(theme::strong(ui, &text));
                                        ui.weak(&note);
                                    });
                                });
                            });
                            if r.clicked() {
                                outcome = Outcome::Done(Some(shortcut));
                            }
                            ui.add_space(4.0);
                        }
                    }
                }
            });
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                if !s.is_last() && button(ui, &self.lang.tr("Skip setup"), Look::Ghost).on_hover_text(self.lang.tr("Keep the defaults; Help → First-start setup… opens this again")).clicked() {
                    outcome = Outcome::Done(None);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let next = if s.is_last() { self.lang.tr("Close") } else { self.lang.tr("Next") };
                    if button(ui, &next, Look::Primary).clicked() && !s.next() {
                        outcome = Outcome::Done(None);
                    }
                    if s.step != Step::Language && button(ui, &self.lang.tr("Back"), Look::Secondary).clicked() {
                        s.back();
                    }
                });
            });
        });
        if let Some(code) = new_language {
            self.set_language(&code);
        }
        if let Some(a) = new_look {
            self.set_appearance(ctx, a);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_in_order() {
        let mut s = Setup::new(String::new(), true);
        assert_eq!(s.step, Step::Language);
        s.back();
        assert_eq!(s.step, Step::Language, "nothing before the first step");
        let mut seen = vec![s.step];
        while s.next() {
            seen.push(s.step);
        }
        assert_eq!(seen, Step::ALL.to_vec());
        assert!(s.is_last());
        assert!(!s.next(), "the finish page is the last");
        s.back();
        assert_eq!(s.step, Step::Updates);
        for st in Step::ALL {
            assert_eq!(Step::ALL[st.index()], st);
            assert!(!st.title().is_empty());
        }
    }

    #[test]
    fn shown_on_the_first_start_only() {
        // Very first start.
        assert!(should_show(None, false, false));
        // `--theme`/`--layout` given: the user (or a script) chose.
        assert!(!should_show(None, false, true));
        // An earlier run saved its language, but never a gui.ini.
        assert!(!should_show(None, true, false));
        // Existing users: gui.ini without the flag, or done.
        assert!(!should_show(Some("theme=classic\n"), false, false));
        assert!(!should_show(Some("setup_done=true\n"), false, false));
        // Asked for again.
        assert!(should_show(Some("layout=classic\nsetup_done=false\n"), true, false));
    }

    #[test]
    fn layout_keeps_its_theme() {
        let a = Appearance::default().with_kind(ThemeKind::WorkspaceLight);
        let c = with_layout(a, Layout::Classic);
        assert_eq!(c.kind(), ThemeKind::Graphite);
        assert_eq!(with_layout(c, Layout::Workspace).kind(), ThemeKind::WorkspaceLight);
    }
}
