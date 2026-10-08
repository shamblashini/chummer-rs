//! The Workspace's item table: one widget for every item page (ware,
//! gear, armor, weapons, drugs, lifestyles, vehicles) and the inline
//! catalog's results.
//!
//! A page describes its columns ([`Col`]: key, label or icon, a width in
//! points or the one flexible Name column, alignment, monospace, whether
//! it sorts, a priority for narrow widths, whether group rows and the
//! footer sum it) and its rows as a [`Node`] tree of [`RowData`] (groups,
//! items with their children, empty-group cards, the catalog's details
//! strip). The table draws them in the mockups' style (D4): 28px rows
//! without stripes, group rows with their subtotals, 16px tree steps with
//! guide lines and a chevron slot, a sticky header that sorts on a click
//! (stable, among siblings only, groups stay where they are) and a sticky
//! totals footer. It owns its scrolling (`TableBuilder::vscroll` with a
//! fixed height).
//!
//! Row states come from the page ([`States`]): selected (selection fill
//! and a 3px accent bar; it drives the inspector), just added (a teal bar,
//! an "Added" chip and an inline Undo), added earlier this visit (the bar
//! only), the target container (an accent outline and a "Target" chip),
//! dimmed with a reason, an issue mark before the name. Hover shows the
//! row's actions. Ratings and quantities get steppers on the hovered or
//! selected row; wireless and equipped are toggle buttons. The table only
//! reports what happened ([`Event`]); the page turns that into the same
//! commands as the inspector.
//!
//! Which nodes are folded, the sort and the columns a user turned on or
//! off are kept in egui memory per table, like `tree_table`. Columns that
//! do not fit are hidden lowest priority first ([`layout`]); hidden
//! rating and grade text moves into the name ("Wired Reflexes 1 Alpha").

use std::collections::HashSet;

use chummer_core::lang::Language;
use chummer_core::tree::{flatten, Node};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui};
use egui_extras::{Column, TableBuilder};

use crate::theme::{self, WsPalette};
use crate::workspace::icons;
use crate::workspace::widgets;

/// One indentation step of the tree.
pub const INDENT: f32 = 16.0;
/// Space each side of a cell's content.
pub const PAD: f32 = 5.0;
/// Space left and right of the rows.
pub const EDGE: f32 = 6.0;
/// The Name column never gets narrower than this while other columns
/// are shown.
pub const MIN_NAME: f32 = 200.0;
pub const HEADER_H: f32 = 26.0;
pub const FOOTER_H: f32 = 30.0;
pub const ROW_H: f32 = 28.0;
const EMPTY_H: f32 = 58.0;

/// Where a cell's content sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// A column's width: fixed points, or the rest (the Name column).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Px(f32),
    Flex,
}

/// A column.
#[derive(Debug, Clone)]
pub struct Col {
    pub key: &'static str,
    /// The header text (translated); empty for the actions column.
    pub label: String,
    /// An icon header instead of text; `label` is its tooltip.
    pub icon: Option<&'static str>,
    pub width: Width,
    pub align: Align,
    pub mono: bool,
    pub sortable: bool,
    /// Higher stays longer when the table is narrow.
    pub priority: u8,
    /// Group rows and the footer show its sum.
    pub sum: bool,
    /// When hidden, its text goes into the name ("Wired Reflexes 1 Alpha").
    pub fold: bool,
}

impl Col {
    pub fn new(key: &'static str, label: impl Into<String>) -> Col {
        Col { key, label: label.into(), icon: None, width: Width::Px(56.0), align: Align::Left, mono: false, sortable: true, priority: 50, sum: false, fold: false }
    }

    /// The Name column: flexible, holds the tree, never hidden.
    pub fn name(label: impl Into<String>) -> Col {
        Col { width: Width::Flex, priority: u8::MAX, ..Col::new("name", label) }
    }

    /// The row actions column: never hidden, not sorted.
    pub fn actions(width: f32) -> Col {
        Col { width: Width::Px(width), align: Align::Right, sortable: false, priority: u8::MAX, ..Col::new("actions", "") }
    }

    pub fn px(mut self, w: f32) -> Col {
        self.width = Width::Px(w);
        self
    }
    pub fn center(mut self) -> Col {
        self.align = Align::Center;
        self
    }
    /// Right-aligned monospace numbers.
    pub fn num(mut self) -> Col {
        self.align = Align::Right;
        self.mono = true;
        self
    }
    pub fn mono(mut self) -> Col {
        self.mono = true;
        self
    }
    pub fn prio(mut self, p: u8) -> Col {
        self.priority = p;
        self
    }
    pub fn sum(mut self) -> Col {
        self.sum = true;
        self
    }
    pub fn fold(mut self) -> Col {
        self.fold = true;
        self
    }
    pub fn icon(mut self, glyph: &'static str) -> Col {
        self.icon = Some(glyph);
        self.align = Align::Center;
        self
    }
    pub fn unsorted(mut self) -> Col {
        self.sortable = false;
        self
    }
}

/// Text colour roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Normal,
    Muted,
    Accent,
    Warn,
    Error,
    Teal,
}

impl Tone {
    pub fn color(self, ws: &WsPalette) -> Color32 {
        match self {
            Tone::Normal => ws.text,
            Tone::Muted => ws.muted,
            Tone::Accent => ws.accent,
            Tone::Warn => ws.warning,
            Tone::Error => ws.error,
            Tone::Teal => ws.stun,
        }
    }
}

/// An editable cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// A rating stepper on the hovered or selected row.
    Rating { value: i32, min: i32, max: i32 },
    /// A quantity stepper on the hovered or selected row.
    Qty { value: f64, step: f64 },
    /// An icon toggle (wireless, equipped), always shown.
    Toggle { on: bool, on_icon: &'static str, off_icon: &'static str, tip: String },
    /// A fill bar with "cur/max" and a label (ammunition).
    Meter { cur: i32, max: i32, label: String },
}

/// One cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cell {
    pub text: String,
    /// The value it sorts and sums by.
    pub num: Option<f64>,
    pub tone: Tone,
    pub edit: Option<Edit>,
    /// Hover text.
    pub tip: String,
}

impl Cell {
    pub fn text(t: impl Into<String>) -> Cell {
        Cell { text: t.into(), ..Default::default() }
    }
    pub fn num(t: impl Into<String>, n: Option<f64>) -> Cell {
        Cell { text: t.into(), num: n, ..Default::default() }
    }
    pub fn tone(mut self, t: Tone) -> Cell {
        self.tone = t;
        self
    }
    pub fn edit(mut self, e: Edit) -> Cell {
        self.edit = Some(e);
        self
    }
    pub fn tip(mut self, t: impl Into<String>) -> Cell {
        self.tip = t.into();
        self
    }
    /// Whether it shows nothing worth folding into a name.
    fn blank(&self) -> bool {
        let t = self.text.trim();
        t.is_empty() || t == "—" || t == "-"
    }
}

/// What kind of row a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Item,
    /// A location, type or category: muted fill, name, count, subtotals.
    Group,
    /// A dashed card: an empty group or list, with a Buy… button.
    Empty,
    /// The catalog's details strip under its selected row (narrow).
    Detail,
}

/// Something after the name: plain small text, or an outlined chip.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub text: String,
    pub tone: Tone,
    pub chip: bool,
    pub icon: Option<&'static str>,
}

impl Tag {
    pub fn text(t: impl Into<String>, tone: Tone) -> Tag {
        Tag { text: t.into(), tone, chip: false, icon: None }
    }
    pub fn chip(t: impl Into<String>, tone: Tone, icon: Option<&'static str>) -> Tag {
        Tag { text: t.into(), tone, chip: true, icon }
    }
}

/// A row button shown on hover (Edit, Move…, Sell, Remove, More, the
/// catalog's Add). With `menu`, a click opens those entries instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: &'static str,
    pub icon: &'static str,
    pub tip: String,
    /// (entry id, label).
    pub menu: Vec<(String, String)>,
    /// Filled with the primary colour on the selected row (the catalog's Add).
    pub primary: bool,
}

impl Action {
    pub fn new(id: &'static str, icon: &'static str, tip: impl Into<String>) -> Action {
        Action { id, icon, tip: tip.into(), menu: Vec::new(), primary: false }
    }
    pub fn menu(mut self, entries: Vec<(String, String)>) -> Action {
        self.menu = entries;
        self
    }
    pub fn primary(mut self) -> Action {
        self.primary = true;
        self
    }
}

