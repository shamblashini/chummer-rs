#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! chummer-rs desktop application.

mod ai_ui;
mod browser;
mod career_ui;
mod combo;
mod dice_ui;
mod drug_ui;
mod gm_ui;
mod improvement_ui;
mod initiative;
mod lifestyle_ui;
mod magic_ui;
mod item_editor;
mod pdf_ui;
mod relationships_ui;
mod play_ui;
mod ruleset_ui;
mod select;
mod settings_ui;
mod theme;
mod tree_table;
mod view;
mod wizard;
#[cfg(test)]
mod tr_coverage;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sources::SourcebookLibrary;
use eframe::egui::{self, RichText};

use view::CharacterView;

const RECENT_KEY: &str = "recent_files";
const LANG_KEY: &str = "language";
const MAX_RECENT: usize = 10;
const ROSTER_KEY: &str = "roster_folders";

/// The non-character MDI tabs (Chummer's "Master Index" and "Character
/// Roster").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Home {
    MasterIndex,
    Roster,
}

/// What the MDI tab strip selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mdi {
    Home(Home),
    Character(usize),
}

enum Pending {
    CloseTab(usize),
    Quit,
}

struct App {
    engine: Arc<Engine>,
    lang: Language,
    lang_dir: PathBuf,
    languages: Vec<(String, String)>,
    views: Vec<CharacterView>,
    active: usize,
    /// A home tab in front of the characters, if one is selected.
    home: Option<Home>,
    show_dice: bool,
    show_about: bool,
    show_sources: bool,
    show_settings: bool,
    show_initiative: bool,
    show_print: bool,
    show_export: bool,
    export_format: String,
    roster_folders: Vec<PathBuf>,
    roster: Vec<chummer_core::roster::Entry>,
    print_sheet: String,
    print_notes: bool,
    initiative: initiative::Tracker,
    settings_editor: settings_ui::SettingsEditor,
    wizard: Option<wizard::Wizard>,
    critter: Option<gm_ui::CritterWizard>,
    pdfs: SourcebookLibrary,
    sources_window: pdf_ui::SourcesWindow,
    browser: browser::DataBrowser,
    dice: dice_ui::DiceRoller,
    recent: Vec<PathBuf>,
    status: Option<(String, bool)>,
    pending: Option<Pending>,
    allow_close: bool,
    theme: theme::ThemeKind,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, engine: Engine, files: Vec<PathBuf>, tab: Option<view::Tab>, theme_arg: Option<theme::ThemeKind>) -> Self {
        let theme = theme_arg.unwrap_or_else(theme::load_kind);
        theme::apply(&cc.egui_ctx, &theme::Theme::of(theme));
        let lang_dir = data::resource_dir("lang").unwrap_or_default();
        let storage = cc.storage;
        let code = storage.and_then(|s| s.get_string(LANG_KEY)).unwrap_or_else(|| "en-us".into());
        let recent = storage
            .and_then(|s| s.get_string(RECENT_KEY))
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        let sources_window = pdf_ui::SourcesWindow::new(&engine.store);
        let mut app = App {
            show_sources: false,
            show_settings: false,
            show_initiative: false,
            show_print: false,
            show_export: false,
            export_format: "JSON".into(),
            roster_folders: Vec::new(),
            roster: Vec::new(),
            print_sheet: chummer_core::print::DEFAULT_SHEET.to_owned(),
            print_notes: false,
            initiative: Default::default(),
            settings_editor: settings_ui::SettingsEditor::new(),
            wizard: None,
            critter: None,
            pdfs: SourcebookLibrary::load(),
            sources_window,
            engine: Arc::new(engine),
            lang: Language::load(&lang_dir, &code),
            languages: Language::available(&lang_dir),
            lang_dir,
            views: Vec::new(),
            active: 0,
            home: None,
            show_dice: false,
            show_about: false,
            browser: Default::default(),
            dice: Default::default(),
            recent,
            status: None,
            pending: None,
            allow_close: false,
            theme,
        };
        app.roster_folders = storage
            .and_then(|s| s.get_string(ROSTER_KEY))
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        app.roster = chummer_core::roster::scan(&app.roster_folders);
        for f in files {
            app.open(&f);
        }
        if let Some(t) = tab {
            for v in &mut app.views {
                v.set_tab(t);
            }
        }
        app
    }

    fn set_theme(&mut self, ctx: &egui::Context, kind: theme::ThemeKind) {
        self.theme = kind;
        theme::apply(ctx, &theme::Theme::of(kind));
        if let Err(e) = theme::save_kind(kind) {
            self.status = Some((format!("Could not save the theme choice: {e}"), true));
        }
    }

    fn open(&mut self, path: &Path) {
        if let Some(i) = self.views.iter().position(|v| v.path().as_deref() == Some(path)) {
            self.active = i;
            self.home = None;
            return;
        }
        match Character::load(path) {
            Ok(ch) => {
                self.views.push(CharacterView::new(ch, &self.engine));
                self.active = self.views.len() - 1;
                self.home = None;
                self.remember(path);
                self.status = Some((format!("Opened {}", path.display()), false));
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }

    fn remember(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(MAX_RECENT);
    }

    fn open_dialog(&mut self) {
        let files = rfd::FileDialog::new().add_filter("Chummer character", &["chum5"]).add_filter("All files", &["*"]).pick_files();
        for f in files.unwrap_or_default() {
            self.open(&f);
        }
    }

    fn save(&mut self, idx: usize, save_as: bool) -> bool {
        let Some(v) = self.views.get_mut(idx) else { return false };
        let path = match (save_as, v.path()) {
            (false, Some(p)) => Some(p),
            _ => rfd::FileDialog::new()
                .add_filter("Chummer character", &["chum5"])
                .set_file_name(format!("{}.chum5", v.ch.display_name()))
                .save_file(),
        };
        let Some(path) = path else { return false };
        match self.engine.save(&mut v.ch, &path) {
            Ok(()) => {
                self.status = Some((format!("Saved {}", path.display()), false));
                self.remember(&path);
                true
            }
            Err(e) => {
                self.status = Some((format!("Could not save {}: {e}", path.display()), true));
                false
            }
        }
    }

    fn close_tab(&mut self, idx: usize, force: bool) {
        if idx >= self.views.len() {
            return;
        }
        if self.views[idx].ch.dirty && !force {
            self.pending = Some(Pending::CloseTab(idx));
            return;
        }
        self.views.remove(idx);
        if self.active >= self.views.len() {
            self.active = self.views.len().saturating_sub(1);
        }
        if self.views.is_empty() {
            self.home = Some(Home::Roster);
        }
    }

    /// The character in front, if any.
    fn current(&self) -> Option<usize> {
        (self.home.is_none() && self.active < self.views.len()).then_some(self.active)
    }

    fn mdi(&self) -> Mdi {
        match (self.home, self.current()) {
            (None, Some(i)) => Mdi::Character(i),
            (Some(h), _) => Mdi::Home(h),
            (None, None) => Mdi::Home(Home::Roster),
        }
    }

    fn select(&mut self, m: Mdi) {
        match m {
            Mdi::Home(h) => self.home = Some(h),
            Mdi::Character(i) => {
                self.active = i;
                self.home = None;
            }
        }
    }

    /// Chummer's main menu: File, Tools, Special, View, Window, Help.
    fn menu(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let has = self.current().is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(self.lang.tr("File"), |ui| {
                if ui.add(egui::Button::new(self.lang.tr("New Character…")).shortcut_text("Ctrl+N")).clicked() {
                    ui.close();
                    self.wizard = Some(wizard::Wizard::new());
                }
                if ui.button(self.lang.tr("New Critter…")).clicked() {
                    ui.close();
                    self.critter = Some(gm_ui::CritterWizard::new());
                }
                if ui.add(egui::Button::new(self.lang.tr("Open…")).shortcut_text("Ctrl+O")).clicked() {
                    ui.close();
                    self.open_dialog();
                }
                ui.menu_button(self.lang.tr("Open recent"), |ui| {
                    if self.recent.is_empty() {
                        ui.weak(self.lang.tr("No recent files"));
                    }
                    for p in self.recent.clone() {
                        let label = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.button(label).on_hover_text(p.display().to_string()).clicked() {
                            ui.close();
                            self.open(&p);
                        }
                    }
                });
                ui.separator();
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Save")).shortcut_text("Ctrl+S")).clicked() {
                    ui.close();
                    self.save(self.active, false);
                }
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Save As…"))).clicked() {
                    ui.close();
                    self.save(self.active, true);
                }
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Print…")).shortcut_text("Ctrl+P")).clicked() {
                    ui.close();
                    self.show_print = true;
                }
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Export…"))).clicked() {
                    ui.close();
                    self.show_export = true;
                }
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Close")).shortcut_text("Ctrl+W")).clicked() {
                    ui.close();
                    self.close_tab(self.active, false);
                }
                ui.separator();
                if ui.add(egui::Button::new(self.lang.tr("Exit")).shortcut_text("Ctrl+Q")).clicked() {
                    ui.close();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(self.lang.tr("Tools"), |ui| {
                if ui.button(self.lang.tr("Dice Roller")).clicked() {
                    ui.close();
                    self.show_dice = true;
                }
                if ui.button(self.lang.tr("Master Index")).clicked() {
                    ui.close();
                    self.home = Some(Home::MasterIndex);
                }
                if ui.button(self.lang.tr("Character Roster")).clicked() {
                    ui.close();
                    self.home = Some(Home::Roster);
                }
                if ui.button(self.lang.tr("Initiative tracker")).clicked() {
                    ui.close();
                    self.show_initiative = true;
                }
                ui.separator();
                if ui.button(self.lang.tr("Character Settings…")).clicked() {
                    ui.close();
                    self.show_settings = true;
                }
                if ui.button(self.lang.tr("Sourcebooks (PDFs)…")).clicked() {
                    ui.close();
                    self.show_sources = true;
                }
            });
            ui.menu_button(self.lang.tr("Special"), |ui| {
                let creating = self.current().is_some_and(|i| !self.views[i].ch.created);
                for (label, mode) in [("Add PACKS Kit…", gm_ui::PacksMode::Add), ("Create PACKS Kit…", gm_ui::PacksMode::Create)] {
                    if ui.add_enabled(creating, egui::Button::new(self.lang.tr(label))).clicked() {
                        ui.close();
                        self.views[self.active].open_packs(mode);
                    }
                }
            });
            ui.menu_button(self.lang.tr("View"), |ui| {
                ui.menu_button(self.lang.tr("Theme"), |ui| {
                    for k in theme::ThemeKind::ALL {
                        if crate::combo::selectable_label(ui, self.theme == k, self.lang.tr(k.label())).clicked() {
                            ui.close();
                            self.set_theme(ctx, k);
                        }
                    }
                });
                ui.menu_button(self.lang.tr("Language"), |ui| {
                    for (code, name) in self.languages.clone() {
                        if crate::combo::selectable_label(ui, self.lang.code == code, name).clicked() {
                            ui.close();
                            self.lang = Language::load(&self.lang_dir, &code);
                        }
                    }
                });
            });
            ui.menu_button(self.lang.tr("Window"), |ui| {
                let current = self.mdi();
                for (m, label) in self.mdi_tabs() {
                    if crate::combo::selectable_label(ui, current == m, label).clicked() {
                        ui.close();
                        self.select(m);
                    }
                }
                ui.separator();
                if ui.add_enabled(has, egui::Button::new(self.lang.tr("Close"))).clicked() {
                    ui.close();
                    self.close_tab(self.active, false);
                }
            });
            ui.menu_button(self.lang.tr("Help"), |ui| {
                if ui.button(self.lang.tr("About")).clicked() {
                    ui.close();
                    self.show_about = true;
                }
            });
        });
    }

    /// The MDI tabs: Master Index, Character Roster, then one per character.
    fn mdi_tabs(&self) -> Vec<(Mdi, String)> {
        let mut tabs = vec![(Mdi::Home(Home::MasterIndex), self.lang.tr("Master Index")), (Mdi::Home(Home::Roster), self.lang.tr("Character Roster"))];
        tabs.extend(self.views.iter().enumerate().map(|(i, v)| (Mdi::Character(i), v.title())));
        tabs
    }

    /// Toolbar (New, Open, Save, Print) and the MDI tab strip.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let has = self.current().is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui.add(egui::Button::new(RichText::new("✨").size(16.0)).frame(false)).on_hover_text(self.lang.tr("New Character…")).clicked() {
                self.wizard = Some(wizard::Wizard::new());
            }
            if ui.add(egui::Button::new(RichText::new("📂").size(16.0)).frame(false)).on_hover_text(self.lang.tr("Open…")).clicked() {
                self.open_dialog();
            }
            if ui.add_enabled(has, egui::Button::new(RichText::new("💾").size(16.0)).frame(false)).on_hover_text(self.lang.tr("Save")).clicked() {
                self.save(self.active, false);
            }
            if ui.add_enabled(has, egui::Button::new(RichText::new("🖶").size(16.0)).frame(false)).on_hover_text(self.lang.tr("Print…")).clicked() {
                self.show_print = true;
            }
            ui.separator();
            if ui.add(egui::Button::new(RichText::new("🎲").size(16.0)).frame(false)).on_hover_text(self.lang.tr("Dice Roller")).clicked() {
                self.show_dice = true;
            }
        });
        let current = self.mdi();
        let mut pick = None;
        let mut close = None;
        theme::strip_frame(ui, |ui| {
            for (m, label) in self.mdi_tabs() {
                let (r, closed) = theme::tab(ui, current == m, &label, matches!(m, Mdi::Character(_)));
                if closed {
                    if let Mdi::Character(i) = m {
                        close = Some(i);
                    }
                } else if r.clicked() {
                    pick = Some(m);
                }
            }
        });
        if let Some(m) = pick {
            self.select(m);
        }
        if let Some(i) = close {
            self.close_tab(i, false);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) && self.current().is_some() {
            self.show_print = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.wizard = Some(wizard::Wizard::new());
        }
        let (open, save, close, quit) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q),
            )
        });
        if open {
            self.open_dialog();
        }
        if save && self.current().is_some() {
            self.save(self.active, false);
        }
        if close && self.current().is_some() {
            self.close_tab(self.active, false);
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            self.open(&p);
        }
    }

    /// The Character Roster tab: Chummer's list of recent and roster
    /// characters on the left, getting started on the right.
    fn welcome(&mut self, ctx: &egui::Context) {
        let mut open_path = None;
        egui::SidePanel::left("roster_panel").resizable(true).default_width(460.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("roster").auto_shrink(false).show(ui, |ui| {
                ui.label(crate::theme::strong(ui, self.lang.tr("Recent Characters")));
                if self.recent.is_empty() {
                    ui.weak(self.lang.tr("No recent files"));
                }
                for p in &self.recent {
                    let label = p.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                    if ui.link(label).on_hover_text(p.display().to_string()).clicked() {
                        open_path = Some(p.clone());
                    }
                }
                ui.add_space(12.0);
                ui.label(crate::theme::strong(ui, self.lang.tr("Character Roster")));
                ui.horizontal(|ui| {
                    if ui.button(self.lang.tr("Add folder…")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.roster_folders.push(d);
                            self.roster = chummer_core::roster::scan(&self.roster_folders);
                        }
                    }
                    if !self.roster_folders.is_empty() && ui.button(self.lang.tr("Refresh")).clicked() {
                        self.roster = chummer_core::roster::scan(&self.roster_folders);
                    }
                    if !self.roster_folders.is_empty() && ui.button(self.lang.tr("Clear folders")).clicked() {
                        self.roster_folders.clear();
                        self.roster.clear();
                    }
                });
                egui::Grid::new("roster_grid").striped(true).num_columns(4).show(ui, |ui| {
                    for e in &self.roster {
                        if ui.link(e.display_name()).on_hover_text(e.path.display().to_string()).clicked() {
                            open_path = Some(e.path.clone());
                        }
                        ui.label(&e.metatype);
                        ui.weak(if e.career { self.lang.tr("Career Mode") } else { self.lang.tr("Create Mode") });
                        ui.weak(e.error.clone().unwrap_or_else(|| format!("{} {}", self.lang.tr("Karma"), e.karma)));
                        ui.end_row();
                    }
                });
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.2);
                ui.label(RichText::new("chummer-rs").size(40.0).color(crate::theme::accent(ui)).strong());
                ui.label(self.lang.tr("Shadowrun 5th Edition character manager"));
                ui.add_space(20.0);
                if ui.add(crate::theme::primary_button(ui, format!("✨  {}", self.lang.tr("Create New Character…")))).clicked() {
                    self.wizard = Some(wizard::Wizard::new());
                }
                if ui.button(format!("📂  {}", self.lang.tr("Open Character…"))).clicked() {
                    self.open_dialog();
                }
                ui.weak(self.lang.tr("or drop .chum5 files onto this window"));
                ui.add_space(16.0);
                if self.pdfs.linked_count() == 0 && ui.button(format!("📖 {}", self.lang.tr("Link your sourcebook PDFs…"))).clicked() {
                    self.show_sources = true;
                }
                if ui.button(self.lang.tr("Master Index")).clicked() {
                    self.home = Some(Home::MasterIndex);
                }
                if ui.button(self.lang.tr("Dice Roller")).clicked() {
                    self.show_dice = true;
                }
            });
        });
        if let Some(p) = open_path {
            self.open(&p);
        }
    }

    fn export_ui(&mut self, ui: &mut egui::Ui) {
        let Some(v) = self.views.get(self.active) else {
            ui.label(self.lang.tr("Open a character first."));
            return;
        };
        let formats: Vec<String> = chummer_core::export::BUILT_IN.iter().map(|s| s.to_string()).chain(chummer_core::export::stylesheets().into_iter().map(|(n, _)| n)).collect();
        crate::combo::Combo::from_id_salt("export_fmt").selected_text(self.export_format.clone()).show_ui(ui, |ui| {
            for f in &formats {
                crate::combo::selectable_value(ui, &mut self.export_format, f.clone(), f);
            }
        });
        ui.weak(self.lang.tr("XML and JSON contain the full print data; stylesheets produce their own format."));
        if ui.button(self.lang.tr("Export…")).clicked() {
            let ext = match self.export_format.as_str() {
                "JSON" => "json",
                "XML" => "xml",
                _ => "txt",
            };
            if let Some(out) = rfd::FileDialog::new().set_file_name(format!("{}.{ext}", v.ch.display_name())).save_file() {
                self.status = Some(match chummer_core::export::export(&v.ch, &self.engine, &self.lang, &self.export_format, &out) {
                    Ok(()) => (format!("Exported to {}", out.display()), false),
                    Err(e) => (e.to_string(), true),
                });
            }
        }
    }

    /// Render the active character with a Chummer sheet and open it.
    fn print_ui(&mut self, ui: &mut egui::Ui) {
        let Some(v) = self.views.get(self.active) else {
            ui.label(self.lang.tr("Open a character first."));
            return;
        };
        let sheets = chummer_core::print::available_sheets(&self.lang.code);
        ui.horizontal(|ui| {
            ui.label(self.lang.tr("Character Sheet:"));
            crate::combo::Combo::from_id_salt("sheet").selected_text(self.print_sheet.clone()).width(320.0).show_ui(ui, |ui| {
                for (name, _) in &sheets {
                    crate::combo::selectable_value(ui, &mut self.print_sheet, name.clone(), name);
                }
            });
        });
        ui.checkbox(&mut self.print_notes, self.lang.tr("Include notes"));
        ui.weak(self.lang.tr("The sheet opens in your browser; use its Print command for paper or PDF."));
        if ui.button(RichText::new(self.lang.tr("Open sheet")).strong()).clicked() {
            let Some((_, path)) = sheets.iter().find(|(n, _)| *n == self.print_sheet).or(sheets.first()) else {
                self.status = Some(("No character sheets found".into(), true));
                return;
            };
            let opts = chummer_core::print::PrintOptions { notes: self.print_notes, ..Default::default() };
            let xml = chummer_core::print::print_xml_with(&v.ch, &self.engine, &self.lang, opts);
            let name: String = v.ch.display_name().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let out = std::env::temp_dir().join(format!("chummer-rs-{name}.html"));
            match chummer_core::print::render(&xml, path, &out) {
                Ok(()) => {
                    let opened = std::process::Command::new("xdg-open").arg(&out).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
                    self.status = Some(match opened {
                        Ok(_) => (format!("Opened {}", out.display()), false),
                        Err(e) => (format!("Sheet written to {} (could not open it: {e})", out.display()), true),
                    });
                }
                Err(e) => self.status = Some((e.to_string(), true)),
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pending else { return };
        let (idx, what) = match p {
            Pending::CloseTab(i) => (Some(*i), self.views.get(*i).map(|v| v.ch.display_name()).unwrap_or_default()),
            Pending::Quit => (None, "your characters".to_owned()),
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.heading(self.lang.tr("Unsaved Changes"));
            ui.label(format!("Save changes to {what} before closing?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(self.lang.tr("Save")).clicked() {
                    choice = Some(0);
                }
                if ui.button(self.lang.tr("Don't save")).clicked() {
                    choice = Some(1);
                }
                if ui.button(self.lang.tr("Cancel")).clicked() {
                    choice = Some(2);
                }
            });
        });
        match (choice, idx) {
            (Some(0), Some(i)) => {
                if self.save(i, false) {
                    self.close_tab(i, true);
                }
                self.pending = None;
            }
            (Some(1), Some(i)) => {
                self.close_tab(i, true);
                self.pending = None;
            }
            (Some(0), None) => {
                let dirty: Vec<usize> = (0..self.views.len()).filter(|&i| self.views[i].ch.dirty).collect();
                if dirty.into_iter().all(|i| self.save(i, false)) {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                self.pending = None;
            }
            (Some(1), None) => {
                self.allow_close = true;
                self.pending = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Some(_), _) => self.pending = None,
            _ => {}
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.views.iter().any(|v| v.ch.dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
        self.shortcuts(ctx);

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            self.menu(ctx, ui);
            self.toolbar(ui);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Chummer's status strip: the character's karma, essence, nuyen.
                if let Some(i) = self.current() {
                    for (label, value) in self.views[i].status_items(&self.lang) {
                        ui.label(label);
                        ui.strong(value);
                        ui.separator();
                    }
                }
                match &self.status {
                    Some((msg, true)) => {
                        ui.colored_label(ui.visuals().error_fg_color, msg);
                    }
                    Some((msg, false)) => {
                        ui.weak(msg);
                    }
                    None => {
                        ui.weak(self.lang.tr("Ready"));
                    }
                }
            });
        });

        match self.mdi() {
            Mdi::Home(Home::Roster) => self.welcome(ctx),
            Mdi::Home(Home::MasterIndex) => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.browser.ui(ui, &self.engine.store, &self.lang, &self.pdfs, &mut self.status);
                });
            }
            Mdi::Character(idx) => {
                let engine = self.engine.clone();
                if let Some(pool) = self.views[idx].ui(ctx, &engine, &self.lang, &self.pdfs, &mut self.status) {
                    self.dice.set_pool(pool);
                    self.show_dice = true;
                }
                if let Some(p) = relationships_ui::take_open_request(ctx) {
                    self.open(&p);
                }
            }
        }

        let mut open = self.show_dice;
        egui::Window::new(self.lang.tr("Dice Roller")).id(egui::Id::new("dice_roller")).open(&mut open).default_width(360.0).show(ctx, |ui| self.dice.ui(ui, &self.lang));
        self.show_dice = open;
        let mut open = self.show_sources;
        egui::Window::new(self.lang.tr("Sourcebooks")).id(egui::Id::new("sourcebooks")).open(&mut open).default_size([820.0, 620.0]).show(ctx, |ui| {
            if self.sources_window.ui(ui, &mut self.pdfs, &self.lang) {
                if let Err(e) = self.pdfs.save() {
                    self.status = Some((format!("Could not save sourcebook settings: {e}"), true));
                }
            }
        });
        self.show_sources = open;
        let mut open = self.show_export;
        egui::Window::new(self.lang.tr("Export Character")).id(egui::Id::new("export_character")).open(&mut open).default_width(420.0).show(ctx, |ui| self.export_ui(ui));
        self.show_export = open;
        let mut open = self.show_print;
        egui::Window::new(self.lang.tr("Character Sheet")).id(egui::Id::new("character_sheet")).open(&mut open).default_width(460.0).show(ctx, |ui| self.print_ui(ui));
        self.show_print = open;
        let mut open = self.show_initiative;
        let chars: Vec<(String, i32, u32)> = self.views.iter().map(|v| (v.ch.display_name(), v.sheet.initiative, v.sheet.initiative_dice.max(1) as u32)).collect();
        egui::Window::new(self.lang.tr("Initiative tracker")).id(egui::Id::new("initiative_tracker")).open(&mut open).default_width(480.0).show(ctx, |ui| self.initiative.ui(ui, &self.lang, &chars));
        self.show_initiative = open;
        let mut open = self.show_settings;
        let mut reload = false;
        if ruleset_ui::take_import_request(ctx) {
            open = true;
            reload = self.settings_editor.start_import(&self.engine, &self.lang);
        }
        egui::Window::new(self.lang.tr("Character Settings")).id(egui::Id::new("character_settings")).open(&mut open).default_size([820.0, 680.0]).show(ctx, |ui| {
            reload |= self.settings_editor.ui(ui, &self.engine, &self.lang);
        });
        self.show_settings = open;
        if reload {
            if let Some(engine) = Arc::get_mut(&mut self.engine) {
                if let Ok(lib) = chummer_core::settings::SettingsLibrary::load(&engine.store, chummer_core::settings::user_settings_dir().as_deref()) {
                    engine.settings = lib;
                }
            } else {
                self.status = Some(("Could not reload the settings library; restart chummer-rs to use the new settings".into(), true));
            }
            for v in &mut self.views {
                v.refresh_settings(&self.engine);
            }
        }
        let mut open = self.show_about;
        egui::Window::new(format!("{} chummer-rs", self.lang.tr("About"))).id(egui::Id::new("about")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("chummer-rs {}", env!("CARGO_PKG_VERSION")));
            ui.label(self.lang.tr("A Rust rewrite of Chummer5a, the Shadowrun 5e character manager."));
            ui.label(self.lang.tr("Game data and translations come from Chummer5a (GPL-3.0)."));
            ui.hyperlink("https://github.com/chummer5a/chummer5a");
        });
        self.show_about = open;
        if let Some(w) = self.wizard.as_mut() {
            match w.show(ctx, &self.engine, &self.lang) {
                wizard::WizardResult::Open => {}
                wizard::WizardResult::Cancel => self.wizard = None,
                wizard::WizardResult::Created(ch) => {
                    let mut v = CharacterView::new(*ch, &self.engine);
                    v.set_tab(view::Tab::Common);
                    self.views.push(v);
                    self.active = self.views.len() - 1;
                    self.home = None;
                    self.wizard = None;
                    self.status = Some(("New character created. Spend your points, then Finish creation.".into(), false));
                }
            }
        }
        if let Some(w) = self.critter.as_mut() {
            match w.show(ctx, &self.engine, &self.lang) {
                gm_ui::CritterResult::Open => {}
                gm_ui::CritterResult::Cancel => self.critter = None,
                gm_ui::CritterResult::Created(ch) => {
                    self.views.push(CharacterView::new(*ch, &self.engine));
                    self.active = self.views.len() - 1;
                    self.home = None;
                    self.critter = None;
                    self.status = Some(("New critter created.".into(), false));
                }
            }
        }
        self.dialogs(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let recent: Vec<String> = self.recent.iter().map(|p| p.display().to_string()).collect();
        storage.set_string(RECENT_KEY, recent.join("\n"));
        storage.set_string(LANG_KEY, self.lang.code.clone());
        let folders: Vec<String> = self.roster_folders.iter().map(|p| p.display().to_string()).collect();
        storage.set_string(ROSTER_KEY, folders.join("\n"));
    }
}

