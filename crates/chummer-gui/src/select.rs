//! Generic "pick a record from the game data" dialog, plus the follow-up
//! prompt for bonus selections.

use std::sync::Arc;

use chummer_core::bonus::Choice;
use chummer_core::calc::Sheet;
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore};
use chummer_core::lang::Language;
use chummer_core::requirements::{self, Check};
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::browser::record_fields;
use crate::pdf_ui::{self, Status};

/// What the dialog lists.
#[derive(Debug, Clone)]
pub struct Kind {
    pub title: &'static str,
    pub file: &'static str,
    pub container: &'static str,
    pub item: &'static str,
    /// Extra columns shown in the list: (header, field).
    pub columns: &'static [(&'static str, &'static str)],
}

pub const QUALITY: Kind = Kind {
    title: "Add quality",
    file: "qualities.xml",
    container: "qualities",
    item: "quality",
    columns: &[("Karma", "karma"), ("Type", "category")],
};

pub enum Step {
    Pick,
    Answer { index: usize, choices: Vec<Choice>, answer: String },
}

/// The dialog's result for this frame.
pub enum Outcome {
    None,
    Cancel,
    /// Chosen record index and the answer to its selection, if any.
    Done { index: usize, answer: Option<String> },
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
}

impl SelectDialog {
    pub fn new(kind: Kind, store: &DataStore, books: Vec<String>) -> Option<Self> {
        let doc = store.doc(kind.file).ok()?;
        Some(SelectDialog { kind, doc, search: String::new(), category: String::new(), show_unavailable: false, selected: None, step: Step::Pick, books })
    }

    /// `choices_for(index)` computes the bonus selections for a record.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ch: &Character,
        sheet: &Sheet,
        lang: &Language,
        pdfs: &SourcebookLibrary,
        status: &mut Status,
        choices_for: &dyn Fn(&Element) -> Vec<Choice>,
    ) -> Outcome {
        let mut result = Outcome::None;
        let mut open = true;
        let doc = self.doc.clone();
        let recs = data::records(&doc, self.kind.container, self.kind.item);
        egui::Window::new(self.kind.title).open(&mut open).default_size([900.0, 620.0]).collapsible(false).show(ctx, |ui| {
            match &mut self.step {
                Step::Answer { index, choices, answer } => {
                    let index = *index;
                    let c = &choices[0];
                    ui.heading(recs[index].name());
                    ui.label(&c.prompt);
                    if c.options.is_empty() {
                        ui.text_edit_singleline(answer);
                    } else {
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for o in &c.options {
                                if ui.selectable_label(answer == o, o).clicked() {
                                    *answer = o.clone();
                                }
                            }
                        });
                    }
                    let mut back = false;
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!answer.trim().is_empty(), egui::Button::new("Add")).clicked() {
                            result = Outcome::Done { index, answer: Some(answer.trim().to_owned()) };
                        }
                        back = ui.button("Back").clicked();
                    });
                    if back {
                        self.step = Step::Pick;
                    }
                    return;
                }
                Step::Pick => {}
            }

            let cats = data::categories(&doc);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(220.0));
                egui::ComboBox::from_id_salt("sel_cat")
                    .selected_text(if self.category.is_empty() { "All categories".to_owned() } else { self.category.clone() })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.category, String::new(), "All categories");
                        for c in &cats {
                            ui.selectable_value(&mut self.category, c.clone(), c);
                        }
                    });
                ui.checkbox(&mut self.show_unavailable, "Show unavailable");
            });
            let check = Check { ch, sheet, ignore_quality: None };
            let needle = self.search.to_lowercase();
            let rows: Vec<(usize, String, Vec<String>)> = recs
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.hidden())
                .filter(|(_, r)| self.books.is_empty() || self.books.contains(&r.source()))
                .filter(|(_, r)| self.category.is_empty() || r.category() == self.category)
                .filter_map(|(i, r)| {
                    let name = lang.data_name(self.kind.file, &r.id(), &r.name());
                    if !needle.is_empty() && !name.to_lowercase().contains(&needle) && !r.name().to_lowercase().contains(&needle) {
                        return None;
                    }
                    let mut why = requirements::unmet(r.el(), &check);
                    if self.kind.item == "quality" && requirements::remaining_quality_slots(r.el(), ch) == Some(0) {
                        why.push("already taken".into());
                    }
                    (self.show_unavailable || why.is_empty()).then_some((i, name, why))
                })
                .collect();
            ui.weak(format!("{} shown", rows.len()));
            ui.separator();
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical().id_salt("sel_list").auto_shrink([false; 2]).max_height(480.0).show_rows(&mut cols[0], 20.0, rows.len(), |ui, range| {
                    for (i, name, why) in &rows[range] {
                        let r = recs[*i];
                        let extra: Vec<String> = self.kind.columns.iter().map(|(_, f)| r.get(f)).filter(|v| !v.is_empty()).collect();
                        let mut text = RichText::new(format!("{name}   {}", extra.join(" · ")));
                        if !why.is_empty() {
                            text = text.weak();
                        }
                        let resp = ui.selectable_label(self.selected == Some(*i), text);
                        if resp.clicked() {
                            self.selected = Some(*i);
                        }
                        if resp.double_clicked() && why.is_empty() {
                            self.selected = Some(*i);
                            result = self.confirm(*i, recs[*i].el(), choices_for);
                        }
                    }
                });
                let ui = &mut cols[1];
                egui::ScrollArea::vertical().id_salt("sel_detail").max_height(480.0).show(ui, |ui| match self.selected {
                    Some(i) => {
                        let r = recs[i];
                        ui.heading(lang.data_name(self.kind.file, &r.id(), &r.name()));
                        pdf_ui::source_link(ui, pdfs, SourceRef::of(r.el()), status);
                        let why = requirements::unmet(r.el(), &check);
                        for w in &why {
                            ui.colored_label(ui.visuals().warn_fg_color, w);
                        }
                        ui.separator();
                        record_fields(ui, r.el(), 0);
                    }
                    None => {
                        ui.weak("Select an entry.");
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let can = self.selected.is_some_and(|i| rows.iter().any(|(j, _, w)| *j == i && w.is_empty()));
                if ui.add_enabled(can, egui::Button::new("Add")).clicked() {
                    let i = self.selected.unwrap();
                    result = self.confirm(i, recs[i].el(), choices_for);
                }
                if ui.button("Cancel").clicked() {
                    result = Outcome::Cancel;
                }
            });
        });
        if !open {
            return Outcome::Cancel;
        }
        result
    }

    fn confirm(&mut self, index: usize, rec: &Element, choices_for: &dyn Fn(&Element) -> Vec<Choice>) -> Outcome {
        let choices = choices_for(rec);
        if choices.is_empty() {
            Outcome::Done { index, answer: None }
        } else {
            let answer = if choices[0].options.len() == 1 { choices[0].options[0].clone() } else { String::new() };
            self.step = Step::Answer { index, choices, answer };
            Outcome::None
        }
    }

    pub fn record(&self, store: &DataStore, index: usize) -> Option<Element> {
        let doc = store.doc(self.kind.file).ok()?;
        data::records(&doc, self.kind.container, self.kind.item).get(index).map(|r| r.el().clone())
    }
}
