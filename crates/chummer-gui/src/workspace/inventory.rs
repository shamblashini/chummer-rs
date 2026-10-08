//! The Workspace item pages' inventory: the section's items as a
//! [`table`](crate::workspace::table) with the page's columns (ware:
//! rating, grade, essence, capacity, wireless, availability, cost,
//! source; weapons: pool, damage, AP, accuracy, mode, RC, ammunition,
//! equipped, wireless, cost; …), group rows with subtotals, and the
//! row actions. Every edit is the same `Command` as in the inspector
//! (`ws_inspector`); removing asks first (the view's `confirm_remove`),
//! career mode sells.
//!
//! The rows are built once per revision of the character (and sort,
//! language and mode) and kept in a [`Memo`](crate::memo::Memo).
//!
//! A child module of `view` (declared there with `#[path]`).

use std::collections::HashSet;
use std::sync::Arc;

use chummer_core::command::Command;
use chummer_core::format;
use chummer_core::items::edit;
use chummer_core::lang::Language;
use chummer_core::play::ammo;
use chummer_core::sections::Section as Sec;
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::tree::{Entry, ItemNode, Label, Node};
use chummer_core::xml::Element;
use eframe::egui;

use super::ws_items::Page;
use super::{display_name, CharacterView};
use crate::pdf_ui::{self, Status};
use crate::workspace::icons;
use crate::workspace::table::{self, Action, Cell, Col, Edit, Event, Kind, Row, RowData, States, Tag, Tone};

/// The Drugs page's items (no tree section of their own).
pub const DRUGS: Sec = Sec { label: "Drugs", container: "drugs", item: "drug", columns: &[], child_containers: &[], data_file: "drugs.xml" };

/// What the rows are built for (besides the revision).
pub type RowsKey = (&'static str, String, Option<(String, bool)>, bool);

/// The inventory's columns for a section's container.
pub fn columns(container: &str, career: bool, lang: &Language) -> Vec<Col> {
    let t = |s: &str| lang.tr(s);
    let actions = Col::actions(if career { 98.0 } else { 74.0 });
    let rating = Col::new("rating", t("Rating")).px(64.0).center().mono().prio(70).fold();
    let avail = Col::new("avail", t("Avail")).px(42.0).num().prio(30);
    let cost = Col::new("cost", t("Cost")).px(80.0).num().prio(90).sum();
    let source = Col::new("source", t("Source")).px(56.0).prio(10);
    let wireless = Col::new("wireless", t("Wireless")).px(22.0).icon(icons::WIFI_HIGH).prio(20).unsorted();
    let equipped = Col::new("equipped", t("Equipped")).px(22.0).icon(icons::CHECK_SQUARE).prio(20).unsorted();
    let mut v = vec![Col::name(t("Name"))];
    match container {
        "cyberwares" => v.extend([
            rating,
            Col::new("grade", t("Grade")).px(48.0).prio(50).fold(),
            Col::new("ess", t("Ess")).px(46.0).num().prio(80).sum(),
            Col::new("cap", t("Cap")).px(50.0).num().prio(40),
            wireless,
            avail,
            cost,
            source,
        ]),
        "gears" => v.extend([rating, Col::new("qty", t("Qty")).px(52.0).num().prio(60), wireless, avail, cost, source]),
        "armors" => v.extend([
            Col::new("armor", t("Armor")).px(44.0).num().prio(75),
            rating,
            Col::new("cap", t("Cap")).px(50.0).num().prio(40),
            equipped,
            wireless,
            avail,
            cost,
            source,
        ]),
        "weapons" => v.extend([
            Col::new("pool", t("Pool")).px(40.0).num().prio(85),
            Col::new("dv", t("DV")).px(56.0).num().prio(75),
            Col::new("ap", t("AP")).px(32.0).num().prio(65),
            Col::new("acc", t("Acc")).px(44.0).num().prio(60),
            Col::new("mode", t("Mode")).px(64.0).prio(35),
            Col::new("rc", t("RC")).px(30.0).num().prio(33),
            Col::new("ammo", t("Ammo")).px(if career { 116.0 } else { 64.0 }).mono().prio(31).unsorted(),
            equipped,
            wireless,
            avail.prio(25),
            cost.px(72.0),
        ]),
        "vehicles" => v.extend([
            Col::new("handling", t("Handling")).px(64.0).num().prio(60),
            Col::new("speed", t("Speed")).px(56.0).num().prio(55),
            Col::new("accel", t("Accel")).px(48.0).num().prio(50),
            Col::new("body", t("Body")).px(40.0).num().prio(45),
            Col::new("armor", t("Armor")).px(44.0).num().prio(47),
            Col::new("pilot", t("Pilot")).px(40.0).num().prio(42),
            Col::new("sensor", t("Sensor")).px(48.0).num().prio(40),
            Col::new("cap", t("Slots")).px(50.0).num().prio(38),
            avail,
            cost,
            source,
        ]),
        "lifestyles" => v.extend([
            Col::new("lifestyle", t("Lifestyle")).px(110.0).prio(60),
            Col::new("months", t("Months")).px(56.0).num().prio(70),
            Col::new("roommates", t("Roommates")).px(76.0).num().prio(30),
            cost,
            source,
        ]),
        "drugs" => v.extend([Col::new("grade", t("Grade")).px(90.0).prio(50), Col::new("qty", t("Qty")).px(52.0).num().prio(60), cost, source]),
        _ => {}
    }
    v.push(actions);
    v
}

/// The icon of a kind of item.
pub fn kind_icon(tag: &str) -> &'static str {
    match tag {
        "cyberware" => icons::CPU,
        "bioware" => icons::DNA,
        "armor" => icons::T_SHIRT,
        "armormod" | "accessory" | "mod" => icons::WRENCH,
        "weapon" => icons::CROSSHAIR,
        "vehicle" => icons::CAR,
        "lifestyle" => icons::HOUSE_LINE,
        "drug" => icons::PILL,
        "weaponmount" => icons::CROSSHAIR,
        _ => icons::PACKAGE,
    }
}

