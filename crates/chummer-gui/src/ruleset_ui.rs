//! Sharing house rules: importing a settings file a GM handed out, the
//! warning on characters whose preset is not installed, and Chummer's
//! "Change Settings File" for an open character.

use chummer_core::character::Character;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::{self, CharacterSettings, FileClash, ImportMode, ImportPlan, Imported};
use eframe::egui::{self, RichText};

use crate::view::WARN;

/// Picks a settings file and installs it into the user settings directory,
/// asking when a different file of the same name is already there.
#[derive(Default)]
pub struct ImportFlow {
    pending: Option<ImportPlan>,
}

impl ImportFlow {
    /// Ask for a file and install it unless it clashes. `Some` when done
    /// (the library must then be reloaded on `Ok`).
    pub fn start(&mut self, engine: &Engine) -> Option<Result<Imported, String>> {
        let src = rfd::FileDialog::new().add_filter("Chummer settings", &["xml"]).add_filter("All files", &["*"]).pick_file()?;
        let Some(dir) = settings::user_settings_dir() else { return Some(Err("no settings directory".into())) };
        let plan = match engine.settings.plan_import(&src, &dir) {
            Ok(p) => p,
            Err(e) => return Some(Err(e)),
        };
        if plan.file_clash == Some(FileClash::Different) {
            self.pending = Some(plan);
            return None;
        }
        Some(install(&plan, engine, &dir, &ImportMode::New))
    }

    /// The clash prompt, while one is open.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> Option<Result<Imported, String>> {
        let file_name = self.pending.as_ref()?.file_name.clone();
        let mut mode = None;
        let mut cancel = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.colored_label(WARN, lang.tr_fmt("A different settings file named {0} is already installed.", &[&file_name]));
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Overwrite")).on_hover_text(lang.tr("Characters using the installed file will use the imported rules.")).clicked() {
                    mode = Some(ImportMode::Overwrite);
                }
                if ui.button(lang.tr("Keep both")).on_hover_text(lang.tr("Save the imported file under a new name.")).clicked() {
                    mode = Some(ImportMode::KeepBoth);
                }
                cancel = ui.button(lang.tr("Cancel")).clicked();
            });
        });
        if cancel {
            self.pending = None;
        }
        let mode = mode?;
        let plan = self.pending.take()?;
        let Some(dir) = settings::user_settings_dir() else { return Some(Err("no settings directory".into())) };
        Some(install(&plan, engine, &dir, &mode))
    }
}

fn install(plan: &ImportPlan, engine: &Engine, dir: &std::path::Path, mode: &ImportMode) -> Result<Imported, String> {
    settings::import(plan, &engine.settings, dir, mode)
}

const IMPORT_REQUEST: &str = "ruleset_import_request";

/// Ask the app to open the settings window and start an import.
fn request_import(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(IMPORT_REQUEST), true));
}

/// Whether an import was requested since the last call.
pub fn take_import_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(IMPORT_REQUEST))).unwrap_or(false)
}

/// The warning shown on a character whose `<settings>` preset is not
/// installed (Chummer5a asks "Cannot Find Settings File" on load), and on
/// the Info tab, the preset in use with Chummer's "Change Settings File".
/// Returns the key of a preset the user picked.
pub fn banner(ui: &mut egui::Ui, ch: &Character, engine: &Engine, lang: &Language, show_row: bool) -> Option<String> {
    let key = ch.field("settings");
    let lib = &engine.settings;
    let mut picked = None;
    if let Some(missing) = lib.missing_preset(&key) {
        let fallback = lib.fallback().map(CharacterSettings::name).unwrap_or_default();
        egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0_f32, WARN)).show(ui, |ui| {
            ui.colored_label(WARN, RichText::new(lang.tr("Cannot Find Settings File")).strong());
            ui.label(lang.tr_fmt("The character's settings file ({0}) could not be found.", &[&missing]));
            ui.label(lang.tr_fmt("Costs and budgets shown use {0}. Saving keeps the character's settings file unless you pick another one.", &[&fallback]));
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Import settings file…")).on_hover_text(lang.tr("Install a settings file someone shared, e.g. your GM's house rules.")).clicked() {
                    request_import(ui.ctx());
                }
                picked = picker(ui, ch, engine, lang, None);
            });
        });
        ui.add_space(4.0);
    } else if show_row {
        ui.horizontal(|ui| {
            ui.label(lang.tr("Settings File:"));
            ui.strong(lib.find(&key).or_else(|| lib.fallback()).map(CharacterSettings::name).unwrap_or_default());
            picked = picker(ui, ch, engine, lang, lib.find(&key));
        });
        ui.add_space(4.0);
    }
    picked
}

/// "Change Settings File": every installed preset. In creation mode the
/// presets with another build method are disabled, since switching build
/// methods is not ported (see `settings::switch_character`).
fn picker(ui: &mut egui::Ui, ch: &Character, engine: &Engine, lang: &Language, current: Option<&CharacterSettings>) -> Option<String> {
    let mut picked = None;
    let bm = match ch.field("buildmethod") {
        b if b.is_empty() => "Priority".to_owned(),
        b => b,
    };
    egui::ComboBox::from_id_salt("change_settings_file").selected_text(lang.tr("Change Settings File")).width(220.0).show_ui(ui, |ui| {
        for p in &engine.settings.presets {
            let same = current.is_some_and(|c| c.key() == p.key());
            let allowed = ch.created || p.build_method() == bm;
            let label = if p.file.is_some() { lang.tr_fmt("{0} (yours)", &[&p.name()]) } else { p.name() };
            let r = ui.add_enabled(allowed, egui::Button::selectable(same, label));
            let r = if allowed {
                r
            } else {
                r.on_disabled_hover_text(lang.tr_fmt(
                    "The selected build method ({0}) is different from the existing build method of the character ({1}).",
                    &[&p.build_method(), &bm],
                ))
            };
            if r.clicked() && !same {
                picked = Some(p.key());
            }
        }
    });
    picked
}
