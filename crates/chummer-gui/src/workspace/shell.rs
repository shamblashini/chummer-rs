//! The Workspace shell: top bar (logo, menu, open documents, search,
//! sync state), sidebar (sections, undo/redo, settings, dark/light),
//! budget strip, page, inspector, status bar, pop-out windows and the
//! command palette. `App::workspace_update` draws a frame; the tool
//! windows and dialogs are shared with Classic (`App::windows`).

use chummer_core::campaign::MemberKind;
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke};

use super::palette::{self, Cmd, Entry, Kind, Target};
use super::popout::{self, Panel, PopKey};
use super::widgets::{self, Look};
use super::{icons, DocKey, NavGroup, NavItem, PanelId, Section};
use crate::theme::{self, Layout, ThemeKind};
use crate::view::CharacterView;
use crate::{App, Home, Mdi};

const SIDEBAR_WIDTH: f32 = 200.0;
/// The undo/redo/settings/theme row under the sidebar, with its line.
const CLUSTER_HEIGHT: f32 = 43.0;

/// A tab of the top bar.
struct DocTab {
    doc: DocKey,
    icon: &'static str,
    name: String,
    /// Small text after the name ("Creation", "Career").
    note: String,
    closable: bool,
}

impl App {
    /// A frame of the Workspace layout.
    pub(crate) fn workspace_update(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            if self.ws.palette.open {
                self.ws.palette.close();
            } else {
                self.ws.palette.show();
            }
        }
        self.shortcuts(ctx);
        self.ws_load_logo(ctx);
        let open = self.ws_docs();
        self.ws.pops.retain_docs(|d| open.contains(&d));
        let doc = self.ws_doc();
        if doc == DocKey::Campaign {
            let engine = self.engine.clone();
            if let Some(gm) = self.gm.as_mut() {
                gm.begin_frame(&engine, &mut self.views, &mut self.online);
            }
        }
        self.ws_top_bar(ctx, doc);
        self.ws_status_bar(ctx, doc);
        self.ws_sidebar(ctx, doc);
        match doc {
            DocKey::Character(id) => self.ws_character(ctx, id),
            DocKey::Campaign => self.ws_campaign(ctx),
            DocKey::Home => self.ws_home(ctx),
        }
        self.ws_popped(ctx);
        self.ws_palette(ctx);
    }

    // ----- documents -----

    /// Every open document.
    fn ws_docs(&self) -> Vec<DocKey> {
        let mut out = vec![DocKey::Home];
        if self.gm.is_some() {
            out.push(DocKey::Campaign);
        }
        out.extend(self.views.iter().map(|v| DocKey::Character(v.ws_id())));
        out
    }

    /// The document in front.
    fn ws_doc(&self) -> DocKey {
        match self.mdi() {
            Mdi::Home(Home::Campaign) => DocKey::Campaign,
            Mdi::Home(_) => DocKey::Home,
            Mdi::Character(i) => DocKey::Character(self.views[i].ws_id()),
        }
    }

    fn ws_index(&self, id: u64) -> Option<usize> {
        self.views.iter().position(|v| v.ws_id() == id)
    }

    /// Bring a document to the front.
    fn ws_select(&mut self, d: DocKey) {
        match d {
            DocKey::Home => self.home = Some(if self.ws.home == Some(Section::DataBrowser) { Home::MasterIndex } else { Home::Roster }),
            DocKey::Campaign => self.home = Some(Home::Campaign),
            DocKey::Character(id) => {
                if let Some(i) = self.ws_index(id) {
                    self.select(Mdi::Character(i));
                }
            }
        }
    }

    fn ws_tabs(&self) -> Vec<DocTab> {
        let mut out = vec![DocTab { doc: DocKey::Home, icon: icons::HOUSE, name: self.lang.tr("Home"), note: String::new(), closable: false }];
        if let Some(gm) = &self.gm {
            out.push(DocTab { doc: DocKey::Campaign, icon: icons::USERS_THREE, name: gm.title(&self.views), note: String::new(), closable: true });
        }
        for v in &self.views {
            let mut name = v.doc().display_name();
            if v.ch().dirty {
                name.push_str(" •");
            }
            let note = self.lang.tr(if v.ch().created { "Career" } else { "Creation" });
            out.push(DocTab { doc: DocKey::Character(v.ws_id()), icon: icons::USER, name, note, closable: true });
        }
        out
    }

    /// Close a document's tab (asks first when it has unsaved changes).
    fn ws_close(&mut self, d: DocKey) {
        match d {
            DocKey::Home => {}
            DocKey::Campaign => {
                self.close_campaign(false);
            }
            DocKey::Character(id) => {
                if let Some(i) = self.ws_index(id) {
                    self.close_tab(i, false);
                }
            }
        }
    }

    fn ws_load_logo(&mut self, ctx: &egui::Context) {
        if self.ws.logo.is_some() {
            return;
        }
        if let Ok(img) = image::load_from_memory(include_bytes!("../../assets/logo/chummer-rs-64.png")) {
            let img = img.into_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            self.ws.logo = Some(ctx.load_texture("chummer-rs logo", image, egui::TextureOptions::LINEAR));
        }
    }

    // ----- sections -----

    /// The section the document shows.
    fn ws_section(&self, doc: DocKey) -> Section {
        match doc {
            DocKey::Home => {
                if self.home == Some(Home::MasterIndex) {
                    Section::DataBrowser
                } else {
                    Section::Home
                }
            }
            DocKey::Campaign => Section::Campaign,
            DocKey::Character(id) => self.ws_index(id).map_or(Section::Page(crate::view::Tab::Common), |i| self.views[i].ws_current(self.ws.special.get(&id).copied())),
        }
    }

    /// Go to a section of a document.
    fn ws_go(&mut self, doc: DocKey, s: Section) {
        match doc {
            DocKey::Home => {
                self.home = Some(if s == Section::DataBrowser { Home::MasterIndex } else { Home::Roster });
                self.ws.home = Some(s);
            }
            DocKey::Campaign => {}
            DocKey::Character(id) => {
                let Some(i) = self.ws_index(id) else { return };
                self.views[i].ws_go(s);
                if matches!(s, Section::Play | Section::History) {
                    self.ws.special.insert(id, (s, self.views[i].ws_tab()));
                } else {
                    self.ws.special.remove(&id);
                }
            }
        }
    }

    /// The sidebar's sections for a document.
    fn ws_nav(&self, doc: DocKey) -> Vec<NavGroup> {
        let item = |s: Section| NavItem { section: s, label: self.lang.tr(s.label()), badge: None };
        match doc {
            DocKey::Home => vec![NavGroup { title: self.lang.tr("Library"), items: vec![item(Section::Home)] }, NavGroup { title: self.lang.tr("Reference"), items: vec![item(Section::DataBrowser)] }],
            DocKey::Campaign => vec![NavGroup { title: self.lang.tr("Campaign"), items: vec![item(Section::Campaign)] }],
            DocKey::Character(id) => self.ws_index(id).map(|i| self.views[i].ws_nav(&self.lang)).unwrap_or_default(),
        }
    }

    // ----- top bar -----

    fn ws_top_bar(&mut self, ctx: &egui::Context, doc: DocKey) {
        let ws = theme::current(ctx).ws;
        let mut pick = None;
        let mut close = None;
        let mut run = None;
        egui::TopBottomPanel::top("ws_top").exact_height(40.0).frame(egui::Frame::new().fill(ws.chrome).inner_margin(Margin::symmetric(10, 0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let start = ui.cursor().min.x;
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(tex) = &self.ws.logo {
                    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(24.0, 24.0)));
                }
                ui.label(RichText::new("chummer-rs").font(widgets::bold(13.5)).color(ws.text));
                let r = widgets::icon_button(ui, icons::LIST, 26.0).on_hover_text(self.lang.tr("Menu"));
                egui::Popup::menu(&r).show(|ui| self.menus(ctx, ui));
                let used = ui.cursor().min.x - start;
                if used < 190.0 {
                    ui.add_space(190.0 - used);
                }
                ui.spacing_mut().item_spacing.x = 2.0;
                for t in self.ws_tabs() {
                    let (r, closed) = doc_tab(ui, &t, t.doc == doc, &self.lang.tr("Close"));
                    if closed {
                        close = Some(t.doc);
                    } else if r.clicked() {
                        pick = Some(t.doc);
                    }
                }
                ui.add_space(4.0);
                let r = widgets::icon_button(ui, icons::PLUS, 24.0).on_hover_text(self.lang.tr("Open or create"));
                egui::Popup::menu(&r).show(|ui| {
                    for c in [Cmd::NewCharacter, Cmd::Open, Cmd::NewCritter, Cmd::NewCampaign, Cmd::OpenCampaign, Cmd::JoinCampaign] {
                        if ui.add(egui::Button::new(format!("{}  {}", c.icon(), self.lang.tr(c.label()))).shortcut_text(c.shortcut())).clicked() {
                            run = Some(c);
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (glyph, text, color) = self.ws_sync_state(doc);
                    ui.label(RichText::new(text).size(12.0).color(color));
                    ui.label(icons::icon(glyph, 14.0, color));
                    ui.add_space(4.0);
                    if search_field(ui, &self.lang.tr("Search or run a command")).clicked() {
                        self.ws.palette.show();
                    }
                });
            });
        });
        if let Some(d) = close {
            self.ws_close(d);
        } else if let Some(d) = pick {
            self.ws_select(d);
        }
        if let Some(c) = run {
            self.ws_run(ctx, c);
        }
    }

    /// The top bar's sync line: an icon, text and colour.
    fn ws_sync_state(&self, doc: DocKey) -> (&'static str, String, Color32) {
        let ws = theme::Theme::of(self.appearance.kind()).ws;
        match doc {
            DocKey::Character(id) => {
                let Some(v) = self.ws_index(id).map(|i| &self.views[i]) else { return (icons::FILE, String::new(), ws.muted) };
                match v.doc().sync_state() {
                    Some(s) if s.refused > 0 => (icons::WARNING, self.lang.tr_fmt("{0} refused", &[&s.refused]), ws.warning),
                    Some(s) if s.pending > 0 => (icons::CLOUD_ARROW_UP, self.lang.tr_fmt("Sending {0}", &[&s.pending]), ws.accent),
                    Some(s) if matches!(s.mode, Some(chummer_sync::SyncMode::Offline) | None) => (icons::CLOUD_SLASH, self.lang.tr("Offline"), ws.muted),
                    Some(_) => (icons::CLOUD_CHECK, self.lang.tr("Synced"), ws.accent),
                    None if v.campaign_member.is_some() => (icons::USERS_THREE, self.lang.tr("Campaign"), ws.muted),
                    None => {
                        let state = if v.path().is_none() {
                            self.lang.tr("not saved yet")
                        } else if v.ch().dirty {
                            self.lang.tr("unsaved changes")
                        } else {
                            self.lang.tr("saved")
                        };
                        (icons::FLOPPY_DISK, format!("{} · {state}", self.lang.tr("Local file")), ws.muted)
                    }
                }
            }
            DocKey::Campaign => (icons::USERS_THREE, self.gm.as_ref().map(|g| g.title(&self.views)).unwrap_or_default(), ws.muted),
            DocKey::Home => (icons::INFO, format!("chummer-rs {}", env!("CARGO_PKG_VERSION")), ws.muted),
        }
    }

    // ----- status bar -----

    fn ws_status_bar(&mut self, ctx: &egui::Context, doc: DocKey) {
        let ws = theme::current(ctx).ws;
        egui::TopBottomPanel::bottom("ws_status").exact_height(24.0).frame(egui::Frame::new().fill(ws.chrome).inner_margin(Margin::symmetric(12, 0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let small = |t: String, c: Color32| RichText::new(t).size(11.5).color(c);
                let (left, right) = match doc {
                    DocKey::Character(id) => match self.ws_index(id) {
                        Some(i) => {
                            let v = &self.views[i];
                            let file = v.path().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string())).unwrap_or_else(|| self.lang.tr("not saved yet"));
                            let state = if v.ch().dirty { self.lang.tr("unsaved changes") } else { self.lang.tr("saved") };
                            (format!("{file} · {state}"), v.ws_settings_name())
                        }
                        None => (String::new(), String::new()),
                    },
                    DocKey::Campaign => (self.gm.as_ref().and_then(|g| g.path.as_ref()).and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), String::new()),
                    DocKey::Home => (String::new(), format!("chummer-rs {}", env!("CARGO_PKG_VERSION"))),
                };
                if !left.is_empty() {
                    ui.label(small(left, ws.muted));
                }
                match &self.status {
                    Some((msg, true)) => ui.label(small(msg.clone(), ws.error)),
                    Some((msg, false)) => ui.label(small(msg.clone(), ws.muted)),
                    None => ui.label(small(self.lang.tr("Ready"), ws.muted)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(small(right, ws.muted));
                });
            });
        });
    }

    // ----- sidebar -----

    fn ws_sidebar(&mut self, ctx: &egui::Context, doc: DocKey) {
        let ws = theme::current(ctx).ws;
        let current = self.ws_section(doc);
        let nav = self.ws_nav(doc);
        let mut go = None;
        let mut open_member = None;
        let mut sources = false;
        // The GM screen's roster is a little wider.
        let width = if doc == DocKey::Campaign { 220.0 } else { SIDEBAR_WIDTH };
        let mut gm_action = None;
        egui::SidePanel::left("ws_sidebar").exact_width(width).resizable(false).frame(egui::Frame::new().fill(ws.chrome)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
            if let (DocKey::Campaign, Some(gm)) = (doc, self.gm.as_mut()) {
                let height = (ui.available_height() - CLUSTER_HEIGHT).max(40.0);
                let engine = self.engine.clone();
                let mut env = gm_env(&engine, &self.lang, &mut self.views, &mut self.status, &mut self.online, &mut self.ws.pops);
                gm_action = gm.ws_sidebar(ui, &mut env, height);
                rule(ui, ws.divider);
                self.ws_cluster(ctx, ui);
                return;
            }
            self.ws_sidebar_header(ui, doc);
            // The sections scroll; the cluster stays at the bottom.
            let nav_height = (ui.available_height() - CLUSTER_HEIGHT).max(40.0);
            {
                {
                    egui::ScrollArea::vertical().id_salt("ws_nav").auto_shrink(false).max_height(nav_height).show(ui, |ui| {
                        egui::Frame::new().inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                            for g in &nav {
                                if g.items.is_empty() {
                                    continue;
                                }
                                widgets::nav_heading(ui, &g.title);
                                for it in &g.items {
                                    if widgets::nav_item(ui, it.section == current, it.section.icon(), &it.label, it.badge).clicked() {
                                        go = Some(it.section);
                                    }
                                }
                            }
                            match doc {
                                DocKey::Home => {
                                    if widgets::nav_item(ui, false, icons::BOOK_OPEN, &self.lang.tr("Sourcebooks (PDFs)…"), None).clicked() {
                                        sources = true;
                                    }
                                }
                                DocKey::Campaign => {
                                    if let Some(gm) = &self.gm {
                                        widgets::nav_heading(ui, &self.lang.tr("Members"));
                                        for m in &gm.campaign.members {
                                            if widgets::nav_item(ui, false, member_icon(&m.kind), &m.name, None).on_hover_text(self.lang.tr("Open")).clicked() {
                                                open_member = Some(m.id);
                                            }
                                        }
                                    }
                                }
                                DocKey::Character(_) => {}
                            }
                            ui.add_space(6.0);
                        });
                    });
                }
            }
            rule(ui, ws.divider);
            self.ws_cluster(ctx, ui);
        });
        if let Some(s) = go {
            self.ws_go(doc, s);
        }
        if let Some(id) = open_member {
            self.open_member(id);
        }
        if let Some(crate::gm_screen::Action::Open(id)) = gm_action {
            self.open_member(id);
        }
        if sources {
            self.show_sources = true;
        }
    }

    /// The sidebar header: who or what the document is.
    fn ws_sidebar_header(&mut self, ui: &mut egui::Ui, doc: DocKey) {
        let ws = theme::ws(ui);
        let (avatar, name, about, mode) = match doc {
            DocKey::Character(id) => match self.ws_index(id) {
                Some(i) => {
                    let h = self.views[i].ws_header(&self.lang);
                    (h.initials, h.name, h.about, h.mode)
                }
                None => return,
            },
            DocKey::Campaign => match &self.gm {
                Some(gm) => (icons::USERS_THREE.to_owned(), gm.title(&self.views), self.lang.tr_fmt("{0} members", &[&gm.campaign.members.len()]), self.lang.tr("GM Screen")),
                None => return,
            },
            DocKey::Home => (icons::HOUSE.to_owned(), self.lang.tr("Home"), self.lang.tr("Characters, campaigns and game data"), String::new()),
        };
        let mut pop = false;
        egui::Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(6), ws.selection);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &avatar, widgets::bold(12.0), ws.accent);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.set_width(ui.available_width() - 26.0);
                    ui.add(egui::Label::new(RichText::new(&name).font(widgets::bold(13.5)).color(ws.text)).truncate());
                    if !about.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&about).size(11.5).color(ws.muted)).truncate());
                    }
                    if !mode.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&mode).size(11.0).color(ws.accent)).truncate());
                    }
                });
                if doc == DocKey::Campaign {
                    let key = PopKey::new(DocKey::Campaign, PanelId::Activity);
                    let out = self.ws.pops.is_out(key);
                    let (glyph, tip) = if out { (icons::ARROW_SQUARE_IN, self.lang.tr("Dock back")) } else { (icons::ARROW_SQUARE_OUT, self.lang.tr("Pop out the activity feed")) };
                    pop = widgets::icon_button(ui, glyph, 22.0).on_hover_text(tip).clicked();
                }
            });
        });
        rule(ui, ws.divider);
        if pop {
            self.ws.pops.toggle(PopKey::new(DocKey::Campaign, PanelId::Activity), ui.ctx());
        }
    }

    /// The bottom-left cluster: Undo, Redo, Settings and the dark/light
    /// switch.
    fn ws_cluster(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut run = None;
        egui::Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let doc = self.current().map(|i| self.views[i].doc());
                let undo = doc.and_then(|d| d.undo_label()).map(str::to_owned);
                let redo = doc.and_then(|d| d.redo_label()).map(str::to_owned);
                let online = doc.is_some_and(crate::doc::Doc::is_online);
                let r = ui.add_enabled_ui(undo.is_some(), |ui| widgets::icon_button(ui, icons::ARROW_U_UP_LEFT, 26.0)).inner;
                let r = match &undo {
                    Some(w) => r.on_hover_text(format!("{}  (Ctrl+Z)", self.lang.tr_fmt("Undo: {0}", &[w]))),
                    None if online => r.on_disabled_hover_text(self.lang.tr(crate::doc::ONLINE_UNDO)),
                    None => r.on_disabled_hover_text(self.lang.tr("Undo")),
                };
                if r.clicked() {
                    run = Some(Cmd::Undo);
                }
                let r = ui.add_enabled_ui(redo.is_some(), |ui| widgets::icon_button(ui, icons::ARROW_U_UP_RIGHT, 26.0)).inner;
                let r = match &redo {
                    Some(w) => r.on_hover_text(format!("{}  (Ctrl+Y)", self.lang.tr_fmt("Redo: {0}", &[w]))),
                    None => r.on_disabled_hover_text(self.lang.tr("Redo")),
                };
                if r.clicked() {
                    run = Some(Cmd::Redo);
                }
                let r = widgets::icon_button(ui, icons::GEAR_SIX, 26.0).on_hover_text(self.lang.tr("Settings"));
                egui::Popup::menu(&r).show(|ui| {
                    for c in [Cmd::CharacterSettings, Cmd::Sourcebooks, Cmd::OnlineSettings] {
                        if ui.button(format!("{}  {}", c.icon(), self.lang.tr(c.label()))).clicked() {
                            run = Some(c);
                        }
                    }
                    ui.separator();
                    let mut guided = crate::view::guided_preference();
                    if widgets::check(ui, &mut guided, &self.lang.tr("Guided creation")).changed() {
                        run = Some(Cmd::GuidedCreation);
                    }
                    if ui.button(format!("{}  {}", icons::CIRCLE_HALF, self.lang.tr("Classic layout"))).on_hover_text(self.lang.tr("View → Appearance")).clicked() {
                        run = Some(Cmd::ClassicLayout);
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let light = self.appearance.kind() == ThemeKind::WorkspaceLight;
                    let dark_tip = self.lang.tr("Dark");
                    let light_tip = self.lang.tr("Light");
                    if let Some(i) = widgets::segmented(ui, &[(icons::MOON, &dark_tip), (icons::SUN, &light_tip)], usize::from(light), 26.0) {
                        run = Some(if i == 0 { Cmd::Dark } else { Cmd::Light });
                    }
                });
            });
        });
        if let Some(c) = run {
            self.ws_run(ctx, c);
        }
    }

    // ----- a character -----

    fn ws_character(&mut self, ctx: &egui::Context, id: u64) {
        let Some(i) = self.ws_index(id) else { return };
        let ws = theme::current(ctx).ws;
        let engine = self.engine.clone();
        let mut changed = self.views[i].ws_begin();
        let mut roll = None;
        // Budget strip.
        let chips = self.views[i].ws_budgets(&self.lang);
        egui::TopBottomPanel::top("ws_budget").exact_height(44.0).frame(egui::Frame::new().fill(ws.ground).inner_margin(Margin::symmetric(16, 0))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                for c in &chips {
                    widgets::budget_chip(ui, &c.label, &c.value, c.fill, c.tone);
                }
            });
        });
        // Inspector.
        egui::SidePanel::right("ws_inspector").default_width(320.0).min_width(240.0).max_width(560.0).resizable(true).frame(egui::Frame::new().fill(ws.chrome)).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("ws_inspector_scroll").auto_shrink(false).show(ui, |ui| {
                changed |= self.ws_inspector(ui, i, &mut roll);
            });
        });
        // The page.
        let section = self.ws_section(DocKey::Character(id));
        let key = PopKey::new(DocKey::Character(id), PanelId::Section(section));
        let mut toggle = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 8 })).show(ctx, |ui| {
            let out = self.ws.pops.is_out(key);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.lang.tr(section.label())).font(widgets::bold(17.0)).color(ws.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (glyph, tip) = if out { (icons::ARROW_SQUARE_IN, self.lang.tr("Dock back")) } else { (icons::ARROW_SQUARE_OUT, self.lang.tr("Pop out into its own window")) };
                    toggle = widgets::icon_button(ui, glyph, 24.0).on_hover_text(tip).clicked();
                });
            });
            ui.add_space(8.0);
            if out {
                widgets::card_frame(&ws).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    toggle |= popout::placeholder(ui, &self.lang);
                });
            } else {
                let page = ui.max_rect();
                changed |= widgets::clip_to(ui, page, |ui| self.views[i].ws_page(ui, section, &engine, &self.lang, &self.pdfs, &mut self.status, &mut roll, &mut self.ws.pops));
            }
        });
        if toggle {
            self.ws.pops.toggle(key, ctx);
        }
        self.views[i].ws_end(ctx, &engine, &self.lang, &self.pdfs, &mut self.status, changed);
        self.ws_after_character(ctx, i, roll);
    }

    /// What a character page asks the app for: the dice roller, opening
    /// a linked character, turning the guide off.
    fn ws_after_character(&mut self, ctx: &egui::Context, i: usize, roll: Option<u32>) {
        if let Some(pool) = roll {
            self.dice.set_pool(pool);
            self.show_dice = true;
        }
        if let Some(p) = crate::relationships_ui::take_open_request(ctx) {
            self.open(&p);
        }
        if self.views.get_mut(i).is_some_and(CharacterView::take_guide_hidden) {
            self.set_guided(false);
        }
    }

    /// The inspector's sections for character `i`. Returns true if the
    /// character changed.
    fn ws_inspector(&mut self, ui: &mut egui::Ui, i: usize, roll: &mut Option<u32>) -> bool {
        let engine = self.engine.clone();
        let doc = DocKey::Character(self.views[i].ws_id());
        let key = |p| PopKey::new(doc, p);
        let lang = &self.lang;
        let mut changed = false;
        let v = &mut self.views[i];
        if v.ws_creating() {
            let (errors, warnings) = v.ws_issue_counts();
            Panel::inspector(key(PanelId::Issues), &lang.tr("Issues")).show(
                ui,
                &mut self.ws.pops,
                lang,
                |ui| {
                    if warnings > 0 {
                        widgets::badge(ui, theme::Badge { count: warnings, error: false });
                    }
                    if errors > 0 {
                        widgets::badge(ui, theme::Badge { count: errors, error: true });
                    }
                },
                |ui| v.ws_issues(ui, lang),
            );
        }
        if v.ws_has_item() {
            let mut close = false;
            Panel::inspector(key(PanelId::Item), &lang.tr("Item")).show(
                ui,
                &mut self.ws.pops,
                lang,
                |ui| close = widgets::icon_button(ui, icons::X, 22.0).on_hover_text(lang.tr("Close")).clicked(),
                |ui| changed |= v.ws_item(ui, &engine, lang, &mut self.status),
            );
            if close {
                v.ws_close_item();
            }
        }
        if self.ws.special.get(&v.ws_id()).is_some_and(|(s, t)| *s == Section::Play && *t == v.ws_tab()) {
            // At the table: the character's dice roller and its rolls.
            return v.ws_play_inspector(ui, lang, &mut self.status, &mut self.ws.pops) || changed;
        }
        let title = if v.ws_creating() { lang.tr("Karma Summary") } else { lang.tr("Other Info") };
        Panel::inspector(key(PanelId::Summary), &title).show(ui, &mut self.ws.pops, lang, |_| {}, |ui| changed |= v.ws_summary(ui, lang, roll));
        let undo = v.doc().undo_label().map(str::to_owned);
        let mut do_undo = false;
        Panel::inspector(key(PanelId::Recent), &lang.tr("History")).show(
            ui,
            &mut self.ws.pops,
            lang,
            |ui| {
                if let Some(w) = &undo {
                    do_undo = widgets::button(ui, Some(icons::ARROW_U_UP_LEFT), &lang.tr("Undo"), Look::Ghost, 22.0).on_hover_text(lang.tr_fmt("Undo: {0}", &[w])).clicked();
                }
            },
            |ui| changed |= v.ws_history(ui, lang),
        );
        if do_undo {
            self.undo();
        }
        changed
    }

    // ----- campaign and home -----

    fn ws_campaign(&mut self, ctx: &egui::Context) {
        let engine = self.engine.clone();
        let action = match self.gm.as_mut() {
            Some(gm) => gm.ws_ui(ctx, &mut gm_env(&engine, &self.lang, &mut self.views, &mut self.status, &mut self.online, &mut self.ws.pops)),
            None => {
                self.home = Some(Home::Roster);
                None
            }
        };
        if let Some(crate::gm_screen::Action::Open(id)) = action {
            self.open_member(id);
        }
    }

    fn ws_home(&mut self, ctx: &egui::Context) {
        if self.home == Some(Home::MasterIndex) {
            let ws = theme::current(ctx).ws;
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(Margin::symmetric(16, 12))).show(ctx, |ui| {
                ui.label(RichText::new(self.lang.tr("Master Index")).font(widgets::bold(17.0)).color(ws.text));
                ui.add_space(8.0);
                self.browser.ui(ui, &self.engine.store, &self.lang, &self.pdfs, &mut self.status);
            });
        } else {
            self.welcome(ctx);
        }
    }

    // ----- pop-outs -----

    /// The title of a popped-out panel's window.
    fn ws_panel_title(&self, key: PopKey) -> String {
        let panel = match key.panel {
            PanelId::Section(s) => self.lang.tr(s.label()),
            PanelId::Issues => self.lang.tr("Issues"),
            PanelId::Item => self.lang.tr("Item"),
            PanelId::Summary => self.lang.tr("Summary"),
            PanelId::Recent => self.lang.tr("History"),
            PanelId::Condition => self.lang.tr("Condition Monitor"),
            PanelId::Dice => self.lang.tr("Dice Roller"),
            PanelId::Initiative => self.lang.tr("Initiative tracker"),
            PanelId::Activity => self.lang.tr("Activity"),
            PanelId::Play(p) => self.lang.tr(p.title()),
            PanelId::Gm(p) => self.lang.tr(p.title()),
        };
        let doc = match key.doc {
            DocKey::Character(id) => self.ws_index(id).map(|i| self.views[i].doc().display_name()),
            DocKey::Campaign => self.gm.as_ref().map(|g| g.campaign.name.clone()),
            DocKey::Home => None,
        };
        match doc {
            Some(d) if !d.is_empty() => format!("{d} · {panel}"),
            _ => panel,
        }
    }

    /// Draw every popped-out panel in its window.
    fn ws_popped(&mut self, ctx: &egui::Context) {
        if self.ws.pops.is_empty() {
            return;
        }
        let keys = self.ws.pops.keys();
        let dock = self.lang.tr("Dock back");
        let icon = super::app_icon();
        let active = self.ws_doc();
        let mut seen: Vec<DocKey> = Vec::new();
        for key in keys {
            let shown = match key.panel {
                PanelId::Dice => self.show_dice,
                PanelId::Initiative => self.show_initiative,
                _ => true,
            };
            if !shown {
                self.ws.pops.dock(key);
                continue;
            }
            // A background character's first window runs its frame
            // (taking what arrived, its dialogs, recomputing).
            let own_frame = key.doc != active && !seen.contains(&key.doc);
            seen.push(key.doc);
            let title = self.ws_panel_title(key);
            let at = self.ws.pops.origin(key);
            let docked = popout::window(ctx, key, &title, at, icon.clone(), &dock, |vctx, ui| self.ws_panel(vctx, ui, key, own_frame));
            if docked {
                self.ws.pops.dock(key);
                // The main window was drawn with the placeholder.
                ctx.request_repaint();
            }
        }
    }

    /// A popped-out panel's contents.
    fn ws_panel(&mut self, vctx: &egui::Context, ui: &mut egui::Ui, key: PopKey, own_frame: bool) {
        let engine = self.engine.clone();
        let scroll = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
            egui::ScrollArea::vertical().id_salt("popout scroll").auto_shrink(false).show(ui, |ui| f(ui));
        };
        match key.doc {
            DocKey::Character(id) => {
                let Some(i) = self.ws_index(id) else { return };
                let mut changed = own_frame && self.views[i].ws_begin();
                let mut roll = None;
                let lang = &self.lang;
                let v = &mut self.views[i];
                match key.panel {
                    PanelId::Section(s) => {
                        // A popped Street Gear sub-tab must not move the
                        // main window's page to it.
                        let gear = v.ws_gear_tab();
                        changed |= v.ws_page(ui, s, &engine, lang, &self.pdfs, &mut self.status, &mut roll, &mut self.ws.pops);
                        v.ws_set_gear_tab(gear);
                    }
                    PanelId::Condition => scroll(ui, &mut |ui| changed |= v.ws_play_panel(ui, crate::view::play::Panel::Condition, lang, &mut self.status)),
                    PanelId::Play(p) => scroll(ui, &mut |ui| changed |= v.ws_play_panel(ui, p, lang, &mut self.status)),
                    PanelId::Issues => scroll(ui, &mut |ui| v.ws_issues(ui, lang)),
                    PanelId::Item => scroll(ui, &mut |ui| {
                        if v.ws_has_item() {
                            changed |= v.ws_item(ui, &engine, lang, &mut self.status);
                        } else {
                            ui.label(RichText::new(lang.tr("Select an item to see its details.")).color(theme::ws(ui).muted));
                        }
                    }),
                    PanelId::Summary => scroll(ui, &mut |ui| changed |= v.ws_summary(ui, lang, &mut roll)),
                    PanelId::Recent => scroll(ui, &mut |ui| changed |= v.ws_history(ui, lang)),
                    _ => {}
                }
                if own_frame {
                    v.ws_end(vctx, &engine, lang, &self.pdfs, &mut self.status, changed);
                }
                if let Some(pool) = roll {
                    self.dice.set_pool(pool);
                    self.show_dice = true;
                }
            }
            DocKey::Campaign => {
                let mut action = None;
                if let Some(gm) = self.gm.as_mut() {
                    // Behind another document, the first popped panel
                    // runs the campaign's frame (new log lines, sheets).
                    if own_frame {
                        gm.begin_frame(&engine, &mut self.views, &mut self.online);
                    }
                    let panel = match key.panel {
                        PanelId::Gm(p) => Some(p),
                        _ => None,
                    };
                    let mut env = gm_env(&engine, &self.lang, &mut self.views, &mut self.status, &mut self.online, &mut self.ws.pops);
                    scroll(ui, &mut |ui| action = gm.ws_panel(ui, panel, &mut env));
                }
                if let Some(crate::gm_screen::Action::Open(id)) = action {
                    self.open_member(id);
                }
            }
            DocKey::Home => match key.panel {
                PanelId::Dice => scroll(ui, &mut |ui| self.dice.ui(ui, &self.lang)),
                PanelId::Initiative => {
                    let chars = self.initiative_characters();
                    scroll(ui, &mut |ui| self.initiative.ui(ui, &self.lang, &chars));
                }
                _ => {}
            },
        }
    }

    /// Whether an app-wide tool panel (dice, initiative) is in its own
    /// window.
    pub(crate) fn ws_out(&self, panel: PanelId) -> bool {
        self.appearance.layout == Layout::Workspace && self.ws.pops.is_out(PopKey::new(DocKey::Home, panel))
    }

    /// The pop-out button at the top of a tool window (Workspace only).
    pub(crate) fn ws_pop_button(&self, ui: &mut egui::Ui) -> bool {
        if self.appearance.layout != Layout::Workspace {
            return false;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| widgets::icon_button(ui, icons::ARROW_SQUARE_OUT, 22.0).on_hover_text(self.lang.tr("Pop out into its own window")).clicked()).inner
    }

    /// Pop a tool panel out when its button was clicked.
    pub(crate) fn ws_pop(&mut self, ctx: &egui::Context, panel: PanelId, clicked: bool) {
        if clicked {
            self.ws.pops.pop_out_near(PopKey::new(DocKey::Home, panel), ctx);
        }
    }

    // ----- palette and commands -----

    fn ws_palette(&mut self, ctx: &egui::Context) {
        if !self.ws.palette.open {
            return;
        }
        if self.ws.palette.wants_records() {
            let records = palette::record_entries(&self.engine.store, &self.lang);
            self.ws.palette.set_records(records);
        }
        let entries = self.ws_entries();
        let picked = self.ws.palette.ui(ctx, &entries, &self.lang);
        if self.ws.palette.wants_records() {
            ctx.request_repaint();
        }
        if let Some(t) = picked {
            self.ws_target(ctx, t);
        }
    }

    /// What the palette searches (besides the game data).
    fn ws_entries(&self) -> Vec<Entry> {
        let lang = &self.lang;
        let mut out = Vec::new();
        let doc = self.current().map(|i| self.views[i].doc());
        let has = doc.is_some();
        let undo = doc.and_then(|d| d.undo_label());
        let redo = doc.and_then(|d| d.redo_label());
        let kind = self.appearance.kind();
        for c in Cmd::ALL {
            let enabled = match c {
                Cmd::Save => has || self.home == Some(Home::Campaign),
                Cmd::SaveAs | Cmd::Print | Cmd::Export | Cmd::Close => has,
                Cmd::Undo => undo.is_some(),
                Cmd::Redo => redo.is_some(),
                Cmd::Dark => kind != ThemeKind::WorkspaceDark,
                Cmd::Light => kind != ThemeKind::WorkspaceLight,
                _ => true,
            };
            let title = match (c, undo, redo) {
                (Cmd::Undo, Some(w), _) => lang.tr_fmt("Undo: {0}", &[&w]),
                (Cmd::Redo, _, Some(w)) => lang.tr_fmt("Redo: {0}", &[&w]),
                _ => lang.tr(c.label()),
            };
            let (detail, keywords) = match c {
                Cmd::Dark | Cmd::Light => (format!("{} · {}", lang.tr("View"), lang.tr("Appearance")), "theme workspace"),
                Cmd::ClassicLayout => (format!("{} · {}", lang.tr("View"), lang.tr("Appearance")), "layout theme chummer"),
                _ => (lang.tr(c.menu()), ""),
            };
            out.push(Entry { kind: Kind::Command, icon: c.icon(), title, detail, hint: c.shortcut().to_owned(), keywords: keywords.to_owned(), enabled, target: Target::Command(c) });
        }
        let active = self.ws_doc();
        for t in self.ws_tabs().into_iter().filter(|t| t.doc != active) {
            out.push(Entry { kind: Kind::Document, icon: t.icon, title: t.name, detail: t.note, hint: String::new(), keywords: String::new(), enabled: true, target: Target::Document(t.doc) });
        }
        for g in self.ws_nav(active) {
            for it in g.items {
                out.push(Entry { kind: Kind::Navigate, icon: it.section.icon(), title: it.label, detail: g.title.clone(), hint: String::new(), keywords: String::new(), enabled: true, target: Target::Section(it.section) });
            }
        }
        if let Some(i) = self.current() {
            for it in self.views[i].ws_items(lang) {
                out.push(Entry {
                    kind: Kind::Item,
                    icon: it.section.icon(),
                    title: it.name,
                    detail: format!("{} · {}", it.list, lang.tr(it.section.label())),
                    hint: String::new(),
                    keywords: String::new(),
                    enabled: true,
                    target: Target::Item { section: it.section, guid: it.guid },
                });
            }
        }
        out
    }

    fn ws_target(&mut self, ctx: &egui::Context, t: Target) {
        match t {
            Target::Command(c) => self.ws_run(ctx, c),
            Target::Section(s) => self.ws_go(self.ws_doc(), s),
            Target::Item { section, guid } => {
                if let Some(i) = self.current() {
                    self.ws.special.remove(&self.views[i].ws_id());
                    self.views[i].ws_show_item(section, &guid);
                }
            }
            Target::Document(d) => self.ws_select(d),
            Target::Record { kind, index } => {
                self.home = Some(Home::MasterIndex);
                self.ws.home = Some(Section::DataBrowser);
                self.browser.show_record(&self.engine.store, kind, index);
            }
        }
    }

    /// Run a menu action (palette, buttons).
    fn ws_run(&mut self, ctx: &egui::Context, c: Cmd) {
        let has = self.current().is_some();
        match c {
            Cmd::NewCharacter => self.wizard = Some(crate::wizard::Wizard::new()),
            Cmd::NewCritter => self.critter = Some(crate::gm_ui::CritterWizard::new()),
            Cmd::Open => self.open_dialog(),
            Cmd::Save if has => {
                self.save(self.active, false);
            }
            Cmd::Save if self.home == Some(Home::Campaign) => {
                self.save_campaign(false);
            }
            Cmd::SaveAs if has => {
                self.save(self.active, true);
            }
            Cmd::Print if has => self.show_print = true,
            Cmd::Export if has => self.show_export = true,
            Cmd::Close if has => self.close_tab(self.active, false),
            Cmd::NewCampaign => self.new_campaign(),
            Cmd::OpenCampaign => self.open_campaign_dialog(),
            Cmd::JoinCampaign => self.online.join = Some((String::new(), self.online.display_name())),
            Cmd::Undo => self.undo(),
            Cmd::Redo => self.redo(),
            Cmd::DiceRoller => self.show_dice = true,
            Cmd::Initiative => self.show_initiative = true,
            Cmd::MasterIndex => self.ws_go(DocKey::Home, Section::DataBrowser),
            Cmd::Roster => self.ws_go(DocKey::Home, Section::Home),
            Cmd::CharacterSettings => self.show_settings = true,
            Cmd::Sourcebooks => self.show_sources = true,
            Cmd::OnlineSettings => self.online.show_settings = true,
            Cmd::Dark => self.set_appearance(ctx, self.appearance.with_kind(ThemeKind::WorkspaceDark)),
            Cmd::Light => self.set_appearance(ctx, self.appearance.with_kind(ThemeKind::WorkspaceLight)),
            Cmd::ClassicLayout => self.set_appearance(ctx, self.appearance.with_layout(Layout::Classic)),
            Cmd::GuidedCreation => self.set_guided(!crate::view::guided_preference()),
            Cmd::About => self.show_about = true,
            Cmd::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Save | Cmd::SaveAs | Cmd::Print | Cmd::Export | Cmd::Close => {}
        }
    }
}