/// "Alphaware" → "Alpha" (the table's grade column).
pub fn grade_short(g: &str) -> String {
    match g {
        "" | "None" => String::new(),
        "Standard" => "Std".into(),
        g => g.strip_suffix("ware").filter(|s| !s.is_empty()).map(|s| s.trim_end_matches(' ').to_owned()).unwrap_or_else(|| g.to_owned()),
    }
}

/// A number from a cell text ("6,000¥", "+2", "−1").
fn num_of(text: &str) -> Option<f64> {
    let t: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '−' | '+')).map(|c| if c == '−' { '-' } else { c }).collect();
    t.parse().ok()
}

/// What the row building needs from the view, borrowed once.
struct Ctx<'a> {
    v: &'a CharacterView,
    sec: &'a Sec,
    cols: &'a [Col],
    lang: &'a Language,
    marks: std::collections::HashMap<String, (String, bool)>,
    career: bool,
}

impl CharacterView {
    /// The rows of a section's inventory, from the memo when nothing
    /// changed.
    pub(super) fn ws_inventory_rows(&self, sec: &Sec, cols: &[Col], lang: &Language, sort: Option<(String, bool)>) -> Arc<Vec<Row>> {
        let key: RowsKey = (sec.container, lang.code.clone(), sort.clone(), self.doc.created);
        self.ws_gear.rows.get(self.doc.revision(), key, || {
            let _s = crate::trace::span("inventory rows");
            let cx = Ctx { v: self, sec, cols, lang, marks: self.item_marks(lang), career: self.doc.created };
            let mut rows = if sec.container == "drugs" {
                self.doc.items("drugs", "drug").iter().map(|d| cx.item(d, d.get("guid"), true, &[])).collect()
            } else {
                let tree = chummer_core::tree::section_tree(&self.doc.doc, sec);
                tree.iter().map(|n| cx.node(n)).collect::<Vec<Row>>()
            };
            if let Some((k, asc)) = &sort {
                if let Some(i) = cols.iter().position(|c| c.key == k) {
                    table::sort_tree(&mut rows, i, *asc);
                }
            }
            Arc::new(rows)
        })
    }
}

