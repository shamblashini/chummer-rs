//! Choosing the metatype, priorities, talent and talent skills: the body
//! of the New Character wizard and of "Change Priority Selection" /
//! "Change Metatype" for a character in creation (Chummer's
//! `SelectMetatypePriority` and `SelectMetatypeKarma`, which the menu item
//! opens again on the existing character).

use chummer_core::chargen::rebuild::{talent_allowed, Choice};
use chummer_core::chargen::{self, HeritageOption, Priorities, TalentOption, CATEGORIES, LETTERS};
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use eframe::egui;

use crate::workspace::dialog;

/// The choices being made.
#[derive(Debug, Clone)]
pub struct ChoicePicker {
    /// Heritage, Talent, Attributes, Skills, Resources.
    pub priorities: [char; 5],
    pub metatype: String,
    pub metavariant: String,
    pub talent: String,
    pub talent_skills: Vec<String>,
}

impl Default for ChoicePicker {
    fn default() -> Self {
        ChoicePicker { priorities: ['D', 'E', 'A', 'B', 'C'], metatype: "Human".into(), metavariant: String::new(), talent: "Mundane".into(), talent_skills: Vec::new() }
    }
}

impl ChoicePicker {
    /// Starting from a character's current choice.
    pub fn from_choice(c: &Choice) -> ChoicePicker {
        ChoicePicker {
            priorities: c.priorities.map_or(ChoicePicker::default().priorities, |p| p.0),
            metatype: c.metatype.clone(),
            metavariant: c.metavariant.clone().unwrap_or_default(),
            talent: if c.talent.is_empty() { "Mundane".into() } else { c.talent.clone() },
            talent_skills: c.talent_skills.clone(),
        }
    }

    /// The choice as the command takes it.
    pub fn choice(&self, karma_build: bool) -> Choice {
        Choice {
            metatype: self.metatype.clone(),
            metavariant: Some(self.metavariant.clone()).filter(|v| !v.is_empty()),
            priorities: (!karma_build).then_some(Priorities(self.priorities)),
            talent: if karma_build { String::new() } else { self.talent.clone() },
            talent_skills: if karma_build { Vec::new() } else { self.talent_skills.clone() },
        }
    }

    /// Whether the choice can be taken: valid letters and every talent
    /// skill picked.
    pub fn ready(&self, settings: &CharacterSettings) -> bool {
        let karma_build = karma_build(settings);
        (karma_build || Priorities(self.priorities).validate(settings).is_ok()) && self.talent_skills.iter().all(|s| !s.is_empty())
    }

    /// The priorities grid (priority builds), then the metatype list and
    /// the talent with its skills side by side.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Engine, settings: &CharacterSettings, lang: &Language) {
        let sum_to_ten = settings.build_method() == "SumtoTen";
        let karma_build = karma_build(settings);
        if !karma_build {
            dialog::heading(ui, &lang.tr("Priorities"));
            egui::Grid::new("wiz_prio").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                for (ci, cat) in CATEGORIES.iter().enumerate() {
                    dialog::label(ui, &lang.tr(cat));
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
                    dialog::note(ui, describe(engine, settings, lang, cat, self.priorities[ci]));
                    ui.end_row();
                }
            });
        }
        let prios = Priorities(self.priorities);
        if !karma_build {
            if let Err(e) = prios.validate(settings) {
                dialog::warning(ui, e);
            }
        } else {
            dialog::note(ui, lang.tr("Everything is bought with karma. Magic and resonance come from qualities (Magician, Adept, Technomancer, ...) added after creation starts."));
        }
        dialog::rule(ui);

        ui.columns(2, |cols| {
            // Metatype
            let ui = &mut cols[0];
            dialog::heading(ui, &lang.tr("Metatype"));
            let heritage: Vec<HeritageOption> = if karma_build { chargen::karma_metatypes(&engine.store) } else { chargen::heritage_options(&engine.store, settings, prios.get("Heritage")) };
            if !heritage.iter().any(|h| h.metatype == self.metatype) {
                if let Some(h) = heritage.first() {
                    self.metatype = h.metatype.clone();
                    self.metavariant.clear();
                }
            }
            dialog::list_frame(ui).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("wiz_meta").max_height(220.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = if dialog::ws(ui.ctx()) { 1.0 } else { ui.spacing().item_spacing.y };
                    for h in &heritage {
                        let extra = if karma_build {
                            lang.tr_fmt("{0} karma", &[&h.karma])
                        } else if h.karma > 0 {
                            format!("{} · {}", lang.tr_fmt("{0} special", &[&h.special]), lang.tr_fmt("{0} karma", &[&h.karma]))
                        } else {
                            lang.tr_fmt("{0} special", &[&h.special])
                        };
                        if dialog::list_row(ui, self.metatype == h.metatype, &h.metatype, &extra, false).clicked() {
                            self.metatype = h.metatype.clone();
                            self.metavariant.clear();
                        }
                    }
                });
            });
            if let Some(h) = heritage.iter().find(|h| h.metatype == self.metatype) {
                if !h.metavariants.is_empty() {
                    ui.add_space(4.0);
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
            dialog::heading(ui, &lang.tr("Magic or Resonance"));
            let talents: Vec<TalentOption> = chargen::talent_options(&engine.store, settings, prios.get("Talent"));
            let allowed: Vec<&TalentOption> = talents.iter().filter(|t| talent_allowed(t, &self.metatype)).collect();
            if !allowed.iter().any(|t| t.value == self.talent) {
                // Mundane when offered, else a talent that needs no
                // skill choices, so the choice works straight away.
                let pick = allowed.iter().find(|t| t.value == "Mundane").or_else(|| allowed.iter().find(|t| t.skill_qty() <= 0)).or(allowed.first());
                if let Some(t) = pick {
                    self.talent = t.value.clone();
                    self.talent_skills.clear();
                }
            }
            dialog::list_frame(ui).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = if dialog::ws(ui.ctx()) { 1.0 } else { ui.spacing().item_spacing.y };
                for t in &allowed {
                    if dialog::list_row(ui, self.talent == t.value, &t.display, "", false).clicked() && self.talent != t.value {
                        self.talent = t.value.clone();
                        self.talent_skills.clear();
                    }
                }
            });
            if let Some(t) = allowed.iter().find(|t| t.value == self.talent) {
                let qty = t.skill_qty().max(0) as usize;
                if qty > 0 {
                    ui.add_space(6.0);
                    dialog::label(ui, &if t.grouped() { lang.tr_fmt("Choose {0} skill groups at rating {1}", &[&qty, &t.skill_val()]) } else { lang.tr_fmt("Choose {0} skills at rating {1}", &[&qty, &t.skill_val()]) });
                    let options = chargen::talent_skill_options(&engine.store, t);
                    self.talent_skills.resize(qty, String::new());
                    for i in 0..qty {
                        let cur = self.talent_skills[i].clone();
                        crate::combo::Combo::from_id_salt(("tskill", i)).selected_text(if cur.is_empty() { lang.tr("Choose…") } else { cur.clone() }).width(220.0).show_ui(ui, |ui| {
                            for o in &options {
                                let taken = self.talent_skills.iter().enumerate().any(|(j, s)| j != i && s == o);
                                if !taken && crate::combo::selectable_label(ui, cur == *o, o).clicked() {
                                    self.talent_skills[i] = o.clone();
                                }
                            }
                        });
                    }
                } else {
                    self.talent_skills.clear();
                }
            }
        });
    }
}