/// One row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowData {
    pub kind: Kind,
    pub name: String,
    pub icon: Option<&'static str>,
    pub tags: Vec<Tag>,
    /// A second line under the name (catalog rows).
    pub sub: Option<Tag>,
    /// Group: "4 items"; empty card: its second line.
    pub note: String,
    /// One per column after Name (in the page's column order).
    pub cells: Vec<Cell>,
    /// Name hover text (notes).
    pub hover: String,
    /// A creation problem: (message, is an error).
    pub issue: Option<(String, bool)>,
    /// Muted, with this reason (amber) under or after the name.
    pub dim: Option<String>,
    /// A click selects it (items the inspector handles).
    pub selectable: bool,
    pub actions: Vec<Action>,
    /// Group rows: a "+" that adds into it; empty cards: the Buy… label.
    pub add_into: Option<String>,
    /// F2 renames it; the current custom name.
    pub rename: Option<String>,
    /// "N mods" when folded.
    pub kids_label: String,
}

pub type Row = Node<RowData>;

/// The states the page knows about.
#[derive(Default)]
pub struct States<'a> {
    pub selected: Option<&'a str>,
    /// The container the catalog adds into.
    pub target: Option<&'a str>,
    /// Items added since the page was opened (a teal bar).
    pub added: Option<&'a HashSet<String>>,
    /// The item added last: (key, Undo's tooltip, whether Undo works).
    pub just_added: Option<(&'a str, String, bool)>,
    /// Scroll this row into view.
    pub scroll_to: Option<&'a str>,
}

