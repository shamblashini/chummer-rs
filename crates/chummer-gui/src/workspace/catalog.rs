//! The Workspace's inline catalog: "Add …" on an item page opens the
//! game data inside the page instead of Classic's selection dialog
//! (`select`). Filters on the left (kind, category, grade, the rating the
//! table shows, what fits the build, legality, books), the results with
//! a search field and sorting in the middle, and the selected record in
//! the inspector with its choices (rating, grade, quantity, where to
//! install it) and a preview: the purchase is applied to a copy of the
//! character and the copy's sheet compared with the character's (essence,
//! nuyen, initiative, attributes, armor, limits). "Add" runs the same
//! `Command::AddItem` as the dialog (career mode pays for it there), with
//! the same follow-up question for bonus selections. A small comparison
//! keeps up to three records side by side.
//!
//! A child module of `view` (declared there with `#[path]`).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chummer_core::bonus::Choice;
use chummer_core::calc::{self, Sheet};
use chummer_core::character::Character;
use chummer_core::chargen;
use chummer_core::command::{self, Command, Envelope, RecordRef};
use chummer_core::data::{self, Record};
use chummer_core::engine::Engine;
use chummer_core::expr::{self, Availability, Legality};
use chummer_core::format;
use chummer_core::items::{self, Kind, Purchase};
use chummer_core::lang::Language;
use chummer_core::requirements::Check;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::xml::Element;
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense};

use super::ws_items::{page_kinds, Page};
use super::{kind_noun, CharacterView};
use crate::pdf_ui::{self, Status};
use crate::select;
use crate::theme;
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

const FILTERS_WIDTH: f32 = 196.0;
const ROW_HEIGHT: f32 = 38.0;
/// Records kept for comparison.
const COMPARE_MAX: usize = 3;

/// One kind of record in the catalog (cyberware and bioware share the
/// Cyberware page).
struct Slot {
    kind: Kind,
    doc: Arc<Element>,
    on: bool,
    /// Cyberware and bioware grades.
    grades: Vec<Grade>,
}

#[derive(Debug, Clone)]
struct Grade {
    name: String,
    ess: f64,
    cost: f64,
    avail: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum Sort {
    #[default]
    Match,
    Name,
    Cost,
    Avail,
    Essence,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Match => "Best match",
            Sort::Name => "Name",
            Sort::Cost => "Cost, low first",
            Sort::Avail => "Availability, low first",
            Sort::Essence => "Essence, low first",
        }
    }
}

/// One result row, with its values at the rating the table shows.
#[derive(Debug, Clone)]
struct Row {
    slot: usize,
    index: usize,
    name: String,
    /// Category (and kind, when several are shown).
    sub: String,
    rating: i32,
    ess: Option<f64>,
    avail: Option<Availability>,
    cost: Option<f64>,
    /// The kind's own columns (damage, armor, handling…).
    extra: Vec<String>,
    source: Option<SourceRef>,
    /// Why it cannot be added now.
    why: Vec<String>,
    /// Availability above the creation limit.
    over: bool,
    /// Search rank: 0 name starts with the text, 1 contains it, 2 other.
    rank: u8,
}

/// The rows for the current filters.
#[derive(Default)]
struct Rows {
    list: Vec<Row>,
    /// Records of the shown kinds in the character's books.
    total: usize,
    /// Matching the search but hidden by a filter.
    hidden: usize,
    categories: BTreeMap<String, usize>,
    books: BTreeMap<String, usize>,
    kinds: Vec<usize>,
}

/// The selected record with the current choices, applied to a copy.
struct Preview {
    sheet: Sheet,
    /// Career: nuyen after; creation: nuyen left after.
    nuyen: f64,
    /// The answer the preview used for a bonus selection.
    assumed: Option<String>,
    /// Why Add would be refused (career: not enough nuyen); the sheet is
    /// then the purchase's effect as if it were free.
    refused: Option<String>,
}

/// A record kept for comparison.
#[derive(Debug, Clone)]
struct Compared {
    name: String,
    ess: String,
    init: String,
    cost: String,
}

/// The catalog's state: one per character, for one item page.
pub struct Catalog {
    /// The page it is open on.
    pub(super) page: Page,
    slots: Vec<Slot>,
    search: String,
    sort: Sort,
    /// Categories shown; empty = all.
    categories: BTreeSet<String>,
    books_off: BTreeSet<String>,
    /// Legal, restricted, forbidden shown.
    legality: [bool; 3],
    fit_avail: bool,
    fit_essence: bool,
    affordable: bool,
    requirements_met: bool,
    /// The rating the table shows (0 = each record's lowest).
    rating: i32,
    selected: Option<(usize, usize)>,
    purchase: Purchase,
    /// The parent was given (an item's "Add …"): no picker.
    parent_locked: bool,
    /// The bonus selection being asked before adding: (choices, answer).
    answer: Option<(Vec<Choice>, String)>,
    preview: Option<(u64, Result<Preview, String>)>,
    compare: Vec<Compared>,
    rows: Option<(u64, Rows)>,
    /// Give the search field focus next frame.
    focus_search: bool,
    /// Scroll the selected row into view (moved with the arrow keys).
    scroll: bool,
}

impl Catalog {
    fn new(page: Page, tags: &[&str], on: &str, store: &chummer_core::data::DataStore, parent: Option<String>) -> Option<Catalog> {
        let mut slots = Vec::new();
        for t in tags {
            let kind = *items::kind(t)?;
            let doc = store.doc(kind.file).ok()?;
            let grades = if matches!(kind.tag, "cyberware" | "bioware") { grades(&doc) } else { Vec::new() };
            slots.push(Slot { kind, doc, on: *t == on || (on == "cyberware" && kind.tag == "bioware"), grades });
        }
        if !slots.iter().any(|s| s.on) {
            slots.first_mut()?.on = true;
        }
        Some(Catalog {
            page,
            slots,
            search: String::new(),
            sort: Sort::Match,
            categories: BTreeSet::new(),
            books_off: BTreeSet::new(),
            legality: [true; 3],
            fit_avail: true,
            fit_essence: false,
            affordable: false,
            requirements_met: false,
            rating: 0,
            selected: None,
            parent_locked: parent.is_some(),
            purchase: Purchase { qty: 1.0, cost_multiplier: 1.0, parent, ..Default::default() },
            answer: None,
            preview: None,
            compare: Vec::new(),
            rows: None,
            focus_search: true,
            scroll: false,
        })
    }

    fn ware(&self) -> bool {
        self.slots.iter().any(|s| s.on && !s.grades.is_empty())
    }

    /// The selected record's slot and element.
    fn record(&self) -> Option<(&Slot, Record<'_>)> {
        let (s, i) = self.selected?;
        let slot = self.slots.get(s)?;
        let rec = *data::records(&slot.doc, slot.kind.data_container, slot.kind.data_item).get(i)?;
        Some((slot, rec))
    }

    fn grade(&self, slot: &Slot) -> Option<Grade> {
        if slot.grades.is_empty() {
            return None;
        }
        let want = self.purchase.grade.as_deref().unwrap_or("Standard");
        slot.grades.iter().find(|g| g.name == want).or_else(|| slot.grades.iter().find(|g| g.name == "Standard")).cloned()
    }

    fn clear_filters(&mut self) {
        self.categories.clear();
        self.books_off.clear();
        self.legality = [true; 3];
        self.fit_essence = false;
        self.affordable = false;
        self.requirements_met = false;
        self.rating = 0;
    }

    fn select(&mut self, slot: usize, index: usize, rating: i32) {
        if self.selected != Some((slot, index)) {
            self.selected = Some((slot, index));
            self.purchase.rating = rating;
            self.purchase.answer = None;
            self.answer = None;
        }
    }
}

