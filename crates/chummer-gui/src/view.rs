//! One open character: its tabs and the stats sidebar.

use std::path::PathBuf;
use std::sync::Arc;

use chummer_core::attributes;
use chummer_core::calc::{self, Rules, Sheet};
use chummer_core::character::{Character, INFO_FIELDS, TEXT_FIELDS};
use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section};
use chummer_core::sources::{SourceRef, SourcebookLibrary};

use chummer_core::career;
use chummer_core::chargen;
use chummer_core::data;
use chummer_core::settings::CharacterSettings;

use crate::pdf_ui::{self, Status};
use crate::select::{self, SelectDialog};
use chummer_core::xml::Element;
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Info,
    Attributes,
    Skills,
    Qualities,
    Magic,
    Equipment,
    Improvements,
    Log,
    Notes,
}

pub(crate) const TABS: &[(Tab, &str)] = &[
    (Tab::Info, "Info"),
    (Tab::Attributes, "Attributes"),
    (Tab::Skills, "Skills"),
    (Tab::Qualities, "Qualities & Contacts"),
    (Tab::Magic, "Magic, Resonance & Martial Arts"),
    (Tab::Equipment, "Equipment"),
    (Tab::Improvements, "Improvements"),
    (Tab::Log, "Karma & Nuyen"),
    (Tab::Notes, "Notes"),
];

pub struct CharacterView {
    pub ch: Character,
    /// Game data with the character's custom data applied.
    store: Arc<chummer_core::data::DataStore>,
    pub sheet: Sheet,
    pub rules: Rules,
    tab: Tab,
    skill_filter: String,
    only_rated: bool,
    equipment: usize,
    magic: usize,
    /// (container, guid, name) of an item waiting for removal confirmation.
    confirm_remove: Option<(String, String, String)>,
    /// Creation-mode budget, recomputed with the sheet.
    budget: Option<chargen::Budget>,
    problems: Vec<String>,
    settings: Option<CharacterSettings>,
    select: Option<SelectDialog>,
    confirm_finish: bool,
    new_kno: (String, String, bool),
    new_contact: (String, String, i32, i32),
    /// Life module picker: (stage, module id, version id).
    life: (String, String, String),
    action: Option<CareerAction>,
    /// Manual ledger entry: (karma?, amount, reason).
    manual: (bool, f64, String),
    initiation: career::InitiationOptions,
    // Magic, lifestyle and drug editors (magic_ui, lifestyle_ui, drug_ui).
    magic_editor: crate::magic_ui::MagicEditor,
    lifestyle_editor: crate::lifestyle_ui::LifestyleEditor,
    drug_builder: crate::drug_ui::DrugBuilder,
    /// Item detail pane: (selected item guid, editor).
    item_editor: Option<(String, crate::item_editor::ItemEditor)>,
}

/// A career-mode purchase chosen while drawing, run afterwards (it needs
/// the engine and may fail with "not enough karma").
#[derive(Debug, Clone)]
enum CareerAction {
    RaiseAttribute(String),
    RaiseSkill(String),
    RaiseGroup(String),
    Specialize(String, String),
    LearnKnowledge(String, String),
    Undo(String),
    Initiate(career::InitiationOptions),
    RemoveQuality(String),
}

pub const ACCENT: Color32 = Color32::from_rgb(0, 200, 170);
pub const WARN: Color32 = Color32::from_rgb(230, 170, 60);

impl Tab {
    pub fn parse(s: &str) -> Option<Tab> {
        let s = s.to_ascii_lowercase();
        TABS.iter().find(|(_, label)| label.to_ascii_lowercase().starts_with(&s)).map(|(t, _)| *t)
    }
}

impl CharacterView {
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    pub fn new(ch: Character, engine: &Engine) -> Self {
        let rules = engine.rules_for(&ch);
        let store = engine.store_for_character(&ch);
        let sheet = calc::compute(&ch, &rules, Some(&store), Some(&engine.catalog));
        let settings = engine.settings.resolve(&ch.field("settings")).cloned();
        let mut v = CharacterView {
            store,
            ch,
            sheet,
            rules,
            tab: Tab::Info,
            skill_filter: String::new(),
            only_rated: false,
            equipment: 0,
            magic: 0,
            confirm_remove: None,
            budget: None,
            problems: Vec::new(),
            settings,
            select: None,
            confirm_finish: false,
            new_kno: (String::new(), "Academic".into(), false),
            new_contact: (String::new(), String::new(), 1, 1),
            life: (String::new(), String::new(), String::new()),
            action: None,
            manual: (true, 0.0, String::new()),
            initiation: career::InitiationOptions::default(),
            magic_editor: Default::default(),
            lifestyle_editor: Default::default(),
            drug_builder: Default::default(),
            item_editor: None,
        };
        v.refresh_budget();
        v
    }

    fn refresh_budget(&mut self) {
        match (&self.settings, self.ch.created) {
            (Some(st), false) => {
                let b = chargen::budget_with(&self.ch, &self.sheet, &self.rules, st, Some(&self.store));
                self.problems = chargen::validity_problems(&self.ch, &b, st);
                self.budget = Some(b);
            }
            _ => {
                self.budget = None;
                self.problems.clear();
            }
        }
    }