/// What the GM screen's Workspace drawing needs from the app.
fn gm_env<'a>(
    engine: &'a std::sync::Arc<chummer_core::engine::Engine>,
    lang: &'a chummer_core::lang::Language,
    views: &'a mut [CharacterView],
    status: &'a mut crate::pdf_ui::Status,
    net: &'a mut crate::online::Online,
    pops: &'a mut popout::PopOuts,
) -> crate::gm_screen::workspace::Env<'a> {
    crate::gm_screen::workspace::Env { engine, lang, views, status, net, pops }
}

/// A 1px line across the `ui`.
fn rule(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, color);
}

fn member_icon(kind: &MemberKind) -> &'static str {
    match kind {
        MemberKind::Player => icons::USER,
        MemberKind::Npc => icons::USER_CIRCLE,
        MemberKind::Critter => icons::PAW_PRINT,
        MemberKind::Enemy => icons::SKULL,
        MemberKind::Spirit => icons::FLAME,
        MemberKind::Drone => icons::ROBOT,
        MemberKind::Other(_) => icons::USER,
    }
}

/// A document tab of the top bar: icon, name, a small note, and a close
/// button. Returns the response and whether the close button was clicked.
fn doc_tab(ui: &mut egui::Ui, t: &DocTab, selected: bool, close_tip: &str) -> (egui::Response, bool) {
    let ws = theme::ws(ui);
    let painter = ui.painter();
    let name = painter.layout_no_wrap(t.name.clone(), FontId::proportional(12.5), Color32::PLACEHOLDER);
    let note = (!t.note.is_empty()).then(|| painter.layout_no_wrap(t.note.clone(), FontId::proportional(11.0), Color32::PLACEHOLDER));
    let close_w = if t.closable { 11.0 + 6.0 } else { 0.0 };
    let note_w = note.as_ref().map_or(0.0, |g| g.size().x + 6.0);
    let width = 10.0 + 14.0 + 6.0 + name.size().x + note_w + close_w + 10.0;
    let (slot, _) = ui.allocate_exact_size(egui::vec2(width, 40.0), Sense::hover());
    let rect = egui::Rect::from_min_max(egui::pos2(slot.left(), slot.bottom() - 30.0), egui::pos2(slot.right(), slot.bottom() + 1.0));
    let resp = ui.interact(rect, ui.id().with(("doc tab", t.doc)), Sense::click());
    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0 - 5.5, rect.center().y), egui::vec2(16.0, 16.0));
    let over_close = t.closable && resp.hover_pos().is_some_and(|p| close_rect.contains(p));
    let closed = t.closable && resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| close_rect.contains(p));
    let painter = ui.painter();
    let radius = CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
    if selected {
        painter.rect(rect, radius, ws.ground, Stroke::new(1.0_f32, ws.divider), egui::StrokeKind::Inside);
        // Open towards the page: no line along the bottom.
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left() + 1.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 1.0, rect.bottom() + 1.0)), CornerRadius::ZERO, ws.ground);
    } else if resp.hovered() {
        painter.rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), radius, ws.hover);
    }
    let ink = if selected || resp.hovered() { ws.text } else { ws.muted };
    let mut x = rect.left() + 10.0;
    icons::paint(painter, egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), egui::Vec2::splat(14.0)), t.icon, 14.0, if selected { ws.accent } else { ws.muted });
    x += 14.0 + 6.0;
    let y = |h: f32| rect.center().y - h / 2.0;
    painter.galley(egui::pos2(x, y(name.size().y)), name.clone(), ink);
    x += name.size().x + 6.0;
    if let Some(g) = note {
        painter.galley(egui::pos2(x, y(g.size().y) + 1.0), g, ws.muted);
    }
    if t.closable {
        if over_close {
            painter.rect_filled(close_rect, CornerRadius::same(3), ws.hover);
        }
        icons::paint(painter, close_rect, icons::X, 11.0, if over_close { ws.error } else { ws.muted });
    }
    let resp = if over_close { resp.on_hover_text(close_tip) } else { resp };
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), closed)
}

