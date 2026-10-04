//! GM tools: the New Critter dialog (`SelectMetatypeKarma` for critters),
//! PACKS kits (`SelectPACKSKit`, `CreatePACKSKit`) and the custom spell
//! designer (`CreateSpell`). The rules live in `chummer_core::gm`.

use chummer_core::calc::Sheet;
use chummer_core::character::Character;
use chummer_core::chargen;
use chummer_core::data::{self, DataStore};
use chummer_core::engine::Engine;
use chummer_core::gm::critter::{self, ForceKind, NewCritter};
use chummer_core::gm::custom_spell::{self, SpellDesign};
use chummer_core::gm::packs::{self, KitParts};
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;

// ---------------------------------------------------------------------------
// New Critter
// ---------------------------------------------------------------------------

pub struct CritterWizard {
    preset: usize,
    category: String,
    search: String,
    /// Id of the selected critter.
    selected: Option<String>,
    metavariant: String,
    force: i32,
    possession: bool,
    method: String,
    picks: Vec<String>,
    name: String,
    error: Option<String>,
}

pub enum CritterResult {
    Open,
    Cancel,
    Created(Box<Character>),
}

impl CritterWizard {
    pub fn new() -> Self {
        CritterWizard {
            preset: 0,
            category: String::new(),
            search: String::new(),
            selected: None,
            metavariant: String::new(),
            force: 1,
            possession: false,
            method: "Possession".into(),
            picks: Vec::new(),
            name: String::new(),
            error: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> CritterResult {
        let presets = chargen::creation_presets(engine);
        if presets.is_empty() {
            return CritterResult::Cancel;
        }
        self.preset = self.preset.min(presets.len() - 1);
        let settings: CharacterSettings = presets[self.preset].clone();
        let store = engine.store_for(&settings);
        let options = critter::critter_options(&store, &settings.books());
        let mut result = CritterResult::Open;
        let mut open = true;
        egui::Window::new(lang.tr("New Critter")).id(egui::Id::new("new_critter")).open(&mut open).default_size([760.0, 600.0]).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("critter_top").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(lang.tr("Name"));
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label(lang.tr("Rules"));
                crate::combo::Combo::from_id_salt("critter_preset").selected_text(settings.name()).width(260.0).show_ui(ui, |ui| {
                    for (i, p) in presets.iter().enumerate() {
                        crate::combo::selectable_value(ui, &mut self.preset, i, p.name());
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Category"));
                let shown = if self.category.is_empty() { lang.tr("Show All") } else { self.category.clone() };
                crate::combo::Combo::from_id_salt("critter_cat").selected_text(shown).width(260.0).show_ui(ui, |ui| {
                    crate::combo::selectable_value(ui, &mut self.category, String::new(), lang.tr("Show All"));
                    for c in critter::categories(&store) {
                        if options.iter().any(|o| o.category == c) {
                            crate::combo::selectable_value(ui, &mut self.category, c.clone(), c);
                        }
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Search"));
                ui.text_edit_singleline(&mut self.search);
                ui.end_row();
            });
            ui.separator();
            ui.columns(2, |cols| {
                let needle = self.search.to_lowercase();
                egui::ScrollArea::vertical().id_salt("critter_list").max_height(380.0).show(&mut cols[0], |ui| {
                    for o in options.iter().filter(|o| (self.category.is_empty() || o.category == self.category) && (needle.is_empty() || o.name.to_lowercase().contains(&needle))) {
                        if crate::combo::selectable_label(ui, self.selected.as_deref() == Some(o.id.as_str()), &o.name).clicked() {
                            self.selected = Some(o.id.clone());
                            self.metavariant.clear();
                            self.picks.clear();
                            self.force = self.force.clamp(1, o.force.max().max(1));
                        }
                    }
                });
                let ui = &mut cols[1];
                let Some(o) = self.selected.as_ref().and_then(|id| options.iter().find(|o| &o.id == id)) else {
                    ui.weak(lang.tr("Choose a critter."));
                    return;
                };
                ui.heading(RichText::new(&o.name).color(crate::theme::accent(ui)));
                ui.weak(format!("{} · {} {}", o.category, o.source, o.page));
                if !o.metavariants.is_empty() {
                    let shown = o.metavariants.iter().find(|(id, _)| *id == self.metavariant).map_or_else(|| lang.tr("None"), |(_, n)| n.clone());
                    crate::combo::Combo::from_id_salt("critter_variant").selected_text(shown).show_ui(ui, |ui| {
                        crate::combo::selectable_value(ui, &mut self.metavariant, String::new(), lang.tr("None"));
                        for (id, n) in &o.metavariants {
                            crate::combo::selectable_value(ui, &mut self.metavariant, id.clone(), n);
                        }
                    });
                }
                match o.force {
                    ForceKind::None => {}
                    ForceKind::Force { levels } => {
                        ui.horizontal(|ui| {
                            ui.label(if levels { lang.tr("Level") } else { lang.tr("Force") });
                            ui.add(egui::DragValue::new(&mut self.force).range(1..=o.force.max()));
                        });
                    }
                    ForceKind::Dice { dice } => {
                        ui.horizontal(|ui| {
                            ui.label(format!("{dice}D6"));
                            ui.add(egui::DragValue::new(&mut self.force).range(1..=o.force.max()));
                        });
                    }
                }
                if o.offers_possession() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.possession, lang.tr("Summoned by Possession-based Tradition"));
                        ui.add_enabled_ui(self.possession, |ui| {
                            crate::combo::Combo::from_id_salt("critter_possession").selected_text(self.method.clone()).show_ui(ui, |ui| {
                                for m in ["Inhabitation", "Possession"] {
                                    crate::combo::selectable_value(ui, &mut self.method, m.to_owned(), m);
                                }
                            });
                        });
                    });
                }
                let force = if o.force == ForceKind::None { 0 } else { self.force };
                let mv = (!self.metavariant.is_empty()).then_some(self.metavariant.as_str());
                if let Some((count, powers)) = critter::optional_power_slots(&store, &o.id, mv, force).filter(|(c, p)| *c > 0 && p.len() > 1) {
                    ui.add_space(6.0);
                    ui.label(lang.tr("Choose a Power to gain."));
                    self.picks.resize(count, String::new());
                    for i in 0..count {
                        let cur = self.picks[i].clone();
                        crate::combo::Combo::from_id_salt(("critter_pick", i)).selected_text(if cur.is_empty() { lang.tr("Choose…") } else { cur.clone() }).width(220.0).show_ui(ui, |ui| {
                            for p in &powers {
                                if crate::combo::selectable_label(ui, cur == *p, p).clicked() {
                                    self.picks[i] = p.clone();
                                }
                            }
                        });
                    }
                }
            });
            ui.separator();
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.selected.is_some(), crate::theme::primary_button(ui, lang.tr("Create Critter"))).clicked() {
                    let o = options.iter().find(|o| Some(&o.id) == self.selected.as_ref());
                    let spec = NewCritter {
                        settings_id: settings.key(),
                        metatype: self.selected.clone().unwrap_or_default(),
                        metavariant: (!self.metavariant.is_empty()).then(|| self.metavariant.clone()),
                        force: self.force,
                        possession: (self.possession && o.is_some_and(|o| o.offers_possession())).then(|| self.method.clone()),
                        optional_powers: self.picks.iter().filter(|p| !p.is_empty()).cloned().collect(),
                        name: if self.name.trim().is_empty() { o.map(|o| o.name.clone()).unwrap_or_default() } else { self.name.trim().to_owned() },
                    };
                    match critter::create_with(&store, &settings, &spec) {
                        Ok(ch) => result = CritterResult::Created(Box::new(ch)),
                        Err(e) => self.error = Some(e),
                    }
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    result = CritterResult::Cancel;
                }
            });
        });
        if !open {
            return CritterResult::Cancel;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// PACKS kits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacksMode {
    Add,
    Create,
}

