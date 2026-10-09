//! App preferences kept in gui.ini next to the appearance (see
//! [`crate::theme::config_path`]), and the Tools → Preferences window
//! (with the Folders list: where the user's files live).
//!
//! Keys added here:
//!
//! * `setup_done=true` — the first-start setup ran (or was skipped).
//! * `check_updates=true|false` — whether the updater may look for new
//!   versions (read by the updater; on unless turned off).
//! * `autosave_minutes=2` — how often unsaved characters are copied to
//!   the recovery folder; 0 turns the timer off (focus loss still saves).
//! * `backup_count=5` — previous versions kept per file on save; 0 keeps
//!   none.
//! * `translation_notice_seen=de-de,fr-fr` — languages whose
//!   incomplete-translation notice was dismissed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chummer_core::paths;
use eframe::egui;

use crate::theme::{self, Layout};

pub const SETUP_DONE: &str = "setup_done";
pub const CHECK_UPDATES: &str = "check_updates";
pub const AUTOSAVE_MINUTES: &str = "autosave_minutes";
pub const BACKUP_COUNT: &str = "backup_count";
pub const NOTICE_SEEN: &str = "translation_notice_seen";

pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 2;
pub const DEFAULT_BACKUP_COUNT: usize = 5;

/// The backups kept per file, for save jobs on other threads.
static BACKUPS: AtomicUsize = AtomicUsize::new(DEFAULT_BACKUP_COUNT);

/// The number of backups to keep per file (save threads read it).
pub fn backup_count() -> usize {
    BACKUPS.load(Ordering::Relaxed)
}

/// Whether the updater may check for new versions (`check_updates=`;
/// on unless it says false). For the updater (`update.rs`).
#[allow(dead_code)]
pub fn check_updates() -> bool {
    parse_bool(theme::load_value(CHECK_UPDATES).as_deref(), true)
}

fn parse_bool(v: Option<&str>, default: bool) -> bool {
    match v.map(str::trim) {
        Some("true" | "1" | "yes" | "on") => true,
        Some("false" | "0" | "no" | "off") => false,
        _ => default,
    }
}

/// The preferences, read from gui.ini once and kept in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub autosave_minutes: u32,
    pub backup_count: usize,
    pub check_updates: bool,
    pub notice_seen: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { autosave_minutes: DEFAULT_AUTOSAVE_MINUTES, backup_count: DEFAULT_BACKUP_COUNT, check_updates: true, notice_seen: Vec::new() }
    }
}

impl Prefs {
    /// The preferences in a gui.ini text (defaults for missing keys).
    pub fn from_config(text: &str) -> Prefs {
        let d = Prefs::default();
        let get = |k: &str| theme::config_get(text, k);
        Prefs {
            autosave_minutes: get(AUTOSAVE_MINUTES).and_then(|v| v.parse().ok()).map_or(d.autosave_minutes, |m: u32| m.min(120)),
            backup_count: get(BACKUP_COUNT).and_then(|v| v.parse().ok()).map_or(d.backup_count, |n: usize| n.min(100)),
            check_updates: parse_bool(get(CHECK_UPDATES).as_deref(), true),
            notice_seen: get(NOTICE_SEEN).map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        }
    }

    /// `text` with these preferences set; other lines are kept.
    pub fn to_config(&self, text: &str) -> String {
        let s = theme::config_set(text, AUTOSAVE_MINUTES, &self.autosave_minutes.to_string());
        let s = theme::config_set(&s, BACKUP_COUNT, &self.backup_count.to_string());
        let s = theme::config_set(&s, CHECK_UPDATES, if self.check_updates { "true" } else { "false" });
        theme::config_set(&s, NOTICE_SEEN, &self.notice_seen.join(","))
    }

    pub fn load() -> Prefs {
        let p = Prefs::from_config(&theme::config_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default());
        p.publish();
        p
    }

    /// Write to gui.ini (keeping its other lines).
    pub fn save(&self) -> std::io::Result<()> {
        self.publish();
        let Some(path) = theme::config_path() else { return Ok(()) };
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_config(&old))
    }

    /// Make the values other threads read current.
    fn publish(&self) {
        BACKUPS.store(self.backup_count, Ordering::Relaxed);
    }

    /// The autosave interval; `None` when the timer is off.
    pub fn autosave_every(&self) -> Option<std::time::Duration> {
        (self.autosave_minutes > 0).then(|| std::time::Duration::from_secs(u64::from(self.autosave_minutes) * 60))
    }
}

/// What the Preferences window asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefsAction {
    RunSetup,
    OpenFolder(PathBuf),
}

/// The folders listed under Preferences → Folders: the user data root,
/// the config root when it is elsewhere (Linux), then each user folder.
pub fn folder_list(lang: &chummer_core::lang::Language) -> Vec<(String, PathBuf)> {
    let Some(roots) = paths::Roots::current() else { return Vec::new() };
    let mut out = vec![(lang.tr("User data"), roots.data.clone())];
    if roots.config != roots.data {
        out.push((lang.tr("Configuration"), roots.config.clone()));
    }
    out.extend(paths::UserDir::ALL.into_iter().map(|d| (lang.tr(d.label()), roots.dir(d))));
    out
}

