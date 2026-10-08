//! New character wizard: build method preset, priorities, metatype,
//! magic or resonance, free talent skills.

use chummer_core::character::Character;
use chummer_core::chargen::{self, HeritageOption, NewCharacter, Priorities, TalentOption, CATEGORIES, LETTERS};
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use eframe::egui;


pub struct Wizard {
    preset: usize,
    priorities: [char; 5],
    /// The preset `priorities` were made for: a new preset gets its own
    /// defaults (Street Scum, High Life and Sum-to-Ten Improved use other
    /// letters or totals).
    priorities_for: Option<usize>,
    metatype: String,
    metavariant: String,
    talent: String,
    talent_skills: Vec<String>,
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
        Wizard {
            preset: 0,
            // Heritage, Talent, Attributes, Skills, Resources
            priorities: ['D', 'E', 'A', 'B', 'C'],
            priorities_for: None,
            metatype: "Human".into(),
            metavariant: String::new(),
            talent: "Mundane".into(),
            talent_skills: Vec::new(),
            name: String::new(),
            error: None,
            guided: crate::view::guided_offer(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> WizardResult {
        let presets = chargen::creation_presets(engine);
        if presets.is_empty() {
            return WizardResult::Cancel;
        }
        self.preset = self.preset.min(presets.len() - 1);
        let settings: CharacterSettings = presets[self.preset].clone();
        if self.priorities_for != Some(self.preset) {
            self.priorities = Priorities::default_for(&settings).0;
            self.priorities_for = Some(self.preset);
        }
        let sum_to_ten = settings.build_method() == "SumtoTen";
        let karma_build = matches!(settings.build_method().as_str(), "Karma" | "LifeModule");
        let mut result = WizardResult::Open;
        let mut open = true;
        egui::Window::new(lang.tr("New Character")).id(egui::Id::new("new_character")).open(&mut open).default_size([760.0, 640.0]).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("wiz_top").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(lang.tr("Name"));
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label(lang.tr("Rules"));
                crate::combo::Combo::from_id_salt("wiz_preset").selected_text(settings.name()).width(260.0).show_ui(ui, |ui| {
                    for (i, p) in presets.iter().enumerate() {
                        crate::combo::selectable_value(ui, &mut self.preset, i, format!("{} ({})", p.name(), p.build_method()));
                    }
                });
                ui.end_row();
            });
            ui.weak(lang.tr_fmt(
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
            ui.separator();

            if !karma_build {
            ui.heading(lang.tr("Priorities"));
            egui::Grid::new("wiz_prio").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                for (ci, cat) in CATEGORIES.iter().enumerate() {
                    ui.label(lang.tr(cat));
                    let current = self.priorities[ci];
                    crate::combo::Combo::from_id_salt(("prio", ci)).selected_text(current.to_string()).width(50.0).show_ui(ui, |ui| {
                        for l in LETTERS {
                            if crate::combo::selectable_label(ui, current == l, l.to_string()).clicked() && l != current {
                                if !sum_to_ten {
                                    // Keep each letter used once: swap with the holder.
                                    if let Some(j) = self.priorities.iter().position(|x| *x == l) {
                                        self.priorities[j] = current;
                                    }
                                }
                                self.priorities[ci] = l;
                            }
                        }
                    });
                    ui.weak(describe(engine, &settings, lang, cat, self.priorities[ci]));
                    ui.end_row();
                }
            });
            }
            let prios = Priorities(self.priorities);
            if !karma_build {
                if let Err(e) = prios.validate(&settings) {
                    ui.colored_label(crate::theme::warn(ui), e);
                }
            } else {
                ui.weak(lang.tr("Everything is bought with karma. Magic and resonance come from qualities (Magician, Adept, Technomancer, ...) added after creation starts."));
            }
            ui.separator();

            ui.columns(2, |cols| {
                // Metatype
                let ui = &mut cols[0];
                ui.heading(lang.tr("Metatype"));
                let heritage: Vec<HeritageOption> =
                    if karma_build { chargen::karma_metatypes(&engine.store) } else { chargen::heritage_options(&engine.store, &settings, prios.get("Heritage")) };
                if !heritage.iter().any(|h| h.metatype == self.metatype) {
                    if let Some(h) = heritage.first() {
                        self.metatype = h.metatype.clone();
                        self.metavariant.clear();
                    }
                }
                egui::ScrollArea::vertical().id_salt("wiz_meta").max_height(220.0).show(ui, |ui| {
                    for h in &heritage {
                        let label = if karma_build {
                            format!("{}  ·  {}", h.metatype, lang.tr_fmt("{0} karma", &[&h.karma]))
                        } else {
                            let karma = if h.karma > 0 { format!(" · {}", lang.tr_fmt("{0} karma", &[&h.karma])) } else { String::new() };
                            format!("{}  ·  {}{karma}", h.metatype, lang.tr_fmt("{0} special", &[&h.special]))
                        };
                        if crate::combo::selectable_label(ui, self.metatype == h.metatype, label).clicked() {
                            self.metatype = h.metatype.clone();
                            self.metavariant.clear();
                        }
                    }
                });
                if let Some(h) = heritage.iter().find(|h| h.metatype == self.metatype) {
                    if !h.metavariants.is_empty() {
                        crate::combo::Combo::from_id_salt("wiz_variant")
                            .selected_text(if self.metavariant.is_empty() { lang.tr("No metavariant") } else { self.metavariant.clone() })
                            .show_ui(ui, |ui| {
                                crate::combo::selectable_value(ui, &mut self.metavariant, String::new(), lang.tr("No metavariant"));
                                for (n, sp, k) in &h.metavariants {
                                    crate::combo::selectable_value(ui, &mut self.metavariant, n.clone(), format!("{n} · {} · {}", lang.tr_fmt("{0} special", &[sp]), lang.tr_fmt("{0} karma", &[k])));
                                }
                            });
                    }
                }

                // Talent
                let ui = &mut cols[1];
                if karma_build {
                    self.talent = "Mundane".into();
                    self.talent_skills.clear();
                    return;
                }
                ui.heading(lang.tr("Magic or Resonance"));
                let talents: Vec<TalentOption> = chargen::talent_options(&engine.store, &settings, prios.get("Talent"));
                let allowed: Vec<&TalentOption> = talents.iter().filter(|t| talent_allowed(t, &self.metatype)).collect();
                if !allowed.iter().any(|t| t.value == self.talent) {
                    // Mundane when offered, else a talent that needs no
                    // skill choices, so Create works straight away.
                    let pick = allowed.iter().find(|t| t.value == "Mundane").or_else(|| allowed.iter().find(|t| t.skill_qty() <= 0)).or(allowed.first());
                    if let Some(t) = pick {
                        self.talent = t.value.clone();
                        self.talent_skills.clear();
                    }
                }
                for t in &allowed {
                    if crate::combo::selectable_label(ui, self.talent == t.value, &t.display).clicked() && self.talent != t.value {
                        self.talent = t.value.clone();
                        self.talent_skills.clear();
                    }
                }
                if let Some(t) = allowed.iter().find(|t| t.value == self.talent) {
                    let qty = t.skill_qty().max(0) as usize;
                    if qty > 0 {
                        ui.add_space(6.0);
                        ui.label(if t.grouped() { lang.tr_fmt("Choose {0} skill groups at rating {1}", &[&qty, &t.skill_val()]) } else { lang.tr_fmt("Choose {0} skills at rating {1}", &[&qty, &t.skill_val()]) });
                        let options = chargen::talent_skill_options(&engine.store, t);
                        self.talent_skills.resize(qty, String::new());
                        for i in 0..qty {
                            let cur = self.talent_skills[i].clone();
                            crate::combo::Combo::from_id_salt(("tskill", i)).selected_text(if cur.is_empty() { lang.tr("Choose…") } else { cur.clone() }).width(220.0).show_ui(
                                ui,
                                |ui| {
                                    for o in &options {
                                        let taken = self.talent_skills.iter().enumerate().any(|(j, s)| j != i && s == o);
                                        if !taken && crate::combo::selectable_label(ui, cur == *o, o).clicked() {
                                            self.talent_skills[i] = o.clone();
                                        }
                                    }
                                },
                            );
                        }
                    } else {
                        self.talent_skills.clear();
                    }
                }
            });
            ui.separator();
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.checkbox(&mut self.guided, lang.tr("Guided creation")).on_hover_text(lang.tr("Walk through character creation one step at a time"));
            ui.horizontal(|ui| {
                let skills_ok = self.talent_skills.iter().all(|s| !s.is_empty());
                let ok = (karma_build || prios.validate(&settings).is_ok()) && skills_ok;
                if ui.add_enabled(ok, crate::theme::primary_button(ui, lang.tr("Create character"))).clicked() {
                    let spec = NewCharacter {
                        settings_id: settings.key(),
                        metatype: self.metatype.clone(),
                        metavariant: (!self.metavariant.is_empty()).then(|| self.metavariant.clone()),
                        priorities: prios,
                        talent: self.talent.clone(),
                        talent_skills: self.talent_skills.clone(),
                        name: if self.name.trim().is_empty() { "New Runner".into() } else { self.name.trim().to_owned() },
                    };
                    crate::view::save_guided_preference(self.guided);
                    match chargen::create(engine, &spec) {
                        Ok(ch) => result = WizardResult::Created(Box::new(ch)),
                        Err(e) => self.error = Some(e),
                    }
                }
                if ui.button(lang.tr("Cancel")).clicked() {
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

fn talent_allowed(t: &TalentOption, metatype: &str) -> bool {
    let has = |which: &str, k: &str| -> Vec<String> {
        t.node.child(which).map(|w| w.children_named("oneof").flat_map(|o| o.children_named(k)).map(|e| e.text()).collect()).unwrap_or_default()
    };
    let forbidden = has("forbidden", "metatype");
    let required = has("required", "metatype");
    !forbidden.iter().any(|m| m == metatype) && (required.is_empty() || required.iter().any(|m| m == metatype))
}

fn describe(engine: &Engine, settings: &CharacterSettings, lang: &Language, cat: &str, letter: char) -> String {
    let Some(n) = chargen::priority_node(&engine.store, settings, cat, letter) else { return String::new() };
    match cat {
        "Attributes" => lang.tr_fmt("{0} attribute points", &[&n.get("attributes")]),
        "Skills" => lang.tr_fmt("{0} skill points, {1} group points", &[&n.get("skills"), &n.get("skillgroups")]),
        "Resources" => chummer_core::format::nuyen(n.get_f64("resources").unwrap_or(0.0)),
        "Heritage" => {
            let names: Vec<String> = n.child("metatypes").map(|m| m.children_named("metatype").map(|x| x.get("name")).take(5).collect()).unwrap_or_default();
            names.join(", ")
        }
        _ => {
            let names: Vec<String> = n.child("talents").map(|m| m.children_named("talent").map(|x| x.get("value")).collect()).unwrap_or_default();
            names.join(", ")
        }
    }
}
