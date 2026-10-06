//! The GM screen's forms: quick-add (character files, critters, PACKS kit
//! NPCs), copies, GM awards, quick damage and dice rolls. `gm_screen`
//! lays them out; this file draws them and turns them into campaign
//! changes or commands.

use chummer_core::campaign::damage::{self, Attack, Defender, Tracks};
use chummer_core::career::ManualExpense;
use chummer_core::command::Command;
use chummer_core::dice::{self, Rng};
use chummer_core::engine::Engine;
use chummer_core::gm::packs;
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

/// Pick character files; empty when cancelled.
pub fn pick_characters() -> Vec<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Chummer character", &["chum5", "chum5lz"])
        .add_filter("All files", &["*"])
        .pick_files()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// NPC from a PACKS kit
// ---------------------------------------------------------------------------

pub struct KitForm {
    /// `packs.xml` with custom kits.
    doc: Element,
    category: String,
    kit: Option<(String, String)>,
    metatype: String,
    name: String,
    count: u32,
    error: Option<String>,
}

/// What the kit window asks for.
pub struct KitRequest {
    pub kit_xml: String,
    pub metatype: String,
    pub name: String,
    pub count: u32,
}

impl KitForm {
    pub fn new(engine: &Engine) -> KitForm {
        KitForm {
            doc: packs::load(&engine.store, packs::packs_dir().as_deref()),
            category: String::new(),
            kit: None,
            metatype: "Human".into(),
            name: String::new(),
            count: 1,
            error: None,
        }
    }

    pub fn fail(&mut self, e: String) {
        self.error = Some(e);
    }

    /// Draw the window. `Some(None)` closes it; `Some(Some(r))` asks for
    /// NPCs.
    pub fn window(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> Option<Option<KitRequest>> {
        let mut out = None;
        let mut open = true;
        let kits = packs::kits(&self.doc);
        let cats: Vec<String> = packs::categories(&self.doc).into_iter().filter(|c| kits.iter().any(|(_, k)| k == c)).collect();
        if !cats.contains(&self.category) {
            self.category = cats.first().cloned().unwrap_or_default();
        }
        egui::Window::new(lang.tr("Select a PACKS Kit")).id(egui::Id::new("gm_kit_npc")).open(&mut open).default_size([620.0, 480.0]).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("gm_kit_top").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(lang.tr("Category"));
                crate::combo::Combo::from_id_salt("gm_kit_cat").selected_text(self.category.clone()).width(240.0).show_ui(ui, |ui| {
                    for c in &cats {
                        if crate::combo::selectable_label(ui, *c == self.category, c).clicked() {
                            self.category = c.clone();
                            self.kit = None;
                        }
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Metatype"));
                crate::combo::Combo::from_id_salt("gm_kit_metatype").selected_text(self.metatype.clone()).width(240.0).show_ui(ui, |ui| {
                    for h in chummer_core::chargen::karma_metatypes(&engine.store) {
                        crate::combo::selectable_value(ui, &mut self.metatype, h.metatype.clone(), h.metatype);
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Name"));
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(self.kit.as_ref().map(|k| k.0.clone()).unwrap_or_default()).desired_width(240.0));
                ui.end_row();
                ui.label(lang.tr("Count"));
                ui.add(egui::DragValue::new(&mut self.count).range(1..=20));
                ui.end_row();
            });
            ui.separator();
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical().id_salt("gm_kit_list").max_height(280.0).show(&mut cols[0], |ui| {
                    for (n, c) in kits.iter().filter(|(_, c)| *c == self.category) {
                        let sel = self.kit.as_ref().is_some_and(|(sn, sc)| sn == n && sc == c);
                        if crate::combo::selectable_label(ui, sel, n).clicked() {
                            self.kit = Some((n.clone(), c.clone()));
                        }
                    }
                });
                let ui = &mut cols[1];
                let Some(kit) = self.kit.as_ref().and_then(|(n, c)| packs::find_kit(&self.doc, n, c)) else {
                    ui.weak(lang.tr("Choose a kit."));
                    return;
                };
                egui::ScrollArea::vertical().id_salt("gm_kit_contents").max_height(280.0).show(ui, |ui| {
                    for (section, lines) in packs::contents(kit) {
                        ui.label(RichText::new(lang.tr(section)).color(crate::theme::accent(ui)));
                        for l in lines {
                            ui.label(format!("  {l}"));
                        }
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                let kit = self.kit.as_ref().and_then(|(n, c)| packs::find_kit(&self.doc, n, c));
                if ui.add_enabled(kit.is_some(), crate::theme::primary_button(ui, lang.tr("Add"))).clicked() {
                    let kit = kit.expect("enabled");
                    let name = if self.name.trim().is_empty() { self.kit.as_ref().map(|k| k.0.clone()).unwrap_or_default() } else { self.name.trim().to_owned() };
                    out = Some(Some(KitRequest { kit_xml: kit.to_xml_string(), metatype: self.metatype.clone(), name, count: self.count }));
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    out = Some(None);
                }
            });
        });
        if !open {
            out = Some(None);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// GM awards
// ---------------------------------------------------------------------------

/// "GM gave you 100 karma: note".
pub struct AwardForm {
    pub karma: bool,
    pub amount: f64,
    pub note: String,
}

impl Default for AwardForm {
    fn default() -> Self {
        AwardForm { karma: true, amount: 5.0, note: String::new() }
    }
}

impl AwardForm {
    /// Draw the form; returns the command to run (give or take).
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language, career: bool) -> Option<Command> {
        let mut out = None;
        ui.add_enabled_ui(career, |ui| {
            ui.horizontal(|ui| {
                crate::combo::selectable_value(ui, &mut self.karma, true, lang.tr("Karma"));
                crate::combo::selectable_value(ui, &mut self.karma, false, lang.tr("Nuyen"));
                let (step, max) = if self.karma { (1.0, 1000.0) } else { (100.0, 10_000_000.0) };
                ui.add(egui::DragValue::new(&mut self.amount).range(0.0..=max).speed(step));
            });
            ui.add(egui::TextEdit::singleline(&mut self.note).hint_text(lang.tr("Reason")).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                let ok = self.amount > 0.0;
                let cmd = |gain: bool| Command::ManualExpense { karma: self.karma, gain, expense: ManualExpense { amount: self.amount, reason: self.note.trim().to_owned(), ..Default::default() } };
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Give"))).clicked() {
                    out = Some(cmd(true));
                }
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Take"))).clicked() {
                    out = Some(cmd(false));
                }
            });
        });
        if !career {
            ui.weak(lang.tr("Awards are for characters in Career Mode."));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Quick damage
// ---------------------------------------------------------------------------

pub struct DamageForm {
    pub code: String,
    pub soak_roll: bool,
}

impl Default for DamageForm {
    fn default() -> Self {
        DamageForm { code: "6P".into(), soak_roll: true }
    }
}

/// What a damage entry did.
pub struct DamageResult {
    pub physical_filled: i32,
    pub stun_filled: i32,
    /// For the feed: "took 8P AP-2: soaked 3 (14 dice), 5 Stun".
    pub text: String,
}

impl DamageForm {
    /// Draw the entry; `Some(attack)` when Apply was clicked with a valid
    /// code.
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language) -> Option<Attack> {
        let mut out = None;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.code).hint_text("8P AP-2").desired_width(90.0));
            let parsed = Attack::parse(&self.code);
            if parsed.is_none() && !self.code.trim().is_empty() {
                r.on_hover_text(lang.tr("Damage code, e.g. 8P AP-2 or 6S"));
            }
            ui.checkbox(&mut self.soak_roll, lang.tr("Soak roll"));
            if ui.add_enabled(parsed.is_some(), egui::Button::new(lang.tr("Apply"))).clicked() {
                out = parsed;
            }
        });
        out
    }
}

