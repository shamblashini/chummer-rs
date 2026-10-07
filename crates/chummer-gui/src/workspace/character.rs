//! The Workspace's access to a character view (`crate::view`): the
//! sidebar model with issue badges, the pages of its sections, the
//! inspector sections, the budget strip and the Play screen.
//!
//! A child module of `view` (declared there with `#[path]`) so it can
//! use the view's state; it only draws, through the same commands and
//! helpers as the Classic tabs.

use std::sync::Arc;

use chummer_core::chargen::issues::{Issue, Severity};
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section as Sec};
use chummer_core::sources::SourcebookLibrary;
use chummer_core::{career, format};
use eframe::egui::{self, RichText};

use super::issues_ui::{gear_sub_tab, tab_of};
use super::{CharacterView, Tab, STREET_GEAR, TABS};
use crate::pdf_ui::Status;
use crate::theme::{self, Badge};
use crate::workspace::widgets::{self, CmClick, Look, Tone, Track};
use crate::workspace::popout::{Panel, PopKey, PopOuts};
use crate::workspace::{icons, DocKey, NavGroup, NavItem, PanelId, Section};

// The rebuilt pages and their inspector sections (children of `view`, so
// they can use the view's state).
pub(super) mod build;

/// The label of Street Gear sub-tab `i`.
pub fn gear_label(i: usize) -> &'static str {
    STREET_GEAR.get(i).map_or("", |(l, _)| l)
}

/// Item lists and the section that shows them, for the palette.
const ITEM_SECTIONS: &[(Sec, Section)] = &[
    (sections::QUALITIES, Section::Page(Tab::Common)),
    (sections::CYBERWARE, Section::Page(Tab::Cyberware)),
    (sections::GEAR, Section::Gear(0)),
    (sections::ARMOR, Section::Gear(1)),
    (sections::WEAPONS, Section::Gear(2)),
    (sections::LIFESTYLES, Section::Gear(4)),
    (sections::VEHICLES, Section::Page(Tab::Vehicles)),
    (sections::SPELLS, Section::Page(Tab::Magician)),
    (sections::SPIRITS, Section::Page(Tab::Magician)),
    (sections::POWERS, Section::Page(Tab::Adept)),
    (sections::COMPLEX_FORMS, Section::Page(Tab::Technomancer)),
    (sections::AI_PROGRAMS, Section::Page(Tab::AdvancedPrograms)),
    (sections::CRITTER_POWERS, Section::Page(Tab::Critter)),
    (sections::METAMAGICS, Section::Page(Tab::Initiation)),
    (sections::MARTIAL_ARTS, Section::Page(Tab::MartialArts)),
    (sections::CONTACTS, Section::Page(Tab::Relationships)),
];

/// An item of the character for the palette.
pub struct ItemRef {
    pub name: String,
    /// The list it is in ("Cyberware", "Weapons"), translated.
    pub list: String,
    pub section: Section,
    pub guid: String,
}

/// One value of the budget strip.
pub struct Chip {
    pub label: String,
    pub value: String,
    pub fill: Option<f32>,
    pub tone: Tone,
}

/// The sidebar header: initials, name, a line about the character, and
/// the mode line.
pub struct Header {
    pub initials: String,
    pub name: String,
    pub about: String,
    pub mode: String,
}

/// Badge for issues that count (errors and warnings).
fn badge<'a>(issues: impl Iterator<Item = &'a Issue>) -> Option<Badge> {
    let (mut count, mut error) = (0, false);
    for i in issues.filter(|i| i.severity != Severity::Info) {
        count += 1;
        error |= i.is_error();
    }
    (count > 0).then_some(Badge { count, error })
}

fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.chars().count() < 2 {
        out = name.chars().filter(|c| c.is_alphanumeric()).take(2).collect();
    }
    out.to_uppercase()
}

impl CharacterView {
    /// Stays the same while the tab is open (`workspace::DocKey`).
    pub fn ws_id(&self) -> u64 {
        self.ws_id
    }

    /// Creation mode (budgets and issues).
    pub fn ws_creating(&self) -> bool {
        self.budget.is_some()
    }

    /// Start of a frame (see `CharacterView::ui`); returns true if the
    /// character changed.
    pub fn ws_begin(&mut self) -> bool {
        self.begin_frame()
    }

