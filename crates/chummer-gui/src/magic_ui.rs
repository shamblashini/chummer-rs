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
use chummer_core::command::{Command, RecordRef};
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

use crate::doc::Doc;
use crate::pdf_ui::Status;
use crate::select::{self, SelectDialog};
use crate::workspace::widgets::{self, Look};
use crate::workspace::{dialog, icons};

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
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        store: &DataStore,
        lang: &Language,
        check: Option<&Check<'_>>,
        choices: &dyn Fn(Record<'_>) -> Vec<Choice>,
        note: &dyn Fn(Record<'_>) -> String,
    ) -> Pick {
        let Ok(doc) = store.doc(self.file) else { return Pick::Cancel };
        let recs = data::records(&doc, self.container, self.item);
        let mut open = true;
        let mut result = Pick::None;
        let title = self.title.clone();
        dialog::window(ctx, ("picker", self.file, self.container), &title, &mut open, egui::vec2(720.0, 520.0), true, |ui| {
            let cats: Vec<String> = {
                let mut c: Vec<String> = recs.iter().map(|r| r.category()).filter(|c| !c.is_empty()).collect();
                c.sort();
                c.dedup();
                c
            };
            ui.horizontal(|ui| {
                dialog::search(ui, &mut self.search, &lang.tr("Search"), 240.0);
                if cats.len() > 1 {
                    crate::combo::Combo::from_id_salt("picker_cat")
                        .selected_text(if self.category.is_empty() { lang.tr("All categories") } else { lang.data_name(self.file, "", &self.category) })
                        .show_ui(ui, |ui| {
                            crate::combo::selectable_value(ui, &mut self.category, String::new(), lang.tr("All categories"));
                            for c in &cats {
                                crate::combo::selectable_value(ui, &mut self.category, c.clone(), lang.data_name(self.file, "", c));
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
            dialog::rule(ui);
            let mut confirm = false;
            ui.columns(2, |cols| {
                dialog::list_frame(&cols[0]).show(&mut cols[0], |ui| {
                    egui::ScrollArea::vertical().id_salt("picker_list").auto_shrink([false; 2]).max_height(380.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (r, why) in &rows {
                            let name = r.name();
                            let extra = note(*r);
                            let resp = dialog::list_row(ui, self.selected.as_deref() == Some(&name), &name, &extra, !why.is_empty());
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
                });
                let ui = &mut cols[1];
                let sel = self.selected.as_ref().and_then(|n| rows.iter().find(|(r, _)| &r.name() == n));
                egui::ScrollArea::vertical().id_salt("picker_detail").max_height(380.0).show(ui, |ui| match sel {
                    Some((r, why)) => {
                        dialog::heading(ui, &r.name());
                        for w in why {
                            dialog::warning(ui, w.clone());
                        }
                        let ch = choices(*r);
                        if let Some(c) = ch.first() {
                            dialog::caption(ui, &c.prompt);
                            if c.options.is_empty() {
                                dialog::text_input(ui, &mut self.answer, "", 260.0);
                            } else {
                                if c.options.len() == 1 && self.answer.is_empty() {
                                    self.answer = c.options[0].clone();
                                }
                                crate::combo::Combo::from_id_salt("picker_answer").selected_text(self.answer.clone()).width(260.0).show_ui(ui, |ui| {
                                    for o in &c.options {
                                        crate::combo::selectable_value(ui, &mut self.answer, o.clone(), o);
                                    }
                                });
                            }
                        }
                        dialog::rule(ui);
                        crate::browser::record_fields(ui, r.el(), 0);
                    }
                    None => {
                        dialog::note(ui, lang.tr("Select an entry."));
                    }
                });
            });
            let ready = self.selected.as_ref().and_then(|n| rows.iter().find(|(r, _)| &r.name() == n)).map(|(r, why)| (why.is_empty(), !choices(*r).is_empty()));
            let can = matches!(ready, Some((true, needs)) if !needs || !self.answer.trim().is_empty());
            dialog::buttons(ui, |ui| {
                if ui.add_enabled_ui(can, |ui| dialog::button(ui, &lang.tr("Add"), true)).inner.clicked() || (confirm && can) {
                    let answer = Some(self.answer.trim().to_owned()).filter(|a| !a.is_empty());
                    result = Pick::Done(self.selected.clone().unwrap_or_default(), answer);
                }
                if dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
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
    /// Career mode: the grade a new metamagic or echo goes to.
    metamagic_grade: Option<i32>,
    /// Mentor picker for a quality granting one: (quality guid, type, picker).
    mentor: Option<(String, String, Picker)>,
    /// Chosen mentor choices per mentor guid, before applying.
    mentor_choices: HashMap<String, (String, String)>,
    /// Technique to learn per martial art guid.
    technique: HashMap<String, String>,
}

impl MagicEditor {
    /// The mentor spirit and foci, shown once per page.
    pub fn shared_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let changed = self.mentor_ui(ui, ch, cx, status);
        changed | foci_ui(ui, ch, cx, status)
    }

    /// The editor for one sub-section (`container`). Returns true if the
    /// character changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, container: &str, status: &mut Status) -> bool {
        let mut changed = false;
        changed |= match container {
            "spells" => self.spells_ui(ui, ch, cx, status),
            "powers" => powers_ui(ui, ch, cx, status),
            "spirits" => spirits_ui(ui, ch, cx, status),
            "metamagics" => self.metamagic_ui(ui, ch, cx, status),
            "martialarts" => self.martial_arts_ui(ui, ch, cx, status),
            _ => false,
        };
        changed
    }

    // ----- spells -----

    fn spells_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let mut changed = false;
        let lang = cx.lang;
        ui.horizontal(|ui| {
            if add_button(ui, &lang.tr("Add Spell…"), true).clicked() {
                let max_avail = cx.settings.map_or(12, |s| s.max_availability());
                self.spell_dialog = SelectDialog::new("spell", cx.store, cx.books(), max_avail, None);
                self.pending_spell = None;
            }
            if !ch.created {
                let c = account::spell_counts(ch, cx.sheet);
                note(ui, format!("{} {} / {}", lang.tr("Free spells used"), c.spells + c.rituals + c.preparations, c.free));
            } else {
                note(ui, lang.tr_fmt("New spell: {0} karma", &[&career::spell_karma_cost(cx.engine, ch, "Spells")]));
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
        if ch.created {
            changed |= quicken_ui(ui, ch, lang, status);
        }
        changed
    }

    /// The `SelectSpell` checkboxes for the picked spell, then add it.
    fn spell_options_window(&mut self, ctx: &egui::Context, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let Some(p) = self.pending_spell.as_mut() else { return false };
        let rec = Record(&p.rec);
        let mut add = false;
        let mut cancel = false;
        let mut open = true;
        let lang = cx.lang;
        dialog::window(ctx, "spell_options", &lang.tr("Spell Options"), &mut open, egui::vec2(360.0, 0.0), false, |ui| {
            dialog::heading(ui, &rec.name());
            dialog::note(ui, format!("{} · {} · DV {}", rec.category(), rec.get("range"), rec.get("dv")));
            ui.add_space(4.0);
            let descriptors = rec.get("descriptor");
            let extended_area = descriptors.split(',').any(|d| d.trim().eq_ignore_ascii_case("Extended Area"));
            check_box(ui, &mut p.opts.limited, &lang.tr("Limited")).on_hover_text(lang.tr("−2 drain, needs a fetish or focus"));
            ui.add_enabled_ui(rec.category() == "Detection" && !extended_area, |ui| check_box(ui, &mut p.opts.extended, &lang.tr("Extended")))
                .inner
                .on_hover_text(lang.tr("Detection spells only: extended area, +2 drain"));
            check_box(ui, &mut p.opts.alchemical, &lang.tr("Alchemical Preparation"));
            check_box(ui, &mut p.opts.free_bonus, &lang.tr("Free")).on_hover_text(lang.tr("Costs no karma and does not count against free spells"));
            if ch.created && !p.opts.free_bonus {
                let category = if p.opts.alchemical {
                    "Preparations"
                } else if rec.category() == "Rituals" {
                    "Rituals"
                } else {
                    "Spells"
                };
                let cost = career::spell_karma_cost(cx.engine, ch, category);
                let t = RichText::new(lang.tr_fmt("Cost: {0} karma (you have {1})", &[&cost, &ch.karma]));
                let short = if ws_layout(ui) { crate::theme::ws(ui).error } else { ui.visuals().error_fg_color };
                ui.label(if cost > ch.karma { t.color(short) } else { t });
            }
            dialog::buttons(ui, |ui| {
                add = dialog::button(ui, &lang.tr("Add"), true).clicked();
                cancel = dialog::button(ui, &lang.tr("Cancel"), false).clicked();
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
        let cmd = Command::AddSpell { record: RecordRef::of(rec), answer: p.answer.clone(), options: p.opts.clone() };
        let learned = if ch.created { format!("Learned {name}") } else { format!("Added {name}") };
        report(status, ch.apply(cmd), |_| learned)
    }

    // ----- mentor spirit -----

    fn mentor_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let mentors: Vec<Element> = ch.items("mentorspirits", "mentorspirit").into_iter().cloned().collect();
        let pending = mentor::pending_mentor_qualities(ch, cx.store);
        if mentors.is_empty() && pending.is_empty() {
            return false;
        }
        let mut changed = false;
        let lang = cx.lang;
        section(ui, "mentor_ed", &lang.tr("Mentor Spirit"), true, |ui| {
            for m in &mentors {
                let guid = m.get("guid");
                let mtype = m.child_text("mentortype").unwrap_or_else(|| "MentorSpirit".into());
                ui.horizontal(|ui| {
                    if ws_layout(ui) {
                        ui.label(icons::icon(icons::SPARKLE, 14.0, crate::theme::ws(ui).accent));
                    }
                    strong(ui, m.get("name"));
                    weak(ui, format!("({})", if mtype == "Paragon" { lang.tr("Paragon") } else { lang.tr("Mentor Spirit") }));
                });
                if !m.get("advantage").is_empty() {
                    note(ui, format!("{} {}", lang.tr("Advantage:"), m.get("advantage")));
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
                    for (label, set, value, n) in [(lang.tr("Choice 1"), &set1, &mut entry.0, 1), (lang.tr("Choice 2"), &set2, &mut entry.1, 2)] {
                        if set.is_empty() {
                            continue;
                        }
                        note(ui, label);
                        crate::combo::Combo::from_id_salt(("mentor_choice", &guid, n))
                            .selected_text(if value.is_empty() { lang.tr("Choose…") } else { value.clone() })
                            .width(360.0)
                            .show_ui(ui, |ui| {
                                for c in set.iter() {
                                    crate::combo::selectable_value(ui, value, c.clone(), c);
                                }
                            });
                        ui.end_row();
                    }
                });
                let (c1, c2) = entry.clone();
                let saved_ok = m.get("extrachoice1") == c1 && m.get("extrachoice2") == c2;
                let complete = (set1.is_empty() || !c1.is_empty()) && (set2.is_empty() || !c2.is_empty());
                if ui.add_enabled_ui(!saved_ok && complete, |ui| action_button(ui, None, &lang.tr("Apply choices"), true)).inner.clicked() {
                    let cmd = Command::SetMentorChoices { mentor: guid.clone(), choice1: Some(c1).filter(|s| !s.is_empty()), choice2: Some(c2).filter(|s| !s.is_empty()) };
                    let r = ch.apply(cmd);
                    changed |= report(status, r, |_| format!("Mentor choices set for {}", m.get("name")));
                }
                ui.add_space(4.0);
            }
            for (qguid, qname, mtype) in &pending {
                ui.horizontal(|ui| {
                    let kind = if mtype == "Paragon" { lang.tr("Paragon") } else { lang.tr("Mentor Spirit") };
                    let text = lang.tr_fmt("{0} grants a {1} that is not chosen yet.", &[qname, &kind]);
                    if ws_layout(ui) {
                        let ws = crate::theme::ws(ui);
                        ui.label(icons::icon(icons::WARNING, 14.0, ws.warning));
                        ui.label(RichText::new(text).size(12.0).color(ws.warning));
                    } else {
                        ui.colored_label(crate::theme::warn(ui), text);
                    }
                    if action_button(ui, Some(icons::SPARKLE), &lang.tr("Choose…"), false).clicked() {
                        let picker = Picker::new(lang.tr_fmt("Choose a {0}", &[&kind]), mentor::data_file(mtype), "mentors", "mentor", cx.books());
                        self.mentor = Some((qguid.clone(), mtype.clone(), picker));
                    }
                });
            }
        });
        if let Some((qguid, mtype, picker)) = self.mentor.as_mut() {
            let check = Check { ch, sheet: cx.sheet, ignore_quality: None };
            match picker.show(ui.ctx(), cx.store, lang, Some(&check), &|_| Vec::new(), &|_| String::new()) {
                Pick::None => {}
                Pick::Cancel => self.mentor = None,
                Pick::Done(name, _) => {
                    let r = ch.apply(Command::ChooseMentor { quality: qguid.clone(), mentor_type: mtype.clone(), name: name.clone() });
                    changed |= report(status, r, |_| format!("{name} is now your mentor; pick its choices below"));
                    self.mentor = None;
                }
            }
        }
        changed
    }

    // ----- metamagics and echoes -----

    fn metamagic_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let echo = ch.is_technomancer() && !ch.is_magician();
        let (_what, file, container, item) = if echo {
            let (f, c, i) = metamagic::data_path("Echo");
            ("echo", f, c, i)
        } else {
            let (f, c, i) = metamagic::data_path("Metamagic");
            ("metamagic", f, c, i)
        };
        let grade = metamagic::current_grade(ch);
        let taken = ch.items("metamagics", "metamagic").iter().filter(|m| m.get_i32("grade").unwrap_or(0) > 0).count() as i32;
        let free = (grade - taken).max(0);
        // Career mode: the first metamagic at a grade is free, more cost
        // karma. As in Chummer (which adds to the grade selected in its
        // tree) the player picks the grade; the lowest grade with a free
        // slot, else the top one, is preselected (LB-30).
        if !self.metamagic_grade.is_some_and(|g| (1..=grade).contains(&g)) {
            self.metamagic_grade = Some(career::default_metamagic_grade(ch));
        }
        let career_grade = self.metamagic_grade.unwrap_or(grade);
        let career_cost = if ch.created && grade > 0 { career::metamagic_karma_cost(cx.engine, ch, career_grade) } else { 0 };
        let lang = cx.lang;
        let what_label = if echo { lang.tr("echo") } else { lang.tr("metamagic") };
        ui.horizontal(|ui| {
            if ch.created && grade > 0 {
                note(ui, lang.tr("Grade"));
                let label = |g: i32| {
                    let cost = career::metamagic_karma_cost(cx.engine, ch, g);
                    if cost == 0 { format!("{g} ({})", lang.tr("Free")) } else { format!("{g} ({})", lang.tr_fmt("{0} karma", &[&cost])) }
                };
                let mut pick = career_grade;
                crate::combo::Combo::from_id_salt("metamagic_grade").selected_text(label(pick)).width(120.0).show_ui(ui, |ui| {
                    for g in 1..=grade {
                        crate::combo::selectable_value(ui, &mut pick, g, label(g));
                    }
                });
                self.metamagic_grade = Some(pick);
            }
            let can = if ch.created { grade > 0 && ch.karma >= career_cost } else { free > 0 };
            let mut text = lang.tr_fmt("Add {0}…", &[&what_label]);
            if career_cost > 0 {
                text += &format!(" ({})", lang.tr_fmt("{0} karma", &[&career_cost]));
            }
            let b = ui.add_enabled_ui(can, |ui| add_button(ui, &text, true)).inner;
            if b.on_disabled_hover_text(if grade == 0 { lang.tr("Initiate or submerge first") } else { lang.tr("Every grade already has one") }).clicked() {
                self.metamagic = Some(Picker::new(lang.tr_fmt("Add {0}", &[&what_label]), file, container, item, cx.books()));
            }
            note(
                ui,
                if free == 1 {
                    lang.tr_fmt("Grade {0}: {1} free {2} slot", &[&grade, &free, &what_label])
                } else {
                    lang.tr_fmt("Grade {0}: {1} free {2} slots", &[&grade, &free, &what_label])
                },
            );
        });
        let mut changed = false;
        if let Some(picker) = self.metamagic.as_mut() {
            let store = cx.store;
            let chr: &Character = ch;
            let check = Check { ch: chr, sheet: cx.sheet, ignore_quality: None };
            let choices = |r: Record<'_>| magic::choices("metamagic", chr, store, r, &Purchase::default());
            match picker.show(ui.ctx(), store, lang, Some(&check), &choices, &|_| String::new()) {
                Pick::None => {}
                Pick::Cancel => self.metamagic = None,
                Pick::Done(name, answer) => {
                    let kind = if echo { "Echo" } else { "Metamagic" };
                    let r = ch.apply(Command::AddMetamagic { kind: kind.into(), name: name.clone(), answer, grade: self.metamagic_grade.unwrap_or(career_grade) });
                    changed |= report(status, r, |_| format!("Added {name}"));
                    self.metamagic = None;
                }
            }
        }
        changed
    }

    // ----- martial arts -----

    fn martial_arts_ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
        let arts: Vec<Element> = ch.items("martialarts", "martialart").into_iter().cloned().collect();
        let Ok(doc) = cx.store.doc("martialarts.xml") else { return false };
        let mut changed = false;
        let lang = cx.lang;
        for art in &arts {
            let guid = art.get("guid");
            let known: Vec<String> = art.child("martialarttechniques").map(|t| t.children_named("martialarttechnique").map(|x| x.get("name")).collect()).unwrap_or_default();
            let rec = mentor_like_find(&doc, "martialarts", "martialart", art);
            // `<alltechniques />` (One Trick Pony) teaches any technique.
            let names = match rec {
                Some(r) if r.el().child("alltechniques").is_some() => data::records(&doc, "techniques", "technique").into_iter().filter(|t| !t.hidden()).map(|t| t.name()).collect(),
                Some(r) => martialart::technique_names(r),
                None => Vec::new(),
            };
            let offered: Vec<String> = names.into_iter().filter(|t| !known.contains(t)).collect();
            ui.horizontal_wrapped(|ui| {
                strong(ui, art.get("name"));
                weak(ui, if known.is_empty() { lang.tr("no techniques") } else { known.join(", ") });
            });
            if offered.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                let pick = self.technique.entry(guid.clone()).or_default();
                if !offered.contains(pick) {
                    pick.clear();
                }
                crate::combo::Combo::from_id_salt(("technique", &guid))
                    .selected_text(if pick.is_empty() { lang.tr("Technique…") } else { pick.clone() })
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for t in &offered {
                            crate::combo::selectable_value(ui, pick, t.clone(), t);
                        }
                    });
                let cost = if ch.created { career::technique_karma_cost(cx.engine, ch, &guid) } else { 0 };
                let label = if ch.created { lang.tr_fmt("Learn ({0} karma)", &[&cost]) } else { lang.tr("Learn") };
                let can = !pick.is_empty() && (!ch.created || ch.karma >= cost);
                if ui.add_enabled_ui(can, |ui| action_button(ui, Some(icons::GRADUATION_CAP), &label, false)).inner.clicked() {
                    let t = pick.clone();
                    let r = ch.apply(Command::LearnTechnique { art: guid.clone(), technique: t.clone() });
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

fn powers_ui(ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
    let powers: Vec<Element> = ch.items("powers", "power").into_iter().cloned().collect();
    let second = cx.settings.is_some_and(|s| s.flag("mysadeptsecondmagattribute"));
    let mag = account::adept_mag(ch, cx.sheet, second);
    let (total, used) = match cx.settings {
        Some(s) => account::power_points_with(ch, cx.sheet, s),
        None => account::power_points(ch, cx.sheet),
    };
    let ignore = ch.flag("ignorerules");
    let mut changed = false;
    let lang = cx.lang;
    ui.horizontal(|ui| {
        let text = lang.tr_fmt("Power points used {0} of {1}", &[&fmt_pp(used), &fmt_pp(total)]);
        if ws_layout(ui) {
            let ws = crate::theme::ws(ui);
            ui.label(RichText::new(text).font(crate::workspace::widgets::bold(12.5)).color(if used > total + 1e-9 { ws.error } else { ws.text }));
        } else {
            let t = RichText::new(text).strong();
            ui.label(if used > total + 1e-9 { t.color(ui.visuals().error_fg_color) } else { t });
        }
        if ch.is_adept() && ch.is_magician() && !second {
            let pp = ch.doc.get_i32("magsplitadept").unwrap_or(0);
            note(ui, lang.tr_fmt("(mystic adept: {0} bought)", &[&pp]));
            if ch.created {
                let cost = career::power_point_karma_cost(cx.engine, ch);
                if ui.add_enabled_ui(ch.karma >= cost, |ui| action_button(ui, Some(icons::PLUS), &lang.tr_fmt("Buy power point ({0} karma)", &[&cost]), false)).inner.clicked() {
                    changed |= report(status, ch.apply(Command::BuyPowerPoint), |_| "Bought a power point".into());
                }
            }
        }
    });
    if powers.is_empty() {
        return changed;
    }
    grid(ui, "power_editor", &lang.tr_all(["Power", "Levels", "Free", "PP / level", "PP"]), &[0.0, 90.0, 50.0, 80.0, 60.0], |ui, w| {
        for (k, p) in powers.iter().enumerate() {
            let guid = p.get("guid");
            let extra = p.get("extra");
            grid_row(ui, w, ("power", k), |ui| {
                col(ui, w, 0, |ui| text_cell(ui, if extra.is_empty() { p.get("name") } else { format!("{} ({extra})", p.get("name")) }));
                let cost_now = power::power_point_cost(ch, p, mag);
                col(ui, w, 1, |ui| {
                    if p.get_bool("levels").unwrap_or(false) {
                        let max = power::total_maximum_levels(p, mag, ignore).max(1);
                        let mut r = p.get_i32("rating").unwrap_or(1);
                        let what = p.get("name");
                        let resp = int_input(ui, ("power_level", k), &mut r, 1, max, lang, &what).on_hover_text(lang.tr_fmt("Up to {0}", &[&max]));
                        if resp.changed() && r != p.get_i32("rating").unwrap_or(1) {
                            // Refused when the power points run out.
                            changed |= ch.run(Command::SetPowerRating { power: guid.clone(), rating: r }, status).is_some();
                        }
                    } else {
                        weak(ui, "—".to_owned());
                    }
                });
                let free = power::free_levels(ch, p, mag);
                col(ui, w, 2, |ui| text_cell(ui, if free > 0 { free.to_string() } else { String::new() }));
                col(ui, w, 3, |ui| text_cell(ui, p.get("pointsperlevel")));
                col(ui, w, 4, |ui| value_cell(ui, fmt_pp(cost_now)));
            });
        }
    });
    changed
}

/// Career mode: spend karma to quicken a spell (`cmdQuickenSpell_Click`).
fn quicken_ui(ui: &mut egui::Ui, ch: &mut Doc, lang: &Language, status: &mut Status) -> bool {
    let spells: Vec<(String, String)> = ch.items("spells", "spell").iter().map(|s| (s.get("guid"), s.get("name"))).collect();
    if spells.is_empty() {
        return false;
    }
    let (pick_id, karma_id) = (egui::Id::new("quicken_spell"), egui::Id::new("quicken_karma"));
    let mut pick: String = ui.data(|d| d.get_temp(pick_id)).unwrap_or_default();
    let mut karma: i32 = ui.data(|d| d.get_temp(karma_id)).unwrap_or(1);
    let mut changed = false;
    ui.horizontal(|ui| {
        let shown = spells.iter().find(|(g, _)| *g == pick).map_or_else(|| lang.tr("Spell…"), |(_, n)| n.clone());
        crate::combo::Combo::from_id_salt("quicken_spell_combo").selected_text(shown).width(220.0).show_ui(ui, |ui| {
            for (g, n) in &spells {
                crate::combo::selectable_value(ui, &mut pick, g.clone(), n);
            }
        });
        if ws_layout(ui) {
            let what = lang.tr("Karma");
            int_input(ui, "quicken_karma_stepper", &mut karma, 1, 999, lang, &what);
            note(ui, lang.tr("karma"));
        } else {
            ui.add(egui::DragValue::new(&mut karma).range(1..=999).suffix(" karma"));
        }
        let can = !pick.is_empty() && ch.karma >= karma;
        if ui.add_enabled_ui(can, |ui| action_button(ui, Some(icons::LIGHTNING), &lang.tr("Quicken"), false)).inner.clicked() {
            changed = report(status, ch.apply(Command::QuickenSpell { spell: pick.clone(), karma }), |_| lang.tr_fmt("Quickened ({0} karma)", &[&karma]));
        }
    });
    ui.data_mut(|d| {
        d.insert_temp(pick_id, pick);
        d.insert_temp(karma_id, karma);
    });
    changed
}

fn fmt_pp(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

// ----- spirits and sprites -----

fn spirits_ui(ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
    let lang = cx.lang;
    let spirits: Vec<Element> = ch.items("spirits", "spirit").into_iter().cloned().collect();
    if spirits.is_empty() {
        return false;
    }
    let mut changed = false;
    grid(ui, "spirit_editor", &lang.tr_all(["Spirit / sprite", "Force", "Services", "Bound", "Fettered"]), &[0.0, 90.0, 90.0, 110.0, 200.0], |ui, w| {
        for (k, s) in spirits.iter().enumerate() {
            let sprite = s.get("type") == "Sprite";
            let mut force = s.get_i32("force").unwrap_or(1);
            let mut services = s.get_i32("services").unwrap_or(0);
            let mut bound = s.get_bool("bound").unwrap_or(false);
            let mut fettered = s.get_bool("fettered").unwrap_or(false);
            let name = s.get("crittername");
            let mut c = false;
            grid_row(ui, w, ("spirit", k), |ui| {
                col(ui, w, 0, |ui| text_cell(ui, if name.is_empty() { s.get("name") } else { format!("{name} ({})", s.get("name")) }));
                col(ui, w, 1, |ui| c |= int_input(ui, ("force", k), &mut force, 1, 24, lang, &lang.tr("Force")).changed());
                col(ui, w, 2, |ui| c |= int_input(ui, ("services", k), &mut services, 0, 99, lang, &lang.tr("Services")).changed());
                col(ui, w, 3, |ui| c |= check_box(ui, &mut bound, &if sprite { lang.tr("Registered") } else { lang.tr("Bound") }).changed());
                col(ui, w, 4, |ui| {
                    if sprite {
                        ui.label("");
                    } else {
                        // A fettered spirit gains Banishing Resistance (SG p. 192).
                        let power = if spirit::gains_banishing_resistance(s) { lang.data_name("critterpowers.xml", "", spirit::FETTERED_POWER) } else { String::new() };
                        c |= check_box(ui, &mut fettered, &power).changed();
                    }
                });
            });
            if c {
                // Career mode: fettering costs karma (Spirit.Fettered).
                match ch.apply(Command::SetSpiritState { spirit: s.get("guid"), force, services, bound, fettered }) {
                    Ok(r) => {
                        if let Some(m) = r.message {
                            *status = Some((m, false));
                        }
                        changed |= r.changed;
                    }
                    Err(e) => *status = Some((e.reason, true)),
                }
            }
        }
    });
    changed
}

// ----- foci -----

fn foci_ui(ui: &mut egui::Ui, ch: &mut Doc, cx: &Ctx<'_>, status: &mut Status) -> bool {
    let foci: Vec<Element> = ch.items("gears", "gear").into_iter().filter(|g| matches!(g.get("category").as_str(), "Foci" | "Metamagic Foci")).cloned().collect();
    if foci.is_empty() || !ch.mag_enabled() {
        return false;
    }
    let bound: Vec<String> = ch.items("foci", "focus").iter().map(|f| f.get("gearid").to_ascii_lowercase()).collect();
    let mut changed = false;
    let lang = cx.lang;
    section(ui, "foci_ed", &lang.tr("Foci"), false, |ui| {
        let total: i32 = foci.iter().filter(|g| bound.contains(&g.get("guid").to_ascii_lowercase())).map(|g| g.get_i32("rating").unwrap_or(0)).sum();
        weak(ui, lang.tr_fmt("Bound force {0} (limit MAG × 5 = {1})", &[&total, &(cx.sheet.attr("MAG") * 5)]));
        grid(ui, "foci_editor", &lang.tr_all(["Focus", "Force", "Binding karma", "Bound"]), &[0.0, 60.0, 110.0, 60.0], |ui, w| {
            for (k, g) in foci.iter().enumerate() {
                let guid = g.get("guid");
                let extra = g.get("extra");
                let cost = career::focus_karma_cost(cx.engine, ch, g);
                let was = bound.contains(&guid.to_ascii_lowercase());
                let mut on = was;
                let mut resp = None;
                grid_row(ui, w, ("focus", k), |ui| {
                    col(ui, w, 0, |ui| text_cell(ui, if extra.is_empty() { g.get("name") } else { format!("{} ({extra})", g.get("name")) }));
                    col(ui, w, 1, |ui| value_cell(ui, g.get("rating")));
                    col(ui, w, 2, |ui| value_cell(ui, cost.to_string()));
                    col(ui, w, 3, |ui| resp = Some(ui.add_enabled_ui(was || !ch.created || ch.karma >= cost, |ui| check_box(ui, &mut on, "")).inner));
                });
                if resp.is_some_and(|r| r.changed()) && on != was {
                    if on {
                        match ch.apply(Command::BindFocus { gear: guid.clone() }) {
                            Ok(r) => {
                                if let Some(m) = r.message {
                                    *status = Some((m, false));
                                }
                                changed = true;
                            }
                            // Creation mode refuses silently, as before.
                            Err(e) if ch.created => *status = Some((e.reason, true)),
                            Err(_) => {}
                        }
                    } else {
                        changed |= ch.set(Command::UnbindFocus { gear: guid.clone() });
                    }
                }
            }
        });
    });
    changed
}

// ----- Workspace or Classic controls -----
//
// The editors draw the same controls in both layouts; these helpers pick
// the Workspace widgets (`crate::workspace::widgets`, Phosphor icons) when
// that layout is active, else the Classic egui ones.

/// Whether the Workspace layout is active.
fn ws_layout(ui: &egui::Ui) -> bool {
    crate::theme::current(ui.ctx()).workspace_layout()
}

/// A muted line of text.
fn note(ui: &mut egui::Ui, text: String) {
    if ws_layout(ui) {
        ui.label(RichText::new(text).size(12.0).color(crate::theme::ws(ui).muted));
    } else {
        ui.label(text);
    }
}

fn strong(ui: &mut egui::Ui, text: String) {
    if ws_layout(ui) {
        ui.label(RichText::new(text).font(widgets::bold(12.5)).color(crate::theme::ws(ui).text));
    } else {
        ui.strong(text);
    }
}

fn weak(ui: &mut egui::Ui, text: String) {
    if ws_layout(ui) {
        ui.label(RichText::new(text).size(12.0).color(crate::theme::ws(ui).muted));
    } else {
        ui.weak(text);
    }
}

/// An "Add …" button: a Workspace button with a plus, or "➕ text".
fn add_button(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    if ws_layout(ui) {
        widgets::button(ui, Some(icons::PLUS), text, if primary { Look::Secondary } else { Look::Ghost }, 24.0)
    } else {
        ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), text))
    }
}

/// An action button ("Learn", "Apply choices"); `primary` fills it in the
/// Workspace.
fn action_button(ui: &mut egui::Ui, glyph: Option<&str>, text: &str, primary: bool) -> egui::Response {
    if ws_layout(ui) {
        widgets::button(ui, glyph, text, if primary { Look::Primary } else { Look::Secondary }, 24.0)
    } else {
        ui.button(text)
    }
}

fn check_box(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    if ws_layout(ui) {
        widgets::check(ui, on, label)
    } else {
        ui.checkbox(on, label)
    }
}

/// A whole number: a stepper in the Workspace, a drag value in Classic.
fn int_input(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut i32, min: i32, max: i32, lang: &Language, what: &str) -> egui::Response {
    if ws_layout(ui) {
        widgets::num_stepper(ui, id, value, min, max, &lang.tr_fmt("Lower {0}", &[&what]), &lang.tr_fmt("Raise {0}", &[&what]))
    } else {
        ui.add(egui::DragValue::new(value).range(min..=max))
    }
}

/// A part of the page with a title: a heading in the Workspace (always
/// open), a collapsing header in Classic.
fn section(ui: &mut egui::Ui, id: &str, title: &str, default_open: bool, add: impl FnOnce(&mut egui::Ui)) {
    if ws_layout(ui) {
        let ws = crate::theme::ws(ui);
        ui.label(widgets::title(title, &ws));
        ui.add_space(2.0);
        ui.vertical(add);
        ui.add_space(6.0);
    } else {
        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(id).default_open(default_open).show(ui, add);
    }
}

/// A table of rows: a Workspace table (`spec`: column widths, 0 for the
/// one that takes the rest) or a Classic striped grid. `body` gets the
/// column widths (empty in Classic) for [`grid_row`] and [`col`].
fn grid(ui: &mut egui::Ui, id: &str, headers: &[String], spec: &[f32], body: impl FnOnce(&mut egui::Ui, &[f32])) {
    if ws_layout(ui) {
        let ws = crate::theme::ws(ui);
        widgets::table_frame(&ws).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let widths = widgets::table_columns(ui, spec);
            let caps: Vec<&str> = headers.iter().map(String::as_str).collect();
            widgets::table_header(ui, &caps, &widths);
            body(ui, &widths);
            ui.add_space(2.0);
        });
    } else {
        egui::Grid::new(id).striped(true).num_columns(headers.len()).spacing([14.0, 4.0]).show(ui, |ui| {
            for h in headers {
                ui.strong(h);
            }
            ui.end_row();
            body(ui, &[]);
        });
    }
}

/// Row height of the Workspace tables.
const ROW: f32 = 30.0;

/// One row of a [`grid`].
fn grid_row(ui: &mut egui::Ui, widths: &[f32], id: impl std::hash::Hash, add: impl FnOnce(&mut egui::Ui)) {
    if widths.is_empty() {
        add(ui);
        ui.end_row();
    } else {
        widgets::table_row(ui, id, false, ROW, add);
    }
}

/// Cell `k` of a [`grid_row`].
fn col(ui: &mut egui::Ui, widths: &[f32], k: usize, add: impl FnOnce(&mut egui::Ui)) {
    match widths.get(k) {
        Some(w) => widgets::cell(ui, *w, ROW, add),
        None => add(ui),
    }
}

/// Text in a cell (truncated in the Workspace).
fn text_cell(ui: &mut egui::Ui, text: String) {
    if ws_layout(ui) {
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(crate::theme::ws(ui).text)).truncate());
    } else {
        ui.label(text);
    }
}

/// A number in a cell (monospace accent in the Workspace).
fn value_cell(ui: &mut egui::Ui, text: String) {
    if ws_layout(ui) {
        ui.label(widgets::mono(text, 12.5, crate::theme::ws(ui).accent));
    } else {
        ui.label(text);
    }
}
