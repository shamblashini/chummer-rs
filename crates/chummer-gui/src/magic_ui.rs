//! Editors for the Magic, Resonance & Martial Arts tab: spells with their
//! `SelectSpell` options, adept power levels, spirits and sprites, the
//! mentor spirit and its choices, martial art techniques, metamagics and
//! echoes, and focus binding. Career-mode purchases go through the
//! `career` functions so karma is spent and logged.

use std::collections::HashMap;

use chummer_core::bonus::Choice;
use chummer_core::calc::Sheet;
use chummer_core::career;
use chummer_core::character::Character;
use chummer_core::data::{self, DataStore, Record};
use chummer_core::engine::Engine;
use chummer_core::items::magic::{self, account, martialart, mentor, metamagic, power, spell, spirit};
use chummer_core::items::{self, Purchase};
use chummer_core::lang::Language;
use chummer_core::requirements::{self, Check};
use chummer_core::settings::CharacterSettings;
use chummer_core::sources::SourcebookLibrary;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::pdf_ui::Status;
use crate::select::{self, SelectDialog};

/// Read-only context the editors need.
pub struct Ctx<'a> {
    pub store: &'a DataStore,
    pub engine: &'a Engine,
    pub sheet: &'a Sheet,
    pub settings: Option<&'a CharacterSettings>,
    pub lang: &'a Language,
    pub pdfs: &'a SourcebookLibrary,
}

impl Ctx<'_> {
    pub fn books(&self) -> Vec<String> {
        self.settings.map(|s| s.books()).unwrap_or_default()
    }
}

fn report<T, E: std::fmt::Display>(status: &mut Status, r: Result<T, E>, ok: impl FnOnce(T) -> String) -> bool {
    match r {
        Ok(v) => {
            *status = Some((ok(v), false));
            true
        }
        Err(e) => {
            *status = Some((e.to_string(), true));
            false
        }
    }
}

// ---------------------------------------------------------------------------
// A small record picker (metamagics, mentors, lifestyle qualities)
// ---------------------------------------------------------------------------

/// What a [`Picker`] returned this frame.
pub enum Pick {
    None,
    Cancel,
    /// Record name and the answer to its bonus selection.
    Done(String, Option<String>),
}

/// A searchable list of data records with requirement checks and the
/// follow-up bonus selection.
pub struct Picker {
    title: String,
    file: &'static str,
    container: &'static str,
    item: &'static str,
    search: String,
    category: String,
    selected: Option<String>,
    answer: String,
    books: Vec<String>,
}

impl Picker {
    pub fn new(title: impl Into<String>, file: &'static str, container: &'static str, item: &'static str, books: Vec<String>) -> Self {
        Picker {
            title: title.into(),
            file,
            container,
            item,
            search: String::new(),
            category: String::new(),
            selected: None,
            answer: String::new(),
            books,
        }
    }