    /// End of a frame: dialogs, purchases, recomputing.
    pub fn ws_end(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, changed: bool) {
        self.end_frame(ctx, engine, lang, pdfs, status, changed);
    }

    /// The section the view is on, given the Workspace's own section
    /// (Play, History) and the tab it was picked on.
    pub fn ws_current(&self, special: Option<(Section, Tab)>) -> Section {
        match special {
            Some((s, t)) if t == self.tab => s,
            _ if self.tab == Tab::StreetGear => Section::Gear(self.gear_tab),
            _ => Section::Page(self.tab),
        }
    }

    /// Go to a section's tab (Play and History keep the tab).
    pub fn ws_go(&mut self, s: Section) {
        match s {
            Section::Page(t) => self.tab = t,
            Section::Gear(i) => {
                self.tab = Tab::StreetGear;
                self.gear_tab = i;
            }
            _ => {}
        }
    }

    /// The tab the view is on.
    pub fn ws_tab(&self) -> Tab {
        self.tab
    }

    /// The Street Gear sub-tab.
    pub fn ws_gear_tab(&self) -> usize {
        self.gear_tab
    }

    pub fn ws_set_gear_tab(&mut self, i: usize) {
        self.gear_tab = i;
    }

    fn tab_badge(&self, t: Tab) -> Option<Badge> {
        badge(self.issues.iter().filter(|i| i.tab().map(tab_of) == Some(t)))
    }

    /// The sidebar: Session (career), Build or Character, Story, Records.
    pub fn ws_nav(&self, lang: &Language) -> Vec<NavGroup> {
        let created = self.doc.created;
        let item = |s: Section, badge: Option<Badge>| NavItem { section: s, label: lang.tr(s.label()), badge };
        let page = |t: Tab| item(Section::Page(t), self.tab_badge(t));
        let mut groups = Vec::new();
        if created {
            groups.push(NavGroup { title: lang.tr("Session"), items: vec![item(Section::Play, None)] });
        }
        let mut build = Vec::new();
        for (t, _) in TABS {
            match t {
                Tab::CharacterInfo | Tab::Karma | Tab::Calendar | Tab::Notes | Tab::Improvements => continue,
                Tab::StreetGear => {
                    for i in 0..STREET_GEAR.len() {
                        let b = badge(self.issues.iter().filter(|x| x.tab().map(tab_of) == Some(Tab::StreetGear) && gear_sub_tab(x.area).unwrap_or(0) == i));
                        build.push(item(Section::Gear(i), b));
                    }
                }
                t if self.visible(*t) => build.push(page(*t)),
                _ => {}
            }
        }
        groups.push(NavGroup { title: lang.tr(if created { "Character" } else { "Build" }), items: build });
        let story: Vec<NavItem> = [Tab::CharacterInfo, Tab::Notes, Tab::Calendar].into_iter().filter(|t| self.visible(*t)).map(page).collect();
        groups.push(NavGroup { title: lang.tr("Story"), items: story });
        let mut records: Vec<NavItem> = [Tab::Karma, Tab::Improvements].into_iter().filter(|t| self.visible(*t)).map(page).collect();
        records.push(item(Section::History, None));
        groups.push(NavGroup { title: lang.tr("Records"), items: records });
        groups
    }

