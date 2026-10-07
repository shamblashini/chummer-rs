//! Custom improvements on the Improvements tab: the Create Improvement
//! dialog (`CreateImprovement`), groups, enable toggles, edit, delete and
//! notes. The engine side is `chummer_core::custom_improvement`.

use chummer_core::character::Character;
use chummer_core::command::{Command, ImprovementRef};
use chummer_core::custom_improvement::{self as custom, Field, Form, ImprovementType};
use chummer_core::data::DataStore;
use chummer_core::improvement::Improvement;
use chummer_core::lang::Language;
use chummer_core::settings::CharacterSettings;
use eframe::egui::{self, RichText};

use crate::doc::Doc;
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

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
    /// Open the Create Improvement dialog with `group` preset (the GM
    /// screen's quick override).
    pub fn open_create(&mut self, store: &DataStore, lang: &Language, group: &str) {
        self.dialog = Some(Dialog::new(store, lang, group));
    }

    /// The custom improvements part of the tab. Returns true on a change.
    pub fn tab(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language) -> bool {
        if ws_layout(ui) {
            return self.ws_tab(ui, ch, store, lang);
        }
        let mut changed = false;
        let groups = custom::groups(ch);
        ui.horizontal(|ui| {
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add Improvement"))).clicked() {
                self.dialog = Some(Dialog::new(store, lang, ""));
            }
            ui.add(egui::TextEdit::singleline(&mut self.new_group).hint_text(lang.tr("Group")).desired_width(160.0));
            if ui.add_enabled(!self.new_group.trim().is_empty(), egui::Button::new(lang.tr("Add Group"))).clicked() && ch.set(Command::AddImprovementGroup { name: self.new_group.clone() }) {
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
                                    changed |= ch.set(Command::SetImprovementNotes { at: at(ch, n), notes: text });
                                }
                                if ui.small_button(crate::theme::glyph("✔")).clicked() {
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

    /// The Edit Improvement dialog for `i` (of type `t`).
    fn open_edit(&mut self, store: &DataStore, lang: &Language, i: &Improvement, t: Option<&ImprovementType>) {
        let mut d = Dialog::new(store, lang, &i.custom_group);
        d.pick = d.types.iter().position(|x| x.id == i.custom_id);
        d.form = Form::from_improvement(i, t);
        d.edit = Some(i.source_name.clone());
        self.dialog = Some(d);
    }

    /// [`Self::tab`] in the Workspace style: a toolbar, then each group as
    /// a heading with its buttons over a table of its improvements.
    fn ws_tab(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language) -> bool {
        let ws = crate::theme::ws(ui);
        let mut changed = false;
        let groups = custom::groups(ch);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::button(ui, Some(icons::PLUS), &lang.tr("Add Improvement"), Look::Primary, 26.0).clicked() {
                self.dialog = Some(Dialog::new(store, lang, ""));
            }
            ui.add_space(10.0);
            let r = widgets::preset_input(ui, "new_group", &mut self.new_group, &[], &lang.tr("Group"), 180.0);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !self.new_group.trim().is_empty();
            let add = ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::FOLDER_PLUS), &lang.tr("Add Group"), Look::Secondary, 26.0)).inner.clicked();
            if ok && (add || enter) && ch.set(Command::AddImprovementGroup { name: self.new_group.clone() }) {
                self.new_group.clear();
                changed = true;
            }
        });
        ui.add_space(6.0);
        let listed = custom::listed(ch);
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
        for (g, rows) in &sections {
            let title = if g.is_empty() { lang.tr("Selected Improvements") } else { g.clone() };
            let open_id = ui.id().with(("imp_group_open", g.as_str()));
            let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(true);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if widgets::icon_button(ui, if open { icons::CARET_DOWN } else { icons::CARET_RIGHT }, 22.0).clicked() {
                    open = !open;
                }
                ui.label(icons::icon(icons::FOLDER_SIMPLE, 14.0, ws.muted));
                ui.label(RichText::new(&title).font(widgets::bold(13.0)).color(ws.text));
                ui.label(widgets::mono(rows.len().to_string(), 11.5, ws.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    changed |= self.ws_group_bar(ui, ch, store, lang, g);
                });
            });
            ui.data_mut(|d| d.insert_temp(open_id, open));
            if !open {
                continue;
            }
            if rows.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
                });
                ui.add_space(4.0);
                continue;
            }
            widgets::table_frame(&ws).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let widths = widgets::table_columns(ui, &[18.0, 0.0, 170.0, 240.0, 96.0]);
                for &n in rows {
                    changed |= self.ws_row(ui, ch, store, lang, n, &groups, &widths);
                }
            });
            if let Some(src) = self.notes.clone() {
                if let Some(n) = rows.iter().copied().find(|&n| ch.improvements.list[n].source_name == src && is_head(ch, n)) {
                    ui.add_space(4.0);
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(lang.tr("Notes:")).size(12.0).color(ws.muted));
                        let mut text = ch.improvements.list[n].notes.clone();
                        if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width((ui.available_width() - 40.0).max(120.0))).changed() {
                            changed |= ch.set(Command::SetImprovementNotes { at: at(ch, n), notes: text });
                        }
                        if widgets::icon_button(ui, icons::CHECK, 24.0).on_hover_text(lang.tr("OK")).clicked() {
                            self.notes = None;
                        }
                    });
                }
            }
            ui.add_space(8.0);
        }
        changed
    }

    /// A group's buttons in the Workspace, drawn right to left.
    fn ws_group_bar(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, g: &str) -> bool {
        let mut changed = false;
        if !g.is_empty() {
            if widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove")).clicked() {
                self.confirm = Some(Confirm::Group(g.to_owned()));
            }
            match &mut self.renaming {
                Some((old, new)) if old == g => {
                    if widgets::icon_button(ui, icons::X, 22.0).on_hover_text(lang.tr("Cancel")).clicked() {
                        self.renaming = None;
                    } else if widgets::icon_button(ui, icons::CHECK, 22.0).on_hover_text(lang.tr("OK")).clicked() {
                        let (old, new) = (old.clone(), new.clone());
                        changed |= ch.set(Command::RenameImprovementGroup { old, new });
                        self.renaming = None;
                    } else {
                        widgets::preset_input(ui, ("rename", g), new, &[], "", 160.0);
                    }
                }
                _ => {
                    if widgets::icon_button(ui, icons::PENCIL_SIMPLE, 22.0).on_hover_text(lang.tr("Rename Location")).clicked() {
                        self.renaming = Some((g.to_owned(), g.to_owned()));
                    }
                }
            }
        }
        if widgets::button(ui, Some(icons::SQUARE), &lang.tr("Disable All"), Look::Ghost, 22.0).clicked() {
            changed |= ch.set(Command::SetImprovementGroupEnabled { group: g.to_owned(), on: false });
        }
        if widgets::button(ui, Some(icons::CHECK_SQUARE), &lang.tr("Enable All"), Look::Ghost, 22.0).clicked() {
            changed |= ch.set(Command::SetImprovementGroupEnabled { group: g.to_owned(), on: true });
        }
        if !g.is_empty() && widgets::button(ui, Some(icons::PLUS), &lang.tr("Add Improvement"), Look::Ghost, 22.0).clicked() {
            self.dialog = Some(Dialog::new(store, lang, g));
        }
        changed
    }

    /// One improvement as a Workspace table row: active check, name, type,
    /// what it does, and edit / notes / group / remove buttons.
    #[allow(clippy::too_many_arguments)]
    fn ws_row(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, n: usize, groups: &[String], widths: &[f32]) -> bool {
        let ws = crate::theme::ws(ui);
        let mut changed = false;
        let i = ch.improvements.list[n].clone();
        let name = display_name(&i);
        let t = (!i.custom_id.is_empty()).then(|| custom::find_type(store, &i.custom_id)).flatten();
        let head = is_head(ch, n);
        widgets::table_row(ui, ("custom_imp", n), false, 30.0, |ui| {
            widgets::cell(ui, widths[0], 30.0, |ui| {
                let mut on = i.enabled;
                if widgets::check(ui, &mut on, "").on_hover_text(lang.tr("Active")).changed() {
                    changed |= ch.set(Command::SetImprovementEnabled { at: at(ch, n), on });
                }
            });
            widgets::cell(ui, widths[1], 30.0, |ui| {
                let r = ui.add(egui::Label::new(RichText::new(&name).size(12.5).color(if i.enabled { ws.text } else { ws.muted })).truncate());
                if !i.notes.is_empty() {
                    r.on_hover_text(&i.notes);
                }
            });
            widgets::cell(ui, widths[2], 30.0, |ui| {
                let kind = match &t {
                    Some(t) => type_name(lang, t),
                    None => i.source.clone(),
                };
                ui.add(egui::Label::new(RichText::new(kind).size(12.0).color(ws.muted)).truncate());
            });
            widgets::cell(ui, widths[3], 30.0, |ui| {
                ui.add(egui::Label::new(RichText::new(summary(lang, &i)).size(12.0).color(ws.accent)).truncate());
            });
            widgets::cell(ui, widths[4], 30.0, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if head && i.custom && i.source == custom::SOURCE && t.is_some() && widgets::icon_button(ui, icons::PENCIL_SIMPLE, 22.0).on_hover_text(lang.tr("Edit Improvement")).clicked() {
                    self.open_edit(store, lang, &i, t.as_ref());
                }
                if head {
                    let glyph = if i.notes.is_empty() { icons::NOTE_BLANK } else { icons::NOTE };
                    if widgets::icon_button(ui, glyph, 22.0).on_hover_text(lang.tr("Notes")).clicked() {
                        self.notes = if self.notes.as_deref() == Some(i.source_name.as_str()) { None } else { Some(i.source_name.clone()) };
                    }
                }
                if i.custom && !groups.is_empty() {
                    let r = widgets::icon_button(ui, icons::FOLDER_SIMPLE, 22.0).on_hover_text(lang.tr("Group"));
                    egui::Popup::menu(&r).show(|ui| {
                        for g in std::iter::once(String::new()).chain(groups.iter().cloned()) {
                            let label = if g.is_empty() { lang.tr("Selected Improvements") } else { g.clone() };
                            if crate::combo::selectable_label(ui, i.custom_group == g, label).clicked() {
                                changed |= ch.set(Command::SetImprovementGroup { at: at(ch, n), group: g.clone() });
                                ui.close();
                            }
                        }
                    });
                }
                if widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove")).clicked() {
                    self.confirm = Some(Confirm::Improvement(i.source_name.clone(), name.clone()));
                }
            });
        });
        changed
    }

    /// Buttons for one group: add into it, enable/disable all, rename, delete.
    fn group_bar(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, g: &str) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            if !g.is_empty() && ui.small_button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add Improvement"))).clicked() {
                self.dialog = Some(Dialog::new(store, lang, g));
            }
            if ui.small_button(lang.tr("Enable All")).clicked() {
                changed |= ch.set(Command::SetImprovementGroupEnabled { group: g.to_owned(), on: true });
            }
            if ui.small_button(lang.tr("Disable All")).clicked() {
                changed |= ch.set(Command::SetImprovementGroupEnabled { group: g.to_owned(), on: false });
            }
            if g.is_empty() {
                return;
            }
            match &mut self.renaming {
                Some((old, new)) if old == g => {
                    ui.add(egui::TextEdit::singleline(new).desired_width(140.0));
                    if ui.small_button(crate::theme::glyph("✔")).clicked() {
                        let (old, new) = (old.clone(), new.clone());
                        changed |= ch.set(Command::RenameImprovementGroup { old, new });
                        self.renaming = None;
                    }
                    if ui.small_button(crate::theme::glyph("✖")).clicked() {
                        self.renaming = None;
                    }
                }
                _ => {
                    if ui.small_button(lang.tr("Rename Location")).clicked() {
                        self.renaming = Some((g.to_owned(), g.to_owned()));
                    }
                }
            }
            if ui.small_button(crate::theme::glyph("🗑")).on_hover_text(lang.tr("Remove")).clicked() {
                self.confirm = Some(Confirm::Group(g.to_owned()));
            }
        });
        changed
    }

    fn row(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, n: usize, groups: &[String]) -> bool {
        let mut changed = false;
        let i = ch.improvements.list[n].clone();
        let mut on = i.enabled;
        if ui.checkbox(&mut on, "").on_hover_text(lang.tr("Active")).changed() {
            changed |= ch.set(Command::SetImprovementEnabled { at: at(ch, n), on });
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
            if head && i.custom && i.source == custom::SOURCE && t.is_some() && ui.small_button(crate::theme::glyph("✏")).on_hover_text(lang.tr("Edit Improvement")).clicked() {
                self.open_edit(store, lang, &i, t.as_ref());
            }
            if head && ui.small_button(crate::theme::glyph("📝")).on_hover_text(lang.tr("Notes")).clicked() {
                self.notes = if self.notes.as_deref() == Some(i.source_name.as_str()) { None } else { Some(i.source_name.clone()) };
            }
            if i.custom && !groups.is_empty() {
                ui.menu_button(crate::theme::glyph("📁"), |ui| {
                    for g in std::iter::once(String::new()).chain(groups.iter().cloned()) {
                        let label = if g.is_empty() { lang.tr("Selected Improvements") } else { g.clone() };
                        if crate::combo::selectable_label(ui, i.custom_group == g, label).clicked() {
                            changed |= ch.set(Command::SetImprovementGroup { at: at(ch, n), group: g.clone() });
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(lang.tr("Group"));
            }
            if ui.small_button(crate::theme::glyph("🗑")).on_hover_text(lang.tr("Remove")).clicked() {
                self.confirm = Some(Confirm::Improvement(i.source_name.clone(), name.clone()));
            }
        });
        ui.end_row();
        changed
    }

    /// The Create Improvement dialog and delete confirmations.
    pub fn window(&mut self, ctx: &egui::Context, ch: &mut Doc, store: &DataStore, settings: Option<&CharacterSettings>, lang: &Language) -> bool {
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
                    // Free text, with the values the selection offers as presets.
                    let presets: Vec<(String, String)> = opts.iter().map(|o| (o.clone(), o.clone())).collect();
                    crate::workspace::widgets::preset_input(ui, "imp_select", &mut d.form.select, &presets, "", 320.0);
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
                ui.colored_label(crate::theme::warn(ui), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled_ui(d.pick.is_some(), |ui| dialog_button(ui, &lang.tr("OK"), true)).inner.clicked() {
                    match ch.apply(Command::CreateImprovement { form: d.form.clone(), group: d.group.clone(), edit: d.edit.clone() }) {
                        Ok(_) => {
                            changed = true;
                            done = true;
                        }
                        Err(e) => d.error = Some(lang.tr(&e.reason)),
                    }
                }
                if dialog_button(ui, &lang.tr("Cancel"), false).clicked() {
                    done = true;
                }
            });
        });
        if !open || done {
            self.dialog = None;
        }
        changed
    }

    fn confirm_window(&mut self, ctx: &egui::Context, ch: &mut Doc, lang: &Language) -> bool {
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
                if dialog_button(ui, &lang.tr("Remove"), true).clicked() {
                    answer = Some(true);
                }
                if dialog_button(ui, &lang.tr("Cancel"), false).clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                let changed = match self.confirm.take() {
                    Some(Confirm::Improvement(src, _)) => ch.set(Command::RemoveImprovement { source: src }),
                    Some(Confirm::Group(g)) => ch.set(Command::RemoveImprovementGroup { name: g }),
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

/// The improvement at `n`, for a command.
fn at(ch: &Character, n: usize) -> ImprovementRef {
    ImprovementRef { index: n as u32, source: ch.improvements.list[n].source_name.clone() }
}

/// The first improvement of its source: it carries the notes and the edit
/// button.
fn is_head(ch: &Character, n: usize) -> bool {
    let src = &ch.improvements.list[n].source_name;
    ch.improvements.list.iter().position(|i| &i.source_name == src && (i.custom || !ch.improvements.list.iter().any(|j| &j.source_name == src && j.custom))) == Some(n)
}

/// Whether the Workspace layout is active (its widgets replace Classic's).
fn ws_layout(ui: &egui::Ui) -> bool {
    crate::theme::current(ui.ctx()).workspace_layout()
}

/// A dialog button: a Workspace button (`primary` filled) or a Classic one.
fn dialog_button(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    if ws_layout(ui) {
        widgets::button(ui, None, text, if primary { Look::Primary } else { Look::Secondary }, 26.0)
    } else {
        ui.button(text)
    }
}
