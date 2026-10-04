//! Custom improvements on the Improvements tab: the Create Improvement
//! dialog (`CreateImprovement`), groups, enable toggles, edit, delete and
//! notes. The engine side is `chummer_core::custom_improvement`.

use chummer_core::character::Character;
use chummer_core::custom_improvement::{self as custom, Field, Form, ImprovementType};
use chummer_core::data::DataStore;
use chummer_core::improvement::Improvement;
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct ImprovementsPanel {
    dialog: Option<Dialog>,
    new_group: String,
    /// Group being renamed: (old name, new name).
    renaming: Option<(String, String)>,
    confirm: Option<Confirm>,
    /// Source GUID whose notes are open for editing.
    notes: Option<String>,
}

enum Confirm {
    /// (source GUID, display name)
    Improvement(String, String),
    Group(String),
}

struct Dialog {
    /// Improvement types, sorted by display name.
    types: Vec<ImprovementType>,
    pick: Option<usize>,
    form: Form,
    group: String,
    /// Source GUID of the improvement being edited.
    edit: Option<String>,
    /// Selection values for `pick`, computed when the type changes.
    options: Option<(usize, Vec<String>)>,
    error: Option<String>,
}

impl Dialog {
    fn new(store: &DataStore, lang: &Language, group: &str) -> Dialog {
        let mut types = custom::types(store);
        types.sort_by_cached_key(|t| type_name(lang, t).to_lowercase());
        Dialog { types, pick: None, form: Form::default(), group: group.to_owned(), edit: None, options: None, error: None }
    }

    fn current(&self) -> Option<&ImprovementType> {
        self.pick.and_then(|p| self.types.get(p))
    }
}

fn type_name(lang: &Language, t: &ImprovementType) -> String {
    lang.data_name(custom::FILE, &t.id, &t.name)
}

/// What a row shows for an improvement (`RefreshSelectedImprovement`).
fn summary(lang: &Language, i: &Improvement) -> String {
    let mut parts = Vec::new();
    let mut put = |label: &str, v: String| parts.push(format!("{} {v}", lang.tr(label)));
    if !i.improved_name.is_empty() {
        put("Selected Value:", i.improved_name.clone());
    }
    if i.rating != 0 && i.rating != 1 {
        put("Rating:", i.rating.to_string());
    }
    for (label, v) in [("Value:", i.val), ("Minimum:", i.min), ("Maximum:", i.max), ("Augmented:", i.aug)] {
        if v != 0.0 {
            put(label, chummer_core::improvement::fmt_num(v));
        }
    }
    parts.join("   ")
}

