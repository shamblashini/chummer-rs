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
use chummer_core::items::{self, edit, place, Kind, Purchase};
use chummer_core::lang::Language;
use chummer_core::requirements::Check;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::xml::Element;
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense};

use super::ws_items::{page_kinds, Page};
use super::ws_inventory::kind_icon;
use super::{kind_noun, CharacterView};
use crate::pdf_ui::{self, Status};
use crate::select;
use crate::theme;
use crate::workspace::{icons, pool_diff};
use crate::workspace::table::{self, Action, Cell, Col, Event, Kind as RowKind, RowData, States, Tag, Tone};
use crate::workspace::widgets::{self, Look};

const FILTERS_WIDTH: f32 = 196.0;
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
    /// "Installed R2 · Alphaware" when the character has it.
    owned: Option<String>,
    /// Goes into the target container (always, without one).
    fits: bool,
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
    /// The dice pools, skills and weapons it changes.
    pools: Vec<pool_diff::Line>,
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
    /// The bonus selection being asked before adding: (choices, answer).
    answer: Option<(Vec<Choice>, String)>,
    preview: Option<(u64, Result<Preview, String>)>,
    compare: Vec<Compared>,
    rows: Option<(u64, Rows)>,
    /// Give the search field focus next frame.
    focus_search: bool,
    /// Scroll the selected row into view (moved with the arrow keys).
    scroll: bool,
    /// The location a group's "+" adds into: (guid, name).
    location: Option<(String, String)>,
    /// The filters are shown.
    show_filters: bool,
    /// The results as table rows, for the rows' key and the sort.
    table: Option<(u64, Vec<table::Row>)>,
    /// The item being bought into ("Adding into"): the catalog then
    /// sells the kinds it takes (`place::accepts`) instead of the page's
    /// own, and it is the purchase's parent.
    anchor: Option<String>,
    /// Why the selected record does not go into the anchor: (refusal,
    /// record, anchor).
    target_note: Option<(place::Misfit, String, String)>,
    /// What the target was last worked out for.
    retarget_key: u64,
    /// The inspector shows the selected record (else the inventory's
    /// selected item, while the record stays selected for the target).
    inspect: bool,
}