    /// The sidebar header.
    pub fn ws_header(&self, lang: &Language) -> Header {
        let name = self.doc.display_name();
        let metatype = self.doc.field("metatype");
        let about = [metatype, self.doc.field("concept")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let mode = if self.doc.created {
            format!("{} · {}", lang.tr("Career"), lang.tr_fmt("{0} karma", &[&career::career_karma(&self.doc)]))
        } else {
            let method = self.settings.as_ref().map(|s| s.build_method()).unwrap_or_else(|| self.doc.field("buildmethod"));
            format!("{} · {}", lang.tr("Creation"), method)
        };
        Header { initials: initials(&name), name, about, mode }
    }

    /// The settings preset's name, for the status bar.
    pub fn ws_settings_name(&self) -> String {
        self.settings.as_ref().map(|s| s.name()).unwrap_or_default()
    }

    /// Error and warning counts of the creation issues.
    pub fn ws_issue_counts(&self) -> (usize, usize) {
        let errors = self.issues.iter().filter(|i| i.is_error()).count();
        let warnings = self.issues.iter().filter(|i| i.severity == Severity::Warning).count();
        (errors, warnings)
    }

    /// The budget strip: creation budgets, or career resources.
    pub fn ws_budgets(&self, lang: &Language) -> Vec<Chip> {
        let s = &self.sheet;
        let essence = format::essence(s.essence, self.rules.essence_decimals);
        let ess_fill = Some((s.essence as f32 / 6.0).clamp(0.0, 1.0));
        let Some(b) = &self.budget else {
            return vec![
                Chip { label: lang.tr("Karma"), value: format!("{}", self.doc.karma), fill: None, tone: Tone::Normal },
                Chip { label: lang.tr("Nuyen"), value: format::nuyen(self.doc.nuyen), fill: None, tone: Tone::Normal },
                Chip { label: lang.tr("Essence"), value: essence, fill: None, tone: Tone::Normal },
                Chip { label: lang.tr("Limits"), value: format!("{} / {} / {}", s.limit_physical, s.limit_mental, s.limit_social), fill: None, tone: Tone::Normal },
                Chip { label: lang.tr("Initiative"), value: format!("{} + {}d6", s.initiative, s.initiative_dice), fill: None, tone: Tone::Normal },
                Chip { label: lang.tr("Armor"), value: s.armor.to_string(), fill: None, tone: Tone::Normal },
            ]
            .into_iter()
            .chain(chummer_core::calendar::weeks(&self.doc).into_iter().max_by_key(|w| (w.year, w.week)).map(|w| Chip { label: lang.tr("Calendar"), value: w.label(), fill: None, tone: Tone::Normal }))
            .collect();
        };
        let points = |label: &str, (total, used): (i32, i32)| {
            let tone = if used > total {
                Tone::Error
            } else if used < total {
                Tone::Warning
            } else {
                Tone::Normal
            };
            let fill = if total > 0 { used as f32 / total as f32 } else { 1.0 };
            Chip { label: lang.tr(label), value: format!("{used} / {total}"), fill: Some(fill), tone }
        };
        let mut out = vec![points("Attributes", b.attribute_points), points("Special", b.special_points), points("Skills", b.skill_points)];
        if b.skill_group_points.0 > 0 {
            out.push(points("Skill Groups", b.skill_group_points));
        }
        out.push(points("Knowledge", b.knowledge_points));
        out.push(points("Contacts", b.contact_points));
        let karma_left = b.karma_left();
        let fill = |used: f64, total: f64| Some(if total > 0.0 { (used / total) as f32 } else { 1.0 });
        out.push(Chip {
            label: lang.tr("Karma"),
            value: lang.tr_fmt("{0} left", &[&karma_left]),
            fill: fill(b.karma.1 as f64, b.karma.0 as f64),
            tone: if karma_left < 0 { Tone::Error } else { Tone::Normal },
        });
        let nuyen_left = b.nuyen_left();
        out.push(Chip { label: lang.tr("Nuyen"), value: format::nuyen(nuyen_left), fill: fill(b.nuyen.1, b.nuyen.0), tone: if nuyen_left < 0.0 { Tone::Error } else { Tone::Normal } });
        out.push(Chip { label: lang.tr("Essence"), value: essence, fill: ess_fill, tone: Tone::Normal });
        out
    }

    /// A section's page: the guide and the tab's issues above a Classic
    /// tab page, or the Workspace's own Play and History screens. Returns
    /// true if the character changed.
    #[allow(clippy::too_many_arguments)]
    pub fn ws_page(&mut self, ui: &mut egui::Ui, section: Section, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>, pops: &mut PopOuts) -> bool {
        let mut changed = false;
        match section {
            Section::Play => {
                let key = PopKey::new(DocKey::Character(self.ws_id), PanelId::Condition);
                let ws = theme::ws(ui);
                let hint = lang.tr("Click a box to mark it; click again to clear.");
                Panel::card(key, &lang.tr("Condition Monitor")).show(ui, pops, lang, |ui| {
                    ui.label(RichText::new(hint).size(11.5).color(ws.muted));
                }, |ui| changed |= self.ws_condition(ui, lang, roll));
            }
            Section::History => {
                let ws = theme::ws(ui);
                widgets::card_frame(&ws).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().id_salt("ws_history").auto_shrink([false, true]).show(ui, |ui| changed |= crate::history_ui::panel(ui, &mut self.doc, lang));
                });
            }
            Section::Page(_) | Section::Gear(_) => {
                let tab = match section {
                    Section::Gear(i) => {
                        self.gear_tab = i;
                        Tab::StreetGear
                    }
                    Section::Page(t) => t,
                    _ => unreachable!(),
                };
                if let Some(key) = crate::ruleset_ui::banner(ui, &self.doc, engine, lang, tab == Tab::Common) {
                    changed |= self.switch_settings(&key, status);
                }
                self.ws_guide(ui, lang, pdfs, status);
                self.ws_issue_strip(ui, lang, tab);
                changed |= self.ws_tab_page(ui, tab, engine, lang, pdfs, status, roll);
            }
            _ => {}
        }
        changed
    }

    /// The Play screen's condition card: the condition monitor, Edge,
    /// and the numbers needed at the table. Returns true if the character
    /// changed.
    pub fn ws_condition(&mut self, ui: &mut egui::Ui, lang: &Language, roll: &mut Option<u32>) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let s = self.sheet.clone();
        {
            let (plabel, slabel) = crate::ai_ui::cm_labels(&self.doc, lang);
            let physical = Track { label: plabel, color: ws.physical, boxes: s.physical_cm, filled: chummer_core::play::ai::physical_filled(&self.doc), threshold: s.cm_threshold };
            let stun = Track { label: slabel, color: ws.stun, boxes: s.stun_cm, filled: chummer_core::play::ai::stun_filled(&self.doc), threshold: if self.doc.is_ai() { 0 } else { s.cm_threshold } };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                match widgets::condition_monitor(ui, "ws_cm", &physical, &stun, s.cm_overflow, &lang.tr("Overflow")) {
                    Some(CmClick::Physical(n)) => changed |= self.doc.set(Command::SetPhysicalDamage { filled: n }),
                    Some(CmClick::Stun(n)) => changed |= self.doc.set(Command::SetStunDamage { filled: n }),
                    None => {}
                }
                let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 120.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0.0, ws.divider);
                ui.vertical(|ui| {
                    ui.set_max_width(260.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    if s.wound_modifier != 0 {
                        ui.horizontal(|ui| {
                            widgets::tag(ui, &format!("{} {}", lang.tr("CM Penalty:"), s.wound_modifier), ws.warning, ws.warning);
                        });
                    }
                    widgets::stat_row(ui, &lang.tr("Armor"), &s.armor.to_string(), false);
                    widgets::stat_row(ui, &lang.tr("Initiative"), &format!("{} + {}d6", s.initiative, s.initiative_dice), true);
                    widgets::stat_row(ui, &lang.tr("Physical limit"), &s.limit_physical.to_string(), false);
                    widgets::stat_row(ui, &lang.tr("Mental limit"), &s.limit_mental.to_string(), false);
                    widgets::stat_row(ui, &lang.tr("Social limit"), &s.limit_social.to_string(), false);
                    if widgets::button(ui, Some(icons::DICE_FIVE), &lang.tr("Open Dice Roller"), Look::Outline, 24.0).clicked() {
                        *roll = Some(6);
                    }
                });
            });
            if self.doc.created {
                ui.add_space(8.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0.0, ws.divider);
                ui.add_space(8.0);
                let total = s.attr("EDG").max(0);
                let used = self.doc.doc.get_i32("edgeused").unwrap_or(0).clamp(0, total);
                let available = total - used;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(lang.tr("Edge")).font(widgets::bold(12.5)).color(ws.text));
                    let tip = |n: i32, on: bool| format!("{} {n} / {total}: {}", lang.tr("Edge"), if on { lang.tr("available") } else { lang.tr("spent") });
                    if let Some(a) = widgets::edge_boxes(ui, "ws_edge", total, available, tip) {
                        changed |= self.doc.set(Command::SetEdgeUsed { used: total - a });
                    }
                    ui.label(widgets::mono(format!("{available}/{total}"), 11.5, ws.muted));
                    if widgets::icon_button(ui, icons::ARROWS_CLOCKWISE, 24.0).on_hover_text(lang.tr("Reset")).clicked() {
                        changed |= self.doc.set(Command::RefreshEdge);
                    }
                });
            }
        }
        changed
    }

    /// Inspector: every creation issue, and Finish creation.
    pub fn ws_issues(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        // Issues wrap, so long ones do not widen the inspector.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        if self.issues.iter().any(Issue::is_error) {
            ui.label(RichText::new(lang.tr("Fix the problems above first")).size(11.5).color(ws.muted));
        }
        self.issue_list(ui, lang);
        ui.add_space(6.0);
        let ok = !self.issues.iter().any(Issue::is_error);
        let r = ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::CHECK), &lang.tr("Finish creation"), Look::Primary, 26.0)).inner;
        if r.on_disabled_hover_text(lang.tr("Fix the problems above first")).clicked() {
            self.confirm_finish = true;
        }
    }

    /// Whether an item is open in the item pane.
    pub fn ws_has_item(&self) -> bool {
        self.item_editor.is_some()
    }

    pub fn ws_close_item(&mut self) {
        self.item_editor = None;
    }

    /// Inspector: the selected item's editor.
    pub fn ws_item(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, status: &mut Status) -> bool {
        self.item_pane(ui, engine, lang, status, false)
    }

    /// Inspector: the Karma Summary in creation, Other Info and Spell
    /// Defense in career.
    pub fn ws_summary(&mut self, ui: &mut egui::Ui, lang: &Language, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        match self.budget.clone() {
            Some(b) => {
                let row = |ui: &mut egui::Ui, label: &str, (total, used): (i32, i32)| {
                    let left = total - used;
                    widgets::stat_row(ui, &lang.tr(label), &format!("{left} / {total}"), left > 0);
                };
                row(ui, "Karma", b.karma);
                row(ui, "Attribute Points", b.attribute_points);
                row(ui, "Special points", b.special_points);
                row(ui, "Skill Points", b.skill_points);
                row(ui, "Skill Group Points", b.skill_group_points);
                row(ui, "Knowledge Points", b.knowledge_points);
                row(ui, "Contact Points", b.contact_points);
                if b.free_spells.0 > 0 {
                    row(ui, "Free Spells", b.free_spells);
                }
                widgets::stat_row(ui, &lang.tr("Positive Qualities"), &format!("{} / {}", b.positive_quality_karma, b.quality_limit), false);
                widgets::stat_row(ui, &lang.tr("Negative Qualities"), &format!("{} / {}", b.negative_quality_karma, b.quality_limit), false);
                widgets::stat_row(ui, &lang.tr("Nuyen left"), &format::nuyen(b.nuyen_left()), false);
            }
            None => {
                changed |= self.other_info(ui, lang, roll);
                ui.add_space(4.0);
                egui::CollapsingHeader::new(lang.s("String_SpellDefense")).id_salt("ws_spell_defense").show(ui, |ui| {
                    changed |= self.spell_defense(ui, lang);
                });
            }
        }
        changed
    }

    /// Inspector: this session's changes (the inspector scrolls; a
    /// nested scroll area would widen the side panel every frame).
    pub fn ws_history(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        // Long entries wrap instead of widening the inspector.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        crate::history_ui::panel(ui, &mut self.doc, lang)
    }

    /// The character's items, for the palette.
    pub fn ws_items(&self, lang: &Language) -> Vec<ItemRef> {
        let mut out = Vec::new();
        for (sec, section) in ITEM_SECTIONS {
            for el in self.doc.items(sec.container, sec.item) {
                let guid = el.get("guid");
                if guid.is_empty() {
                    continue;
                }
                out.push(ItemRef { name: super::display_name(sec, el, lang), list: lang.tr(sec.label), section: *section, guid });
            }
        }
        out
    }

    /// Go to an item: its section, and its editor in the inspector (a
    /// contact opens in the Relationships tab).
    pub fn ws_show_item(&mut self, section: Section, guid: &str) {
        self.ws_go(section);
        if section == Section::Page(Tab::Relationships) {
            self.relationships.show_contact(guid);
        } else if chummer_core::items::edit::find(&self.doc, guid).is_some_and(chummer_core::items::edit::is_item) {
            self.item_editor = Some((guid.to_owned(), crate::item_editor::ItemEditor::default()));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn initials() {
        assert_eq!(super::initials("Apex"), "AP");
        assert_eq!(super::initials("Davis Jones"), "DJ");
        assert_eq!(super::initials("  "), "");
    }
}