/// Karma and Life Module builds buy the metatype with karma.
pub fn karma_build(settings: &CharacterSettings) -> bool {
    matches!(settings.build_method().as_str(), "Karma" | "LifeModule")
}

/// The label of the menu item / button that opens [`ChangeDialog`]:
/// Chummer's "Change Priority Selection" for priority builds, "Change
/// Metatype" for karma builds.
pub fn change_label(lang: &Language, buildmethod: &str) -> String {
    if chummer_core::character::uses_priority_tables(buildmethod) {
        lang.tr("Change Priority Selection…")
    } else {
        lang.tr("Change Metatype…")
    }
}

/// What the priorities of a category give at a letter, in a few words.
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

/// "Change Priority Selection" / "Change Metatype" for a character in
/// creation: the picker, started from the character's choice (or a
/// proposed one, like two swapped priorities), and OK / Cancel.
pub struct ChangeDialog {
    picker: ChoicePicker,
    /// Why the last OK (or the swap that opened the dialog) was refused.
    pub error: Option<String>,
}

/// What [`ChangeDialog::show`] returned this frame.
pub enum ChangeOutcome {
    None,
    Cancel,
    Apply(Choice),
}

impl ChangeDialog {
    pub fn new(start: &Choice, error: Option<String>) -> ChangeDialog {
        ChangeDialog { picker: ChoicePicker::from_choice(start), error }
    }

    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine, settings: &CharacterSettings, buildmethod: &str, lang: &Language) -> ChangeOutcome {
        let mut open = true;
        let mut out = ChangeOutcome::None;
        let title = change_label(lang, buildmethod).trim_end_matches('…').to_owned();
        let karma = karma_build(settings);
        dialog::window(ctx, "change_metatype", &title, &mut open, egui::vec2(760.0, 600.0), true, |ui| {
            dialog::note(ui, lang.tr("Points, karma and items already spent stay; attributes are cut to the new maximums. Undo takes the change back."));
            ui.add_space(4.0);
            self.picker.ui(ui, engine, settings, lang);
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                dialog::warning(ui, e.clone());
            }
            dialog::buttons(ui, |ui| {
                let ok = ui.add_enabled_ui(self.picker.ready(settings), |ui| dialog::button(ui, &lang.tr("OK"), true)).inner;
                if ok.clicked() {
                    out = ChangeOutcome::Apply(self.picker.choice(karma));
                }
                if dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                    out = ChangeOutcome::Cancel;
                }
            });
        });
        if !open {
            return ChangeOutcome::Cancel;
        }
        out
    }
}
