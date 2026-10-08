//! New character wizard: build method preset, priorities, metatype,
//! magic or resonance, free talent skills (the choices are
//! [`crate::metatype_ui::ChoicePicker`], shared with "Change Priority
//! Selection").

use chummer_core::character::Character;
use chummer_core::chargen::{self, NewCharacter, Priorities};
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use eframe::egui;

use crate::metatype_ui::ChoicePicker;
use crate::workspace::dialog;

pub struct Wizard {
    preset: usize,
    /// The preset the priorities were made for: a new preset gets its own
    /// defaults (Street Scum, High Life and Sum-to-Ten Improved use other
    /// letters or totals).
    priorities_for: Option<usize>,
    picker: ChoicePicker,
    name: String,
    error: Option<String>,
    /// Open the character with guided creation (saved as the preference).
    guided: bool,
}

pub enum WizardResult {
    Open,
    Cancel,
    Created(Box<Character>),
}

impl Wizard {
    pub fn new() -> Self {
        Wizard { preset: 0, priorities_for: None, picker: ChoicePicker::default(), name: String::new(), error: None, guided: crate::view::guided_offer() }
    }

    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> WizardResult {
        let presets = chargen::creation_presets(engine);
        if presets.is_empty() {
            return WizardResult::Cancel;
        }
        self.preset = self.preset.min(presets.len() - 1);
        let settings: CharacterSettings = presets[self.preset].clone();
        if self.priorities_for != Some(self.preset) {
            self.picker.priorities = Priorities::default_for(&settings).0;
            self.priorities_for = Some(self.preset);
        }
        let sum_to_ten = settings.build_method() == "SumtoTen";
        let karma_build = crate::metatype_ui::karma_build(&settings);
        let mut result = WizardResult::Open;
        let mut open = true;
        dialog::window(ctx, "new_character", &lang.tr("New Character"), &mut open, egui::vec2(760.0, 640.0), true, |ui| {
            egui::Grid::new("wiz_top").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                dialog::label(ui, &lang.tr("Name"));
                dialog::text_input(ui, &mut self.name, "", 260.0);
                ui.end_row();
                dialog::label(ui, &lang.tr("Rules"));
                crate::combo::Combo::from_id_salt("wiz_preset").selected_text(settings.name()).width(260.0).show_ui(ui, |ui| {
                    for (i, p) in presets.iter().enumerate() {
                        crate::combo::selectable_value(ui, &mut self.preset, i, format!("{} ({})", p.name(), p.build_method()));
                    }
                });
                ui.end_row();
            });
            dialog::note(ui, lang.tr_fmt(
                "Build: {0} · {1} karma · availability {2}",
                &[
                    &if karma_build {
                        if settings.build_method() == "LifeModule" { lang.tr("Life modules (karma)") } else { lang.tr("Point buy (karma)") }
                    } else if sum_to_ten {
                        format!("{} ({})", lang.tr("Sum-to-Ten"), settings.int("sumtoten", 10))
                    } else {
                        lang.tr("Priority")
                    },
                    &settings.int("buildpoints", 25),
                    &settings.max_availability(),
                ],
            ));
            dialog::rule(ui);
            self.picker.ui(ui, engine, &settings, lang);
            dialog::rule(ui);
            if let Some(e) = &self.error {
                dialog::warning(ui, e.clone());
            }
            dialog::check(ui, &mut self.guided, &lang.tr("Guided creation")).on_hover_text(lang.tr("Walk through character creation one step at a time"));
            dialog::buttons(ui, |ui| {
                let ok = self.picker.ready(&settings);
                let create = if dialog::ws(ui.ctx()) { ui.add_enabled_ui(ok, |ui| dialog::button(ui, &lang.tr("Create character"), true)).inner } else { ui.add_enabled(ok, crate::theme::primary_button(ui, lang.tr("Create character"))) };
                if create.clicked() {
                    let p = &self.picker;
                    let spec = NewCharacter {
                        settings_id: settings.key(),
                        metatype: p.metatype.clone(),
                        metavariant: (!p.metavariant.is_empty()).then(|| p.metavariant.clone()),
                        priorities: Priorities(p.priorities),
                        talent: p.talent.clone(),
                        talent_skills: p.talent_skills.clone(),
                        name: if self.name.trim().is_empty() { "New Runner".into() } else { self.name.trim().to_owned() },
                    };
                    crate::view::save_guided_preference(self.guided);
                    match chargen::create(engine, &spec) {
                        Ok(ch) => result = WizardResult::Created(Box::new(ch)),
                        Err(e) => self.error = Some(e),
                    }
                }
                if dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                    result = WizardResult::Cancel;
                }
            });
        });
        if !open {
            return WizardResult::Cancel;
        }
        result
    }
}