impl Ctx<'_> {
    fn node(&self, n: &ItemNode) -> Row {
        match &n.value {
            Entry::Group(label) => {
                let name = match label {
                    Label::Ui(s) => self.lang.tr(s),
                    Label::Text(s) => s.clone(),
                    Label::Grade(g) => format!("{} {g}", self.lang.tr("Grade")),
                };
                let items = n.children.iter().filter(|c| matches!(c.value, Entry::Item { .. })).count();
                let is_loc = n.key.contains("/loc/");
                let icon = if is_loc {
                    Some(icons::MAP_PIN)
                } else {
                    match label {
                        Label::Ui("Cyberware") | Label::Ui("Unequipped Modular Cyberware") => Some(icons::CPU),
                        Label::Ui("Bioware") | Label::Ui("Unequipped Modular Bioware") => Some(icons::DNA),
                        _ => None,
                    }
                };
                let top_group = !n.key.contains("/modcat/") && !n.key.ends_with("/weaponmounts") && n.key.starts_with(self.sec.container);
                let add_into = top_group.then(|| self.lang.tr_fmt("Add into {0}", &[&name]));
                let mut row = Node::new(
                    n.key.clone(),
                    RowData {
                        kind: Kind::Group,
                        name: name.clone(),
                        icon,
                        note: items_note(self.lang, items),
                        add_into,
                        cells: vec![Cell::default(); self.cols.len() - 1],
                        ..Default::default()
                    },
                );
                row.children = n.children.iter().map(|c| self.node(c)).collect();
                if row.children.is_empty() && is_loc {
                    row.children.push(Node::new(
                        format!("{}/empty", n.key),
                        RowData { kind: Kind::Empty, name: self.lang.tr_fmt("Nothing in {0}", &[&name]), note: String::new(), add_into: Some(self.lang.tr("Buy…")), ..Default::default() },
                    ));
                }
                row
            }
            Entry::Item { el, top } => {
                let kids: Vec<Row> = n.children.iter().map(|c| self.node(c)).collect();
                self.item(el, n.key.clone(), *top, &kids).with_children(kids)
            }
        }
    }

    /// An item's row (its children are added by the caller).
    fn item(&self, el: &Element, key: String, top: bool, kids: &[Row]) -> Row {
        let v = self.v;
        let lang = self.lang;
        let ch = &v.doc;
        let guid = el.get("guid");
        let tag = edit::tag_of(el).to_owned();
        let included = !guid.is_empty() && edit::is_included(ch, &guid);
        let creating = !self.career;
        let rating = el.get_i32("rating").unwrap_or(0);
        let range = if guid.is_empty() { None } else { edit::rating_range(ch, &v.store, &guid) };
        let total = if guid.is_empty() { super::cell(el, "cost").parse::<f64>().unwrap_or(0.0) } else { edit::total_cost(ch, &v.store, &guid) };
        // Own cost: the total less what the children's totals hold.
        let kids_total: f64 = item_children(kids).iter().map(|k| row_total(k, self)).sum();
        let own = (total - kids_total).max(0.0);
        let weapon = (el.name == "weapon").then(|| v.weapon_stats(el, true));
        let vstats = (el.name == "vehicle").then(|| chummer_core::items::vehicle::stats(el));
        let mut cells = Vec::with_capacity(self.cols.len());
        for c in &self.cols[1..] {
            let cell = match c.key {
                "rating" => match range {
                    Some((min, max)) => {
                        let mut cell = Cell::num(rating.to_string(), Some(rating as f64));
                        if creating && !included && max > min {
                            cell = cell.edit(Edit::Rating { value: rating, min, max });
                        }
                        cell
                    }
                    None if rating > 0 => Cell::num(rating.to_string(), Some(rating as f64)),
                    None => Cell::text("—").tone(Tone::Muted),
                },
                "grade" => {
                    let g = el.get("grade");
                    let s = grade_short(&g);
                    if s.is_empty() {
                        Cell::text("—").tone(Tone::Muted)
                    } else {
                        Cell::text(s).tone(if g == "Standard" { Tone::Muted } else { Tone::Accent }).tip(g)
                    }
                }
                "ess" => match (el.name == "cyberware").then(|| edit::essence(ch, &v.store, &v.rules, &guid)).flatten() {
                    Some(e) if e.abs() > 1e-9 || top => Cell::num(format::essence(e, v.rules.essence_decimals), Some(e)),
                    _ => Cell::text("—").tone(Tone::Muted),
                },
                "cap" => match edit::capacity(ch, &guid) {
                    Some((used, total)) => {
                        let f = chummer_core::improvement::fmt_num;
                        Cell::num(format!("{}/{}", f(used), f(total)), Some(total - used)).tone(if used > total { Tone::Error } else { Tone::Normal })
                    }
                    None => {
                        let raw = el.get(if el.name == "armormod" { "armorcapacity" } else { "capacity" });
                        let raw = raw.trim();
                        if raw.starts_with('[') {
                            let n = edit::parse_capacity(raw, rating).1;
                            Cell::num(format!("[{}]", chummer_core::improvement::fmt_num(n)), Some(n)).tone(Tone::Muted)
                        } else {
                            Cell::text("—").tone(Tone::Muted)
                        }
                    }
                },
                "wireless" => match el.child("wirelesson") {
                    Some(_) => {
                        let on = el.get_bool("wirelesson").unwrap_or(false);
                        let tip = if on { lang.tr("Wireless on (click to turn off)") } else { lang.tr("Wireless off (click to turn on)") };
                        Cell::default().edit(Edit::Toggle { on, on_icon: icons::WIFI_HIGH, off_icon: icons::WIFI_SLASH, tip })
                    }
                    None => Cell::default(),
                },
                "equipped" => {
                    if edit::can_equip(el) {
                        let on = el.get_bool("equipped").unwrap_or(true);
                        let tip = if on { lang.tr("Equipped (click to unequip)") } else { lang.tr("Not equipped (click to equip)") };
                        Cell::default().edit(Edit::Toggle { on, on_icon: icons::CHECK_SQUARE, off_icon: icons::SQUARE, tip })
                    } else {
                        Cell::default()
                    }
                }
                "avail" => {
                    let a = if guid.is_empty() { super::cell(el, "avail") } else { edit::availability(ch, &v.store, &guid) };
                    let forbidden = a.ends_with('F');
                    let n = num_of(a.trim_end_matches(['R', 'F']));
                    if a.trim().is_empty() {
                        Cell::text("—").tone(Tone::Muted)
                    } else {
                        Cell::num(a, n).tone(if forbidden { Tone::Warn } else if top { Tone::Normal } else { Tone::Muted })
                    }
                }
                "cost" => {
                    if included || (own == 0.0 && !top) {
                        Cell::num(lang.tr("incl."), Some(0.0)).tone(Tone::Muted)
                    } else {
                        Cell::num(format::nuyen(own), Some(own)).tone(if top { Tone::Normal } else { Tone::Muted })
                    }
                }
                "source" => match SourceRef::of(el) {
                    Some(r) => Cell::text(format!("{} {}", r.book, r.page)).tone(Tone::Muted).tip(r.to_string()),
                    None => Cell::default(),
                },
                "qty" => {
                    let q = el.get_f64("qty").unwrap_or(1.0);
                    let mut cell = Cell::num(format!("×{}", chummer_core::improvement::fmt_num(q)), Some(q));
                    if creating && !included && matches!(el.name.as_str(), "gear" | "drug") {
                        let step = el.get_f64("costfor").filter(|c| *c > 0.0).unwrap_or(1.0);
                        cell = cell.edit(Edit::Qty { value: q, step });
                    }
                    cell
                }
                "armor" if el.name == "vehicle" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.armor.to_string(), Some(s.armor as f64))),
                "armor" => {
                    let a = el.get("armor");
                    Cell::num(a.clone(), num_of(&a)).tone(if top { Tone::Normal } else { Tone::Muted })
                }
                "pool" => match &weapon {
                    Some(s) => Cell::num(s.dice_pool.to_string(), Some(s.dice_pool as f64)).tone(Tone::Accent),
                    None => Cell::default(),
                },
                "dv" => weapon.as_ref().map_or_else(Cell::default, |s| Cell::num(s.damage.clone(), num_of(&s.damage))),
                "ap" => weapon.as_ref().map_or_else(Cell::default, |s| Cell::num(s.ap.replace('-', "−"), num_of(&s.ap))),
                "acc" => weapon.as_ref().map_or_else(Cell::default, |s| Cell::num(s.accuracy.to_string(), Some(s.accuracy as f64))),
                "mode" => Cell::text(if weapon.is_some() { el.get("mode") } else { String::new() }),
                "rc" => match &weapon {
                    Some(s) => Cell::num(s.rc.clone(), num_of(&s.rc)),
                    None => {
                        let rc = el.get("rc");
                        Cell::num(rc.clone(), num_of(&rc)).tone(Tone::Muted)
                    }
                },
                "ammo" => {
                    if weapon.is_some() && ammo::uses_ammo(el) {
                        if self.career {
                            let cur = ammo::remaining(el);
                            let max = ammo::capacity(ch, el).max(cur);
                            let label = ammo::loaded(ch, el).map(|g| g.get("name").trim_start_matches("Ammo: ").to_owned()).unwrap_or_default();
                            Cell::num(format!("{cur}/{max}"), Some(cur as f64)).edit(Edit::Meter { cur, max, label })
                        } else {
                            Cell::text(el.get("ammo"))
                        }
                    } else {
                        Cell::default()
                    }
                }
                "handling" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.handling_text.clone(), num_of(&s.handling_text))),
                "speed" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.speed_text.clone(), num_of(&s.speed_text))),
                "accel" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.accel_text.clone(), num_of(&s.accel_text))),
                "body" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.body.to_string(), Some(s.body as f64))),
                "pilot" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.pilot.to_string(), Some(s.pilot as f64))),
                "sensor" => vstats.as_ref().map_or_else(Cell::default, |s| Cell::num(s.sensor.to_string(), Some(s.sensor as f64))),
                "lifestyle" => Cell::text(el.get("baselifestyle")),
                "months" => {
                    let m = el.get("months");
                    Cell::num(m.clone(), num_of(&m))
                }
                "roommates" => {
                    let m = el.get("roommates");
                    Cell::num(m.clone(), num_of(&m))
                }
                _ => Cell::default(),
            };
            cells.push(cell);
        }
        // Vehicles' slots read "used/total" in the Cap column.
        let mut tags = Vec::new();
        if el.name == "cyberware" {
            let side = el.get("location");
            if !side.is_empty() {
                tags.push(Tag::text(side, Tone::Muted));
            }
        }
        let extra = el.get("extra");
        if !extra.is_empty() && !matches!(el.name.as_str(), "lifestyle") {
            tags.push(Tag::text(extra, Tone::Muted));
        }
        if el.name == "drug" {
            tags.clear();
        }
        let n_kids = item_children(kids).len();
        let kids_label = if n_kids == 0 {
            String::new()
        } else if el.name == "gear" {
            lang.tr_fmt("{0} items", &[&n_kids])
        } else {
            lang.tr_fmt("{0} mods", &[&n_kids])
        };
        let selectable = edit::is_item(el) || el.name == "drug";
        let mut actions = Vec::new();
        if selectable && el.name != "drug" {
            actions.push(Action::new("edit", icons::PENCIL_SIMPLE, lang.tr("Edit")));
        }
        if !guid.is_empty() && edit::has_location(ch, &guid) {
            let mut menu = vec![(String::new(), lang.tr("None"))];
            menu.extend(edit::locations(ch, &guid));
            actions.push(Action::new("move", icons::ARROWS_OUT_CARDINAL, lang.tr("Move…")).menu(menu));
        }
        if !included && !guid.is_empty() {
            if self.career && tag != "lifestyle" && el.name != "drug" {
                let menu = [100, 75, 50, 25]
                    .into_iter()
                    .map(|p| (p.to_string(), lang.tr_fmt("Sell at {0} % ({1})", &[&p, &format::nuyen(total * p as f64 / 100.0)])))
                    .collect();
                actions.push(Action::new("sell", icons::COINS, lang.tr("Sell…")).menu(menu));
                actions.push(Action::new("more", icons::DOTS_THREE, lang.tr("More")).menu(vec![("delete".into(), lang.tr("Remove without refund")), ("source".into(), lang.tr("Open the sourcebook at this page"))]));
            } else {
                actions.push(Action::new("remove", icons::TRASH, lang.tr("Remove (also removes its improvements)")));
            }
        }
        let rename = edit::custom_name_field(&tag).filter(|_| !guid.is_empty()).map(|f| {
            let c = el.get(f);
            if c.is_empty() { display_name(self.sec, el, lang) } else { c }
        });
        let icon = if el.name == "cyberware" {
            Some(if chummer_core::items::cyberware::is_bioware(el) { icons::DNA } else { icons::CPU })
        } else {
            Some(kind_icon(&el.name))
        };
        let name = match el.name.as_str() {
            "drug" => el.get("name"),
            _ => display_name(self.sec, el, lang),
        };
        let mut hover = el.get("notes");
        if included {
            hover = [lang.tr("Included with its parent item."), hover].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
        }
        Node::new(
            key,
            RowData {
                kind: Kind::Item,
                name,
                icon,
                tags,
                cells,
                hover,
                issue: self.marks.get(&guid).cloned(),
                selectable,
                actions,
                rename,
                kids_label,
                ..Default::default()
            },
        )
    }

    fn col(&self, key: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.key == key)
    }
}