/// What happened this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A click (or the arrow keys) selected an item row.
    Select(String),
    /// Double click or Enter.
    Open(String),
    /// A row action, with the menu entry picked.
    Action(String, &'static str, Option<String>),
    SetRating(String, i32),
    SetQty(String, f64),
    /// A toggle cell: (row, column key, new state).
    Toggle(String, &'static str, bool),
    /// A group's "+", or an empty card's button ("" for the whole table).
    AddInto(String),
    /// The just-added row's inline Undo.
    Undo(String),
    /// Del on the selected row.
    Delete(String),
    /// + / − on the selected row.
    Step(String, i32),
    /// Space on the selected row.
    Space(String),
    /// F2 renaming finished: (row, new name).
    Rename(String, String),
}

/// The footer: the left text (count, "Essence left 2.35"), sums of the
/// summed columns, and a note in the actions column ("purchase value").
#[derive(Default, Clone)]
pub struct Footer {
    pub left: String,
    /// A highlighted value after `left`.
    pub accent: String,
    /// Cell text by column key, instead of the computed sum.
    pub cells: Vec<(&'static str, String)>,
    pub tail: String,
}

/// The empty-table card.
pub struct EmptyCard {
    pub title: String,
    pub sub: String,
    pub button: String,
}

// ----- pure parts: column layout, sorting, sums -----

/// Which columns show at a width, and how wide each one is.
#[derive(Debug, Clone, PartialEq)]
pub struct ColLayout {
    /// Column indices shown, in order.
    pub shown: Vec<usize>,
    /// Width of each shown column (cell padding included).
    pub widths: Vec<f32>,
    /// Hidden columns whose text goes into the name.
    pub folded: Vec<usize>,
}

/// Lay out `cols` in `width`: drop the lowest-priority columns until the
/// fixed ones leave the Name column [`MIN_NAME`]. `overrides` are the
/// user's choices from the Columns menu (key, shown): a column turned off
/// is never shown, one turned on is dropped only after every other.
pub fn layout(cols: &[Col], width: f32, overrides: &[(String, bool)]) -> ColLayout {
    let choice = |c: &Col| overrides.iter().find(|(k, _)| k == c.key).map(|(_, v)| *v);
    let px = |c: &Col| match c.width {
        Width::Px(w) => w + 2.0 * PAD,
        Width::Flex => 0.0,
    };
    let mut on: Vec<bool> = cols.iter().map(|c| choice(c) != Some(false) || c.priority == u8::MAX).collect();
    let room = width - 2.0 * EDGE;
    loop {
        let fixed: f32 = cols.iter().zip(&on).filter(|(_, o)| **o).map(|(c, _)| px(c)).sum();
        if room - fixed >= MIN_NAME {
            break;
        }
        // The lowest priority still shown (chosen ones last; later
        // columns first among equals).
        let victim = cols
            .iter()
            .enumerate()
            .filter(|(i, c)| on[*i] && c.priority < u8::MAX)
            .min_by_key(|(i, c)| (choice(c) == Some(true), c.priority, std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        match victim {
            Some(i) => on[i] = false,
            None => break,
        }
    }
    let shown: Vec<usize> = (0..cols.len()).filter(|i| on[*i]).collect();
    let fixed: f32 = shown.iter().map(|i| px(&cols[*i])).sum();
    let flex = (room - fixed).max(60.0);
    let mut widths: Vec<f32> = shown.iter().map(|i| if cols[*i].width == Width::Flex { flex } else { px(&cols[*i]) }).collect();
    // The edges go to the first and last columns.
    if let Some(w) = widths.first_mut() {
        *w += EDGE;
    }
    if let Some(w) = widths.last_mut() {
        *w += EDGE;
    }
    let folded = (0..cols.len()).filter(|i| !on[*i] && cols[*i].fold && choice(&cols[*i]) != Some(false)).collect();
    ColLayout { shown, widths, folded }
}

/// Sort order of two cells: numbers by value, text without case; empty
/// cells ("", "—") last either way.
fn cmp_cells(a: Option<&Cell>, b: Option<&Cell>, asc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let blank = |c: Option<&Cell>| c.is_none_or(|c| c.num.is_none() && c.blank());
    match (blank(a), blank(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let (a, b) = (a.expect("not blank"), b.expect("not blank"));
    let o = match (a.num, b.num) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.text.to_lowercase().cmp(&b.text.to_lowercase()),
    };
    if asc { o } else { o.reverse() }
}

/// The cell of `row` for column `col` (0 = Name).
fn cell_of(row: &RowData, col: usize) -> Option<&Cell> {
    if col == 0 { None } else { row.cells.get(col - 1) }
}

/// Sort `nodes` by column `col` (0 = Name), stably and among siblings
/// only: items move among the places items had; groups, empty cards and
/// details strips stay where they are. Children are sorted the same way.
pub fn sort_tree(nodes: &mut [Row], col: usize, asc: bool) {
    let slots: Vec<usize> = (0..nodes.len()).filter(|i| nodes[*i].value.kind == Kind::Item).collect();
    let mut items: Vec<Row> = slots.iter().map(|i| std::mem::replace(&mut nodes[*i], Node::new("", RowData::default()))).collect();
    if col == 0 {
        items.sort_by(|a, b| {
            let o = a.value.name.to_lowercase().cmp(&b.value.name.to_lowercase());
            if asc { o } else { o.reverse() }
        });
    } else {
        items.sort_by(|a, b| cmp_cells(cell_of(&a.value, col), cell_of(&b.value, col), asc));
    }
    for (slot, item) in slots.into_iter().zip(items) {
        nodes[slot] = item;
    }
    for n in nodes.iter_mut() {
        sort_tree(&mut n.children, col, asc);
    }
}

/// The sum of column `col` over the items below `node` (all depths).
pub fn subtotal(node: &Row, col: usize) -> Option<f64> {
    let mut sum = None;
    for c in &node.children {
        if c.value.kind == Kind::Item {
            if let Some(v) = cell_of(&c.value, col).and_then(|c| c.num) {
                *sum.get_or_insert(0.0) += v;
            }
        }
        if let Some(v) = subtotal(c, col) {
            *sum.get_or_insert(0.0) += v;
        }
    }
    sum
}

/// The sum of column `col` over every item of the table.
pub fn total(nodes: &[Row], col: usize) -> Option<f64> {
    let mut sum = None;
    for n in nodes {
        if n.value.kind == Kind::Item {
            if let Some(v) = cell_of(&n.value, col).and_then(|c| c.num) {
                *sum.get_or_insert(0.0) += v;
            }
        }
        if let Some(v) = subtotal(n, col) {
            *sum.get_or_insert(0.0) += v;
        }
    }
    sum
}

/// Items in the table (all depths).
pub fn item_count(nodes: &[Row]) -> usize {
    nodes.iter().map(|n| usize::from(n.value.kind == Kind::Item) + item_count(&n.children)).sum()
}

/// The name a row shows with folded columns: "Wired Reflexes 1 Alpha".
pub fn folded_name(row: &RowData, folded: &[usize]) -> String {
    let mut s = row.name.clone();
    for i in folded {
        if let Some(c) = cell_of(row, *i).filter(|c| !c.blank() && c.tone != Tone::Muted && c.edit.as_ref().is_none_or(|e| matches!(e, Edit::Rating { .. } | Edit::Qty { .. }))) {
            s.push(' ');
            s.push_str(c.text.trim());
        }
    }
    s
}

// ----- state in egui memory -----

fn closed_id(id: egui::Id) -> egui::Id {
    id.with("closed")
}
fn sort_id(id: egui::Id) -> egui::Id {
    id.with("sort")
}
fn cols_id(id: egui::Id) -> egui::Id {
    id.with("cols")
}

/// The table id for a salt (what [`Table::new`] uses).
pub fn table_id(salt: impl std::hash::Hash) -> egui::Id {
    egui::Id::new(("ws_table", salt))
}

/// The sort a table shows: (column key, ascending).
pub fn sort_of(ctx: &egui::Context, id: egui::Id) -> Option<(String, bool)> {
    ctx.data_mut(|d| d.get_persisted::<Option<(String, bool)>>(sort_id(id))).flatten()
}

/// Set (or clear) a table's sort.
pub fn set_sort(ctx: &egui::Context, id: egui::Id, sort: Option<(String, bool)>) {
    ctx.data_mut(|d| d.insert_persisted(sort_id(id), sort));
}

/// The folded node keys of a table.
pub fn closed_of(ctx: &egui::Context, id: egui::Id) -> HashSet<String> {
    ctx.data_mut(|d| d.get_persisted::<HashSet<String>>(closed_id(id))).unwrap_or_default()
}

pub fn set_closed(ctx: &egui::Context, id: egui::Id, closed: HashSet<String>) {
    ctx.data_mut(|d| d.insert_persisted(closed_id(id), closed));
}

/// Keys of every node with children (for Collapse all).
pub fn parent_keys(nodes: &[Row]) -> HashSet<String> {
    let mut out = HashSet::new();
    fn walk(nodes: &[Row], out: &mut HashSet<String>) {
        for n in nodes {
            if !n.children.is_empty() {
                out.insert(n.key.clone());
                walk(&n.children, out);
            }
        }
    }
    walk(nodes, &mut out);
    out
}

/// The user's column choices.
pub fn column_choices(ctx: &egui::Context, id: egui::Id) -> Vec<(String, bool)> {
    ctx.data_mut(|d| d.get_persisted::<Vec<(String, bool)>>(cols_id(id))).unwrap_or_default()
}

/// The Columns button: a menu with a check box per column (hidden ones
/// turn on, shown ones off).
pub fn columns_button(ui: &mut Ui, id: egui::Id, cols: &[Col], lang: &Language) {
    let r = widgets::icon_button(ui, icons::COLUMNS, 26.0).on_hover_text(lang.tr("Choose columns"));
    egui::Popup::menu(&r).show(|ui| {
        let mut choices = column_choices(ui.ctx(), id);
        let width = ui.ctx().data(|d| d.get_temp::<f32>(id.with("width"))).unwrap_or(f32::INFINITY);
        let lay = layout(cols, width, &choices);
        let mut changed = false;
        for (i, c) in cols.iter().enumerate() {
            if c.priority == u8::MAX {
                continue;
            }
            let mut on = lay.shown.contains(&i);
            let label = if c.label.is_empty() { c.key.to_owned() } else { c.label.clone() };
            if widgets::check(ui, &mut on, &label).changed() {
                choices.retain(|(k, _)| k != c.key);
                choices.push((c.key.to_owned(), on));
                changed = true;
            }
        }
        ui.add_space(4.0);
        if widgets::button(ui, None, &lang.tr("Reset"), widgets::Look::Ghost, 22.0).clicked() {
            choices.clear();
            changed = true;
        }
        if changed {
            ui.ctx().data_mut(|d| d.insert_persisted(cols_id(id), choices));
        }
    });
}

// ----- the widget -----

pub struct Table<'a> {
    id: egui::Id,
    cols: &'a [Col],
    height: f32,
    row_h: f32,
    footer: Option<Footer>,
    empty: Option<EmptyCard>,
    focused: bool,
    tree: bool,
    detail_h: f32,
}

/// A row of the flattened tree with what drawing needs.
struct Line<'n> {
    node: &'n Row,
    depth: usize,
    open: bool,
    last: bool,
    guides: Vec<bool>,
}

impl<'a> Table<'a> {
    pub fn new(salt: impl std::hash::Hash, cols: &'a [Col]) -> Table<'a> {
        Table { id: table_id(salt), cols, height: 300.0, row_h: ROW_H, footer: None, empty: None, focused: false, tree: true, detail_h: 52.0 }
    }

    /// The whole table's height (header, rows and footer).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    /// Item row height (28; the catalog's two-line rows 36).
    pub fn row_height(mut self, h: f32) -> Self {
        self.row_h = h;
        self
    }

    pub fn footer(mut self, f: Footer) -> Self {
        self.footer = Some(f);
        self
    }

    /// The card shown when there are no rows.
    pub fn empty(mut self, e: EmptyCard) -> Self {
        self.empty = Some(e);
        self
    }

    /// Keys go to this table (↑↓ ←→ Enter Del Space + − F2).
    pub fn focused(mut self, f: bool) -> Self {
        self.focused = f;
        self
    }

    /// Without tree lines and chevrons (the catalog's flat list).
    pub fn flat(mut self) -> Self {
        self.tree = false;
        self
    }

    /// Height of the details strip rows.
    pub fn detail_height(mut self, h: f32) -> Self {
        self.detail_h = h;
        self
    }

    /// Draw the table. `detail` fills [`Kind::Detail`] rows.
    pub fn show(self, ui: &mut Ui, roots: &[Row], states: &States<'_>, lang: &Language, mut detail: impl FnMut(&mut Ui, &str)) -> Vec<Event> {
        let ws = theme::ws(ui);
        let id = self.id;
        let ctx = ui.ctx().clone();
        let mut events = Vec::new();
        let width = ui.available_width();
        ctx.data_mut(|d| d.insert_temp(id.with("width"), width));
        let choices = column_choices(&ctx, id);
        let lay = layout(self.cols, width, &choices);
        let mut closed = closed_of(&ctx, id);
        let sort = sort_of(&ctx, id);
        let mut toggled: Option<String> = None;
        let mut new_sort: Option<Option<(String, bool)>> = None;

        let lines: Vec<Line<'_>> = flatten(roots, &|n: &Row| !closed.contains(&n.key)).into_iter().map(|r| Line { node: r.node, depth: r.depth, open: r.open, last: r.last, guides: r.guides }).collect();

        // Keys.
        let mut scroll_to: Option<usize> = states.scroll_to.and_then(|k| lines.iter().position(|l| l.node.key == k));
        let rename_id = id.with("rename");
        let mut renaming: Option<(String, String)> = ctx.data(|d| d.get_temp(rename_id));
        if self.focused && !ctx.wants_keyboard_input() && renaming.is_none() && !ctx.is_popup_open() {
            use egui::{Key, Modifiers};
            let sel = states.selected.and_then(|s| lines.iter().position(|l| l.node.key == s));
            let (down, up, left, right, enter, del, space, plus, minus, f2) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Delete),
                    i.consume_key(Modifiers::NONE, Key::Space),
                    i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals) || i.consume_key(Modifiers::SHIFT, Key::Equals),
                    i.consume_key(Modifiers::NONE, Key::Minus),
                    i.consume_key(Modifiers::NONE, Key::F2),
                )
            });
            if down || up {
                let selectable: Vec<usize> = (0..lines.len()).filter(|i| lines[*i].node.value.selectable).collect();
                let next = match sel.and_then(|s| selectable.iter().position(|i| *i == s)) {
                    Some(p) if down => selectable.get(p + 1).or(selectable.last()),
                    Some(p) => selectable.get(p.saturating_sub(1)),
                    None => selectable.first(),
                };
                if let Some(&n) = next {
                    events.push(Event::Select(lines[n].node.key.clone()));
                    scroll_to = Some(n);
                }
            }
            if let Some(s) = sel {
                let l = &lines[s];
                let key = l.node.key.clone();
                if !l.node.children.is_empty() && ((left && l.open) || (right && !l.open)) {
                    toggled = Some(key.clone());
                }
                if enter {
                    events.push(Event::Open(key.clone()));
                }
                if del {
                    events.push(Event::Delete(key.clone()));
                }
                if space {
                    events.push(Event::Space(key.clone()));
                }
                if plus || minus {
                    events.push(Event::Step(key.clone(), if plus { 1 } else { -1 }));
                }
                if f2 {
                    if let Some(n) = &l.node.value.rename {
                        renaming = Some((key, n.clone()));
                    }
                }
            }
        }

        let header_shadow_id = id.with("scrolled");
        let scrolled: bool = ctx.data(|d| d.get_temp(header_shadow_id)).unwrap_or(false);
        let footer_h = if self.footer.is_some() { FOOTER_H } else { 0.0 };
        let body_h = (self.height - HEADER_H - footer_h).max(self.row_h);
        let shown_key: Vec<&str> = lay.shown.iter().map(|i| self.cols[*i].key).collect();

        let start = ui.cursor().min;
        ui.push_id(id, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            // Rows paint their own hover.
            ui.visuals_mut().widgets.hovered.bg_fill = Color32::TRANSPARENT;
            let mut tb = TableBuilder::new(ui)
                .id_salt(("t", &shown_key))
                .striped(false)
                .resizable(false)
                .sense(Sense::click())
                .vscroll(true)
                .auto_shrink([false, false])
                .min_scrolled_height(body_h)
                .max_scroll_height(body_h)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
            if let Some(i) = scroll_to {
                tb = tb.scroll_to_row(i, None);
            }
            for w in &lay.widths {
                tb = tb.column(Column::exact(*w).clip(false));
            }
            let cols = self.cols;
            let first_w = lay.widths.first().copied().unwrap_or(0.0);
            let last_w = lay.widths.last().copied().unwrap_or(0.0);
            let total_w: f32 = lay.widths.iter().sum();
            let n_shown = lay.shown.len();
            let out = tb
                .header(HEADER_H, |mut h| {
                    for (k, &ci) in lay.shown.iter().enumerate() {
                        let c = &cols[ci];
                        let active = sort.as_ref().filter(|(key, _)| key == c.key).map(|(_, asc)| *asc);
                        let color = if active.is_some() { ws.accent } else { ws.muted };
                        let (_, resp) = h.col(|ui| {
                            let r = ui.max_rect();
                            if k == 0 {
                                let full = Rect::from_min_size(r.min, egui::vec2(total_w, HEADER_H));
                                ui.painter().rect_filled(full, CornerRadius::ZERO, ws.raised);
                                ui.painter().rect_filled(Rect::from_min_size(egui::pos2(full.left(), full.bottom() - 1.0), egui::vec2(total_w, 1.0)), CornerRadius::ZERO, ws.divider);
                            }
                            let inner = cell_inner(r, k == 0, k + 1 == n_shown, first_w, last_w);
                            let hovered = c.sortable && ui.rect_contains_pointer(r);
                            let p = ui.painter();
                            if let Some(g) = c.icon {
                                icons::paint(p, inner, g, 12.0, color);
                            } else if !c.label.is_empty() {
                                let galley = p.layout_no_wrap(c.label.to_uppercase(), widgets::bold(10.5), color);
                                let caret = match active {
                                    Some(true) => Some(icons::CARET_UP),
                                    Some(false) => Some(icons::CARET_DOWN),
                                    None if hovered => Some(icons::CARET_UP_DOWN),
                                    None => None,
                                };
                                let cw = if caret.is_some() { 12.0 } else { 0.0 };
                                let gw = galley.size().x;
                                let (gx, cx) = match c.align {
                                    Align::Right => (inner.right() - gw, inner.right() - gw - cw),
                                    Align::Center => (inner.center().x - (gw + cw) / 2.0, inner.center().x + gw / 2.0 - cw / 2.0 + 2.0),
                                    Align::Left => (inner.left(), inner.left() + gw + 2.0),
                                };
                                p.with_clip_rect(inner.expand2(egui::vec2(PAD, 0.0))).galley(egui::pos2(gx, inner.center().y - galley.size().y / 2.0), galley, color);
                                if let Some(g) = caret {
                                    icons::paint(p, Rect::from_min_size(egui::pos2(cx, inner.center().y - 6.0), egui::vec2(10.0, 12.0)), g, 10.0, color);
                                }
                            }
                        });
                        let resp = if !c.label.is_empty() && c.icon.is_some() { resp.on_hover_text(&c.label) } else { resp };
                        if c.sortable && resp.clicked() {
                            // Ascending, descending, off.
                            new_sort = Some(match active {
                                None => Some((c.key.to_owned(), c.key == "name" || !c.mono)),
                                Some(asc) if asc == (c.key == "name" || !c.mono) => Some((c.key.to_owned(), !asc)),
                                Some(_) => None,
                            });
                        }
                        if c.sortable {
                            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        }
                    }
                })
                .body(|body| {
                    let heights = lines.iter().map(|l| match l.node.value.kind {
                        Kind::Item => self.row_h,
                        Kind::Group => ROW_H,
                        Kind::Empty => EMPTY_H,
                        Kind::Detail => self.detail_h,
                    });
                    body.heterogeneous_rows(heights, |mut row| {
                        let index = row.index();
                        let l = &lines[index];
                        let v = &l.node.value;
                        let key = l.node.key.as_str();
                        let selected = states.selected == Some(key);
                        let is_target = states.target == Some(key);
                        let just = states.just_added.as_ref().filter(|(k, _, _)| *k == key);
                        let added = just.is_some() || states.added.is_some_and(|a| a.contains(key));
                        let mut hovered = false;
                        let mut toggle_hit = false;
                        let h = row_height_of(v.kind, self.row_h, self.detail_h);
                        for (k, &ci) in lay.shown.iter().enumerate() {
                            let c = &cols[ci];
                            row.col(|ui| {
                                let r = ui.max_rect();
                                let full = Rect::from_min_size(r.min, egui::vec2(total_w, h));
                                if k == 0 {
                                    hovered = ui.rect_contains_pointer(full);
                                    paint_row_back(ui, &ws, full, v.kind, selected, hovered, added, is_target);
                                }
                                let inner = cell_inner(r, k == 0, k + 1 == n_shown, first_w, last_w);
                                match v.kind {
                                    Kind::Detail => {
                                        if k == 0 {
                                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full.shrink2(egui::vec2(EDGE + 4.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                                            child.set_clip_rect(full.intersect(ui.clip_rect()));
                                            detail(&mut child, key);
                                        }
                                    }
                                    Kind::Empty => {
                                        if k == 0 {
                                            let indent = if self.tree { (l.depth as f32 + 1.0) * INDENT } else { 0.0 };
                                            let card = Rect::from_min_max(egui::pos2(full.left() + EDGE + indent, full.top() + 6.0), egui::pos2(full.right() - EDGE - 4.0, full.bottom() - 6.0));
                                            if empty_card(ui, &ws, card, &v.name, &v.note, v.add_into.as_deref()) {
                                                events.push(Event::AddInto(key.to_owned()));
                                            }
                                        }
                                    }
                                    _ if k == 0 => {
                                        toggle_hit |= self.name_cell(ui, &ws, inner, l, v, selected, is_target, just.is_some(), &lay, lang, &mut renaming, &mut events);
                                    }
                                    _ => {
                                        let cell = v.cells.get(ci - 1);
                                        if c.key == "actions" {
                                            self.actions_cell(ui, &ws, inner, l, v, hovered || selected, selected, just, lang, &mut events);
                                        } else if v.kind == Kind::Group {
                                            if c.sum {
                                                if let Some(s) = subtotal(l.node, ci) {
                                                    let text = cell.filter(|c| !c.text.is_empty()).map_or_else(|| fmt_sum(c.key, s), |c| c.text.clone());
                                                    paint_text(ui, inner, c.align, &text, c.mono, ws.muted, false);
                                                }
                                            }
                                        } else if let Some(cell) = cell {
                                            if let Some(e) = edit_cell(ui, &ws, inner, c, cell, hovered || selected, v.dim.is_some(), lang) {
                                                events.push(match e {
                                                    CellOut::Rating(r) => Event::SetRating(key.to_owned(), r),
                                                    CellOut::Qty(q) => Event::SetQty(key.to_owned(), q),
                                                    CellOut::Toggle(on) => Event::Toggle(key.to_owned(), c.key, on),
                                                });
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        let resp = row.response();
                        if v.kind == Kind::Group || v.kind == Kind::Item {
                            if toggle_hit || (!l.node.children.is_empty() && v.kind == Kind::Group && resp.clicked()) {
                                toggled = Some(key.to_owned());
                            } else if resp.clicked() && v.selectable {
                                events.push(Event::Select(key.to_owned()));
                            }
                            if resp.double_clicked() && v.selectable {
                                events.push(Event::Open(key.to_owned()));
                            }
                            if v.selectable {
                                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                            }
                        }
                    });
                });
            ctx.data_mut(|d| d.insert_temp(header_shadow_id, out.state.offset.y > 0.5));

            // The whole-table empty card.
            if lines.is_empty() {
                if let Some(e) = &self.empty {
                    let top = start.y + HEADER_H + 8.0;
                    let card = Rect::from_min_max(egui::pos2(start.x + EDGE + 4.0, top), egui::pos2(start.x + width - EDGE - 4.0, top + EMPTY_H - 12.0));
                    if empty_card(ui, &ws, card, &e.title, &e.sub, Some(&e.button)) {
                        events.push(Event::AddInto(String::new()));
                    }
                }
            }
            // The header's shadow while scrolled.
            if scrolled {
                let y = start.y + HEADER_H;
                for (i, a) in [0.22_f32, 0.12, 0.05].into_iter().enumerate() {
                    let r = Rect::from_min_size(egui::pos2(start.x, y + i as f32 * 2.0), egui::vec2(total_w, 2.0));
                    ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha((a * 255.0) as u8));
                }
            }
            // Footer.
            if let Some(f) = &self.footer {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(total_w.max(1.0), FOOTER_H), Sense::hover());
                let top = rect.top();
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::ZERO, ws.chrome);
                p.rect_filled(Rect::from_min_size(rect.min, egui::vec2(rect.width(), 1.0)), CornerRadius::ZERO, ws.divider);
                let mut x = start.x;
                for (k, (&ci, w)) in lay.shown.iter().zip(&lay.widths).enumerate() {
                    let c = &cols[ci];
                    let r = Rect::from_min_size(egui::pos2(x, top), egui::vec2(*w, FOOTER_H));
                    let inner = cell_inner(r, k == 0, k + 1 == n_shown, first_w, last_w);
                    x += w;
                    if k == 0 {
                        let g = p.layout_no_wrap(f.left.clone(), FontId::proportional(11.5), ws.muted);
                        let gw = g.size().x;
                        let pc = p.with_clip_rect(inner);
                        pc.galley(egui::pos2(inner.left(), inner.center().y - g.size().y / 2.0), g, ws.muted);
                        if !f.accent.is_empty() {
                            pc.text(egui::pos2(inner.left() + gw + 5.0, inner.center().y), Align2::LEFT_CENTER, &f.accent, FontId::monospace(12.0), ws.accent);
                        }
                        continue;
                    }
                    let text = match f.cells.iter().find(|(key, _)| *key == c.key) {
                        Some((_, t)) => Some(t.clone()),
                        None if c.key == "actions" => (!f.tail.is_empty()).then(|| f.tail.clone()),
                        None if c.sum => total(roots, ci).map(|s| fmt_sum(c.key, s)),
                        None => None,
                    };
                    match text {
                        // The note may reach into the columns before it.
                        Some(t) if c.key == "actions" => {
                            p.with_clip_rect(rect).text(egui::pos2(inner.right(), inner.center().y), Align2::RIGHT_CENTER, t, FontId::proportional(11.0), ws.muted);
                        }
                        Some(t) => paint_text(ui, inner, c.align, &t, true, ws.text, false),
                        None => {}
                    }
                }
            }
        });

        if renaming.is_some() {
            ctx.data_mut(|d| d.insert_temp(rename_id, renaming));
        } else {
            ctx.data_mut(|d| d.remove::<Option<(String, String)>>(rename_id));
        }
        if let Some(k) = toggled {
            if !closed.remove(&k) {
                closed.insert(k);
            }
            set_closed(&ctx, id, closed);
            ctx.request_repaint();
        }
        if let Some(s) = new_sort {
            set_sort(&ctx, id, s);
            ctx.request_repaint();
        }
        events
    }

    /// The name cell: tree guides, chevron, icon, issue mark, the name
    /// (or the rename field), tags and chips. Returns whether the chevron
    /// was clicked.
    #[allow(clippy::too_many_arguments)]
    fn name_cell(&self, ui: &mut Ui, ws: &WsPalette, inner: Rect, l: &Line<'_>, v: &RowData, selected: bool, target: bool, just: bool, lay: &ColLayout, lang: &Language, renaming: &mut Option<(String, String)>, events: &mut Vec<Event>) -> bool {
        let mut toggle_hit = false;
        let mut x = inner.left();
        let mid = inner.center().y;
        let has_kids = !l.node.children.is_empty();
        if self.tree {
            let (top, bottom) = (inner.top(), inner.bottom());
            let guide = Stroke::new(1.0_f32, ws.control);
            let p = ui.painter();
            for level in 0..l.depth {
                let gx = x + level as f32 * INDENT + 7.5;
                if level + 1 < l.depth {
                    if l.guides.get(level + 1).copied().unwrap_or(false) {
                        p.line_segment([egui::pos2(gx, top), egui::pos2(gx, bottom)], guide);
                    }
                } else {
                    // ├ / └ joint at this row's own level.
                    p.line_segment([egui::pos2(gx, top), egui::pos2(gx, if l.last { mid } else { bottom })], guide);
                    p.line_segment([egui::pos2(gx, mid), egui::pos2(gx + 8.0, mid)], guide);
                }
            }
            x += l.depth as f32 * INDENT;
            let chevron = Rect::from_min_size(egui::pos2(x, mid - 8.0), egui::vec2(16.0, 16.0));
            if has_kids {
                let resp = ui.interact(chevron, ui.id().with(("chev", &l.node.key)), Sense::click());
                let hot = resp.hovered();
                if hot {
                    ui.painter().rect_filled(chevron, CornerRadius::same(4), ws.hover);
                }
                let g = if l.open { icons::CARET_DOWN } else { icons::CARET_RIGHT };
                icons::paint(ui.painter(), chevron, g, 11.0, if hot { ws.accent } else { ws.muted });
                let label = if l.open { lang.tr("Collapse") } else { lang.tr("Expand") };
                toggle_hit = resp.on_hover_text(label).clicked();
            }
            x += 16.0 + 4.0;
        }
        let p = ui.painter().with_clip_rect(inner.intersect(ui.clip_rect()));
        let group = v.kind == Kind::Group;
        if let Some(g) = v.icon {
            let c = if selected { ws.accent } else { ws.muted };
            icons::paint(&p, Rect::from_min_size(egui::pos2(x, mid - 7.0), egui::vec2(14.0, 14.0)), g, 14.0, c);
            x += 14.0 + 6.0;
        }
        if let Some((msg, error)) = &v.issue {
            let r = Rect::from_min_size(egui::pos2(x, mid - 7.0), egui::vec2(14.0, 14.0));
            icons::paint(&p, r, icons::WARNING, 13.0, if *error { ws.error } else { ws.warning });
            ui.interact(r, ui.id().with(("issue", &l.node.key)), Sense::hover()).on_hover_text(msg);
            x += 14.0 + 5.0;
        }
        // Rename field.
        if let Some((k, text)) = renaming.as_mut().filter(|(k, _)| *k == l.node.key) {
            let k = k.clone();
            let r = Rect::from_min_max(egui::pos2(x, mid - 10.0), egui::pos2(inner.right(), mid + 10.0));
            let te = ui.put(r, egui::TextEdit::singleline(text).font(FontId::proportional(12.5)).margin(egui::vec2(4.0, 1.0)));
            te.request_focus();
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if enter || te.lost_focus() && !esc {
                events.push(Event::Rename(k, text.trim().to_owned()));
                *renaming = None;
            } else if esc {
                *renaming = None;
            }
            return toggle_hit;
        }
        let dim = v.dim.is_some();
        let ink = if dim { ws.muted } else { ws.text };
        let name = if lay.folded.is_empty() || group { v.name.clone() } else { folded_name(v, &lay.folded) };
        let font = if group || selected { widgets::bold(12.5) } else { FontId::proportional(12.5) };
        let two_line = v.sub.is_some();
        let name_y = if two_line { inner.top() + inner.height() / 2.0 - 8.0 } else { mid };
        let mut job = egui::text::LayoutJob::single_section(name, egui::TextFormat::simple(font, ink));
        job.wrap = egui::text::TextWrapping { max_width: (inner.right() - x).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let g = p.layout_job(job);
        let gw = g.size().x;
        let name_rect = Rect::from_min_size(egui::pos2(x, name_y - g.size().y / 2.0), g.size());
        p.galley(name_rect.min, g, ink);
        if !v.hover.trim().is_empty() {
            ui.interact(name_rect.intersect(inner), ui.id().with(("name", &l.node.key)), Sense::hover()).on_hover_text(&v.hover);
        }
        let mut tx = x + gw + 6.0;
        let mut tags: Vec<Tag> = v.tags.clone();
        if group && !v.note.is_empty() {
            tags.insert(0, Tag::text(&v.note, Tone::Muted));
        }
        if has_kids && !l.open && !v.kids_label.is_empty() {
            tags.push(Tag::text(&v.kids_label, Tone::Muted));
        }
        if target {
            tags.push(Tag::chip(lang.tr("Target"), Tone::Accent, Some(icons::ARROW_BEND_DOWN_RIGHT)));
        }
        if just {
            tags.push(Tag::chip(lang.tr("Added"), Tone::Teal, None));
        }
        if let (Some(why), false) = (&v.dim, two_line) {
            tags.push(Tag::text(format!("{} {why}", icons::WARNING), Tone::Warn));
        }
        for t in &tags {
            tx += paint_tag(&p, ws, egui::pos2(tx, name_y), t) + 6.0;
        }
        if let Some(sub) = &v.sub {
            let sub = match &v.dim {
                Some(why) => Tag::text(format!("{} {why}", icons::WARNING), Tone::Warn),
                None => sub.clone(),
            };
            let text = match sub.icon {
                Some(g) => format!("{g} {}", sub.text),
                None => sub.text.clone(),
            };
            p.text(egui::pos2(x, inner.top() + inner.height() / 2.0 + 8.0), Align2::LEFT_CENTER, text, FontId::proportional(11.0), sub.tone.color(ws));
        }
        toggle_hit
    }

    /// The actions cell: the just-added row's Undo and Remove, or the
    /// row's actions while hovered or selected; groups' "+".
    #[allow(clippy::too_many_arguments)]
    fn actions_cell(&self, ui: &mut Ui, ws: &WsPalette, inner: Rect, l: &Line<'_>, v: &RowData, show: bool, selected: bool, just: Option<&(&str, String, bool)>, lang: &Language, events: &mut Vec<Event>) {
        let key = l.node.key.as_str();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = 2.0;
        if v.kind == Kind::Group {
            if let Some(tip) = &v.add_into {
                if widgets::icon_button(&mut child, icons::PLUS, 22.0).on_hover_text(tip).clicked() {
                    events.push(Event::AddInto(key.to_owned()));
                }
            }
            return;
        }
        let mut list: Vec<&Action> = Vec::new();
        if let Some((_, tip, can)) = just {
            // Remove stays; Undo in front of it.
            if let Some(rm) = v.actions.iter().find(|a| matches!(a.id, "remove" | "sell")) {
                list.push(rm);
            }
            for a in list {
                action_button(&mut child, ws, a, key, false, events);
            }
            let r = child.add_enabled_ui(*can, |ui| outlined_icon_button(ui, ws, icons::ARROW_COUNTER_CLOCKWISE, ws.stun)).inner;
            let r = r.on_hover_text(tip).on_disabled_hover_text(tip);
            if r.clicked() {
                events.push(Event::Undo(key.to_owned()));
            }
            let _ = lang;
            return;
        }
        if show {
            for a in v.actions.iter().rev() {
                action_button(&mut child, ws, a, key, selected, events);
            }
        }
    }
}

fn row_height_of(kind: Kind, row_h: f32, detail_h: f32) -> f32 {
    match kind {
        Kind::Item => row_h,
        Kind::Group => ROW_H,
        Kind::Empty => EMPTY_H,
        Kind::Detail => detail_h,
    }
}

/// A cell's content rectangle (padding, and the table's edges on the
/// first and last column).
fn cell_inner(r: Rect, first: bool, last: bool, _first_w: f32, _last_w: f32) -> Rect {
    let left = r.left() + PAD + if first { EDGE } else { 0.0 };
    let right = r.right() - PAD - if last { EDGE } else { 0.0 };
    Rect::from_min_max(egui::pos2(left, r.top()), egui::pos2(right.max(left), r.bottom()))
}

/// The row's background: group fill, hover, selection with its accent
/// bar, the teal bar of an added row, the target's outline.
#[allow(clippy::too_many_arguments)]
fn paint_row_back(ui: &Ui, ws: &WsPalette, full: Rect, kind: Kind, selected: bool, hovered: bool, added: bool, target: bool) {
    let p = ui.painter();
    let fill = if selected {
        Some(ws.selection)
    } else if kind == Kind::Group {
        Some(ws.ground)
    } else if kind == Kind::Detail {
        Some(ws.selection)
    } else if added {
        Some(teal_wash(ws))
    } else if hovered && kind == Kind::Item {
        Some(ws.ground)
    } else {
        None
    };
    if let Some(f) = fill {
        p.rect_filled(full, CornerRadius::ZERO, f);
    }
    if kind == Kind::Group {
        p.rect_filled(Rect::from_min_size(full.min, egui::vec2(full.width(), 1.0)), CornerRadius::ZERO, ws.divider);
        p.rect_filled(Rect::from_min_size(egui::pos2(full.left(), full.bottom() - 1.0), egui::vec2(full.width(), 1.0)), CornerRadius::ZERO, ws.divider);
    }
    let bar = if added {
        Some(ws.stun)
    } else if selected || kind == Kind::Detail {
        Some(ws.primary)
    } else {
        None
    };
    if let Some(c) = bar {
        p.rect_filled(Rect::from_min_max(egui::pos2(full.left(), full.top() + 2.0), egui::pos2(full.left() + 3.0, full.bottom() - 2.0)), CornerRadius::same(2), c);
    }
    if target {
        p.rect_stroke(full.shrink(0.5), CornerRadius::ZERO, Stroke::new(1.0_f32, ws.primary), StrokeKind::Inside);
    }
}

/// The just-added row's tint: the teal over the card colour.
pub fn teal_wash(ws: &WsPalette) -> Color32 {
    let a = ws.raised;
    let b = ws.stun;
    let mix = |x: u8, y: u8| (x as f32 * 0.9 + y as f32 * 0.1).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Paint text in a cell, aligned; too long, it ends in "…".
fn paint_text(ui: &Ui, inner: Rect, align: Align, text: &str, mono: bool, color: Color32, bold: bool) {
    let font = if mono {
        FontId::monospace(12.0)
    } else if bold {
        widgets::bold(12.5)
    } else {
        FontId::proportional(12.5)
    };
    let p = ui.painter();
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: (inner.width() + 2.0 * PAD - 2.0).max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = p.layout_job(job);
    let size = g.size();
    let x = match align {
        Align::Left => inner.left(),
        Align::Right => inner.right() - size.x,
        Align::Center => inner.center().x - size.x / 2.0,
    };
    let x = x.max(inner.left() - PAD);
    p.with_clip_rect(inner.expand2(egui::vec2(PAD, 0.0)).intersect(ui.clip_rect())).galley(egui::pos2(x, inner.center().y - size.y / 2.0), g, color);
}

/// A tag after the name; returns its width.
fn paint_tag(p: &egui::Painter, ws: &WsPalette, at: egui::Pos2, t: &Tag) -> f32 {
    let color = t.tone.color(ws);
    let text = match t.icon {
        Some(g) => format!("{g} {}", t.text),
        None => t.text.clone(),
    };
    let g = p.layout_no_wrap(text, FontId::proportional(11.0), color);
    let w = g.size().x;
    if t.chip {
        let r = Rect::from_min_size(egui::pos2(at.x, at.y - 8.5), egui::vec2(w + 14.0, 17.0));
        p.rect_stroke(r, CornerRadius::same(9), Stroke::new(1.0_f32, if t.tone == Tone::Muted { ws.divider } else { color }), StrokeKind::Inside);
        p.galley(egui::pos2(at.x + 7.0, at.y - g.size().y / 2.0), g, color);
        w + 14.0
    } else {
        p.galley(egui::pos2(at.x, at.y - g.size().y / 2.0), g, color);
        w
    }
}

/// Formats a summed value for a column (essence with two decimals, cost
/// in nuyen, plain numbers otherwise).
fn fmt_sum(key: &str, v: f64) -> String {
    match key {
        "ess" => format!("{v:.2}"),
        "cost" => chummer_core::format::nuyen(v),
        _ => chummer_core::improvement::fmt_num(v),
    }
}

enum CellOut {
    Rating(i32),
    Qty(f64),
    Toggle(bool),
}

/// A value cell: text, or its editor (steppers when `live`, toggles and
/// meters always).
#[allow(clippy::too_many_arguments)]
fn edit_cell(ui: &mut Ui, ws: &WsPalette, inner: Rect, c: &Col, cell: &Cell, live: bool, dim: bool, lang: &Language) -> Option<CellOut> {
    let color = if dim { ws.muted } else { cell.tone.color(ws) };
    match &cell.edit {
        Some(Edit::Rating { value, min, max }) if live && max > min => {
            let mut v = *value;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(stepper_rect(inner, c.align)));
            let r = mini_stepper(&mut child, ws, &mut v, *min, *max, lang);
            (r && v != *value).then_some(CellOut::Rating(v))
        }
        Some(Edit::Qty { value, step }) if live => {
            let mut v = *value;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(stepper_rect(inner, c.align)));
            let changed = qty_mini(&mut child, ws, &mut v, *step, lang);
            (changed && v != *value).then_some(CellOut::Qty(v))
        }
        Some(Edit::Toggle { on, on_icon, off_icon, tip }) => {
            let r = Rect::from_center_size(inner.center(), egui::vec2(22.0, 22.0));
            let resp = ui.interact(r, ui.id().with(("toggle", c.key)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), ws.hover);
            }
            icons::paint(ui.painter(), r, if *on { on_icon } else { off_icon }, 13.0, if *on { ws.accent } else { ws.muted });
            let resp = resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
            resp.clicked().then_some(CellOut::Toggle(!*on))
        }
        Some(Edit::Meter { cur, max, label }) => {
            let p = ui.painter();
            let frac = if *max > 0 { (*cur as f32 / *max as f32).clamp(0.0, 1.0) } else { 0.0 };
            let low = *max > 0 && (*cur as f32) < *max as f32 / 2.0;
            let bar_c = if low { ws.warning } else { ws.primary };
            let bar = Rect::from_min_size(egui::pos2(inner.left(), inner.center().y - 2.0), egui::vec2(30.0, 4.0));
            p.rect_filled(bar, CornerRadius::same(2), ws.divider);
            p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(30.0 * frac, 4.0)), CornerRadius::same(2), bar_c);
            let g = p.layout_no_wrap(format!("{cur}/{max}"), FontId::monospace(12.0), if low { ws.warning } else { ws.text });
            let gw = g.size().x;
            let pc = p.with_clip_rect(inner.intersect(ui.clip_rect()));
            pc.galley(egui::pos2(bar.right() + 5.0, inner.center().y - g.size().y / 2.0), g, if low { ws.warning } else { ws.text });
            pc.text(egui::pos2(bar.right() + 10.0 + gw, inner.center().y), Align2::LEFT_CENTER, label, FontId::proportional(11.0), ws.muted);
            None
        }
        _ => {
            paint_text(ui, inner, c.align, &cell.text, c.mono, color, false);
            if !cell.tip.is_empty() {
                ui.interact(inner, ui.id().with(("tip", c.key)), Sense::hover()).on_hover_text(&cell.tip);
            }
            None
        }
    }
}

/// Where a 58×20 stepper goes in a cell.
fn stepper_rect(inner: Rect, align: Align) -> Rect {
    let size = egui::vec2(STEPPER_W, 20.0);
    match align {
        Align::Left => Rect::from_min_size(egui::pos2(inner.left(), inner.center().y - 10.0), size),
        Align::Right => Rect::from_min_size(egui::pos2(inner.right() - STEPPER_W, inner.center().y - 10.0), size),
        Align::Center => Rect::from_center_size(inner.center(), size),
    }
}

const STEPPER_W: f32 = 58.0;

/// A compact − value + stepper (20px). Returns whether a button moved it.
fn mini_stepper(ui: &mut Ui, ws: &WsPalette, v: &mut i32, min: i32, max: i32, lang: &Language) -> bool {
    let h = 20.0;
    let w = STEPPER_W;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(5), ws.well, Stroke::new(1.0_f32, ws.control), StrokeKind::Inside);
    let minus = Rect::from_min_size(rect.min, egui::vec2(18.0, h));
    let plus = Rect::from_min_size(egui::pos2(rect.right() - 18.0, rect.top()), egui::vec2(18.0, h));
    p.text(rect.center(), Align2::CENTER_CENTER, v.to_string(), FontId::monospace(12.0), ws.text);
    p.line_segment([egui::pos2(minus.right(), rect.top() + 1.0), egui::pos2(minus.right(), rect.bottom() - 1.0)], Stroke::new(1.0_f32, ws.divider));
    p.line_segment([egui::pos2(plus.left(), rect.top() + 1.0), egui::pos2(plus.left(), rect.bottom() - 1.0)], Stroke::new(1.0_f32, ws.divider));
    let mut changed = false;
    for (r, glyph, d, tip) in [(minus, icons::MINUS, -1, lang.tr("Lower Rating")), (plus, icons::PLUS, 1, lang.tr("Raise Rating"))] {
        let ok = if d < 0 { *v > min } else { *v < max };
        let resp = ui.interact(r, ui.id().with(("step", d)), if ok { Sense::click() } else { Sense::hover() });
        icons::paint(ui.painter(), r, glyph, 10.0, if !ok { ws.muted.gamma_multiply(0.4) } else if resp.hovered() { ws.text } else { ws.muted });
        if resp.on_hover_text(tip).clicked() && ok {
            *v += d;
            changed = true;
        }
    }
    changed
}