impl Catalog {
    fn new(page: Page, tags: &[&str], on: &str, store: &chummer_core::data::DataStore) -> Option<Catalog> {
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
            categories: BTreeSet::new(),
            books_off: BTreeSet::new(),
            legality: [true; 3],
            fit_avail: true,
            fit_essence: false,
            affordable: false,
            requirements_met: false,
            rating: 0,
            selected: None,
            purchase: Purchase { qty: 1.0, cost_multiplier: 1.0, ..Default::default() },
            answer: None,
            preview: None,
            compare: Vec::new(),
            rows: None,
            focus_search: true,
            scroll: false,
            location: None,
            show_filters: false,
            table: None,
            anchor: None,
            target_note: None,
            retarget_key: 0,
            inspect: true,
        })
    }

    /// How many filters are on (the Filters button's count).
    fn active_filters(&self) -> usize {
        usize::from(self.fit_essence && self.ware())
            + usize::from(self.fit_avail)
            + usize::from(self.affordable)
            + usize::from(self.requirements_met)
            + usize::from(self.legality != [true; 3])
            + usize::from(self.rating > 0)
            + self.books_off.len().min(1)
            + self.categories.len()
    }

    /// The item it adds into while it sells the kinds that item takes.
    pub(super) fn anchor(&self) -> Option<&str> {
        self.anchor.as_deref()
    }

    /// The container it adds into.
    pub(super) fn target(&self) -> Option<&str> {
        self.purchase.parent.as_deref()
    }

    /// Whether it sells kind `tag`.
    pub(super) fn sells(&self, tag: &str) -> bool {
        self.slots.iter().any(|s| s.kind.tag == tag)
    }

    /// The kinds it sells, in order.
    fn tags(&self) -> Vec<&'static str> {
        self.slots.iter().map(|s| s.kind.tag).collect()
    }

    /// The kind shown (the first, when several are).
    fn shown(&self) -> Option<&'static str> {
        self.slots.iter().find(|s| s.on).map(|s| s.kind.tag)
    }

    /// Sell the kinds `tags`, showing only `on` (else what a new catalog
    /// of them shows). New kinds start over: the search, the selection,
    /// the category filter and the comparison; another kind shown drops
    /// the selection and the category filter.
    fn set_kinds(&mut self, tags: &[&str], on: Option<&str>, store: &chummer_core::data::DataStore) {
        if self.tags() != tags {
            let Some(fresh) = Catalog::new(self.page, tags, on.unwrap_or(tags.first().copied().unwrap_or("")), store) else { return };
            self.slots = fresh.slots;
            self.search.clear();
            self.compare.clear();
            if let Some(on) = on {
                for s in self.slots.iter_mut() {
                    s.on = s.kind.tag == on;
                }
            }
            self.kind_changed();
        } else if let Some(on) = on {
            if self.slots.iter().any(|s| s.on != (s.kind.tag == on)) {
                for s in self.slots.iter_mut() {
                    s.on = s.kind.tag == on;
                }
                self.kind_changed();
            }
        }
    }

    fn kind_changed(&mut self) {
        self.selected = None;
        self.categories.clear();
        self.preview = None;
        self.rows = None;
        self.table = None;
        self.answer = None;
        self.purchase.answer = None;
        self.purchase.rating = 0;
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
        self.inspect = true;
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

/// What the character has, by record id (or name), lowercase: (rating,
/// grade) of each.
fn owned_items(ch: &Character, sec: &chummer_core::sections::Section) -> std::collections::HashMap<String, Vec<(i32, String)>> {
    let mut out: std::collections::HashMap<String, Vec<(i32, String)>> = std::collections::HashMap::new();
    fn walk(nodes: &[chummer_core::tree::ItemNode], out: &mut std::collections::HashMap<String, Vec<(i32, String)>>) {
        for n in nodes {
            if let chummer_core::tree::Entry::Item { el, .. } = &n.value {
                let id = el.get("sourceid");
                let key = if id.is_empty() { el.get("name") } else { id };
                out.entry(key.to_lowercase()).or_default().push((el.get_i32("rating").unwrap_or(0), el.get("grade")));
            }
            walk(&n.children, out);
        }
    }
    walk(&chummer_core::tree::section_tree(&ch.doc, sec), &mut out);
    out
}

/// "Installed R2 · Alphaware", "Installed ×2".
fn owned_line(list: Option<&Vec<(i32, String)>>, lang: &Language) -> Option<String> {
    let list = list.filter(|l| !l.is_empty())?;
    if list.len() > 1 {
        return Some(lang.tr_fmt("Installed ×{0}", &[&list.len()]));
    }
    let (r, g) = &list[0];
    let mut s = if *r > 0 { lang.tr_fmt("Installed R{0}", &[r]) } else { lang.tr("Installed") };
    if !g.is_empty() && g != "Standard" && g != "None" {
        s = format!("{s} · {g}");
    }
    Some(s)
}

/// Whether a record of kind `tag` is one to buy: weapon mounts by their
/// size (the other `weaponmount` records are a mount's options).
fn listed(tag: &str, r: Record<'_>) -> bool {
    tag != "weaponmount" || r.category() == "Size"
}

/// The names of the kinds an item takes, for the "Adding into" switch:
/// "Mods", "Weapon mounts", "Gear"…
pub(super) fn into_label(lang: &Language, parent: &str, tag: &str, label: &str) -> String {
    let en = match (parent, tag) {
        ("gear", "gear") => "Plugins",
        (_, "gear") => "Gear",
        (_, "mod") => "Mods",
        (_, "weaponmount") => "Weapon mounts",
        (_, "accessory") => "Accessories",
        (_, "armormod") => "Armor mods",
        ("weapon", "weapon") => "Underbarrel weapons",
        (_, "weapon") => "Weapons",
        (_, "cyberware") => "Cyberware",
        (_, "bioware") => "Bioware",
        _ => label,
    };
    lang.tr(en)
}

fn hash_of(v: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

impl CharacterView {
    /// Open the inline catalog on `page` for kind `tag` (of the page's
    /// kinds), at the top level.
    pub(crate) fn ws_open_catalog(&mut self, page: Page, tag: &str) {
        let container = super::ws_items::page_container(page);
        let kinds = page_kinds(container);
        let tags: Vec<&str> = if !kinds.contains(&tag) { vec![tag] } else { kinds.to_vec() };
        self.ws_gear.catalog = Catalog::new(page, &tags, tag, &self.store);
        self.ws_gear.focus_catalog = true;
    }

    /// The catalog has a record selected and is on the page shown.
    pub(crate) fn ws_catalog_has_selection(&self) -> bool {
        self.ws_gear.catalog.as_ref().is_some_and(|c| c.selected.is_some() && self.ws_item_page(self.tab) == Some(c.page))
    }

    /// The inspector shows the catalog's selected record (not the
    /// inventory's selected item).
    pub(crate) fn ws_catalog_inspecting(&self) -> bool {
        self.ws_catalog_has_selection() && self.ws_gear.catalog.as_ref().is_some_and(|c| c.inspect)
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
        let _s = crate::trace::span("catalog rows");
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let key = hash_of((
            &c.search,
            &c.categories,
            &c.books_off,
            c.legality,
            (c.fit_avail, c.fit_essence, c.affordable, c.requirements_met),
            c.rating,
            c.slots.iter().map(|s| (s.kind.tag, s.on)).collect::<Vec<_>>(),
            &c.purchase.grade,
            &c.purchase.parent,
            &c.anchor,
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
        let owned = owned_items(&self.doc, &super::ws_items::section_of(c.page));
        // The target: what fits it sorts first.
        let host = c.purchase.parent.as_ref().and_then(|g| place::Host::of(&self.doc, &self.store, g));
        let target_name = host.as_ref().map(|h| super::display_name(&super::ws_items::section_of(c.page), &h.el, lang)).unwrap_or_default();
        let enforce = self.settings.as_ref().is_none_or(|s| s.flag("enforcecapacity"));
        let mut out = Rows { kinds: vec![0; c.slots.len()], ..Default::default() };
        for (si, slot) in c.slots.iter().enumerate() {
            let grade = c.grade(slot);
            let cols: Vec<&str> = select::columns(slot.kind.tag).iter().map(|(_, f)| *f).filter(|f| !matches!(*f, "avail" | "cost" | "ess" | "rating")).collect();
            let kind_label = lang.tr(slot.kind.label);
            for (i, r) in data::records(&slot.doc, slot.kind.data_container, slot.kind.data_item).into_iter().enumerate() {
                if r.hidden() || (!books.is_empty() && !r.source().is_empty() && !books.contains(&r.source())) || !listed(slot.kind.tag, r) {
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
                // What fits the target: the purchase rules, free capacity at
                // the rating shown (`items::place`).
                let fits = match host.as_ref().map(|h| h.takes(slot.kind.tag, r.el(), None, rating, enforce, false)) {
                    Some(Err(m)) => {
                        why.insert(0, super::ws_inventory::misfit_text(lang, &m, &name, &target_name));
                        false
                    }
                    _ => true,
                };
                let owned = owned_line(owned.get(&r.id().to_lowercase()).or_else(|| owned.get(&r.name().to_lowercase())), lang);
                let sub = if several { [kind_label.clone(), shown_category].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ") } else { shown_category };
                let extra = cols.iter().map(|f| r.get(f)).collect();
                out.list.push(Row { slot: si, index: i, name, sub, rating, ess, avail, cost, extra, source: SourceRef::of(r.el()), why, over: is_over, rank, owned, fits });
            }
        }
        out.list.sort_by_key(|r| r.rank);
        // What fits the target first.
        out.list.sort_by_key(|r| !r.fits);
        if let Some(c) = self.ws_gear.catalog.as_mut() {
            c.rows = Some((key, out));
        }
    }

    /// The catalog panel in `rect`: its head, the search with the kind
    /// switch, the active filters as chips (all of them behind Filters),
    /// the target bar, the results and the key hints. `stacked`: above
    /// the inventory (one-line rows); `fold`: the inspector is folded
    /// away, so the selected row opens its details strip. Returns true if
    /// the character changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ws_catalog_panel(&mut self, ui: &mut egui::Ui, rect: egui::Rect, stacked: bool, fold: bool, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        self.ws_catalog_retarget();
        self.ws_catalog_rows(lang);
        if fold {
            self.ws_catalog_preview(engine);
        }
        let ws = theme::ws(ui);
        let mut changed = false;
        let mut close = false;
        let mut add: Option<bool> = None;
        // Keys, before the search field takes them: arrows move, Enter
        // adds (and hands the keys to the inventory), Shift+Enter adds and
        // stays, Esc closes.
        let search_id = egui::Id::new("ws_catalog_search");
        let search_focused = ui.ctx().memory(|m| m.has_focus(search_id));
        let free = (self.ws_gear.focus_catalog && !ui.ctx().wants_keyboard_input()) || search_focused;
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
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut child;
        ui.painter().rect(rect, CornerRadius::same(6), ws.raised, egui::Stroke::new(1.0_f32, ws.divider), egui::StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        ui.set_clip_rect(inner.intersect(ui.clip_rect()));
        if ui.rect_contains_pointer(inner) && ui.input(|i| i.pointer.any_pressed()) {
            self.ws_gear.focus_catalog = true;
        }
        // Head.
        let head = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), 36.0));
        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(head.left(), head.bottom() - 1.0), egui::vec2(head.width(), 1.0)), CornerRadius::ZERO, ws.divider);
        {
            let Some(c) = self.ws_gear.catalog.as_mut() else { return false };
            let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(egui::vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            hu.spacing_mut().item_spacing.x = 8.0;
            hu.label(icons::icon(icons::STOREFRONT, 15.0, ws.accent));
            hu.label(widgets::title(&lang.tr("Catalog"), &ws));
            let kinds: Vec<String> = c.slots.iter().filter(|s| s.on || c.anchor.is_none()).map(|s| lang.tr(s.kind.label)).collect();
            hu.add(egui::Label::new(RichText::new(kinds.join(" & ")).size(11.0).color(ws.muted)).truncate());
            hu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let glyph = if stacked { icons::CARET_UP } else { icons::CARET_DOUBLE_LEFT };
                if widgets::icon_button(ui, glyph, 22.0).on_hover_text(lang.tr("Close the catalog (Esc)")).clicked() {
                    close = true;
                }
                let n = c.active_filters();
                let label = if n > 0 { format!("{} {n}", lang.tr("Filters")) } else { lang.tr("Filters") };
                let r = widgets::button(ui, Some(icons::FUNNEL_SIMPLE), &label, if c.show_filters { Look::Secondary } else { Look::Ghost }, 24.0);
                if r.on_hover_text(lang.tr("Show or hide every filter")).clicked() {
                    c.show_filters = !c.show_filters;
                }
                if let Some((_, rows)) = &c.rows {
                    ui.label(RichText::new(lang.tr_fmt("{0} of {1}", &[&rows.list.len(), &rows.total])).size(11.0).color(ws.muted));
                }
            });
        }
        let body = egui::Rect::from_min_max(egui::pos2(inner.left() + 10.0, head.bottom() + 8.0), egui::pos2(inner.right() - 10.0, inner.bottom()));
        let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        bu.spacing_mut().item_spacing.y = 8.0;
        self.ws_catalog_search(&mut bu, lang);
        self.ws_catalog_chips(&mut bu, lang);
        if self.ws_gear.catalog.as_ref().is_some_and(|c| c.show_filters) {
            let max_h = (body.height() * 0.4).clamp(120.0, 320.0);
            widgets::card_frame(&ws).fill(ws.chrome).inner_margin(egui::Margin::same(10)).show(&mut bu, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("ws_catalog_filters").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 10.0);
                    self.ws_catalog_filters(ui, lang);
                });
            });
        }
        self.ws_catalog_target_bar(&mut bu, lang);
        // Results, then the key hints.
        let hints_h = 30.0;
        let rest = bu.available_rect_before_wrap();
        let results = egui::Rect::from_min_max(egui::pos2(inner.left(), rest.top()), egui::pos2(inner.right(), inner.bottom() - hints_h));
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(results).layout(egui::Layout::top_down(egui::Align::Min)));
        ru.painter().rect_filled(egui::Rect::from_min_size(results.min, egui::vec2(results.width(), 1.0)), CornerRadius::ZERO, ws.divider);
        if let Some(a) = self.ws_catalog_results(&mut ru, results.height(), stacked, fold, lang, pdfs, status) {
            add = Some(a);
        }
        let hints = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - hints_h), inner.max);
        ui.painter().rect_filled(hints, CornerRadius::ZERO, ws.chrome);
        ui.painter().rect_filled(egui::Rect::from_min_size(hints.min, egui::vec2(hints.width(), 1.0)), CornerRadius::ZERO, ws.divider);
        let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(hints.shrink2(egui::vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        hu.spacing_mut().item_spacing.x = 5.0;
        let target = self.ws_gear.catalog.as_ref().and_then(|c| c.target()).and_then(|g| edit::find(&self.doc, g)).map(|e| e.get("name"));
        let enter = match &target {
            Some(t) => lang.tr_fmt("add into {0}", &[t]),
            None => lang.tr("add"),
        };
        for (k, t) in [("Enter", enter), ("Shift+Enter", lang.tr("add, keep focus")), ("Tab", lang.tr("to the inventory"))] {
            widgets::kbd(&mut hu, k);
            hu.label(RichText::new(t).size(11.0).color(ws.muted));
            hu.add_space(6.0);
        }
        if let Some(to_inventory) = add {
            let added = self.ws_catalog_add(lang, status, to_inventory);
            if added && to_inventory {
                // The keys go to the inventory, where the new row is.
                ui.memory_mut(|m| m.surrender_focus(search_id));
            }
            changed |= added;
        }
        if close {
            self.ws_gear.catalog = None;
            self.ws_gear.focus_catalog = false;
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
        if c.slots.len() > 1 && c.anchor.is_none() {
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
/// `before` is the character's current sheet, for the dice pools.
fn preview(ch: &Character, before: &Sheet, settings: Option<&chummer_core::settings::CharacterSettings>, engine: &Engine, tag: &str, rec: &Element, p: &Purchase) -> Result<Preview, String> {
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
    let pools = pool_diff::diff((ch, before), (&copy, &sheet));
    Ok(Preview { sheet, nuyen, assumed, refused, pools })
}

/// Attributes the preview compares.
const ATTRIBUTES: &[&str] = &["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES", "DEP"];

impl CharacterView {
    /// The search field and, for pages with several kinds, the kind switch
    /// (All / Cyberware / Bioware).
    fn ws_catalog_search(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let total = c.rows.as_ref().map_or(0, |(_, r)| r.total);
        let kinds: Vec<String> = c.slots.iter().filter(|s| s.on).map(|s| kind_noun(lang, s.kind.label)).collect();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if c.slots.len() > 1 && c.anchor.is_none() {
                    let labels: Vec<String> = std::iter::once(lang.tr("All")).chain(c.slots.iter().map(|s| short_kind(lang, s.kind.label))).collect();
                    let items: Vec<(&str, &str)> = labels.iter().map(|l| (l.as_str(), "")).collect();
                    let on: Vec<usize> = (0..c.slots.len()).filter(|i| c.slots[*i].on).collect();
                    let cur = if on.len() == 1 { on[0] + 1 } else { 0 };
                    if let Some(i) = widgets::segmented(ui, &items, cur, 28.0) {
                        for (k, s) in c.slots.iter_mut().enumerate() {
                            s.on = i == 0 || k + 1 == i;
                        }
                        c.selected = None;
                    }
                }
                let field_w = ui.available_width();
                let field = egui::Frame::new().fill(ws.well).stroke(egui::Stroke::new(1.0_f32, ws.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width((field_w - 18.0).max(0.0));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(icons::icon(icons::MAGNIFYING_GLASS, 13.0, ws.muted));
                        let hint = if total > 0 { lang.tr_fmt("Search {0} {1}", &[&total, &kinds.join(" / ")]) } else { lang.tr_fmt("Search {0}", &[&kinds.join(" / ")]) };
                        let r = ui.add(egui::TextEdit::singleline(&mut c.search).id(egui::Id::new("ws_catalog_search")).frame(false).hint_text(hint).desired_width(f32::INFINITY));
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
            });
        });
    }

    /// The active filters as chips; × turns one off. The rest of the
    /// line says how many match.
    fn ws_catalog_chips(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let essence = format::essence(self.sheet.essence, self.rules.essence_decimals);
        let max_avail = self.ws_max_avail();
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let mut chips: Vec<(String, u8)> = Vec::new();
        if c.fit_essence && c.ware() {
            chips.push((lang.tr_fmt("Fits essence {0}", &[&essence]), 0));
        }
        if let (true, Some(m)) = (c.fit_avail, max_avail) {
            chips.push((lang.tr_fmt("Avail ≤ {0}", &[&m]), 1));
        }
        if c.affordable {
            chips.push((lang.tr("Affordable"), 2));
        }
        if c.requirements_met {
            chips.push((lang.tr("Requirements met"), 3));
        }
        if c.legality != [true; 3] {
            chips.push((lang.tr("Legality"), 4));
        }
        if c.rating > 0 {
            chips.push((lang.tr_fmt("Rating {0}", &[&c.rating]), 5));
        }
        if !c.books_off.is_empty() {
            chips.push((lang.tr_fmt("{0} books off", &[&c.books_off.len()]), 6));
        }
        let cats: Vec<String> = c.categories.iter().cloned().collect();
        let mut drop = None;
        let mut drop_cat = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            for (text, id) in &chips {
                if chip_x(ui, text, &ws) {
                    drop = Some(*id);
                }
            }
            let file = c.slots.iter().find(|s| s.on).map_or("", |s| s.kind.file);
            for cat in &cats {
                if chip_x(ui, &lang.data_name(file, "", cat), &ws) {
                    drop_cat = Some(cat.clone());
                }
            }
            if c.ware() {
                let g = c.purchase.grade.clone().unwrap_or_else(|| "Standard".into());
                widgets::tag(ui, &lang.tr_fmt("Grade: {0}", &[&g]), ws.muted, ws.divider);
            }
            if let Some((_, rows)) = &c.rows {
                if rows.hidden > 0 {
                    ui.label(RichText::new(lang.tr_fmt("{0} hidden by filters", &[&rows.hidden])).size(11.0).color(ws.muted));
                }
            }
        });
        match drop {
            Some(0) => c.fit_essence = false,
            Some(1) => c.fit_avail = false,
            Some(2) => c.affordable = false,
            Some(3) => c.requirements_met = false,
            Some(4) => c.legality = [true; 3],
            Some(5) => c.rating = 0,
            Some(6) => c.books_off.clear(),
            _ => {}
        }
        if let Some(cat) = drop_cat {
            c.categories.remove(&cat);
        }
    }

    /// "Adding into Hermes Ikon · in Carried" with Change and ×, while
    /// the catalog has a target container (or a location), and under it
    /// the switch between the kinds the target takes ("Mods | Weapon
    /// mounts | Gear"); "Choose where to install it" when the kind needs
    /// one.
    fn ws_catalog_target_bar(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let page = c.page;
        let tag = c.shown().unwrap_or("");
        let needs = c.slots.iter().filter(|s| s.on).all(|s| select::parent_of(s.kind.tag).is_some());
        let target = c.target().and_then(|g| edit::find(&self.doc, g)).cloned();
        let location = c.location.clone();
        let note = c.target_note.as_ref().map(|(m, item, at)| super::ws_inventory::misfit_text(lang, m, item, at));
        if target.is_none() && location.is_none() && !needs && note.is_none() {
            return;
        }
        // The kinds the anchor takes, for the switch.
        let into: Vec<(&'static str, String)> = match (&c.anchor, &target) {
            (Some(a), Some(t)) if t.get("guid").eq_ignore_ascii_case(a) => {
                let ptag = edit::tag_of(t);
                place::accepts(&self.doc, &self.store, a).into_iter().map(|k| (k.tag, into_label(lang, ptag, k.tag, k.label))).collect()
            }
            _ => Vec::new(),
        };
        let cap = target.as_ref().and_then(|t| edit::capacity(&self.doc, &t.get("guid")));
        let sec = super::ws_items::section_of(c.page);
        let loc_name = target.as_ref().and_then(|t| {
            let top = top_ancestor(&self.doc, &t.get("guid"));
            let l = top.get("location");
            (!l.is_empty()).then(|| edit::locations(&self.doc, &top.get("guid")).into_iter().find(|(g, n)| g.eq_ignore_ascii_case(&l) || *n == l).map_or(l, |(_, n)| n))
        });
        let mut pick: Option<Option<String>> = None;
        let mut clear = false;
        let mut switch: Option<&'static str> = None;
        egui::Frame::new().fill(ws.selection).stroke(egui::Stroke::new(1.0_f32, ws.primary)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin { left: 10, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(24.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(icons::icon(icons::ARROW_BEND_DOWN_RIGHT, 14.0, ws.accent));
                match (&target, &location) {
                    (Some(t), _) => {
                        ui.label(RichText::new(lang.tr("Adding into")).size(12.0).color(ws.muted));
                        ui.label(icons::icon(kind_icon(&t.name), 14.0, ws.text));
                        ui.add(egui::Label::new(RichText::new(super::display_name(&sec, t, lang)).font(widgets::bold(12.5)).color(ws.text)).truncate());
                        if let Some(l) = &loc_name {
                            ui.label(RichText::new(lang.tr_fmt("in {0}", &[l])).size(11.0).color(ws.muted));
                        }
                        if let Some((used, total)) = cap {
                            let f = chummer_core::improvement::fmt_num;
                            let full = used >= total - 1e-9;
                            let text = lang.tr_fmt("{0}/{1} capacity", &[&f(used), &f(total)]);
                            widgets::tag(ui, &text, if full { ws.warning } else { ws.muted }, if full { ws.warning } else { ws.divider });
                        }
                    }
                    (None, Some((_, name))) => {
                        ui.label(RichText::new(lang.tr("Adding into")).size(12.0).color(ws.muted));
                        ui.label(icons::icon(icons::MAP_PIN, 14.0, ws.text));
                        ui.label(RichText::new(name).font(widgets::bold(12.5)).color(ws.text));
                    }
                    (None, None) => match &note {
                        Some(n) => {
                            ui.label(RichText::new(lang.tr("Adding at the top level")).size(12.0).color(ws.text));
                            ui.add(egui::Label::new(RichText::new(n).size(11.0).color(ws.warning)).truncate()).on_hover_text(n);
                        }
                        None => {
                            ui.label(RichText::new(lang.tr("Choose where to install it")).size(12.0).color(ws.text));
                        }
                    },
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if (target.is_some() || location.is_some()) && widgets::icon_button(ui, icons::X, 22.0).on_hover_text(lang.tr("Add at top level instead")).clicked() {
                        clear = true;
                    }
                    let r = widgets::button(ui, Some(icons::CARET_DOWN), &lang.tr("Change"), Look::Ghost, 24.0);
                    egui::Popup::menu(&r).show(|ui| {
                        ui.set_min_width(220.0);
                        if !needs && ui.button(lang.tr("Nothing (on its own)")).clicked() {
                            pick = Some(None);
                        }
                        for (g, n) in self.ws_catalog_parents(&sec, tag, lang) {
                            if ui.button(n).clicked() {
                                pick = Some(Some(g));
                            }
                        }
                    });
                });
            });
            if !into.is_empty() {
                // The kinds it takes; the shown one selected.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let items: Vec<(&str, &str)> = into.iter().map(|(_, l)| (l.as_str(), "")).collect();
                    let cur = into.iter().position(|(t, _)| *t == tag).unwrap_or(0);
                    if let Some(i) = widgets::segmented(ui, &items, cur, 24.0) {
                        if i != cur {
                            switch = Some(into[i].0);
                        }
                    }
                    if let Some(n) = &note {
                        ui.add(egui::Label::new(RichText::new(n).size(11.0).color(ws.warning)).truncate()).on_hover_text(n);
                    }
                });
            }
        });
        if clear {
            self.ws_catalog_leave_into();
        }
        if let Some(p) = pick {
            match p {
                Some(g) => {
                    self.ws_catalog_into(page, &g, Some(tag));
                }
                None => self.ws_catalog_leave_into(),
            }
        }
        if let (Some(t), Some(a)) = (switch, self.ws_gear.catalog.as_ref().and_then(|c| c.anchor.clone())) {
            self.ws_remember_into_kind(&a, t);
            self.ws_catalog_into(page, &a, Some(t));
        }
    }

    /// Items of the page that can hold kind `tag`: (guid, name).
    fn ws_catalog_parents(&self, sec: &chummer_core::sections::Section, tag: &str, lang: &Language) -> Vec<(String, String)> {
        let mut out = Vec::new();
        fn walk(nodes: &[chummer_core::tree::ItemNode], f: &mut dyn FnMut(&Element)) {
            for n in nodes {
                if let chummer_core::tree::Entry::Item { el, .. } = &n.value {
                    f(el);
                }
                walk(&n.children, f);
            }
        }
        walk(&chummer_core::tree::section_tree(&self.doc.doc, sec), &mut |el| {
            let g = el.get("guid");
            if !g.is_empty() && place::accepts(&self.doc, &self.store, &g).iter().any(|k| k.tag == tag) {
                out.push((g, super::display_name(sec, el, lang)));
            }
        });
        out
    }

    /// An inventory row was selected: when the item takes other items,
    /// the catalog adds into it ([`Self::ws_catalog_into`]); else it adds
    /// at the top level, in the page's own kinds.
    pub(crate) fn ws_catalog_target(&mut self, page: Page, guid: &str) {
        let Some(c) = self.ws_gear.catalog.as_mut().filter(|c| c.page == page) else { return };
        // The record stays selected (for the target); the inspector shows
        // the item.
        c.inspect = false;
        if !self.ws_catalog_into(page, guid, None) {
            self.ws_catalog_leave_into();
        }
    }

    /// Add into `guid`: the catalog sells the kinds it takes, showing
    /// `prefer`, else the kind last chosen for items of its type this
    /// session, else the most natural one (`place::accepts` order), with
    /// the categories it takes as the category filter. Returns false
    /// when it takes nothing (the catalog is left as it was).
    pub(crate) fn ws_catalog_into(&mut self, page: Page, guid: &str, prefer: Option<&str>) -> bool {
        let accepted = place::accepts(&self.doc, &self.store, guid);
        if accepted.is_empty() {
            return false;
        }
        let item_tag = edit::find(&self.doc, guid).map(|e| edit::tag_of(e).to_owned()).unwrap_or_default();
        let has = |t: &str| accepted.iter().any(|a| a.tag == t);
        let remembered = self.ws_gear.into_kinds.get(&item_tag).copied();
        let want = prefer.filter(|t| has(t)).or(remembered.filter(|t| has(t))).unwrap_or(accepted[0].tag);
        let tags: Vec<&str> = accepted.iter().map(|a| a.tag).collect();
        let cats = accepted.iter().find(|a| a.tag == want).map(|a| a.categories.clone()).unwrap_or_default();
        let store = self.store.clone();
        let Some(c) = self.ws_gear.catalog.as_mut().filter(|c| c.page == page) else { return false };
        let fresh = c.anchor.as_deref().is_none_or(|a| !a.eq_ignore_ascii_case(guid)) || c.shown() != Some(want);
        c.set_kinds(&tags, Some(want), &store);
        if fresh {
            // The categories it takes, as the data spells them.
            c.categories.clear();
            if let Some(slot) = c.slots.iter().find(|s| s.on) {
                let recs = data::records(&slot.doc, slot.kind.data_container, slot.kind.data_item);
                for w in &cats {
                    if let Some(r) = recs.iter().find(|r| r.category().eq_ignore_ascii_case(w)) {
                        c.categories.insert(r.category());
                    }
                }
            }
        }
        c.anchor = Some(guid.to_owned());
        c.purchase.parent = Some(guid.to_owned());
        c.location = None;
        c.target_note = None;
        c.retarget_key = 0;
        true
    }

    /// Stop adding into an item: the top level, in the page's own kinds.
    pub(crate) fn ws_catalog_leave_into(&mut self) {
        let store = self.store.clone();
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let tags = page_kinds(super::ws_items::page_container(c.page));
        if c.anchor.take().is_some() && !tags.is_empty() {
            c.set_kinds(tags, None, &store);
            c.categories.clear();
        } else if c.slots.iter().all(|s| !s.on || select::parent_of(s.kind.tag).is_some()) {
            // A kind that needs a container: back to the page's own kind.
            for (i, s) in c.slots.iter_mut().enumerate() {
                s.on = i == 0;
            }
            c.selected = None;
        }
        c.purchase.parent = None;
        c.location = None;
        c.target_note = None;
    }

    /// Remember the kind chosen for adding into items like `guid`.
    pub(crate) fn ws_remember_into_kind(&mut self, guid: &str, tag: &'static str) {
        if let Some(e) = edit::find(&self.doc, guid) {
            self.ws_gear.into_kinds.insert(edit::tag_of(e).to_owned(), tag);
        }
    }

    /// Add into `guid` from its inspector's "Add …" or the inventory's
    /// hint: open the catalog on the page when it is closed, select the
    /// item and switch the catalog to kind `prefer` (remembered).
    pub(crate) fn ws_add_into(&mut self, page: Page, guid: &str, prefer: Option<&'static str>) {
        if self.ws_gear.catalog.as_ref().is_none_or(|c| c.page != page) {
            let Some(first) = page_kinds(super::ws_items::page_container(page)).first() else { return };
            self.ws_open_catalog(page, first);
        }
        if self.item_editor.as_ref().is_none_or(|(g, _)| g != guid) {
            self.item_editor = Some((guid.to_owned(), crate::item_editor::ItemEditor::default()));
        }
        if let Some(t) = prefer {
            self.ws_remember_into_kind(guid, t);
        }
        if let Some(c) = self.ws_gear.catalog.as_mut() {
            c.inspect = false;
        }
        self.ws_catalog_into(page, guid, prefer);
        self.ws_gear.focus_catalog = true;
    }

    /// A group's "+": add into location `loc` (its guid).
    pub(crate) fn ws_catalog_set_location(&mut self, loc: Option<String>, _lang: &Language) {
        let names: Vec<(String, String)> = {
            let Some(c) = self.ws_gear.catalog.as_ref() else { return };
            let container = super::ws_items::page_container(c.page);
            let tag = page_kinds(container).first().copied().unwrap_or("");
            let lc = match tag {
                "gear" => "gearlocations",
                "armor" => "armorlocations",
                "weapon" => "weaponlocations",
                "vehicle" => "vehiclelocations",
                _ => "",
            };
            self.doc.doc.child(lc).map(|l| l.children_named("location").map(|e| (e.get("guid"), e.get("name"))).collect()).unwrap_or_default()
        };
        let loc = loc.and_then(|g| names.into_iter().find(|(lg, _)| *lg == g));
        if loc.is_some() {
            self.ws_catalog_leave_into();
        }
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        c.location = loc;
    }

    /// Whether the selected record goes into the anchor (at the chosen
    /// rating): the reason in the target bar when it does not (Add then
    /// refuses it). An anchor that is gone ends adding into it.
    pub(crate) fn ws_catalog_retarget(&mut self) {
        let enforce = self.settings.as_ref().is_none_or(|s| s.flag("enforcecapacity"));
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let Some(anchor) = c.anchor.clone() else { return };
        let key = hash_of((&anchor, c.selected, c.purchase.rating, self.doc.revision()));
        if c.retarget_key == key {
            return;
        }
        let Some(names) = edit::find(&self.doc, &anchor).map(|e| e.get("name")) else {
            self.ws_catalog_leave_into();
            return;
        };
        let record = c.record().map(|(slot, r)| (slot.kind.tag, r.el().clone(), r.name()));
        let rating = c.purchase.rating;
        let fit = record.as_ref().map(|(tag, rec, _)| place::Host::of(&self.doc, &self.store, &anchor).map_or(Err(place::Misfit::Missing), |h| h.takes(tag, rec, None, rating, enforce, false)));
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        c.retarget_key = key;
        c.target_note = match (fit, record) {
            (Some(Err(m)), Some((_, _, n))) => Some((m, n, names)),
            _ => None,
        };
    }

    /// The catalog's target container (tests).
    #[cfg(test)]
    pub(crate) fn ws_catalog_target_guid(&self) -> Option<String> {
        self.ws_gear.catalog.as_ref().and_then(|c| c.target().map(str::to_owned))
    }

    /// The kinds the catalog sells and the one it shows (tests).
    #[cfg(test)]
    pub(crate) fn ws_catalog_kinds(&self) -> (Vec<&'static str>, Option<&'static str>) {
        self.ws_gear.catalog.as_ref().map_or((Vec::new(), None), |c| (c.tags(), c.shown()))
    }

    /// The selected record's preview after the purchase: (essence after,
    /// nuyen after, what), for the budget strip.
    pub(crate) fn ws_catalog_preview_after(&self) -> Option<(f64, f64, String)> {
        let c = self.ws_gear.catalog.as_ref()?;
        if self.ws_item_page(self.tab) != Some(c.page) {
            return None;
        }
        let (slot, r) = c.record()?;
        let (_, Ok(p)) = c.preview.as_ref()? else { return None };
        let name = r.name();
        let what = if c.purchase.rating > 0 && select::rating_max(r) > 0 { format!("{name} {}", c.purchase.rating) } else { name };
        let _ = slot;
        Some((p.sheet.essence, p.nuyen, what))
    }

    /// The results as table rows (kept until the rows or the sort change).
    fn ws_catalog_table_rows(&mut self, cols: &[Col], sort: Option<(String, bool)>, stacked: bool, lang: &Language) {
        let Some(c) = self.ws_gear.catalog.as_mut() else { return };
        let Some((rows_key, rows)) = &c.rows else { return };
        let key = hash_of((rows_key, &sort, stacked, cols.len()));
        if c.table.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let col = |k: &str| cols.iter().position(|c| c.key == k);
        let mut out: Vec<table::Row> = rows
            .list
            .iter()
            .map(|r| {
                let tag = c.slots.get(r.slot).map_or("gear", |s| s.kind.tag);
                let mut cells = vec![Cell::default(); cols.len() - 1];
                let mut set = |k: &str, cell: Cell| {
                    if let Some(i) = col(k) {
                        cells[i - 1] = cell;
                    }
                };
                set("rating", if r.rating > 0 { Cell::num(r.rating.to_string(), Some(r.rating as f64)) } else { Cell::text("—").tone(Tone::Muted) });
                set("ess", r.ess.map_or_else(|| Cell::text("—").tone(Tone::Muted), |e| Cell::num(format!("{e:.2}"), Some(e))));
                let over = r.over || r.avail.as_ref().is_some_and(|a| a.legality == Legality::Forbidden);
                set("avail", r.avail.as_ref().map_or_else(|| Cell::text("—").tone(Tone::Muted), |a| Cell::num(a.to_string(), Some(a.value as f64)).tone(if over { Tone::Warn } else { Tone::Normal })));
                set("cost", r.cost.map_or_else(|| Cell::text("—").tone(Tone::Muted), |v| Cell::num(format::nuyen(v), Some(v))));
                set("source", r.source.as_ref().map_or_else(Cell::default, |s| Cell::text(format!("{} {}", s.book, s.page)).tone(Tone::Muted).tip(s.to_string())));
                for (k, v) in r.extra.iter().enumerate() {
                    if let Some(i) = cols.iter().position(|c| c.key == EXTRA_KEYS.get(k).copied().unwrap_or("")) {
                        cells[i - 1] = Cell::num(v.clone(), v.trim().parse::<f64>().ok());
                    }
                }
                let sub = match &r.owned {
                    Some(o) => Tag { text: o.clone(), tone: Tone::Teal, chip: false, icon: Some(icons::CHECK) },
                    None => Tag::text(r.sub.clone(), Tone::Muted),
                };
                let (sub, tags) = if stacked { (None, if sub.text.is_empty() { vec![] } else { vec![sub] }) } else { (Some(sub), vec![]) };
                let add = Action::new("add", icons::PLUS, lang.tr_fmt("Add {0}", &[&r.name])).primary();
                table::Row::new(
                    format!("{}:{}", r.slot, r.index),
                    RowData {
                        kind: RowKind::Item,
                        name: r.name.clone(),
                        icon: Some(kind_icon(tag)),
                        tags,
                        sub,
                        cells,
                        dim: r.why.first().cloned(),
                        hover: if r.why.len() > 1 { r.why.join("\n") } else { String::new() },
                        selectable: true,
                        actions: if r.why.is_empty() { vec![add] } else { vec![] },
                        ..Default::default()
                    },
                )
            })
            .collect();
        if let Some((k, asc)) = &sort {
            if let Some(i) = col(k) {
                table::sort_tree(&mut out, i, *asc);
            }
        }
        c.table = Some((key, out));
    }

    /// The results table. Returns `Some(to_inventory)` when a row asks to
    /// be added (double click: add and stay).
    #[allow(clippy::too_many_arguments)]
    fn ws_catalog_results(&mut self, ui: &mut egui::Ui, height: f32, stacked: bool, fold: bool, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<bool> {
        let (cols, salt) = {
            let c = self.ws_gear.catalog.as_ref()?;
            let first = c.slots.iter().find(|s| s.on).map(|s| s.kind.tag).unwrap_or("gear");
            let extra: Vec<&str> = select::columns(first).iter().filter(|(_, f)| !matches!(*f, "avail" | "cost" | "ess" | "rating")).map(|(h, _)| *h).collect();
            let mut cols = vec![Col::name(lang.tr("Name")), Col::new("rating", "R").px(24.0).num().prio(70)];
            for (k, h) in extra.iter().enumerate().take(EXTRA_KEYS.len()) {
                cols.push(Col::new(EXTRA_KEYS[k], lang.tr(h)).px(52.0).mono().prio(20 - k as u8));
            }
            if c.ware() {
                cols.push(Col::new("ess", lang.tr("Ess")).px(40.0).num().prio(80));
            }
            cols.push(Col::new("avail", lang.tr("Avail")).px(40.0).num().prio(60));
            cols.push(Col::new("cost", lang.tr("Cost")).px(72.0).num().prio(90));
            cols.push(Col::new("source", lang.tr("Source")).px(60.0).prio(5));
            cols.push(Col::actions(26.0));
            (cols, ("ws_catalog", c.page.0 as u8, c.page.1, first))
        };
        let id = table::table_id(salt);
        let sort = table::sort_of(ui.ctx(), id);
        self.ws_catalog_table_rows(&cols, sort, stacked, lang);
        let c = self.ws_gear.catalog.as_mut()?;
        let (key, mut rows) = c.table.take()?;
        let selected = c.selected.map(|(s, i)| format!("{s}:{i}"));
        let scroll = std::mem::take(&mut c.scroll);
        // The details strip under the selected row.
        let detail_at = selected.as_ref().filter(|_| fold).and_then(|k| rows.iter().position(|r| r.key == *k));
        if let Some(i) = detail_at {
            rows[i].children.push(table::Row::new("detail", RowData { kind: RowKind::Detail, ..Default::default() }));
        }
        let states = States { selected: selected.as_deref(), scroll_to: if scroll { selected.as_deref() } else { None }, ..Default::default() };
        let empty = table::EmptyCard { title: lang.tr("Nothing matches."), sub: lang.tr("Clear a filter or change the search."), button: lang.tr("Clear filters") };
        let row_h = if stacked { 30.0 } else { 36.0 };
        let mut strip_add = false;
        let t = table::Table::new(salt, &cols).height(height).row_height(row_h).flat().empty(empty).detail_height(54.0).draggable();
        let events = {
            let (sheet, decimals, nuyen_now) = (&self.sheet, self.rules.essence_decimals, self.budget.as_ref().map_or(self.doc.nuyen, |b| b.nuyen_left()));
            let max_avail = (!self.doc.created).then(|| self.settings.as_ref().map_or(12, |s| s.max_availability())).filter(|m| *m > 0);
            let c = self.ws_gear.catalog.as_mut()?;
            t.show(ui, &rows, &states, lang, |ui, _| {
                strip_add |= details_strip(ui, c, sheet, decimals, nuyen_now, max_avail, lang, pdfs, status);
            })
        };
        if let Some(i) = detail_at {
            rows[i].children.clear();
        }
        let c = self.ws_gear.catalog.as_mut()?;
        c.table = Some((key, rows));
        let mut out = strip_add.then_some(false);
        let rating_of = |c: &Catalog, k: &str| -> Option<(usize, usize, i32)> {
            let (s, i) = k.split_once(':')?;
            let (s, i) = (s.parse().ok()?, i.parse().ok()?);
            let r = c.rows.as_ref()?.1.list.iter().find(|r| r.slot == s && r.index == i)?;
            Some((s, i, r.rating))
        };
        for e in events {
            match e {
                Event::Select(k) => {
                    if let Some((s, i, r)) = rating_of(c, &k) {
                        c.select(s, i, r);
                    }
                }
                Event::Open(k) | Event::Action(k, "add", _) => {
                    if let Some((s, i, r)) = rating_of(c, &k) {
                        c.select(s, i, r);
                        out = Some(false);
                    }
                }
                Event::AddInto(_) => c.clear_filters(),
                _ => {}
            }
        }
        out
    }

    /// Add the selected record (`Command::AddItem`, as the selection
    /// dialog does). A bonus selection is asked first, in the inspector.
    /// `close` closes the catalog afterwards. Returns true if the
    /// character changed.
    fn ws_catalog_add(&mut self, lang: &Language, status: &mut Status, to_inventory: bool) -> bool {
        let store = self.store.clone();
        // The purchase rules for the place (as the rows show them).
        if let Some(c) = self.ws_gear.catalog.as_ref() {
            if let (Some((slot, r)), Some(pg)) = (c.record(), c.purchase.parent.as_deref()) {
                let enforce = self.settings.as_ref().is_none_or(|s| s.flag("enforcecapacity"));
                if let Some(h) = place::Host::of(&self.doc, &store, pg) {
                    if let Err(m) = h.takes(slot.kind.tag, r.el(), None, c.purchase.rating, enforce, false) {
                        let sec = super::ws_items::section_of(c.page);
                        let name = lang.data_name(slot.kind.file, &r.id(), &r.name());
                        *status = Some((super::ws_inventory::misfit_text(lang, &m, &name, &super::display_name(&sec, &h.el, lang)), true));
                        return false;
                    }
                }
            }
        }
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
        let sec = super::ws_items::section_of(c.page);
        // A location's "+": one command (the add puts it there).
        purchase.location = c.location.as_ref().filter(|_| purchase.parent.is_none()).map(|(g, _)| g.clone());
        let before = super::ws_items::section_guids(&self.doc, &sec);
        match self.doc.apply(Command::AddItem { tag: tag.to_owned(), record: RecordRef::of(Record(&rec)), purchase }) {
            Ok(rep) => {
                *status = rep.message.map(|m| (m, false));
                if let Some(c) = self.ws_gear.catalog.as_mut() {
                    c.answer = None;
                    c.purchase.answer = None;
                }
                // The new item: the one added whose parent is not new too.
                let after = super::ws_items::section_guids(&self.doc, &sec);
                let new: Vec<&String> = after.difference(&before).collect();
                let top = new.iter().find(|g| edit::parent(&self.doc, g).is_none_or(|p| !new.contains(&&p.get("guid")))).map(|g| (*g).clone());
                if let Some(g) = top {
                    self.ws_record_add(g);
                }
                if to_inventory {
                    self.ws_gear.focus_catalog = false;
                    if let Some(c) = self.ws_gear.catalog.as_mut() {
                        c.focus_search = false;
                    }
                }
                true
            }
            Err(e) => {
                *status = Some((e.reason, true));
                false
            }
        }
    }

    /// The catalog record a results row key ("slot:index") names: (kind,
    /// data element, the rating the table shows, name).
    pub(crate) fn ws_catalog_record_at(&self, key: &str) -> Option<(String, Element, i32, String)> {
        let c = self.ws_gear.catalog.as_ref()?;
        let (s, i) = key.split_once(':')?;
        let (s, i): (usize, usize) = (s.parse().ok()?, i.parse().ok()?);
        let slot = c.slots.get(s)?;
        let rec = *data::records(&slot.doc, slot.kind.data_container, slot.kind.data_item).get(i)?;
        let rating = c.rows.as_ref().and_then(|(_, r)| r.list.iter().find(|r| r.slot == s && r.index == i)).map_or(0, |r| r.rating);
        Some((slot.kind.tag.to_owned(), rec.el().clone(), rating, rec.name()))
    }

    /// A catalog row dropped on the inventory: select it and buy it
    /// into that place (the same path as Add; a bonus question is asked
    /// in the inspector first). Returns true if the character changed.
    pub(crate) fn ws_catalog_drop_buy(&mut self, key: &str, to: place::Dest, lang: &Language, status: &mut Status) -> bool {
        let Some((s, i)) = key.split_once(':').and_then(|(s, i)| Some((s.parse::<usize>().ok()?, i.parse::<usize>().ok()?))) else { return false };
        let rating = self.ws_catalog_record_at(key).map_or(0, |r| r.2);
        let names: Vec<(String, String)> = {
            let Some(c) = self.ws_gear.catalog.as_ref() else { return false };
            let container = super::ws_items::page_container(c.page);
            let lc = match page_kinds(container).first().copied().unwrap_or("") {
                "gear" => "gearlocations",
                "armor" => "armorlocations",
                "weapon" => "weaponlocations",
                "vehicle" => "vehiclelocations",
                _ => "",
            };
            self.doc.doc.child(lc).map(|l| l.children_named("location").map(|e| (e.get("guid"), e.get("name"))).collect()).unwrap_or_default()
        };
        let Some(c) = self.ws_gear.catalog.as_mut() else { return false };
        c.select(s, i, rating);
        // Bought where it was dropped; the catalog's target stays.
        let keep = (c.purchase.parent.clone(), c.location.clone());
        match to {
            place::Dest::Item(g) => {
                c.purchase.parent = Some(g);
                c.location = None;
            }
            place::Dest::Top => {
                c.purchase.parent = None;
                c.location = None;
            }
            place::Dest::Location(l) => {
                c.purchase.parent = None;
                c.location = names.into_iter().find(|(g, _)| *g == l);
            }
        }
        self.item_editor = None;
        let added = self.ws_catalog_add(lang, status, false);
        if let Some(c) = self.ws_gear.catalog.as_mut() {
            (c.purchase.parent, c.location) = keep;
        }
        added
    }

    /// Compute the preview of the selected record when it changed.
    pub(crate) fn ws_catalog_preview(&mut self, engine: &Engine) {
        let Some(c) = self.ws_gear.catalog.as_ref() else { return };
        let Some((slot, r)) = c.record() else { return };
        let p = &c.purchase;
        let key = hash_of((slot.kind.tag, c.selected, p.rating, &p.grade, p.qty.to_bits(), &p.parent, &p.answer, self.doc.revision()));
        if c.preview.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let result = crate::trace::time("catalog preview (clone+apply+compute)", || preview(&self.doc, &self.sheet, self.settings.as_ref(), engine, slot.kind.tag, r.el(), p));
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
        let target_name = self.ws_gear.catalog.as_ref().and_then(|c| c.purchase.parent.clone()).and_then(|g| items::edit::find(&self.doc, &g).map(|e| e.get("name")));
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
                widgets::rating_stepper(ui, &mut c.purchase.rating, min.max(1), max_rating, &lang.tr("Lower Rating"), &lang.tr("Raise Rating"));
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
                widgets::qty_stepper(ui, "ws_catalog_qty", &mut c.purchase.qty, 1.0, 1000.0, 1.0, 0, &lang.tr("Lower"), &lang.tr("Raise"));
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
        widgets::rule(ui);
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
            pool_lines(ui, &a.pools, lang);
        }
        let capacity = r.get("capacity");
        if !capacity.trim().is_empty() {
            widgets::value_row(ui, &lang.tr("Capacity"), &capacity, ws.muted, "");
        }

        // Checks.
        widgets::rule(ui);
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
            widgets::rule(ui);
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
                let label = match &target_name {
                    Some(t) => lang.tr_fmt("Add to {0}", &[t]),
                    None if c.purchase.rating > 0 && max_rating > 0 => lang.tr_fmt("Add {0}", &[&format!("{name} {}", c.purchase.rating)]),
                    None => lang.tr_fmt("Add {0}", &[&name]),
                };
                let r = ui.add_enabled_ui(can, |ui| widgets::button(ui, Some(icons::PLUS), &label, Look::Primary, 30.0)).inner;
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
            widgets::rule(ui);
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
            Some(close) => self.ws_catalog_add(lang, status, close),
            None => false,
        }
    }
}

