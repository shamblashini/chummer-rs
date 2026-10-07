//! The magic, resonance, critter, A.I. and martial arts pages: summary
//! cards (tradition and drain, power points, stream and fading,
//! initiation), the editors of `magic_ui` in cards, and each section's
//! items as a Workspace table.

use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section as Sec};
use chummer_core::sources::SourcebookLibrary;
use chummer_core::{career, data};
use eframe::egui::{self, RichText};

use super::k;
use crate::pdf_ui::Status;
use crate::theme;
use crate::view::{CareerAction, CharacterView, Tab};
use crate::workspace::icons;
use crate::workspace::widgets::{self, Look};

/// A line of a summary card: label, value, accent.
type Line = (String, String, bool);

impl CharacterView {
    pub(super) fn ws_magic(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        changed |= self.ws_magic_summary(ui, tab, engine, lang, status);
        if matches!(tab, Tab::Magician | Tab::Adept) {
            changed |= self.ws_editor_card(ui, engine, lang, pdfs, status, None);
        }
        let secs: Vec<Sec> = match tab {
            Tab::MartialArts => vec![sections::MARTIAL_ARTS],
            // Spirits sit with spells; a technomancer's sprites with complex forms.
            Tab::Magician => vec![sections::SPELLS, sections::SPIRITS],
            Tab::Adept => vec![sections::POWERS],
            Tab::Technomancer if self.visible(Tab::Magician) => vec![sections::COMPLEX_FORMS],
            Tab::Technomancer => vec![sections::COMPLEX_FORMS, sections::SPIRITS],
            Tab::Critter => vec![sections::CRITTER_POWERS],
            Tab::Initiation => vec![sections::METAMAGICS],
            Tab::AdvancedPrograms => vec![sections::AI_PROGRAMS],
            _ => Vec::new(),
        };
        for sec in secs {
            ui.add_space(4.0);
            let count = self.doc.items(sec.container, sec.item).len();
            let mut design = false;
            widgets::heading(ui, &lang.tr(sec.label), &count.to_string(), 13.0, |ui| {
                // Right to left: the last button drawn is the first.
                if sec.container == "spells" {
                    design = widgets::button(ui, Some(icons::SPARKLE), &lang.tr("Create Spell…"), Look::Ghost, 24.0).clicked();
                }
                self.ws_add_buttons(ui, engine, lang, sec.container);
            });
            if design {
                self.spell_designer.open = true;
            }
            changed |= self.ws_editor_card(ui, engine, lang, pdfs, status, Some(sec.container));
            // `ai_ui` lists the programs with their requirements itself.
            if sec.container != "aiprograms" {
                changed |= self.ws_item_table(ui, &sec, lang, pdfs, status);
            }
        }
        changed
    }