/// `<grades>` of cyberware.xml/bioware.xml (as `select::grades`, with
/// their cost and availability).
fn grades(doc: &Element) -> Vec<Grade> {
    let num = |e: &Element, f: &str, d: f64| e.get(f).trim().parse::<f64>().unwrap_or(d);
    doc.child("grades")
        .map(|g| {
            g.children_named("grade")
                .filter(|e| e.child("hide").is_none() && e.get("name") != "None")
                .map(|e| Grade { name: e.get("name"), ess: num(e, "ess", 1.0), cost: num(e, "cost", 1.0), avail: num(e, "avail", 0.0) as i32 })
                .collect()
        })
        .unwrap_or_default()
}

/// `text` shortened with "…" to `width` points at the check box's size.
fn fit_text(ui: &egui::Ui, text: &str, width: f32) -> String {
    let w = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), FontId::proportional(12.5), Color32::PLACEHOLDER).size().x;
    if w(text) <= width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if w(&t) <= width {
            return t;
        }
    }
    "…".into()
}

/// A data expression ("Rating * 0.1") at `rating`.
fn eval_at(raw: &str, rating: i32, min: i32) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("Variable") || raw.contains("Parent") || raw.contains("Gear") || raw.contains('{') {
        return None;
    }
    let s = expr::fixed_values(raw, rating).replace("MinRating", &min.to_string()).replace("Rating", &rating.to_string());
    if expr::needs_evaluation(&s) {
        expr::evaluate_num(&s).ok()
    } else {
        expr::parse_plain(&s)
    }
}

/// Availability of a record at `rating`, with the grade's modifier.
fn avail_at(r: Record<'_>, rating: i32, grade: Option<&Grade>) -> Option<Availability> {
    let raw = r.get("avail");
    if raw.trim().is_empty() || raw.contains("Gear") || raw.contains('{') {
        return None;
    }
    let mut a = Availability::parse(&raw, rating, r.el().get_i32("minrating").unwrap_or(0), &expr::NoAttributes);
    if let Some(g) = grade {
        a.value += g.avail;
    }
    Some(a)
}

fn legality_index(a: Option<&Availability>) -> usize {
    match a.map(|a| a.legality) {
        Some(Legality::Restricted) => 1,
        Some(Legality::Forbidden) => 2,
        _ => 0,
    }
}

/// The rating a record is shown and bought at: `want` within its range,
/// or its lowest (0 when it has none).
fn rating_for(r: Record<'_>, want: i32) -> i32 {
    let max = select::rating_max(r);
    let lowest = select::rating_default(r);
    if max == 0 {
        0
    } else if want > 0 {
        want.clamp(lowest, max)
    } else {
        lowest
    }
}

fn kind_icon(tag: &str) -> &'static str {
    match tag {
        "cyberware" => icons::CPU,
        "bioware" => icons::DNA,
        "armor" => icons::T_SHIRT,
        "armormod" | "accessory" | "mod" => icons::WRENCH,
        "weapon" => icons::CROSSHAIR,
        "vehicle" => icons::CAR,
        "lifestyle" => icons::HOUSE_LINE,
        "drug" => icons::PILL,
        _ => icons::PACKAGE,
    }
}

