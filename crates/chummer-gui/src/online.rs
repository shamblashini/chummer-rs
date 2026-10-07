//! Online campaigns in the app: the network node, the campaigns this
//! player has joined, and the Join and Online Settings windows. The GM's
//! side (hosting) is on the GM screen (`gm_screen`), which uses the node
//! from here.
//!
//! Networking runs on a tokio runtime owned by [`Online`]; the UI thread
//! only calls synchronous methods (edits are applied locally and sent by
//! background tasks) and repaints twice a second while anything is online
//! so arriving changes show.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_net::config::{OnlineSettings, DEFAULT_MAILBOX_ID, DEFAULT_RELAY_URL};
use chummer_net::invite::InviteLink;
use chummer_net::SecretKey;
use chummer_sync::joined::{Joined, JoinedList};
use chummer_sync::{CharacterId, Node, PlayerConfig, PlayerSession, SyncMode};
use eframe::egui::{self, RichText};

/// The Online Settings window's certificate dialog (`bg`).
const PEM_DIALOG: &str = "dialog:pem";

/// How often a player's app tries the GM again (and the mailbox) while
/// not connected.
pub const PLAYER_SYNC_EVERY: Duration = Duration::from_secs(60);

/// How often the GM's app does a mailbox round while hosting.
pub const GM_MAIL_EVERY: Duration = Duration::from_secs(180);

/// A joined campaign with its running session.
pub struct JoinedCampaign {
    pub key: String,
    pub session: PlayerSession,
}

impl JoinedCampaign {
    pub fn name(&self) -> String {
        let r = self.session.replica();
        match r.membership().map(|m| m.campaign_name.clone()).filter(|n| !n.trim().is_empty()) {
            Some(n) => n,
            None => format!("Campaign {}", &self.key[..8.min(self.key.len())]),
        }
    }
}

/// What the start screen's Campaigns list asks for.
pub enum CampaignAction {
    /// Open a player's character as a tab.
    Open(usize, CharacterId),
}

pub struct Online {
    rt: tokio::runtime::Runtime,
    pub settings: OnlineSettings,
    secret: Option<SecretKey>,
    node: Option<Arc<Node>>,
    /// The settings the node was started with (a change needs a restart).
    node_settings: Option<OnlineSettings>,
    pub joined: Vec<JoinedCampaign>,
    list: JoinedList,
    dir: Option<PathBuf>,
    pub show_settings: bool,
    form: SettingsForm,
    /// The Join window, when open: the link and the name to join with.
    pub join: Option<(String, String)>,
    pub error: Option<String>,
}

#[derive(Default)]
struct SettingsForm {
    name: String,
    relays: String,
    ca_files: Vec<PathBuf>,
    message: Option<(String, bool)>,
}

impl Online {
    pub fn new() -> Online {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("chummer-net").enable_all().build().expect("a tokio runtime");
        let dir = chummer_sync::joined::dir();
        let list = dir.as_deref().map(JoinedList::load).unwrap_or_default();
        Online { rt, settings: OnlineSettings::load(), secret: None, node: None, node_settings: None, joined: Vec::new(), list, dir, show_settings: false, form: SettingsForm::default(), join: None, error: None }
    }