    pub fn title(&self) -> String {
        let name = self.ch.display_name();
        if self.ch.dirty {
            format!("{name} •")
        } else {
            name
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.ch.file.clone()
    }

    fn recompute(&mut self, engine: &Engine) {
        if !self.ch.created {
            // Essence loss in creation follows the ware installed now.
            chummer_core::essence_loss::refresh(&mut self.ch, &self.store, &self.rules);
        }
        self.sheet = calc::compute(&self.ch, &self.rules, Some(&self.store), Some(&engine.catalog));
        self.refresh_budget();
    }

    pub fn ui(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<u32> {
        let mut changed = false;
        let mut roll: Option<u32> = None;
        egui::SidePanel::right("sheet_panel").resizable(true).default_width(270.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                changed |= self.sidebar(ui, lang, &mut roll);
            });
        });
        changed |= self.item_editor_panel(ctx, engine, lang, status);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (t, label) in TABS {
                    ui.selectable_value(&mut self.tab, *t, lang.tr(label));
                }
            });
            ui.separator();
            changed |= match self.tab {
                Tab::Info => self.info_tab(ui, lang),
                Tab::Attributes => self.attributes_tab(ui, engine, lang),
                Tab::Skills => self.skills_tab(ui, engine, lang, pdfs, status, &mut roll),
                Tab::Qualities => {
                    let mut c = false;
                    egui::ScrollArea::both().show(ui, |ui| {
                        if ui.button(format!("➕ {}", lang.tr("Add Quality…"))).clicked() {
                            self.open_select("quality", engine);
                        }
                        if !self.ch.created && self.ch.field("buildmethod") == "LifeModule" {
                            c |= self.life_module_picker(ui, engine, lang, status);
                        }
                        c |= self.section(ui, &sections::QUALITIES, lang, pdfs, status);
                        ui.add_space(12.0);
                        c |= self.contact_form(ui, lang);
                        c |= self.section(ui, &sections::CONTACTS, lang, pdfs, status);
                    });
                    c
                }
                Tab::Magic => self.magic_tab(ui, engine, lang, pdfs, status),
                Tab::Equipment => self.equipment_tab(ui, engine, lang, pdfs, status),
                Tab::Improvements => self.improvements_tab(ui, lang),
                Tab::Log => {
                    let mut c = self.log_tab(ui, engine, lang);
                    if self.ch.created {
                        ui.separator();
                        c |= self.calendar_ui(ui, lang);
                    }
                    c
                }
                Tab::Notes => self.notes_tab(ui, lang),
            };
        });
        changed |= self.confirm_dialog(ctx, lang);
        changed |= self.select_dialog(ctx, engine, lang, pdfs, status);
        changed |= self.drug_builder.window(ctx, &mut self.ch, &self.store, lang, status);
        changed |= self.finish_dialog(ctx, lang);
        if let Some(a) = self.action.take() {
            changed |= self.run_action(a, engine, status);
        }
        if changed {
            self.ch.dirty = true;
            self.recompute(engine);
        }
        roll
    }

    // ----- sidebar -----

    fn sidebar(&mut self, ui: &mut egui::Ui, lang: &Language, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        let s = &self.sheet;
        ui.add_space(4.0);
        ui.heading(RichText::new(self.ch.display_name()).color(ACCENT));
        let meta: Vec<String> = ["metatype", "metavariant"].iter().map(|k| self.ch.field(k)).filter(|v| !v.is_empty()).collect();
        ui.label(meta.join(" · "));
        ui.weak(format!(
            "{} · {}",
            if self.ch.created { lang.tr("Career") } else { lang.tr("Creation") },
            match self.ch.field("buildmethod").as_str() {
                "SumtoTen" => lang.tr("Sum-to-Ten"),
                "" => lang.tr("Priority"),
                b => lang.tr(b),
            }
        ));
        ui.separator();

        egui::Grid::new("resources").num_columns(2).show(ui, |ui| {
            ui.label(lang.tr("Karma"));
            changed |= ui.add(egui::DragValue::new(&mut self.ch.karma).speed(0.2)).changed();
            ui.end_row();
            ui.label(lang.tr("Nuyen"));
            ui.horizontal(|ui| {
                changed |= ui.add(egui::DragValue::new(&mut self.ch.nuyen).speed(10.0).max_decimals(2).suffix("¥")).changed();
            });
            ui.end_row();
            ui.label(lang.tr("Essence"));
            ui.strong(format::essence(s.essence, self.rules.essence_decimals));
            ui.end_row();
        });
        ui.separator();

        let stat = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.label(label);
            ui.strong(value);
            ui.end_row();
        };
        egui::Grid::new("derived").num_columns(2).striped(true).show(ui, |ui| {
            stat(ui, &lang.tr("Initiative"), format!("{} + {}d6", s.initiative, s.initiative_dice));
            stat(ui, &lang.tr("Astral"), format!("{} + {}d6", s.astral_initiative, s.astral_initiative_dice));
            stat(ui, &lang.tr("Matrix cold-sim"), format!("{} + {}d6", s.matrix_cold_initiative, s.matrix_cold_dice));
            stat(ui, &lang.tr("Matrix hot-sim"), format!("{} + {}d6", s.matrix_hot_initiative, s.matrix_hot_dice));
            stat(ui, &lang.tr("Physical limit"), s.limit_physical.to_string());
            stat(ui, &lang.tr("Mental limit"), s.limit_mental.to_string());
            stat(ui, &lang.tr("Social limit"), s.limit_social.to_string());
            if self.ch.mag_enabled() {
                stat(ui, &lang.tr("Astral limit"), s.limit_astral.to_string());
            }
            stat(ui, &lang.tr("Armor"), s.armor.to_string());
            stat(ui, &lang.tr("Composure"), s.composure.to_string());
            stat(ui, &lang.tr("Judge Intentions"), s.judge_intentions.to_string());
            stat(ui, &lang.tr("Memory"), s.memory.to_string());
            stat(ui, &lang.tr("Lift and Carry"), s.lift_carry.to_string());
            if s.wound_modifier != 0 {
                ui.colored_label(WARN, lang.tr("Wound modifier"));
                ui.colored_label(WARN, s.wound_modifier.to_string());
                ui.end_row();
            }
        });
        ui.separator();

        let (pcm, scm, thr) = (s.physical_cm, s.stun_cm, s.cm_threshold);
        ui.label(RichText::new(lang.tr("Physical damage")).strong());
        changed |= cm_track(ui, "pcm", pcm, thr, &mut self.ch.physical_cm_filled, Color32::from_rgb(200, 60, 60));
        ui.label(RichText::new(lang.tr("Stun damage")).strong());
        changed |= cm_track(ui, "scm", scm, thr, &mut self.ch.stun_cm_filled, Color32::from_rgb(70, 130, 220));
        ui.weak(lang.tr_fmt("Overflow {0} · −1 die per {1} boxes", &[&s.cm_overflow, &thr]));
        ui.separator();
        changed |= self.budget_panel(ui, lang);
        if ui.button(format!("🎲 {}", lang.tr("Open Dice Roller"))).clicked() {
            *roll = Some(6);
        }
        changed
    }

    // ----- tabs -----

    fn info_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("info").num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                for (i, (key, label)) in INFO_FIELDS.iter().enumerate() {
                    ui.label(lang.tr(label));
                    let mut v = self.ch.field(key);
                    let editable = !matches!(*key, "metatype" | "metavariant");
                    let r = ui.add_enabled_ui(editable, |ui| ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut v))).inner;
                    if r.changed() {
                        self.ch.set_field(key, v);
                        changed = true;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(8.0);
            egui::Grid::new("reputation").num_columns(6).spacing([16.0, 6.0]).show(ui, |ui| {
                for (key, label) in [("streetcred", lang.tr("Street Cred")), ("notoriety", lang.tr("Notoriety")), ("publicawareness", lang.tr("Public Awareness"))] {
                    ui.label(label);
                    let mut v = self.ch.doc.get_i32(key).unwrap_or(0);
                    if ui.add(egui::DragValue::new(&mut v).range(0..=100)).changed() {
                        self.ch.set_field(key, v.to_string());
                        changed = true;
                    }
                }
            });
            ui.add_space(8.0);
            for (key, label) in TEXT_FIELDS.iter().filter(|(k, _)| matches!(*k, "concept" | "description" | "background")) {
                ui.label(RichText::new(lang.tr(label)).strong());
                let mut v = self.ch.field(key);
                if ui.add(egui::TextEdit::multiline(&mut v).desired_width(f32::INFINITY).desired_rows(4)).changed() {
                    self.ch.set_field(key, v);
                    changed = true;
                }
                ui.add_space(6.0);
            }
        });
        changed
    }

    fn attributes_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut changed = false;
        let career = self.ch.created;
        let priority = chummer_core::character::uses_priority_tables(&self.ch.field("buildmethod"));
        let shown: Vec<&str> = attributes::PHYSICAL
            .iter()
            .chain(attributes::MENTAL)
            .chain(attributes::SPECIAL)
            .copied()
            .filter(|n| match *n {
                "ESS" => false,
                "MAG" => self.ch.mag_enabled(),
                "MAGAdept" => self.ch.mag_enabled() && self.ch.is_adept() && self.ch.is_magician(),
                "RES" => self.ch.res_enabled(),
                "DEP" => self.ch.dep_enabled(),
                _ => true,
            })
            .collect();
        ui.label(if career {
            lang.tr("Career mode: Raise spends karma and records it in the Karma & Nuyen log, where it can be undone.")
        } else {
            lang.tr("Creation mode: base uses attribute points (priority builds). Changing levels does not deduct karma automatically; the Karma cost column shows what they are worth.")
        });
        ui.add_space(6.0);
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::exact(130.0))
            .columns(Column::auto().at_least(60.0), 6)
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for t in lang.tr_all(["Attribute", "Min/Max", "Base", "Karma", "Natural", "Augmented", "Karma cost", "Next level"]) {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for name in shown {
                    let Some(v) = self.sheet.attr_values(name).cloned() else { continue };
                    body.row(24.0, |mut row| {
                        row.col(|ui| {
                            let key = format!("String_Attribute{name}Long");
                            let long = if lang.has(&key) { lang.s(&key) } else { lang.tr(attributes::long_name(name)) };
                            ui.label(format!("{long} ({name})"));
                        });
                        row.col(|ui| {
                            ui.label(format!("{}/{} ({})", v.total_min, v.total_max, v.total_aug_max));
                        });
                        row.col(|ui| {
                            if let Some(a) = self.ch.attribute_mut(name) {
                                let max = (v.total_max - v.total_min - v.free_base - a.karma).max(a.base);
                                let r = ui.add_enabled(priority && !career, egui::DragValue::new(&mut a.base).range(0..=max));
                                changed |= r.changed();
                            }
                        });
                        row.col(|ui| {
                            if let Some(a) = self.ch.attribute_mut(name) {
                                let max = (v.total_max - v.total_base).max(a.karma);
                                changed |= ui.add_enabled(!career, egui::DragValue::new(&mut a.karma).range(0..=max)).changed();
                            }
                        });
                        row.col(|ui| {
                            ui.strong(v.value.to_string());
                        });
                        row.col(|ui| {
                            if v.total != v.value {
                                ui.colored_label(ACCENT, v.total.to_string());
                            } else {
                                ui.label(v.total.to_string());
                            }
                        });
                        row.col(|ui| {
                            ui.label(calc::attribute_karma_cost(&v, &self.rules).to_string());
                        });
                        row.col(|ui| {
                            if career {
                                match career::attribute_upgrade_karma_cost(engine, &self.ch, name) {
                                    Some(c) => {
                                        let r = ui.add_enabled(self.ch.karma >= c, egui::Button::new(lang.tr_fmt("Raise ({0} karma)", &[&c])));
                                        if r.clicked() {
                                            self.action = Some(CareerAction::RaiseAttribute(name.to_owned()));
                                        }
                                    }
                                    None => {
                                        ui.weak(lang.tr("at maximum"));
                                    }
                                }
                            } else {
                                match calc::attribute_upgrade_cost(&v, &self.rules) {
                                    Some(c) => ui.label(lang.tr_fmt("{0} karma", &[&c])),
                                    None => ui.weak(lang.tr("at maximum")),
                                };
                            }
                        });
                    });
                }
            });
        ui.add_space(8.0);
        ui.label(format!("{} {}", lang.tr("Karma spent on attributes:"), self.sheet.attribute_karma_spent));
        changed
    }

    fn skills_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.skill_filter).hint_text(lang.tr("Filter skills")).desired_width(200.0));
            ui.checkbox(&mut self.only_rated, lang.tr("Only skills with a rating"));
            ui.separator();
            if self.ch.created {
                ui.label(format!("{} {}", lang.tr("Karma value of skills:"), self.sheet.skill_karma_spent));
            } else {
                ui.label(format!(
                    "{} {} / {} · {} {}",
                    lang.tr("Knowledge Points:"),
                    self.sheet.knowledge_points_used,
                    self.sheet.knowledge_points,
                    lang.tr("karma spent on skills:"),
                    self.sheet.skill_karma_spent
                ));
            }
        });
        ui.add_space(4.0);
        let needle = self.skill_filter.to_lowercase();
        let filter = |name: &str, rating: i32| (needle.is_empty() || name.to_lowercase().contains(&needle)) && (!self.only_rated || rating > 0);
        let rows: Vec<(usize, calc::SkillValues)> =
            self.sheet.skills.iter().cloned().enumerate().filter(|(_, s)| filter(&s.name, s.rating)).collect();
        let kno: Vec<(usize, calc::SkillValues)> =
            self.sheet.knowledge_skills.iter().cloned().enumerate().filter(|(_, s)| filter(&s.name, s.rating)).collect();
        let career = self.ch.created;
        // Skills cap at 6 during creation (setting-dependent), 12 in career.
        let cap = if career { self.rules.max_skill_rating_career } else { self.rules.max_skill_rating_create };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(lang.tr("Active Skills"));
            egui::Grid::new("skills").striped(true).num_columns(8).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in lang.tr_all(["Skill", "Attr", "Group", "Base", "Karma", "Rating", "Pool", "Specializations"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, s) in &rows {
                    let r = SourceRef::new(&s.source, &s.page);
                    let label = if s.disabled { RichText::new(&s.name).weak() } else { RichText::new(&s.name) };
                    let name = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
                    if let Some(r) = r {
                        if name.on_hover_text(format!("{r} — {}", lang.tr("click to open the rulebook"))).clicked() {
                            pdf_ui::open(pdfs, &r, status);
                        }
                    }
                    ui.label(&s.attribute);
                    ui.weak(&s.group);
                    let sk = &mut self.ch.skills[*i];
                    let on = !s.disabled;
                    changed |= ui.add_enabled(!career && on, egui::DragValue::new(&mut sk.base).range(0..=cap)).changed();
                    changed |= ui.add_enabled(on && !career, egui::DragValue::new(&mut sk.karma).range(0..=cap)).changed();
                    ui.label(s.rating.to_string());
                    let pool = if s.rating == 0 && !s.default { "—".to_owned() } else { s.pool.to_string() };
                    if ui.add(egui::Button::new(RichText::new(pool).strong()).frame(false)).on_hover_text(lang.tr("Roll this pool")).clicked() {
                        *roll = Some(s.pool.max(1) as u32);
                    }
                    ui.horizontal(|ui| {
                        if career && !s.disabled {
                            if let Some(c) = career::skill_upgrade_karma_cost(engine, &self.ch, &s.guid) {
                                if ui.add_enabled(self.ch.karma >= c, egui::Button::new(format!("▲ {c}"))).on_hover_text(lang.tr("Raise for karma")).clicked() {
                                    self.action = Some(CareerAction::RaiseSkill(s.guid.clone()));
                                }
                            }
                        }
                        if !s.specs.is_empty() {
                            ui.label(format!("{} (+{})", s.specs.join(", "), s.spec_bonus));
                        }
                        let guid = s.guid.clone();
                        let suid = self.ch.skills[*i].suid.clone();
                        ui.menu_button("＋", |ui| {
                            let opts = engine.catalog.get(&suid).map(|d| d.specs.clone()).unwrap_or_default();
                            for o in opts.iter().filter(|o| !s.specs.contains(o)) {
                                if ui.button(o).clicked() {
                                    if career {
                                        self.action = Some(CareerAction::Specialize(guid.clone(), o.clone()));
                                    } else {
                                        chargen::add_specialization(&mut self.ch, &guid, o);
                                        changed = true;
                                    }
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text(lang.tr("Add a specialization"));
                    });
                    ui.end_row();
                }
            });
            ui.add_space(12.0);
            ui.heading(lang.tr("Knowledge Skills"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.new_kno.0).hint_text(lang.tr("New Knowledge Skill")).desired_width(200.0));
                egui::ComboBox::from_id_salt("kno_type").selected_text(lang.data_name("skills.xml", "", &self.new_kno.1)).show_ui(ui, |ui| {
                    for t in ["Academic", "Interest", "Language", "Professional", "Street"] {
                        ui.selectable_value(&mut self.new_kno.1, t.to_owned(), lang.data_name("skills.xml", "", t));
                    }
                });
                if self.new_kno.1 == "Language" {
                    ui.checkbox(&mut self.new_kno.2, lang.tr("Native"));
                }
                if ui.add_enabled(!self.new_kno.0.trim().is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                    let native = self.new_kno.1 == "Language" && self.new_kno.2;
                    if career {
                        self.action = Some(CareerAction::LearnKnowledge(self.new_kno.0.trim().to_owned(), self.new_kno.1.clone()));
                    } else {
                        chargen::add_knowledge_skill(&mut self.ch, self.new_kno.0.trim(), &self.new_kno.1.clone(), native);
                        changed = true;
                    }
                    self.new_kno.0.clear();
                }
            });
            let mut remove_kno: Option<String> = None;
            egui::Grid::new("kskills").striped(true).num_columns(7).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in lang.tr_all(["Skill", "Type", "Base", "Karma", "Rating", "Pool", ""]) {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, s) in &kno {
                    ui.label(&s.name);
                    ui.weak(lang.data_name("skills.xml", "", &s.category));
                    if s.native {
                        ui.weak(lang.tr("native"));
                        ui.label("");
                        ui.label("N");
                        ui.label("N");
                    } else {
                        let k = &mut self.ch.knowledge_skills[*i];
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut k.base).range(0..=cap)).changed();
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut k.karma).range(0..=cap)).changed();
                        ui.horizontal(|ui| {
                            ui.label(s.rating.to_string());
                            if career {
                                if let Some(c) = career::skill_upgrade_karma_cost(engine, &self.ch, &s.guid) {
                                    if ui.add_enabled(self.ch.karma >= c, egui::Button::new(format!("▲ {c}"))).clicked() {
                                        self.action = Some(CareerAction::RaiseSkill(s.guid.clone()));
                                    }
                                }
                            }
                        });
                        ui.strong(s.pool.to_string());
                    }
                    if ui.small_button("🗑").on_hover_text(lang.tr("Remove")).clicked() {
                        remove_kno = Some(s.guid.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(g) = remove_kno {
                chargen::remove_knowledge_skill(&mut self.ch, &g);
                changed = true;
            }
            if !self.ch.skill_groups.is_empty() {
                ui.add_space(12.0);
                ui.heading(lang.tr("Skill Groups"));
                egui::Grid::new("groups").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                    for h in lang.tr_all(["Group", "Base", "Karma", "Rating"]) {
                        ui.strong(h);
                    }
                    ui.end_row();
                    let costs: Vec<Option<i32>> = self
                        .ch
                        .skill_groups
                        .iter()
                        .map(|g| if career { career::skill_group_upgrade_karma_cost(engine, &self.ch, &g.name) } else { None })
                        .collect();
                    let karma = self.ch.karma;
                    for (g, cost) in self.ch.skill_groups.iter_mut().zip(costs) {
                        ui.label(&g.name);
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut g.base).range(0..=cap)).changed();
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut g.karma).range(0..=cap)).changed();
                        ui.horizontal(|ui| {
                            ui.label(g.rating().to_string());
                            if let Some(c) = cost {
                                if ui.add_enabled(karma >= c, egui::Button::new(format!("▲ {c}"))).clicked() {
                                    self.action = Some(CareerAction::RaiseGroup(g.name.clone()));
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
            }
        });
        changed
    }

    fn magic_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        if self.ch.mag_enabled() && self.ch.is_magician() {
            let current = self.ch.doc.child("tradition").map(|t| t.get("name")).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(lang.tr("Tradition"));
                let mut pick: Option<String> = None;
                egui::ComboBox::from_id_salt("tradition").selected_text(if current.is_empty() { lang.tr("Choose…") } else { current.clone() }).width(240.0).show_ui(ui, |ui| {
                    if let Ok(doc) = self.store.doc("traditions.xml") {
                        for r in data::records(&doc, "traditions", "tradition") {
                            if ui.selectable_label(current == r.name(), r.name()).clicked() {
                                pick = Some(r.name());
                            }
                        }
                    }
                });
                if let Some(p) = pick {
                    if let Err(e) = chargen::set_tradition(&mut self.ch, &self.store, &p) {
                        *status = Some((e, true));
                    }
                    changed = true;
                }
            });
        }
        let present: Vec<&Section> = sections::MAGIC.iter().chain([&sections::COMPLEX_FORMS, &sections::MARTIAL_ARTS]).collect();
        ui.horizontal(|ui| {
            for (i, s) in present.iter().enumerate() {
                let n = self.ch.items(s.container, s.item).len();
                ui.selectable_value(&mut self.magic, i, format!("{} ({n})", lang.tr(s.label)));
            }
        });
        let m = chummer_core::items::magic::magic_summary_with(&self.ch, &self.sheet, Some(&self.store));
        ui.horizontal_wrapped(|ui| {
            if !m.tradition.is_empty() {
                ui.label(format!("{} {}", lang.tr("Tradition:"), m.tradition));
                ui.label(lang.tr_fmt("Drain {0} = {1} dice", &[&m.drain_expression.replace(['{', '}'], ""), &m.drain_pool]));
            }
            if !m.stream.is_empty() {
                ui.label(lang.tr_fmt("Stream: {0} · Fading {1} = {2} dice", &[&m.stream, &m.fading_expression.replace(['{', '}'], ""), &m.fading_pool]));
            }
            if let Some((total, used)) = m.power_points {
                let t = RichText::new(format!("{} {used} / {total}", lang.tr("Power Points")));
                ui.label(if used > total { t.color(ui.visuals().error_fg_color) } else { t });
            }
            if self.ch.mag_enabled() {
                ui.label(lang.tr_fmt("Astral {0} + {1}d6, limit {2}", &[&m.astral_initiative, &m.astral_initiative_dice, &m.astral_limit]));
            }
        });
        let techno = self.ch.res_enabled() && !self.ch.mag_enabled();
        let grade = self.ch.doc.get_i32(if techno { "submersiongrade" } else { "initiategrade" }).unwrap_or(0);
        ui.horizontal(|ui| {
            if grade > 0 {
                ui.label(format!("{} {grade}", if techno { lang.tr("Submersion Grade") } else { lang.tr("Initiate Grade") }));
            }
            if self.ch.created && (self.ch.mag_enabled() || self.ch.res_enabled()) {
                ui.checkbox(&mut self.initiation.group, lang.tr("Group"));
                ui.checkbox(&mut self.initiation.ordeal, lang.tr("Ordeal"));
                ui.checkbox(&mut self.initiation.schooling, lang.tr("Schooling"));
                let cost = career::initiation_karma_cost(engine, &self.ch, self.initiation);
                let label = format!("{} ({cost} {})", if techno { lang.tr("Submerge") } else { lang.tr("Initiate") }, lang.tr("karma"));
                if ui.add_enabled(self.ch.karma >= cost, egui::Button::new(label)).clicked() {
                    self.action = Some(CareerAction::Initiate(self.initiation));
                }
            }
        });
        ui.separator();
        if let Some(sec) = present.get(self.magic) {
            let sec = **sec;
            let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
            changed |= self.magic_editor.ui(ui, &mut self.ch, &cx, sec.container, status);
            self.add_buttons(ui, engine, lang, sec.container);
            egui::ScrollArea::both().show(ui, |ui| changed |= self.section(ui, &sec, lang, pdfs, status));
        }
        changed
    }

    /// Final weapon stats (damage with STR, AP, accuracy, dice pool, ranges).
    fn weapon_summary(&self, ui: &mut egui::Ui, lang: &Language) {
        let weapons = self.ch.items("weapons", "weapon");
        if weapons.is_empty() {
            return;
        }
        let rules = self.settings.as_ref().map(chummer_core::items::weapon::WeaponRules::from_settings).unwrap_or_default();
        egui::CollapsingHeader::new(RichText::new(lang.tr("Combat stats")).strong()).id_salt("combat_stats").default_open(true).show(ui, |ui| {
            egui::Grid::new("weapon_stats").striped(true).num_columns(8).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Weapon", "Pool", "Damage", "AP", "Acc", "RC", "Reach", "Ranges"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for w in weapons {
                    let st = chummer_core::items::weapon::stats_with(&self.ch, &self.sheet, Some(&self.store), w, &rules);
                    ui.label(w.get("name"));
                    ui.strong(st.dice_pool.to_string()).on_hover_text(&st.skill);
                    ui.label(&st.damage);
                    ui.label(&st.ap);
                    ui.label(st.accuracy.to_string());
                    ui.label(&st.rc);
                    ui.label(if st.reach != 0 { st.reach.to_string() } else { String::new() });
                    let r = &st.ranges;
                    let bands: Vec<&str> = [&r.short, &r.medium, &r.long, &r.extreme].into_iter().map(String::as_str).filter(|b| !b.is_empty()).collect();
                    ui.label(bands.join(" / "));
                    ui.end_row();
                }
            });
        });
    }

    /// Vehicle totals after mods.
    fn vehicle_summary(&self, ui: &mut egui::Ui, lang: &Language) {
        let vehicles = self.ch.items("vehicles", "vehicle");
        if vehicles.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(RichText::new(lang.tr("Vehicle stats")).strong()).id_salt("vehicle_stats").default_open(true).show(ui, |ui| {
            egui::Grid::new("vehicle_stats").striped(true).num_columns(10).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Vehicle", "Handling", "Speed", "Accel", "Body", "Armor", "Pilot", "Sensor", "Seats", "Slots"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for v in vehicles {
                    let st = chummer_core::items::vehicle::stats(v);
                    ui.label(v.get("name"));
                    ui.label(&st.handling_text);
                    ui.label(&st.speed_text);
                    ui.label(&st.accel_text);
                    ui.label(st.body.to_string());
                    ui.label(st.armor.to_string());
                    ui.label(st.pilot.to_string());
                    ui.label(st.sensor.to_string());
                    ui.label(st.seats.to_string());
                    if st.is_drone {
                        ui.label(format!("{}/{}", st.drone_mod_slots_used, st.drone_mod_slots));
                    } else {
                        ui.label(format!("{}/{}", st.slots_used, st.slots));
                    }
                    ui.end_row();
                }
            });
        });
    }

    /// "Add …" buttons for the kinds that live in a section's container.
    fn add_buttons(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, container: &str) {
        let tags: &[&str] = match container {
            "gears" => &["gear"],
            "cyberwares" => &["cyberware", "bioware"],
            "armors" => &["armor", "armormod"],
            "weapons" => &["weapon", "accessory"],
            "vehicles" => &["vehicle", "mod"],
            "lifestyles" => &["lifestyle"],
            "spells" => &[], // magic_ui: spell options and career karma
            "powers" => &["power"],
            "complexforms" => &["complexform"],
            "spirits" => &["spirit"],
            "metamagics" => &[], // magic_ui: one per grade, echoes for technomancers
            "martialarts" => &["martialart"],
            "critterpowers" => &["critterpower"],
            _ => &[],
        };
        ui.horizontal(|ui| {
            for t in tags {
                let label = chummer_core::items::kind(t).map_or(*t, |k| k.label);
                if ui.button(format!("➕ {}", lang.tr_fmt("Add {0}…", &[&kind_noun(lang, label)]))).clicked() {
                    self.open_select(t, engine);
                }
            }
        });
    }

    fn equipment_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            for (i, s) in sections::EQUIPMENT.iter().enumerate() {
                let n = self.ch.items(s.container, s.item).len();
                ui.selectable_value(&mut self.equipment, i, format!("{} ({n})", lang.tr(s.label)));
            }
        });
        ui.separator();
        let sec = sections::EQUIPMENT[self.equipment];
        self.add_buttons(ui, engine, lang, sec.container);
        match sec.container {
            "weapons" => self.weapon_summary(ui, lang),
            "vehicles" => self.vehicle_summary(ui, lang),
            "gears" => {
                if ui.button(format!("🧪 {}", lang.tr("Build custom drug…"))).clicked() {
                    self.drug_builder.open = true;
                }
            }
            "lifestyles" => {
                let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
                changed |= self.lifestyle_editor.ui(ui, &mut self.ch, &cx, status);
            }
            _ => {}
        }
        egui::ScrollArea::both().show(ui, |ui| changed |= self.section(ui, &sec, lang, pdfs, status));
        changed
    }

    /// A table of items. Returns true if the character changed.
    fn section(&mut self, ui: &mut egui::Ui, sec: &Section, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let items: Vec<Element> = self.ch.items(sec.container, sec.item).into_iter().cloned().collect();
        ui.heading(format!("{} ({})", lang.tr(sec.label), items.len()));
        if items.is_empty() {
            ui.weak(lang.tr("None."));
            return false;
        }
        egui::Grid::new(sec.container).striped(true).num_columns(sec.columns.len() + 1).spacing([14.0, 4.0]).show(ui, |ui| {
            for c in sec.columns {
                ui.strong(lang.tr(c.header));
            }
            ui.label("");
            ui.end_row();
            let mut clicked: Option<String> = None;
            for it in &items {
                clicked = item_rows(ui, sec, it, lang, 0).or(clicked);
                ui.horizontal(|ui| {
                    pdf_ui::source_icon(ui, pdfs, SourceRef::of(it), status);
                    if ui.small_button("🗑").on_hover_text(lang.tr("Remove (also removes its improvements)")).clicked() {
                        self.confirm_remove = Some((sec.container.to_owned(), it.get("guid"), display_name(sec, it, lang)));
                    }
                });
                ui.end_row();
                for (container, item) in sec.child_containers {
                    if let Some(c) = it.child(container) {
                        for child in c.children_named(item) {
                            clicked = child_rows(ui, sec, child, lang, 1, pdfs, status).or(clicked);
                        }
                    }
                }
            }
            if let Some(g) = clicked {
                self.item_editor = Some((g, crate::item_editor::ItemEditor::default()));
            }
        });
        false
    }

    /// The item detail pane, when an item is selected (see `item_editor`).
    fn item_editor_panel(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, status: &mut Status) -> bool {
        let Some((guid, mut ed)) = self.item_editor.take() else { return false };
        let store = self.store.clone();
        let mut res = crate::item_editor::EditorResult::default();
        let mut close = false;
        egui::SidePanel::right("item_editor").resizable(true).default_width(300.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(lang.tr("Item"));
                close = ui.small_button("✖").on_hover_text(lang.tr("Close")).clicked();
            });
            egui::ScrollArea::vertical().show(ui, |ui| res = ed.ui(ui, &mut self.ch, &store, engine, lang, &guid));
        });
        if let Some(s) = res.status.take() {
            *status = Some(s);
        }
        if let Some((tag, parent)) = res.add_child.take() {
            self.open_select(&tag, engine);
            self.select = self.select.take().map(|d| d.with_parent(Some(parent)));
        }
        if let Some(g) = res.select.take() {
            self.item_editor = Some((g, crate::item_editor::ItemEditor::default()));
        } else if !close && !res.removed {
            self.item_editor = Some((guid, ed));
        }
        res.changed
    }

    /// Confirmation dialog for item removal. Returns true if an item went.
    fn confirm_dialog(&mut self, ctx: &egui::Context, lang: &Language) -> bool {
        let Some((container, guid, name)) = self.confirm_remove.clone() else { return false };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm_remove")).show(ctx, |ui| {
            ui.heading(lang.tr("Remove item"));
            ui.label(lang.tr_fmt("Remove {0}? Its improvements are removed too. This cannot be undone.", &[&name]));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Remove")).clicked() {
                    choice = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(yes) => {
                self.confirm_remove = None;
                if yes && container == "qualities" {
                    if self.ch.created {
                        // Career mode: buying off a negative quality costs karma.
                        self.action = Some(CareerAction::RemoveQuality(guid));
                        return false;
                    }
                    chargen::remove_quality(&mut self.ch, &guid);
                    true
                } else if yes && container == "cyberwares" {
                    chummer_core::items::cyberware::remove(&mut self.ch, &guid)
                } else {
                    yes && self.ch.remove_item(&container, &guid)
                }
            }
            None => false,
        }
    }

    /// Creation-mode budgets in the sidebar, with Finish creation.
    fn budget_panel(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let Some(b) = self.budget.clone() else { return false };
        let mut changed = false;
        ui.label(RichText::new(lang.tr("Creation")).strong());
        let row = |ui: &mut egui::Ui, label: &str, total: i32, used: i32| {
            let left = total - used;
            ui.label(label);
            let t = RichText::new(format!("{left} / {total}"));
            ui.label(if left < 0 { t.color(ui.visuals().error_fg_color) } else if left == 0 { t.weak() } else { t.color(ACCENT) });
            ui.end_row();
        };
        egui::Grid::new("budget").num_columns(2).striped(true).show(ui, |ui| {
            row(ui, &lang.tr("Karma"), b.karma.0, b.karma.1);
            row(ui, &lang.tr("Attribute Points"), b.attribute_points.0, b.attribute_points.1);
            row(ui, &lang.tr("Special points"), b.special_points.0, b.special_points.1);
            row(ui, &lang.tr("Skill Points"), b.skill_points.0, b.skill_points.1);
            row(ui, &lang.tr("Skill Group Points"), b.skill_group_points.0, b.skill_group_points.1);
            row(ui, &lang.tr("Knowledge Points"), b.knowledge_points.0, b.knowledge_points.1);
            row(ui, &lang.tr("Contact Points"), b.contact_points.0, b.contact_points.1);
            if b.free_spells.0 > 0 {
                row(ui, &lang.tr("Free Spells"), b.free_spells.0, b.free_spells.1);
            }
            ui.label(lang.tr("Positive Qualities"));
            ui.label(format!("{} / {}", b.positive_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label(lang.tr("Negative Qualities"));
            ui.label(format!("{} / {}", b.negative_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label(lang.tr("Nuyen left"));
            let left = b.nuyen_left();
            let t = RichText::new(chummer_core::format::nuyen(left));
            ui.label(if left < 0.0 { t.color(ui.visuals().error_fg_color) } else { t });
            ui.end_row();
            ui.label(lang.tr("Karma for nuyen"));
            let mut bp = self.ch.doc.get_i32("nuyenbp").unwrap_or(0);
            let max = self.settings.as_ref().map_or(10, |s| s.int("nuyenmaxbp", 10));
            if ui.add(egui::DragValue::new(&mut bp).range(0..=max).suffix(format!(" {}", lang.tr("karma")))).on_hover_text(lang.tr("2,000¥ per karma")).changed() {
                self.ch.set_field("nuyenbp", bp.to_string());
                changed = true;
            }
            ui.end_row();
        });
        for p in &self.problems {
            ui.colored_label(WARN, format!("• {p}"));
        }
        let ok = self.problems.is_empty();
        let r = ui.add_enabled(ok, egui::Button::new(RichText::new(lang.tr("Finish creation")).color(ACCENT)));
        if r.on_disabled_hover_text(lang.tr("Fix the problems above first")).clicked() {
            self.confirm_finish = true;
        }
        ui.separator();
        changed
    }

    fn finish_dialog(&mut self, ctx: &egui::Context, lang: &Language) -> bool {
        if !self.confirm_finish {
            return false;
        }
        let mut choice = None;
        let b = self.budget.clone().unwrap_or_default();
        egui::Modal::new(egui::Id::new("finish_creation")).show(ctx, |ui| {
            ui.heading(lang.tr("Finish creation?"));
            ui.label(lang.tr("The character switches to career mode. Creation budgets go away; karma and nuyen become plain resources."));
            let carry_k = self.settings.as_ref().map_or(7, |s| s.karma("karmacarryover", 7));
            if b.karma_left() > carry_k {
                ui.colored_label(WARN, lang.tr_fmt("{0} karma is left, only {1} carries over.", &[&b.karma_left(), &carry_k]));
            }
            if b.nuyen_left() > 5000.0 {
                ui.colored_label(WARN, lang.tr_fmt("{0} is left, only 5,000¥ carries over.", &[&chummer_core::format::nuyen(b.nuyen_left())]));
            }
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Finish")).clicked() {
                    choice = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                if let Some(st) = self.settings.clone() {
                    chargen::finalize(&mut self.ch, &b, &st);
                }
                self.confirm_finish = false;
                true
            }
            Some(false) => {
                self.confirm_finish = false;
                false
            }
            None => false,
        }
    }

    /// Open the add dialog for an item kind (see `items::KINDS`).
    fn open_select(&mut self, tag: &str, _engine: &Engine) {
        let books = self.settings.as_ref().map(|s| s.books()).unwrap_or_default();
        let max_avail = self.settings.as_ref().map_or(12, |s| s.max_availability());
        let nuyen_left = self.budget.as_ref().map(|b| b.nuyen_left());
        self.select = SelectDialog::new(tag, &self.store, books, max_avail, nuyen_left);
    }

    fn select_dialog(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let Some(dlg) = self.select.as_mut() else { return false };
        let tag = dlg.kind.tag;
        let ch = &self.ch;
        let store_arc = self.store.clone();
        let store = &*store_arc;
        let choices_for = |rec: &chummer_core::xml::Element, p: &chummer_core::items::Purchase| chummer_core::items::choices(tag, ch, store, data::Record(rec), p);
        match dlg.show(ctx, ch, &self.sheet, lang, pdfs, status, &choices_for) {
            select::Outcome::None => false,
            select::Outcome::Cancel => {
                self.select = None;
                false
            }
            select::Outcome::Done { index, purchase } => {
                let Some(rec) = dlg.record(store, index) else { return false };
                let rec = data::Record(&rec);
                let name = rec.name();
                let karma = rec.el().get_i32("karma").unwrap_or(0);
                let _ = karma;
                if self.ch.created && tag == "quality" {
                    // Career mode: karma is spent and logged (double for most qualities).
                    return match career::add_quality(&mut self.ch, engine, rec, purchase.answer.as_deref()) {
                        Ok(_) => {
                            *status = Some((format!("Added {name}"), false));
                            self.select = None;
                            true
                        }
                        Err(e) => {
                            *status = Some((e.to_string(), true));
                            false
                        }
                    };
                }
                match chummer_core::items::add(tag, &mut self.ch, store, rec, &purchase) {
                    Ok(guid) => {
                        chummer_core::items::edit::settle_new_item(&mut self.ch, &guid);
                        let mut msg = format!("Added {name}");
                        // Martial arts cost karma, not nuyen (MartialArt.Cost).
                        if self.ch.created && tag == "martialart" {
                            let karma = chummer_core::items::find_by_guid_mut(&mut self.ch.doc, &guid).and_then(|e| e.get_f64("cost")).unwrap_or(0.0);
                            let rules = career::CareerRules::for_character(engine, &self.ch);
                            let entry = career::ManualExpense { amount: karma, reason: format!("Learned martial art {name}"), ..Default::default() };
                            if karma > 0.0 {
                                if let Err(e) = career::karma_spent(&mut self.ch, &rules, &entry) {
                                    self.ch.remove_item_anywhere(&guid);
                                    *status = Some((e.to_string(), true));
                                    return true;
                                }
                                msg = format!("Learned {name} for {karma} karma");
                            }
                        }
                        let nuyen_kind = matches!(
                            tag,
                            "gear" | "cyberware" | "bioware" | "armor" | "armormod" | "weapon" | "accessory" | "vehicle" | "mod" | "weaponmount" | "lifestyle" | "drug"
                        );
                        if self.ch.created && nuyen_kind {
                            // Career mode: pay for it and log the purchase.
                            let cost = chummer_core::items::edit::total_cost(&self.ch, store, &guid);
                            let parent_tag = purchase.parent.as_ref().and_then(|p| chummer_core::items::find_by_guid_mut(&mut self.ch.doc, p).map(|e| e.name.clone()));
                            if cost > 0.0 {
                                match career::pay_for_item(&mut self.ch, tag, parent_tag.as_deref(), &guid, cost) {
                                    Ok(_) => msg = format!("Bought {name} for {}", format::nuyen(cost)),
                                    Err(e) => {
                                        // Not affordable: take it back out.
                                        self.ch.remove_item_anywhere(&guid);
                                        *status = Some((e.to_string(), true));
                                        return true;
                                    }
                                }
                            }
                        }
                        *status = Some((msg, false));
                        self.select = None;
                        true
                    }
                    Err(e) => {
                        *status = Some((format!("Could not add {name}: {e}"), true));
                        false
                    }
                }
            }
        }
    }

    fn life_module_picker(&mut self, ui: &mut egui::Ui, _engine: &Engine, lang: &Language, status: &mut Status) -> bool {
        let (stages, modules) = chargen::life_modules(&self.store);
        let mut added = false;
        ui.group(|ui| {
            ui.label(RichText::new(lang.tr("Life Modules")).strong());
            ui.horizontal(|ui| {
                if self.life.0.is_empty() {
                    self.life.0 = stages.first().cloned().unwrap_or_default();
                }
                egui::ComboBox::from_id_salt("lm_stage").selected_text(self.life.0.clone()).show_ui(ui, |ui| {
                    for st in &stages {
                        if ui.selectable_label(self.life.0 == *st, st).clicked() {
                            self.life = (st.clone(), String::new(), String::new());
                        }
                    }
                });
                let in_stage: Vec<&chargen::LifeModule> = modules.iter().filter(|m| m.stage == self.life.0).collect();
                let cur = in_stage.iter().find(|m| m.id == self.life.1).map(|m| format!("{} ({} {})", m.name, m.karma, lang.tr("karma"))).unwrap_or_else(|| lang.tr("Choose a module…"));
                egui::ComboBox::from_id_salt("lm_module").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                    for m in &in_stage {
                        if ui.selectable_label(self.life.1 == m.id, format!("{} ({} {})", m.name, m.karma, lang.tr("karma"))).clicked() {
                            self.life.1 = m.id.clone();
                            self.life.2 = m.versions.first().map(|v| v.0.clone()).unwrap_or_default();
                        }
                    }
                });
                if let Some(m) = in_stage.iter().find(|m| m.id == self.life.1) {
                    if m.versions.len() > 1 {
                        let cur = m.versions.iter().find(|v| v.0 == self.life.2).map(|v| v.1.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt("lm_version").selected_text(cur).show_ui(ui, |ui| {
                            for (id, n) in &m.versions {
                                ui.selectable_value(&mut self.life.2, id.clone(), n);
                            }
                        });
                    }
                }
                if ui.add_enabled(!self.life.1.is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                    let v = (!self.life.2.is_empty()).then(|| self.life.2.clone());
                    match chargen::add_life_module(&mut self.ch, &self.store, &self.life.1, v.as_deref()) {
                        Ok(_) => added = true,
                        Err(e) => *status = Some((e, true)),
                    }
                }
            });
        });
        added
    }

    fn contact_form(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_contact.0).hint_text(lang.tr("Contact name")).desired_width(160.0));
            ui.add(egui::TextEdit::singleline(&mut self.new_contact.1).hint_text(lang.tr("Role")).desired_width(120.0));
            ui.label(lang.tr("Connection"));
            ui.add(egui::DragValue::new(&mut self.new_contact.2).range(1..=12));
            ui.label(lang.tr("Loyalty"));
            ui.add(egui::DragValue::new(&mut self.new_contact.3).range(1..=6));
            if ui.add_enabled(!self.new_contact.0.trim().is_empty(), egui::Button::new(format!("➕ {}", lang.tr("Add Contact")))).clicked() {
                let (n, r, c, l) = self.new_contact.clone();
                chargen::add_contact(&mut self.ch, n.trim(), r.trim(), c, l);
                self.new_contact.0.clear();
                self.new_contact.1.clear();
                changed = true;
            }
        });
        changed
    }

    fn improvements_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let imps = &self.ch.improvements;
        ui.label(lang.tr_fmt(
            "{0} improvements ({1} active). These modifiers come from qualities, ware, powers and gear.",
            &[&imps.list.len(), &imps.active().count()],
        ));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("imps").striped(true).num_columns(7).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Type", "Target", "Value", "Aug", "Min/Max", "Source", "Condition"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for i in &imps.list {
                    let label = if imps.applies(i) { RichText::new(&i.kind) } else { RichText::new(&i.kind).weak() };
                    ui.label(label);
                    ui.label(&i.improved_name);
                    ui.label(fmt_opt(i.val));
                    ui.label(fmt_opt(i.aug));
                    ui.label(if i.min != 0.0 || i.max != 0.0 { format!("{}/{}", i.min, i.max) } else { String::new() });
                    ui.weak(&i.source);
                    ui.weak(&i.condition);
                    ui.end_row();
                }
            });
        });
        false
    }

    fn run_action(&mut self, a: CareerAction, engine: &Engine, status: &mut Status) -> bool {
        let r: Result<String, career::CareerError> = match &a {
            CareerAction::RaiseAttribute(n) => career::improve_attribute(&mut self.ch, engine, n),
            CareerAction::RaiseSkill(g) => career::improve_skill(&mut self.ch, engine, g),
            CareerAction::RaiseGroup(g) => career::improve_skill_group(&mut self.ch, engine, g),
            CareerAction::Specialize(g, n) => career::buy_specialization(&mut self.ch, engine, g, n),
            CareerAction::LearnKnowledge(n, k) => career::learn_knowledge_skill(&mut self.ch, engine, n, k),
            CareerAction::Undo(g) => career::undo_expense(&mut self.ch, engine, g).map(|_| String::new()),
            CareerAction::Initiate(o) => career::add_initiation_grade(&mut self.ch, engine, *o),
            CareerAction::RemoveQuality(g) => career::remove_quality(&mut self.ch, engine, g).map(|x| x.unwrap_or_default()),
        };
        match r {
            Ok(_) => {
                *status = Some((format!("Done: {}", describe_action(&a)), false));
                true
            }
            Err(e) => {
                *status = Some((e.to_string(), true));
                false
            }
        }
    }

    /// Career calendar: in-game weeks with notes (`CalendarWeek`).
    fn calendar_ui(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        use chummer_core::calendar;
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new(lang.tr("Calendar")).strong()).id_salt("calendar").show(ui, |ui| {
            if ui.button(format!("➕ {}", lang.tr("Add Week"))).clicked() {
                calendar::add_next_week(&mut self.ch, None);
                changed = true;
            }
            let mut weeks = calendar::weeks(&self.ch);
            weeks.sort_by_key(|w| std::cmp::Reverse((w.year, w.week)));
            let mut remove = None;
            egui::Grid::new("calendar_weeks").striped(true).num_columns(3).show(ui, |ui| {
                for w in &weeks {
                    ui.label(w.label());
                    let mut notes = w.notes.clone();
                    if ui.add(egui::TextEdit::singleline(&mut notes).desired_width(420.0)).changed() {
                        calendar::set_notes(&mut self.ch, &w.guid, &notes);
                        changed = true;
                    }
                    if ui.small_button("🗑").clicked() {
                        remove = Some(w.guid.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(g) = remove {
                changed |= calendar::remove_week(&mut self.ch, &g);
            }
        });
        changed
    }

    fn log_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut changed = false;
        let entries = career::entries(&self.ch);
        let totals = career::totals(&self.ch);
        if self.ch.created {
            let rep = career::reputation_for(engine, &self.ch);
            ui.label(format!(
                "{} {} · {} {} · {} {} · {} {}",
                lang.tr("Career Karma"),
                career::career_karma(&self.ch),
                lang.tr("Street Cred"),
                rep.street_cred,
                lang.tr("Notoriety"),
                rep.notoriety,
                lang.tr("Public Awareness"),
                rep.public_awareness
            ));
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("manual_kind").selected_text(if self.manual.0 { lang.tr("Karma") } else { lang.tr("Nuyen") }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.manual.0, true, lang.tr("Karma"));
                    ui.selectable_value(&mut self.manual.0, false, lang.tr("Nuyen"));
                });
                ui.add(egui::DragValue::new(&mut self.manual.1).range(0.0..=1_000_000.0).max_decimals(2));
                ui.add(egui::TextEdit::singleline(&mut self.manual.2).hint_text(lang.tr("Reason (e.g. run payout)")).desired_width(240.0));
                let ok = self.manual.1 > 0.0;
                let rules = career::CareerRules::for_character(engine, &self.ch);
                let entry = career::ManualExpense { amount: self.manual.1, reason: self.manual.2.clone(), ..Default::default() };
                let mut result = None;
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Gain"))).clicked() {
                    result = Some(if self.manual.0 { career::karma_gained(&mut self.ch, &rules, &entry) } else { career::nuyen_gained(&mut self.ch, &rules, &entry) });
                }
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Spend"))).clicked() {
                    result = Some(if self.manual.0 { career::karma_spent(&mut self.ch, &rules, &entry) } else { career::nuyen_spent(&mut self.ch, &rules, &entry) });
                }
                if let Some(r) = result {
                    match r {
                        Ok(_) => {
                            self.manual.1 = 0.0;
                            self.manual.2.clear();
                            changed = true;
                        }
                        Err(e) => {
                            ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                        }
                    }
                }
            });
        }
        ui.label(lang.tr_fmt(
            "{0} entries · karma earned {1} · spent {2} · nuyen earned {3}",
            &[&entries.len(), &totals.career_karma, &totals.karma_spent, &format::nuyen(totals.career_nuyen)],
        ));
        ui.add_space(4.0);
        if entries.is_empty() {
            ui.weak(lang.tr("No entries yet. Career-mode spending and income appear here."));
            return changed;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("log").striped(true).num_columns(5).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Date", "Type", "Amount", "Reason", ""]) {
                    ui.strong(h);
                }
                ui.end_row();
                for e in entries.iter().rev() {
                    ui.label(e.date.replace('T', " "));
                    let karma = e.kind == career::ExpenseType::Karma;
                    ui.label(if karma { lang.tr("Karma") } else { lang.tr("Nuyen") });
                    let text = if karma { chummer_core::improvement::fmt_num(e.amount) } else { format::nuyen(e.amount) };
                    ui.colored_label(if e.amount < 0.0 { WARN } else { ACCENT }, text);
                    ui.label(&e.reason);
                    if self.ch.created && e.undo.is_some() {
                        if ui.small_button(lang.tr("Undo")).on_hover_text(lang.tr("Reverse this and refund it")).clicked() {
                            self.action = Some(CareerAction::Undo(e.guid.clone()));
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        });
        changed
    }

    fn notes_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (key, label) in TEXT_FIELDS.iter().filter(|(k, _)| matches!(*k, "notes" | "gamenotes")) {
                ui.label(RichText::new(lang.tr(label)).strong());
                let mut v = self.ch.field(key);
                if ui.add(egui::TextEdit::multiline(&mut v).desired_width(f32::INFINITY).desired_rows(12)).changed() {
                    self.ch.set_field(key, v);
                    changed = true;
                }
                ui.add_space(8.0);
            }
        });
        changed
    }
}

