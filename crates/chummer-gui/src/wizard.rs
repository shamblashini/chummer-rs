//! New character wizard: build method preset, priorities, metatype,
//! magic or resonance, free talent skills.

use chummer_core::character::Character;
use chummer_core::chargen::{self, HeritageOption, NewCharacter, Priorities, TalentOption, CATEGORIES, LETTERS};
use chummer_core::engine::Engine;
use chummer_core::settings::CharacterSettings;
use eframe::egui::{self, RichText};

use crate::view::{ACCENT, WARN};

pub struct Wizard {
    preset: usize,
    priorities: [char; 5],
    metatype: String,
    metavariant: String,
    talent: String,
    talent_skills: Vec<String>,
    name: String,
    error: Option<String>,
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
            metatype: "Human".into(),
            metavariant: String::new(),
            talent: "Mundane".into(),
            talent_skills: Vec::new(),
            name: String::new(),
            error: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine) -> WizardResult {
        let presets = chargen::creation_presets(engine);
        if presets.is_empty() {
            return WizardResult::Cancel;
        }
        self.preset = self.preset.min(presets.len() - 1);
        let settings: CharacterSettings = presets[self.preset].clone();
        let sum_to_ten = settings.build_method() == "SumtoTen";
        let karma_build = matches!(settings.build_method().as_str(), "Karma" | "LifeModule");
        let mut result = WizardResult::Open;
        let mut open = true;
        egui::Window::new("New character").open(&mut open).default_size([760.0, 640.0]).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("wiz_top").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label("Rules");
                egui::ComboBox::from_id_salt("wiz_preset").selected_text(settings.name()).width(260.0).show_ui(ui, |ui| {
                    for (i, p) in presets.iter().enumerate() {
                        ui.selectable_value(&mut self.preset, i, format!("{} ({})", p.name(), p.build_method()));
                    }
                });
                ui.end_row();
            });
            ui.weak(format!(
                "Build: {} · {} karma · availability {}",
                if karma_build {
                    if settings.build_method() == "LifeModule" { "Life modules (karma)".to_owned() } else { "Point buy (karma)".to_owned() }
                } else if sum_to_ten {
                    format!("Sum-to-Ten ({})", settings.int("sumtoten", 10))
                } else {
                    "Priority".to_owned()
                },
                settings.int("buildpoints", 25),
                settings.max_availability()
            ));
            ui.separator();

            if !karma_build {
            ui.heading("Priorities");
            egui::Grid::new("wiz_prio").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                for (ci, cat) in CATEGORIES.iter().enumerate() {
                    ui.label(*cat);
                    let current = self.priorities[ci];
                    egui::ComboBox::from_id_salt(("prio", ci)).selected_text(current.to_string()).width(50.0).show_ui(ui, |ui| {
                        for l in LETTERS {
                            if ui.selectable_label(current == l, l.to_string()).clicked() && l != current {
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
                    ui.weak(describe(engine, &settings, cat, self.priorities[ci]));
                    ui.end_row();
                }
            });
            }
            let prios = Priorities(self.priorities);
            if !karma_build {
                if let Err(e) = prios.validate(&settings) {
                    ui.colored_label(WARN, e);
                }
            } else {
                ui.weak("Everything is bought with karma. Magic and resonance come from qualities (Magician, Adept, Technomancer, ...) added after creation starts.");
            }
            ui.separator();

            ui.columns(2, |cols| {
                // Metatype
                let ui = &mut cols[0];
                ui.heading("Metatype");
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
                            format!("{}  ·  {} karma", h.metatype, h.karma)
                        } else {
                            format!("{}  ·  {} special{}", h.metatype, h.special, if h.karma > 0 { format!(" · {} karma", h.karma) } else { String::new() })
                        };
                        if ui.selectable_label(self.metatype == h.metatype, label).clicked() {
                            self.metatype = h.metatype.clone();
                            self.metavariant.clear();
                        }
                    }
                });
                if let Some(h) = heritage.iter().find(|h| h.metatype == self.metatype) {
                    if !h.metavariants.is_empty() {
                        egui::ComboBox::from_id_salt("wiz_variant")
                            .selected_text(if self.metavariant.is_empty() { "No metavariant".to_owned() } else { self.metavariant.clone() })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.metavariant, String::new(), "No metavariant");
                                for (n, sp, k) in &h.metavariants {
                                    ui.selectable_value(&mut self.metavariant, n.clone(), format!("{n} · {sp} special · {k} karma"));
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
                ui.heading("Magic or Resonance");
                let talents: Vec<TalentOption> = chargen::talent_options(&engine.store, &settings, prios.get("Talent"));
                let allowed: Vec<&TalentOption> = talents.iter().filter(|t| talent_allowed(t, &self.metatype)).collect();
                if !allowed.iter().any(|t| t.value == self.talent) {
                    if let Some(t) = allowed.first() {
                        self.talent = t.value.clone();
                        self.talent_skills.clear();
                    }
                }
                for t in &allowed {
                    if ui.selectable_label(self.talent == t.value, &t.display).clicked() && self.talent != t.value {
                        self.talent = t.value.clone();
                        self.talent_skills.clear();
                    }
                }
                if let Some(t) = allowed.iter().find(|t| t.value == self.talent) {
                    let qty = t.skill_qty().max(0) as usize;
                    if qty > 0 {
                        ui.add_space(6.0);
                        ui.label(format!("Choose {qty} {} at rating {}", if t.grouped() { "skill groups" } else { "skills" }, t.skill_val()));
                        let options = chargen::talent_skill_options(&engine.store, t);
                        self.talent_skills.resize(qty, String::new());
                        for i in 0..qty {
                            let cur = self.talent_skills[i].clone();
                            egui::ComboBox::from_id_salt(("tskill", i)).selected_text(if cur.is_empty() { "Choose…".to_owned() } else { cur.clone() }).width(220.0).show_ui(
                                ui,
                                |ui| {
                                    for o in &options {
                                        let taken = self.talent_skills.iter().enumerate().any(|(j, s)| j != i && s == o);
                                        if !taken && ui.selectable_label(cur == *o, o).clicked() {
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
            ui.horizontal(|ui| {
                let skills_ok = self.talent_skills.iter().all(|s| !s.is_empty());
                let ok = (karma_build || prios.validate(&settings).is_ok()) && skills_ok;
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Create character").color(ACCENT))).clicked() {
                    let spec = NewCharacter {
                        settings_id: settings.key(),
                        metatype: self.metatype.clone(),
                        metavariant: (!self.metavariant.is_empty()).then(|| self.metavariant.clone()),
                        priorities: prios,
                        talent: self.talent.clone(),
                        talent_skills: self.talent_skills.clone(),
                        name: if self.name.trim().is_empty() { "New Runner".into() } else { self.name.trim().to_owned() },
                    };
                    match chargen::create(engine, &spec) {
                        Ok(ch) => result = WizardResult::Created(Box::new(ch)),
                        Err(e) => self.error = Some(e),
                    }
                }
                if ui.button("Cancel").clicked() {
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

fn describe(engine: &Engine, settings: &CharacterSettings, cat: &str, letter: char) -> String {
    let Some(n) = chargen::priority_node(&engine.store, settings, cat, letter) else { return String::new() };
    match cat {
        "Attributes" => format!("{} attribute points", n.get("attributes")),
        "Skills" => format!("{} skill points, {} group points", n.get("skills"), n.get("skillgroups")),
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
