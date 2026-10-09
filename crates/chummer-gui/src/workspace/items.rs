//! The Workspace's item pages: Cyberware & Bioware, the Street Gear
//! sub-tabs (Gear, Clothing & Armor, Weapons, Drugs, Lifestyles) and
//! Vehicles & Drones ("D · Purchase & inventory").
//!
//! Each page is a toolbar, the page's summary (essence; the lifestyle
//! editor) and the inventory: the section's items in the item table
//! (`ws_inventory`, `workspace::table`) in a fixed-height panel that
//! scrolls itself. "Add …" opens the inline catalog (`ws_catalog`) next to
//! the inventory, side by side, or above it with a drag handle when the
//! page is narrow; the inventory stays in view, and what is added shows
//! there at once, marked, with an inline Undo and the "Added since you
//! opened this page" tray. A click on an item opens it in the inspector
//! (`ws_inspector`) and, while the catalog is open, makes it the catalog's
//! target when it can hold what the catalog sells.
//!
//! A child module of `view` (declared there with `#[path]`) so it can
//! use the view's state; changes go through the same commands as the
//! Classic tab pages.

use std::collections::HashSet;
use std::sync::Arc;

use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::items::{edit, place};
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section as Sec};
use chummer_core::sources::SourcebookLibrary;
use eframe::egui::{self, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind};

use super::ws_inventory::{self, DRUGS};
use super::{kind_noun, CharacterView, Tab, STREET_GEAR};
use crate::pdf_ui::Status;
use crate::theme;
use crate::workspace::icons;
use crate::workspace::table::{self, EmptyCard, Footer, Row};
use crate::workspace::widgets::{self, Look};

/// The page is laid out side by side from this width (else stacked).
pub const SIDE_BY_SIDE: f32 = 820.0;
/// Below this window width the inspector folds into the catalog's
/// details strip while the catalog is open.
pub const FOLD_INSPECTOR: f32 = 1180.0;
const HEAD_H: f32 = 36.0;
const HANDLE_H: f32 = 10.0;

/// The Workspace's item-page state, kept in the view.
#[derive(Default)]
pub struct GearState {
    /// The inline catalog, while one is open.
    pub(super) catalog: Option<super::ws_catalog::Catalog>,
    /// The Workspace item inspector's own state (sell percentage,
    /// location being typed, ammunition choices).
    pub(super) editor: super::ws_inspector::WsItemEditor,
    /// The inventory rows per revision (`ws_inventory_rows`).
    pub(super) rows: crate::memo::Memo<ws_inventory::RowsKey, Arc<Vec<Row>>>,
    /// The page being visited and what was bought on it.
    pub(super) visit: Option<Visit>,
    /// Scroll the inventory to this row next frame (just added).
    pub(super) scroll_to: Option<String>,
    /// Keys go to the catalog (else the inventory); Tab switches.
    pub(super) focus_catalog: bool,
    /// While a row is dragged: the inventory's drop targets
    /// (`ws_inventory_drops`), with the key they were computed for.
    pub(super) drops: Option<(u64, table::Drops)>,
    /// The kind last chosen for adding into an item, by the item's kind
    /// ("vehicle" → "weaponmount"), for this session.
    pub(super) into_kinds: std::collections::HashMap<String, &'static str>,
}

/// A visit of an item page: from showing it until another page shows.
pub struct Visit {
    pub page: Page,
    /// Items bought here, oldest first.
    pub adds: Vec<Added>,
}

/// An item bought on the page: which, and where its command sits in the
/// session log (to tell whether Undo would take back exactly it).
pub struct Added {
    pub guid: String,
    /// The session log's length right after it.
    pub log_len: usize,
}

/// An item page: its tab and, on Street Gear, the sub-tab.
pub type Page = (Tab, usize);

/// The kinds bought on a page whose items are in `container` (what
/// Classic's "Add …" buttons offer).
pub(crate) fn page_kinds(container: &str) -> &'static [&'static str] {
    match container {
        "gears" => &["gear"],
        "cyberwares" => &["cyberware", "bioware"],
        "armors" => &["armor", "armormod"],
        "weapons" => &["weapon", "accessory"],
        "vehicles" => &["vehicle", "mod"],
        "lifestyles" => &["lifestyle"],
        _ => &[],
    }
}

