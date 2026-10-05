//! Generic "pick a record from the game data" dialog for every item kind,
//! with the purchase options (rating, quantity, grade, parent) and the
//! follow-up prompt for bonus selections.

use std::sync::Arc;

use chummer_core::bonus::Choice;
use chummer_core::calc::Sheet;
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore, Record};
use chummer_core::expr::{self, Availability};
use chummer_core::items::{self, Kind, Purchase};
use chummer_core::lang::Language;
use chummer_core::requirements::{self, Check};
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::browser::record_fields;
use crate::pdf_ui::{self, Status};

/// Extra list columns per kind: (header, data field).
fn columns(tag: &str) -> &'static [(&'static str, &'static str)] {
    match tag {
        "quality" => &[("Karma", "karma"), ("Type", "category")],
        "gear" => &[("Rating", "rating"), ("Avail", "avail"), ("Cost", "cost")],
        "cyberware" | "bioware" => &[("Ess", "ess"), ("Avail", "avail"), ("Cost", "cost")],
        "armor" | "armormod" => &[("Armor", "armor"), ("Avail", "avail"), ("Cost", "cost")],
        "weapon" => &[("DV", "damage"), ("AP", "ap"), ("Avail", "avail"), ("Cost", "cost")],
        "accessory" | "mod" => &[("Avail", "avail"), ("Cost", "cost")],
        "vehicle" => &[("Handling", "handling"), ("Speed", "speed"), ("Avail", "avail"), ("Cost", "cost")],
        "spell" => &[("Type", "type"), ("Range", "range"), ("DV", "dv")],
        "power" => &[("PP", "points")],
        "complexform" => &[("Target", "target"), ("FV", "fv")],
        "aiprogram" => &[("Category", "category")],
        "lifestyle" => &[("Cost", "cost")],
        _ => &[],
    }
}

/// Kinds that must go inside another item: (parent container, parent tag).
pub fn parent_of(tag: &str) -> Option<(&'static str, &'static str)> {
    match tag {
        "armormod" => Some(("armors", "armor")),
        "accessory" => Some(("weapons", "weapon")),
        "mod" => Some(("vehicles", "vehicle")),
        _ => None,
    }
}

/// Kinds that may optionally go inside another item.
fn optional_parent(tag: &str) -> Option<(&'static str, &'static str)> {
    match tag {
        "gear" => Some(("gears", "gear")),
        "cyberware" | "bioware" => Some(("cyberwares", "cyberware")),
        _ => None,
    }
}

enum Step {
    Pick,
    Answer { index: usize, choices: Vec<Choice>, answer: String },
}

/// The dialog's result for this frame.
pub enum Outcome {
    None,
    Cancel,
    Done { index: usize, purchase: Purchase },
}

pub struct SelectDialog {
    pub kind: Kind,
    doc: Arc<Element>,
    search: String,
    category: String,
    show_unavailable: bool,
    selected: Option<usize>,
    step: Step,
    books: Vec<String>,
    purchase: Purchase,
    max_avail: i32,
    nuyen_left: Option<f64>,
    /// The parent was preset by [`SelectDialog::with_parent`]: no picker.
    parent_locked: bool,
}

impl SelectDialog {
    /// `nuyen_left` is shown as a budget check while creating.
    pub fn new(tag: &str, store: &DataStore, books: Vec<String>, max_avail: i32, nuyen_left: Option<f64>) -> Option<Self> {
        let kind = *items::kind(tag)?;
        let doc = store.doc(kind.file).ok()?;
        Some(SelectDialog {
            kind,
            doc,
            search: String::new(),
            category: String::new(),
            show_unavailable: false,
            selected: None,
            step: Step::Pick,
            books,
            purchase: Purchase { qty: 1.0, cost_multiplier: 1.0, ..Default::default() },
            max_avail,
            nuyen_left,
            parent_locked: false,
        })
    }

