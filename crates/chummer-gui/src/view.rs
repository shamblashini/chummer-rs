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

const TABS: &[(Tab, &str)] = &[
    (Tab::Info, "Info"),
    (Tab::Attributes, "Attributes"),
    (Tab::Skills, "Skills"),
    (Tab::Qualities, "Qualities & Contacts"),
    (Tab::Magic, "Magic & Resonance"),
    (Tab::Equipment, "Equipment"),
    (Tab::Improvements, "Improvements"),
    (Tab::Log, "Karma & Nuyen"),
    (Tab::Notes, "Notes"),
];

pub struct CharacterView {
    pub ch: Character,
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
        let sheet = calc::compute(&ch, &rules, Some(&engine.store), Some(&engine.catalog));
        let settings = engine.settings.resolve(&ch.field("settings")).cloned();
        let mut v = CharacterView {
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
        };
        v.refresh_budget();
        v
    }

    fn refresh_budget(&mut self) {
        match (&self.settings, self.ch.created) {
            (Some(st), false) => {
                let b = chargen::budget(&self.ch, &self.sheet, &self.rules, st);
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
        self.sheet = calc::compute(&self.ch, &self.rules, Some(&engine.store), Some(&engine.catalog));
        self.refresh_budget();
    }

    pub fn ui(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<u32> {
        let mut changed = false;
        let mut roll: Option<u32> = None;
        egui::SidePanel::right("sheet_panel").resizable(true).default_width(270.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                changed |= self.sidebar(ui, &mut roll);
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (t, label) in TABS {
                    if *t == Tab::Magic && !(self.ch.mag_enabled() || self.ch.res_enabled() || !self.ch.items("critterpowers", "critterpower").is_empty()) {
                        continue;
                    }
                    ui.selectable_value(&mut self.tab, *t, *label);
                }
            });
            ui.separator();
            changed |= match self.tab {
                Tab::Info => self.info_tab(ui),
                Tab::Attributes => self.attributes_tab(ui),
                Tab::Skills => self.skills_tab(ui, engine, pdfs, status, &mut roll),
                Tab::Qualities => {
                    let mut c = false;
                    egui::ScrollArea::both().show(ui, |ui| {
                        if ui.button("➕ Add quality…").clicked() {
                            let books = self.settings.as_ref().map(|s| s.books()).unwrap_or_default();
                            self.select = SelectDialog::new(select::QUALITY, &engine.store, books);
                        }
                        c |= self.section(ui, &sections::QUALITIES, lang, pdfs, status);
                        ui.add_space(12.0);
                        c |= self.contact_form(ui);
                        c |= self.section(ui, &sections::CONTACTS, lang, pdfs, status);
                    });
                    c
                }
                Tab::Magic => self.magic_tab(ui, engine, lang, pdfs, status),
                Tab::Equipment => self.equipment_tab(ui, lang, pdfs, status),
                Tab::Improvements => self.improvements_tab(ui),
                Tab::Log => self.log_tab(ui),
                Tab::Notes => self.notes_tab(ui),
            };
        });
        changed |= self.confirm_dialog(ctx);
        changed |= self.select_dialog(ctx, engine, lang, pdfs, status);
        changed |= self.finish_dialog(ctx);
        if changed {
            self.ch.dirty = true;
            self.recompute(engine);
        }
        roll
    }

    // ----- sidebar -----

    fn sidebar(&mut self, ui: &mut egui::Ui, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        let s = &self.sheet;
        ui.add_space(4.0);
        ui.heading(RichText::new(self.ch.display_name()).color(ACCENT));
        let meta: Vec<String> = ["metatype", "metavariant"].iter().map(|k| self.ch.field(k)).filter(|v| !v.is_empty()).collect();
        ui.label(meta.join(" · "));
        ui.weak(format!(
            "{} · {}",
            if self.ch.created { "Career" } else { "Creation" },
            match self.ch.field("buildmethod").as_str() {
                "SumtoTen" => "Sum-to-Ten".to_owned(),
                "" => "Priority".to_owned(),
                b => b.to_owned(),
            }
        ));
        ui.separator();

        egui::Grid::new("resources").num_columns(2).show(ui, |ui| {
            ui.label("Karma");
            changed |= ui.add(egui::DragValue::new(&mut self.ch.karma).speed(0.2)).changed();
            ui.end_row();
            ui.label("Nuyen");
            ui.horizontal(|ui| {
                changed |= ui.add(egui::DragValue::new(&mut self.ch.nuyen).speed(10.0).max_decimals(2).suffix("¥")).changed();
            });
            ui.end_row();
            ui.label("Essence");
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
            stat(ui, "Initiative", format!("{} + {}d6", s.initiative, s.initiative_dice));
            stat(ui, "Astral", format!("{} + {}d6", s.astral_initiative, s.astral_initiative_dice));
            stat(ui, "Matrix cold-sim", format!("{} + {}d6", s.matrix_cold_initiative, s.matrix_cold_dice));
            stat(ui, "Matrix hot-sim", format!("{} + {}d6", s.matrix_hot_initiative, s.matrix_hot_dice));
            stat(ui, "Physical limit", s.limit_physical.to_string());
            stat(ui, "Mental limit", s.limit_mental.to_string());
            stat(ui, "Social limit", s.limit_social.to_string());
            if self.ch.mag_enabled() {
                stat(ui, "Astral limit", s.limit_astral.to_string());
            }
            stat(ui, "Armor", s.armor.to_string());
            stat(ui, "Composure", s.composure.to_string());
            stat(ui, "Judge Intentions", s.judge_intentions.to_string());
            stat(ui, "Memory", s.memory.to_string());
            stat(ui, "Lift / Carry", s.lift_carry.to_string());
            if s.wound_modifier != 0 {
                ui.colored_label(WARN, "Wound modifier");
                ui.colored_label(WARN, s.wound_modifier.to_string());
                ui.end_row();
            }
        });
        ui.separator();

        let (pcm, scm, thr) = (s.physical_cm, s.stun_cm, s.cm_threshold);
        ui.label(RichText::new("Physical damage").strong());
        changed |= cm_track(ui, "pcm", pcm, thr, &mut self.ch.physical_cm_filled, Color32::from_rgb(200, 60, 60));
        ui.label(RichText::new("Stun damage").strong());
        changed |= cm_track(ui, "scm", scm, thr, &mut self.ch.stun_cm_filled, Color32::from_rgb(70, 130, 220));
        ui.weak(format!("Overflow {} · −1 die per {thr} boxes", s.cm_overflow));
        ui.separator();
        changed |= self.budget_panel(ui);
        if ui.button("🎲 Open dice roller").clicked() {
            *roll = Some(6);
        }
        changed
    }

    // ----- tabs -----

    fn info_tab(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("info").num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                for (i, (key, label)) in INFO_FIELDS.iter().enumerate() {
                    ui.label(*label);
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
                for (key, label) in [("streetcred", "Street Cred"), ("notoriety", "Notoriety"), ("publicawareness", "Public Awareness")] {
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
                ui.label(RichText::new(*label).strong());
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

    fn attributes_tab(&mut self, ui: &mut egui::Ui) -> bool {
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
            "Career mode: raising an attribute here spends no karma automatically — adjust Karma in the sidebar."
        } else {
            "Creation mode: base uses attribute points (priority builds). Changing levels does not deduct karma automatically; the Karma cost column shows what they are worth."
        });
        ui.add_space(6.0);
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::exact(130.0))
            .columns(Column::auto().at_least(60.0), 6)
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for t in ["Attribute", "Min/Max", "Base", "Karma", "Natural", "Augmented", "Karma cost", "Next level"] {
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
                            ui.label(format!("{} ({name})", attributes::long_name(name)));
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
                                changed |= ui.add(egui::DragValue::new(&mut a.karma).range(0..=max)).changed();
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
                            match calc::attribute_upgrade_cost(&v, &self.rules) {
                                Some(c) => ui.label(format!("{c} karma")),
                                None => ui.weak("at maximum"),
                            };
                        });
                    });
                }
            });
        ui.add_space(8.0);
        ui.label(format!("Karma spent on attributes: {}", self.sheet.attribute_karma_spent));
        changed
    }

    fn skills_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.skill_filter).hint_text("Filter skills").desired_width(200.0));
            ui.checkbox(&mut self.only_rated, "Only skills with a rating");
            ui.separator();
            if self.ch.created {
                ui.label(format!("Karma value of skills: {}", self.sheet.skill_karma_spent));
            } else {
                ui.label(format!(
                    "Knowledge points: {} / {} · karma spent on skills: {}",
                    self.sheet.knowledge_points_used, self.sheet.knowledge_points, self.sheet.skill_karma_spent
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Active skills");
            egui::Grid::new("skills").striped(true).num_columns(8).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in ["Skill", "Attr", "Group", "Base", "Karma", "Rating", "Pool", "Specializations"] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, s) in &rows {
                    let r = SourceRef::new(&s.source, &s.page);
                    let name = ui.add(egui::Label::new(&s.name).sense(egui::Sense::click()));
                    if let Some(r) = r {
                        if name.on_hover_text(format!("{r} — click to open the rulebook")).clicked() {
                            pdf_ui::open(pdfs, &r, status);
                        }
                    }
                    ui.label(&s.attribute);
                    ui.weak(&s.group);
                    let sk = &mut self.ch.skills[*i];
                    changed |= ui.add_enabled(!career, egui::DragValue::new(&mut sk.base).range(0..=12)).changed();
                    changed |= ui.add(egui::DragValue::new(&mut sk.karma).range(0..=12)).changed();
                    ui.label(s.rating.to_string());
                    let pool = if s.rating == 0 && !s.default { "—".to_owned() } else { s.pool.to_string() };
                    if ui.add(egui::Button::new(RichText::new(pool).strong()).frame(false)).on_hover_text("Roll this pool").clicked() {
                        *roll = Some(s.pool.max(1) as u32);
                    }
                    ui.horizontal(|ui| {
                        if !s.specs.is_empty() {
                            ui.label(format!("{} (+{})", s.specs.join(", "), s.spec_bonus));
                        }
                        let guid = s.guid.clone();
                        let suid = self.ch.skills[*i].suid.clone();
                        ui.menu_button("＋", |ui| {
                            let opts = engine.catalog.get(&suid).map(|d| d.specs.clone()).unwrap_or_default();
                            for o in opts.iter().filter(|o| !s.specs.contains(o)) {
                                if ui.button(o).clicked() {
                                    chargen::add_specialization(&mut self.ch, &guid, o);
                                    changed = true;
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Add a specialization");
                    });
                    ui.end_row();
                }
            });
            ui.add_space(12.0);
            ui.heading("Knowledge & language skills");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.new_kno.0).hint_text("New knowledge skill").desired_width(200.0));
                egui::ComboBox::from_id_salt("kno_type").selected_text(self.new_kno.1.clone()).show_ui(ui, |ui| {
                    for t in ["Academic", "Interest", "Language", "Professional", "Street"] {
                        ui.selectable_value(&mut self.new_kno.1, t.to_owned(), t);
                    }
                });
                if self.new_kno.1 == "Language" {
                    ui.checkbox(&mut self.new_kno.2, "Native");
                }
                if ui.add_enabled(!self.new_kno.0.trim().is_empty(), egui::Button::new("Add")).clicked() {
                    let native = self.new_kno.1 == "Language" && self.new_kno.2;
                    chargen::add_knowledge_skill(&mut self.ch, self.new_kno.0.trim(), &self.new_kno.1.clone(), native);
                    self.new_kno.0.clear();
                    changed = true;
                }
            });
            let mut remove_kno: Option<String> = None;
            egui::Grid::new("kskills").striped(true).num_columns(7).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in ["Skill", "Type", "Base", "Karma", "Rating", "Pool", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, s) in &kno {
                    ui.label(&s.name);
                    ui.weak(&s.category);
                    if s.native {
                        ui.weak("native");
                        ui.label("");
                        ui.label("N");
                        ui.label("N");
                    } else {
                        let k = &mut self.ch.knowledge_skills[*i];
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut k.base).range(0..=12)).changed();
                        changed |= ui.add(egui::DragValue::new(&mut k.karma).range(0..=12)).changed();
                        ui.label(s.rating.to_string());
                        ui.strong(s.pool.to_string());
                    }
                    if ui.small_button("🗑").on_hover_text("Remove").clicked() {
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
                ui.heading("Skill groups");
                egui::Grid::new("groups").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                    for h in ["Group", "Base", "Karma", "Rating"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for g in &mut self.ch.skill_groups {
                        ui.label(&g.name);
                        changed |= ui.add_enabled(!career, egui::DragValue::new(&mut g.base).range(0..=12)).changed();
                        changed |= ui.add(egui::DragValue::new(&mut g.karma).range(0..=12)).changed();
                        ui.label(g.rating().to_string());
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
                ui.label("Tradition");
                let mut pick: Option<String> = None;
                egui::ComboBox::from_id_salt("tradition").selected_text(if current.is_empty() { "Choose…".to_owned() } else { current.clone() }).width(240.0).show_ui(ui, |ui| {
                    if let Ok(doc) = engine.store.doc("traditions.xml") {
                        for r in data::records(&doc, "traditions", "tradition") {
                            if ui.selectable_label(current == r.name(), r.name()).clicked() {
                                pick = Some(r.name());
                            }
                        }
                    }
                });
                if let Some(p) = pick {
                    if let Err(e) = chargen::set_tradition(&mut self.ch, &engine.store, &p) {
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
                ui.selectable_value(&mut self.magic, i, format!("{} ({n})", s.label));
            }
        });
        let tradition = self.ch.doc.child("tradition").map(|t| t.get("name")).unwrap_or_default();
        if !tradition.is_empty() {
            ui.label(format!("Tradition: {tradition} · drain {}", self.ch.doc.child("tradition").map(|t| t.get("drain")).unwrap_or_default()));
        }
        let grade = self.ch.doc.get_i32("initiategrade").unwrap_or(0);
        if grade > 0 {
            ui.label(format!("Initiate grade {grade}"));
        }
        ui.separator();
        if let Some(sec) = present.get(self.magic) {
            let sec = **sec;
            egui::ScrollArea::both().show(ui, |ui| changed |= self.section(ui, &sec, lang, pdfs, status));
        }
        changed
    }

    fn equipment_tab(&mut self, ui: &mut egui::Ui, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            for (i, s) in sections::EQUIPMENT.iter().enumerate() {
                let n = self.ch.items(s.container, s.item).len();
                ui.selectable_value(&mut self.equipment, i, format!("{} ({n})", s.label));
            }
        });
        ui.separator();
        let sec = sections::EQUIPMENT[self.equipment];
        egui::ScrollArea::both().show(ui, |ui| changed |= self.section(ui, &sec, lang, pdfs, status));
        changed
    }

    /// A table of items. Returns true if the character changed.
    fn section(&mut self, ui: &mut egui::Ui, sec: &Section, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let items: Vec<Element> = self.ch.items(sec.container, sec.item).into_iter().cloned().collect();
        ui.heading(format!("{} ({})", sec.label, items.len()));
        if items.is_empty() {
            ui.weak("None.");
            return false;
        }
        egui::Grid::new(sec.container).striped(true).num_columns(sec.columns.len() + 1).spacing([14.0, 4.0]).show(ui, |ui| {
            for c in sec.columns {
                ui.strong(c.header);
            }
            ui.label("");
            ui.end_row();
            for it in &items {
                item_rows(ui, sec, it, lang, 0);
                ui.horizontal(|ui| {
                    pdf_ui::source_icon(ui, pdfs, SourceRef::of(it), status);
                    if ui.small_button("🗑").on_hover_text("Remove (also removes its improvements)").clicked() {
                        self.confirm_remove = Some((sec.container.to_owned(), it.get("guid"), display_name(sec, it, lang)));
                    }
                });
                ui.end_row();
                for (container, item) in sec.child_containers {
                    if let Some(c) = it.child(container) {
                        for child in c.children_named(item) {
                            child_rows(ui, sec, child, lang, 1, pdfs, status);
                        }
                    }
                }
            }
        });
        false
    }

    /// Confirmation dialog for item removal. Returns true if an item went.
    fn confirm_dialog(&mut self, ctx: &egui::Context) -> bool {
        let Some((container, guid, name)) = self.confirm_remove.clone() else { return false };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm_remove")).show(ctx, |ui| {
            ui.heading("Remove item");
            ui.label(format!("Remove {name}? Its improvements are removed too. This cannot be undone."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Remove").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(yes) => {
                self.confirm_remove = None;
                if yes && container == "qualities" {
                    chargen::remove_quality(&mut self.ch, &guid);
                    true
                } else {
                    yes && self.ch.remove_item(&container, &guid)
                }
            }
            None => false,
        }
    }

    /// Creation-mode budgets in the sidebar, with Finish creation.
    fn budget_panel(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(b) = self.budget.clone() else { return false };
        let mut changed = false;
        ui.label(RichText::new("Creation").strong());
        let row = |ui: &mut egui::Ui, label: &str, total: i32, used: i32| {
            let left = total - used;
            ui.label(label);
            let t = RichText::new(format!("{left} / {total}"));
            ui.label(if left < 0 { t.color(ui.visuals().error_fg_color) } else if left == 0 { t.weak() } else { t.color(ACCENT) });
            ui.end_row();
        };
        egui::Grid::new("budget").num_columns(2).striped(true).show(ui, |ui| {
            row(ui, "Karma", b.karma.0, b.karma.1);
            row(ui, "Attribute points", b.attribute_points.0, b.attribute_points.1);
            row(ui, "Special points", b.special_points.0, b.special_points.1);
            row(ui, "Skill points", b.skill_points.0, b.skill_points.1);
            row(ui, "Skill group points", b.skill_group_points.0, b.skill_group_points.1);
            row(ui, "Knowledge points", b.knowledge_points.0, b.knowledge_points.1);
            row(ui, "Contact points", b.contact_points.0, b.contact_points.1);
            if b.free_spells.0 > 0 {
                row(ui, "Free spells", b.free_spells.0, b.free_spells.1);
            }
            ui.label("Positive qualities");
            ui.label(format!("{} / {}", b.positive_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label("Negative qualities");
            ui.label(format!("{} / {}", b.negative_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label("Nuyen left");
            let left = b.nuyen_left();
            let t = RichText::new(chummer_core::format::nuyen(left));
            ui.label(if left < 0.0 { t.color(ui.visuals().error_fg_color) } else { t });
            ui.end_row();
            ui.label("Karma for nuyen");
            let mut bp = self.ch.doc.get_i32("nuyenbp").unwrap_or(0);
            let max = self.settings.as_ref().map_or(10, |s| s.int("nuyenmaxbp", 10));
            if ui.add(egui::DragValue::new(&mut bp).range(0..=max).suffix(" karma")).on_hover_text("2,000¥ per karma").changed() {
                self.ch.set_field("nuyenbp", bp.to_string());
                changed = true;
            }
            ui.end_row();
        });
        for p in &self.problems {
            ui.colored_label(WARN, format!("• {p}"));
        }
        let ok = self.problems.is_empty();
        let r = ui.add_enabled(ok, egui::Button::new(RichText::new("Finish creation").color(ACCENT)));
        if r.on_disabled_hover_text("Fix the problems above first").clicked() {
            self.confirm_finish = true;
        }
        ui.separator();
        changed
    }

    fn finish_dialog(&mut self, ctx: &egui::Context) -> bool {
        if !self.confirm_finish {
            return false;
        }
        let mut choice = None;
        let b = self.budget.clone().unwrap_or_default();
        egui::Modal::new(egui::Id::new("finish_creation")).show(ctx, |ui| {
            ui.heading("Finish creation?");
            ui.label("The character switches to career mode. Creation budgets go away; karma and nuyen become plain resources.");
            let carry_k = self.settings.as_ref().map_or(7, |s| s.karma("karmacarryover", 7));
            if b.karma_left() > carry_k {
                ui.colored_label(WARN, format!("{} karma is left, only {carry_k} carries over.", b.karma_left()));
            }
            if b.nuyen_left() > 5000.0 {
                ui.colored_label(WARN, format!("{} is left, only 5,000¥ carries over.", chummer_core::format::nuyen(b.nuyen_left())));
            }
            ui.horizontal(|ui| {
                if ui.button("Finish").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
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

    fn select_dialog(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let Some(dlg) = self.select.as_mut() else { return false };
        let ch = &self.ch;
        let store = &engine.store;
        let choices_for = |rec: &chummer_core::xml::Element| chummer_core::items::quality_choices(ch, store, data::Record(rec));
        match dlg.show(ctx, ch, &self.sheet, lang, pdfs, status, &choices_for) {
            select::Outcome::None => false,
            select::Outcome::Cancel => {
                self.select = None;
                false
            }
            select::Outcome::Done { index, answer } => {
                let Some(rec) = dlg.record(store, index) else { return false };
                let rec = data::Record(&rec);
                let karma = rec.el().get_i32("karma").unwrap_or(0);
                chargen::add_quality(&mut self.ch, store, rec, answer.as_deref());
                if self.ch.created && karma > 0 {
                    // Career mode: positive qualities cost double karma.
                    let cost = if rec.el().get_bool("doublecareer").unwrap_or(true) { karma * 2 } else { karma };
                    self.ch.karma -= cost;
                    *status = Some((format!("Added {} for {cost} karma", rec.name()), false));
                } else {
                    *status = Some((format!("Added {}", rec.name()), false));
                }
                self.select = None;
                true
            }
        }
    }

    fn contact_form(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_contact.0).hint_text("Contact name").desired_width(160.0));
            ui.add(egui::TextEdit::singleline(&mut self.new_contact.1).hint_text("Role").desired_width(120.0));
            ui.label("Connection");
            ui.add(egui::DragValue::new(&mut self.new_contact.2).range(1..=12));
            ui.label("Loyalty");
            ui.add(egui::DragValue::new(&mut self.new_contact.3).range(1..=6));
            if ui.add_enabled(!self.new_contact.0.trim().is_empty(), egui::Button::new("➕ Add contact")).clicked() {
                let (n, r, c, l) = self.new_contact.clone();
                chargen::add_contact(&mut self.ch, n.trim(), r.trim(), c, l);
                self.new_contact.0.clear();
                self.new_contact.1.clear();
                changed = true;
            }
        });
        changed
    }

    fn improvements_tab(&mut self, ui: &mut egui::Ui) -> bool {
        let imps = &self.ch.improvements;
        ui.label(format!(
            "{} improvements ({} active). These modifiers come from qualities, ware, powers and gear.",
            imps.list.len(),
            imps.active().count()
        ));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("imps").striped(true).num_columns(7).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in ["Type", "Target", "Value", "Aug", "Min/Max", "Source", "Condition"] {
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

    fn log_tab(&mut self, ui: &mut egui::Ui) -> bool {
        let entries: Vec<Element> = self.ch.items("expenses", "expense").into_iter().cloned().collect();
        let (mut karma, mut nuyen) = (0.0, 0.0);
        for e in &entries {
            let amount = e.get_f64("amount").unwrap_or(0.0);
            if e.get("type") == "Karma" {
                karma += amount;
            } else {
                nuyen += amount;
            }
        }
        ui.label(format!("{} entries · karma total {karma} · nuyen total {}", entries.len(), format::nuyen(nuyen)));
        ui.add_space(4.0);
        if entries.is_empty() {
            ui.weak("No expenses recorded. Career-mode spending appears here.");
            return false;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("log").striped(true).num_columns(4).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in ["Date", "Type", "Amount", "Reason"] {
                    ui.strong(h);
                }
                ui.end_row();
                for e in entries.iter().rev() {
                    ui.label(e.get("date").replace('T', " "));
                    ui.label(e.get("type"));
                    let a = e.get_f64("amount").unwrap_or(0.0);
                    let text = if e.get("type") == "Karma" { format!("{a}") } else { format::nuyen(a) };
                    ui.colored_label(if a < 0.0 { WARN } else { ACCENT }, text);
                    ui.label(e.get("reason"));
                    ui.end_row();
                }
            });
        });
        false
    }

    fn notes_tab(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (key, label) in TEXT_FIELDS.iter().filter(|(k, _)| matches!(*k, "notes" | "gamenotes")) {
                ui.label(RichText::new(*label).strong());
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

fn item_rows(ui: &mut egui::Ui, sec: &Section, it: &Element, lang: &Language, depth: usize) {
    for (i, c) in sec.columns.iter().enumerate() {
        if i == 0 {
            let indent = "    ".repeat(depth);
            let mut text = RichText::new(format!("{indent}{}", display_name(sec, it, lang)));
            if depth == 0 {
                text = text.strong();
            }
            let r = ui.label(text);
            let notes = it.get("notes");
            if !notes.trim().is_empty() {
                r.on_hover_text(notes);
            }
        } else {
            ui.label(cell(it, c.field));
        }
    }
}

fn child_rows(ui: &mut egui::Ui, sec: &Section, it: &Element, lang: &Language, depth: usize, pdfs: &SourcebookLibrary, status: &mut Status) {
    if depth > 6 {
        return;
    }
    item_rows(ui, sec, it, lang, depth);
    pdf_ui::source_icon(ui, pdfs, SourceRef::of(it), status);
    ui.end_row();
    for (container, item) in sec.child_containers {
        if let Some(c) = it.child(container) {
            for child in c.children_named(item) {
                child_rows(ui, sec, child, lang, depth + 1, pdfs, status);
            }
        }
    }
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
                painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color), egui::StrokeKind::Inside);
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