#[derive(Default)]
pub struct PacksWindow {
    mode: Option<PacksMode>,
    /// `packs.xml` with custom kits, loaded when the window opens.
    doc: Option<Element>,
    category: String,
    selected: Option<(String, String)>,
    confirm_delete: bool,
    kit_name: String,
    file_name: String,
    parts: Option<KitParts>,
    message: Option<(String, bool)>,
}

impl PacksWindow {
    pub fn open(&mut self, mode: PacksMode) {
        self.mode = Some(mode);
        self.doc = None;
        self.message = None;
        self.confirm_delete = false;
    }

    /// Draw the window when open. Returns true if the character changed.
    #[allow(clippy::too_many_arguments)]
    pub fn window(&mut self, ctx: &egui::Context, ch: &mut Character, store: &DataStore, settings: Option<&CharacterSettings>, sheet: &Sheet, lang: &Language, status: &mut Status) -> bool {
        let Some(mode) = self.mode else { return false };
        let dir = packs::packs_dir();
        if self.doc.is_none() {
            self.doc = Some(packs::load(store, dir.as_deref()));
        }
        let mut changed = false;
        let mut open = true;
        let title = if mode == PacksMode::Add { lang.tr("Select a PACKS Kit") } else { lang.tr("Create PACKS Kit") };
        egui::Window::new(title).id(egui::Id::new("packs_kit")).open(&mut open).default_size([760.0, 520.0]).collapsible(false).show(ctx, |ui| {
            if ch.created {
                ui.colored_label(crate::theme::warn(ui), lang.tr("PACKS kits can only be used while the character is in Create Mode."));
                return;
            }
            match mode {
                PacksMode::Add => changed |= self.add_ui(ui, ch, store, settings, dir.as_deref(), lang, status),
                PacksMode::Create => self.create_ui(ui, ch, sheet, settings, dir.as_deref(), lang),
            }
            if let Some((m, err)) = &self.message {
                if *err {
                    ui.colored_label(ui.visuals().error_fg_color, m);
                } else {
                    ui.colored_label(crate::theme::accent(ui), m);
                }
            }
        });
        if !open {
            self.mode = None;
        }
        changed
    }