/// An "Open folder" button (a Workspace button in the Workspace layout).
pub fn open_button(ui: &mut egui::Ui, lang: &chummer_core::lang::Language, text: &str) -> egui::Response {
    if theme::current(ui.ctx()).kind.layout() == Layout::Workspace {
        crate::workspace::widgets::button(ui, Some(crate::workspace::icons::FOLDER_OPEN), &lang.tr(text), crate::workspace::widgets::Look::Secondary, 24.0)
    } else {
        ui.button(lang.tr(text))
    }
}

/// A "Copy path" button; copies `dir` to the clipboard.
fn copy_button(ui: &mut egui::Ui, lang: &chummer_core::lang::Language, dir: &Path) {
    let r = if theme::current(ui.ctx()).kind.layout() == Layout::Workspace {
        crate::workspace::widgets::button(ui, Some(crate::workspace::icons::COPY), &lang.tr("Copy path"), crate::workspace::widgets::Look::Ghost, 24.0)
    } else {
        ui.button(lang.tr("Copy path"))
    };
    if r.clicked() {
        ui.ctx().copy_text(dir.display().to_string());
    }
}

/// Preferences → Folders: each folder with its path, Open and Copy path.
fn folders_ui(ui: &mut egui::Ui, lang: &chummer_core::lang::Language) -> Option<PathBuf> {
    let mut open = None;
    ui.label(theme::strong(ui, lang.tr("Folders")));
    ui.weak(lang.tr("Your own files live here; nothing in the program's folder needs editing."));
    ui.add_space(4.0);
    egui::Grid::new("prefs_folders").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
        for (label, dir) in folder_list(lang) {
            ui.label(label);
            ui.add(egui::Label::new(egui::RichText::new(dir.display().to_string()).monospace().small()).wrap_mode(egui::TextWrapMode::Extend));
            ui.horizontal(|ui| {
                if open_button(ui, lang, "Open").clicked() {
                    open = Some(dir.clone());
                }
                copy_button(ui, lang, &dir);
            });
            ui.end_row();
        }
    });
    open
}

/// Tools → Preferences: autosave, backups, updates, folders. Returns an
/// action and whether a value changed (the caller saves).
pub fn prefs_ui(ui: &mut egui::Ui, p: &mut Prefs, lang: &chummer_core::lang::Language) -> (Option<PrefsAction>, bool) {
    let mut changed = false;
    let mut action = None;
    let workspace = theme::current(ui.ctx()).kind.layout() == Layout::Workspace;
    egui::Grid::new("prefs_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        ui.label(lang.tr("Autosave every"));
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut p.autosave_minutes).range(0..=120)).changed();
            ui.label(lang.tr("minutes"));
        });
        ui.end_row();
        ui.label("");
        ui.weak(lang.tr("Unsaved characters are copied to a recovery folder (never over your file). 0 turns the timer off."));
        ui.end_row();
        ui.label(lang.tr("Backups per file"));
        changed |= ui.add(egui::DragValue::new(&mut p.backup_count).range(0..=100)).changed();
        ui.end_row();
        ui.label("");
        ui.weak(lang.tr("Saving over a file keeps its previous version; File → Restore backup… lists them."));
        ui.end_row();
        ui.label(lang.tr("Updates"));
        changed |= if workspace { crate::workspace::widgets::check(ui, &mut p.check_updates, &lang.tr("Check for updates")).changed() } else { ui.checkbox(&mut p.check_updates, lang.tr("Check for updates")).changed() };
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(lang.tr("Run the first-start setup again…")).clicked() {
            action = Some(PrefsAction::RunSetup);
        }
    });
    ui.add_space(10.0);
    ui.separator();
    if let Some(dir) = folders_ui(ui, lang) {
        action = Some(PrefsAction::OpenFolder(dir));
    }
    (action, changed)
}

/// Open a folder in the desktop's file manager (without waiting).
pub fn open_folder(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(dir).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip() {
        let p = Prefs::from_config("");
        assert_eq!(p, Prefs::default());
        assert_eq!((p.autosave_minutes, p.backup_count, p.check_updates), (2, 5, true));
        let p = Prefs { autosave_minutes: 5, backup_count: 0, check_updates: false, notice_seen: vec!["de-de".into(), "fr-fr".into()] };
        let text = p.to_config("theme=classic\n");
        assert!(text.starts_with("theme=classic\n"));
        assert!(text.contains("check_updates=false\n"));
        assert_eq!(Prefs::from_config(&text), p);
        // Out-of-range and broken values.
        let q = Prefs::from_config("autosave_minutes=9999\nbackup_count=x\ncheck_updates=maybe\n");
        assert_eq!((q.autosave_minutes, q.backup_count, q.check_updates), (120, 5, true));
        assert_eq!(Prefs { autosave_minutes: 0, ..Prefs::default() }.autosave_every(), None);
    }
}