fn hash_of(v: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

impl CharacterView {
    /// Open the inline catalog on `page` for kind `tag`; `parent` presets
    /// where it goes (an item's "Add …").
    pub(crate) fn ws_open_catalog(&mut self, page: Page, tag: &str, parent: Option<String>) {
        let container = super::ws_items::page_container(page);
        let kinds = page_kinds(container);
        let tags: Vec<&str> = if parent.is_some() || !kinds.contains(&tag) { vec![tag] } else { kinds.to_vec() };
        self.ws_gear.catalog = Catalog::new(page, &tags, tag, &self.store, parent);
    }

    pub(crate) fn ws_catalog_has_selection(&self) -> bool {
        self.ws_gear.catalog.as_ref().is_some_and(|c| c.selected.is_some())
    }

    pub(crate) fn ws_catalog_deselect(&mut self) {
        if let Some(c) = &mut self.ws_gear.catalog {
            c.selected = None;
        }
    }

    /// The nuyen the character can spend: nuyen left while creating.
    fn ws_nuyen_now(&self) -> f64 {
        self.budget.as_ref().map_or(self.doc.nuyen, |b| b.nuyen_left())
    }

    fn ws_max_avail(&self) -> Option<i32> {
        (!self.doc.created).then(|| self.settings.as_ref().map_or(12, |s| s.max_availability())).filter(|m| *m > 0)
    }

    /// Recompute the rows when the filters or the character changed.
    fn ws_catalog_rows(&mut self, lang: &Language) {
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let key = hash_of((
            &c.search,
            c.sort,
            &c.categories,
            &c.books_off,
            c.legality,
            (c.fit_avail, c.fit_essence, c.affordable, c.requirements_met),
            c.rating,
            c.slots.iter().map(|s| s.on).collect::<Vec<_>>(),
            &c.purchase.grade,
            self.doc.revision(),
            &lang.code,
        ));
        if c.rows.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let books = self.settings.as_ref().map(|s| s.books()).unwrap_or_default();
        let max_avail = self.ws_max_avail();
        let essence = self.sheet.essence;
        let nuyen = self.ws_nuyen_now();
        let check = Check { ch: &self.doc, sheet: &self.sheet, ignore_quality: None };
        let several = c.slots.iter().filter(|s| s.on).count() > 1;
        let needle = c.search.trim().to_lowercase();
        let mut out = Rows { kinds: vec![0; c.slots.len()], ..Default::default() };
        for (si, slot) in c.slots.iter().enumerate() {
            let grade = c.grade(slot);
            let cols: Vec<&str> = select::columns(slot.kind.tag).iter().map(|(_, f)| *f).filter(|f| !matches!(*f, "avail" | "cost" | "ess" | "rating")).collect();
            let kind_label = lang.tr(slot.kind.label);
            for (i, r) in data::records(&slot.doc, slot.kind.data_container, slot.kind.data_item).into_iter().enumerate() {
                if r.hidden() || (!books.is_empty() && !r.source().is_empty() && !books.contains(&r.source())) {
                    continue;
                }
                out.kinds[si] += 1;
                if !slot.on {
                    continue;
                }
                out.total += 1;
                let name = lang.data_name(slot.kind.file, &r.id(), &r.name());
                let category = r.category();
                let shown_category = if category.is_empty() { String::new() } else { lang.data_name(slot.kind.file, "", &category) };
                let rank = if needle.is_empty() {
                    0
                } else {
                    let lower = name.to_lowercase();
                    if lower.starts_with(&needle) {
                        0
                    } else if lower.contains(&needle) {
                        1
                    } else if crate::combo::matches(&name, &needle) || crate::combo::matches(&r.name(), &needle) || crate::combo::matches(&shown_category, &needle) {
                        2
                    } else {
                        continue;
                    }
                };
                let rating = rating_for(r, c.rating);
                let min = r.el().get_i32("minrating").unwrap_or(0);
                let avail = avail_at(r, rating, grade.as_ref());
                let cost = select::preview_cost(r, &Purchase { rating, qty: 1.0, cost_multiplier: 1.0, ..Default::default() }).map(|v| v * grade.as_ref().map_or(1.0, |g| g.cost));
                let ess = if slot.grades.is_empty() { None } else { eval_at(&r.get("ess"), rating, min).map(|v| v * grade.as_ref().map_or(1.0, |g| g.ess)) };
                let mut why = select::unavailable_reasons(slot.kind.tag, r, &check, 0);
                let over = match (max_avail, &avail) {
                    (Some(m), Some(a)) if !a.add_to_parent && a.value > m => Some(lang.tr_fmt("Avail {0} over limit {1}", &[&a.to_string(), &m])),
                    _ => None,
                };
                // Filters (counted as hidden).
                let mut hide = !c.legality[legality_index(avail.as_ref())];
                hide |= c.fit_avail && over.is_some();
                hide |= c.fit_essence && ess.is_some_and(|e| e > essence);
                hide |= c.affordable && cost.is_some_and(|v| v > nuyen);
                hide |= c.requirements_met && !why.is_empty();
                hide |= c.books_off.contains(&r.source());
                if hide {
                    out.hidden += 1;
                    continue;
                }
                *out.categories.entry(category.clone()).or_default() += 1;
                *out.books.entry(r.source()).or_default() += 1;
                if !c.categories.is_empty() && !c.categories.contains(&category) {
                    out.hidden += 1;
                    continue;
                }
                let is_over = over.is_some();
                if let Some(o) = over {
                    why.push(o);
                }
                let sub = if several { [kind_label.clone(), shown_category].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ") } else { shown_category };
                let extra = cols.iter().map(|f| r.get(f)).collect();
                out.list.push(Row { slot: si, index: i, name, sub, rating, ess, avail, cost, extra, source: SourceRef::of(r.el()), why, over: is_over, rank });
            }
        }
        let key_f = |v: Option<f64>| v.unwrap_or(f64::MAX);
        match c.sort {
            Sort::Match => out.list.sort_by_key(|r| r.rank),
            Sort::Name => out.list.sort_by_key(|a| a.name.to_lowercase()),
            Sort::Cost => out.list.sort_by(|a, b| key_f(a.cost).total_cmp(&key_f(b.cost))),
            Sort::Avail => out.list.sort_by_key(|r| r.avail.as_ref().map_or(i32::MAX, |a| a.value)),
            Sort::Essence => out.list.sort_by(|a, b| key_f(a.ess).total_cmp(&key_f(b.ess))),
        }
        if let Some(c) = self.ws_gear.catalog.as_mut() {
            c.rows = Some((key, out));
        }
    }

    /// The catalog page: filters, the search and the results. Returns
    /// true if the character changed.
    pub(crate) fn ws_catalog_page(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        self.ws_catalog_rows(lang);
        let ws = theme::ws(ui);
        let mut changed = false;
        let mut close = false;
        let mut add: Option<bool> = None;
        // Keys, before the search field takes them: arrows move, Enter
        // adds (Shift+Enter stays), Esc closes.
        let search_id = egui::Id::new("ws_catalog_search");
        let free = !ui.ctx().wants_keyboard_input() || ui.ctx().memory(|m| m.has_focus(search_id));
        if free && !ui.ctx().is_popup_open() {
            use egui::{Key, Modifiers};
            let (down, up, enter_stay, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::SHIFT, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if down || up {
                self.ws_catalog_step(if down { 1 } else { -1 });
            }
            if (enter || enter_stay) && self.ws_catalog_has_selection() && self.ws_gear.catalog.as_ref().is_some_and(|c| c.answer.is_none()) {
                add = Some(enter);
            }
            if esc && self.ws_gear.catalog.as_ref().is_some_and(|c| c.answer.is_none()) {
                close = true;
            }
        }
        egui::SidePanel::left("ws_catalog_filters")
            .exact_width(FILTERS_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(ws.chrome).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(12)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("ws_catalog_filters").auto_shrink(false).show(ui, |ui| {
                    // Room for the scroll bar.
                    ui.set_max_width(ui.available_width() - 10.0);
                    self.ws_catalog_filters(ui, lang)
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 0 })).show_inside(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let Some(c) = self.ws_gear.catalog.as_ref() else { return };
            let kinds: Vec<String> = c.slots.iter().filter(|s| s.on).map(|s| kind_noun(lang, s.kind.label)).collect();
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.tr_fmt("Add {0}", &[&kinds.join(" / ")])).font(widgets::bold(15.0)).color(ws.text));
                if let Some(p) = c.purchase.parent.as_deref().and_then(|g| items::edit::find(&self.doc, g)) {
                    ui.label(RichText::new(lang.tr_fmt("in {0}", &[&p.get("name")])).size(13.0).color(ws.muted));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, Some(icons::CHECK), &lang.tr("Done"), Look::Secondary, 26.0).on_hover_text(lang.tr("Back to the list (Esc)")).clicked() {
                        close = true;
                    }
                });
            });
            self.ws_catalog_installed(ui, lang);
            self.ws_catalog_search(ui, lang);
            if let Some(a) = self.ws_catalog_results(ui, lang, pdfs, status) {
                add = Some(a);
            }
        });
        if let Some(and_close) = add {
            changed |= self.ws_catalog_add(engine, lang, status, and_close);
        }
        if close {
            self.ws_gear.catalog = None;
        }
        changed
    }

    /// Move the selection `by` rows.
    fn ws_catalog_step(&mut self, by: i32) {
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let Some((_, rows)) = &c.rows else { return };
        if rows.list.is_empty() {
            return;
        }
        let at = c.selected.and_then(|s| rows.list.iter().position(|r| (r.slot, r.index) == s));
        let next = match at {
            Some(i) => (i as i32 + by).clamp(0, rows.list.len() as i32 - 1) as usize,
            None => 0,
        };
        let r = &rows.list[next];
        let (slot, index, rating) = (r.slot, r.index, r.rating);
        c.select(slot, index, rating);
        c.scroll = true;
    }

    /// The filters column.
    fn ws_catalog_filters(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let essence = format::essence(self.sheet.essence, self.rules.essence_decimals);
        let max_avail = self.ws_max_avail();
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        // Out while the filters change `c`; back at the end.
        let Some((key, rows)) = c.rows.take() else { return };
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.label(icons::icon(icons::FUNNEL, 14.0, ws.muted));
            ui.label(widgets::title(&lang.tr("Filters"), &ws));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, None, &lang.tr("Clear"), Look::Ghost, 22.0).clicked() {
                    c.clear_filters();
                }
            });
        });
        let heading = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(10.0);
            ui.label(widgets::overline(t, &ws));
            ui.add_space(2.0);
        };
        let count_check = |ui: &mut egui::Ui, on: &mut bool, label: &str, count: Option<usize>| -> bool {
            let mut changed = false;
            ui.horizontal(|ui| {
                let room = ui.available_width() - 14.0 - 7.0 - if count.is_some() { 30.0 } else { 0.0 };
                changed = widgets::check(ui, on, &fit_text(ui, label, room)).on_hover_text(label).changed();
                if let Some(n) = count {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(n.to_string()).size(11.0).color(ws.muted));
                    });
                }
            });
            changed
        };
        if c.slots.len() > 1 {
            heading(ui, &lang.tr("Kind"));
            for i in 0..c.slots.len() {
                let mut on = c.slots[i].on;
                let label = lang.tr(c.slots[i].kind.label);
                if count_check(ui, &mut on, &label, rows.kinds.get(i).copied()) && (on || c.slots.iter().filter(|s| s.on).count() > 1) {
                    c.slots[i].on = on;
                    c.selected = None;
                }
            }
        }
        if !rows.categories.is_empty() {
            heading(ui, &lang.tr("Category"));
            let file = c.slots.iter().find(|s| s.on).map_or("", |s| s.kind.file);
            for (cat, n) in &rows.categories {
                let mut on = c.categories.contains(cat);
                let label = if cat.is_empty() { lang.tr("Other") } else { lang.data_name(file, "", cat) };
                if count_check(ui, &mut on, &label, Some(*n)) {
                    if on {
                        c.categories.insert(cat.clone());
                    } else {
                        c.categories.remove(cat);
                    }
                }
            }
        }
        if c.ware() {
            heading(ui, &lang.tr("Grade"));
            let names: Vec<(String, f64, i32)> = {
                let mut seen = Vec::new();
                for s in c.slots.iter().filter(|s| s.on) {
                    for g in &s.grades {
                        if !seen.iter().any(|(n, _, _): &(String, f64, i32)| *n == g.name) {
                            seen.push((g.name.clone(), g.ess, g.avail));
                        }
                    }
                }
                seen
            };
            let cur = c.purchase.grade.clone().unwrap_or_else(|| "Standard".into());
            crate::combo::Combo::from_id_salt("ws_catalog_grade").selected_text(cur.clone()).width(FILTERS_WIDTH - 24.0).show_ui(ui, |ui| {
                for (g, ess, _) in &names {
                    if crate::combo::selectable_label(ui, cur == *g, format!("{g} ×{}", chummer_core::improvement::fmt_num(*ess))).clicked() {
                        c.purchase.grade = Some(g.clone());
                    }
                }
            });
            if let Some((_, _, a)) = names.iter().find(|(n, _, _)| *n == cur).filter(|(_, _, a)| *a != 0) {
                ui.label(RichText::new(lang.tr_fmt("Availability {0}", &[&format!("{a:+}")])).size(11.0).color(ws.muted));
            }
        }
        heading(ui, &lang.tr("Rating"));
        let shown = if c.rating == 0 { lang.tr("Lowest") } else { c.rating.to_string() };
        crate::combo::Combo::from_id_salt("ws_catalog_rating").selected_text(shown).width(FILTERS_WIDTH - 24.0).show_ui(ui, |ui| {
            crate::combo::selectable_value(ui, &mut c.rating, 0, lang.tr("Lowest"));
            for r in 1..=12 {
                crate::combo::selectable_value(ui, &mut c.rating, r, r.to_string());
            }
        });
        ui.label(RichText::new(lang.tr("Values in the table are at this rating.")).size(11.0).color(ws.muted));
        heading(ui, &lang.tr("Fits this build"));
        if let Some(m) = max_avail {
            count_check(ui, &mut c.fit_avail, &lang.tr_fmt("Availability ≤ {0}", &[&m]), None);
        }
        if c.ware() {
            count_check(ui, &mut c.fit_essence, &lang.tr_fmt("Fits Essence {0}", &[&essence]), None);
        }
        count_check(ui, &mut c.affordable, &lang.tr("Affordable"), None);
        count_check(ui, &mut c.requirements_met, &lang.tr("Requirements met"), None);
        heading(ui, &lang.tr("Legality"));
        for (i, l) in ["Legal", "Restricted", "Forbidden"].into_iter().enumerate() {
            count_check(ui, &mut c.legality[i], &lang.tr(l), None);
        }
        if rows.books.len() > 1 || !c.books_off.is_empty() {
            heading(ui, &lang.tr("Books"));
            let mut books: Vec<(&String, &usize)> = rows.books.iter().collect();
            books.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let off: Vec<String> = c.books_off.iter().filter(|b| !rows.books.contains_key(*b)).cloned().collect();
            for (b, n) in books.into_iter().map(|(b, n)| (b.clone(), Some(*n))).chain(off.into_iter().map(|b| (b, None))) {
                let mut on = !c.books_off.contains(&b);
                if count_check(ui, &mut on, &b, n) {
                    if on {
                        c.books_off.remove(&b);
                    } else {
                        c.books_off.insert(b);
                    }
                }
            }
        }
        c.rows = Some((key, rows));
    }
}