    #[allow(clippy::too_many_arguments)]
    fn add_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, settings: Option<&CharacterSettings>, dir: Option<&std::path::Path>, lang: &Language, status: &mut Status) -> bool {
        let Some(doc) = self.doc.clone() else { return false };
        let kits = packs::kits(&doc);
        let cats: Vec<String> = packs::categories(&doc).into_iter().filter(|c| kits.iter().any(|(_, k)| k == c)).collect();
        if self.category.is_empty() || !cats.contains(&self.category) {
            self.category = cats.first().cloned().unwrap_or_default();
        }
        ui.horizontal(|ui| {
            ui.label(lang.tr("Category"));
            crate::combo::Combo::from_id_salt("packs_cat").selected_text(self.category.clone()).width(240.0).show_ui(ui, |ui| {
                for c in &cats {
                    if crate::combo::selectable_label(ui, *c == self.category, c).clicked() {
                        self.category = c.clone();
                        self.selected = None;
                    }
                }
            });
        });
        ui.separator();
        let mut changed = false;
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("packs_list").max_height(360.0).show(&mut cols[0], |ui| {
                for (n, c) in kits.iter().filter(|(_, c)| *c == self.category) {
                    let sel = self.selected.as_ref().is_some_and(|(sn, sc)| sn == n && sc == c);
                    if crate::combo::selectable_label(ui, sel, n).clicked() {
                        self.selected = Some((n.clone(), c.clone()));
                        self.confirm_delete = false;
                    }
                }
            });
            let ui = &mut cols[1];
            let Some((name, cat)) = self.selected.clone() else {
                ui.weak(lang.tr("Choose a kit."));
                return;
            };
            let Some(kit) = packs::find_kit(&doc, &name, &cat) else { return };
            ui.label(RichText::new(lang.tr("Contents:")).strong());
            egui::ScrollArea::vertical().id_salt("packs_contents").max_height(300.0).show(ui, |ui| {
                for (section, lines) in packs::contents(kit) {
                    ui.label(RichText::new(lang.tr(section)).color(crate::theme::accent(ui)));
                    for l in lines {
                        ui.label(format!("  {l}"));
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.add(crate::theme::primary_button(ui, lang.tr("Add Kit"))).clicked() {
                    let report = packs::apply(ch, store, settings, kit);
                    changed = true;
                    let msg = lang.tr_fmt("Added {0}: {1} items", &[&name, &report.added.len()]);
                    self.message = Some(if report.skipped.is_empty() { (msg.clone(), false) } else { (format!("{msg}\n{}", report.skipped.join("\n")), true) });
                    *status = Some((msg, false));
                }
                if cat == packs::CUSTOM {
                    if !self.confirm_delete {
                        if ui.button(lang.tr("Delete")).clicked() {
                            self.confirm_delete = true;
                        }
                    } else {
                        ui.colored_label(crate::theme::warn(ui), lang.tr_fmt("Are you sure you want to delete the custom PACKS Kit {0}?", &[&name]));
                        if ui.button(lang.tr("Yes")).clicked() {
                            self.confirm_delete = false;
                            match dir.map(|d| packs::delete(d, &name)) {
                                Some(Ok(true)) => {
                                    self.selected = None;
                                    self.doc = None;
                                }
                                Some(Err(e)) => self.message = Some((e.to_string(), true)),
                                _ => self.message = Some((lang.tr("The kit is not in a custom PACKS file."), true)),
                            }
                        }
                        if ui.button(lang.tr("No")).clicked() {
                            self.confirm_delete = false;
                        }
                    }
                }
            });
        });
        changed
    }

    fn create_ui(&mut self, ui: &mut egui::Ui, ch: &Character, sheet: &Sheet, settings: Option<&CharacterSettings>, dir: Option<&std::path::Path>, lang: &Language) {
        let parts = self.parts.get_or_insert_with(KitParts::default);
        egui::Grid::new("packs_create").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(lang.tr("Kit Name:"));
            ui.text_edit_singleline(&mut self.kit_name);
            ui.end_row();
            ui.label(lang.tr("File Name:"));
            ui.add(egui::TextEdit::singleline(&mut self.file_name).hint_text("custom_mykits_packs.xml"));
            ui.end_row();
        });
        ui.weak(dir.map_or_else(|| lang.tr("No PACKS folder."), |d| d.display().to_string()));
        ui.separator();
        egui::Grid::new("packs_parts").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
            let boxes: [(&mut bool, &str); 12] = [
                (&mut parts.attributes, "Attributes"),
                (&mut parts.qualities, "Qualities"),
                (&mut parts.starting_nuyen, "Starting Nuyen"),
                (&mut parts.martial_arts, "Martial Arts"),
                (&mut parts.spells, "Spells"),
                (&mut parts.complex_forms, "Complex Forms"),
                (&mut parts.cyberware, "Cyberware/Bioware"),
                (&mut parts.lifestyles, "Lifestyles"),
                (&mut parts.armor, "Armor"),
                (&mut parts.weapons, "Weapons"),
                (&mut parts.gear, "Gear"),
                (&mut parts.vehicles, "Vehicles"),
            ];
            for (i, (b, label)) in boxes.into_iter().enumerate() {
                ui.checkbox(b, lang.tr(label));
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        if ui.add(crate::theme::primary_button(ui, lang.tr("Create PACKS Kit"))).clicked() {
            let Some(dir) = dir else {
                self.message = Some((lang.tr("No PACKS folder."), true));
                return;
            };
            let kit = packs::from_character(ch, sheet, settings, self.kit_name.trim(), *parts);
            let merged = self.doc.clone().unwrap_or_else(|| Element::new("chummer"));
            self.message = Some(match packs::save(dir, &self.file_name, &kit, &merged) {
                Ok(p) => {
                    self.doc = None;
                    (format!("{} {}", lang.tr_fmt("PACKS Kit \"{0}\" created.", &[&self.kit_name.trim()]), p.display()), false)
                }
                Err(e) => (lang.tr(&e.to_string()), true),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Custom spells
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct SpellDesigner {
    pub open: bool,
    design: Option<SpellDesign>,
    message: Option<String>,
}

impl SpellDesigner {
    /// Draw the designer when open. Returns true if a spell was added.
    #[allow(clippy::too_many_arguments)]
    pub fn window(&mut self, ctx: &egui::Context, ch: &mut Character, engine: &Engine, store: &DataStore, sheet: &Sheet, lang: &Language, status: &mut Status) -> bool {
        if !self.open {
            return false;
        }
        let categories = store.doc("spells.xml").map(|d| data::categories(&d)).unwrap_or_default();
        let d = self.design.get_or_insert_with(SpellDesign::default);
        let mut add = false;
        let mut open = true;
        egui::Window::new(lang.tr("Create Spell")).id(egui::Id::new("create_spell")).open(&mut open).default_width(560.0).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("spell_design").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(lang.tr("Name"));
                ui.text_edit_singleline(&mut d.name);
                ui.end_row();
                ui.label(lang.tr("Category"));
                let mut cat = d.category.clone();
                crate::combo::Combo::from_id_salt("spell_cat").selected_text(lang.tr(&cat)).show_ui(ui, |ui| {
                    for c in &categories {
                        crate::combo::selectable_value(ui, &mut cat, c.clone(), lang.tr(c));
                    }
                });
                if cat != d.category {
                    custom_spell::set_category(d, &cat);
                }
                ui.end_row();
                ui.label(lang.tr("Type"));
                ui.add_enabled_ui(!d.kind_locked, |ui| combo(ui, "spell_type", &mut d.kind, custom_spell::TYPES, lang));
                ui.end_row();
                ui.label(lang.tr("Range"));
                ui.horizontal(|ui| {
                    combo(ui, "spell_range", &mut d.range, custom_spell::RANGES, lang);
                    let allowed = custom_spell::area_allowed(d);
                    ui.add_enabled(allowed, egui::Checkbox::new(&mut d.area, lang.tr("Area")));
                });
                ui.end_row();
                ui.label(lang.tr("Duration"));
                combo(ui, "spell_duration", &mut d.duration, custom_spell::DURATIONS, lang);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.restricted, lang.tr("Restricted Target"));
                ui.checkbox(&mut d.very_restricted, lang.tr("Very Restricted Target"));
                ui.checkbox(&mut d.limited, lang.tr("Limited"));
            });
            if d.restricted || d.very_restricted {
                ui.horizontal(|ui| {
                    ui.label(lang.tr("Restriction"));
                    ui.text_edit_singleline(&mut d.restriction);
                });
            } else {
                d.restriction.clear();
            }
            ui.separator();
            let table = custom_spell::modifier_table(&d.category);
            let multiplied = custom_spell::effects_slot(&d.category);
            for (i, (label, dv)) in table.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut on = d.mods[i];
                    let text = format!("{} ({}{dv})", lang.tr(label), if *dv >= 0 { "+" } else { "" });
                    if ui.add_enabled(!d.disabled[i], egui::Checkbox::new(&mut on, text)).changed() {
                        custom_spell::set_modifier(d, i, on);
                    }
                    if Some(i) == multiplied {
                        let enabled = custom_spell::effects_enabled(d);
                        ui.add_enabled(enabled, egui::DragValue::new(&mut d.effects).range(1..=20));
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(lang.tr("DV:"));
                ui.strong(custom_spell::drain(d).replace('/', "÷"));
                let desc = custom_spell::descriptors(d);
                if !desc.is_empty() {
                    ui.weak(desc);
                }
            });
            for p in custom_spell::problems(d) {
                ui.colored_label(crate::theme::warn(ui), lang.tr(p));
            }
            if let Some(m) = &self.message {
                ui.colored_label(ui.visuals().error_fg_color, m);
            }
            ui.horizontal(|ui| {
                let label = if ch.created { lang.tr_fmt("Create Spell ({0} karma)", &[&chummer_core::career::spell_karma_cost(engine, ch, "Spells")]) } else { lang.tr("Create Spell") };
                if ui.add_enabled(custom_spell::problems(d).is_empty(), crate::theme::primary_button(ui, label)).clicked() {
                    add = true;
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    self.open = false;
                }
            });
        });
        if !open {
            self.open = false;
        }
        if !add {
            return false;
        }
        let d = self.design.clone().unwrap_or_default();
        if !ch.created && custom_spell::creation_limit_reached(ch, sheet) {
            self.message = Some(lang.tr("You cannot have more spells, rituals or alchemical preparations than twice your MAG score. Ref: Page 69, SR5 Core."));
            return false;
        }
        match custom_spell::add(ch, engine, &d) {
            Ok(_) => {
                *status = Some((lang.tr_fmt("Created {0}", &[&d.name]), false));
                self.design = None;
                self.message = None;
                self.open = false;
                true
            }
            Err(e) => {
                self.message = Some(e.to_string());
                false
            }
        }
    }
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)], lang: &Language) {
    let shown = options.iter().find(|(v, _)| v == value).map_or_else(|| value.clone(), |(_, l)| lang.tr(l));
    crate::combo::Combo::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
        for (v, l) in options {
            crate::combo::selectable_value(ui, value, (*v).to_owned(), lang.tr(l));
        }
    });
}