fn main() -> anyhow::Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut tab = None;
    let mut window: Option<String> = None;
    let mut theme_arg = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tab" => tab = args.next().and_then(|t| view::Tab::parse(&t)),
            "--window" => window = args.next(),
            "--theme" => theme_arg = args.next().and_then(|t| theme::ThemeKind::parse(&t)),
            "--new" => window = Some("new".into()),
            "-h" | "--help" => {
                println!("usage: chummer-rs [--tab <common|skills|limits|martial|spells|adept|complex|critter|initiation|cyberware|street|vehicles|character|karma|calendar|game|improvements|relationships>] [--window <sources|browser|dice>] [--theme <classic|graphite>] [file.chum5 ...]");
                return Ok(());
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    let engine = match Engine::load() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("chummer-rs: {e}");
            std::process::exit(1);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("chummer-rs")
            .with_app_id("chummer-rs")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("chummer-rs", options, Box::new(move |cc| {
        let mut app = App::new(cc, engine, files, tab, theme_arg);
        match window.as_deref() {
            Some("sources") => app.show_sources = true,
            Some("browser") => app.home = Some(Home::MasterIndex),
            Some("dice") => app.show_dice = true,
            Some("new") => app.wizard = Some(wizard::Wizard::new()),
            Some("settings") => app.show_settings = true,
            _ => {}
        }
        Ok(Box::new(app))
    }))
        .map_err(|e| anyhow::anyhow!("{e}"))
}