    /// Preset where the new item goes (an item editor's "Add …" command).
    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent_locked = parent.is_some();
        self.purchase.parent = parent;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ch: &Character,
        sheet: &Sheet,
        lang: &Language,
        pdfs: &SourcebookLibrary,
        status: &mut Status,
        choices_for: &dyn Fn(&Element, &Purchase) -> Vec<Choice>,
    ) -> Outcome {
        let mut result = Outcome::None;
        let mut open = true;
        let doc = self.doc.clone();
        let recs = data::records(&doc, self.kind.data_container, self.kind.data_item);
        let title = lang.tr_fmt("Add {0}", &[&crate::view::kind_noun(lang, self.kind.label)]);
        egui::Window::new(title).id(egui::Id::new(("select_dialog", self.kind.tag))).open(&mut open).default_size([960.0, 640.0]).collapsible(false).show(ctx, |ui| {
            if let Step::Answer { index, choices, answer } = &mut self.step {
                let index = *index;
                let c = &choices[0];
                ui.heading(recs[index].name());
                ui.label(&c.prompt);
                if c.options.is_empty() {
                    ui.text_edit_singleline(answer);
                } else {
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        for o in &c.options {
                            if crate::combo::selectable_label(ui, answer == o, o).clicked() {
                                *answer = o.clone();
                            }
                        }
                    });
                }
                let mut back = false;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!answer.trim().is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                        let mut p = self.purchase.clone();
                        p.answer = Some(answer.trim().to_owned());
                        result = Outcome::Done { index, purchase: p };
                    }
                    back = ui.button(lang.tr("Back")).clicked();
                });
                if back {
                    self.step = Step::Pick;
                }
                return;
            }

            let cats = data::categories(&doc);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(lang.tr("Search")).desired_width(220.0));
                if !cats.is_empty() {
                    crate::combo::Combo::from_id_salt("sel_cat")
                        .selected_text(if self.category.is_empty() { lang.tr("All categories") } else { lang.data_name(self.kind.file, "", &self.category) })
                        .show_ui(ui, |ui| {
                            crate::combo::selectable_value(ui, &mut self.category, String::new(), lang.tr("All categories"));
                            for c in &cats {
                                crate::combo::selectable_value(ui, &mut self.category, c.clone(), lang.data_name(self.kind.file, "", c));
                            }
                        });
                }
                ui.checkbox(&mut self.show_unavailable, lang.tr("Show unavailable"));
            });
            let check = Check { ch, sheet, ignore_quality: None };
            let needle = self.search.to_lowercase();
            let cols = columns(self.kind.tag);
            let rows: Vec<(usize, String, Vec<String>)> = recs
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.hidden())
                .filter(|(_, r)| self.books.is_empty() || r.source().is_empty() || self.books.contains(&r.source()))
                .filter(|(_, r)| self.category.is_empty() || r.category() == self.category)
                .filter_map(|(i, r)| {
                    let name = lang.data_name(self.kind.file, &r.id(), &r.name());
                    if !needle.is_empty() && !name.to_lowercase().contains(&needle) && !r.name().to_lowercase().contains(&needle) {
                        return None;
                    }
                    let mut why = requirements::unmet(r.el(), &check);
                    if self.kind.tag == "quality" && requirements::remaining_quality_slots(r.el(), ch) == Some(0) {
                        why.push("already taken".into());
                    }
                    if !ch.created && self.max_avail > 0 {
                        let a = Availability::parse(&r.get("avail"), rating_default(*r), 0, &expr::NoAttributes);
                        if !a.add_to_parent && a.value > self.max_avail {
                            why.push(format!("availability {a} is above {}", self.max_avail));
                        }
                    }
                    (self.show_unavailable || why.is_empty()).then_some((i, name, why))
                })
                .collect();
            ui.weak(lang.tr_fmt("{0} shown", &[&rows.len()]));
            ui.separator();
            let mut confirm: Option<usize> = None;
            ui.columns(2, |colsui| {
                egui::ScrollArea::vertical().id_salt("sel_list").auto_shrink([false; 2]).max_height(470.0).show_rows(&mut colsui[0], 20.0, rows.len(), |ui, range| {
                    for (i, name, why) in &rows[range] {
                        let r = recs[*i];
                        let extra: Vec<String> = cols.iter().map(|(_, f)| r.get(f)).filter(|v| !v.is_empty()).collect();
                        let mut text = RichText::new(format!("{name}   {}", extra.join(" · ")));
                        if !why.is_empty() {
                            text = text.weak();
                        }
                        let resp = crate::combo::selectable_label(ui, self.selected == Some(*i), text);
                        if resp.clicked() && self.selected != Some(*i) {
                            self.selected = Some(*i);
                            self.purchase.rating = rating_default(r);
                        }
                        if resp.double_clicked() && why.is_empty() {
                            self.selected = Some(*i);
                            confirm = Some(*i);
                        }
                    }
                });
                let ui = &mut colsui[1];
                egui::ScrollArea::vertical().id_salt("sel_detail").max_height(470.0).show(ui, |ui| match self.selected {
                    Some(i) => {
                        let r = recs[i];
                        ui.heading(lang.data_name(self.kind.file, &r.id(), &r.name()));
                        pdf_ui::source_link(ui, pdfs, lang, SourceRef::of(r.el()), status);
                        for w in requirements::unmet(r.el(), &check) {
                            ui.colored_label(ui.visuals().warn_fg_color, w);
                        }
                        self.purchase_options(ui, ch, lang, r);
                        ui.separator();
                        record_fields(ui, r.el(), 0);
                    }
                    None => {
                        ui.weak(lang.tr("Select an entry."));
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let needs_parent = parent_of(self.kind.tag).is_some() && self.purchase.parent.is_none();
                let can = !needs_parent && self.selected.is_some_and(|i| rows.iter().any(|(j, _, w)| *j == i && w.is_empty()));
                let add = ui.add_enabled(can, egui::Button::new(lang.tr("Add")));
                let add = if needs_parent { add.on_disabled_hover_text(lang.tr("Choose where to install it")) } else { add };
                if add.clicked() {
                    confirm = self.selected;
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    result = Outcome::Cancel;
                }
            });
            if let Some(i) = confirm {
                result = self.confirm(i, recs[i].el(), choices_for);
            }
        });
        if !open {
            return Outcome::Cancel;
        }
        result
    }

    /// Rating, quantity, grade and parent pickers plus a cost preview.
    fn purchase_options(&mut self, ui: &mut egui::Ui, ch: &Character, lang: &Language, r: Record<'_>) {
        let max_rating = rating_max(r);
        egui::Grid::new("purchase").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if max_rating > 0 {
                let min = r.el().get_i32("minrating").unwrap_or(1).clamp(0, max_rating);
                ui.label(r.el().child_text("ratinglabel").map(|l| if lang.has(&l) { lang.s(&l) } else { l }).unwrap_or_else(|| lang.tr("Rating")));
                self.purchase.rating = self.purchase.rating.clamp(min, max_rating);
                ui.add(egui::DragValue::new(&mut self.purchase.rating).range(min..=max_rating));
                ui.end_row();
            }
            if matches!(self.kind.tag, "gear" | "drug") {
                ui.label(lang.tr("Quantity"));
                ui.add(egui::DragValue::new(&mut self.purchase.qty).range(1.0..=1000.0).max_decimals(0));
                ui.end_row();
            }
            if matches!(self.kind.tag, "cyberware" | "bioware") {
                ui.label(lang.tr("Grade"));
                let grades = grades(&self.doc);
                let cur = self.purchase.grade.clone().unwrap_or_else(|| "Standard".into());
                crate::combo::Combo::from_id_salt("grade").selected_text(cur.clone()).show_ui(ui, |ui| {
                    for (g, mult) in &grades {
                        if crate::combo::selectable_label(ui, cur == *g, format!("{g} ({} ×{mult})", lang.tr("ess"))).clicked() {
                            self.purchase.grade = Some(g.clone());
                        }
                    }
                });
                ui.end_row();
            }
            if self.parent_locked {
                let name = self.purchase.parent.as_deref().and_then(|g| items::edit::find(ch, g)).map(|e| e.get("name")).unwrap_or_default();
                ui.label(lang.tr("Install in"));
                ui.label(name);
                ui.end_row();
            } else if let Some((container, tag)) = parent_of(self.kind.tag).or(optional_parent(self.kind.tag)) {
                ui.label(lang.tr("Install in"));
                let parents: Vec<(String, String)> = ch.items(container, tag).iter().map(|e| (e.get("guid"), e.get("name"))).collect();
                let cur = self.purchase.parent.as_ref().and_then(|g| parents.iter().find(|(pg, _)| pg == g)).map(|(_, n)| n.clone());
                let required = parent_of(self.kind.tag).is_some();
                let none_label = if required { lang.tr("Choose…") } else { lang.tr("Nothing (on its own)") };
                crate::combo::Combo::from_id_salt("parent").selected_text(cur.unwrap_or(none_label)).show_ui(ui, |ui| {
                    if !required && crate::combo::selectable_label(ui, self.purchase.parent.is_none(), lang.tr("Nothing (on its own)")).clicked() {
                        self.purchase.parent = None;
                    }
                    for (g, n) in &parents {
                        if crate::combo::selectable_label(ui, self.purchase.parent.as_deref() == Some(g), n).clicked() {
                            self.purchase.parent = Some(g.clone());
                        }
                    }
                });
                ui.end_row();
            }
            if !r.get("avail").is_empty() {
                let avail = Availability::parse(&r.get("avail"), self.purchase.rating, r.el().get_i32("minrating").unwrap_or(0), &expr::NoAttributes);
                ui.label(lang.tr("Availability"));
                ui.label(avail.to_string());
                ui.end_row();
            }
            if let Some(cost) = preview_cost(r, &self.purchase) {
                ui.label(lang.tr("Cost"));
                let text = chummer_core::format::nuyen(cost);
                match self.nuyen_left {
                    Some(left) if cost > left => ui.colored_label(ui.visuals().error_fg_color, lang.tr_fmt("{0} (only {1} left)", &[&text, &chummer_core::format::nuyen(left)])),
                    _ => ui.label(text),
                };
                ui.end_row();
            }
        });
    }

    fn confirm(&mut self, index: usize, rec: &Element, choices_for: &dyn Fn(&Element, &Purchase) -> Vec<Choice>) -> Outcome {
        let choices = choices_for(rec, &self.purchase);
        if choices.is_empty() {
            Outcome::Done { index, purchase: self.purchase.clone() }
        } else {
            let answer = if choices[0].options.len() == 1 { choices[0].options[0].clone() } else { String::new() };
            self.step = Step::Answer { index, choices, answer };
            Outcome::None
        }
    }

    pub fn record(&self, store: &DataStore, index: usize) -> Option<Element> {
        let doc = store.doc(self.kind.file).ok()?;
        data::records(&doc, self.kind.data_container, self.kind.data_item).get(index).map(|r| r.el().clone())
    }
}