/// A compact quantity stepper. Returns whether a button moved it.
fn qty_mini(ui: &mut Ui, ws: &WsPalette, v: &mut f64, step: f64, lang: &Language) -> bool {
    let mut n = (*v / step).round() as i32;
    let changed = mini_stepper(ui, ws, &mut n, 1, 100_000, lang);
    if changed {
        *v = n as f64 * step;
    }
    changed
}

/// An icon button with an outline in `color` (the inline Undo).
fn outlined_icon_button(ui: &mut Ui, ws: &WsPalette, glyph: &str, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::click());
    let enabled = ui.is_enabled();
    let c = if enabled { color } else { ws.muted.gamma_multiply(0.5) };
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, CornerRadius::same(5), ws.hover);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0_f32, c), StrokeKind::Inside);
    icons::paint(ui.painter(), rect, glyph, 13.0, c);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One row action: an icon button, a filled one for `primary`, or a
/// menu.
fn action_button(ui: &mut Ui, ws: &WsPalette, a: &Action, key: &str, selected: bool, events: &mut Vec<Event>) {
    let r = if a.primary && selected {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::click());
        ui.painter().rect_filled(rect, CornerRadius::same(5), if resp.hovered() { ws.primary.gamma_multiply(0.9) } else { ws.primary });
        icons::paint(ui.painter(), rect, a.icon, 13.0, ws.on_primary);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        widgets::icon_button(ui, a.icon, 22.0)
    };
    let r = r.on_hover_text(&a.tip);
    if a.menu.is_empty() {
        if r.clicked() {
            events.push(Event::Action(key.to_owned(), a.id, None));
        }
    } else {
        egui::Popup::menu(&r).show(|ui| {
            for (id, label) in &a.menu {
                if ui.button(label).clicked() {
                    events.push(Event::Action(key.to_owned(), a.id, Some(id.clone())));
                }
            }
        });
    }
}