    /// The `magic_ui` editor for a section (spell picker and quickening,
    /// power points, spirit services, metamagic grades, techniques), or
    /// with `None` the mentor and foci, in a card when it draws anything.
    fn ws_editor_card(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, container: Option<&str>) -> bool {
        if container.is_some_and(|c| !matches!(c, "spells" | "powers" | "spirits" | "metamagics" | "martialarts" | "aiprograms")) {
            return false;
        }
        let ws = theme::ws(ui);
        let mut changed = false;
        // The card only shows when the editor drew something.
        let mut card = widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(12, 8)).begin(ui);
        {
            let ui = &mut card.content_ui;
            ui.set_min_width(ui.available_width());
            let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
            changed |= match container {
                None => self.magic_editor.shared_ui(ui, &mut self.doc, &cx, status),
                Some("aiprograms") => crate::ai_ui::tab(ui, &mut self.doc, engine, lang, status),
                Some(c) => self.magic_editor.ui(ui, &mut self.doc, &cx, c, status),
            };
        }
        if card.content_ui.min_rect().height() > 1.0 {
            card.end(ui);
        }
        changed
    }

    /// The cards at the top of a magic page.
    fn ws_magic_summary(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Engine, lang: &Language, status: &mut Status) -> bool {
        let mut changed = false;
        let m = chummer_core::items::magic::magic_summary_with(&self.doc, &self.sheet, Some(&self.store));
        let row = |l: String, v: String, accent: bool| (l, v, accent);
        let mut cards: Vec<(String, Vec<Line>)> = Vec::new();
        match tab {
            Tab::Magician => {
                if self.doc.mag_enabled() && self.doc.is_magician() {
                    changed |= self.ws_tradition(ui, lang, status);
                }
                if !m.tradition.is_empty() {
                    cards.push((lang.tr("Drain"), vec![row(m.drain_expression.replace(['{', '}'], ""), lang.tr_fmt("{0} dice", &[&m.drain_pool]), true)]));
                }
                if self.doc.mag_enabled() {
                    cards.push((
                        lang.tr("Astral"),
                        vec![row(lang.tr("Initiative"), format!("{} + {}d6", m.astral_initiative, m.astral_initiative_dice), true), row(lang.tr("Astral limit"), m.astral_limit.to_string(), false)],
                    ));
                }
            }
            Tab::Technomancer if !m.stream.is_empty() => {
                cards.push((lang.tr("Stream"), vec![row(lang.tr("Stream"), m.stream.clone(), false), row(m.fading_expression.replace(['{', '}'], ""), lang.tr_fmt("{0} dice", &[&m.fading_pool]), true)]));
            }
            Tab::Adept => {
                if let Some((total, used)) = m.power_points {
                    let left = total - used;
                    cards.push((lang.tr("Power Points"), vec![row(lang.tr("Used"), format!("{used} / {total}"), false), row(lang.tr("Left"), format!("{left}"), left > 0.0)]));
                }
            }
            Tab::Initiation => {
                changed |= self.ws_initiation(ui, engine, lang);
            }
            _ => {}
        }
        if !cards.is_empty() {
            let gap = 8.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (caption, rows) in &cards {
                    widgets::mini_card(ui, caption, rows, 220.0);
                }
            });
        }
        changed
    }

    /// The tradition picker.
    fn ws_tradition(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let current = self.doc.doc.child("tradition").map(|t| t.get("name")).unwrap_or_default();
        let mut pick: Option<String> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(lang.tr("Tradition")).font(widgets::bold(13.0)).color(ws.text));
            crate::combo::Combo::from_id_salt("ws_tradition").selected_text(if current.is_empty() { lang.tr("Choose…") } else { current.clone() }).width(240.0).show_ui(ui, |ui| {
                if let Ok(doc) = self.store.doc("traditions.xml") {
                    for r in data::records(&doc, "traditions", "tradition") {
                        if crate::combo::selectable_label(ui, current == r.name(), r.name()).clicked() {
                            pick = Some(r.name());
                        }
                    }
                }
            });
        });
        match pick {
            Some(p) => self.doc.run(Command::SetTradition { name: p }, status).is_some(),
            None => false,
        }
    }

    /// Initiate or submersion grade and, in career, the next grade with
    /// its options and karma cost.
    fn ws_initiation(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let techno = self.doc.res_enabled() && !self.doc.mag_enabled();
        let grade = self.doc.doc.get_i32(if techno { "submersiongrade" } else { "initiategrade" }).unwrap_or(0);
        widgets::card_frame(&ws).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new(if techno { lang.tr("Submersion Grade") } else { lang.tr("Initiate Grade") }).font(widgets::bold(13.0)).color(ws.text));
                ui.label(widgets::mono(grade.to_string(), 17.0, ws.accent));
                if self.doc.created && (self.doc.mag_enabled() || self.doc.res_enabled()) {
                    ui.add_space(8.0);
                    widgets::check(ui, &mut self.initiation.group, &lang.tr("Group"));
                    widgets::check(ui, &mut self.initiation.ordeal, &lang.tr("Ordeal"));
                    widgets::check(ui, &mut self.initiation.schooling, &lang.tr("Schooling"));
                    let cost = career::initiation_karma_cost(engine, &self.doc, self.initiation);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = format!("{} · {}", if techno { lang.tr("Submerge") } else { lang.tr("Initiate") }, k(cost));
                        let tip = lang.tr_fmt("Grade {0} for {1} karma", &[&(grade + 1), &cost]);
                        if widgets::cost_button(ui, &label, self.doc.karma >= cost, &tip, &lang.tr("Not enough karma")).clicked() {
                            self.action = Some(CareerAction::Initiate(self.initiation));
                        }
                    });
                }
            });
        });
        false
    }
}