    /// The runtime, for entering it on another thread.
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(f)
    }

    /// This machine's node key (made on first use).
    pub fn secret(&mut self) -> Result<SecretKey, String> {
        if let Some(s) = &self.secret {
            return Ok(s.clone());
        }
        let s = chummer_net::identity::load_or_create_default().map_err(|e| format!("Could not load the node key: {e}"))?;
        self.secret = Some(s.clone());
        Ok(s)
    }

    /// The node, started on first use.
    pub fn node(&mut self) -> Result<Arc<Node>, String> {
        if let Some(n) = &self.node {
            return Ok(n.clone());
        }
        let secret = self.secret()?;
        let cfg = self.settings.net_config()?;
        let node = crate::trace::time("network node bind", || self.rt.block_on(Node::bind(secret, cfg))).map_err(|e| e.to_string())?;
        let node = Arc::new(node);
        self.node = Some(node.clone());
        self.node_settings = Some(self.settings.clone());
        Ok(node)
    }

    pub fn node_if_started(&self) -> Option<Arc<Node>> {
        self.node.clone()
    }

    /// The display name, or a default.
    pub fn display_name(&self) -> String {
        let n = self.settings.name.trim();
        if n.is_empty() {
            whoami()
        } else {
            n.to_owned()
        }
    }

    /// Whether anything online is running (the UI then repaints now and
    /// then).
    pub fn active(&self) -> bool {
        self.node.is_some()
    }

    /// Starts the sessions of the joined campaigns (at app start).
    pub fn start_joined(&mut self, engine: &Arc<Engine>) {
        let list = self.list.campaigns.clone();
        for j in list {
            if let Err(e) = self.open_session(&j, engine) {
                self.error = Some(e);
            }
        }
    }

    fn open_session(&mut self, j: &Joined, engine: &Arc<Engine>) -> Result<usize, String> {
        let key = j.key();
        if let Some(i) = self.joined.iter().position(|c| c.key == key) {
            return Ok(i);
        }
        let link = j.link().ok_or("not a valid invite link")?;
        let node = self.node()?;
        let dir = self.dir.clone().ok_or("no user config directory found")?;
        let mut cfg = PlayerConfig::new(self.display_name(), link);
        cfg.mailbox = node.mailbox_id();
        cfg.path = Some(JoinedList::replica_path(&dir, j));
        let session = {
            let _s = crate::trace::span("player session start (replica load)");
            let _g = self.rt.enter();
            PlayerSession::new(node.endpoint().clone(), node.secret().clone(), engine.clone(), cfg).map_err(|e| e.to_string())?
        };
        self.rt.spawn(session.clone().keep_synced(PLAYER_SYNC_EVERY));
        self.joined.push(JoinedCampaign { key, session });
        Ok(self.joined.len() - 1)
    }

    /// File → Join Campaign: joins with `link`.
    pub fn join(&mut self, link: &str, name: &str, engine: &Arc<Engine>) -> Result<usize, String> {
        let link: InviteLink = link.trim().parse().map_err(|e| format!("{e}"))?;
        if !name.trim().is_empty() && name.trim() != self.settings.name {
            self.settings.name = name.trim().to_owned();
            let _ = self.settings.save();
        }
        if self.node().is_ok_and(|n| n.id() == link.host) {
            return Err("This is your own campaign's link (you are its GM).".into());
        }
        let j = self.list.add(&link);
        self.save_list();
        // Joining again with a new link replaces the old session.
        if let Some(i) = self.joined.iter().position(|c| c.key == j.key()) {
            self.joined.remove(i).session.close();
        }
        self.open_session(&j, engine)
    }

    /// Forgets a campaign; its local copy file stays.
    pub fn leave(&mut self, i: usize) {
        let c = self.joined.remove(i);
        c.session.close();
        self.list.remove(&c.key);
        self.save_list();
    }

    fn save_list(&mut self) {
        if let Some(d) = &self.dir {
            if let Err(e) = self.list.save(d) {
                self.error = Some(format!("Could not save the campaign list: {e}"));
            }
        }
    }

    /// Ends every session and the node (at exit).
    pub fn shutdown(&mut self) {
        for c in self.joined.drain(..) {
            c.session.close();
        }
        if let Some(n) = self.node.take() {
            if let Ok(n) = Arc::try_unwrap(n) {
                let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), n.shutdown()).await });
            }
        }
    }

    /// The Campaigns list on the start screen.
    pub fn campaigns_ui(&mut self, ui: &mut egui::Ui, lang: &Language) -> Option<CampaignAction> {
        let mut out = None;
        ui.horizontal(|ui| {
            ui.label(crate::theme::strong(ui, lang.tr("Campaigns")));
            if ui.small_button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Join Campaign…"))).clicked() {
                self.join = Some((String::new(), self.display_name()));
            }
        });
        if self.joined.is_empty() {
            ui.weak(lang.tr("No campaigns joined. Ask your GM for an invite link."));
        }
        let mut leave = None;
        for (i, c) in self.joined.iter().enumerate() {
            let name = c.name();
            let r = c.session.replica();
            let pending = r.outbox_len();
            let refused = r.refused().len();
            let status = match c.session.last_mode() {
                Some(SyncMode::Online) => lang.tr("online"),
                Some(SyncMode::Mailbox) => lang.tr("via mailbox"),
                Some(SyncMode::Offline) => lang.tr("offline"),
                None => lang.tr("connecting…"),
            };
            let chars: Vec<(CharacterId, String)> = r.characters().map(|id| (id.clone(), r.name(id).unwrap_or_default().to_owned())).collect();
            drop(r);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&name).strong());
                ui.weak(status);
                if pending > 0 {
                    ui.weak(lang.tr_fmt("{0} pending", &[&pending]));
                }
                if refused > 0 {
                    ui.colored_label(ui.visuals().error_fg_color, lang.tr_fmt("{0} refused", &[&refused]));
                }
                if ui.small_button(crate::theme::glyph("⟳")).on_hover_text(lang.tr("Sync now")).clicked() {
                    c.session.sync_soon();
                }
                ui.menu_button("…", |ui| {
                    if ui.button(lang.tr("Copy invite link")).clicked() {
                        ui.ctx().copy_text(c.session.link().to_string());
                        ui.close();
                    }
                    if ui.button(lang.tr("Leave campaign")).on_hover_text(lang.tr("Stop syncing it here; your copy file stays")).clicked() {
                        leave = Some(i);
                        ui.close();
                    }
                });
            });
            if chars.is_empty() {
                ui.weak(format!("   {}", lang.tr("The GM has not given you a character yet.")));
            }
            for (id, n) in chars {
                if ui.link(format!("   {n}")).clicked() {
                    out = Some(CampaignAction::Open(i, id));
                }
            }
        }
        if let Some(i) = leave {
            self.leave(i);
        }
        out
    }

    /// The Join Campaign and Online Settings windows.
    pub fn windows(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, status: &mut crate::pdf_ui::Status) {
        if self.active() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if let Some(e) = self.error.take() {
            *status = Some((e, true));
        }
        if let Some((mut link, mut name)) = self.join.take() {
            let mut open = true;
            let mut keep = true;
            egui::Window::new(lang.tr("Join Campaign")).id(egui::Id::new("join_campaign")).open(&mut open).collapsible(false).default_width(560.0).show(ctx, |ui| {
                ui.label(lang.tr("Paste the invite link your GM sent you:"));
                ui.add(egui::TextEdit::singleline(&mut link).hint_text("chummer-rs://join/…").desired_width(f32::INFINITY));
                let parsed = link.trim().parse::<InviteLink>();
                if let (Err(e), false) = (&parsed, link.trim().is_empty()) {
                    ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                }
                ui.horizontal(|ui| {
                    ui.label(lang.tr("Your name"));
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0));
                });
                ui.weak(lang.tr("The GM sees this name. Your characters sync when the GM's app is online; otherwise changes go through the relay's mailbox."));
                ui.horizontal(|ui| {
                    if ui.add_enabled(parsed.is_ok(), crate::theme::primary_button(ui, lang.tr("Join"))).clicked() {
                        match self.join(&link, &name, engine) {
                            Ok(_) => {
                                *status = Some((lang.tr("Joined. Your characters appear on the start screen once the GM gives them to you."), false));
                                keep = false;
                            }
                            Err(e) => *status = Some((e, true)),
                        }
                    }
                    if ui.button(lang.tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            });
            if open && keep {
                self.join = Some((link, name));
            }
        }
        if self.show_settings {
            if self.form.relays.is_empty() && self.form.name.is_empty() && self.form.ca_files.is_empty() {
                self.load_form();
            }
            let mut open = true;
            egui::Window::new(lang.tr("Online Settings")).id(egui::Id::new("online_settings")).open(&mut open).default_width(620.0).show(ctx, |ui| self.settings_ui(ui, lang));
            if !open {
                self.show_settings = false;
                self.form = SettingsForm::default();
            }
        }
    }

    fn load_form(&mut self) {
        let s = &self.settings;
        self.form.name = s.name.clone();
        self.form.relays = if s.relays.is_empty() { default_entry() } else { s.relays.join("\n") };
        self.form.ca_files = s.ca_files.clone();
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, lang: &Language) {
        if let Some(Some(p)) = crate::bg::take::<Option<PathBuf>>(PEM_DIALOG) {
            self.form.ca_files.push(p);
        }
        egui::Grid::new("online_settings_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(lang.tr("Your name"));
            ui.add(egui::TextEdit::singleline(&mut self.form.name).hint_text(whoami()).desired_width(260.0));
            ui.end_row();
            ui.label(lang.tr("Node id"));
            match self.secret() {
                Ok(k) => {
                    let id = k.public().to_string();
                    ui.horizontal(|ui| {
                        ui.monospace(&id[..16]).on_hover_text(&id);
                        if ui.small_button(lang.tr("Copy")).on_hover_text(lang.tr("Your identity in campaigns; a GM can assign you a character with it")).clicked() {
                            ui.ctx().copy_text(id.clone());
                        }
                    });
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(RichText::new(lang.tr("Relays")).strong());
        ui.weak(lang.tr("One per line: https://relay.example.org#<mailbox node id>, as chummer-relay prints it. The first one with a mailbox is used for play-by-post."));
        ui.add(egui::TextEdit::multiline(&mut self.form.relays).desired_rows(3).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
        ui.horizontal(|ui| {
            if ui.button(lang.tr("Project relay")).on_hover_text(DEFAULT_RELAY_URL).clicked() {
                self.form.relays = default_entry();
            }
            if ui.button(lang.tr("Local test relay")).on_hover_text(lang.tr("chummer-relay --dev on this machine; add its mailbox id after #")).clicked() {
                self.form.relays = "https://127.0.0.1:3443#".into();
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(lang.tr("Trusted certificates")).strong());
        ui.weak(lang.tr("For a relay with a self-signed certificate (chummer-relay --dev): its self-signed-cert.pem."));
        let mut remove = None;
        for (i, f) in self.form.ca_files.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(f.display().to_string());
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.form.ca_files.remove(i);
        }
        if ui.button(lang.tr("Add certificate…")).clicked() {
            crate::bg::dialog(ui.ctx(), PEM_DIALOG, || rfd::FileDialog::new().add_filter("PEM", &["pem", "crt"]).add_filter("All files", &["*"]).pick_file());
        }
        #[cfg(windows)]
        {
            ui.add_space(6.0);
            if ui.button(lang.tr("Open chummer-rs:// links with this program")).clicked() {
                self.form.message = Some(match register_url_scheme() {
                    Ok(()) => (lang.tr("Invite links now open chummer-rs."), false),
                    Err(e) => (e, true),
                });
            }
        }
        ui.separator();
        if let Some((m, err)) = &self.form.message {
            if *err {
                ui.colored_label(ui.visuals().error_fg_color, m);
            } else {
                ui.label(m);
            }
        }
        if ui.add(crate::theme::primary_button(ui, lang.tr("Save"))).clicked() {
            let relays: Vec<String> = self.form.relays.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            let relays = if relays.join("\n") == default_entry() { Vec::new() } else { relays };
            let s = OnlineSettings { name: self.form.name.trim().to_owned(), relays, ca_files: self.form.ca_files.clone(), port: self.settings.port };
            self.form.message = Some(match s.net_config() {
                Err(e) => (e, true),
                Ok(_) => match s.save() {
                    Err(e) => (format!("Could not save: {e}"), true),
                    Ok(()) => {
                        let restart = self.node_settings.as_ref().is_some_and(|n| n.relays != s.relays || n.ca_files != s.ca_files);
                        self.settings = s;
                        if restart {
                            (lang.tr("Saved. Restart chummer-rs to use the new relays."), false)
                        } else {
                            (lang.tr("Saved."), false)
                        }
                    }
                },
            });
        }
    }
}

fn default_entry() -> String {
    match DEFAULT_MAILBOX_ID {
        Some(m) => format!("{DEFAULT_RELAY_URL}#{m}"),
        None => DEFAULT_RELAY_URL.to_owned(),
    }
}

fn whoami() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "Player".into())
}

/// Registers `chummer-rs://` links for the current user (no admin rights
/// needed): `HKCU\Software\Classes\chummer-rs`.
#[cfg(windows)]
fn register_url_scheme() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!("\"{}\" \"%1\"", exe.display());
    let key = r"HKCU\Software\Classes\chummer-rs";
    let run = |args: &[&str]| -> Result<(), String> {
        let s = std::process::Command::new("reg").args(args).status().map_err(|e| e.to_string())?;
        if s.success() {
            Ok(())
        } else {
            Err(format!("reg {} failed", args.join(" ")))
        }
    };
    run(&["add", key, "/ve", "/d", "URL:chummer-rs invite", "/f"])?;
    run(&["add", key, "/v", "URL Protocol", "/d", "", "/f"])?;
    run(&["add", &format!(r"{key}\shell\open\command"), "/ve", "/d", &cmd, "/f"])
}