/// The section of an item page, `None` for Drugs (no tree section).
fn page_section(page: Page) -> Option<Sec> {
    match page.0 {
        Tab::Cyberware => Some(sections::CYBERWARE),
        Tab::Vehicles => Some(sections::VEHICLES),
        Tab::StreetGear => STREET_GEAR.get(page.1).and_then(|(_, s)| *s),
        _ => None,
    }
}

/// The container of an item page's items ("" for Drugs).
pub(crate) fn page_container(page: Page) -> &'static str {
    page_section(page).map_or("", |s| s.container)
}

impl CharacterView {
    /// The page the view shows, when it is an item page.
    pub(crate) fn ws_item_page(&self, tab: Tab) -> Option<Page> {
        match tab {
            Tab::Cyberware | Tab::Vehicles => Some((tab, 0)),
            Tab::StreetGear => Some((tab, self.gear_tab)),
            _ => None,
        }
    }

    /// Whether the inspector folds away: a narrow window with the catalog
    /// open on the page shown and no owned item selected (the catalog's
    /// details strip shows its record instead).
    pub(crate) fn ws_inspector_folded(&self, window_width: f32) -> bool {
        window_width < FOLD_INSPECTOR && (self.item_editor.is_none() || self.ws_catalog_inspecting()) && self.ws_gear.catalog.as_ref().is_some_and(|c| self.ws_item_page(self.tab) == Some(c.page))
    }

    /// The Workspace page for an item tab, `None` for other tabs. Returns
    /// whether the character changed.
    pub(crate) fn ws_items_page(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<bool> {
        let page = self.ws_item_page(tab)?;
        let sec = page_section(page).unwrap_or(DRUGS);
        if self.ws_gear.visit.as_ref().is_none_or(|v| v.page != page) {
            self.ws_gear.visit = Some(Visit { page, adds: Vec::new() });
        }
        let open = self.ws_gear.catalog.as_ref().is_some_and(|c| c.page == page);
        let mut changed = false;
        ui.spacing_mut().item_spacing.y = 10.0;
        let wide = ui.available_width().min(ui.clip_rect().width()) >= SIDE_BY_SIDE;
        self.ws_items_toolbar(ui, page, &sec, open, wide, lang);
        if !open {
            match sec.container {
                "cyberwares" => self.ws_essence_card(ui, lang),
                "lifestyles" if !self.doc.items("lifestyles", "lifestyle").is_empty() => changed |= self.ws_lifestyle_card(ui, engine, lang, pdfs, status),
                _ => {}
            }
        }
        // Within the page (a wide card above may have widened the Ui).
        let rect = ui.available_rect_before_wrap();
        let clip = ui.clip_rect();
        let rect = Rect::from_min_max(rect.min, egui::pos2(rect.right().min(clip.right()), rect.bottom().min(clip.bottom()).max(rect.top() + 200.0)));
        let open = self.ws_gear.catalog.as_ref().is_some_and(|c| c.page == page);
        if open {
            // Tab switches between the catalog and the inventory.
            let keys_free = !ui.ctx().wants_keyboard_input() || ui.ctx().memory(|m| m.has_focus(egui::Id::new("ws_catalog_search")));
            if keys_free && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
                self.ws_gear.focus_catalog = !self.ws_gear.focus_catalog;
                ui.memory_mut(|m| m.surrender_focus(egui::Id::new("ws_catalog_search")));
            }
            let fold = self.ws_inspector_folded(ui.ctx().content_rect().width());
            if rect.width() >= SIDE_BY_SIDE {
                let cat_w = (rect.width() * 0.47).clamp(380.0, 540.0);
                let left = Rect::from_min_size(rect.min, egui::vec2(cat_w, rect.height()));
                let right = Rect::from_min_max(egui::pos2(left.right() + 12.0, rect.top()), rect.max);
                changed |= self.ws_catalog_panel(ui, left, false, fold, engine, lang, pdfs, status);
                changed |= self.ws_inventory_panel(ui, right, page, &sec, false, lang, pdfs, status);
            } else {
                // Stacked: the catalog above, the inventory below a handle.
                let id = egui::Id::new(("ws_items_split", page.0 as u8, page.1));
                let mut inv_h: f32 = ui.ctx().data_mut(|d| d.get_persisted(id)).unwrap_or(318.0);
                inv_h = inv_h.clamp(120.0, (rect.height() - 180.0).max(120.0));
                let top = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.bottom() - inv_h - HANDLE_H));
                let handle = Rect::from_min_max(egui::pos2(rect.left(), top.bottom()), egui::pos2(rect.right(), top.bottom() + HANDLE_H));
                let bottom = Rect::from_min_max(egui::pos2(rect.left(), handle.bottom()), rect.max);
                changed |= self.ws_catalog_panel(ui, top, true, fold, engine, lang, pdfs, status);
                let r = ui.interact(handle, id.with("handle"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(lang.tr("Resize catalog and inventory"));
                let ws = theme::ws(ui);
                let bar = Rect::from_center_size(handle.center(), egui::vec2(36.0, 4.0));
                ui.painter().rect_filled(bar, CornerRadius::same(2), if r.hovered() || r.dragged() { ws.muted } else { ws.control });
                if r.dragged() {
                    inv_h -= r.drag_delta().y;
                    ui.ctx().data_mut(|d| d.insert_persisted(id, inv_h.clamp(120.0, (rect.height() - 180.0).max(120.0))));
                }
                changed |= self.ws_inventory_panel(ui, bottom, page, &sec, true, lang, pdfs, status);
            }
        } else {
            changed |= self.ws_inventory_panel(ui, rect, page, &sec, false, lang, pdfs, status);
        }
        ui.allocate_rect(rect, Sense::hover());
        Some(changed)
    }