/// An item kind's label for use inside a sentence ("Add weapon…"). Only
/// English lowercases it; other languages (German nouns) keep their case.
pub fn kind_noun(lang: &Language, label: &str) -> String {
    let t = lang.tr(label);
    if lang.code.starts_with("en") { t.to_lowercase() } else { t }
}

fn fmt_opt(v: f64) -> String {
    if v == 0.0 {
        String::new()
    } else {
        chummer_core::improvement::fmt_num(v)
    }
}

fn display_name(sec: &Section, it: &Element, lang: &Language) -> String {
    let name = it.get("name");
    let id = it.child_text("sourceid").or_else(|| it.child_text("id")).unwrap_or_default();
    let mut shown = if sec.data_file.is_empty() { name.clone() } else { lang.data_name(sec.data_file, &id, &name) };
    for custom in ["gearname", "weaponname", "armorname", "vehiclename", "crittername"] {
        let c = it.get(custom);
        if !c.is_empty() && c != name {
            shown = format!("{shown} (“{c}”)");
        }
    }
    shown
}

fn cell(it: &Element, field: &str) -> String {
    use chummer_core::expr;
    let v = it.get(field);
    let rating = it.get_i32("rating").unwrap_or(0);
    let min_rating = it.get_i32("minrating").unwrap_or(0);
    // Data expressions such as "Rating * 250" are stored as written; show
    // the value at the item's rating. Strings that need a parent item or a
    // vehicle stay as written.
    let eval = |s: &str| -> Option<f64> {
        let s = expr::fixed_values(s.trim(), rating).replace("MinRating", &min_rating.to_string());
        let r = rating.to_string();
        let s = s.replace("{Rating}", &r).replace("Rating", &r);
        if expr::needs_evaluation(&s) { expr::evaluate_num(&s).ok() } else { expr::parse_plain(&s) }
    };
    match field {
        "equipped" | "bound" => if chummer_core::xml::parse_bool(&v) { "✓".into() } else { String::new() },
        "cost" => eval(&v).map(format::nuyen).unwrap_or(v),
        "avail" if !v.trim().is_empty() && !v.contains("Gear") && !v.contains('{') => {
            expr::Availability::parse(&v, rating, min_rating, &expr::NoAttributes).to_string()
        }
        "ess" => eval(&v).map(|e| format!("{e:.2}")).unwrap_or(v),
        _ => v,
    }
}