/// A dashed card with a title, a line and a button. Returns whether the
/// button was clicked.
fn empty_card(ui: &mut Ui, ws: &WsPalette, card: Rect, title: &str, sub: &str, button: Option<&str>) -> bool {
    let p = ui.painter();
    dashed_rect(p, card, ws.control);
    let mid = card.center().y;
    icons::paint(p, Rect::from_min_size(egui::pos2(card.left() + 12.0, mid - 8.0), egui::vec2(16.0, 16.0)), icons::CIRCLE_DASHED, 16.0, ws.muted);
    let tx = card.left() + 12.0 + 16.0 + 10.0;
    if sub.is_empty() {
        p.text(egui::pos2(tx, mid), Align2::LEFT_CENTER, title, FontId::proportional(12.0), ws.text);
    } else {
        p.text(egui::pos2(tx, mid - 7.0), Align2::LEFT_CENTER, title, FontId::proportional(12.0), ws.text);
        p.text(egui::pos2(tx, mid + 8.0), Align2::LEFT_CENTER, sub, FontId::proportional(11.0), ws.muted);
    }
    let Some(b) = button else { return false };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink2(egui::vec2(12.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    widgets::button(&mut child, Some(icons::PLUS), b, widgets::Look::Ghost, 24.0).clicked()
}

fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let s = Stroke::new(1.0_f32, color);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        p.extend(egui::Shape::dashed_line(&[w[0], w[1]], s, 4.0, 3.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<Col> {
        vec![
            Col::name("Name"),
            Col::new("rating", "Rating").px(64.0).center().prio(70).fold(),
            Col::new("grade", "Grade").px(48.0).prio(50).fold(),
            Col::new("ess", "Ess").px(46.0).num().prio(80).sum(),
            Col::new("cap", "Cap").px(50.0).num().prio(40),
            Col::new("wireless", "Wireless").px(26.0).icon(icons::WIFI_HIGH).prio(20),
            Col::new("avail", "Avail").px(42.0).num().prio(30),
            Col::new("cost", "Cost").px(80.0).num().prio(90).sum(),
            Col::new("source", "Source").px(56.0).prio(10),
            Col::actions(98.0),
        ]
    }

    fn keys(cols: &[Col], l: &ColLayout) -> Vec<&'static str> {
        l.shown.iter().map(|i| cols[*i].key).collect()
    }

    #[test]
    fn wide_tables_show_every_column() {
        let c = cols();
        let l = layout(&c, 1200.0, &[]);
        assert_eq!(l.shown.len(), c.len());
        assert!(l.folded.is_empty());
        assert!((l.widths.iter().sum::<f32>() - 1200.0).abs() < 0.01, "the columns fill the width: {:?}", l.widths);
    }

    #[test]
    fn narrow_tables_hide_lowest_priority_first() {
        let c = cols();
        // Source (10) goes first, then Wireless (20), Avail (30), Cap (40).
        let fixed: f32 = c.iter().filter_map(|c| if let Width::Px(w) = c.width { Some(w + 2.0 * PAD) } else { None }).sum();
        let l = layout(&c, fixed + 2.0 * EDGE + MIN_NAME - 1.0, &[]);
        assert!(!keys(&c, &l).contains(&"source"));
        assert!(keys(&c, &l).contains(&"wireless"));
        let l = layout(&c, 700.0, &[]);
        assert_eq!(keys(&c, &l), ["name", "rating", "grade", "ess", "cap", "cost", "actions"]);
        assert!(l.folded.is_empty());
    }

    #[test]
    fn at_470_rating_and_grade_fold_into_the_name() {
        let c = cols();
        let l = layout(&c, 470.0, &[]);
        assert_eq!(keys(&c, &l), ["name", "ess", "cost", "actions"]);
        assert_eq!(l.folded, [1, 2], "rating and grade go into the name");
        let row = RowData { name: "Wired Reflexes".into(), cells: vec![Cell::num("1", Some(1.0)), Cell::text("Alpha"), Cell::num("1.60", Some(1.6))], ..Default::default() };
        assert_eq!(folded_name(&row, &l.folded), "Wired Reflexes 1 Alpha");
        let row = RowData { name: "Datajack".into(), cells: vec![Cell::text("—"), Cell::text("Std").tone(Tone::Muted)], ..Default::default() };
        assert_eq!(folded_name(&row, &l.folded), "Datajack", "neither a dash nor a muted Standard grade is folded in");
    }

    #[test]
    fn the_columns_menu_overrides_priority() {
        let c = cols();
        // Turned off: gone even when there is room.
        let l = layout(&c, 1200.0, &[("cost".into(), false)]);
        assert!(!keys(&c, &l).contains(&"cost"));
        // Turned on: kept while others go.
        let l = layout(&c, 700.0, &[("source".into(), true)]);
        assert!(keys(&c, &l).contains(&"source"));
        assert!(!keys(&c, &l).contains(&"cap"));
        // Name and the actions never go.
        let l = layout(&c, 100.0, &[("name".into(), false), ("actions".into(), false)]);
        assert_eq!(keys(&c, &l), ["name", "actions"]);
    }

    fn item(key: &str, name: &str, cost: Option<f64>) -> Row {
        Node::new(key, RowData { name: name.into(), cells: vec![Cell::default(), Cell::default(), Cell::default(), Cell::default(), Cell::default(), Cell::default(), Cell::num(cost.map(|c| c.to_string()).unwrap_or_default(), cost)], ..Default::default() })
    }

    fn group(key: &str, children: Vec<Row>) -> Row {
        let mut n = Node::new(key, RowData { kind: Kind::Group, name: key.into(), ..Default::default() });
        n.children = children;
        n
    }

    fn names(nodes: &[Row]) -> Vec<String> {
        flatten(nodes, &|_| true).iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.node.value.name)).collect()
    }

    const COST: usize = 7;

    #[test]
    fn sorting_is_stable_among_siblings_and_keeps_groups() {
        let mut arm = item("arm", "Cyberarm", Some(15000.0));
        arm.children = vec![item("g", "Gyromount", Some(6000.0)), item("s", "Smuggling", Some(7500.0)), item("h", "Holster", Some(6000.0))];
        let mut t = vec![
            group("Body", vec![item("w", "Wired Reflexes", Some(46800.0)), item("m", "Muscle Toner", Some(64000.0)), item("o", "Orthoskin", Some(6000.0)), item("x", "No price", None)]),
            group("Arm", vec![arm]),
            item("loose", "Loose", Some(1.0)),
        ];
        sort_tree(&mut t, COST, true);
        assert_eq!(
            names(&t),
            ["Body", "  Orthoskin", "  Wired Reflexes", "  Muscle Toner", "  No price", "Arm", "  Cyberarm", "    Gyromount", "    Holster", "    Smuggling", "Loose"],
            "groups stay; equal costs keep their order; no price last"
        );
        sort_tree(&mut t, COST, false);
        assert_eq!(names(&t)[1..5], ["  Muscle Toner", "  Wired Reflexes", "  Orthoskin", "  No price"], "descending, blanks still last");
        // Gyromount and Holster tie: still in their earlier order.
        assert_eq!(names(&t)[7..10], ["    Smuggling", "    Gyromount", "    Holster"]);
        sort_tree(&mut t, 0, true);
        assert_eq!(names(&t)[1..5], ["  Muscle Toner", "  No price", "  Orthoskin", "  Wired Reflexes"]);
    }

    #[test]
    fn items_sort_around_groups_in_mixed_lists() {
        let mut t = vec![item("b", "B", Some(2.0)), group("Trunk", vec![]), item("a", "A", Some(1.0))];
        sort_tree(&mut t, COST, true);
        assert_eq!(names(&t), ["A", "Trunk", "B"]);
    }

    #[test]
    fn subtotals_sum_every_item_below_a_group() {
        let mut arm = item("arm", "Cyberarm", Some(15000.0));
        arm.children = vec![item("g", "Gyromount", Some(6000.0)), item("x", "Free", None)];
        let g = group("Arm", vec![arm, item("e", "Eyes", Some(6000.0))]);
        assert_eq!(subtotal(&g, COST), Some(27000.0));
        assert_eq!(subtotal(&g, 1), None, "nothing to sum");
        let t = vec![g, item("d", "Datajack", Some(1000.0))];
        assert_eq!(total(&t, COST), Some(28000.0));
        assert_eq!(item_count(&t), 5);
    }

    #[test]
    fn folded_nodes_hide_their_rows() {
        let mut eyes = item("eyes", "Cybereyes", Some(6000.0));
        eyes.children = vec![item("s", "Smartlink", Some(4000.0)), item("f", "Flare", Some(1000.0))];
        let t = vec![group("Head", vec![eyes, item("d", "Datajack", Some(1000.0))])];
        let closed: HashSet<String> = ["eyes".to_owned()].into();
        let rows = flatten(&t, &|n: &Row| !closed.contains(&n.key));
        let shown: Vec<&str> = rows.iter().map(|r| r.node.value.name.as_str()).collect();
        assert_eq!(shown, ["Head", "Cybereyes", "Datajack"]);
        assert!(!rows[1].open, "folded");
        assert!(rows[2].last);
        assert_eq!(parent_keys(&t), ["Head".to_owned(), "eyes".to_owned()].into());
    }
}
