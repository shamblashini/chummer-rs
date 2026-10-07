//! The Workspace's item pages: Cyberware & Bioware, the Street Gear
//! sub-tabs (Gear, Clothing & Armor, Weapons, Drugs, Lifestyles) and
//! Vehicles & Drones. Each is an "Add …" toolbar, the page's summary
//! (essence, combat stats, vehicle stats, the lifestyle editor) and the
//! section's tree table (`tree_table`, which draws itself in the
//! Workspace style) in a card. A click on an item opens it in the
//! inspector (`ws_inspector`); "Add …" opens the inline catalog
//! (`ws_catalog`) in the page instead of Classic's selection dialog.
//!
//! A child module of `view` (declared there with `#[path]`) so it can
//! use the view's state; changes go through the same commands as the
//! Classic tab pages.

use std::sync::Arc;

use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section as Sec};
use chummer_core::sources::{SourceRef, SourcebookLibrary};
use chummer_core::tree::Entry;
use eframe::egui::{self, RichText};

use super::{display_name, kind_noun, tree_row, CharacterView, Tab, STREET_GEAR};
use crate::pdf_ui::{self, Status};
use crate::theme;
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// The Workspace's item-page state, kept in the view.
#[derive(Default)]
pub struct GearState {
    /// The inline catalog, while one is open.
    pub(super) catalog: Option<super::ws_catalog::Catalog>,
    /// The Workspace item inspector's own state (sell percentage,
    /// location being typed, ammunition choices).
    pub(super) editor: super::ws_inspector::WsItemEditor,
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

