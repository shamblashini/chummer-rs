//! Creation issues in the character view: badges on the tabs, the hint
//! line at the top of the focused tab (`guide_ui`), marks on rows, and
//! the full list in the Karma Summary (see `chummer_core::chargen::issues`).
//!
//! A child module of `view`, so it can work on the view's state.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use chummer_core::chargen::issues::{Area, Issue, IssueTab, Severity};
use chummer_core::lang::Language;
use eframe::egui::{self, RichText};

use super::{CharacterView, Tab};
use crate::theme::{Badge, TabDeco};

/// The GUI tab for a core tab.
pub(super) fn tab_of(t: IssueTab) -> Tab {
    match t {
        IssueTab::Common => Tab::Common,
        IssueTab::Skills => Tab::Skills,
        IssueTab::MartialArts => Tab::MartialArts,
        IssueTab::Magician => Tab::Magician,
        IssueTab::Adept => Tab::Adept,
        IssueTab::Technomancer => Tab::Technomancer,
        IssueTab::Cyberware => Tab::Cyberware,
        IssueTab::StreetGear => Tab::StreetGear,
        IssueTab::Vehicles => Tab::Vehicles,
        IssueTab::Relationships => Tab::Relationships,
        IssueTab::CharacterInfo => Tab::CharacterInfo,
    }
}

/// The core tab of a GUI tab; `None` for tabs no issue or step is on.
pub(super) fn issue_tab(t: Tab) -> Option<IssueTab> {
    Some(match t {
        Tab::Common => IssueTab::Common,
        Tab::Skills => IssueTab::Skills,
        Tab::MartialArts => IssueTab::MartialArts,
        Tab::Magician => IssueTab::Magician,
        Tab::Adept => IssueTab::Adept,
        Tab::Technomancer => IssueTab::Technomancer,
        Tab::Cyberware => IssueTab::Cyberware,
        Tab::StreetGear => IssueTab::StreetGear,
        Tab::Vehicles => IssueTab::Vehicles,
        Tab::Relationships => IssueTab::Relationships,
        Tab::CharacterInfo => IssueTab::CharacterInfo,
        _ => return None,
    })
}

/// The Street Gear sub-tab (index into `STREET_GEAR`) of an area.
pub(super) fn gear_sub_tab(area: Area) -> Option<usize> {
    match area {
        Area::Gear => Some(0),
        Area::Armor => Some(1),
        Area::Weapons => Some(2),
        Area::Lifestyles => Some(4),
        _ => None,
    }
}

/// The issue's message in the UI language.
pub(super) fn message(lang: &Language, i: &Issue) -> String {
    let args: Vec<&dyn std::fmt::Display> = i.args.iter().map(|a| a as &dyn std::fmt::Display).collect();
    lang.tr_fmt(i.template(), &args)
}

/// Badge for a set of issues: errors and warnings count, infos do not.
fn badge<'a>(issues: impl Iterator<Item = &'a Issue>) -> Option<Badge> {
    let (mut count, mut error) = (0, false);
    for i in issues.filter(|i| i.severity != Severity::Info) {
        count += 1;
        error |= i.is_error();
    }
    (count > 0).then_some(Badge { count, error })
}

/// Which tabs' issue hints the user closed, by a fingerprint of what they
/// showed; a panel comes back when its issues change. Not saved.
#[derive(Default)]
pub(super) struct Dismissed(HashMap<Tab, u64>);

impl Dismissed {
    /// Whether the user closed a tab's panel while it showed `issues`.
    pub(super) fn is(&self, tab: Tab, issues: &[Issue]) -> bool {
        self.0.get(&tab) == Some(&fingerprint(&issues.iter().collect::<Vec<_>>()))
    }

    /// Close a tab's panel until its issues change.
    pub(super) fn dismiss(&mut self, tab: Tab, issues: &[Issue]) {
        self.0.insert(tab, fingerprint(&issues.iter().collect::<Vec<_>>()));
    }
}

fn fingerprint(issues: &[&Issue]) -> u64 {
    let mut h = DefaultHasher::new();
    for i in issues {
        i.message().hash(&mut h);
        i.item.hash(&mut h);
    }
    h.finish()
}