    /// `choices` gives the selections a record's bonus needs; `note` an
    /// extra line shown next to an entry (e.g. its cost).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        store: &DataStore,
        check: Option<&Check<'_>>,
        choices: &dyn Fn(Record<'_>) -> Vec<Choice>,
        note: &dyn Fn(Record<'_>) -> String,
    ) -> Pick {
        let Ok(doc) = store.doc(self.file) else { return Pick::Cancel };
        let recs = data::records(&doc, self.container, self.item);
        let mut open = true;
        let mut result = Pick::None;
        egui::Window::new(self.title.clone()).id(egui::Id::new(("picker", self.file, self.container))).open(&mut open).default_size([720.0, 520.0]).collapsible(false).show(ctx, |ui| {
            let cats: Vec<String> = {
                let mut c: Vec<String> = recs.iter().map(|r| r.category()).filter(|c| !c.is_empty()).collect();
                c.sort();
                c.dedup();
                c
            };
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(220.0));
                if cats.len() > 1 {
                    egui::ComboBox::from_id_salt("picker_cat")
                        .selected_text(if self.category.is_empty() { "All categories".to_owned() } else { self.category.clone() })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.category, String::new(), "All categories");
                            for c in &cats {
                                ui.selectable_value(&mut self.category, c.clone(), c);
                            }
                        });
                }
            });
            let needle = self.search.to_lowercase();
            let rows: Vec<(Record<'_>, Vec<String>)> = recs
                .iter()
                .copied()
                .filter(|r| !r.hidden())
                .filter(|r| self.books.is_empty() || r.source().is_empty() || self.books.contains(&r.source()))
                .filter(|r| self.category.is_empty() || r.category() == self.category)
                .filter(|r| needle.is_empty() || r.name().to_lowercase().contains(&needle))
                .map(|r| (r, check.map(|c| requirements::unmet(r.el(), c)).unwrap_or_default()))
                .collect();
            ui.separator();
            let mut confirm = false;
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical().id_salt("picker_list").auto_shrink([false; 2]).max_height(380.0).show(&mut cols[0], |ui| {
                    for (r, why) in &rows {
                        let name = r.name();
                        let extra = note(*r);
                        let mut text = RichText::new(if extra.is_empty() { name.clone() } else { format!("{name}   {extra}") });
                        if !why.is_empty() {
                            text = text.weak();
                        }
                        let resp = ui.selectable_label(self.selected.as_deref() == Some(&name), text);
                        if resp.clicked() && self.selected.as_deref() != Some(&name) {
                            self.selected = Some(name.clone());
                            self.answer.clear();
                        }
                        if resp.double_clicked() && why.is_empty() {
                            self.selected = Some(name);
                            confirm = true;
                        }
                    }
                });
                let ui = &mut cols[1];
                let sel = self.selected.as_ref().and_then(|n| rows.iter().find(|(r, _)| &r.name() == n));
                egui::ScrollArea::vertical().id_salt("picker_detail").max_height(380.0).show(ui, |ui| match sel {
                    Some((r, why)) => {
                        ui.heading(r.name());
                        for w in why {
                            ui.colored_label(ui.visuals().warn_fg_color, w);
                        }
                        let ch = choices(*r);
                        if let Some(c) = ch.first() {
                            ui.label(&c.prompt);
                            if c.options.is_empty() {
                                ui.text_edit_singleline(&mut self.answer);
                            } else {
                                if c.options.len() == 1 && self.answer.is_empty() {
                                    self.answer = c.options[0].clone();
                                }
                                egui::ComboBox::from_id_salt("picker_answer").selected_text(self.answer.clone()).width(260.0).show_ui(ui, |ui| {
                                    for o in &c.options {
                                        ui.selectable_value(&mut self.answer, o.clone(), o);
                                    }
                                });
                            }
                        }
                        ui.separator();
                        crate::browser::record_fields(ui, r.el(), 0);
                    }
                    None => {
                        ui.weak("Select an entry.");
                    }
                });
            });
            ui.separator();
            let ready = self.selected.as_ref().and_then(|n| rows.iter().find(|(r, _)| &r.name() == n)).map(|(r, why)| (why.is_empty(), !choices(*r).is_empty()));
            let can = matches!(ready, Some((true, needs)) if !needs || !self.answer.trim().is_empty());
            ui.horizontal(|ui| {
                if ui.add_enabled(can, egui::Button::new("Add")).clicked() || (confirm && can) {
                    let answer = Some(self.answer.trim().to_owned()).filter(|a| !a.is_empty());
                    result = Pick::Done(self.selected.clone().unwrap_or_default(), answer);
                }
                if ui.button("Cancel").clicked() {
                    result = Pick::Cancel;
                }
            });
        });
        if !open {
            return Pick::Cancel;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// The magic editor
// ---------------------------------------------------------------------------

/// A spell picked in the select dialog, waiting for its options.
struct PendingSpell {
    rec: Element,
    answer: Option<String>,
    opts: spell::SpellOptions,
}

#[derive(Default)]
pub struct MagicEditor {
    spell_dialog: Option<SelectDialog>,
    pending_spell: Option<PendingSpell>,
    metamagic: Option<Picker>,
    /// Mentor picker for a quality granting one: (quality guid, type, picker).
    mentor: Option<(String, String, Picker)>,
    /// Chosen mentor choices per mentor guid, before applying.
    mentor_choices: HashMap<String, (String, String)>,
    /// Technique to learn per martial art guid.
    technique: HashMap<String, String>,
}

impl MagicEditor {
    /// The editor for the selected sub-section (`container`) plus the
    /// mentor spirit and foci. Returns true if the character changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, container: &str, status: &mut Status) -> bool {
        let mut changed = false;
        changed |= self.mentor_ui(ui, ch, cx, status);
        changed |= foci_ui(ui, ch, cx, status);
        changed |= match container {
            "spells" => self.spells_ui(ui, ch, cx, status),
            "powers" => powers_ui(ui, ch, cx, status),
            "spirits" => spirits_ui(ui, ch),
            "metamagics" => self.metamagic_ui(ui, ch, cx, status),
            "martialarts" => self.martial_arts_ui(ui, ch, cx, status),
            _ => false,
        };
        changed
    }

    // ----- spells -----

    fn spells_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.button("➕ Add spell…").clicked() {
                let max_avail = cx.settings.map_or(12, |s| s.max_availability());
                self.spell_dialog = SelectDialog::new("spell", cx.store, cx.books(), max_avail, None);
                self.pending_spell = None;
            }
            if !ch.created {
                let c = account::spell_counts(ch, cx.sheet);
                ui.label(format!("Free spells used {} / {}", c.spells + c.rituals + c.preparations, c.free));
            } else {
                ui.label(format!("New spell: {} karma", career::spell_karma_cost(cx.engine, ch, "Spells")));
            }
        });
        let ctx = ui.ctx().clone();
        if let Some(dlg) = self.spell_dialog.as_mut() {
            let store = cx.store;
            let chr: &Character = ch;
            let choices_for = |rec: &Element, p: &Purchase| items::choices("spell", chr, store, Record(rec), p);
            match dlg.show(&ctx, chr, cx.sheet, cx.lang, cx.pdfs, status, &choices_for) {
                select::Outcome::None => {}
                select::Outcome::Cancel => self.spell_dialog = None,
                select::Outcome::Done { index, purchase } => {
                    if let Some(rec) = dlg.record(store, index) {
                        self.pending_spell = Some(PendingSpell { rec, answer: purchase.answer, opts: spell::SpellOptions::default() });
                    }
                    self.spell_dialog = None;
                }
            }
        }
        changed |= self.spell_options_window(&ctx, ch, cx, status);
        changed
    }

    /// The `SelectSpell` checkboxes for the picked spell, then add it.
    fn spell_options_window(&mut self, ctx: &egui::Context, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let Some(p) = self.pending_spell.as_mut() else { return false };
        let rec = Record(&p.rec);
        let mut add = false;
        let mut cancel = false;
        let mut open = true;
        egui::Window::new("Spell options").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.heading(rec.name());
            ui.weak(format!("{} · {} · DV {}", rec.category(), rec.get("range"), rec.get("dv")));
            let descriptors = rec.get("descriptor");
            let extended_area = descriptors.split(',').any(|d| d.trim().eq_ignore_ascii_case("Extended Area"));
            ui.checkbox(&mut p.opts.limited, "Limited").on_hover_text("−2 drain, needs a fetish or focus");
            ui.add_enabled(rec.category() == "Detection" && !extended_area, egui::Checkbox::new(&mut p.opts.extended, "Extended"))
                .on_hover_text("Detection spells only: extended area, +2 drain");
            ui.checkbox(&mut p.opts.alchemical, "Alchemical preparation");
            ui.checkbox(&mut p.opts.free_bonus, "Free").on_hover_text("Costs no karma and does not count against free spells");
            if ch.created && !p.opts.free_bonus {
                let category = if p.opts.alchemical {
                    "Preparations"
                } else if rec.category() == "Rituals" {
                    "Rituals"
                } else {
                    "Spells"
                };
                let cost = career::spell_karma_cost(cx.engine, ch, category);
                let t = RichText::new(format!("Cost: {cost} karma (you have {})", ch.karma));
                ui.label(if cost > ch.karma { t.color(ui.visuals().error_fg_color) } else { t });
            }
            ui.horizontal(|ui| {
                add = ui.button("Add").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || !open {
            self.pending_spell = None;
            return false;
        }
        if !add {
            return false;
        }
        let p = self.pending_spell.take().expect("checked above");
        let rec = Record(&p.rec);
        let name = rec.name();
        if ch.created {
            report(status, career::learn_spell_with(ch, cx.engine, cx.store, rec, p.answer.as_deref(), &p.opts), |_| format!("Learned {name}"))
        } else {
            spell::add(ch, cx.store, rec, p.answer.as_deref(), &p.opts);
            *status = Some((format!("Added {name}"), false));
            true
        }
    }

    // ----- mentor spirit -----

    fn mentor_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let mentors: Vec<Element> = ch.items("mentorspirits", "mentorspirit").into_iter().cloned().collect();
        let pending = mentor::pending_mentor_qualities(ch, cx.store);
        if mentors.is_empty() && pending.is_empty() {
            return false;
        }
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new("Mentor spirit").strong()).id_salt("mentor_ed").default_open(true).show(ui, |ui| {
            for m in &mentors {
                let guid = m.get("guid");
                let mtype = m.child_text("mentortype").unwrap_or_else(|| "MentorSpirit".into());
                ui.horizontal(|ui| {
                    ui.strong(m.get("name"));
                    ui.weak(if mtype == "Paragon" { "(paragon)" } else { "(mentor spirit)" });
                });
                if !m.get("advantage").is_empty() {
                    ui.label(format!("Advantage: {}", m.get("advantage")));
                }
                let Ok(doc) = cx.store.doc(mentor::data_file(&mtype)) else { continue };
                let Some(rec) = mentor_record(&doc, m) else { continue };
                let (set1, set2) = choice_sets(rec);
                if set1.is_empty() && set2.is_empty() {
                    continue;
                }
                let entry = self.mentor_choices.entry(guid.clone()).or_insert_with(|| {
                    let cur = |k: &str, set: &[String]| {
                        let v = m.get(k);
                        if set.contains(&v) {
                            v
                        } else if set.len() == 1 {
                            set[0].clone()
                        } else {
                            String::new()
                        }
                    };
                    (cur("extrachoice1", &set1), cur("extrachoice2", &set2))
                });
                egui::Grid::new(("mentor_choices", &guid)).num_columns(2).show(ui, |ui| {
                    for (label, set, value, n) in [("Choice 1", &set1, &mut entry.0, 1), ("Choice 2", &set2, &mut entry.1, 2)] {
                        if set.is_empty() {
                            continue;
                        }
                        ui.label(label);
                        egui::ComboBox::from_id_salt(("mentor_choice", &guid, n))
                            .selected_text(if value.is_empty() { "Choose…".to_owned() } else { value.clone() })
                            .width(360.0)
                            .show_ui(ui, |ui| {
                                for c in set.iter() {
                                    ui.selectable_value(value, c.clone(), c);
                                }
                            });
                        ui.end_row();
                    }
                });
                let (c1, c2) = entry.clone();
                let saved_ok = m.get("extrachoice1") == c1 && m.get("extrachoice2") == c2;
                if ui.add_enabled(!saved_ok, egui::Button::new("Apply choices")).clicked() {
                    let r = mentor::set_mentor_choices(ch, cx.store, &guid, Some(c1.as_str()).filter(|s| !s.is_empty()), Some(c2.as_str()).filter(|s| !s.is_empty()));
                    changed |= report(status, r, |_| format!("Mentor choices set for {}", m.get("name")));
                }
                ui.add_space(4.0);
            }
            for (qguid, qname, mtype) in &pending {
                ui.horizontal(|ui| {
                    ui.colored_label(crate::view::WARN, format!("{qname} grants a {} that is not chosen yet.", if mtype == "Paragon" { "paragon" } else { "mentor spirit" }));
                    if ui.button("Choose…").clicked() {
                        let picker = Picker::new(format!("Choose a {}", if mtype == "Paragon" { "paragon" } else { "mentor spirit" }), mentor::data_file(mtype), "mentors", "mentor", cx.books());
                        self.mentor = Some((qguid.clone(), mtype.clone(), picker));
                    }
                });
            }
        });
        if let Some((qguid, mtype, picker)) = self.mentor.as_mut() {
            let check = Check { ch, sheet: cx.sheet, ignore_quality: None };
            match picker.show(ui.ctx(), cx.store, Some(&check), &|_| Vec::new(), &|_| String::new()) {
                Pick::None => {}
                Pick::Cancel => self.mentor = None,
                Pick::Done(name, _) => {
                    let r = mentor::add_mentor_for_quality(ch, cx.store, qguid, mtype, &name, None, None);
                    changed |= report(status, r, |_| format!("{name} is now your mentor; pick its choices below"));
                    self.mentor = None;
                }
            }
        }
        changed
    }

    // ----- metamagics and echoes -----

    fn metamagic_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let echo = ch.is_technomancer() && !ch.is_magician();
        let (what, file, container, item) = if echo {
            let (f, c, i) = metamagic::data_path("Echo");
            ("echo", f, c, i)
        } else {
            let (f, c, i) = metamagic::data_path("Metamagic");
            ("metamagic", f, c, i)
        };
        let grade = metamagic::current_grade(ch);
        let taken = ch.items("metamagics", "metamagic").iter().filter(|m| m.get_i32("grade").unwrap_or(0) > 0).count() as i32;
        let free = (grade - taken).max(0);
        ui.horizontal(|ui| {
            let b = ui.add_enabled(free > 0, egui::Button::new(format!("➕ Add {what}…")));
            if b.on_disabled_hover_text(if grade == 0 { "Initiate or submerge first" } else { "Every grade already has one" }).clicked() {
                self.metamagic = Some(Picker::new(format!("Add {what}"), file, container, item, cx.books()));
            }
            ui.label(format!("Grade {grade}: {free} free {what} slot{}", if free == 1 { "" } else { "s" }));
        });
        let mut changed = false;
        if let Some(picker) = self.metamagic.as_mut() {
            let store = cx.store;
            let chr: &Character = ch;
            let check = Check { ch: chr, sheet: cx.sheet, ignore_quality: None };
            let choices = |r: Record<'_>| magic::choices("metamagic", chr, store, r, &Purchase::default());
            match picker.show(ui.ctx(), store, Some(&check), &choices, &|_| String::new()) {
                Pick::None => {}
                Pick::Cancel => self.metamagic = None,
                Pick::Done(name, answer) => {
                    let r = store.doc(file).map_err(|e| e.to_string()).and_then(|doc| {
                        let rec = data::find(&doc, container, item, &name).ok_or_else(|| format!("unknown {what} {name}"))?;
                        metamagic::add(ch, store, rec, answer.as_deref())
                    });
                    changed |= report(status, r, |_| format!("Added {name}"));
                    self.metamagic = None;
                }
            }
        }
        changed
    }

    // ----- martial arts -----

    fn martial_arts_ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let arts: Vec<Element> = ch.items("martialarts", "martialart").into_iter().cloned().collect();
        let Ok(doc) = cx.store.doc("martialarts.xml") else { return false };
        let mut changed = false;
        for art in &arts {
            let guid = art.get("guid");
            let known: Vec<String> = art.child("martialarttechniques").map(|t| t.children_named("martialarttechnique").map(|x| x.get("name")).collect()).unwrap_or_default();
            let rec = mentor_like_find(&doc, "martialarts", "martialart", art);
            let offered: Vec<String> = rec.map(martialart::technique_names).unwrap_or_default().into_iter().filter(|t| !known.contains(t)).collect();
            ui.horizontal_wrapped(|ui| {
                ui.strong(art.get("name"));
                ui.weak(if known.is_empty() { "no techniques".to_owned() } else { known.join(", ") });
            });
            if offered.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                let pick = self.technique.entry(guid.clone()).or_default();
                if !offered.contains(pick) {
                    pick.clear();
                }
                egui::ComboBox::from_id_salt(("technique", &guid))
                    .selected_text(if pick.is_empty() { "Technique…".to_owned() } else { pick.clone() })
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for t in &offered {
                            ui.selectable_value(pick, t.clone(), t);
                        }
                    });
                let cost = if ch.created { career::technique_karma_cost(cx.engine, ch, &guid) } else { 0 };
                let label = if ch.created { format!("Learn ({cost} karma)") } else { "Learn".to_owned() };
                let can = !pick.is_empty() && (!ch.created || ch.karma >= cost);
                if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                    let t = pick.clone();
                    let r = if ch.created { career::learn_technique(ch, cx.engine, cx.store, &guid, &t).map_err(|e| e.to_string()) } else { martialart::add_technique(ch, cx.store, &guid, &t) };
                    changed |= report(status, r, |_| format!("Learned {t}"));
                    pick.clear();
                }
            });
        }
        changed
    }
}

