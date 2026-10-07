//! Data browser: search and read every record in the game data.

use std::sync::Arc;

use chummer_core::data::{self, DataStore};
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use eframe::egui;

use crate::pdf_ui::{self, Status};

pub struct DataBrowser {
    kind: usize,
    search: String,
    selected: Option<usize>,
    doc: Option<Arc<Element>>,
    loaded_kind: Option<usize>,
    error: Option<String>,
}

impl Default for DataBrowser {
    fn default() -> Self {
        Self { kind: 18, search: String::new(), selected: None, doc: None, loaded_kind: None, error: None }
    }
}

impl DataBrowser {
    /// Show record `index` of kind `kind` (an index into
    /// `data::BROWSABLE`), as the Workspace's palette does.
    pub fn show_record(&mut self, store: &DataStore, kind: usize, index: usize) {
        self.kind = kind.min(data::BROWSABLE.len() - 1);
        self.search.clear();
        self.load(store);
        self.selected = Some(index);
    }

    fn load(&mut self, store: &DataStore) {
        let (_, file, _, _) = data::BROWSABLE[self.kind];
        if self.loaded_kind != Some(self.kind) {
            match store.doc(file) {
                Ok(d) => {
                    self.doc = Some(d);
                    self.error = None;
                }
                Err(e) => {
                    self.doc = None;
                    self.error = Some(e.to_string());
                }
            }
            self.loaded_kind = Some(self.kind);
            self.selected = None;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, store: &DataStore, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) {
        let (label, file, container, item) = data::BROWSABLE[self.kind];
        self.load(store);

        ui.horizontal(|ui| {
            crate::combo::Combo::from_id_salt("browser_kind").selected_text(lang.tr(label)).width(180.0).show_ui(ui, |ui| {
                for (i, (l, ..)) in data::BROWSABLE.iter().enumerate() {
                    crate::combo::selectable_value(ui, &mut self.kind, i, lang.tr(l));
                }
            });
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(lang.tr("Search name, category or source")).desired_width(280.0));
            if ui.button(lang.tr("Clear")).clicked() {
                self.search.clear();
            }
        });
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let _s = crate::trace::span("master index search");
        let recs = data::records(&doc, container, item);
        let needle = self.search.to_lowercase();
        let shown: Vec<(usize, String, String)> = recs
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let name = lang.data_name(file, &r.id(), &r.name());
                let cat = r.category();
                let hay = format!("{} {} {} {}", name, r.name(), cat, r.source()).to_lowercase();
                (needle.is_empty() || hay.contains(&needle)).then_some((i, name, cat))
            })
            .collect();
        drop(_s);
        ui.label(lang.tr_fmt("{0} of {1} records", &[&shown.len(), &recs.len()]));
        ui.separator();

        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("browser_list").auto_shrink([false; 2]).show_rows(
                &mut cols[0],
                18.0,
                shown.len(),
                |ui, range| {
                    for (i, name, cat) in &shown[range] {
                        let text = if cat.is_empty() { name.clone() } else { format!("{name}  ·  {}", lang.data_name(file, "", cat)) };
                        if crate::combo::selectable_label(ui, self.selected == Some(*i), text).clicked() {
                            self.selected = Some(*i);
                        }
                    }
                },
            );
            egui::ScrollArea::vertical().id_salt("browser_detail").auto_shrink([false; 2]).show(&mut cols[1], |ui| {
                match self.selected.and_then(|i| recs.get(i)) {
                    Some(r) => {
                        ui.heading(lang.data_name(file, &r.id(), &r.name()));
                        pdf_ui::source_link(ui, pdfs, lang, SourceRef::of(r.el()), status);
                        ui.separator();
                        record_fields(ui, r.el(), 0);
                    }
                    None => {
                        ui.weak(lang.tr("Select a record to see its details."));
                    }
                }
            });
        });
    }
}

/// Show a record's fields as a nested key/value grid.
pub fn record_fields(ui: &mut egui::Ui, el: &Element, depth: usize) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).striped(depth == 0).show(ui, |ui| {
        for c in el.elements() {
            if depth == 0 && matches!(c.name.as_str(), "id" | "name" | "source" | "page") {
                continue;
            }
            ui.strong(&c.name);
            if c.elements().next().is_some() {
                ui.vertical(|ui| record_fields(ui, c, depth + 1));
            } else {
                let mut t = c.text();
                for (k, v) in &c.attrs {
                    t.push_str(&format!("  [{k}={v}]"));
                }
                if t.is_empty() {
                    t = "✓".into();
                }
                ui.label(t);
            }
            ui.end_row();
        }
    });
}