    /// The page's toolbar: a line about the page, the layout switch while
    /// the catalog is open, and the "Add …" buttons.
    fn ws_items_toolbar(&mut self, ui: &mut egui::Ui, page: Page, sec: &Sec, open: bool, wide: bool, lang: &Language) {
        let ws = theme::ws(ui);
        let mut buy = None;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(self.ws_page_note(sec, lang)).size(12.0).color(ws.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if sec.container == "drugs" {
                    if widgets::button(ui, Some(icons::FLASK), &lang.tr("Build custom drug…"), Look::Primary, 26.0).clicked() {
                        self.drug_builder.open = true;
                    }
                    return;
                }
                let kinds = page_kinds(sec.container);
                if open {
                    let shop = if wide { lang.tr("Side by side") } else { lang.tr("Stacked") };
                    if widgets::segmented(ui, &[(&shop, ""), (&lang.tr("Inventory only"), "")], 0, 26.0) == Some(1) {
                        close = true;
                    }
                } else {
                    for (i, tag) in kinds.iter().enumerate().rev() {
                        let label = chummer_core::items::kind(tag).map_or(*tag, |k| k.label);
                        let look = if i == 0 { Look::Primary } else { Look::Secondary };
                        let r = widgets::button(ui, Some(icons::PLUS), &lang.tr_fmt("Add {0}", &[&kind_noun(lang, label)]), look, 26.0);
                        let r = match super::select::parent_of(tag) {
                            Some(_) => r.on_hover_text(lang.tr("Choose where to install it")),
                            None => r,
                        };
                        if r.clicked() {
                            buy = Some(*tag);
                        }
                    }
                }
            });
        });
        if let Some(tag) = buy {
            // Into the selected item when it takes that kind; else at the
            // top level (a vehicle selected and "Add vehicle": a vehicle).
            let parent = self.item_editor.as_ref().map(|(g, _)| g.clone()).filter(|g| place::accepts(&self.doc, &self.store, g).iter().any(|k| k.tag == tag));
            self.ws_open_catalog(page, tag);
            if let Some(g) = parent {
                self.ws_catalog_into(page, &g, Some(tag));
            }
        }
        if close {
            self.ws_gear.catalog = None;
        }
    }

    /// "3 items · Essence 5.45" and the like, in the toolbar.
    fn ws_page_note(&self, sec: &Sec, lang: &Language) -> String {
        let count = self.doc.doc.child(sec.container).map_or(0, |c| c.children_named(sec.item).count());
        let items = lang.tr_fmt("{0} items", &[&count]);
        match sec.container {
            "cyberwares" => format!("{items} · {} {}", lang.tr("Essence"), format::essence(self.sheet.essence, self.rules.essence_decimals)),
            _ => match &self.budget {
                Some(b) => format!("{items} · {} {}", lang.tr("Nuyen left"), format::nuyen(b.nuyen_left())),
                None => format!("{items} · {}", format::nuyen(self.doc.nuyen)),
            },
        }
    }

    /// Cyberware & Bioware: the essence left and what the ware takes.
    fn ws_essence_card(&self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let s = &self.sheet;
        let d = self.rules.essence_decimals;
        widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                widgets::budget_chip(ui, &lang.tr("Essence"), &format::essence(s.essence, d), Some((s.essence / 6.0) as f32), widgets::Tone::Normal);
                widgets::budget_chip(ui, &lang.tr("Cyberware"), &format::essence(s.cyberware_essence, d), None, widgets::Tone::Normal);
                widgets::budget_chip(ui, &lang.tr("Bioware"), &format::essence(s.bioware_essence, d), None, widgets::Tone::Normal);
                widgets::budget_chip(ui, &lang.tr("Initiative"), &format!("{} + {}d6", s.initiative, s.initiative_dice), None, widgets::Tone::Normal);
                widgets::budget_chip(ui, &lang.tr("Armor"), &s.armor.to_string(), None, widgets::Tone::Normal);
            });
        });
    }

    /// Lifestyles: the lifestyle editor in a card of its own height
    /// (scrolls when long, so the table keeps room).
    fn ws_lifestyle_card(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let max_h = (ui.available_height() * 0.45).max(160.0);
        let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("ws_lifestyle_editor").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                changed |= self.lifestyle_editor.ui(ui, &mut self.doc, &cx, status);
            });
        });
        changed
    }

    /// The inventory panel in `rect`: its head (title, count, what was
    /// bought here, expand/collapse, columns), the table and the "Added
    /// since you opened this page" tray. `compact`: the stacked layout
    /// (the tray folds into the head).
    #[allow(clippy::too_many_arguments)]
    fn ws_inventory_panel(&mut self, ui: &mut egui::Ui, rect: Rect, page: Page, sec: &Sec, compact: bool, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let catalog_open = self.ws_gear.catalog.as_ref().is_some_and(|c| c.page == page);
        let cols = ws_inventory::columns(sec.container, self.doc.created, lang);
        let id = ws_inventory::table_id(page);
        let sort = table::sort_of(ui.ctx(), id);
        let group = if sec.container == "drugs" { table::GroupBy::Default } else { table::group_of(ui.ctx(), id) };
        let rows = self.ws_inventory_rows(sec, &cols, lang, sort, group);
        self.ws_inventory_drops(ui.ctx(), page, sec, &rows, lang);
        let added: HashSet<String> = self.ws_gear.visit.as_ref().map(|v| v.adds.iter().map(|a| a.guid.clone()).filter(|g| edit::find(&self.doc, g).is_some()).collect()).unwrap_or_default();
        let events;
        let mut undo_all = false;
        let mut undo_one: Option<String> = None;
        let mut fold_all: Option<bool> = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut child;
        ui.painter().rect(rect, CornerRadius::same(6), ws.raised, Stroke::new(1.0_f32, ws.divider), StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        ui.set_clip_rect(inner.intersect(ui.clip_rect()));
        let tray_lines = if compact || !catalog_open { 0 } else { added.len().min(3) };
        let tray_h = if tray_lines > 0 { 40.0 + tray_lines as f32 * 24.0 } else { 0.0 };
        // Head.
        let head = Rect::from_min_size(inner.min, egui::vec2(inner.width(), HEAD_H));
        ui.painter().rect_filled(Rect::from_min_size(egui::pos2(head.left(), head.bottom() - 1.0), egui::vec2(head.width(), 1.0)), CornerRadius::ZERO, ws.divider);
        let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(egui::vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        hu.spacing_mut().item_spacing.x = 8.0;
        let (glyph, title) = match sec.container {
            "cyberwares" => (icons::USER_FOCUS, lang.tr("Installed")),
            "gears" => (icons::PACKAGE, lang.tr("Your gear")),
            "drugs" => (icons::PILL, lang.tr("Your drugs")),
            _ => (super::ws_inventory::kind_icon(sec.item), lang.tr(sec.label)),
        };
        hu.label(icons::icon(glyph, 15.0, ws.accent));
        hu.label(widgets::title(&title, &ws));
        widgets::count_pill(&mut hu, &table::item_count(&rows).to_string(), ws.selection, ws.accent);
        hu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            table::columns_button(ui, id, &cols, lang);
            if sec.container != "drugs" {
                let default = if sec.container == "cyberwares" { lang.tr("Type") } else if sec.container == "lifestyles" { lang.tr("Chummer's groups") } else { lang.tr("Location") };
                table::group_button(ui, id, &[(table::GroupBy::Default, default), (table::GroupBy::Category, lang.tr("Category")), (table::GroupBy::None, lang.tr("None"))], lang);
            }
            let parents = table::parent_keys(&rows);
            if !parents.is_empty() {
                let all_open = table::closed_of(ui.ctx(), id).is_empty();
                let (g, t) = if all_open { (icons::ARROWS_IN_LINE_VERTICAL, lang.tr("Collapse all")) } else { (icons::ARROWS_OUT_LINE_VERTICAL, lang.tr("Expand all")) };
                if widgets::button(ui, Some(g), &t, Look::Ghost, 24.0).clicked() {
                    fold_all = Some(all_open);
                }
            }
            if compact && !added.is_empty() {
                let online = self.doc.is_online();
                let r = ui.add_enabled_ui(!online, |ui| widgets::button(ui, Some(icons::ARROW_COUNTER_CLOCKWISE), &lang.tr("Undo all"), Look::Ghost, 24.0)).inner;
                if r.on_hover_text(lang.tr("Take back everything bought on this page")).on_disabled_hover_text(lang.tr(crate::doc::ONLINE_UNDO)).clicked() {
                    undo_all = true;
                }
                ui.label(RichText::new(lang.tr_fmt("{0} added on this page", &[&added.len()])).size(11.5).color(ws.stun));
                ui.label(icons::icon(icons::CLOCK_COUNTER_CLOCKWISE, 13.0, ws.stun));
            }
        });
        // The selected item takes other items: what the catalog does with
        // it, or a way to add into it.
        let mut add_into = None;
        let hint = self.ws_into_hint(sec, page, compact, lang);
        let hint_h = if hint.is_some() { 32.0 } else { 0.0 };
        if let Some((text, button)) = hint {
            let strip = Rect::from_min_size(egui::pos2(inner.left(), head.bottom()), egui::vec2(inner.width(), hint_h));
            ui.painter().rect_filled(strip, CornerRadius::ZERO, ws.selection);
            ui.painter().rect_filled(Rect::from_min_size(egui::pos2(strip.left(), strip.bottom() - 1.0), egui::vec2(strip.width(), 1.0)), CornerRadius::ZERO, ws.divider);
            let mut su = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(egui::vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            su.spacing_mut().item_spacing.x = 6.0;
            su.label(icons::icon(icons::ARROW_BEND_DOWN_RIGHT, 13.0, ws.accent));
            su.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some((g, label)) = &button {
                    if widgets::button(ui, Some(icons::PLUS), label, Look::Secondary, 24.0).clicked() {
                        add_into = Some(g.clone());
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&text).size(12.0).color(ws.text)).truncate()).on_hover_text(&text);
                });
            });
        }
        // Table.
        let table_rect = Rect::from_min_max(egui::pos2(inner.left(), head.bottom() + hint_h), egui::pos2(inner.right(), inner.bottom() - tray_h));
        let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(table_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        let essence_left = (sec.container == "cyberwares").then(|| format::essence(self.sheet.essence, self.rules.essence_decimals));
        let footer = Footer {
            left: match &essence_left {
                Some(_) => format!("{} · {}", ws_inventory::count_line(lang, &rows), lang.tr("Essence left")),
                None => ws_inventory::count_line(lang, &rows),
            },
            accent: essence_left.unwrap_or_default(),
            cells: Vec::new(),
            tail: lang.tr("purchase value"),
        };
        let kinds = page_kinds(sec.container);
        let empty = EmptyCard {
            title: lang.tr_fmt("No {0} yet", &[&kind_noun(lang, sec.label)]),
            sub: if kinds.is_empty() { lang.tr("Build one from its components.") } else { lang.tr("Buy from the catalog; it opens next to this list.") },
            button: if kinds.is_empty() { lang.tr("Build custom drug…") } else { lang.tr("Buy…") },
        };
        let focused = !catalog_open || !self.ws_gear.focus_catalog;
        {
            let states = self.ws_inventory_states(lang, &added);
            let t = table::Table::new(ws_inventory::table_salt(page), &cols).height(table_rect.height()).footer(footer).focused(focused).draggable().row_menu();
            let t = t.empty(empty);
            events = t.show(&mut tu, &rows, &states, lang, |_, _| {});
        }
        self.ws_gear.scroll_to = None;
        if tu.ui_contains_pointer() && tu.input(|i| i.pointer.any_pressed()) {
            self.ws_gear.focus_catalog = false;
        }
        // Tray.
        if tray_lines > 0 {
            let tray = Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - tray_h), inner.max);
            ui.painter().rect_filled(Rect::from_min_size(tray.min, egui::vec2(tray.width(), 1.0)), CornerRadius::ZERO, ws.divider);
            let mut tr = ui.new_child(egui::UiBuilder::new().max_rect(tray.shrink2(egui::vec2(10.0, 8.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            tr.spacing_mut().item_spacing.y = 2.0;
            let (a, b) = self.ws_tray(&mut tr, sec, &added, lang);
            undo_all |= a;
            undo_one = undo_one.or(b);
        }
        // Events.
        if let Some(close) = fold_all {
            table::set_closed(ui.ctx(), id, if close { table::parent_keys(&rows) } else { HashSet::new() });
        }
        let mut changed = false;
        if let Some(g) = add_into {
            self.ws_add_into(page, &g, None);
        }
        let out = self.ws_inventory_events(events, page, sec, lang, pdfs, status);
        changed |= out.changed;
        if let Some(g) = undo_one {
            changed |= self.ws_undo_added(sec, &g, status);
        }
        if undo_all {
            let adds: Vec<String> = self.ws_gear.visit.as_ref().map(|v| v.adds.iter().rev().map(|a| a.guid.clone()).collect()).unwrap_or_default();
            for g in adds {
                if edit::find(&self.doc, &g).is_some() {
                    changed |= self.ws_undo_added(sec, &g, status);
                }
            }
        }
        if let Some((key, to)) = out.drop_buy {
            changed |= self.ws_catalog_drop_buy(&key, to, lang, status);
        }
        if let Some((tag, into)) = out.buy {
            if !catalog_open || self.ws_gear.catalog.as_ref().is_some_and(|c| !c.sells(&tag) || c.target().is_some()) {
                self.ws_open_catalog(page, &tag);
            }
            self.ws_catalog_set_location(into, lang);
            self.ws_gear.focus_catalog = true;
        }
        changed
    }

    /// The inventory's line about the selected item when it takes other
    /// items: "Buying into X — pick what to add above" while the catalog
    /// adds into it, else what it takes and a button to add into it.
    /// `stacked`: the catalog is above the inventory (else on its left).
    fn ws_into_hint(&self, sec: &Sec, page: Page, stacked: bool, lang: &Language) -> Option<(String, Option<(String, String)>)> {
        let g = self.item_editor.as_ref().map(|(g, _)| g.as_str())?;
        if place::root_container(&self.doc, g) != Some(sec.container) {
            return None;
        }
        let el = edit::find(&self.doc, g)?;
        let name = super::display_name(sec, el, lang);
        let into = self.ws_gear.catalog.as_ref().is_some_and(|c| c.page == page && c.anchor().is_some_and(|a| a.eq_ignore_ascii_case(g)));
        if into {
            let t = if stacked { "Buying into {0} — pick what to add above" } else { "Buying into {0} — pick what to add on the left" };
            return Some((lang.tr_fmt(t, &[&name]), None));
        }
        let accepted = place::accepts(&self.doc, &self.store, g);
        if accepted.is_empty() {
            return None;
        }
        let ptag = edit::tag_of(el);
        let kinds: Vec<String> = accepted.iter().map(|a| super::ws_catalog::into_label(lang, ptag, a.tag, a.label)).collect();
        Some((lang.tr_fmt("{0} takes {1}", &[&name, &kinds.join(", ")]), Some((g.to_owned(), lang.tr_fmt("Add into {0}", &[&name])))))
    }

    /// "Added since you opened this page": a line per item, newest first,
    /// with its essence and cost and an Undo each. Returns (Undo all,
    /// Undo one).
    fn ws_tray(&self, ui: &mut egui::Ui, sec: &Sec, added: &HashSet<String>, lang: &Language) -> (bool, Option<String>) {
        let ws = theme::ws(ui);
        let mut all = false;
        let mut one = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(icons::icon(icons::CLOCK_COUNTER_CLOCKWISE, 13.0, ws.stun));
            ui.label(RichText::new(lang.tr("Added since you opened this page")).font(widgets::bold(12.0)).color(ws.text));
            widgets::count_pill(ui, &added.len().to_string(), ws.ground, ws.stun);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if added.len() == 2 { lang.tr("Undo both") } else { lang.tr("Undo all") };
                let online = self.doc.is_online();
                let r = ui.add_enabled_ui(!online, |ui| widgets::button(ui, Some(icons::ARROW_COUNTER_CLOCKWISE), &label, Look::Ghost, 22.0)).inner;
                if r.on_disabled_hover_text(lang.tr(crate::doc::ONLINE_UNDO)).clicked() {
                    all = true;
                }
            });
        });
        let Some(v) = &self.ws_gear.visit else { return (all, one) };
        let last = self.ws_last_added().map(str::to_owned);
        for a in v.adds.iter().rev().filter(|a| added.contains(&a.guid)).take(3) {
            let Some(el) = edit::find(&self.doc, &a.guid) else { continue };
            let mut name = super::display_name(sec, el, lang);
            if let Some(r) = el.get_i32("rating").filter(|r| *r > 0) {
                name = format!("{name} {r}");
            }
            let g = el.get("grade");
            if !g.is_empty() && g != "Standard" && g != "None" {
                name = format!("{name} · {g}");
            }
            if let Some(p) = edit::parent(&self.doc, &a.guid) {
                name = format!("{name} → {}", p.get("name"));
            }
            let ess = edit::essence(&self.doc, &self.store, &self.rules, &a.guid).filter(|e| *e > 0.0);
            let cost = edit::total_cost(&self.doc, &self.store, &a.guid);
            let latest = last.as_deref() == Some(a.guid.as_str());
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(icons::icon(icons::PLUS, 12.0, ws.stun));
                let w = (ui.available_width() - 70.0 - 70.0 - 50.0 - 24.0 - 32.0).max(60.0);
                let cell = |ui: &mut egui::Ui, w: f32, right: bool, text: RichText| {
                    let layout = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
                    ui.allocate_ui_with_layout(egui::vec2(w, 20.0), layout, |ui| {
                        ui.set_width(w);
                        ui.add(egui::Label::new(text).truncate());
                    });
                };
                cell(ui, w, false, RichText::new(name).size(12.5).color(ws.text));
                cell(ui, 70.0, true, widgets::mono(ess.map_or(String::new(), |e| format!("−{} {}", format::essence(e, self.rules.essence_decimals), lang.tr("Ess"))), 11.5, ws.muted));
                cell(ui, 70.0, true, widgets::mono(format::nuyen(cost), 11.5, ws.text));
                cell(ui, 50.0, false, RichText::new(if latest { lang.tr("just now") } else { String::new() }).size(11.0).color(ws.muted));
                let tip = if self.doc.is_online() {
                    lang.tr(crate::doc::ONLINE_UNDO)
                } else if self.ws_added_is_latest(&a.guid) {
                    lang.tr("Undo this purchase (Ctrl+Z)")
                } else {
                    lang.tr("Remove it (other changes came after it, so this is not an undo)")
                };
                let r = ui.add_enabled_ui(!self.doc.is_online(), |ui| widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, 22.0)).inner;
                let r = r.on_hover_text(&tip).on_disabled_hover_text(&tip);
                if r.clicked() {
                    one = Some(a.guid.clone());
                }
            });
        }
        (all, one)
    }

    /// Record an item bought on the page (for the marks, the inline Undo
    /// and the tray).
    pub(super) fn ws_record_add(&mut self, guid: String) {
        let log_len = self.doc.session().map_or(0, |s| s.log().len());
        if let Some(v) = &mut self.ws_gear.visit {
            v.adds.push(Added { guid: guid.clone(), log_len });
        }
        self.ws_gear.scroll_to = Some(guid);
    }
}

/// Every item guid in a section's container (all depths).
pub(super) fn section_guids(ch: &chummer_core::character::Character, sec: &Sec) -> HashSet<String> {
    let tree = if sec.container == "drugs" {
        return ch.items("drugs", "drug").iter().map(|d| d.get("guid")).collect();
    } else {
        chummer_core::tree::section_tree(&ch.doc, sec)
    };
    let mut out = HashSet::new();
    fn walk(nodes: &[chummer_core::tree::ItemNode], out: &mut HashSet<String>) {
        for n in nodes {
            if let chummer_core::tree::Entry::Item { el, .. } = &n.value {
                let g = el.get("guid");
                if !g.is_empty() {
                    out.insert(g);
                }
            }
            walk(&n.children, out);
        }
    }
    walk(&tree, &mut out);
    out
}

/// The section of the page a catalog was opened on.
pub(super) fn section_of(page: Page) -> Sec {
    page_section(page).unwrap_or(DRUGS)
}