/// The highest rating a record allows (`<rating>`), 0 when it has none.
fn rating_max(r: Record<'_>) -> i32 {
    let t = r.get("rating");
    if t.trim().is_empty() {
        return 0;
    }
    expr::parse_plain(&t).map(|v| v as i32).unwrap_or(6)
}

fn rating_default(r: Record<'_>) -> i32 {
    let max = rating_max(r);
    if max == 0 {
        0
    } else {
        r.el().get_i32("minrating").unwrap_or(1).clamp(0, max).max(1)
    }
}

/// `<grades>` of cyberware.xml/bioware.xml as (name, essence multiplier).
fn grades(doc: &Element) -> Vec<(String, String)> {
    doc.child("grades")
        .map(|g| g.children_named("grade").filter(|e| e.child("hide").is_none() && e.get("name") != "None").map(|e| (e.get("name"), e.get("ess"))).collect())
        .unwrap_or_default()
}

/// Cost at the chosen rating and quantity, when it is a plain expression.
fn preview_cost(r: Record<'_>, p: &Purchase) -> Option<f64> {
    let raw = r.get("cost");
    if raw.trim().is_empty() || raw.contains("Variable") || raw.contains("Parent") || raw.contains("Gear") {
        return None;
    }
    let min = r.el().get_i32("minrating").unwrap_or(0).to_string();
    let s = expr::fixed_values(raw.trim(), p.rating).replace("MinRating", &min).replace("Rating", &p.rating.to_string());
    let v = if expr::needs_evaluation(&s) { expr::evaluate_num(&s).ok()? } else { expr::parse_plain(&s)? };
    Some(v * p.qty() * if p.cost_multiplier > 0.0 { p.cost_multiplier } else { 1.0 })
}