/// Resolve an attack against a defender and its tracks: soak (rolled or
/// not), conversion to Stun, overflow.
pub fn resolve(rng: &mut Rng, a: Attack, d: Defender, t: Tracks, roll_soak: bool) -> DamageResult {
    let inc = damage::incoming(a, d);
    let (hits, soak) = if roll_soak {
        let r = dice::roll(rng, inc.soak_pool.max(0) as u32, false, None);
        (r.hits, format!(", soaked {} of {} dice", r.hits, inc.soak_pool))
    } else {
        (0, String::new())
    };
    let boxes = damage::after_soak(inc, hits);
    let (p, s) = damage::apply(t, boxes, inc.physical);
    let kind = if inc.physical { "Physical" } else { "Stun" };
    let ap = if a.ap != 0 { format!(" AP{:+}", a.ap) } else { String::new() };
    let conv = if inc.converted { ", Stun (DV below armor)" } else { "" };
    let result = if boxes == 0 { "no damage".to_owned() } else { format!("{boxes} {kind}") };
    DamageResult { physical_filled: p, stun_filled: s, text: format!("took {}{}{ap}{conv}{soak}: {result}", a.dv, if a.physical { "P" } else { "S" }) }
}

/// A pool as a chip; click rolls it. Returns a line for the roll log.
pub fn pool_roll(ui: &mut egui::Ui, rng: &mut Rng, lang: &Language, who: &str, label: &str, pool: i32) -> Option<String> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(label);
        if crate::theme::pool_chip(ui, pool.to_string()).on_hover_text(lang.tr("Roll")).clicked() {
            let r = dice::roll(rng, pool.max(0) as u32, false, None);
            let glitch = match r.glitch {
                dice::Glitch::None => String::new(),
                dice::Glitch::Glitch => format!(" — {}", lang.tr("GLITCH")),
                dice::Glitch::Critical => format!(" — {}", lang.tr("CRITICAL GLITCH")),
            };
            let dice: Vec<String> = r.dice.iter().map(u8::to_string).collect();
            out = Some(format!("{who}: {label} {pool}d6 → {}{glitch}  [{}]", lang.tr_fmt("{0} hits", &[&r.hits]), dice.join(" ")));
        }
    });
    out
}
