#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! chummer-rs desktop application.

mod ai_ui;
mod browser;
mod campaign_ui;
mod career_ui;
mod combo;
mod dice_ui;
mod doc;
mod drug_ui;
mod gm_screen;
mod gm_ui;
mod history_ui;
mod improvement_ui;
mod initiative;
mod lifestyle_ui;
mod magic_ui;
mod item_editor;
mod online;
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
mod workspace;
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
    /// The open campaign's GM screen (`gm_screen`).
    Campaign,
}

/// What the MDI tab strip selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mdi {
    Home(Home),
    Character(usize),
}

enum Pending {
    CloseTab(usize),
    CloseCampaign,
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
    /// The open campaign, shown as the GM Screen tab.
    gm: Option<gm_screen::GmScreen>,
    pdfs: SourcebookLibrary,
    sources_window: pdf_ui::SourcesWindow,
    browser: browser::DataBrowser,
    dice: dice_ui::DiceRoller,
    recent: Vec<PathBuf>,
    status: Option<(String, bool)>,
    pending: Option<Pending>,
    allow_close: bool,
    /// Layout and theme (View → Appearance).
    appearance: theme::Appearance,
    /// Online campaigns: the network node, joined campaigns, settings.
    online: online::Online,
    /// The Workspace layout's state.
    ws: workspace::Workspace,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, engine: Engine, files: Vec<PathBuf>, tab: Option<view::Tab>, look: (Option<theme::ThemeKind>, Option<theme::Layout>), join: Option<String>) -> Self {
        let mut appearance = theme::load_appearance();
        if let Some(k) = look.0 {
            appearance = appearance.with_kind(k);
        }
        if let Some(l) = look.1 {
            appearance = appearance.with_layout(l);
        }
        theme::apply(&cc.egui_ctx, &theme::Theme::of(appearance.kind()));
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
            gm: None,
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
            appearance,
            online: online::Online::new(),
            ws: Default::default(),
        };
        app.online.start_joined(&app.engine);
        if let Some(link) = join {
            app.online.join = Some((link, app.online.display_name()));
        }
        app.roster_folders = storage
            .and_then(|s| s.get_string(ROSTER_KEY))
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        app.roster = chummer_core::roster::scan(&app.roster_folders);
        if let Some(t) = storage.and_then(|s| s.get_string(workspace::POPOUTS_KEY)) {
            app.ws.pops.restore(&t);
        }
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

    /// View → Appearance: switch layout and theme, and remember them.
    fn set_appearance(&mut self, ctx: &egui::Context, a: theme::Appearance) {
        self.appearance = a;
        theme::apply(ctx, &theme::Theme::of(a.kind()));
        if let Err(e) = theme::save_appearance(&a) {
            self.status = Some((format!("Could not save the theme choice: {e}"), true));
        }
    }

    /// Guided creation on or off, for every open character and new ones.
    fn set_guided(&mut self, on: bool) {
        view::save_guided_preference(on);
        for v in &mut self.views {
            v.set_guided(on);
        }
    }

    fn open(&mut self, path: &Path) {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(chummer_core::campaign::EXTENSION)) {
            self.open_campaign(path);
            return;
        }
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
        let files = rfd::FileDialog::new()
            .add_filter("Chummer character", &["chum5", "chum5lz"])
            .add_filter("Raw Chummer5 Saves", &["chum5"])
            .add_filter("Compressed Chummer5 Saves", &["chum5lz"])
            .add_filter("All files", &["*"])
            .pick_files();
        for f in files.unwrap_or_default() {
            self.open(&f);
        }
    }

    fn save(&mut self, idx: usize, save_as: bool) -> bool {
        if self.views.get(idx).is_some_and(|v| v.campaign_member.is_some()) {
            // A campaign member is saved with its campaign.
            return self.save_campaign(false);
        }
        let Some(v) = self.views.get_mut(idx) else { return false };
        let path = match (save_as, v.path()) {
            (false, Some(p)) => Some(p),
            _ => {
                // Keep the current file's format; Chummer's Save As offers
                // both (`DialogFilter_Chum5` / `DialogFilter_Chum5lz`).
                let compressed = v.path().is_some_and(|p| chummer_core::chum5lz::is_chum5lz(&p));
                let (first, second) = if compressed { (("Compressed Chummer5 Saves", "chum5lz"), ("Raw Chummer5 Saves", "chum5")) } else { (("Raw Chummer5 Saves", "chum5"), ("Compressed Chummer5 Saves", "chum5lz")) };
                rfd::FileDialog::new()
                    .add_filter(first.0, &[first.1])
                    .add_filter(second.0, &[second.1])
                    .set_file_name(format!("{}.{}", v.ch().display_name(), first.1))
                    .save_file()
            }
        };
        let Some(path) = path else { return false };
        match v.save(&path) {
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
        if let Some(id) = self.views[idx].campaign_member.filter(|_| self.gm.is_some()) {
            // A campaign member: its changes stay in the GM screen.
            let doc = self.views.remove(idx).into_doc();
            if let Some(gm) = self.gm.as_mut() {
                gm.give_back(id, doc);
            }
            self.active = self.active.min(self.views.len().saturating_sub(1));
            self.home = Some(Home::Campaign);
            return;
        } else {
            if self.views[idx].ch().dirty && !force {
                self.pending = Some(Pending::CloseTab(idx));
                return;
            }
            self.views.remove(idx);
        }
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

    /// File → New Campaign.
    fn new_campaign(&mut self) {
        if !self.close_campaign(false) {
            return;
        }
        self.gm = Some(gm_screen::GmScreen::new_campaign(&self.lang.tr("New Campaign")));
        self.home = Some(Home::Campaign);
    }

    fn open_campaign_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("chummer-rs campaign", &[chummer_core::campaign::EXTENSION]).add_filter("All files", &["*"]).pick_file() {
            self.open_campaign(&p);
        }
    }

    fn open_campaign(&mut self, path: &Path) {
        if self.gm.as_ref().and_then(|g| g.path.as_deref()) == Some(path) {
            self.home = Some(Home::Campaign);
            return;
        }
        if !self.close_campaign(false) {
            self.status = Some(("Save or close the open campaign first.".into(), true));
            return;
        }
        match gm_screen::GmScreen::open(path, &self.engine, &mut self.online) {
            Ok(gm) => {
                self.gm = Some(gm);
                self.home = Some(Home::Campaign);
                self.remember(path);
                self.status = Some((format!("Opened {}", path.display()), false));
            }
            Err(e) => self.status = Some((e, true)),
        }
    }

    fn save_campaign(&mut self, save_as: bool) -> bool {
        let Some(gm) = self.gm.as_mut() else { return false };
        match gm.save(&mut self.views, save_as) {
            Ok(Some(p)) => {
                self.status = Some((format!("Saved {}", p.display()), false));
                self.remember(&p);
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.status = Some((e, true));
                false
            }
        }
    }

    /// Close the campaign and its members' tabs. With unsaved changes
    /// and no `force`, asks first and returns false.
    fn close_campaign(&mut self, force: bool) -> bool {
        let Some(gm) = &self.gm else { return true };
        if !force && gm.is_dirty(&self.views) {
            self.pending = Some(Pending::CloseCampaign);
            return false;
        }
        self.views.retain(|v| v.campaign_member.is_none());
        if let Some(gm) = self.gm.as_mut() {
            gm.close_online(&mut self.online);
        }
        self.gm = None;
        self.active = self.active.min(self.views.len().saturating_sub(1));
        if self.home == Some(Home::Campaign) || self.views.is_empty() {
            self.home = Some(Home::Roster);
        }
        true
    }

    /// Open a campaign member as a character tab (or bring its tab front).
    fn open_member(&mut self, id: chummer_core::campaign::MemberId) {
        if let Some(i) = self.views.iter().position(|v| v.campaign_member == Some(id)) {
            self.select(Mdi::Character(i));
            return;
        }
        let Some(doc) = self.gm.as_mut().and_then(|g| g.lend(id)) else { return };
        let mut v = CharacterView::from_doc(doc, &self.engine);
        v.campaign_member = Some(id);
        self.views.push(v);
        self.select(Mdi::Character(self.views.len() - 1));
    }

    /// Open a joined campaign's character (or bring its tab front).
    fn open_player(&mut self, campaign: usize, id: chummer_sync::CharacterId) {
        let Some(c) = self.online.joined.get(campaign) else { return };
        let key = (c.key.clone(), id.clone());
        if let Some(i) = self.views.iter().position(|v| v.doc().player_key().as_ref() == Some(&key)) {
            self.select(Mdi::Character(i));
            return;
        }
        let backend = doc::Backend::Player { session: c.session.clone(), id };
        match doc::Doc::online(backend, self.engine.clone()) {
            Some(d) => {
                self.views.push(CharacterView::from_doc(d, &self.engine));
                self.select(Mdi::Character(self.views.len() - 1));
            }
            None => self.status = Some(("The character has not arrived yet; try again in a moment.".into(), true)),
        }
    }

    /// Edit → Undo on the open character.
    fn undo(&mut self) {
        let engine = self.engine.clone();
        let Some(i) = self.current() else { return };
        if let Some(what) = self.views[i].undo(&engine) {
            self.status = Some((self.lang.tr_fmt("Undone: {0}", &[&what]), false));
        }
    }

    fn redo(&mut self) {
        let engine = self.engine.clone();
        let Some(i) = self.current() else { return };
        if let Some(what) = self.views[i].redo(&engine) {
            self.status = Some((self.lang.tr_fmt("Redone: {0}", &[&what]), false));
        }
    }

    /// Chummer's main menu: File, Edit, Tools, Special, View, Window, Help.
    fn menu(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| self.menus(ctx, ui));
    }

    /// The menus themselves; the Workspace shows them under its menu
    /// button.
    fn menus(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let has = self.current().is_some();
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
            ui.separator();
            if ui.button(self.lang.tr("New Campaign")).on_hover_text(self.lang.tr("A GM screen: players, NPCs, critters, initiative and damage")).clicked() {
                ui.close();
                self.new_campaign();
            }
            if ui.button(self.lang.tr("Open Campaign…")).clicked() {
                ui.close();
                self.open_campaign_dialog();
            }
            let has_gm = self.gm.is_some();
            if ui.add_enabled(has_gm, egui::Button::new(self.lang.tr("Save Campaign"))).clicked() {
                ui.close();
                self.save_campaign(false);
            }
            if ui.add_enabled(has_gm, egui::Button::new(self.lang.tr("Save Campaign As…"))).clicked() {
                ui.close();
                self.save_campaign(true);
            }
            if ui.add_enabled(has_gm, egui::Button::new(self.lang.tr("Close Campaign"))).clicked() {
                ui.close();
                self.close_campaign(false);
            }
            if ui.button(self.lang.tr("Join Campaign…")).on_hover_text(self.lang.tr("Play in a GM's online campaign with an invite link")).clicked() {
                ui.close();
                self.online.join = Some((String::new(), self.online.display_name()));
            }
            ui.separator();
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
        ui.menu_button(self.lang.tr("Edit"), |ui| {
            let doc = self.current().map(|i| self.views[i].doc());
            let undo = doc.and_then(|d| d.undo_label()).map(str::to_owned);
            let redo = doc.and_then(|d| d.redo_label()).map(str::to_owned);
            let online = doc.is_some_and(doc::Doc::is_online);
            let undo_text = undo.as_ref().map_or_else(|| self.lang.tr("Undo"), |w| self.lang.tr_fmt("Undo: {0}", &[w]));
            let redo_text = redo.as_ref().map_or_else(|| self.lang.tr("Redo"), |w| self.lang.tr_fmt("Redo: {0}", &[w]));
            let r = ui.add_enabled(undo.is_some(), egui::Button::new(undo_text).shortcut_text("Ctrl+Z"));
            let r = if online { r.on_disabled_hover_text(self.lang.tr(doc::ONLINE_UNDO)) } else { r };
            if r.clicked() {
                ui.close();
                self.undo();
            }
            if ui.add_enabled(redo.is_some(), egui::Button::new(redo_text).shortcut_text("Ctrl+Y")).clicked() {
                ui.close();
                self.redo();
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
            if ui.button(self.lang.tr("Online Settings…")).on_hover_text(self.lang.tr("Your name, relays and the mailbox for online campaigns")).clicked() {
                ui.close();
                self.online.show_settings = true;
            }
        });
        ui.menu_button(self.lang.tr("Special"), |ui| {
            let creating = self.current().is_some_and(|i| !self.views[i].ch().created);
            for (label, mode) in [("Add PACKS Kit…", gm_ui::PacksMode::Add), ("Create PACKS Kit…", gm_ui::PacksMode::Create)] {
                if ui.add_enabled(creating, egui::Button::new(self.lang.tr(label))).clicked() {
                    ui.close();
                    self.views[self.active].open_packs(mode);
                }
            }
        });
        ui.menu_button(self.lang.tr("View"), |ui| {
            if ui.add_enabled(has, egui::Button::new(self.lang.tr("History"))).on_hover_text(self.lang.tr("This session's changes to the character")).clicked() {
                ui.close();
                self.views[self.active].show_history();
            }
            let mut guided = view::guided_preference();
            if ui.checkbox(&mut guided, self.lang.tr("Guided creation")).on_hover_text(self.lang.tr("Walk through character creation one step at a time")).changed() {
                self.set_guided(guided);
            }
            ui.menu_button(self.lang.tr("Appearance"), |ui| {
                // Each layout with its themes under it.
                let a = self.appearance;
                for (layout, kinds) in [(theme::Layout::Classic, theme::ThemeKind::CLASSIC), (theme::Layout::Workspace, theme::ThemeKind::WORKSPACE)] {
                    if crate::combo::selectable_label(ui, a.layout == layout, crate::theme::strong(ui, self.lang.tr(layout.label()))).clicked() {
                        ui.close();
                        self.set_appearance(ctx, a.with_layout(layout));
                    }
                    ui.indent(layout.as_str(), |ui| {
                        for k in kinds {
                            if crate::combo::selectable_label(ui, a.kind() == k, self.lang.tr(k.label())).clicked() {
                                ui.close();
                                self.set_appearance(ctx, a.with_kind(k));
                            }
                        }
                    });
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
    }

    /// The MDI tabs: Master Index, Character Roster, then one per character.
    fn mdi_tabs(&self) -> Vec<(Mdi, String)> {
        let mut tabs = vec![(Mdi::Home(Home::MasterIndex), self.lang.tr("Master Index")), (Mdi::Home(Home::Roster), self.lang.tr("Character Roster"))];
        if let Some(gm) = &self.gm {
            tabs.push((Mdi::Home(Home::Campaign), format!("{} {}", crate::theme::glyph("🎭"), gm.title(&self.views))));
        }
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
        let mut close_gm = false;
        theme::strip_frame(ui, |ui| {
            for (m, label) in self.mdi_tabs() {
                let (r, closed) = theme::tab(ui, current == m, &label, matches!(m, Mdi::Character(_) | Mdi::Home(Home::Campaign)));
                if closed {
                    match m {
                        Mdi::Character(i) => close = Some(i),
                        _ => close_gm = true,
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
        if close_gm {
            self.close_campaign(false);
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
        } else if save && self.home == Some(Home::Campaign) {
            self.save_campaign(false);
        }
        if close && self.current().is_some() {
            self.close_tab(self.active, false);
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Undo/redo, unless a text box has the keyboard (it has its own).
        if self.current().is_some() && !ctx.wants_keyboard_input() {
            let (redo_shift, redo_y, undo) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                )
            });
            if redo_shift || redo_y {
                self.redo();
            } else if undo {
                self.undo();
            }
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
        let mut open_player = None;
        egui::SidePanel::left("roster_panel").resizable(true).default_width(460.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("roster").auto_shrink(false).show(ui, |ui| {
                if let Some(online::CampaignAction::Open(c, id)) = self.online.campaigns_ui(ui, &self.lang) {
                    open_player = Some((c, id));
                }
                ui.add_space(12.0);
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
                if ui.add(crate::theme::primary_button(ui, format!("{}  {}", crate::theme::glyph(crate::theme::glyph("✨")), self.lang.tr("Create New Character…")))).clicked() {
                    self.wizard = Some(wizard::Wizard::new());
                }
                if ui.button(format!("{}  {}", crate::theme::glyph(crate::theme::glyph("📂")), self.lang.tr("Open Character…"))).clicked() {
                    self.open_dialog();
                }
                ui.weak(self.lang.tr("or drop .chum5 or .chum5lz files onto this window"));
                ui.add_space(16.0);
                if self.pdfs.linked_count() == 0 && ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("📖")), self.lang.tr("Link your sourcebook PDFs…"))).clicked() {
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
        if let Some((c, id)) = open_player {
            self.open_player(c, id);
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
            if let Some(out) = rfd::FileDialog::new().set_file_name(format!("{}.{ext}", v.ch().display_name())).save_file() {
                self.status = Some(match chummer_core::export::export(v.ch(), &self.engine, &self.lang, &self.export_format, &out) {
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
            let xml = chummer_core::print::print_xml_with(v.ch(), &self.engine, &self.lang, opts);
            let name: String = v.ch().display_name().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
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
            Pending::CloseTab(i) => (Some(*i), self.views.get(*i).map(|v| v.ch().display_name()).unwrap_or_default()),
            Pending::Quit => (None, "your characters".to_owned()),
            Pending::CloseCampaign => (None, self.gm.as_ref().map(|g| g.campaign.name.clone()).unwrap_or_default()),
        };
        let campaign = matches!(p, Pending::CloseCampaign);
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
            (Some(0), None) if campaign => {
                if self.save_campaign(false) {
                    self.close_campaign(true);
                }
                self.pending = None;
            }
            (Some(1), None) if campaign => {
                self.close_campaign(true);
                self.pending = None;
            }
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
                let gm_ok = !self.gm.as_ref().is_some_and(|g| g.is_dirty(&self.views)) || self.save_campaign(false);
                let dirty: Vec<usize> = (0..self.views.len()).filter(|&i| self.views[i].ch().dirty && self.views[i].campaign_member.is_none()).collect();
                if gm_ok && dirty.into_iter().all(|i| self.save(i, false)) {
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
        let gm_dirty = self.gm.as_ref().is_some_and(|g| g.is_dirty(&self.views));
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && (gm_dirty || self.views.iter().any(|v| v.ch().dirty)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
        if self.appearance.layout == theme::Layout::Workspace {
            self.workspace_update(ctx);
        } else {
            self.classic_update(ctx);
        }
        self.windows(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(gm) = self.gm.as_mut() {
            gm.close_online(&mut self.online);
        }
        self.online.shutdown();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let recent: Vec<String> = self.recent.iter().map(|p| p.display().to_string()).collect();
        storage.set_string(RECENT_KEY, recent.join("\n"));
        storage.set_string(LANG_KEY, self.lang.code.clone());
        let folders: Vec<String> = self.roster_folders.iter().map(|p| p.display().to_string()).collect();
        storage.set_string(ROSTER_KEY, folders.join("\n"));
        storage.set_string(workspace::POPOUTS_KEY, self.ws_popouts_text());
    }
}

impl App {
    /// A frame of the Classic layout: menu, toolbar, MDI tabs, status
    /// strip and the selected tab.
    fn classic_update(&mut self, ctx: &egui::Context) {
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
            Mdi::Home(Home::Campaign) => {
                let engine = self.engine.clone();
                let action = match self.gm.as_mut() {
                    Some(gm) => gm.ui(ctx, &engine, &self.lang, &mut self.views, &mut self.status, &mut self.online, true),
                    None => {
                        self.home = Some(Home::Roster);
                        None
                    }
                };
                if let Some(gm_screen::Action::Open(id)) = action {
                    self.open_member(id);
                }
            }
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
                if self.views.get_mut(idx).is_some_and(CharacterView::take_guide_hidden) {
                    self.set_guided(false);
                }
            }
        }
    }

    /// The tool windows and dialogs of both layouts.
    fn windows(&mut self, ctx: &egui::Context) {
        let dice = workspace::PanelId::Dice;
        if !self.ws_out(dice) {
            let mut open = self.show_dice;
            let mut pop = false;
            egui::Window::new(self.lang.tr("Dice Roller")).id(egui::Id::new("dice_roller")).open(&mut open).default_width(360.0).show(ctx, |ui| {
                pop = self.ws_pop_button(ui);
                self.dice.ui(ui, &self.lang)
            });
            self.show_dice = open;
            self.ws_pop(ctx, dice, pop);
        }
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
        let initiative = workspace::PanelId::Initiative;
        if !self.ws_out(initiative) {
            let mut open = self.show_initiative;
            let mut pop = false;
            let chars = self.initiative_characters();
            egui::Window::new(self.lang.tr("Initiative tracker")).id(egui::Id::new("initiative_tracker")).open(&mut open).default_width(480.0).show(ctx, |ui| {
                pop = self.ws_pop_button(ui);
                self.initiative.ui(ui, &self.lang, &chars)
            });
            self.show_initiative = open;
            self.ws_pop(ctx, initiative, pop);
        }
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
                    // Guided: the guide already opened its first step.
                    if !v.guided() {
                        v.set_tab(view::Tab::Common);
                    }
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
        let engine = self.engine.clone();
        self.online.windows(ctx, &engine, &self.lang, &mut self.status);
        self.dialogs(ctx);
    }

    /// The open characters' initiative, for the tracker.
    fn initiative_characters(&self) -> Vec<(String, i32, u32)> {
        self.views.iter().map(|v| (v.ch().display_name(), v.sheet.initiative, v.sheet.initiative_dice.max(1) as u32)).collect()
    }
}

/// `file:///home/x/My%20Runner.chum5` → `/home/x/My Runner.chum5`.
fn url_to_path(url: &str) -> PathBuf {
    let rest = url.trim_start_matches("file://");
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

fn main() -> anyhow::Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut tab = None;
    let mut window: Option<String> = None;
    let mut theme_arg = None;
    let mut layout_arg = None;
    let mut join = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tab" => tab = args.next().and_then(|t| view::Tab::parse(&t)),
            "--window" => window = args.next(),
            "--theme" => theme_arg = args.next().and_then(|t| theme::ThemeKind::parse(&t)),
            "--layout" => layout_arg = args.next().and_then(|t| theme::Layout::parse(&t)),
            "--new" => window = Some("new".into()),
            "-h" | "--help" => {
                println!("usage: chummer-rs [chummer-rs://join/... invite link] [--tab <common|skills|limits|martial|spells|adept|complex|critter|initiation|cyberware|street|vehicles|character|karma|calendar|game|improvements|relationships>] [--window <sources|browser|dice>] [--layout <classic|workspace>] [--theme <classic|graphite|dark|light>] [file.chum5|file.chum5lz|file.chummercampaign ...]");
                return Ok(());
            }
            // An invite link (the chummer-rs:// handler passes it as an argument).
            _ if a.starts_with("chummer-rs://") => join = Some(a),
            // Desktop launchers with %U pass files as file:// URLs.
            _ if a.starts_with("file://") => files.push(url_to_path(&a)),
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
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("chummer-rs")
            .with_app_id("chummer-rs")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    if let Some(icon) = workspace::app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    eframe::run_native("chummer-rs", options, Box::new(move |cc| {
        let mut app = App::new(cc, engine, files, tab, (theme_arg, layout_arg), join);
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

#[cfg(test)]
mod url_tests {
    #[test]
    fn file_urls_become_paths() {
        assert_eq!(super::url_to_path("file:///home/x/My%20Runner.chum5"), std::path::PathBuf::from("/home/x/My Runner.chum5"));
        assert_eq!(super::url_to_path("file:///a/b%"), std::path::PathBuf::from("/a/b%"));
    }
}