/// One clickable issue line: a mark and the message.
fn issue_row(ui: &mut egui::Ui, lang: &Language, i: &Issue) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if i.severity == Severity::Info {
            ui.add_sized([14.0, 14.0], egui::Label::new(RichText::new("ℹ").weak()));
        } else {
            crate::theme::warning_mark(ui, i.is_error());
        }
        let text = RichText::new(message(lang, i));
        let text = match i.severity {
            Severity::Error => text.color(ui.visuals().error_fg_color),
            Severity::Warning => text,
            Severity::Info => text.weak(),
        };
        ui.add(egui::Label::new(text).sense(egui::Sense::click())).on_hover_text(lang.tr("Show")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    })
    .inner
}

impl CharacterView {
    /// Tabs with their issue badges and, with the guide, their checklist
    /// mark (done, or not visited yet).
    pub(super) fn decorated_tabs(&self, tabs: Vec<(Tab, String)>) -> Vec<(Tab, String, TabDeco)> {
        tabs.into_iter()
            .map(|(t, l)| {
                let deco = TabDeco { badge: badge(self.issues.iter().filter(|i| i.tab().map(tab_of) == Some(t))), check: self.tab_check(t) };
                (t, l, deco)
            })
            .collect()
    }

    /// Badge for the Karma Summary side tab: every issue.
    pub(super) fn summary_badge(&self) -> TabDeco {
        TabDeco { badge: badge(self.issues.iter()), check: None }
    }

    /// Guids of rows with problems, with their messages and whether any is
    /// an error, for the marks in tables.
    pub(super) fn item_marks(&self, lang: &Language) -> HashMap<String, (String, bool)> {
        let mut out: HashMap<String, (String, bool)> = HashMap::new();
        for i in self.issues.iter().filter(|i| i.severity != Severity::Info) {
            if let Some(g) = &i.item {
                let e = out.entry(g.clone()).or_default();
                if !e.0.is_empty() {
                    e.0.push('\n');
                }
                e.0.push_str(&message(lang, i));
                e.1 |= i.is_error();
            }
        }
        out
    }

    /// Go to where an issue can be fixed: its tab and sub-tab, and its row
    /// or item when it has one.
    pub(super) fn jump_to(&mut self, i: &Issue) {
        self.set_reviewing(false);
        self.guide_follow(i);
        if let Some(t) = i.tab() {
            self.tab = tab_of(t);
        }
        if let Some(sub) = gear_sub_tab(i.area) {
            self.gear_tab = sub;
        }
        let Some(guid) = i.item.clone() else { return };
        match i.area {
            Area::ActiveSkills | Area::SkillGroups | Area::KnowledgeSkills => {
                self.skill_filter = i.args.first().cloned().unwrap_or_default();
                self.only_rated = false;
            }
            Area::Contacts => self.relationships.show_contact(&guid),
            // Items the detail pane edits; others only get their tab.
            _ if chummer_core::items::edit::find(&self.doc, &guid).is_some_and(chummer_core::items::edit::is_item) => self.item_editor = Some((guid, crate::item_editor::ItemEditor::default())),
            _ => {}
        }
    }

    /// Every issue, grouped by tab, for the Karma Summary.
    pub(super) fn issue_list(&mut self, ui: &mut egui::Ui, lang: &Language) {
        if self.issues.is_empty() {
            ui.weak(lang.tr("No problems found."));
            return;
        }
        let mut jump: Option<Issue> = None;
        let mut groups: Vec<(Option<IssueTab>, Vec<&Issue>)> = Vec::new();
        for i in &self.issues {
            match groups.iter_mut().find(|(t, _)| *t == i.tab()) {
                Some((_, v)) => v.push(i),
                None => groups.push((i.tab(), vec![i])),
            }
        }
        // Character-wide totals first, then in tab order.
        groups.sort_by_key(|(t, _)| t.map_or(0, |t| 1 + super::TABS.iter().position(|(x, _)| *x == tab_of(t)).unwrap_or(0)));
        for (t, list) in groups {
            let title = match t {
                Some(t) => super::TABS.iter().find(|(x, _)| *x == tab_of(t)).map(|(_, l)| lang.tr(l)).unwrap_or_default(),
                None => lang.tr("Karma & Nuyen"),
            };
            ui.add_space(4.0);
            ui.label(crate::theme::strong(ui, title));
            for i in list {
                if issue_row(ui, lang, i) {
                    jump = Some(i.clone());
                }
            }
        }
        if let Some(i) = jump {
            self.jump_to(&i);
        }
    }
}