/// One table row. Returns the item's guid when its name was clicked.
fn item_rows(ui: &mut egui::Ui, sec: &Section, it: &Element, lang: &Language, depth: usize) -> Option<String> {
    let mut clicked = None;
    for (i, c) in sec.columns.iter().enumerate() {
        if i == 0 {
            let indent = "    ".repeat(depth);
            let mut text = RichText::new(format!("{indent}{}", display_name(sec, it, lang)));
            if depth == 0 {
                text = text.strong();
            }
            let editable = chummer_core::items::edit::is_item(it);
            let r = ui.add(egui::Label::new(text).sense(if editable { egui::Sense::click() } else { egui::Sense::hover() }));
            if r.clicked() {
                clicked = Some(it.get("guid"));
            }
            let notes = it.get("notes");
            let r = if editable { r.on_hover_cursor(egui::CursorIcon::PointingHand) } else { r };
            if !notes.trim().is_empty() {
                r.on_hover_text(notes);
            }
        } else {
            ui.label(cell(it, c.field));
        }
    }
    clicked
}

fn child_rows(ui: &mut egui::Ui, sec: &Section, it: &Element, lang: &Language, depth: usize, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<String> {
    if depth > 6 {
        return None;
    }
    let mut clicked = item_rows(ui, sec, it, lang, depth);
    pdf_ui::source_icon(ui, pdfs, SourceRef::of(it), status);
    ui.end_row();
    for (container, item) in sec.child_containers {
        if let Some(c) = it.child(container) {
            for child in c.children_named(item) {
                clicked = child_rows(ui, sec, child, lang, depth + 1, pdfs, status).or(clicked);
            }
        }
    }
    clicked
}

/// Clickable condition-monitor boxes. Clicking box N sets damage to N, or
/// clears it if N was the last filled box.
fn cm_track(ui: &mut egui::Ui, id: &str, boxes: i32, threshold: i32, filled: &mut i32, color: Color32) -> bool {
    let mut changed = false;
    let size = 18.0;
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
            for n in 1..=boxes.max(0) {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
                let on = n <= *filled;
                let painter = ui.painter();
                painter.rect_filled(rect, 3.0, if on { color } else { ui.visuals().extreme_bg_color });
                painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.fg_stroke.color), egui::StrokeKind::Inside);
                if threshold > 0 && n % threshold == 0 {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("-{}", n / threshold), egui::FontId::proportional(9.0), ui.visuals().weak_text_color());
                }
                if resp.clicked() {
                    *filled = if *filled == n { n - 1 } else { n };
                    changed = true;
                }
            }
        });
    });
    changed
}

fn describe_action(a: &CareerAction) -> String {
    match a {
        CareerAction::RaiseAttribute(n) => format!("raised {n}"),
        CareerAction::RaiseSkill(_) => "raised skill".into(),
        CareerAction::RaiseGroup(g) => format!("raised {g}"),
        CareerAction::Specialize(_, n) => format!("specialized in {n}"),
        CareerAction::LearnKnowledge(n, _) => format!("learned {n}"),
        CareerAction::Undo(_) => "undone".into(),
        CareerAction::Initiate(_) => "initiated".into(),
        CareerAction::RemoveQuality(_) => "quality removed".into(),
    }
}