/// How many skill lines the preview lists before "and N more".
const SKILL_LINES: usize = 6;

/// The preview's dice pools (in [`pool_diff::diff`]'s order: fixed
/// pools, skills, weapons): "Pistols 12 → 14", a weapon's changed values
/// under its name.
fn pool_lines(ui: &mut egui::Ui, lines: &[pool_diff::Line], lang: &Language) {
    use pool_diff::Subject;
    if lines.is_empty() {
        return;
    }
    let ws = theme::ws(ui);
    widgets::rule(ui);
    ui.label(widgets::overline(&lang.tr("Dice Pools"), &ws));
    let row = |ui: &mut egui::Ui, label: &str, indent: bool, part: &pool_diff::Part| {
        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            if indent {
                ui.add_space(12.0);
            }
            ui.add(egui::Label::new(RichText::new(label).size(12.5).color(if indent { ws.muted } else { ws.text })).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(&part.after, 12.5, ws.accent));
                if let Some(b) = &part.before {
                    ui.label(icons::icon(icons::ARROW_RIGHT, 11.0, ws.muted));
                    ui.label(widgets::mono(b, 12.0, ws.muted));
                }
            });
        });
    };
    let skill_name = |n: &str| lang.data_name("skills.xml", "", n);
    let mut skills = 0;
    let mut hidden: Vec<String> = Vec::new();
    let flush = |ui: &mut egui::Ui, hidden: &mut Vec<String>| {
        if !hidden.is_empty() {
            ui.label(RichText::new(lang.tr_fmt("and {0} more", &[&hidden.len()])).size(11.5).color(ws.muted)).on_hover_text(hidden.join("\n"));
            hidden.clear();
        }
    };
    for l in lines {
        match &l.subject {
            Subject::Fixed(label) => row(ui, &lang.tr(label), false, &l.parts[0]),
            Subject::Skill(n) => {
                let p = &l.parts[0];
                if skills < SKILL_LINES {
                    row(ui, &skill_name(n), false, p);
                } else {
                    hidden.push(format!("{} {} → {}", skill_name(n), p.before.as_deref().unwrap_or("—"), p.after));
                }
                skills += 1;
            }
            Subject::Weapon { name, new } => {
                flush(ui, &mut hidden);
                let name = lang.data_name("weapons.xml", "", name);
                let title = if *new { format!("{name} ({})", lang.tr("New")) } else { name };
                match l.parts.as_slice() {
                    // Only its pool: one line ("Ares Predator V 12 → 14").
                    [p] if p.field == pool_diff::Field::Pool => row(ui, &title, false, p),
                    parts => {
                        ui.add(egui::Label::new(RichText::new(title).size(12.5).color(ws.text)).truncate());
                        for p in parts {
                            row(ui, &lang.tr(p.field.label()), true, p);
                        }
                    }
                }
            }
        }
    }
    flush(ui, &mut hidden);
}