/// The data record of a saved item: by `sourceid`, legacy `id`, then name.
fn mentor_like_find<'a>(doc: &'a Element, container: &str, item: &'a str, saved: &Element) -> Option<Record<'a>> {
    for k in ["sourceid", "id"] {
        let id = saved.get(k);
        if !id.is_empty() {
            if let Some(r) = data::find(doc, container, item, &id) {
                return Some(r);
            }
        }
    }
    data::find(doc, container, item, &saved.get("name"))
}

fn mentor_record<'a>(doc: &'a Element, saved: &Element) -> Option<Record<'a>> {
    mentor_like_find(doc, "mentors", "mentor", saved)
}

/// A mentor's choices split as `SelectMentorSpirit` does: `set="2"` ones
/// fill the second box, the rest the first.
fn choice_sets(rec: Record<'_>) -> (Vec<String>, Vec<String>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for c in rec.el().child("choices").into_iter().flat_map(|c| c.children_named("choice")) {
        if c.attr("set") == Some("2") {
            b.push(c.get("name"));
        } else {
            a.push(c.get("name"));
        }
    }
    (a, b)
}

// ----- adept powers -----

fn powers_ui(ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
    let powers: Vec<Element> = ch.items("powers", "power").into_iter().cloned().collect();
    let second = cx.settings.is_some_and(|s| s.flag("mysadeptsecondmagattribute"));
    let mag = account::adept_mag(ch, cx.sheet, second);
    let (total, used) = match cx.settings {
        Some(s) => account::power_points_with(ch, cx.sheet, s),
        None => account::power_points(ch, cx.sheet),
    };
    let ignore = ch.flag("ignorerules");
    let mut changed = false;
    ui.horizontal(|ui| {
        let t = RichText::new(format!("Power points used {} of {}", fmt_pp(used), fmt_pp(total))).strong();
        ui.label(if used > total + 1e-9 { t.color(ui.visuals().error_fg_color) } else { t });
        if ch.is_adept() && ch.is_magician() && !second {
            let pp = ch.doc.get_i32("magsplitadept").unwrap_or(0);
            ui.label(format!("(mystic adept: {pp} bought)"));
            if ch.created {
                let cost = career::power_point_karma_cost(cx.engine, ch);
                if ui.add_enabled(ch.karma >= cost, egui::Button::new(format!("Buy power point ({cost} karma)"))).clicked() {
                    changed |= report(status, career::buy_power_point(ch, cx.engine), |_| "Bought a power point".into());
                }
            }
        }
    });
    if powers.is_empty() {
        return changed;
    }
    egui::Grid::new("power_editor").striped(true).num_columns(5).spacing([14.0, 4.0]).show(ui, |ui| {
        for h in ["Power", "Levels", "Free", "PP / level", "PP"] {
            ui.strong(h);
        }
        ui.end_row();
        for p in &powers {
            let guid = p.get("guid");
            let extra = p.get("extra");
            ui.label(if extra.is_empty() { p.get("name") } else { format!("{} ({extra})", p.get("name")) });
            let cost_now = power::power_point_cost(ch, p, mag);
            if p.get_bool("levels").unwrap_or(false) {
                let max = power::total_maximum_levels(p, mag, ignore).max(1);
                let mut r = p.get_i32("rating").unwrap_or(1);
                let resp = ui.add(egui::DragValue::new(&mut r).range(1..=max)).on_hover_text(format!("Up to {max}"));
                if resp.changed() && r != p.get_i32("rating").unwrap_or(1) {
                    let mut probe = p.clone();
                    probe.set_child_text("rating", r.to_string());
                    let cost_new = power::power_point_cost(ch, &probe, mag);
                    if !ignore && cost_new > cost_now && used - cost_now + cost_new > total + 1e-9 {
                        *status = Some((format!("Not enough power points for {} level {r}", p.get("name")), true));
                    } else {
                        match power::set_rating(ch, cx.store, &guid, r) {
                            Ok(()) => changed = true,
                            Err(e) => *status = Some((e, true)),
                        }
                    }
                }
            } else {
                ui.label("—");
            }
            let free = power::free_levels(ch, p, mag);
            ui.label(if free > 0 { free.to_string() } else { String::new() });
            ui.label(p.get("pointsperlevel"));
            ui.label(fmt_pp(cost_now));
            ui.end_row();
        }
    });
    changed
}