    /// The Workspace page for an item tab, `None` for other tabs. Returns
    /// whether the character changed.
    pub(crate) fn ws_items_page(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<bool> {
        let page = self.ws_item_page(tab)?;
        if self.ws_gear.catalog.as_ref().is_some_and(|c| c.page == page) {
            return Some(self.ws_catalog_page(ui, engine, lang, pdfs, status));
        }
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt(("ws_items", tab as u8, page.1)).auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            match page_section(page) {
                Some(sec) => changed |= self.ws_item_list(ui, page, sec, engine, lang, pdfs, status),
                None => changed |= self.ws_drugs(ui, lang),
            }
        });
        Some(changed)
    }

    /// Toolbar, summary and tree of one item page.
    #[allow(clippy::too_many_arguments)]
    fn ws_item_list(&mut self, ui: &mut egui::Ui, page: Page, sec: Sec, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        // Toolbar: the "Add …" buttons and a line about the page.
        let mut open = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, tag) in page_kinds(sec.container).iter().enumerate() {
                let label = chummer_core::items::kind(tag).map_or(*tag, |k| k.label);
                let look = if i == 0 { Look::Primary } else { Look::Secondary };
                let tip = match super::select::parent_of(tag) {
                    Some(_) => lang.tr("Choose where to install it"),
                    None => String::new(),
                };
                let r = widgets::button(ui, Some(icons::PLUS), &lang.tr_fmt("Add {0}", &[&kind_noun(lang, label)]), look, 26.0);
                let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
                if r.clicked() {
                    open = Some(*tag);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(self.ws_page_note(sec, lang)).size(11.5).color(ws.muted));
            });
        });
        if let Some(tag) = open {
            self.ws_open_catalog(page, tag, None);
        }
        match sec.container {
            "cyberwares" => self.ws_essence_card(ui, lang),
            "weapons" => self.ws_weapon_card(ui, lang),
            "vehicles" => self.ws_vehicle_card(ui, lang),
            "lifestyles" => {
                let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
                if !self.doc.items("lifestyles", "lifestyle").is_empty() {
                    widgets::card_frame(&ws).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        changed |= self.lifestyle_editor.ui(ui, &mut self.doc, &cx, status);
                    });
                }
            }
            _ => {}
        }
        changed |= self.ws_section(ui, &sec, lang, pdfs, status);
        changed
    }

    /// "3 items · Essence 5.45" and the like, right of the toolbar.
    fn ws_page_note(&self, sec: Sec, lang: &Language) -> String {
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
        widgets::card_frame(&ws).show(ui, |ui| {
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

    /// Weapons: final stats (damage with STR, AP, accuracy, pool, ranges).
    fn ws_weapon_card(&self, ui: &mut egui::Ui, lang: &Language) {
        let weapons = self.doc.items("weapons", "weapon");
        if weapons.is_empty() {
            return;
        }
        let rows: Vec<Vec<String>> = weapons
            .iter()
            .map(|w| {
                let st = self.weapon_stats(w, true);
                let r = &st.ranges;
                let bands: Vec<&str> = [&r.short, &r.medium, &r.long, &r.extreme].into_iter().map(String::as_str).filter(|b| !b.is_empty()).collect();
                vec![w.get("name"), st.dice_pool.to_string(), st.damage.clone(), st.ap.clone(), st.accuracy.to_string(), st.rc.clone(), if st.reach != 0 { st.reach.to_string() } else { String::new() }, bands.join(" / ")]
            })
            .collect();
        stats_card(ui, "ws_weapon_stats", &lang.tr("Combat stats"), &lang.tr_all(["Weapon", "Pool", "Damage", "AP", "Acc", "RC", "Reach", "Ranges"]), &rows, Some(1));
    }

    /// Vehicles: totals after mods.
    fn ws_vehicle_card(&self, ui: &mut egui::Ui, lang: &Language) {
        let vehicles = self.doc.items("vehicles", "vehicle");
        if vehicles.is_empty() {
            return;
        }
        let rows: Vec<Vec<String>> = vehicles
            .iter()
            .map(|v| {
                let st = chummer_core::items::vehicle::stats(v);
                let slots = if st.is_drone { format!("{}/{}", st.drone_mod_slots_used, st.drone_mod_slots) } else { format!("{}/{}", st.slots_used, st.slots) };
                vec![v.get("name"), st.handling_text.clone(), st.speed_text.clone(), st.accel_text.clone(), st.body.to_string(), st.armor.to_string(), st.pilot.to_string(), st.sensor.to_string(), st.seats.to_string(), slots]
            })
            .collect();
        stats_card(ui, "ws_vehicle_stats", &lang.tr("Vehicle stats"), &lang.tr_all(["Vehicle", "Handling", "Speed", "Accel", "Body", "Armor", "Pilot", "Sensor", "Seats", "Slots"]), &rows, None);
    }

    /// A section's items as a tree table in a card: groups, locations,
    /// nesting, issue marks, the source and remove buttons.
    fn ws_section(&mut self, ui: &mut egui::Ui, sec: &Sec, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let count = self.doc.doc.child(sec.container).map_or(0, |c| c.children_named(sec.item).count());
        let mut remove = None;
        let mut clicked = None;
        widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(widgets::title(&lang.tr(sec.label), &ws));
                widgets::count_pill(ui, &count.to_string(), ws.selection, ws.accent);
            });
            if count == 0 {
                ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
                return;
            }
            let tree = chummer_core::tree::section_tree(&self.doc.doc, sec);
            let headers: Vec<String> = sec.columns.iter().map(|c| lang.tr(c.header)).collect();
            let selected = self.item_editor.as_ref().map(|(g, _)| g.as_str());
            let marks = self.item_marks(lang);
            let remove_tip = lang.tr("Remove (also removes its improvements)");
            let out = crate::tree_table::TreeTable::new(("ws", sec.container), &headers).selected(selected).show(ui, &tree, |n| tree_row(sec, n, lang, &marks), |ui, n| {
                let Entry::Item { el, top } = n.value else { return };
                if let Some(r) = SourceRef::of(el) {
                    let tip = if pdfs.is_linked(&r.book) { format!("{r}") } else { format!("{r} · {}", lang.tr("No PDF linked for this book — Tools → Sourcebooks")) };
                    if widgets::icon_button(ui, icons::BOOK_OPEN, 22.0).on_hover_text(tip).clicked() {
                        pdf_ui::open(pdfs, &r, status);
                    }
                }
                if top && widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(&remove_tip).clicked() {
                    remove = Some((sec.container.to_owned(), el.get("guid"), display_name(sec, el, lang)));
                }
            });
            clicked = out.clicked;
        });
        if remove.is_some() {
            self.confirm_remove = remove;
        }
        if let Some(g) = clicked {
            self.ws_gear.catalog = None;
            self.item_editor = Some((g, crate::item_editor::ItemEditor::default()));
        }
        false
    }

    /// Drugs: the custom drug builder and the drugs the character has.
    fn ws_drugs(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        ui.horizontal(|ui| {
            if widgets::button(ui, Some(icons::FLASK), &lang.tr("Build custom drug…"), Look::Primary, 26.0).clicked() {
                self.drug_builder.open = true;
            }
        });
        let drugs: Vec<chummer_core::xml::Element> = self.doc.items("drugs", "drug").into_iter().cloned().collect();
        widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(widgets::title(&lang.tr("Your drugs"), &ws));
                widgets::count_pill(ui, &drugs.len().to_string(), ws.selection, ws.accent);
            });
            if drugs.is_empty() {
                ui.label(RichText::new(lang.tr("None.")).size(12.0).color(ws.muted));
                return;
            }
            egui::Grid::new("ws_drugs").num_columns(4).spacing([18.0, 4.0]).min_row_height(24.0).show(ui, |ui| {
                for h in lang.tr_all(["Name", "Grade", "Quantity"]) {
                    ui.label(RichText::new(h).size(10.5).color(ws.muted));
                }
                ui.label("");
                ui.end_row();
                for d in &drugs {
                    ui.label(RichText::new(d.get("name")).size(12.5).color(ws.text));
                    ui.label(RichText::new(d.get("grade")).size(12.0).color(ws.muted));
                    ui.label(widgets::mono(format!("×{}", d.get("quantity")), 12.0, ws.text));
                    if widgets::icon_button(ui, icons::TRASH, 22.0).on_hover_text(lang.tr("Remove (no refund)")).clicked() {
                        changed |= self.doc.set(Command::RemoveItem { container: "drugs".into(), guid: d.get("guid") });
                    }
                    ui.end_row();
                }
            });
        });
        changed
    }
}

/// A card with a small table: small muted headers, the name column in
/// the text face, the values monospace, column `accent` (the dice pool)
/// in the accent colour.
fn stats_card(ui: &mut egui::Ui, id: &str, title: &str, headers: &[String], rows: &[Vec<String>], accent: Option<usize>) {
    let ws = theme::ws(ui);
    widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(widgets::title(title, &ws));
        egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
            egui::Grid::new(id).num_columns(headers.len()).spacing([16.0, 2.0]).min_row_height(22.0).show(ui, |ui| {
                for h in headers {
                    ui.label(RichText::new(h).size(10.5).color(ws.muted));
                }
                ui.end_row();
                for row in rows {
                    for (i, c) in row.iter().enumerate() {
                        if i == 0 {
                            ui.label(RichText::new(c).size(12.5).color(ws.text));
                        } else if accent == Some(i) {
                            ui.label(widgets::mono(c, 12.5, ws.accent));
                        } else {
                            ui.label(widgets::mono(c, 12.0, ws.text));
                        }
                    }
                    ui.end_row();
                }
            });
        });
    });
}