/// Apply the purchase to a copy of the character and compute its sheet
/// (and, while creating, the nuyen left).
fn preview(ch: &Character, settings: Option<&chummer_core::settings::CharacterSettings>, engine: &Engine, tag: &str, rec: &Element, p: &Purchase) -> Result<Preview, String> {
    let mut copy = ch.clone();
    let store = engine.store_for_character(&copy);
    let mut p = p.clone();
    let mut assumed = None;
    if p.answer.is_none() {
        // Preview with the first choice a bonus selection offers.
        let choices = items::choices(tag, &copy, &store, Record(rec), &p);
        if let Some(first) = choices.first().and_then(|c| c.options.first()) {
            p.answer = Some(first.clone());
            assumed = Some(first.clone());
        }
    }
    let cmd = |p: Purchase| Command::AddItem { tag: tag.to_owned(), record: RecordRef::of(Record(rec)), purchase: p };
    let mut refused = None;
    if let Err(e) = command::apply(&mut copy, engine, &Envelope::new(cmd(p.clone()), 0, 0, "")) {
        // Show what it would do anyway (a refused career purchase).
        let free = Purchase { free: true, ..p };
        command::apply(&mut copy, engine, &Envelope::new(cmd(free), 0, 0, "")).map_err(|_| e.reason.clone())?;
        refused = Some(e.reason);
    }
    let rules = engine.rules_for(&copy);
    let sheet = calc::compute(&copy, &rules, Some(&store), Some(&engine.catalog));
    let nuyen = match (settings, copy.created) {
        (Some(st), false) => chargen::budget_with(&copy, &sheet, &rules, st, Some(&store)).nuyen_left(),
        _ => copy.nuyen,
    };
    Ok(Preview { sheet, nuyen, assumed, refused })
}

/// Attributes the preview compares.
const ATTRIBUTES: &[&str] = &["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES", "DEP"];