/// The search field of the top bar (a button that opens the palette).
fn search_field(ui: &mut egui::Ui, hint: &str) -> egui::Response {
    let ws = theme::ws(ui);
    let width = 340.0_f32.min((ui.available_width() - 160.0).max(160.0));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 28.0), Sense::click());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(5), ws.well, Stroke::new(1.0_f32, if resp.hovered() { ws.primary } else { ws.control }), egui::StrokeKind::Inside);
    icons::paint(painter, egui::Rect::from_min_size(egui::pos2(rect.left() + 8.0, rect.center().y - 7.0), egui::Vec2::splat(14.0)), icons::MAGNIFYING_GLASS, 14.0, ws.muted);
    let kbd = painter.layout_no_wrap("Ctrl K".into(), FontId::monospace(10.5), ws.muted);
    let k = egui::Rect::from_min_size(egui::pos2(rect.right() - 8.0 - kbd.size().x - 8.0, rect.center().y - 8.0), egui::vec2(kbd.size().x + 8.0, 16.0));
    let text = painter.layout_no_wrap(hint.to_owned(), FontId::proportional(12.5), ws.muted);
    painter.with_clip_rect(egui::Rect::from_min_max(rect.min, egui::pos2(k.left() - 6.0, rect.bottom()))).galley(egui::pos2(rect.left() + 30.0, rect.center().y - text.size().y / 2.0), text, ws.muted);
    painter.rect(k, CornerRadius::same(3), ws.well, Stroke::new(1.0_f32, ws.divider), egui::StrokeKind::Inside);
    painter.galley(k.center() - kbd.size() / 2.0, kbd, ws.muted);
    resp.on_hover_cursor(egui::CursorIcon::Text)
}
