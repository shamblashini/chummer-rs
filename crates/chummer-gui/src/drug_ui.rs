//! The custom drug builder (`CreateCustomDrug`): one foundation plus
//! blocks and enhancers at chosen levels, with the combined effects and
//! cost previewed before the drug is added.

use chummer_core::career;
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore, Record};
use chummer_core::format;
use chummer_core::items::{self, drug};
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;

const FILE: &str = "drugcomponents.xml";
const CATEGORIES: &[&str] = &["Foundation", "Block", "Enhancer"];

pub struct DrugBuilder {
    pub open: bool,
    name: String,
    grade: String,
    /// Chosen (component name, level).
    chosen: Vec<(String, i32)>,
    /// Component highlighted in the list and the level to add it at.
    pick: Option<(String, i32)>,
    message: Option<String>,
}

impl Default for DrugBuilder {
    fn default() -> Self {
        DrugBuilder { open: false, name: String::new(), grade: "Standard".into(), chosen: Vec::new(), pick: None, message: None }
    }
}

impl DrugBuilder {
    /// Draw the builder window when open. Returns true if a drug was added
    /// or removed.
    pub fn window(&mut self, ctx: &egui::Context, ch: &mut Character, store: &DataStore, lang: &Language, status: &mut Status) -> bool {
        if !self.open {
            return false;
        }
        let Ok(doc) = store.doc(FILE) else {
            self.open = false;
            return false;
        };
        let mut changed = false;
        let mut open = true;
        let mut add = false;
        egui::Window::new(lang.tr("Build custom drug")).id(egui::Id::new("drug_builder")).open(&mut open).default_size([860.0, 560.0]).collapsible(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(lang.tr("Name"));
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(lang.tr("Custom drug")).desired_width(220.0));
                ui.label(lang.tr("Grade"));
                let grades = data::records(&doc, "grades", "grade");
                crate::combo::Combo::from_id_salt("drug_grade").selected_text(self.grade.clone()).show_ui(ui, |ui| {
                    for g in &grades {
                        crate::combo::selectable_value(ui, &mut self.grade, g.name(), format!("{} ({} ×{})", g.name(), lang.tr("cost"), g.get("cost")));
                    }
                });
            });
            ui.separator();
            ui.columns(2, |cols| {
                self.components_list(&mut cols[0], &doc, lang);
                self.chosen_panel(&mut cols[1], &doc, store, lang, &mut add);
            });
            changed |= existing_drugs(ui, ch, lang);
        });
        if !open {
            self.open = false;
        }
        if add {
            changed |= self.add(ch, store, status);
        }
        changed
    }

    /// Available components by category, with a level picker.
    fn components_list(&mut self, ui: &mut egui::Ui, doc: &Element, lang: &Language) {
        let comps = data::records(doc, "drugcomponents", "drugcomponent");
        egui::ScrollArea::vertical().id_salt("drug_components").max_height(330.0).show(ui, |ui| {
            for cat in CATEGORIES {
                egui::CollapsingHeader::new(RichText::new(lang.data_name(FILE, "", cat)).strong()).id_salt(("drug_cat", *cat)).default_open(true).show(ui, |ui| {
                    for r in comps.iter().filter(|r| r.category() == *cat && !r.hidden()) {
                        let name = r.name();
                        let sel = self.pick.as_ref().is_some_and(|(n, _)| *n == name);
                        if crate::combo::selectable_label(ui, sel, format!("{name}   {}¥", r.get("cost"))).clicked() && !sel {
                            let first = drug::component_levels(*r).first().copied().unwrap_or(0);
                            self.pick = Some((name, first));
                            self.message = None;
                        }
                    }
                });
            }
        });
        let Some((name, mut level)) = self.pick.clone() else {
            ui.weak(lang.tr("Select a component."));
            return;
        };
        let Some(rec) = data::find(doc, "drugcomponents", "drugcomponent", &name) else { return };
        let levels = drug::component_levels(rec);
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(rec.name());
            if levels.len() > 1 {
                ui.label(lang.tr("Level"));
                crate::combo::Combo::from_id_salt("drug_level").selected_text((index_of(&levels, level) + 1).to_string()).show_ui(ui, |ui| {
                    for (i, l) in levels.iter().enumerate() {
                        crate::combo::selectable_value(ui, &mut level, *l, (i + 1).to_string());
                    }
                });
            }
            if ui.button(lang.tr("Add component")).clicked() {
                self.message = self.check_component(doc, rec, &name, level).err();
                if self.message.is_none() {
                    self.chosen.push((name.clone(), level));
                }
            }
        });
        self.pick = Some((name, level));
        let probe = component_effect_text(rec, level, lang);
        if !probe.is_empty() {
            ui.weak(probe);
        }
        if let Some(m) = &self.message {
            ui.colored_label(ui.visuals().warn_fg_color, m);
        }
    }

    /// `CreateCustomDrug.AddSelectedComponent`'s checks.
    fn check_component(&self, doc: &Element, rec: Record<'_>, name: &str, level: i32) -> Result<(), String> {
        let limit = rec.el().get_i32("limit").unwrap_or(1);
        let count = self.chosen.iter().filter(|(n, _)| n == name).count() as i32;
        if limit != 0 && count >= limit {
            return Err(format!("{name} can be added at most {limit} time{}.", if limit == 1 { "" } else { "s" }));
        }
        if rec.category() == "Foundation" && self.chosen.iter().any(|(n, _)| category(doc, n) == "Foundation") {
            return Err("A drug can have only one foundation.".into());
        }
        // A block above level 2 cannot raise what the foundation lowers (CF 191).
        let index = index_of(&drug::component_levels(rec), level);
        if index + 1 > 2 {
            let block = effect_attributes(rec, level);
            for (n, _) in self.chosen.iter().filter(|(n, _)| category(doc, n) == "Foundation") {
                let Some(f) = data::find(doc, "drugcomponents", "drugcomponent", n) else { continue };
                let first = drug::component_levels(f).first().copied().unwrap_or(0);
                for (attr, v) in effect_attributes(f, first) {
                    if v < 0.0 && block.iter().any(|(a, b)| *a == attr && *b > 0.0) {
                        return Err(format!("{}: {attr} {v} cannot be raised by {} at this level.", f.name(), rec.name()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Chosen components, effects and cost preview, and the Add button.
    fn chosen_panel(&mut self, ui: &mut egui::Ui, doc: &Element, store: &DataStore, lang: &Language, add: &mut bool) {
        ui.strong(lang.tr("Components"));
        let mut remove = None;
        egui::Grid::new("drug_chosen").striped(true).num_columns(4).show(ui, |ui| {
            for (i, (n, l)) in self.chosen.iter().enumerate() {
                let levels = data::find(doc, "drugcomponents", "drugcomponent", n).map(drug::component_levels).unwrap_or_default();
                ui.label(n);
                ui.weak(lang.data_name(FILE, "", &category(doc, n)));
                ui.label(if levels.len() > 1 { lang.tr_fmt("level {0}", &[&(index_of(&levels, *l) + 1)]) } else { String::new() });
                if ui.small_button("🗑").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            self.chosen.remove(i);
        }
        if self.chosen.is_empty() {
            ui.weak(lang.tr("Add a foundation, then blocks and enhancers."));
        }
        ui.separator();
        match self.build(store) {
            Ok(d) => {
                let fx = drug::effects(&d);
                ui.strong(lang.tr("Effects"));
                for line in effect_lines(&fx, lang) {
                    ui.label(line);
                }
                ui.label(format!("{} {}", lang.tr("Cost per dose:"), format::nuyen(drug::cost(&d))));
                ui.add_space(6.0);
                if ui.add(crate::theme::primary_button(ui, lang.tr("Add drug"))).clicked() {
                    *add = true;
                }
            }
            Err(e) => {
                ui.weak(e);
            }
        }
    }

    fn build(&self, store: &DataStore) -> Result<Element, String> {
        let comps: Vec<(&str, i32)> = self.chosen.iter().map(|(n, l)| (n.as_str(), *l)).collect();
        let name = if self.name.trim().is_empty() { "Custom Drug" } else { self.name.trim() };
        drug::custom_drug(store, name, &self.grade, &comps, &items::new_guid())
    }

    fn add(&mut self, ch: &mut Character, store: &DataStore, status: &mut Status) -> bool {
        let d = match self.build(store) {
            Ok(d) => d,
            Err(e) => {
                *status = Some((e, true));
                return false;
            }
        };
        let guid = d.get("guid");
        let name = d.get("name");
        let cost = drug::cost(&d);
        drug::add_element(ch, d);
        if ch.created && cost > 0.0 {
            if let Err(e) = career::pay_for_item(ch, "drug", None, &guid, cost) {
                ch.improvements.remove_from_source(&guid);
                ch.remove_item("drugs", &guid);
                *status = Some((e.to_string(), true));
                return true;
            }
        }
        *status = Some((format!("Added {name} ({} per dose)", format::nuyen(cost)), false));
        self.chosen.clear();
        self.name.clear();
        self.pick = None;
        true
    }
}

/// The character's drugs (no other tab lists them), with removal.
fn existing_drugs(ui: &mut egui::Ui, ch: &mut Character, lang: &Language) -> bool {
    let drugs: Vec<Element> = ch.items("drugs", "drug").into_iter().cloned().collect();
    if drugs.is_empty() {
        return false;
    }
    let mut changed = false;
    ui.separator();
    egui::CollapsingHeader::new(RichText::new(format!("{} ({})", lang.tr("Your drugs"), drugs.len())).strong()).id_salt("drugs_owned").default_open(true).show(ui, |ui| {
        egui::Grid::new("drugs_owned_grid").striped(true).num_columns(4).spacing([14.0, 3.0]).show(ui, |ui| {
            for d in &drugs {
                ui.label(d.get("name"));
                ui.weak(d.get("grade"));
                ui.label(format!("×{}", d.get("quantity")));
                if ui.small_button("🗑").on_hover_text(lang.tr("Remove (no refund)")).clicked() {
                    let g = d.get("guid");
                    ch.improvements.remove_from_source(&g);
                    changed |= ch.remove_item("drugs", &g);
                }
                ui.end_row();
            }
        });
    });
    changed
}

fn category(doc: &Element, name: &str) -> String {
    data::find(doc, "drugcomponents", "drugcomponent", name).map(|r| r.category()).unwrap_or_default()
}

fn index_of(levels: &[i32], level: i32) -> usize {
    levels.iter().position(|l| *l == level).unwrap_or(0)
}

fn effect_at<'a>(rec: Record<'a>, level: i32) -> Option<&'a Element> {
    rec.el().child("effects")?.children_named("effect").find(|e| e.get_i32("level").unwrap_or(0) == level)
}

fn effect_attributes(rec: Record<'_>, level: i32) -> Vec<(String, f64)> {
    effect_at(rec, level).map(|e| e.children_named("attribute").map(|a| (a.get("name"), a.get_f64("value").unwrap_or(0.0))).collect()).unwrap_or_default()
}

/// One component's effect at a level, as a single line.
fn component_effect_text(rec: Record<'_>, level: i32, lang: &Language) -> String {
    let Some(e) = effect_at(rec, level) else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    for tag in ["attribute", "limit"] {
        for a in e.children_named(tag) {
            parts.push(format!("{} {}", a.get("name"), signed(a.get_f64("value").unwrap_or(0.0))));
        }
    }
    for q in e.children_named("quality") {
        parts.push(q.text());
    }
    let labels = lang.tr_all(["Initiative", "Initiative Dice", "Duration", "Speed", "Crash damage"]);
    for (k, label) in ["initiative", "initiativedice", "duration", "speed", "crashdamage"].into_iter().zip(labels) {
        if let Some(v) = e.get_i32(k).filter(|v| *v != 0) {
            parts.push(format!("{label} {}", signed(f64::from(v))));
        }
    }
    for i in e.children_named("info") {
        parts.push(i.text());
    }
    parts.join(", ")
}

fn signed(v: f64) -> String {
    if v > 0.0 {
        format!("+{v}")
    } else {
        format!("{v}")
    }
}

fn effect_lines(fx: &drug::Effects, lang: &Language) -> Vec<String> {
    let mut out = Vec::new();
    if !fx.attributes.is_empty() {
        out.push(fx.attributes.iter().map(|(a, v)| format!("{a} {}", signed(*v))).collect::<Vec<_>>().join(", "));
    }
    if !fx.limits.is_empty() {
        out.push(format!("{} {}", lang.tr("Limits:"), fx.limits.iter().map(|(a, v)| format!("{a} {}", signed(f64::from(*v)))).collect::<Vec<_>>().join(", ")));
    }
    if fx.initiative != 0 || fx.initiative_dice != 0 {
        out.push(format!("{} {} / +{}d6", lang.tr("Initiative"), signed(f64::from(fx.initiative)), fx.initiative_dice));
    }
    if !fx.qualities.is_empty() {
        out.push(format!("{} {}", lang.tr("Qualities:"), fx.qualities.join(", ")));
    }
    out.push(format!("{} {} · {} {}", lang.tr("Speed"), fx.speed, lang.tr("Crash damage"), fx.crash_damage));
    out
}