impl CharacterView {
    /// The page's items above the search ("Installed": name, rating,
    /// grade, essence).
    fn ws_catalog_installed(&self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let container = super::ws_items::page_container(c.page);
        let Some(sec) = chummer_core::sections::EQUIPMENT.iter().find(|s| s.container == container) else { return };
        let list = self.doc.items(sec.container, sec.item);
        if list.is_empty() {
            return;
        }
        let ware = container == "cyberwares";
        let note = if ware {
            format!("{} · {} {}", lang.tr_fmt("{0} items", &[&list.len()]), lang.tr("Essence"), format::essence(self.sheet.essence, self.rules.essence_decimals))
        } else {
            lang.tr_fmt("{0} items", &[&list.len()])
        };
        ui.horizontal(|ui| {
            ui.label(widgets::title(&lang.tr(if ware { "Installed" } else { sec.label }), &ws));
            ui.label(RichText::new(note).size(11.5).color(ws.muted));
        });
        let width = ui.available_width();
        let fit = (((width + 8.0) / (170.0 + 8.0)) as usize).clamp(1, 4);
        let per = fit.min(list.len()).max(1);
        // Room for "+N more" when some do not fit.
        let room = if list.len() > per { 90.0 } else { 0.0 };
        let card_w = ((width - room - (per as f32 - 1.0) * 8.0) / per as f32).floor() - 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for el in list.iter().take(per) {
                let name = super::display_name(sec, el, lang);
                let mut bits = Vec::new();
                if let Some(r) = el.get_i32("rating").filter(|r| *r > 0) {
                    bits.push(format!("R{r}"));
                }
                let grade = el.get("grade");
                if !grade.is_empty() {
                    bits.push(grade);
                }
                let value = if ware { format!("{} {}", super::cell(el, "ess"), lang.tr("Ess")) } else { super::cell(el, "cost") };
                egui::Frame::new().fill(ws.raised).stroke(egui::Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.set_width(card_w - 20.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(RichText::new(&name).size(12.5).color(ws.text)).truncate());
                    bits.push(value.trim().to_owned());
                    ui.add(egui::Label::new(RichText::new(bits.join(" · ")).size(11.0).color(ws.muted)).truncate());
                });
            }
            if list.len() > per {
                ui.label(RichText::new(lang.tr_fmt("+{0} more", &[&(list.len() - per)])).size(11.5).color(ws.muted));
            }
        });
    }

    /// The search field and the sort order.
    fn ws_catalog_search(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let kinds: Vec<String> = c.slots.iter().filter(|s| s.on).map(|s| kind_noun(lang, s.kind.label)).collect();
        let ware = c.ware();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let sort_w = 180.0;
            // The combo adds its padding and arrow to `sort_w`.
            let field_w = (ui.available_width() - sort_w - 40.0).max(160.0);
            let field = egui::Frame::new().fill(ws.well).stroke(egui::Stroke::new(1.0_f32, ws.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                ui.set_width(field_w - 18.0);
                ui.horizontal(|ui| {
                    ui.label(icons::icon(icons::MAGNIFYING_GLASS, 14.0, ws.muted));
                    let r = ui.add(egui::TextEdit::singleline(&mut c.search).id(egui::Id::new("ws_catalog_search")).frame(false).hint_text(lang.tr_fmt("Search {0}", &[&kinds.join(" / ")])).desired_width(f32::INFINITY));
                    if c.focus_search {
                        r.request_focus();
                        c.focus_search = false;
                    }
                });
            });
            // A click anywhere on the field (the icon, the margin) types in it.
            if ui.interact(field.response.rect, egui::Id::new("ws_catalog_search_frame"), Sense::click()).clicked() {
                ui.memory_mut(|m| m.request_focus(egui::Id::new("ws_catalog_search")));
            }
            crate::combo::Combo::from_id_salt("ws_catalog_sort").selected_text(lang.tr(c.sort.label())).width(sort_w).show_ui(ui, |ui| {
                for s in [Sort::Match, Sort::Name, Sort::Cost, Sort::Avail, Sort::Essence] {
                    if s == Sort::Essence && !ware {
                        continue;
                    }
                    crate::combo::selectable_value(ui, &mut c.sort, s, lang.tr(s.label()));
                }
            });
        });
        if let Some((_, rows)) = &c.rows {
            ui.horizontal(|ui| {
                let mut line = lang.tr_fmt("{0} of {1}", &[&rows.list.len(), &rows.total]);
                if !c.search.trim().is_empty() {
                    line = format!("{line} {}", lang.tr_fmt("match “{0}”", &[&c.search.trim()]));
                }
                if rows.hidden > 0 {
                    line = format!("{line} · {}", lang.tr_fmt("{0} hidden by filters", &[&rows.hidden]));
                }
                ui.label(RichText::new(line).size(11.5).color(ws.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(lang.tr("Enter adds · Shift+Enter adds and stays")).size(11.0).color(ws.muted));
                });
            });
        }
    }

    /// The results table. Returns `Some(close)` when a row asks to be
    /// added (double click: add and stay).
    fn ws_catalog_results(&mut self, ui: &mut egui::Ui, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<bool> {
        let ws = theme::ws(ui);
        let c = self.ws_gear.catalog.as_mut()?;
        let (key, rows) = c.rows.take()?;
        let first = c.slots.iter().find(|s| s.on).map(|s| s.kind.tag).unwrap_or("gear");
        let extra: Vec<&str> = select::columns(first).iter().filter(|(_, f)| !matches!(*f, "avail" | "cost" | "ess" | "rating")).map(|(h, _)| *h).collect();
        let ware = c.ware();
        // Fixed columns from the right; the name takes the rest.
        let mut cols: Vec<(String, f32)> = vec![(lang.tr("Rating"), 46.0)];
        cols.extend(extra.iter().map(|h| (lang.tr(h), 56.0)));
        if ware {
            cols.push((lang.tr("Ess"), 48.0));
        }
        cols.push((lang.tr("Avail"), 48.0));
        cols.push((lang.tr("Cost"), 86.0));
        cols.push((lang.tr("Source"), 64.0));
        let gap = 10.0;
        let fixed: f32 = cols.iter().map(|(_, w)| w + gap).sum();
        let mut out = None;
        widgets::card_frame(&ws).inner_margin(egui::Margin::same(0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            let name_w = (width - 20.0 - 18.0 - gap - fixed).max(120.0);
            let xs = |rect: egui::Rect| {
                let mut x = rect.left() + 10.0 + 18.0 + gap + name_w + gap;
                cols.iter().map(|(_, w)| {
                    let r = (x, *w);
                    x += w + gap;
                    r
                }).collect::<Vec<_>>()
            };
            // Header.
            let (head, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), Sense::hover());
            let hp = ui.painter();
            let small = FontId::proportional(10.5);
            hp.text(egui::pos2(head.left() + 10.0 + 18.0 + gap, head.center().y), egui::Align2::LEFT_CENTER, lang.tr("Name"), small.clone(), ws.muted);
            for ((x, w), (h, _)) in xs(head).into_iter().zip(&cols) {
                let right = h == &lang.tr("Cost");
                let pos = if right { egui::pos2(x + w, head.center().y) } else { egui::pos2(x, head.center().y) };
                hp.text(pos, if right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER }, h, small.clone(), ws.muted);
            }
            hp.rect_filled(egui::Rect::from_min_size(egui::pos2(head.left(), head.bottom() - 1.0), egui::vec2(width, 1.0)), 0.0, ws.divider);
            if rows.list.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(lang.tr("Nothing matches. Clear a filter or change the search.")).size(12.0).color(ws.muted));
                });
                ui.add_space(8.0);
                return;
            }
            let selected = c.selected;
            let scroll_to = selected.and_then(|s| rows.list.iter().position(|r| (r.slot, r.index) == s));
            let mut pick = None;
            let mut area = egui::ScrollArea::vertical().id_salt("ws_catalog_rows").auto_shrink([false, true]).max_height(ui.available_height() - 4.0);
            if std::mem::take(&mut c.scroll) {
                if let Some(i) = scroll_to {
                    area = area.vertical_scroll_offset((i as f32 - 3.0).max(0.0) * ROW_HEIGHT);
                }
            }
            area.show_rows(ui, ROW_HEIGHT, rows.list.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in range {
                    let r = &rows.list[i];
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
                    let on = selected == Some((r.slot, r.index));
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let p = ui.painter();
                    if on {
                        p.rect_filled(rect, 0.0, ws.selection);
                        p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, ws.primary);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 0.0, ws.hover);
                    }
                    let blocked = !r.why.is_empty();
                    let ink = if blocked { ws.muted } else { ws.text };
                    let tag = c.slots.get(r.slot).map_or("gear", |s| s.kind.tag);
                    icons::paint(p, egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0, rect.center().y - 9.0), egui::vec2(18.0, 18.0)), kind_icon(tag), 14.0, if on { ws.accent } else { ws.muted });
                    let nx = rect.left() + 10.0 + 18.0 + gap;
                    let clip = egui::Rect::from_min_max(egui::pos2(nx, rect.top()), egui::pos2(nx + name_w, rect.bottom()));
                    let pc = p.with_clip_rect(clip);
                    pc.text(egui::pos2(nx, rect.top() + 11.0), egui::Align2::LEFT_CENTER, &r.name, FontId::proportional(12.5), ink);
                    let (sub, sub_color) = match r.why.first() {
                        Some(w) => (format!("{} {w}", icons::WARNING), ws.warning),
                        None => (r.sub.clone(), ws.muted),
                    };
                    pc.text(egui::pos2(nx, rect.top() + 27.0), egui::Align2::LEFT_CENTER, sub, FontId::proportional(11.0), sub_color);
                    let mono = |v: String| (v, FontId::monospace(12.0));
                    let mut cells: Vec<(String, FontId, Color32)> = Vec::new();
                    let (t, f) = mono(if r.rating > 0 { r.rating.to_string() } else { "—".into() });
                    cells.push((t, f, ink));
                    for k in 0..extra.len() {
                        let (t, f) = mono(r.extra.get(k).cloned().unwrap_or_default());
                        cells.push((t, f, ink));
                    }
                    if ware {
                        let (t, f) = mono(r.ess.map_or("—".into(), |e| format!("{e:.2}")));
                        cells.push((t, f, ink));
                    }
                    let over = r.over || r.avail.as_ref().is_some_and(|a| a.legality == Legality::Forbidden);
                    let (t, f) = mono(r.avail.as_ref().map_or("—".into(), |a| a.to_string()));
                    cells.push((t, f, if over { ws.warning } else { ink }));
                    let (t, f) = mono(r.cost.map_or("—".into(), format::nuyen));
                    cells.push((t, f, ink));
                    cells.push((r.source.as_ref().map_or(String::new(), |s| format!("{} {}", s.book, s.page)), FontId::proportional(11.0), ws.muted));
                    let n = cells.len();
                    for (k, ((x, w), (t, f, color))) in xs(rect).into_iter().zip(cells).enumerate() {
                        let right = k == n - 2;
                        let cell = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + w, rect.bottom()));
                        let pos = if right { egui::pos2(x + w, rect.center().y) } else { egui::pos2(x, rect.center().y) };
                        p.with_clip_rect(cell.expand2(egui::vec2(2.0, 0.0))).text(pos, if right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER }, t, f, color);
                    }
                    // The source opens the PDF.
                    let (sx, sw) = *xs(rect).last().unwrap_or(&(0.0, 0.0));
                    let src_rect = egui::Rect::from_min_max(egui::pos2(sx, rect.top()), egui::pos2(sx + sw, rect.bottom()));
                    let over_src = resp.hover_pos().is_some_and(|p| src_rect.contains(p)) && r.source.is_some();
                    let resp = if over_src {
                        resp.on_hover_text(r.source.as_ref().map(|s| s.to_string()).unwrap_or_default())
                    } else if blocked {
                        resp.on_hover_text(r.why.join("\n"))
                    } else {
                        resp
                    };
                    if resp.clicked() {
                        if over_src {
                            if let Some(s) = &r.source {
                                pdf_ui::open(pdfs, s, status);
                            }
                        }
                        pick = Some((r.slot, r.index, r.rating, false));
                    }
                    if resp.double_clicked() && !blocked {
                        pick = Some((r.slot, r.index, r.rating, true));
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            });
            if let Some((s, i, rating, add)) = pick {
                c.select(s, i, rating);
                if add {
                    out = Some(false);
                }
            }
        });
        c.rows = Some((key, rows));
        out
    }

    /// Add the selected record (`Command::AddItem`, as the selection
    /// dialog does). A bonus selection is asked first, in the inspector.
    /// `close` closes the catalog afterwards. Returns true if the
    /// character changed.
    fn ws_catalog_add(&mut self, _engine: &Arc<Engine>, lang: &Language, status: &mut Status, close: bool) -> bool {
        let store = self.store.clone();
        let Some(c) = self.ws_gear.catalog.as_mut() else { return false };
        let Some((slot, r)) = c.record() else { return false };
        let tag = slot.kind.tag;
        let rec = r.el().clone();
        if select::parent_of(tag).is_some() && c.purchase.parent.is_none() {
            *status = Some((lang.tr("Choose where to install it"), true));
            return false;
        }
        let mut purchase = c.purchase.clone();
        match &c.answer {
            Some((_, text)) if text.trim().is_empty() => return false,
            Some((_, text)) => purchase.answer = Some(text.trim().to_owned()),
            None => {
                let choices = items::choices(tag, &self.doc, &store, Record(&rec), &purchase);
                if !choices.is_empty() {
                    let answer = if choices[0].options.len() == 1 { choices[0].options[0].clone() } else { String::new() };
                    c.answer = Some((choices, answer));
                    return false;
                }
            }
        }
        match self.doc.apply(Command::AddItem { tag: tag.to_owned(), record: RecordRef::of(Record(&rec)), purchase }) {
            Ok(rep) => {
                *status = rep.message.map(|m| (m, false));
                c.answer = None;
                c.purchase.answer = None;
                if close {
                    self.ws_gear.catalog = None;
                }
                true
            }
            Err(e) => {
                *status = Some((e.reason, true));
                false
            }
        }
    }

    /// Compute the preview of the selected record when it changed.
    fn ws_catalog_preview(&mut self, engine: &Engine) {
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let Some((slot, r)) = c.record() else { return };
        let p = &c.purchase;
        let key = hash_of((slot.kind.tag, c.selected, p.rating, &p.grade, p.qty.to_bits(), &p.parent, &p.answer, self.doc.revision()));
        if c.preview.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let result = preview(&self.doc, self.settings.as_ref(), engine, slot.kind.tag, r.el(), p);
        if let Some(c) = self.ws_gear.catalog.as_mut() {
            c.preview = Some((key, result));
        }
    }

    /// The inspector for the catalog's selected record: choices, the
    /// before → after preview, the checks, Add and the comparison.
    /// Returns true if the character changed.
    pub(crate) fn ws_catalog_inspector(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        self.ws_catalog_preview(engine);
        let ws = theme::ws(ui);
        let max_avail = self.ws_max_avail();
        let nuyen_now = self.ws_nuyen_now();
        let creating = !self.doc.created;
        let decimals = self.rules.essence_decimals;
        let check_failures = {
            let Some(c) = self.ws_gear.catalog.as_ref() else { return false };
            let Some((slot, r)) = c.record() else { return false };
            let check = Check { ch: &self.doc, sheet: &self.sheet, ignore_quality: None };
            select::unavailable_reasons(slot.kind.tag, r, &check, 0)
        };
        let parents: Vec<(String, String)> = {
            let Some(c) = self.ws_gear.catalog.as_ref() else { return false };
            let tag = c.record().map_or("", |(s, _)| s.kind.tag);
            match select::parent_of(tag).or(select::optional_parent(tag)) {
                Some((container, ptag)) if !c.parent_locked => self.doc.items(container, ptag).iter().map(|e| (e.get("guid"), e.get("name"))).collect(),
                _ => Vec::new(),
            }
        };
        let locked_parent = self.ws_gear.catalog.as_ref().and_then(|c| c.purchase.parent.clone()).and_then(|g| items::edit::find(&self.doc, &g).map(|e| e.get("name")));
        let sheet = &self.sheet;
        let Some(c) = self.ws_gear.catalog.as_mut() else { return false };
        let Some((slot_i, index)) = c.selected else { return false };
        let slot = &c.slots[slot_i];
        let tag = slot.kind.tag;
        let kind_label = lang.tr(slot.kind.label);
        let doc = slot.doc.clone();
        let recs = data::records(&doc, slot.kind.data_container, slot.kind.data_item);
        let Some(r) = recs.get(index).copied() else { return false };
        let grade = c.grade(slot);
        let has_grades = !slot.grades.is_empty();
        let grade_list = slot.grades.clone();
        let name = lang.data_name(slot.kind.file, &r.id(), &r.name());
        let mut add: Option<bool> = None;
        let mut to_compare = false;
        ui.spacing_mut().item_spacing.y = 6.0;

        // Header.
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&name).font(widgets::bold(14.0)).color(ws.text)).wrap());
            if let Some(src) = SourceRef::of(r.el()) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tip = if pdfs.is_linked(&src.book) { lang.tr("Open the sourcebook at this page") } else { lang.tr("No PDF linked for this book — Tools → Sourcebooks") };
                    if widgets::button(ui, Some(icons::BOOK_OPEN), &src.to_string(), Look::Ghost, 22.0).on_hover_text(tip).clicked() {
                        pdf_ui::open(pdfs, &src, status);
                    }
                });
            }
        });
        let avail = avail_at(r, c.purchase.rating, grade.as_ref());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            widgets::tag(ui, &kind_label, ws.muted, ws.divider);
            let cat = r.category();
            if !cat.is_empty() {
                widgets::tag(ui, &lang.data_name(slot.kind.file, "", &cat), ws.muted, ws.divider);
            }
            match avail.as_ref().map(|a| a.legality) {
                Some(Legality::Restricted) => {
                    widgets::tag(ui, &lang.tr("Restricted"), ws.warning, ws.warning);
                }
                Some(Legality::Forbidden) => {
                    widgets::tag(ui, &lang.tr("Forbidden"), ws.error, ws.error);
                }
                _ => {}
            }
        });

        // Choices.
        let max_rating = select::rating_max(r);
        egui::Grid::new("ws_catalog_choices").num_columns(2).spacing([12.0, 6.0]).min_row_height(26.0).show(ui, |ui| {
            let caption = |ui: &mut egui::Ui, t: &str| {
                ui.label(RichText::new(t).size(12.0).color(ws.muted));
            };
            if max_rating > 0 {
                let min = r.el().get_i32("minrating").unwrap_or(1).clamp(0, max_rating);
                caption(ui, &r.el().child_text("ratinglabel").map(|l| if lang.has(&l) { lang.s(&l) } else { l }).filter(|l| !l.starts_with("String_") && !l.starts_with("Label_")).unwrap_or_else(|| lang.tr("Rating")));
                c.purchase.rating = c.purchase.rating.clamp(min.max(1), max_rating);
                widgets::stepper(ui, &mut c.purchase.rating, min.max(1), max_rating, &lang.tr("Lower Rating"), &lang.tr("Raise Rating"));
                ui.end_row();
            }
            if has_grades {
                caption(ui, &lang.tr("Grade"));
                let cur = grade.as_ref().map_or_else(|| "Standard".to_owned(), |g| g.name.clone());
                crate::combo::Combo::from_id_salt("ws_catalog_buy_grade").selected_text(cur.clone()).width(150.0).show_ui(ui, |ui| {
                    for g in &grade_list {
                        if crate::combo::selectable_label(ui, cur == g.name, format!("{} ({} ×{})", g.name, lang.tr("ess"), chummer_core::improvement::fmt_num(g.ess))).clicked() {
                            c.purchase.grade = Some(g.name.clone());
                        }
                    }
                });
                ui.end_row();
            }
            if matches!(tag, "gear" | "drug") {
                caption(ui, &lang.tr("Quantity"));
                ui.add(egui::DragValue::new(&mut c.purchase.qty).range(1.0..=1000.0).max_decimals(0));
                ui.end_row();
            }
            if let Some(n) = &locked_parent {
                caption(ui, &lang.tr("Install in"));
                ui.label(RichText::new(n).size(12.5).color(ws.text));
                ui.end_row();
            } else if !parents.is_empty() || select::parent_of(tag).is_some() {
                caption(ui, &lang.tr("Install in"));
                let required = select::parent_of(tag).is_some();
                let none_label = if required { lang.tr("Choose…") } else { lang.tr("Nothing (on its own)") };
                let cur = c.purchase.parent.as_ref().and_then(|g| parents.iter().find(|(pg, _)| pg == g)).map(|(_, n)| n.clone());
                crate::combo::Combo::from_id_salt("ws_catalog_parent").selected_text(cur.unwrap_or(none_label)).width(170.0).show_ui(ui, |ui| {
                    if !required && crate::combo::selectable_label(ui, c.purchase.parent.is_none(), lang.tr("Nothing (on its own)")).clicked() {
                        c.purchase.parent = None;
                    }
                    for (g, n) in &parents {
                        if crate::combo::selectable_label(ui, c.purchase.parent.as_deref() == Some(g), n).clicked() {
                            c.purchase.parent = Some(g.clone());
                        }
                    }
                });
                ui.end_row();
            }
        });

        // Before → after.
        let pv = c.preview.as_ref().map(|(_, p)| p);
        let after = pv.and_then(|p| p.as_ref().ok());
        if has_grades || after.is_some_and(|a| (a.sheet.essence - sheet.essence).abs() > 1e-9) {
            let ess_after = after.map(|a| a.sheet.essence);
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.tr("Essence")).size(12.0).color(ws.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(e) = ess_after {
                        ui.label(widgets::mono(format::essence(e, decimals), 13.0, if e < 0.0 { ws.error } else { ws.accent }));
                        ui.label(icons::icon(icons::ARROW_RIGHT, 12.0, ws.muted));
                    }
                    ui.label(widgets::mono(format::essence(sheet.essence, decimals), 12.0, ws.muted));
                });
            });
            widgets::essence_bar(ui, sheet.essence, ess_after.unwrap_or(sheet.essence));
            if let Some(e) = ess_after {
                ui.label(RichText::new(lang.tr_fmt("Light segment: this purchase ({0}).", &[&format::essence(sheet.essence - e, decimals)])).size(11.0).color(ws.muted));
            }
        }
        widgets::divider(ui);
        let base_cost = select::preview_cost(r, &c.purchase);
        let static_cost = base_cost.map(|b| b * grade.as_ref().map_or(1.0, |g| g.cost));
        let cost = match after {
            Some(a) if a.refused.is_none() => Some(nuyen_now - a.nuyen),
            _ => static_cost,
        };
        let cost_note = match (&grade, base_cost) {
            (Some(g), Some(b)) if (g.cost - 1.0).abs() > 1e-9 => format!("{} × {}", format::nuyen(b), chummer_core::improvement::fmt_num(g.cost)),
            _ => String::new(),
        };
        widgets::value_row(ui, &lang.tr("Cost"), &cost.map_or("—".into(), format::nuyen), ws.text, &cost_note);
        if let Some(a) = &avail {
            let (note, color) = match max_avail {
                Some(m) if !a.add_to_parent && a.value > m => (lang.tr_fmt("over {0}", &[&m]), ws.warning),
                Some(m) => (lang.tr_fmt("≤ {0} ok", &[&m]), ws.text),
                None => (String::new(), ws.text),
            };
            widgets::value_row(ui, &lang.tr("Availability"), &a.to_string(), color, &note);
        }
        if let Some(a) = after {
            let label = if creating { lang.tr("Nuyen left after") } else { lang.tr("Nuyen after") };
            let nuyen = if a.refused.is_some() { cost.map_or(a.nuyen, |c| nuyen_now - c) } else { a.nuyen };
            widgets::value_row(ui, &label, &format::nuyen(nuyen), if nuyen < 0.0 { ws.error } else { ws.accent }, "");
            let init = |s: &Sheet| format!("{} + {}d6", s.initiative, s.initiative_dice);
            if init(&a.sheet) != init(sheet) {
                widgets::value_row(ui, &lang.tr("Initiative after"), &init(&a.sheet), ws.accent, &lang.tr_fmt("was {0}", &[&init(sheet)]));
            }
            for at in ATTRIBUTES {
                let (b, x) = (sheet.attr(at), a.sheet.attr(at));
                if b != x {
                    widgets::value_row(ui, &lang.tr_fmt("{0} after", &[&lang.tr(at)]), &x.to_string(), ws.accent, &lang.tr_fmt("was {0}", &[&b]));
                }
            }
            if a.sheet.armor != sheet.armor {
                widgets::value_row(ui, &lang.tr("Armor after"), &a.sheet.armor.to_string(), ws.accent, &lang.tr_fmt("was {0}", &[&sheet.armor]));
            }
            let limits = |s: &Sheet| format!("{} / {} / {}", s.limit_physical, s.limit_mental, s.limit_social);
            if limits(&a.sheet) != limits(sheet) {
                widgets::value_row(ui, &lang.tr("Limits after"), &limits(&a.sheet), ws.accent, &lang.tr_fmt("was {0}", &[&limits(sheet)]));
            }
            if (a.sheet.physical_cm, a.sheet.stun_cm) != (sheet.physical_cm, sheet.stun_cm) {
                let cm = |s: &Sheet| format!("{} / {}", s.physical_cm, s.stun_cm);
                widgets::value_row(ui, &lang.tr("Condition Monitor"), &cm(&a.sheet), ws.accent, &lang.tr_fmt("was {0}", &[&cm(sheet)]));
            }
        }
        let capacity = r.get("capacity");
        if !capacity.trim().is_empty() {
            widgets::value_row(ui, &lang.tr("Capacity"), &capacity, ws.muted, "");
        }

        // Checks.
        widgets::divider(ui);
        ui.label(widgets::overline(&lang.tr("Rules"), &ws));
        let mut blocked = false;
        if check_failures.is_empty() {
            widgets::icon_line(ui, icons::CHECK_CIRCLE, &lang.tr("Requirements met."), ws.accent, ws.text);
        }
        for w in &check_failures {
            widgets::icon_line(ui, icons::WARNING, w, ws.warning, ws.text);
            blocked = true;
        }
        if let (Some(m), Some(a)) = (max_avail, &avail) {
            if !a.add_to_parent && a.value > m {
                widgets::icon_line(ui, icons::WARNING, &lang.tr_fmt("Availability {0} is above the limit of {1}.", &[&a.to_string(), &m]), ws.warning, ws.text);
                blocked = true;
            }
        }
        if let Some(a) = after {
            if let Some(r) = &a.refused {
                widgets::icon_line(ui, icons::X_CIRCLE, r, ws.error, ws.text);
                blocked = true;
            } else if a.nuyen < 0.0 {
                widgets::icon_line(ui, icons::WARNING, &lang.tr("Not enough nuyen."), ws.warning, ws.text);
            }
            if a.sheet.essence < 0.0 {
                widgets::icon_line(ui, icons::X_CIRCLE, &lang.tr("Essence would drop below zero."), ws.error, ws.text);
            }
            if let Some(v) = &a.assumed {
                widgets::icon_line(ui, icons::INFO, &lang.tr_fmt("Preview with “{0}”; Add asks which.", &[v]), ws.muted, ws.muted);
            }
        }
        if let Some(Err(e)) = pv {
            // Not blocking: Add may still ask a question the preview could not answer.
            widgets::icon_line(ui, icons::X_CIRCLE, e, ws.error, ws.text);
        }
        let needs_parent = select::parent_of(tag).is_some() && c.purchase.parent.is_none();
        if needs_parent {
            widgets::icon_line(ui, icons::INFO, &lang.tr("Choose where to install it"), ws.muted, ws.text);
        }

        // The bonus selection, when Add asked for it.
        let mut back = false;
        if let Some((choices, answer)) = &mut c.answer {
            widgets::divider(ui);
            if let Some(ch) = choices.first() {
                ui.label(RichText::new(&ch.prompt).font(widgets::bold(12.5)).color(ws.text));
                if ch.options.is_empty() {
                    ui.add(egui::TextEdit::singleline(answer).desired_width(f32::INFINITY));
                } else {
                    egui::ScrollArea::vertical().id_salt("ws_catalog_answer").max_height(220.0).show(ui, |ui| {
                        for o in &ch.options {
                            if crate::combo::selectable_label(ui, answer == o, o).clicked() {
                                *answer = o.clone();
                            }
                        }
                    });
                }
            }
            let ok = !answer.trim().is_empty();
            ui.horizontal(|ui| {
                if ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::PLUS), &lang.tr("Add"), Look::Primary, 28.0)).inner.clicked() {
                    add = Some(false);
                }
                back = widgets::button(ui, None, &lang.tr("Back"), Look::Ghost, 28.0).clicked();
            });
        } else {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let can = !blocked && !needs_parent;
                let r = ui.add_enabled_ui(can, |ui| widgets::button(ui, Some(icons::PLUS), &lang.tr_fmt("Add {0}", &[&name]), Look::Primary, 30.0)).inner;
                let r = r.on_disabled_hover_text(lang.tr("Fix the problems above first"));
                if r.clicked() {
                    add = Some(false);
                }
                let full = c.compare.len() >= COMPARE_MAX;
                let r = ui.add_enabled_ui(!full, |ui| widgets::icon_button(ui, icons::SCALES, 30.0)).inner;
                if r.on_hover_text(lang.tr("Add to comparison")).on_disabled_hover_text(lang.tr_fmt("Up to {0} records", &[&COMPARE_MAX])).clicked() {
                    to_compare = true;
                }
            });
            let note = if !creating { lang.tr("Career mode pays for it from your nuyen.") } else { lang.tr("Remove it later from the list.") };
            ui.label(RichText::new(note).size(11.0).color(ws.muted));
        }
        if back {
            c.answer = None;
        }
        if to_compare {
            let short = match (c.purchase.rating, &grade) {
                (r, Some(g)) if r > 0 && g.name != "Standard" => format!("{name} {r} {}", g.name),
                (r, _) if r > 0 => format!("{name} {r}"),
                (_, Some(g)) if g.name != "Standard" => format!("{name} {}", g.name),
                _ => name.clone(),
            };
            let ess = after.map_or("—".into(), |a| format::essence(sheet.essence - a.sheet.essence, decimals));
            let init = after.map_or("—".into(), |a| {
                let (d, dd) = (a.sheet.initiative - sheet.initiative, a.sheet.initiative_dice - sheet.initiative_dice);
                match (d, dd) {
                    (0, 0) => "—".into(),
                    (0, dd) => format!("{dd:+}d6"),
                    (d, 0) => format!("{d:+}"),
                    (d, dd) => format!("{d:+} {dd:+}d6"),
                }
            });
            let entry = Compared { name: short, ess, init, cost: cost.map_or("—".into(), format::nuyen) };
            c.compare.retain(|x| x.name != entry.name);
            c.compare.push(entry);
        }

        // Comparison.
        if !c.compare.is_empty() {
            widgets::divider(ui);
            let mut clear = false;
            let mut drop = None;
            ui.horizontal(|ui| {
                ui.label(widgets::overline(&lang.tr("Compare"), &ws));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clear = widgets::button(ui, None, &lang.tr("Clear"), Look::Ghost, 20.0).clicked();
                });
            });
            // Fixed columns (a grid would widen the inspector).
            let widths = [40.0_f32, 54.0, 70.0, 20.0];
            let name_w = (ui.available_width() - widths.iter().sum::<f32>() - 5.0 * 4.0 - 4.0).max(40.0);
            let row = |ui: &mut egui::Ui, cells: [RichText; 4], name: RichText| -> egui::Response {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    ui.allocate_ui_with_layout(egui::vec2(name_w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(name_w);
                        ui.add(egui::Label::new(name).truncate());
                    });
                    let mut last = None;
                    for (k, (c, w)) in cells.into_iter().zip(widths).enumerate() {
                        if k == 3 {
                            last = Some(ui.allocate_ui(egui::vec2(w, 20.0), |ui| widgets::icon_button(ui, icons::X, 20.0)).inner);
                        } else {
                            ui.allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_width(w);
                                ui.add(egui::Label::new(c).truncate());
                            });
                        }
                    }
                    last
                })
                .inner
                .unwrap_or_else(|| ui.label(""))
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let h = |t: &str| RichText::new(lang.tr(t)).size(10.5).color(ws.muted);
                for (t, w) in ["Item", "Ess", "Init", "Cost"].into_iter().zip(std::iter::once(name_w).chain(widths)) {
                    ui.allocate_ui_with_layout(egui::vec2(w, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(w);
                        ui.label(h(t));
                    });
                }
            });
            for (i, e) in c.compare.iter().enumerate() {
                let m = |t: &str| widgets::mono(t, 11.5, ws.text);
                let r = row(ui, [m(&e.ess), m(&e.init), m(&e.cost), RichText::new("")], RichText::new(&e.name).size(12.0).color(ws.text));
                if r.on_hover_text(lang.tr("Remove")).clicked() {
                    drop = Some(i);
                }
            }
            if clear {
                c.compare.clear();
            } else if let Some(i) = drop {
                c.compare.remove(i);
            }
        }

        // The record's data.
        ui.add_space(2.0);
        egui::CollapsingHeader::new(RichText::new(lang.tr("Data")).size(12.0).color(ws.muted)).id_salt("ws_catalog_data").show(ui, |ui| {
            crate::browser::record_fields(ui, r.el(), 0);
        });

        match add {
            Some(close) => self.ws_catalog_add(engine, lang, status, close),
            None => false,
        }
    }
}