impl ImprovementsPanel {
    /// The custom improvements part of the tab. Returns true on a change.
    pub fn tab(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language) -> bool {
        let mut changed = false;
        let groups = custom::groups(ch);
        ui.horizontal(|ui| {
            if ui.button(format!("➕ {}", lang.tr("Add Improvement"))).clicked() {
                self.dialog = Some(Dialog::new(store, lang, ""));
            }
            ui.add(egui::TextEdit::singleline(&mut self.new_group).hint_text(lang.tr("Group")).desired_width(160.0));
            if ui.add_enabled(!self.new_group.trim().is_empty(), egui::Button::new(lang.tr("Add Group"))).clicked() && custom::add_group(ch, &self.new_group) {
                self.new_group.clear();
                changed = true;
            }
        });
        let listed = custom::listed(ch);
        // Each group's rows, sorted by name; unknown groups go to the root.
        let rows_of = |g: &str| -> Vec<usize> {
            let mut v: Vec<usize> = listed
                .iter()
                .copied()
                .filter(|&n| {
                    let cg = &ch.improvements.list[n].custom_group;
                    if g.is_empty() {
                        cg.is_empty() || !groups.contains(cg)
                    } else {
                        cg == g
                    }
                })
                .collect();
            v.sort_by_cached_key(|&n| display_name(&ch.improvements.list[n]).to_lowercase());
            v
        };
        let sections: Vec<(String, Vec<usize>)> = std::iter::once(String::new()).chain(groups.iter().cloned()).map(|g| { let r = rows_of(&g); (g, r) }).collect();
        egui::ScrollArea::vertical().id_salt("custom_imps").max_height(ui.available_height() * 0.55).show(ui, |ui| {
            for (g, rows) in &sections {
                let title = if g.is_empty() { lang.tr("Selected Improvements") } else { g.clone() };
                egui::CollapsingHeader::new(RichText::new(format!("{title} ({})", rows.len())).strong()).id_salt(("imp_group", g.as_str())).default_open(true).show(ui, |ui| {
                    changed |= self.group_bar(ui, ch, store, lang, g);
                    egui::Grid::new(("imp_rows", g.as_str())).striped(true).num_columns(5).spacing([10.0, 3.0]).show(ui, |ui| {
                        for &n in rows {
                            changed |= self.row(ui, ch, store, lang, n, &groups);
                        }
                    });
                    if let Some(src) = self.notes.clone() {
                        if let Some(n) = rows.iter().copied().find(|&n| ch.improvements.list[n].source_name == src && is_head(ch, n)) {
                            ui.horizontal(|ui| {
                                ui.label(lang.tr("Notes:"));
                                let mut text = ch.improvements.list[n].notes.clone();
                                if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width(480.0)).changed() {
                                    custom::set_notes(ch, n, &text);
                                    changed = true;
                                }
                                if ui.small_button("✔").clicked() {
                                    self.notes = None;
                                }
                            });
                        }
                    }
                });
            }
        });
        changed
    }

    /// Buttons for one group: add into it, enable/disable all, rename, delete.
    fn group_bar(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language, g: &str) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            if !g.is_empty() && ui.small_button(format!("➕ {}", lang.tr("Add Improvement"))).clicked() {
                self.dialog = Some(Dialog::new(store, lang, g));
            }
            if ui.small_button(lang.tr("Enable All")).clicked() {
                changed |= custom::set_group_enabled(ch, g, true) > 0;
            }
            if ui.small_button(lang.tr("Disable All")).clicked() {
                changed |= custom::set_group_enabled(ch, g, false) > 0;
            }
            if g.is_empty() {
                return;
            }
            match &mut self.renaming {
                Some((old, new)) if old == g => {
                    ui.add(egui::TextEdit::singleline(new).desired_width(140.0));
                    if ui.small_button("✔").clicked() {
                        let (old, new) = (old.clone(), new.clone());
                        changed |= custom::rename_group(ch, &old, &new);
                        self.renaming = None;
                    }
                    if ui.small_button("✖").clicked() {
                        self.renaming = None;
                    }
                }
                _ => {
                    if ui.small_button(lang.tr("Rename Location")).clicked() {
                        self.renaming = Some((g.to_owned(), g.to_owned()));
                    }
                }
            }
            if ui.small_button("🗑").on_hover_text(lang.tr("Remove")).clicked() {
                self.confirm = Some(Confirm::Group(g.to_owned()));
            }
        });
        changed
    }

    fn row(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language, n: usize, groups: &[String]) -> bool {
        let mut changed = false;
        let i = ch.improvements.list[n].clone();
        let mut on = i.enabled;
        if ui.checkbox(&mut on, "").on_hover_text(lang.tr("Active")).changed() {
            changed |= custom::set_enabled(ch, n, on);
        }
        let name = display_name(&i);
        let label = if i.enabled { RichText::new(&name) } else { RichText::new(&name).weak() };
        let resp = ui.label(label);
        if !i.notes.is_empty() {
            resp.on_hover_text(&i.notes);
        }
        let t = (!i.custom_id.is_empty()).then(|| custom::find_type(store, &i.custom_id)).flatten();
        match &t {
            Some(t) => ui.weak(type_name(lang, t)),
            None => ui.weak(&i.source),
        };
        ui.label(summary(lang, &i));
        ui.horizontal(|ui| {
            let head = is_head(ch, n);
            if head && i.custom && i.source == custom::SOURCE && t.is_some() && ui.small_button("✏").on_hover_text(lang.tr("Edit Improvement")).clicked() {
                let mut d = Dialog::new(store, lang, &i.custom_group);
                d.pick = d.types.iter().position(|x| x.id == i.custom_id);
                d.form = Form::from_improvement(&i, t.as_ref());
                d.edit = Some(i.source_name.clone());
                self.dialog = Some(d);
            }
            if head && ui.small_button("📝").on_hover_text(lang.tr("Notes")).clicked() {
                self.notes = if self.notes.as_deref() == Some(i.source_name.as_str()) { None } else { Some(i.source_name.clone()) };
            }
            if i.custom && !groups.is_empty() {
                ui.menu_button("📁", |ui| {
                    for g in std::iter::once(String::new()).chain(groups.iter().cloned()) {
                        let label = if g.is_empty() { lang.tr("Selected Improvements") } else { g.clone() };
                        if crate::combo::selectable_label(ui, i.custom_group == g, label).clicked() {
                            custom::set_group(ch, n, &g);
                            changed = true;
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(lang.tr("Group"));
            }
            if ui.small_button("🗑").on_hover_text(lang.tr("Remove")).clicked() {
                self.confirm = Some(Confirm::Improvement(i.source_name.clone(), name.clone()));
            }
        });
        ui.end_row();
        changed
    }

    /// The Create Improvement dialog and delete confirmations.
    pub fn window(&mut self, ctx: &egui::Context, ch: &mut Character, store: &DataStore, settings: Option<&CharacterSettings>, lang: &Language) -> bool {
        let mut changed = self.confirm_window(ctx, ch, lang);
        let Some(d) = &mut self.dialog else { return changed };
        let mut open = true;
        let mut done = false;
        let title = if d.edit.is_some() { lang.tr("Edit Improvement") } else { lang.tr("Create Improvement") };
        egui::Window::new(title).id(egui::Id::new("create_improvement")).open(&mut open).default_width(520.0).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("create_imp").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(lang.tr("Improvement Type:"));
                let shown = d.current().map(|t| type_name(lang, t)).unwrap_or_default();
                let before = d.pick;
                crate::combo::Combo::from_id_salt("imp_type").width(320.0).selected_text(shown).height(420.0).show_ui(ui, |ui| {
                    for (k, t) in d.types.iter().enumerate() {
                        crate::combo::selectable_value(ui, &mut d.pick, Some(k), type_name(lang, t));
                    }
                });
                if d.pick != before {
                    // cboImprovemetType_SelectedIndexChanged clears the selection.
                    d.form.select.clear();
                    d.form.apply_to_rating = false;
                    d.form.free = false;
                    d.error = None;
                }
                ui.end_row();
                ui.label(lang.tr("Name:"));
                ui.add(egui::TextEdit::singleline(&mut d.form.name).desired_width(320.0));
                ui.end_row();
                let Some(t) = d.current().cloned() else { return };
                d.form.type_id = t.id.clone();
                if let Some(sel) = t.selection() {
                    let pick = d.pick.unwrap_or_default();
                    if d.options.as_ref().is_none_or(|(p, _)| *p != pick) {
                        d.options = Some((pick, custom::options(ch, store, settings, sel)));
                    }
                    let opts = d.options.as_ref().map(|(_, o)| o.as_slice()).unwrap_or_default();
                    ui.label(lang.tr("Selected Value:"));
                    if opts.is_empty() {
                        ui.add(egui::TextEdit::singleline(&mut d.form.select).desired_width(320.0));
                    } else {
                        let form = &mut d.form;
                        crate::combo::Combo::from_id_salt("imp_select").width(320.0).selected_text(form.select.clone()).height(360.0).show_ui(ui, |ui| {
                            for o in opts.iter() {
                                crate::combo::selectable_value(ui, &mut form.select, o.clone(), o);
                            }
                        });
                    }
                    ui.end_row();
                }
                let num = |ui: &mut egui::Ui, label: &str, v: &mut f64, decimals: usize| {
                    ui.label(lang.tr(label));
                    ui.add(egui::DragValue::new(v).speed(0.1).max_decimals(decimals));
                    ui.end_row();
                };
                if t.has(&Field::Val) {
                    num(ui, "Value:", &mut d.form.val, 2);
                }
                if t.has(&Field::Min) {
                    num(ui, "Minimum:", &mut d.form.min, 0);
                }
                if t.has(&Field::Max) {
                    num(ui, "Maximum:", &mut d.form.max, 0);
                }
                if t.has(&Field::Percent) {
                    num(ui, "Percent:", &mut d.form.aug, 0);
                } else if t.has(&Field::Aug) {
                    num(ui, "Augmented:", &mut d.form.aug, 0);
                }
                if t.has(&Field::ApplyToRating) {
                    ui.label("");
                    ui.checkbox(&mut d.form.apply_to_rating, lang.tr("Apply to Rating"));
                    ui.end_row();
                }
                if t.has(&Field::Free) {
                    ui.label("");
                    ui.checkbox(&mut d.form.free, lang.tr("Free!"));
                    ui.end_row();
                }
            });
            if let Some(t) = d.current() {
                ui.separator();
                egui::ScrollArea::vertical().id_salt("imp_help").max_height(140.0).show(ui, |ui| {
                    ui.weak(&t.page);
                });
            }
            if let Some(e) = &d.error {
                ui.colored_label(crate::view::WARN, e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(d.pick.is_some(), egui::Button::new(lang.tr("OK"))).clicked() {
                    match custom::create(ch, store, &d.form, &d.group, d.edit.as_deref()) {
                        Ok(_) => {
                            changed = true;
                            done = true;
                        }
                        Err(e) => d.error = Some(lang.tr(&e.to_string())),
                    }
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    done = true;
                }
            });
        });
        if !open || done {
            self.dialog = None;
        }
        changed
    }

    fn confirm_window(&mut self, ctx: &egui::Context, ch: &mut Character, lang: &Language) -> bool {
        let Some(c) = &self.confirm else { return false };
        let text = match c {
            Confirm::Improvement(_, name) => format!("{}\n{name}", lang.tr("Are you sure you want to delete this Improvement?")),
            Confirm::Group(g) => format!(
                "{}\n{g}",
                lang.tr("Are you sure you want to delete this Improvement Group? All of the Improvements in this group will be moved to the Selected Improvements container.")
            ),
        };
        let mut answer = None;
        egui::Window::new(lang.tr("Remove")).id(egui::Id::new("confirm_imp_delete")).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(text);
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Remove")).clicked() {
                    answer = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                let changed = match self.confirm.take() {
                    Some(Confirm::Improvement(src, _)) => custom::remove(ch, &src),
                    Some(Confirm::Group(g)) => custom::remove_group(ch, &g),
                    None => false,
                };
                self.notes = None;
                changed
            }
            Some(false) => {
                self.confirm = None;
                false
            }
            None => false,
        }
    }
}

/// The custom name, or what the improvement does when it has none.
fn display_name(i: &Improvement) -> String {
    if !i.custom_name.is_empty() {
        i.custom_name.clone()
    } else if i.improved_name.is_empty() {
        i.kind.clone()
    } else {
        format!("{} {}", i.kind, i.improved_name)
    }
}

/// The first improvement of its source: it carries the notes and the edit
/// button.
fn is_head(ch: &Character, n: usize) -> bool {
    let src = &ch.improvements.list[n].source_name;
    ch.improvements.list.iter().position(|i| &i.source_name == src && (i.custom || !ch.improvements.list.iter().any(|j| &j.source_name == src && j.custom))) == Some(n)
}
