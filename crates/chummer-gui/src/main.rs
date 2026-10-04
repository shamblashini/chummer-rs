//! chummer-rs desktop application.

mod browser;
mod dice_ui;
mod initiative;
mod pdf_ui;
mod select;
mod settings_ui;
mod view;
mod wizard;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chummer_core::character::Character;
use chummer_core::data;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sources::SourcebookLibrary;
use eframe::egui::{self, RichText};

use view::{CharacterView, ACCENT};

const RECENT_KEY: &str = "recent_files";
const LANG_KEY: &str = "language";
const MAX_RECENT: usize = 10;

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
    show_browser: bool,
    show_dice: bool,
    show_about: bool,
    show_sources: bool,
    show_settings: bool,
    show_initiative: bool,
    initiative: initiative::Tracker,
    settings_editor: settings_ui::SettingsEditor,
    wizard: Option<wizard::Wizard>,
    pdfs: SourcebookLibrary,
    sources_window: pdf_ui::SourcesWindow,
    browser: browser::DataBrowser,
    dice: dice_ui::DiceRoller,
    recent: Vec<PathBuf>,
    status: Option<(String, bool)>,
    pending: Option<Pending>,
    allow_close: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, engine: Engine, files: Vec<PathBuf>, tab: Option<view::Tab>) -> Self {
        setup_style(&cc.egui_ctx);
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
            initiative: Default::default(),
            settings_editor: settings_ui::SettingsEditor::new(),
            wizard: None,
            pdfs: SourcebookLibrary::load(),
            sources_window,
            engine: Arc::new(engine),
            lang: Language::load(&lang_dir, &code),
            languages: Language::available(&lang_dir),
            lang_dir,
            views: Vec::new(),
            active: 0,
            show_browser: false,
            show_dice: false,
            show_about: false,
            browser: Default::default(),
            dice: Default::default(),
            recent,
            status: None,
            pending: None,
            allow_close: false,
        };
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

    fn open(&mut self, path: &Path) {
        if let Some(i) = self.views.iter().position(|v| v.path().as_deref() == Some(path)) {
            self.active = i;
            return;
        }
        match Character::load(path) {
            Ok(ch) => {
                self.views.push(CharacterView::new(ch, &self.engine));
                self.active = self.views.len() - 1;
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
    }

    fn menu(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New character…").shortcut_text("Ctrl+N")).clicked() {
                    ui.close();
                    self.wizard = Some(wizard::Wizard::new());
                }
                if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).clicked() {
                    ui.close();
                    self.open_dialog();
                }
                ui.menu_button("Open recent", |ui| {
                    if self.recent.is_empty() {
                        ui.weak("No recent files");
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
                let has = !self.views.is_empty();
                if ui.add_enabled(has, egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    ui.close();
                    self.save(self.active, false);
                }
                if ui.add_enabled(has, egui::Button::new("Save as…")).clicked() {
                    ui.close();
                    self.save(self.active, true);
                }
                if ui.add_enabled(has, egui::Button::new("Close").shortcut_text("Ctrl+W")).clicked() {
                    ui.close();
                    self.close_tab(self.active, false);
                }
                ui.separator();
                if ui.add(egui::Button::new("Quit").shortcut_text("Ctrl+Q")).clicked() {
                    ui.close();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Tools", |ui| {
                if ui.button("Character settings (house rules)…").clicked() {
                    ui.close();
                    self.show_settings = true;
                }
                if ui.button("Sourcebooks (PDFs)…").clicked() {
                    ui.close();
                    self.show_sources = true;
                }
                if ui.button("Data browser").clicked() {
                    ui.close();
                    self.show_browser = true;
                }
                if ui.button("Dice roller").clicked() {
                    ui.close();
                    self.show_dice = true;
                }
                if ui.button("Initiative tracker").clicked() {
                    ui.close();
                    self.show_initiative = true;
                }
            });
            ui.menu_button("Language", |ui| {
                for (code, name) in self.languages.clone() {
                    if ui.selectable_label(self.lang.code == code, name).clicked() {
                        ui.close();
                        self.lang = Language::load(&self.lang_dir, &code);
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About").clicked() {
                    ui.close();
                    self.show_about = true;
                }
            });
        });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
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
        if save && !self.views.is_empty() {
            self.save(self.active, false);
        }
        if close && !self.views.is_empty() {
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

    fn welcome(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.2);
                ui.label(RichText::new("chummer-rs").size(40.0).color(ACCENT).strong());
                ui.label("Shadowrun 5th Edition character manager");
                ui.add_space(20.0);
                if ui.button(RichText::new("✨  Create a new character…").size(18.0)).clicked() {
                    self.wizard = Some(wizard::Wizard::new());
                }
                if ui.button(RichText::new("📂  Open a character…").size(18.0)).clicked() {
                    self.open_dialog();
                }
                ui.weak("or drop .chum5 files onto this window");
                ui.add_space(16.0);
                if !self.recent.is_empty() {
                    ui.label(RichText::new("Recent").strong());
                    for p in self.recent.clone() {
                        let label = p.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.link(label).on_hover_text(p.display().to_string()).clicked() {
                            self.open(&p);
                        }
                    }
                }
                ui.add_space(16.0);
                if self.pdfs.linked_count() == 0 && ui.button("📖 Link your sourcebook PDFs…").clicked() {
                    self.show_sources = true;
                }
                if ui.button("Data browser").clicked() {
                    self.show_browser = true;
                }
                if ui.button("Dice roller").clicked() {
                    self.show_dice = true;
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pending else { return };
        let (idx, what) = match p {
            Pending::CloseTab(i) => (Some(*i), self.views.get(*i).map(|v| v.ch.display_name()).unwrap_or_default()),
            Pending::Quit => (None, "your characters".to_owned()),
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!("Save changes to {what} before closing?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
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
            if !self.views.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    let mut close = None;
                    for (i, v) in self.views.iter().enumerate() {
                        ui.selectable_value(&mut self.active, i, v.title());
                        if ui.small_button("×").on_hover_text("Close").clicked() {
                            close = Some(i);
                        }
                        ui.separator();
                    }
                    if let Some(i) = close {
                        self.close_tab(i, false);
                    }
                });
            }
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| match &self.status {
                Some((msg, true)) => {
                    ui.colored_label(ui.visuals().error_fg_color, msg);
                }
                Some((msg, false)) => {
                    ui.weak(msg);
                }
                None => {
                    ui.weak("Ready");
                }
            });
        });

        if self.views.is_empty() {
            self.welcome(ctx);
        } else {
            let engine = self.engine.clone();
            let idx = self.active.min(self.views.len() - 1);
            if let Some(pool) = self.views[idx].ui(ctx, &engine, &self.lang, &self.pdfs, &mut self.status) {
                self.dice.set_pool(pool);
                self.show_dice = true;
            }
        }

        let mut open = self.show_browser;
        egui::Window::new("Data browser").open(&mut open).default_size([900.0, 600.0]).show(ctx, |ui| {
            self.browser.ui(ui, &self.engine.store, &self.lang, &self.pdfs, &mut self.status);
        });
        self.show_browser = open;
        let mut open = self.show_dice;
        egui::Window::new("Dice roller").open(&mut open).default_width(360.0).show(ctx, |ui| self.dice.ui(ui));
        self.show_dice = open;
        let mut open = self.show_sources;
        egui::Window::new("Sourcebooks").open(&mut open).default_size([820.0, 620.0]).show(ctx, |ui| {
            if self.sources_window.ui(ui, &mut self.pdfs) {
                if let Err(e) = self.pdfs.save() {
                    self.status = Some((format!("Could not save sourcebook settings: {e}"), true));
                }
            }
        });
        self.show_sources = open;
        let mut open = self.show_initiative;
        let chars: Vec<(String, i32, u32)> = self.views.iter().map(|v| (v.ch.display_name(), v.sheet.initiative, v.sheet.initiative_dice.max(1) as u32)).collect();
        egui::Window::new("Initiative tracker").open(&mut open).default_width(480.0).show(ctx, |ui| self.initiative.ui(ui, &chars));
        self.show_initiative = open;
        let mut open = self.show_settings;
        let mut reload = false;
        egui::Window::new("Character settings").open(&mut open).default_size([820.0, 680.0]).show(ctx, |ui| {
            reload = self.settings_editor.ui(ui, &self.engine, &self.lang);
        });
        self.show_settings = open;
        if reload {
            if let Some(engine) = Arc::get_mut(&mut self.engine) {
                if let Ok(lib) = chummer_core::settings::SettingsLibrary::load(&engine.store, chummer_core::settings::user_settings_dir().as_deref()) {
                    engine.settings = lib;
                }
            }
        }
        let mut open = self.show_about;
        egui::Window::new("About chummer-rs").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("chummer-rs {}", env!("CARGO_PKG_VERSION")));
            ui.label("A Rust rewrite of Chummer5a, the Shadowrun 5e character manager.");
            ui.label("Game data and translations come from Chummer5a (GPL-3.0).");
            ui.hyperlink("https://github.com/chummer5a/chummer5a");
        });
        self.show_about = open;
        if let Some(w) = self.wizard.as_mut() {
            match w.show(ctx, &self.engine) {
                wizard::WizardResult::Open => {}
                wizard::WizardResult::Cancel => self.wizard = None,
                wizard::WizardResult::Created(ch) => {
                    let mut v = CharacterView::new(*ch, &self.engine);
                    v.set_tab(view::Tab::Attributes);
                    self.views.push(v);
                    self.active = self.views.len() - 1;
                    self.wizard = None;
                    self.status = Some(("New character created. Spend your points, then Finish creation.".into(), false));
                }
            }
        }
        self.dialogs(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let recent: Vec<String> = self.recent.iter().map(|p| p.display().to_string()).collect();
        storage.set_string(RECENT_KEY, recent.join("\n"));
        storage.set_string(LANG_KEY, self.lang.code.clone());
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.selection.bg_fill = egui::Color32::from_rgb(0, 120, 105);
    visuals.hyperlink_color = ACCENT;
    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
    });
}

fn main() -> anyhow::Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut tab = None;
    let mut window: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tab" => tab = args.next().and_then(|t| view::Tab::parse(&t)),
            "--window" => window = args.next(),
            "--new" => window = Some("new".into()),
            "-h" | "--help" => {
                println!("usage: chummer-rs [--tab <info|attributes|skills|qualities|magic|equipment|improvements|karma|notes>] [--window <sources|browser|dice>] [file.chum5 ...]");
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
        let mut app = App::new(cc, engine, files, tab);
        match window.as_deref() {
            Some("sources") => app.show_sources = true,
            Some("browser") => app.show_browser = true,
            Some("dice") => app.show_dice = true,
            Some("new") => app.wizard = Some(wizard::Wizard::new()),
            Some("settings") => app.show_settings = true,
            _ => {}
        }
        Ok(Box::new(app))
    }))
        .map_err(|e| anyhow::anyhow!("{e}"))
}