fn fmt_pp(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

// ----- spirits and sprites -----

fn spirits_ui(ui: &mut egui::Ui, ch: &mut Character) -> bool {
    let spirits: Vec<Element> = ch.items("spirits", "spirit").into_iter().cloned().collect();
    if spirits.is_empty() {
        return false;
    }
    let mut changed = false;
    egui::Grid::new("spirit_editor").striped(true).num_columns(5).spacing([14.0, 4.0]).show(ui, |ui| {
        for h in ["Spirit / sprite", "Force", "Services", "Bound", "Fettered"] {
            ui.strong(h);
        }
        ui.end_row();
        for s in &spirits {
            let sprite = s.get("type") == "Sprite";
            let mut force = s.get_i32("force").unwrap_or(1);
            let mut services = s.get_i32("services").unwrap_or(0);
            let mut bound = s.get_bool("bound").unwrap_or(false);
            let mut fettered = s.get_bool("fettered").unwrap_or(false);
            let name = s.get("crittername");
            ui.label(if name.is_empty() { s.get("name") } else { format!("{name} ({})", s.get("name")) });
            let mut c = ui.add(egui::DragValue::new(&mut force).range(1..=24)).changed();
            c |= ui.add(egui::DragValue::new(&mut services).range(0..=99)).changed();
            c |= ui.checkbox(&mut bound, if sprite { "Registered" } else { "Bound" }).changed();
            if sprite {
                ui.label("");
            } else {
                c |= ui.checkbox(&mut fettered, "").changed();
            }
            if c {
                changed |= spirit::set_state(ch, &s.get("guid"), force, services, bound, fettered);
            }
            ui.end_row();
        }
    });
    changed
}

// ----- foci -----

fn foci_ui(ui: &mut egui::Ui, ch: &mut Character, cx: &Ctx<'_>, status: &mut Status) -> bool {
    let foci: Vec<Element> = ch.items("gears", "gear").into_iter().filter(|g| matches!(g.get("category").as_str(), "Foci" | "Metamagic Foci")).cloned().collect();
    if foci.is_empty() || !ch.mag_enabled() {
        return false;
    }
    let bound: Vec<String> = ch.items("foci", "focus").iter().map(|f| f.get("gearid").to_ascii_lowercase()).collect();
    let mut changed = false;
    egui::CollapsingHeader::new(RichText::new("Foci").strong()).id_salt("foci_ed").default_open(false).show(ui, |ui| {
        let total: i32 = foci.iter().filter(|g| bound.contains(&g.get("guid").to_ascii_lowercase())).map(|g| g.get_i32("rating").unwrap_or(0)).sum();
        ui.weak(format!("Bound force {total} (limit MAG × 5 = {})", cx.sheet.attr("MAG") * 5));
        egui::Grid::new("foci_editor").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
            for h in ["Focus", "Force", "Binding karma", "Bound"] {
                ui.strong(h);
            }
            ui.end_row();
            for g in &foci {
                let guid = g.get("guid");
                let extra = g.get("extra");
                ui.label(if extra.is_empty() { g.get("name") } else { format!("{} ({extra})", g.get("name")) });
                ui.label(g.get("rating"));
                let cost = career::focus_karma_cost(cx.engine, ch, g);
                ui.label(cost.to_string());
                let was = bound.contains(&guid.to_ascii_lowercase());
                let mut on = was;
                let resp = ui.add_enabled(was || !ch.created || ch.karma >= cost, egui::Checkbox::new(&mut on, ""));
                if resp.changed() && on != was {
                    if on {
                        let ok = if ch.created {
                            report(status, career::bind_focus(ch, cx.engine, &guid), |_| format!("Bound {} for {cost} karma", g.get("name")))
                        } else {
                            account::bind_focus(ch, &guid).is_some()
                        };
                        if ok {
                            account::set_focus_bonded(ch, cx.store, &guid, true);
                            changed = true;
                        }
                    } else {
                        account::unbind_focus(ch, &guid);
                        account::set_focus_bonded(ch, cx.store, &guid, false);
                        changed = true;
                    }
                }
                ui.end_row();
            }
        });
    });
    changed
}