/// Keys of the kinds' own columns in the results table.
const EXTRA_KEYS: [&str; 4] = ["x0", "x1", "x2", "x3"];

/// "Cyber" for Cyberware in the kind switch.
fn short_kind(lang: &Language, label: &str) -> String {
    match label {
        "Cyberware" => lang.tr("Cyber"),
        "Bioware" => lang.tr("Bio"),
        l => lang.tr(l),
    }
}

/// An accent chip with an ×; returns whether it was clicked.
fn chip_x(ui: &mut egui::Ui, text: &str, ws: &theme::WsPalette) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.0), ws.accent);
    let w = galley.size().x + 14.0 + 14.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 18.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(9), ws.hover);
    }
    p.rect_stroke(rect, CornerRadius::same(9), egui::Stroke::new(1.0_f32, ws.primary), egui::StrokeKind::Inside);
    p.galley(egui::pos2(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0), galley, ws.accent);
    icons::paint(p, egui::Rect::from_min_size(egui::pos2(rect.right() - 16.0, rect.top() + 2.0), egui::vec2(12.0, 14.0)), icons::X, 10.0, ws.accent);
    resp.on_hover_text(text).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The top-level item an item is in (itself at the top).
fn top_ancestor(ch: &Character, guid: &str) -> Element {
    let mut cur = edit::find(ch, guid).cloned().unwrap_or_else(|| Element::new("none"));
    for _ in 0..16 {
        match edit::parent(ch, &cur.get("guid")) {
            Some(p) => cur = p.clone(),
            None => break,
        }
    }
    cur
}

