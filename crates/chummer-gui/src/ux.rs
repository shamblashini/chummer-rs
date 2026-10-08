//! The app's comfort and safety features, hooked into `main.rs` with
//! one-line calls: crash recovery and autosave ([`crate::safety`]),
//! backups ([`crate::backups`]), the first-start setup
//! ([`crate::setup_ui`]), the translation notice
//! ([`crate::lang_notice`]) and Tools → Preferences ([`crate::prefs`]).

use eframe::egui;

use crate::prefs::{self, Prefs, PrefsAction};
use crate::App;

/// Their state in the app.
#[derive(Default)]
pub struct Ux {
    pub prefs: Prefs,
    pub safety: crate::safety::Safety,
    pub setup: Option<crate::setup_ui::Setup>,
    /// The language whose translation notice is showing.
    pub notice: Option<String>,
    pub restore: crate::backups::RestoreWindow,
    pub show_prefs: bool,
}

impl App {
    /// At start (from `App::new`): preferences, the recovery scan and,
    /// on the very first start, the setup. `ran_before`: an earlier run
    /// saved app state; `look_from_command_line`: `--theme`/`--layout`.
    pub(crate) fn ux_start(&mut self, ran_before: bool, look_from_command_line: bool) {
        self.ux.prefs = Prefs::load();
        self.safety_start();
        let gui_ini = crate::theme::config_path().and_then(|p| std::fs::read_to_string(p).ok());
        if crate::setup_ui::should_show(gui_ini.as_deref(), ran_before, look_from_command_line) {
            self.open_setup();
        } else if gui_ini.is_none() && ran_before && !look_from_command_line {
            // An earlier version that never wrote gui.ini ran the Classic
            // layout: keep it rather than switch silently.
            let ctx = self.ctx.clone();
            self.set_appearance(&ctx, self.appearance.with_layout(crate::theme::Layout::Classic));
        }
    }

    /// Each frame, after the layout and windows.
    pub(crate) fn ux_frame(&mut self, ctx: &egui::Context) {
        self.safety_frame(ctx);
        self.restore_window(ctx);
        self.prefs_window(ctx);
        self.notice_window(ctx);
        self.setup_window(ctx);
    }

    /// A clean exit.
    pub(crate) fn ux_exit(&mut self) {
        self.safety_exit();
    }

    /// File menu: Restore backup….
    pub(crate) fn ux_file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.lang.tr("Restore backup…")).on_hover_text(self.lang.tr("Earlier versions kept when you saved over a file")).clicked() {
            ui.close();
            self.show_restore_backup();
        }
    }

    /// Tools menu: Preferences….
    pub(crate) fn ux_tools_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.lang.tr("Preferences…")).on_hover_text(self.lang.tr("Autosave, backups and updates")).clicked() {
            ui.close();
            self.ux.show_prefs = true;
        }
    }

    /// Help menu: First-start setup….
    pub(crate) fn ux_help_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.lang.tr("First-start setup…")).clicked() {
            ui.close();
            self.open_setup();
        }
    }

    fn prefs_window(&mut self, ctx: &egui::Context) {
        if !self.ux.show_prefs {
            return;
        }
        let mut open = true;
        let mut out = (None, false);
        egui::Window::new(self.lang.tr("Preferences")).id(egui::Id::new("preferences")).open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
            out = prefs::prefs_ui(ui, &mut self.ux.prefs, &self.lang);
        });
        self.ux.show_prefs = open;
        let (action, changed) = out;
        if changed {
            if let Err(e) = self.ux.prefs.save() {
                self.status = Some((format!("Could not save the settings: {e}"), true));
            }
        }
        let root = crate::safety::data_dir();
        let folder = match action {
            Some(PrefsAction::RunSetup) => {
                self.ux.show_prefs = false;
                self.open_setup();
                None
            }
            Some(PrefsAction::OpenRecoveryFolder) => root.map(|r| crate::safety::recovery_root(&r)),
            Some(PrefsAction::OpenBackupsFolder) => crate::backups::backups_dir(),
            None => None,
        };
        if let Some(dir) = folder {
            if let Err(e) = prefs::open_folder(&dir) {
                self.status = Some((format!("Could not open {}: {e}", dir.display()), true));
            }
        }
    }
}
