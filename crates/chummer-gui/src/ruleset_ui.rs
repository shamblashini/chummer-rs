//! Sharing house rules: importing a settings file a GM handed out, the
//! warning on characters whose preset is not installed, and Chummer's
//! "Change Settings File" for an open character.

use chummer_core::character::Character;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::{self, CharacterSettings, FileClash, ImportMode, ImportPlan, Imported};
use eframe::egui::{self, RichText};


/// Picks a settings file and installs it into the user settings directory,
/// asking when a different file of the same name is already there.
#[derive(Default)]
pub struct ImportFlow {
    pending: Option<ImportPlan>,
}

impl ImportFlow {
    /// Ask for a file (the dialog runs on its own thread; [`ImportFlow::ui`]
    /// goes on when it is picked).
    pub fn start(&mut self, ctx: &egui::Context) {
        crate::bg::dialog(ctx, IMPORT_DIALOG, || rfd::FileDialog::new().add_filter("Chummer settings", &["xml"]).add_filter("All files", &["*"]).pick_file());
    }

    /// The picked file: installed unless it clashes. `Some` when done
    /// (the library must then be reloaded on `Ok`).
    fn picked(&mut self, engine: &Engine) -> Option<Result<Imported, String>> {
        let src = crate::bg::take::<Option<std::path::PathBuf>>(IMPORT_DIALOG)??;
        let Some(dir) = settings::user_settings_dir() else { return Some(Err("no settings directory".into())) };
        let plan = match engine.settings().plan_import(&src, &dir) {
            Ok(p) => p,
            Err(e) => return Some(Err(e)),
        };
        if plan.file_clash == Some(FileClash::Different) {
            self.pending = Some(plan);
            return None;
        }
        Some(install(&plan, engine, &dir, &ImportMode::New))
    }

    /// The picked file, then the clash prompt while one is open. `Some`
    /// when an import is done.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> Option<Result<Imported, String>> {
        if let Some(done) = self.picked(engine) {
            return Some(done);
        }
        let file_name = self.pending.as_ref()?.file_name.clone();
        let mut mode = None;
        let mut cancel = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.colored_label(crate::theme::warn(ui), lang.tr_fmt("A different settings file named {0} is already installed.", &[&file_name]));
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
    settings::import(plan, &engine.settings(), dir, mode)
}

const IMPORT_REQUEST: &str = "ruleset_import_request";
const IMPORT_DIALOG: &str = "dialog:settings-import";

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
    if crate::theme::current(ui.ctx()).workspace_layout() {
        return ws_banner(ui, ch, engine, lang, show_row);
    }
    let lib = engine.settings();
    let mut picked = None;
    if let Some(missing) = lib.missing_for(ch) {
        let fallback = lib.fallback().map(CharacterSettings::name).unwrap_or_default();
        egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0_f32, crate::theme::warn(ui))).show(ui, |ui| {
            ui.colored_label(crate::theme::warn(ui), RichText::new(lang.tr("Cannot Find Settings File")).strong());
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
            ui.strong(lib.resolve_for(ch).map(|p| preset_label(p, lang)).unwrap_or_default());
            picked = picker(ui, ch, engine, lang, lib.find_for(ch));
        });
        ui.add_space(4.0);
    }
    if let Some(text) = missing_custom_data(ch, engine, lang) {
        ui.colored_label(crate::theme::warn(ui), text);
        ui.add_space(4.0);
    }
    picked
}

/// A preset's name, marked when a campaign shared it.
pub fn preset_label(p: &CharacterSettings, lang: &Language) -> String {
    match &p.campaign {
        Some(c) => lang.tr_fmt("{0} (from campaign {1})", &[&p.name(), &c.campaign]),
        None if p.is_user() => lang.tr_fmt("{0} (yours)", &[&p.name()]),
        None => p.name(),
    }
}

/// "Custom data not installed: …" when the character's preset enables
/// custom data folders this app does not have (a GM's preset, a file
/// from another machine).
pub fn missing_custom_data(ch: &Character, engine: &Engine, lang: &Language) -> Option<String> {
    let lib = engine.settings();
    let p = lib.find_for(ch)?;
    let missing = engine.missing_custom_data(p);
    if missing.is_empty() {
        return None;
    }
    let hint = if p.campaign.is_some() { lang.tr("Ask your GM for these custom data folders; until then their content is missing here.") } else { lang.tr("Its content is missing here until the folders are installed.") };
    Some(format!("{} {hint}", lang.tr_fmt("Custom data this settings file uses is not installed: {0}.", &[&missing.join(", ")])))
}