/// The inspector folded under the selected catalog row (narrow
/// windows): rating, grade, essence before → after, nuyen after,
/// availability, the source and Add. Returns whether Add was clicked.
#[allow(clippy::too_many_arguments)]
fn details_strip(ui: &mut egui::Ui, c: &mut Catalog, sheet: &Sheet, decimals: u32, nuyen_now: f64, max_avail: Option<i32>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
    let ws = theme::ws(ui);
    let Some((s, i)) = c.selected else { return false };
    let Some(slot) = c.slots.get(s) else { return false };
    let doc = slot.doc.clone();
    let recs = data::records(&doc, slot.kind.data_container, slot.kind.data_item);
    let Some(r) = recs.get(i).copied() else { return false };
    let grade = c.grade(slot);
    let grade_list = slot.grades.clone();
    let mut add = false;
    ui.spacing_mut().item_spacing.x = 14.0;
    ui.add_space(24.0);
    let max_rating = select::rating_max(r);
    if max_rating > 0 {
        let min = r.el().get_i32("minrating").unwrap_or(1).clamp(1, max_rating);
        c.purchase.rating = c.purchase.rating.clamp(min, max_rating);
        widgets::rating_stepper(ui, &mut c.purchase.rating, min, max_rating, &lang.tr("Lower Rating"), &lang.tr("Raise Rating"));
    }
    if !grade_list.is_empty() {
        let cur = grade.as_ref().map_or_else(|| "Standard".to_owned(), |g| g.name.clone());
        crate::combo::Combo::from_id_salt("ws_catalog_strip_grade").selected_text(cur.clone()).width(110.0).show_ui(ui, |ui| {
            for g in &grade_list {
                if crate::combo::selectable_label(ui, cur == g.name, &g.name).clicked() {
                    c.purchase.grade = Some(g.name.clone());
                }
            }
        });
    }
    let after = c.preview.as_ref().and_then(|(_, p)| p.as_ref().ok());
    let stat = |ui: &mut egui::Ui, caption: &str, add: &dyn Fn(&mut egui::Ui)| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(widgets::overline(caption, &ws));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                add(ui);
            });
        });
    };
    if !grade_list.is_empty() {
        let e = after.map(|a| a.sheet.essence);
        stat(ui, &lang.tr("Essence"), &|ui| {
            ui.label(widgets::mono(format::essence(sheet.essence, decimals), 12.0, ws.muted));
            if let Some(e) = e {
                ui.label(icons::icon(icons::ARROW_RIGHT, 10.0, ws.muted));
                ui.label(widgets::mono(format::essence(e, decimals), 12.5, if e < 0.0 { ws.error } else { ws.accent }));
            }
        });
    }
    if let Some(a) = after {
        let n = a.nuyen;
        stat(ui, &lang.tr("Nuyen after"), &|ui| {
            ui.label(widgets::mono(format::nuyen(n), 12.5, if n < 0.0 { ws.error } else { ws.accent }));
        });
    } else if let Some(v) = select::preview_cost(r, &c.purchase) {
        stat(ui, &lang.tr("Nuyen after"), &|ui| {
            ui.label(widgets::mono(format::nuyen(nuyen_now - v), 12.5, ws.accent));
        });
    }
    if let Some(av) = avail_at(r, c.purchase.rating, grade.as_ref()) {
        let ok = max_avail.is_none_or(|m| av.add_to_parent || av.value <= m);
        stat(ui, &lang.tr("Avail"), &|ui| {
            ui.label(widgets::mono(av.to_string(), 12.5, if ok { ws.text } else { ws.warning }));
            ui.label(icons::icon(if ok { icons::CHECK } else { icons::WARNING }, 11.0, if ok { ws.stun } else { ws.warning }));
        });
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        let blocked = after.is_some_and(|a| a.refused.is_some());
        if ui.add_enabled_ui(!blocked, |ui| widgets::button(ui, Some(icons::PLUS), &lang.tr("Add"), Look::Primary, 26.0)).inner.on_hover_text(lang.tr("Add (Enter)")).clicked() {
            add = true;
        }
        if let Some(src) = SourceRef::of(r.el()) {
            if widgets::button(ui, Some(icons::BOOK_OPEN), &src.to_string(), Look::Ghost, 24.0).clicked() {
                pdf_ui::open(pdfs, &src, status);
            }
        }
    });
    add
}