/// The item rows directly below a node's children (through groups: a
/// vehicle's mod categories and locations).
fn item_children(kids: &[Row]) -> Vec<&Row> {
    let mut out = Vec::new();
    for k in kids {
        match k.value.kind {
            Kind::Item => out.push(k),
            Kind::Group => out.extend(item_children(&k.children)),
            _ => {}
        }
    }
    out
}

/// An item row's own cost plus everything below it.
fn row_total(row: &Row, cx: &Ctx<'_>) -> f64 {
    let Some(i) = cx.col("cost") else { return 0.0 };
    let own = row.value.cells.get(i - 1).and_then(|c| c.num).unwrap_or(0.0);
    own + item_children(&row.children).iter().map(|k| row_total(k, cx)).sum::<f64>()
}

trait WithChildren {
    fn with_children(self, kids: Vec<Row>) -> Row;
}

impl WithChildren for Row {
    fn with_children(mut self, kids: Vec<Row>) -> Row {
        self.children = kids;
        self
    }
}

/// What the page does after the table's events.
#[derive(Default)]
pub struct Outcome {
    pub changed: bool,
    /// Open the catalog for kind `.0`, into item / location `.1`.
    pub buy: Option<(String, Option<String>)>,
}

impl CharacterView {
    /// Run the inventory table's events through the commands the
    /// inspector uses.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ws_inventory_events(&mut self, events: Vec<Event>, page: Page, sec: &Sec, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Outcome {
        let mut out = Outcome::default();
        for e in events {
            match e {
                Event::Select(k) | Event::Open(k) => {
                    if edit::find(&self.doc, &k).is_some() {
                        self.ws_select_item(page, &k);
                    }
                }
                Event::SetRating(k, r) => out.changed |= self.doc.run(Command::SetItemRating { guid: k, rating: r }, status).is_some(),
                Event::SetQty(k, q) => out.changed |= self.doc.set(Command::SetItemQuantity { guid: k, qty: q }),
                Event::Toggle(k, col, on) => {
                    out.changed |= match col {
                        "equipped" => self.doc.set(Command::SetItemEquipped { guid: k, on }),
                        _ => self.doc.set(Command::SetItemWireless { guid: k, on }),
                    }
                }
                Event::Space(k) => {
                    if let Some(el) = edit::find(&self.doc, &k) {
                        if el.child("wirelesson").is_some() {
                            let on = !el.get_bool("wirelesson").unwrap_or(false);
                            out.changed |= self.doc.set(Command::SetItemWireless { guid: k, on });
                        } else if edit::can_equip(el) {
                            let on = !el.get_bool("equipped").unwrap_or(true);
                            out.changed |= self.doc.set(Command::SetItemEquipped { guid: k, on });
                        }
                    }
                }
                Event::Step(k, d) => {
                    if self.doc.created || edit::is_included(&self.doc, &k) {
                        continue;
                    }
                    if let Some((min, max)) = edit::rating_range(&self.doc, &self.store, &k) {
                        let r = edit::find(&self.doc, &k).and_then(|e| e.get_i32("rating")).unwrap_or(min);
                        let n = (r + d).clamp(min, max);
                        if n != r {
                            out.changed |= self.doc.run(Command::SetItemRating { guid: k, rating: n }, status).is_some();
                        }
                    }
                }
                Event::Rename(k, name) => {
                    let Some(el) = edit::find(&self.doc, &k) else { continue };
                    let Some(field) = edit::custom_name_field(edit::tag_of(el)) else { continue };
                    let name = if field != "name" && name == display_name(sec, el, lang) { String::new() } else { name };
                    if !(field == "name" && name.is_empty()) {
                        out.changed |= self.doc.set(Command::SetItemText { guid: k, field: field.to_owned(), value: name });
                    }
                }
                Event::Delete(k) => self.ws_ask_remove(sec, &k, lang),
                Event::Action(k, id, choice) => match (id, choice) {
                    ("edit", _) => self.ws_select_item(page, &k),
                    ("remove", _) => self.ws_ask_remove(sec, &k, lang),
                    ("move", Some(loc)) => out.changed |= self.doc.set(Command::SetItemText { guid: k, field: "location".into(), value: loc }),
                    ("sell", Some(p)) => {
                        let fraction = p.parse::<f64>().unwrap_or(50.0) / 100.0;
                        if let Some(r) = self.doc.run(Command::SellItem { guid: k, fraction }, status) {
                            *status = r.message.map(|m| (m, false));
                            out.changed = true;
                        }
                    }
                    ("more", Some(c)) if c == "delete" => {
                        let name = edit::find(&self.doc, &k).map(|e| display_name(sec, e, lang)).unwrap_or_default();
                        self.confirm_remove = Some((String::new(), k, name));
                    }
                    ("more", Some(c)) if c == "source" => {
                        if let Some(r) = edit::find(&self.doc, &k).and_then(SourceRef::of) {
                            pdf_ui::open(pdfs, &r, status);
                        }
                    }
                    _ => {}
                },
                Event::AddInto(k) => {
                    let kinds = super::ws_items::page_kinds(sec.container);
                    let k = k.strip_suffix("/empty").unwrap_or(&k).to_owned();
                    let tag = if k.ends_with("/Bioware") || k.ends_with("Modular Bioware") { "bioware" } else { kinds.first().copied().unwrap_or("gear") };
                    let loc = k.split_once("/loc/").map(|(_, g)| g.to_owned());
                    out.buy = Some((tag.to_owned(), loc));
                }
                Event::Undo(k) => out.changed |= self.ws_undo_added(sec, &k, status),
            }
        }
        out
    }

    /// Select an owned item: it shows in the inspector; the catalog keeps
    /// its search but drops its selection, and adds into the item when
    /// it can hold the catalog's kind.
    pub(super) fn ws_select_item(&mut self, page: Page, guid: &str) {
        self.item_editor = Some((guid.to_owned(), crate::item_editor::ItemEditor::default()));
        self.ws_catalog_target(page, guid);
    }

    /// Ask before removing (the view's confirmation dialog): top-level
    /// items with `RemoveItem`, nested ones with `DeleteItem`.
    fn ws_ask_remove(&mut self, sec: &Sec, guid: &str, lang: &Language) {
        let Some(el) = edit::find(&self.doc, guid) else { return };
        if edit::is_included(&self.doc, guid) {
            return;
        }
        let top = self.doc.doc.child(sec.container).is_some_and(|c| c.children_named(sec.item).any(|e| e.get("guid") == guid));
        let container = if top { sec.container.to_owned() } else { String::new() };
        self.confirm_remove = Some((container, guid.to_owned(), display_name(sec, el, lang)));
    }

    /// The inline Undo of an item added this visit: the normal undo when
    /// its add is still the latest change, else a Remove of that item.
    pub(super) fn ws_undo_added(&mut self, sec: &Sec, guid: &str, status: &mut Status) -> bool {
        if self.ws_added_is_latest(guid) {
            let steps = self.ws_gear.visit.as_ref().and_then(|v| v.adds.iter().find(|a| a.guid == guid)).map_or(1, |a| a.steps);
            for _ in 0..steps {
                self.doc.undo();
            }
            return true;
        }
        let top = self.doc.doc.child(sec.container).is_some_and(|c| c.children_named(sec.item).any(|e| e.get("guid") == guid));
        let cmd = if top { Command::RemoveItem { container: sec.container.to_owned(), guid: guid.to_owned() } } else { Command::DeleteItem { guid: guid.to_owned() } };
        self.doc.run(cmd, status).is_some()
    }

    /// Whether the add of `guid` is still the character's latest change
    /// (so Undo takes it back exactly).
    pub(super) fn ws_added_is_latest(&self, guid: &str) -> bool {
        let Some(s) = self.doc.session() else { return false };
        s.can_undo() && self.ws_gear.visit.as_ref().is_some_and(|v| v.adds.iter().rev().find(|a| a.guid == guid).is_some_and(|a| a.log_len == s.log().len()))
    }

    /// The item added last this visit, while it is still there.
    pub(super) fn ws_last_added(&self) -> Option<&str> {
        let v = self.ws_gear.visit.as_ref()?;
        v.adds.iter().rev().map(|a| a.guid.as_str()).find(|g| edit::find(&self.doc, g).is_some())
    }

    /// The states the inventory shows; `added` are the items added this
    /// visit.
    pub(super) fn ws_inventory_states<'a>(&'a self, lang: &Language, added: &'a HashSet<String>) -> States<'a> {
        let selected = self.item_editor.as_ref().map(|(g, _)| g.as_str());
        let target = self.ws_gear.catalog.as_ref().and_then(|c| c.target());
        let just = self.ws_last_added().filter(|g| added.contains(*g)).map(|g| {
            let (tip, can) = if self.doc.is_online() {
                (lang.tr(crate::doc::ONLINE_UNDO), false)
            } else if self.ws_added_is_latest(g) {
                (lang.tr("Undo this purchase (Ctrl+Z)"), true)
            } else {
                (lang.tr("Remove it (other changes came after it, so this is not an undo)"), true)
            };
            (g, tip, can)
        });
        States { selected, target, added: Some(added), just_added: just, scroll_to: self.ws_gear.scroll_to.as_deref() }
    }
}

/// "empty", "1 item", "4 items".
pub fn items_note(lang: &Language, n: usize) -> String {
    match n {
        0 => lang.tr("empty"),
        1 => lang.tr("1 item"),
        n => lang.tr_fmt("{0} items", &[&n]),
    }
}

/// The page's item count for the footer: "12 items".
pub fn count_line(lang: &Language, rows: &[Row]) -> String {
    items_note(lang, table::item_count(rows))
}

/// The inventory table's id for a page.
pub fn table_salt(page: Page) -> (&'static str, u8, usize) {
    ("ws_inventory", page.0 as u8, page.1)
}

/// The table id egui memory uses for a page's inventory.
pub fn table_id(page: Page) -> egui::Id {
    table::table_id(table_salt(page))
}