/// [`banner`] in the Workspace style: a card with a warning border and
/// icon, or a one-line settings row.
fn ws_banner(ui: &mut egui::Ui, ch: &Character, engine: &Engine, lang: &Language, show_row: bool) -> Option<String> {
    use crate::workspace::{icons, widgets};
    let ws = crate::theme::ws(ui);
    let lib = engine.settings();
    let mut picked = None;
    if let Some(missing) = lib.missing_for(ch) {
        let fallback = lib.fallback().map(CharacterSettings::name).unwrap_or_default();
        widgets::card_frame(&ws).stroke(egui::Stroke::new(1.0_f32, ws.warning)).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(icons::icon(icons::WARNING, 18.0, ws.warning));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(lang.tr("Cannot Find Settings File")).font(widgets::bold(13.0)).color(ws.warning));
                    let line = |ui: &mut egui::Ui, t: String| ui.add(egui::Label::new(RichText::new(t).size(12.0).color(ws.muted)).wrap());
                    line(ui, lang.tr_fmt("The character's settings file ({0}) could not be found.", &[&missing]));
                    line(ui, lang.tr_fmt("Costs and budgets shown use {0}. Saving keeps the character's settings file unless you pick another one.", &[&fallback]));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let r = widgets::button(ui, Some(icons::FILE_ARROW_DOWN), &lang.tr("Import settings file…"), widgets::Look::Secondary, 24.0);
                        if r.on_hover_text(lang.tr("Install a settings file someone shared, e.g. your GM's house rules.")).clicked() {
                            request_import(ui.ctx());
                        }
                        picked = picker(ui, ch, engine, lang, None);
                    });
                });
            });
        });
        ui.add_space(6.0);
    } else if show_row {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(icons::icon(icons::SLIDERS_HORIZONTAL, 14.0, ws.muted));
            ui.label(RichText::new(lang.tr("Settings File:")).size(12.0).color(ws.muted));
            ui.label(RichText::new(lib.resolve_for(ch).map(|p| preset_label(p, lang)).unwrap_or_default()).font(widgets::bold(12.5)).color(ws.text));
            picked = picker(ui, ch, engine, lang, lib.find_for(ch));
        });
        ui.add_space(6.0);
    }
    if let Some(text) = missing_custom_data(ch, engine, lang) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(icons::icon(icons::WARNING, 14.0, ws.warning));
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(ws.warning)).wrap());
        });
        ui.add_space(6.0);
    }
    picked
}

/// "Change Settings File": every installed preset; for a campaign's
/// character, the presets the campaign shared and the built-ins (the GM's
/// app must know the preset too). In creation mode the presets with
/// another build method are disabled, since switching build methods is not
/// ported (see `settings::switch_character`).
fn picker(ui: &mut egui::Ui, ch: &Character, engine: &Engine, lang: &Language, current: Option<&CharacterSettings>) -> Option<String> {
    let mut picked = None;
    let bm = match ch.field("buildmethod") {
        b if b.is_empty() => "Priority".to_owned(),
        b => b,
    };
    let lib = engine.settings();
    let presets: Vec<&CharacterSettings> = match ch.settings_scope.as_deref() {
        Some(scope) => {
            let shared: Vec<&CharacterSettings> = lib.campaign_presets(scope).collect();
            let builtins = lib.presets.iter().filter(|p| p.is_builtin() && !shared.iter().any(|s| s.key() == p.key()));
            shared.iter().copied().chain(builtins).collect()
        }
        None => lib.presets.iter().collect(),
    };
    crate::combo::Combo::from_id_salt("change_settings_file").selected_text(lang.tr("Change Settings File")).width(220.0).show_ui(ui, |ui| {
        for p in presets {
            let same = current.is_some_and(|c| c.key() == p.key());
            let allowed = ch.created || p.build_method() == bm;
            let label = preset_label(p, lang);
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
