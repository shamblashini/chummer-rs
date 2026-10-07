//! One open character: its tabs and the stats sidebar.

use std::path::PathBuf;
use std::sync::Arc;

use chummer_core::attributes;
use chummer_core::calc::{self, Rules, Sheet};
use chummer_core::character::{Character, INFO_FIELDS, TEXT_FIELDS};
use chummer_core::engine::Engine;
use chummer_core::format;
use chummer_core::lang::Language;
use chummer_core::sections::{self, Section};
use chummer_core::tree::Entry;
use chummer_core::sources::{SourceRef, SourcebookLibrary};

use chummer_core::career;
use chummer_core::chargen;
use chummer_core::command::{Command, RecordRef};
use chummer_core::data;
use chummer_core::settings::CharacterSettings;

use crate::doc::Doc;
use crate::pdf_ui::{self, Status};
use crate::select::{self, SelectDialog};
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};

// Creation issues and guided creation; child modules so they can use the
// view's state.
#[path = "guide_ui.rs"]
mod guide_ui;
#[path = "issues_ui.rs"]
mod issues_ui;
pub use guide_ui::{guided_offer, guided_preference, paint_step_mark, save_guided_preference};
// The Workspace layout's access to the view (`crate::workspace`).
#[path = "workspace/character.rs"]
pub(crate) mod workspace;
// The Workspace's Play screen ("At the table").
#[path = "workspace/play.rs"]
pub(crate) mod play;
// The Workspace's item pages, inline catalog and item inspector.
#[path = "workspace/items.rs"]
pub(crate) mod ws_items;
#[path = "workspace/catalog.rs"]
pub(crate) mod ws_catalog;
#[path = "workspace/inspector.rs"]
pub(crate) mod ws_inspector;

/// The character tabs, in Chummer5a's order (CharacterCareer.Designer.cs).
/// Magic, resonance and critter tabs only show when the character has
/// them, like in Chummer; see [`CharacterView::visible`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Common,
    Skills,
    Limits,
    MartialArts,
    Magician,
    Adept,
    Technomancer,
    AdvancedPrograms,
    Critter,
    Initiation,
    Cyberware,
    StreetGear,
    Vehicles,
    CharacterInfo,
    Karma,
    Calendar,
    Notes,
    Improvements,
    Relationships,
}

pub(crate) const TABS: &[(Tab, &str)] = &[
    (Tab::Common, "Common"),
    (Tab::Skills, "Skills"),
    (Tab::Limits, "Limits"),
    (Tab::MartialArts, "Martial Arts"),
    (Tab::Magician, "Spells & Spirits"),
    (Tab::Adept, "Adept Powers"),
    (Tab::Technomancer, "Complex Forms & Sprites"),
    (Tab::AdvancedPrograms, "Advanced Programs"),
    (Tab::Critter, "Critter Powers"),
    (Tab::Initiation, "Initiation"),
    (Tab::Cyberware, "Cyberware & Bioware"),
    (Tab::StreetGear, "Street Gear"),
    (Tab::Vehicles, "Vehicles & Drones"),
    (Tab::CharacterInfo, "Character Info"),
    (Tab::Karma, "Karma & Nuyen"),
    (Tab::Calendar, "Calendar"),
    (Tab::Notes, "Game Notes"),
    (Tab::Improvements, "Improvements"),
    (Tab::Relationships, "Relationships"),
];

/// Tabs of the right-hand panel (Chummer's `tabInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Summary,
    OtherInfo,
    Condition,
    Defense,
    /// This session's changes (`history_ui`).
    History,
}

/// Street Gear sub-tabs: Gear, Clothing & Armor, Weapons, Drugs, Lifestyles.
const STREET_GEAR: [(&str, Option<Section>); 5] = [
    ("Gear", Some(sections::GEAR)),
    ("Clothing & Armor", Some(sections::ARMOR)),
    ("Weapons", Some(sections::WEAPONS)),
    ("Drugs", None),
    ("Lifestyles", Some(sections::LIFESTYLES)),
];

pub struct CharacterView {
    /// The character; changed only through commands (`Doc::apply`).
    doc: Doc,
    /// The settings preset key `rules`, `store` and `settings` are for.
    settings_key: String,
    /// `Session::revision` the sheet was computed for.
    seen_revision: u64,
    /// Game data with the character's custom data applied.
    store: Arc<chummer_core::data::DataStore>,
    pub sheet: Sheet,
    pub rules: Rules,
    tab: Tab,
    skill_filter: String,
    only_rated: bool,
    /// (container, guid, name) of an item waiting for removal confirmation.
    confirm_remove: Option<(String, String, String)>,
    /// Creation-mode budget, recomputed with the sheet.
    budget: Option<chargen::Budget>,
    /// Creation issues (`chargen::issues`), recomputed with the budget.
    issues: Vec<chargen::issues::Issue>,
    /// Issue hint lines closed until their issues change.
    dismissed: issues_ui::Dismissed,
    /// Guided creation, when on.
    guide: Option<guide_ui::Guide>,
    /// "Hide guide" was clicked; the app turns the preference off.
    guide_hidden: bool,
    settings: Option<CharacterSettings>,
    select: Option<SelectDialog>,
    confirm_finish: bool,
    new_kno: (String, String, bool),
    relationships: crate::relationships_ui::RelationshipsPanel,
    /// Life module picker: (stage, module id, version id).
    life: (String, String, String),
    action: Option<CareerAction>,
    /// Manual ledger entry: (karma?, amount, reason).
    manual: (bool, f64, String),
    initiation: career::InitiationOptions,
    // Magic, lifestyle and drug editors (magic_ui, lifestyle_ui, drug_ui).
    magic_editor: crate::magic_ui::MagicEditor,
    lifestyle_editor: crate::lifestyle_ui::LifestyleEditor,
    drug_builder: crate::drug_ui::DrugBuilder,
    custom_improvements: crate::improvement_ui::ImprovementsPanel,
    // GM tools (gm_ui): PACKS kits and the custom spell designer.
    packs: crate::gm_ui::PacksWindow,
    spell_designer: crate::gm_ui::SpellDesigner,
    /// Item detail pane: (selected item guid, editor).
    item_editor: Option<(String, crate::item_editor::ItemEditor)>,
    side_tab: SideTab,
    /// Counterspelling dice added to the spell defense pools; not saved
    /// (Chummer's `CurrentCounterspellingDice`).
    counterspelling: i32,
    /// Street Gear sub-tab, an index into `STREET_GEAR`.
    gear_tab: usize,
    /// Character Info sub-tab: the text field shown.
    info_text: &'static str,
    /// The campaign member this tab edits (`gm_screen`), if any.
    pub campaign_member: Option<chummer_core::campaign::MemberId>,
    /// Names the tab for the Workspace (`workspace::DocKey`).
    ws_id: u64,
    /// The Workspace's Play screen: rolls, initiative, ammunition choices.
    play: play::PlayState,
    /// The Workspace pages' own state (selection, ledger filter).
    ws_build: workspace::build::State,
    /// The Workspace's item pages: the inline catalog and item inspector.
    ws_gear: ws_items::GearState,
}

/// A career-mode purchase chosen while drawing, run afterwards (it needs
/// the engine and may fail with "not enough karma").
#[derive(Debug, Clone)]
enum CareerAction {
    RaiseAttribute(String),
    RaiseSkill(String),
    RaiseGroup(String),
    Specialize(String, String),
    LearnKnowledge(String, String),
    Undo(String),
    Initiate(career::InitiationOptions),
    RemoveQuality(String),
}

impl CareerAction {
    fn command(&self) -> Command {
        match self.clone() {
            CareerAction::RaiseAttribute(attribute) => Command::RaiseAttribute { attribute },
            CareerAction::RaiseSkill(skill) => Command::RaiseSkill { skill },
            CareerAction::RaiseGroup(group) => Command::RaiseSkillGroup { group },
            CareerAction::Specialize(skill, name) => Command::BuySpecialization { skill, name },
            CareerAction::LearnKnowledge(name, kind) => Command::LearnKnowledgeSkill { name, kind },
            CareerAction::Undo(entry) => Command::UndoExpense { entry },
            CareerAction::Initiate(options) => Command::Initiate { options },
            CareerAction::RemoveQuality(guid) => Command::RemoveItem { container: "qualities".into(), guid },
        }
    }
}


impl Tab {
    /// `--tab` argument: a tab label prefix ("skills", "street"), or one of
    /// the names of chummer-rs's older tabs.
    pub fn parse(s: &str) -> Option<Tab> {
        let s = s.to_ascii_lowercase().replace(['-', '_', ' '], "");
        let old = match s.as_str() {
            "info" => Some(Tab::CharacterInfo),
            "attributes" | "qualities" => Some(Tab::Common),
            "magic" | "spells" => Some(Tab::Magician),
            "equipment" | "gear" => Some(Tab::StreetGear),
            "contacts" => Some(Tab::Relationships),
            "log" => Some(Tab::Karma),
            "notes" => Some(Tab::Notes),
            _ => None,
        };
        old.or_else(|| TABS.iter().find(|(_, label)| label.to_ascii_lowercase().replace([' ', '&'], "").starts_with(&s)).map(|(t, _)| *t))
    }
}

impl CharacterView {
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Whether "Hide guide" was clicked since the last call.
    pub fn take_guide_hidden(&mut self) -> bool {
        std::mem::take(&mut self.guide_hidden)
    }

    pub fn open_packs(&mut self, mode: crate::gm_ui::PacksMode) {
        self.packs.open(mode);
    }

    pub fn new(ch: Character, engine: &Arc<Engine>) -> Self {
        CharacterView::from_doc(Doc::new(ch, engine.clone()), engine)
    }

    /// A tab for an open document (a campaign member lent by the GM
    /// screen, with its history).
    pub fn from_doc(doc: Doc, engine: &Arc<Engine>) -> Self {
        let rules = engine.rules_for(&doc);
        let store = engine.store_for_character(&doc);
        let sheet = calc::compute(&doc, &rules, Some(&store), Some(&engine.catalog));
        let settings_key = doc.field("settings");
        let settings = engine.settings.resolve(&settings_key).cloned();
        let mut v = CharacterView {
            settings_key,
            seen_revision: doc.revision(),
            store,
            doc,
            sheet,
            rules,
            tab: Tab::Common,
            skill_filter: String::new(),
            only_rated: false,
            confirm_remove: None,
            budget: None,
            issues: Vec::new(),
            dismissed: Default::default(),
            guide: None,
            guide_hidden: false,
            settings,
            select: None,
            confirm_finish: false,
            new_kno: (String::new(), "Academic".into(), false),
            relationships: Default::default(),
            life: (String::new(), String::new(), String::new()),
            action: None,
            manual: (true, 0.0, String::new()),
            initiation: career::InitiationOptions::default(),
            magic_editor: Default::default(),
            lifestyle_editor: Default::default(),
            drug_builder: Default::default(),
            custom_improvements: Default::default(),
            packs: Default::default(),
            spell_designer: Default::default(),
            item_editor: None,
            side_tab: SideTab::Summary,
            counterspelling: 0,
            gear_tab: 0,
            info_text: "description",
            campaign_member: None,
            ws_id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            play: Default::default(),
            ws_build: Default::default(),
            ws_gear: Default::default(),
        };
        v.refresh_budget();
        v.set_guided(guided_preference());
        v
    }

    fn refresh_budget(&mut self) {
        match (&self.settings, self.doc.created) {
            (Some(st), false) => {
                let b = chargen::budget_with(&self.doc, &self.sheet, &self.rules, st, Some(&self.store));
                self.issues = chargen::issues::issues(&self.doc, &b, &self.sheet, st, Some(&self.store));
                self.budget = Some(b);
            }
            _ => {
                self.budget = None;
                self.issues.clear();
            }
        }
    }

    pub fn title(&self) -> String {
        let name = match self.doc.sync_state() {
            Some(s) => format!("{} {}", self.doc.display_name(), s.badge()),
            None => self.doc.display_name(),
        };
        if self.doc.dirty {
            format!("{name} •")
        } else {
            name
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.doc.file.clone()
    }

    /// The character, read-only; changes go through [`CharacterView::apply`].
    pub fn ch(&self) -> &Character {
        self.doc.ch()
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// For the GM screen's edits to a member open in this tab; the sheet
    /// follows on the next frame (the session's revision changes).
    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// Close the tab, keeping the document.
    pub fn into_doc(self) -> Doc {
        self.doc
    }

    /// Edit → Undo. Returns what was undone.
    pub fn undo(&mut self, engine: &Engine) -> Option<String> {
        let r = self.doc.undo();
        self.recompute(engine);
        r
    }

    pub fn redo(&mut self, engine: &Engine) -> Option<String> {
        let r = self.doc.redo();
        self.recompute(engine);
        r
    }

    pub fn save(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        self.doc.save(path)
    }

    /// Show the History side tab.
    pub fn show_history(&mut self) {
        self.side_tab = SideTab::History;
    }

    /// Re-resolve the character's preset, e.g. after the settings library
    /// was reloaded.
    pub fn refresh_settings(&mut self, engine: &Engine) {
        self.settings_key = self.doc.field("settings");
        self.rules = engine.rules_for(&self.doc);
        self.store = engine.store_for_character(&self.doc);
        self.settings = engine.settings.resolve(&self.settings_key).cloned();
        self.recompute(engine);
    }

    /// Chummer's "Change Settings File": use another preset.
    fn switch_settings(&mut self, key: &str, status: &mut Status) -> bool {
        self.doc.run(Command::SwitchSettings { key: key.to_owned() }, status).is_some()
    }

    /// The sheet and budgets for the character as it is now. Essence loss
    /// is refreshed by the commands themselves (`command::apply`).
    fn recompute(&mut self, engine: &Engine) {
        self.seen_revision = self.doc.revision();
        if self.doc.field("settings") != self.settings_key {
            // Switched (or undone back to) another preset.
            self.refresh_settings(engine);
            return;
        }
        self.sheet = calc::compute(&self.doc, &self.rules, Some(&self.store), Some(&engine.catalog));
        self.refresh_budget();
    }

    /// Whether a tab shows for this character. Like Chummer, the magic,
    /// resonance and critter tabs follow the character's flags; a tab also
    /// shows while the character has items it lists, so nothing becomes
    /// unreachable.
    pub fn visible(&self, tab: Tab) -> bool {
        let ch = &self.doc;
        let has = |s: &Section| !ch.items(s.container, s.item).is_empty();
        match tab {
            Tab::Magician => (ch.mag_enabled() && ch.is_magician()) || has(&sections::SPELLS) || (has(&sections::SPIRITS) && !ch.res_enabled()),
            Tab::Adept => (ch.mag_enabled() && ch.is_adept()) || has(&sections::POWERS),
            Tab::Technomancer => ch.res_enabled() || has(&sections::COMPLEX_FORMS),
            Tab::AdvancedPrograms => ch.advanced_programs_enabled() || has(&sections::AI_PROGRAMS),
            Tab::Critter => ch.flag("critter") || has(&sections::CRITTER_POWERS),
            Tab::Initiation => ch.mag_enabled() || ch.res_enabled() || has(&sections::METAMAGICS),
            Tab::Karma => ch.created || !career::entries(ch).is_empty(),
            Tab::Calendar | Tab::Notes => ch.created,
            Tab::Improvements => ch.created || !chummer_core::custom_improvement::listed(ch).is_empty(),
            _ => true,
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> Option<u32> {
        let mut changed = self.begin_frame();
        let mut roll: Option<u32> = None;
        egui::SidePanel::right("sheet_panel").resizable(true).default_width(310.0).min_width(220.0).show(ctx, |ui| {
            changed |= self.side_panel(ui, lang, &mut roll);
        });
        changed |= self.item_editor_panel(ctx, engine, lang, status);
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(key) = crate::ruleset_ui::banner(ui, &self.doc, engine, lang, self.tab == Tab::Common) {
                changed |= self.switch_settings(&key, status);
            }
            let tabs: Vec<(Tab, String)> = TABS.iter().filter(|(t, _)| self.visible(*t)).map(|(t, l)| (*t, lang.tr(l))).collect();
            let tabs = self.decorated_tabs(tabs);
            crate::theme::tab_strip_with(ui, &mut self.tab, &tabs);
            self.page_hint(ui, lang, pdfs, status, self.tab, None, false);
            changed |= self.tab_page(ui, self.tab, engine, lang, pdfs, status, &mut roll);
        });
        self.end_frame(ctx, engine, lang, pdfs, status, changed);
        roll
    }

    /// Start of a frame: take what arrived for an online character, and
    /// leave a tab the character no longer has. Returns true if the
    /// character changed.
    fn begin_frame(&mut self) -> bool {
        // An online character takes what arrived from the campaign.
        let changed = self.doc.refresh();
        if !self.visible(self.tab) {
            self.tab = Tab::Common;
        }
        changed
    }

    /// One tab's page. Returns true if the character changed.
    #[allow(clippy::too_many_arguments)]
    fn tab_page(&mut self, ui: &mut egui::Ui, tab: Tab, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        let salt = tab as u8;
        let page = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui) -> bool| egui::ScrollArea::both().id_salt(("tab_page", salt)).auto_shrink(false).show(ui, |ui| f(ui)).inner;
        match tab {
            Tab::Common => self.common_tab(ui, engine, lang, pdfs, status),
            Tab::Skills => self.skills_tab(ui, engine, lang, pdfs, status, roll),
            Tab::Limits => page(ui, &mut |ui| self.limits_tab(ui, lang)),
            Tab::MartialArts | Tab::Magician | Tab::Adept | Tab::Technomancer | Tab::Critter | Tab::Initiation => page(ui, &mut |ui| self.magic_page(ui, engine, lang, pdfs, status, tab)),
            Tab::AdvancedPrograms => page(ui, &mut |ui| {
                self.add_buttons(ui, engine, lang, "aiprograms");
                crate::ai_ui::tab(ui, &mut self.doc, engine, lang, status)
            }),
            Tab::Cyberware => page(ui, &mut |ui| self.gear_page(ui, engine, lang, pdfs, status, sections::CYBERWARE)),
            Tab::StreetGear => self.street_gear_tab(ui, engine, lang, pdfs, status),
            Tab::Vehicles => page(ui, &mut |ui| self.gear_page(ui, engine, lang, pdfs, status, sections::VEHICLES)),
            Tab::CharacterInfo => self.info_tab(ui, lang),
            Tab::Karma => self.log_tab(ui, engine, lang),
            Tab::Calendar => page(ui, &mut |ui| self.calendar_ui(ui, lang)),
            Tab::Notes => self.notes_tab(ui, lang),
            Tab::Improvements => self.improvements_tab(ui, lang),
            Tab::Relationships => self.relationships.ui(ui, &mut self.doc, &self.store, lang, status),
        }
    }

    /// End of a frame: the dialogs, a career purchase picked while
    /// drawing, and the sheet and budgets after a change.
    fn end_frame(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, mut changed: bool) {
        changed |= self.confirm_dialog(ctx, lang);
        changed |= self.select_dialog(ctx, lang, pdfs, status);
        changed |= self.drug_builder.window(ctx, &mut self.doc, &self.store, lang, status);
        changed |= self.custom_improvements.window(ctx, &mut self.doc, &self.store, self.settings.as_ref(), lang);
        changed |= self.packs.window(ctx, &mut self.doc, &self.store, self.settings.as_ref(), &self.sheet, lang, status);
        changed |= self.spell_designer.window(ctx, &mut self.doc, engine, &self.store, lang, status);
        changed |= self.finish_dialog(ctx, lang);
        if let Some(a) = self.action.take() {
            changed |= self.run_action(a, status);
        }
        if changed || self.doc.revision() != self.seen_revision {
            self.recompute(engine);
        }
    }

    /// Chummer's status strip: karma, essence and nuyen at a glance.
    pub fn status_items(&self, lang: &Language) -> Vec<(String, String)> {
        let mut out = Vec::new();
        match &self.budget {
            Some(b) => {
                out.push((lang.tr("Karma:"), b.karma.0.to_string()));
                out.push((lang.tr("Karma Remaining:"), b.karma_left().to_string()));
            }
            None => out.push((lang.tr("Karma:"), self.doc.karma.to_string())),
        }
        out.push((lang.tr("Essence:"), format::essence(self.sheet.essence, self.rules.essence_decimals)));
        match &self.budget {
            Some(b) => out.push((lang.tr("Nuyen Remaining:"), format::nuyen(b.nuyen_left()))),
            None => out.push((lang.tr("Nuyen:"), format::nuyen(self.doc.nuyen))),
        }
        out
    }

    // ----- right-hand panel -----

    /// Chummer's right-hand tabs: Karma Summary (creation), Condition
    /// Monitor and Other Info.
    fn side_panel(&mut self, ui: &mut egui::Ui, lang: &Language, roll: &mut Option<u32>) -> bool {
        // Creation: Karma Summary, Other Info, Spell Defense; career puts
        // the Condition Monitor first instead.
        let mut tabs = vec![(SideTab::OtherInfo, lang.tr("Other Info")), (SideTab::Defense, lang.s("String_SpellDefense")), (SideTab::History, lang.tr("History"))];
        if self.budget.is_some() {
            tabs.insert(0, (SideTab::Summary, lang.tr("Karma Summary")));
        } else {
            tabs.insert(0, (SideTab::Condition, lang.tr("Condition Monitor")));
        }
        if !tabs.iter().any(|(t, _)| *t == self.side_tab) {
            self.side_tab = tabs[0].0;
        }
        let summary = self.summary_badge();
        let tabs: Vec<_> = tabs.into_iter().map(|(t, l)| (t, l, if t == SideTab::Summary { summary } else { Default::default() })).collect();
        crate::theme::tab_strip_with(ui, &mut self.side_tab, &tabs);
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("side_scroll").auto_shrink(false).show(ui, |ui| {
            changed = match self.side_tab {
                SideTab::Summary => self.budget_panel(ui, lang),
                SideTab::OtherInfo => self.other_info(ui, lang, roll),
                SideTab::Condition => self.condition_monitor(ui, lang),
                SideTab::Defense => self.spell_defense(ui, lang),
                SideTab::History => crate::history_ui::panel(ui, &mut self.doc, lang),
            };
        });
        changed
    }

    /// Resources and derived values (Chummer's "Other Info").
    fn other_info(&mut self, ui: &mut egui::Ui, lang: &Language, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        let s = &self.sheet;
        egui::Grid::new("resources").num_columns(2).show(ui, |ui| {
            ui.label(lang.tr("Karma"));
            let mut karma = self.doc.karma;
            if ui.add(egui::DragValue::new(&mut karma).speed(0.2)).changed() {
                changed |= self.doc.set(Command::SetKarma { value: karma });
            }
            ui.end_row();
            ui.label(lang.tr("Nuyen"));
            let mut nuyen = self.doc.nuyen;
            if ui.add(egui::DragValue::new(&mut nuyen).speed(10.0).max_decimals(2).suffix("¥")).changed() {
                changed |= self.doc.set(Command::SetNuyen { value: nuyen });
            }
            ui.end_row();
            ui.label(lang.tr("Essence"));
            ui.strong(format::essence(s.essence, self.rules.essence_decimals));
            ui.end_row();
        });
        ui.separator();
        let stat = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.label(label);
            ui.label(RichText::new(value).monospace());
            ui.end_row();
        };
        egui::Grid::new("derived").num_columns(2).striped(true).show(ui, |ui| {
            stat(ui, &lang.tr("Physical CM"), s.physical_cm.to_string());
            stat(ui, &lang.tr("Stun CM"), s.stun_cm.to_string());
            stat(ui, &lang.tr("Initiative"), format!("{} + {}d6", s.initiative, s.initiative_dice));
            stat(ui, &lang.tr("Astral"), format!("{} + {}d6", s.astral_initiative, s.astral_initiative_dice));
            stat(ui, &lang.tr("Matrix cold-sim"), format!("{} + {}d6", s.matrix_cold_initiative, s.matrix_cold_dice));
            stat(ui, &lang.tr("Matrix hot-sim"), format!("{} + {}d6", s.matrix_hot_initiative, s.matrix_hot_dice));
            stat(ui, &lang.tr("Physical limit"), s.limit_physical.to_string());
            stat(ui, &lang.tr("Mental limit"), s.limit_mental.to_string());
            stat(ui, &lang.tr("Social limit"), s.limit_social.to_string());
            if self.doc.mag_enabled() {
                stat(ui, &lang.tr("Astral limit"), s.limit_astral.to_string());
            }
            stat(ui, &lang.tr("Armor"), s.armor.to_string());
            if self.doc.created {
                stat(ui, &lang.tr("Career Karma"), career::career_karma(&self.doc).to_string());
            }
            stat(ui, &lang.tr("Composure"), s.composure.to_string());
            stat(ui, &lang.tr("Judge Intentions"), s.judge_intentions.to_string());
            stat(ui, &lang.tr("Lift and Carry"), s.lift_carry.to_string());
            stat(ui, &lang.tr("Memory"), s.memory.to_string());
        });
        ui.add_space(6.0);
        if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Open Dice Roller"))).clicked() {
            *roll = Some(6);
        }
        changed
    }

    /// Damage tracks side by side, the wound penalty and Edge (Chummer's
    /// "Condition Monitor").
    /// Chummer's Spell Defense tab: each pool with the counterspelling dice
    /// added in parentheses.
    fn spell_defense(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        egui::Grid::new("spell_defense").num_columns(2).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(lang.s("Label_CounterspellingDice"));
            ui.add(egui::DragValue::new(&mut self.counterspelling).range(0..=100));
            ui.end_row();
            for (key, pool) in chummer_core::calc::spell_defense(&self.doc, &self.sheet) {
                ui.label(lang.s(key));
                let text = if self.counterspelling == 0 { pool.to_string() } else { format!("{pool} ({})", pool + self.counterspelling) };
                ui.label(RichText::new(text).monospace());
                ui.end_row();
            }
        });
        false
    }

    fn condition_monitor(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        let s = &self.sheet;
        egui::Grid::new("cm_stats").num_columns(2).show(ui, |ui| {
            ui.label(lang.tr("CM Penalty:"));
            let t = RichText::new(s.wound_modifier.to_string()).monospace();
            ui.label(if s.wound_modifier != 0 { t.color(crate::theme::warn(ui)) } else { t });
            ui.end_row();
            ui.label(lang.tr("Armor"));
            ui.label(RichText::new(s.armor.to_string()).monospace());
            ui.end_row();
        });
        ui.add_space(4.0);
        let (pcm, scm, thr) = (s.physical_cm, s.stun_cm, s.cm_threshold);
        let overflow = s.cm_overflow;
        let (pal_p, pal_s) = (crate::theme::palette(ui).physical, crate::theme::palette(ui).stun);
        let (plabel, slabel) = crate::ai_ui::cm_labels(&self.doc, lang);
        ui.columns(2, |cols| {
            cols[0].label(RichText::new(plabel).strong());
            let mut pf = chummer_core::play::ai::physical_filled(&self.doc);
            if cm_track(&mut cols[0], "pcm", pcm, thr, &mut pf, pal_p) {
                changed |= self.doc.set(Command::SetPhysicalDamage { filled: pf });
            }
            cols[1].label(RichText::new(slabel).strong());
            let mut sf = chummer_core::play::ai::stun_filled(&self.doc);
            if cm_track(&mut cols[1], "scm", scm, if self.doc.is_ai() { 0 } else { thr }, &mut sf, pal_s) {
                changed |= self.doc.set(Command::SetStunDamage { filled: sf });
            }
        });
        ui.weak(lang.tr_fmt("Overflow {0} · −1 die per {1} boxes", &[&overflow, &thr]));
        ui.separator();
        changed |= crate::play_ui::edge_track(ui, &mut self.doc, &self.sheet, lang);
        changed
    }

    // ----- tabs -----

    /// Character Info: personal details and reputation on top, the long
    /// texts below as Chummer's sub-tabs.
    fn info_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("info_scroll").auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("info").num_columns(6).spacing([12.0, 6.0]).show(ui, |ui| {
                for (i, (key, label)) in INFO_FIELDS.iter().enumerate() {
                    ui.label(lang.tr(label));
                    let mut v = self.doc.field(key);
                    let editable = !matches!(*key, "metatype" | "metavariant");
                    let r = ui.add_enabled_ui(editable, |ui| ui.add_sized([170.0, 20.0], egui::TextEdit::singleline(&mut v))).inner;
                    if r.changed() {
                        changed |= self.doc.set(Command::SetField { key: (*key).to_owned(), value: v });
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(8.0);
            egui::Grid::new("reputation").num_columns(6).spacing([12.0, 6.0]).show(ui, |ui| {
                for (key, label) in [("streetcred", lang.tr("Street Cred")), ("notoriety", lang.tr("Notoriety")), ("publicawareness", lang.tr("Public Awareness"))] {
                    ui.label(label);
                    let mut v = self.doc.doc.get_i32(key).unwrap_or(0);
                    if ui.add(egui::DragValue::new(&mut v).range(0..=100)).changed() {
                        changed |= self.doc.set(Command::SetField { key: key.to_owned(), value: v.to_string() });
                    }
                }
            });
            ui.add_space(10.0);
            let mut subs: Vec<(&'static str, String)> = vec![
                ("description", lang.tr("Description")),
                ("background", lang.tr("Background")),
                ("concept", lang.tr("Concept")),
                ("notes", lang.tr("Character Notes")),
            ];
            if !self.doc.created {
                // Career mode has its own Game Notes tab.
                subs.push(("gamenotes", lang.tr("Game Notes")));
            }
            if !subs.iter().any(|(k, _)| *k == self.info_text) {
                self.info_text = "description";
            }
            crate::theme::tab_strip(ui, &mut self.info_text, &subs);
            let key = self.info_text;
            let mut v = self.doc.field(key);
            if ui.add(egui::TextEdit::multiline(&mut v).id_salt(key).desired_width(f32::INFINITY).desired_rows(18)).changed() {
                changed |= self.doc.set(Command::SetField { key: key.to_owned(), value: v });
            }
        });
        changed
    }

    /// Chummer's Common tab: qualities on the left; alias, metatype and
    /// starting nuyen above the attributes on the right.
    fn common_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let mut changed = false;
        egui::SidePanel::left("common_qualities").resizable(true).default_width(280.0).min_width(200.0).show_inside(ui, |ui| {
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add Quality…"))).clicked() {
                self.open_select("quality", engine);
            }
            egui::ScrollArea::both().id_salt("qualities_scroll").auto_shrink(false).show(ui, |ui| {
                changed |= self.section(ui, &sections::QUALITIES, lang, pdfs, status);
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 10, ..Default::default() })).show_inside(ui, |ui| {
            changed |= self.common_header(ui, lang);
            if !self.doc.created && self.doc.field("buildmethod") == "LifeModule" {
                changed |= self.life_module_picker(ui, engine, lang, status);
            }
            ui.add_space(6.0);
            changed |= self.attributes_tab(ui, engine, lang);
        });
        changed
    }

    /// Alias, metatype, build and Chummer's karma-for-nuyen spinner.
    fn common_header(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(lang.tr("Alias:"));
            let mut alias = self.doc.field("alias");
            if ui.add(egui::TextEdit::singleline(&mut alias).desired_width(200.0)).changed() {
                changed |= self.doc.set(Command::SetField { key: "alias".into(), value: alias });
            }
            ui.separator();
            ui.label(lang.tr("Metatype:"));
            let meta: Vec<String> = ["metatype", "metavariant"].iter().map(|k| self.doc.field(k)).filter(|v| !v.is_empty()).collect();
            ui.strong(meta.join(" · "));
            ui.separator();
            ui.weak(format!(
                "{} · {}",
                if self.doc.created { lang.tr("Career") } else { lang.tr("Creation") },
                match self.doc.field("buildmethod").as_str() {
                    "SumtoTen" => lang.tr("Sum-to-Ten"),
                    "" => lang.tr("Priority"),
                    b => lang.tr(b),
                }
            ));
            if let Some(b) = &self.budget {
                ui.separator();
                ui.label(lang.tr("Nuyen:"));
                let mut bp = self.doc.doc.get_i32("nuyenbp").unwrap_or(0);
                let max = self.settings.as_ref().map_or(10, |s| s.int("nuyenmaxbp", 10));
                if ui.add(egui::DragValue::new(&mut bp).range(0..=max).suffix(format!(" {}", lang.tr("karma")))).on_hover_text(lang.tr("2,000¥ per karma")).changed() {
                    changed |= self.doc.set(Command::SetField { key: "nuyenbp".into(), value: bp.to_string() });
                }
                ui.label(format!("= {}", format::nuyen(b.nuyen.0)));
            }
        });
        changed
    }

    /// Limits and the improvements that modify them.
    fn limits_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let s = &self.sheet;
        egui::Grid::new("limits").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            let mut row = |label: &str, v: i32| {
                ui.label(label);
                ui.strong(v.to_string());
                ui.end_row();
            };
            row(&lang.tr("Physical"), s.limit_physical);
            row(&lang.tr("Mental"), s.limit_mental);
            row(&lang.tr("Social"), s.limit_social);
            if self.doc.mag_enabled() {
                row(&lang.tr("Astral"), s.limit_astral);
            }
        });
        ui.add_space(10.0);
        let imps = &self.doc.improvements;
        let mods: Vec<_> = imps.list.iter().filter(|i| i.kind.contains("Limit")).collect();
        ui.heading(lang.tr("Limit Modifiers"));
        if mods.is_empty() {
            ui.weak(lang.tr("None."));
            return false;
        }
        egui::Grid::new("limit_mods").striped(true).num_columns(5).spacing([14.0, 3.0]).show(ui, |ui| {
            for h in lang.tr_all(["Type", "Target", "Value", "Source", "Condition"]) {
                ui.strong(h);
            }
            ui.end_row();
            for i in mods {
                let label = if imps.applies(i) { RichText::new(&i.kind) } else { RichText::new(&i.kind).weak() };
                ui.label(label);
                ui.label(&i.improved_name);
                ui.label(fmt_opt(i.val));
                ui.weak(&i.source);
                ui.weak(&i.condition);
                ui.end_row();
            }
        });
        false
    }

    /// The attribute table, in Chummer's column order (Points, Karma,
    /// Val (Aug), Metatype Limits), plus karma costs.
    fn attributes_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut changed = false;
        let career = self.doc.created;
        let priority = chummer_core::character::uses_priority_tables(&self.doc.field("buildmethod"));
        let shown = self.shown_attributes();
        let accent = crate::theme::accent(ui);
        // Scrolls sideways instead of clipping the Raise buttons when the
        // window is narrow; cells never wrap.
        egui::ScrollArea::horizontal().id_salt("attributes_scroll").show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        TableBuilder::new(ui)
            .id_salt("attributes")
            .striped(true)
            .column(Column::auto().at_least(130.0))
            .columns(Column::auto().at_least(56.0), 6)
            .header(22.0, |mut h| {
                for t in lang.tr_all(["Attributes", "Points", "Karma", "Val (Aug)", "Metatype Limits", "Karma cost", "Next level"]) {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for name in shown {
                    let Some(v) = self.sheet.attr_values(name).cloned() else { continue };
                    body.row(24.0, |mut row| {
                        row.col(|ui| {
                            let key = format!("String_Attribute{name}Long");
                            let long = if lang.has(&key) { lang.s(&key) } else { lang.tr(attributes::long_name(name)) };
                            ui.label(format!("{long} ({name})"));
                        });
                        row.col(|ui| {
                            if let Some(a) = self.doc.attribute(name) {
                                let (mut base, karma) = (a.base, a.karma);
                                let max = attribute_base_max(&v, base, karma);
                                let r = ui.add_enabled(priority && !career, egui::DragValue::new(&mut base).range(0..=max));
                                if r.changed() {
                                    changed |= self.doc.set(Command::SetAttributeBase { attribute: name.to_owned(), value: base });
                                }
                            }
                        });
                        row.col(|ui| {
                            if let Some(a) = self.doc.attribute(name) {
                                let mut karma = a.karma;
                                let max = attribute_karma_max(&v, karma);
                                if ui.add_enabled(!career, egui::DragValue::new(&mut karma).range(0..=max)).changed() {
                                    changed |= self.doc.set(Command::SetAttributeKarma { attribute: name.to_owned(), value: karma });
                                }
                            }
                        });
                        row.col(|ui| {
                            ui.horizontal(|ui| {
                                ui.strong(v.value.to_string());
                                if v.total != v.value {
                                    ui.colored_label(accent, format!("({})", v.total));
                                }
                            });
                        });
                        row.col(|ui| {
                            ui.label(format!("{} / {} ({})", v.total_min, v.total_max, v.total_aug_max));
                        });
                        row.col(|ui| {
                            ui.label(calc::attribute_karma_cost(&v, &self.rules).to_string());
                        });
                        row.col(|ui| {
                            if career {
                                match career::attribute_upgrade_karma_cost(engine, &self.doc, name) {
                                    Some(c) => {
                                        let r = ui.add_enabled(self.doc.karma >= c, egui::Button::new(lang.tr_fmt("Raise ({0} karma)", &[&c])));
                                        if r.clicked() {
                                            self.action = Some(CareerAction::RaiseAttribute(name.to_owned()));
                                        }
                                    }
                                    None => {
                                        ui.weak(lang.tr("at maximum"));
                                    }
                                }
                            } else {
                                match calc::attribute_upgrade_cost(&v, &self.rules) {
                                    Some(c) => ui.label(lang.tr_fmt("{0} karma", &[&c])),
                                    None => ui.weak(lang.tr("at maximum")),
                                };
                            }
                        });
                    });
                }
            });
        });
        ui.add_space(8.0);
        ui.weak(if career {
            lang.tr("Career mode: Raise spends karma and records it in the Karma & Nuyen log, where it can be undone.")
        } else {
            lang.tr("Creation mode: base uses attribute points (priority builds). Changing levels does not deduct karma automatically; the Karma cost column shows what they are worth.")
        });
        ui.label(format!("{} {}", lang.tr("Karma spent on attributes:"), self.sheet.attribute_karma_spent));
        changed
    }

    fn skills_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, roll: &mut Option<u32>) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.skill_filter).hint_text(lang.tr("Filter skills")).desired_width(200.0));
            ui.checkbox(&mut self.only_rated, lang.tr("Only skills with a rating"));
            ui.separator();
            if self.doc.created {
                ui.label(format!("{} {}", lang.tr("Karma value of skills:"), self.sheet.skill_karma_spent));
            } else {
                ui.label(format!(
                    "{} {} / {} · {} {}",
                    lang.tr("Knowledge Points:"),
                    self.sheet.knowledge_points_used,
                    self.sheet.knowledge_points,
                    lang.tr("karma spent on skills:"),
                    self.sheet.skill_karma_spent
                ));
            }
        });
        ui.add_space(4.0);
        let marks = self.item_marks(lang);
        let filter = |name: &str, rating: i32| skill_matches(&self.skill_filter, self.only_rated, name, rating);
        let rows: Vec<(usize, calc::SkillValues)> =
            self.sheet.skills.iter().cloned().enumerate().filter(|(_, s)| filter(&s.name, s.rating)).collect();
        let kno: Vec<(usize, calc::SkillValues)> =
            self.sheet.knowledge_skills.iter().cloned().enumerate().filter(|(_, s)| filter(&s.name, s.rating)).collect();
        let career = self.doc.created;
        // Skills cap at 6 during creation (setting-dependent), 12 in career.
        let cap = if career { self.rules.max_skill_rating_career } else { self.rules.max_skill_rating_create };
        egui::TopBottomPanel::bottom("knowledge_panel").resizable(true).default_height(240.0).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("kno_scroll").auto_shrink(false).show(ui, |ui| {
                    ui.heading(lang.tr("Knowledge Skills"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.new_kno.0).hint_text(lang.tr("New Knowledge Skill")).desired_width(200.0));
                        crate::combo::Combo::from_id_salt("kno_type").selected_text(lang.data_name("skills.xml", "", &self.new_kno.1)).show_ui(ui, |ui| {
                            for t in ["Academic", "Interest", "Language", "Professional", "Street"] {
                                crate::combo::selectable_value(ui, &mut self.new_kno.1, t.to_owned(), lang.data_name("skills.xml", "", t));
                            }
                        });
                        if self.new_kno.1 == "Language" {
                            ui.checkbox(&mut self.new_kno.2, lang.tr("Native"));
                        }
                        if ui.add_enabled(!self.new_kno.0.trim().is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                            let native = self.new_kno.1 == "Language" && self.new_kno.2;
                            if career {
                                self.action = Some(CareerAction::LearnKnowledge(self.new_kno.0.trim().to_owned(), self.new_kno.1.clone()));
                            } else {
                                let cmd = Command::AddKnowledgeSkill { name: self.new_kno.0.trim().to_owned(), kind: self.new_kno.1.clone(), native };
                                changed |= self.doc.set(cmd);
                            }
                            self.new_kno.0.clear();
                        }
                    });
                    let mut remove_kno: Option<String> = None;
                    egui::Grid::new("kskills").striped(true).num_columns(7).spacing([14.0, 4.0]).show(ui, |ui| {
                        for h in lang.tr_all(["Skill", "Type", "Base", "Karma", "Rating", "Pool", ""]) {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for (_, s) in &kno {
                            ui.horizontal(|ui| {
                                if let Some((msg, err)) = marks.get(&s.guid) {
                                    crate::theme::warning_mark(ui, *err).on_hover_text(msg);
                                }
                                ui.label(&s.name);
                            });
                            ui.weak(lang.data_name("skills.xml", "", &s.category));
                            if s.native {
                                ui.weak(lang.tr("native"));
                                ui.label("");
                                ui.label("N");
                                ui.label("N");
                            } else {
                                let (mut base, mut karma) = self.doc.knowledge_skills.iter().find(|k| k.guid == s.guid).map_or((0, 0), |k| (k.base, k.karma));
                                if ui.add_enabled(!career, egui::DragValue::new(&mut base).range(0..=cap)).changed() {
                                    changed |= self.doc.set(Command::SetKnowledgeBase { skill: s.guid.clone(), value: base });
                                }
                                if ui.add_enabled(!career, egui::DragValue::new(&mut karma).range(0..=cap)).changed() {
                                    changed |= self.doc.set(Command::SetKnowledgeKarma { skill: s.guid.clone(), value: karma });
                                }
                                ui.horizontal(|ui| {
                                    ui.label(s.rating.to_string());
                                    if career {
                                        if let Some(c) = career::skill_upgrade_karma_cost(engine, &self.doc, &s.guid) {
                                            if ui.add_enabled(self.doc.karma >= c, egui::Button::new(format!("↑ {c}"))).clicked() {
                                                self.action = Some(CareerAction::RaiseSkill(s.guid.clone()));
                                            }
                                        }
                                    }
                                });
                                ui.strong(s.pool.to_string());
                            }
                            if ui.small_button(crate::theme::glyph("🗑")).on_hover_text(lang.tr("Remove")).clicked() {
                                remove_kno = Some(s.guid.clone());
                            }
                            ui.end_row();
                        }
                    });
                    if let Some(g) = remove_kno {
                        changed |= self.doc.set(Command::RemoveKnowledgeSkill { skill: g });
                    }
            });
        });
        if !self.doc.skill_groups.is_empty() {
            egui::SidePanel::left("skill_groups_panel").resizable(true).default_width(270.0).show_inside(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("group_scroll").auto_shrink(false).show(ui, |ui| {
                    if !self.doc.skill_groups.is_empty() {
                        ui.add_space(12.0);
                        ui.heading(lang.tr("Skill Groups"));
                        egui::Grid::new("groups").striped(true).num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                            for h in lang.tr_all(["Group", "Base", "Karma", "Rating"]) {
                                ui.strong(h);
                            }
                            ui.end_row();
                            let costs: Vec<Option<i32>> = self
                                .doc
                                .skill_groups
                                .iter()
                                .map(|g| if career { career::skill_group_upgrade_karma_cost(engine, &self.doc, &g.name) } else { None })
                                .collect();
                            let karma = self.doc.karma;
                            let groups = self.doc.skill_groups.clone();
                            for (g, cost) in groups.iter().zip(costs) {
                                ui.label(&g.name);
                                let (mut base, mut gk) = (g.base, g.karma);
                                if ui.add_enabled(!career, egui::DragValue::new(&mut base).range(0..=cap)).changed() {
                                    changed |= self.doc.set(Command::SetGroupBase { group: g.name.clone(), value: base });
                                }
                                if ui.add_enabled(!career, egui::DragValue::new(&mut gk).range(0..=cap)).changed() {
                                    changed |= self.doc.set(Command::SetGroupKarma { group: g.name.clone(), value: gk });
                                }
                                ui.horizontal(|ui| {
                                    ui.label(g.rating().to_string());
                                    if let Some(c) = cost {
                                        if ui.add_enabled(karma >= c, egui::Button::new(format!("↑ {c}"))).clicked() {
                                            self.action = Some(CareerAction::RaiseGroup(g.name.clone()));
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    }
                });
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 8, ..Default::default() })).show_inside(ui, |ui| {
            egui::ScrollArea::both().id_salt("active_scroll").auto_shrink(false).show(ui, |ui| {
                    ui.heading(lang.tr("Active Skills"));
                    egui::Grid::new("skills").striped(true).num_columns(8).spacing([14.0, 4.0]).show(ui, |ui| {
                        for h in lang.tr_all(["Skill", "Attr", "Group", "Base", "Karma", "Rating", "Pool", "Specializations"]) {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for (_, s) in &rows {
                            let r = SourceRef::new(&s.source, &s.page);
                            let label = if s.disabled { RichText::new(&s.name).weak() } else { RichText::new(&s.name) };
                            let name = ui
                                .horizontal(|ui| {
                                    if let Some((msg, err)) = marks.get(&s.guid) {
                                        crate::theme::warning_mark(ui, *err).on_hover_text(msg);
                                    }
                                    ui.add(egui::Label::new(label).sense(egui::Sense::click()))
                                })
                                .inner;
                            if let Some(r) = r {
                                if name.on_hover_text(format!("{r} — {}", lang.tr("click to open the rulebook"))).clicked() {
                                    pdf_ui::open(pdfs, &r, status);
                                }
                            }
                            ui.label(&s.attribute);
                            ui.weak(&s.group);
                            let (mut base, mut karma) = self.doc.skills.iter().find(|k| k.guid == s.guid).map_or((0, 0), |k| (k.base, k.karma));
                            let on = !s.disabled;
                            if ui.add_enabled(!career && on, egui::DragValue::new(&mut base).range(0..=cap)).changed() {
                                changed |= self.doc.set(Command::SetSkillBase { skill: s.guid.clone(), value: base });
                            }
                            if ui.add_enabled(on && !career, egui::DragValue::new(&mut karma).range(0..=cap)).changed() {
                                changed |= self.doc.set(Command::SetSkillKarma { skill: s.guid.clone(), value: karma });
                            }
                            ui.label(s.rating.to_string());
                            let pool = if s.rating == 0 && !s.default { "—".to_owned() } else { s.pool.to_string() };
                            if crate::theme::pool_chip(ui, pool).on_hover_text(lang.tr("Roll this pool")).clicked() {
                                *roll = Some(s.pool.max(1) as u32);
                            }
                            ui.horizontal(|ui| {
                                if career && !s.disabled {
                                    if let Some(c) = career::skill_upgrade_karma_cost(engine, &self.doc, &s.guid) {
                                        if ui.add_enabled(self.doc.karma >= c, egui::Button::new(format!("↑ {c}"))).on_hover_text(lang.tr("Raise for karma")).clicked() {
                                            self.action = Some(CareerAction::RaiseSkill(s.guid.clone()));
                                        }
                                    }
                                }
                                if !s.specs.is_empty() {
                                    ui.label(format!("{} (+{})", s.specs.join(", "), s.spec_bonus));
                                }
                                let guid = s.guid.clone();
                                let suid = self.doc.skills.iter().find(|k| k.guid == s.guid).map(|k| k.suid.clone()).unwrap_or_default();
                                ui.menu_button("+", |ui| {
                                    let opts = engine.catalog.get(&suid).map(|d| d.specs.clone()).unwrap_or_default();
                                    for o in opts.iter().filter(|o| !s.specs.contains(o)) {
                                        if ui.button(o).clicked() {
                                            if career {
                                                self.action = Some(CareerAction::Specialize(guid.clone(), o.clone()));
                                            } else {
                                                changed |= self.doc.set(Command::AddSpecialization { skill: guid.clone(), name: o.clone() });
                                            }
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(lang.tr("Add a specialization"));
                            });
                            ui.end_row();
                        }
                    });
            });
        });
        changed
    }

    /// One of the magic, resonance, critter and martial arts tabs: its
    /// summary line, then its sections (spells and spirits, powers, ...).
    fn magic_page(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, tab: Tab) -> bool {
        let mut changed = false;
        if tab == Tab::Magician && self.doc.mag_enabled() && self.doc.is_magician() {
            let current = self.doc.doc.child("tradition").map(|t| t.get("name")).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(lang.tr("Tradition"));
                let mut pick: Option<String> = None;
                crate::combo::Combo::from_id_salt("tradition").selected_text(if current.is_empty() { lang.tr("Choose…") } else { current.clone() }).width(240.0).show_ui(ui, |ui| {
                    if let Ok(doc) = self.store.doc("traditions.xml") {
                        for r in data::records(&doc, "traditions", "tradition") {
                            if crate::combo::selectable_label(ui, current == r.name(), r.name()).clicked() {
                                pick = Some(r.name());
                            }
                        }
                    }
                });
                if let Some(p) = pick {
                    changed |= self.doc.run(Command::SetTradition { name: p }, status).is_some();
                }
            });
        }
        let m = chummer_core::items::magic::magic_summary_with(&self.doc, &self.sheet, Some(&self.store));
        ui.horizontal_wrapped(|ui| match tab {
            Tab::Magician => {
                if !m.tradition.is_empty() {
                    ui.label(format!("{} {}", lang.tr("Tradition:"), m.tradition));
                    ui.label(lang.tr_fmt("Drain {0} = {1} dice", &[&m.drain_expression.replace(['{', '}'], ""), &m.drain_pool]));
                }
                if self.doc.mag_enabled() {
                    ui.label(lang.tr_fmt("Astral {0} + {1}d6, limit {2}", &[&m.astral_initiative, &m.astral_initiative_dice, &m.astral_limit]));
                }
            }
            Tab::Technomancer => {
                if !m.stream.is_empty() {
                    ui.label(lang.tr_fmt("Stream: {0} · Fading {1} = {2} dice", &[&m.stream, &m.fading_expression.replace(['{', '}'], ""), &m.fading_pool]));
                }
            }
            Tab::Adept => {
                if let Some((total, used)) = m.power_points {
                    let t = RichText::new(format!("{} {used} / {total}", lang.tr("Power Points")));
                    ui.label(if used > total { t.color(ui.visuals().error_fg_color) } else { t });
                }
            }
            _ => {}
        });
        if tab == Tab::Initiation {
            let techno = self.doc.res_enabled() && !self.doc.mag_enabled();
            let grade = self.doc.doc.get_i32(if techno { "submersiongrade" } else { "initiategrade" }).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.label(format!("{} {grade}", if techno { lang.tr("Submersion Grade") } else { lang.tr("Initiate Grade") }));
                if self.doc.created && (self.doc.mag_enabled() || self.doc.res_enabled()) {
                    ui.checkbox(&mut self.initiation.group, lang.tr("Group"));
                    ui.checkbox(&mut self.initiation.ordeal, lang.tr("Ordeal"));
                    ui.checkbox(&mut self.initiation.schooling, lang.tr("Schooling"));
                    let cost = career::initiation_karma_cost(engine, &self.doc, self.initiation);
                    let label = format!("{} ({cost} {})", if techno { lang.tr("Submerge") } else { lang.tr("Initiate") }, lang.tr("karma"));
                    if ui.add_enabled(self.doc.karma >= cost, egui::Button::new(label)).clicked() {
                        self.action = Some(CareerAction::Initiate(self.initiation));
                    }
                }
            });
        }
        let secs: Vec<Section> = match tab {
            Tab::MartialArts => vec![sections::MARTIAL_ARTS],
            // Spirits sit with spells; a technomancer's sprites with complex forms.
            Tab::Magician => vec![sections::SPELLS, sections::SPIRITS],
            Tab::Adept => vec![sections::POWERS],
            Tab::Technomancer if self.visible(Tab::Magician) => vec![sections::COMPLEX_FORMS],
            Tab::Technomancer => vec![sections::COMPLEX_FORMS, sections::SPIRITS],
            Tab::Critter => vec![sections::CRITTER_POWERS],
            Tab::Initiation => vec![sections::METAMAGICS],
            _ => Vec::new(),
        };
        if matches!(tab, Tab::Magician | Tab::Adept) {
            let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
            changed |= self.magic_editor.shared_ui(ui, &mut self.doc, &cx, status);
        }
        for (i, sec) in secs.into_iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            ui.separator();
            let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
            changed |= self.magic_editor.ui(ui, &mut self.doc, &cx, sec.container, status);
            if sec.container == "spells" && ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("✨")), lang.tr("Create Spell…"))).clicked() {
                self.spell_designer.open = true;
            }
            self.add_buttons(ui, engine, lang, sec.container);
            changed |= self.section(ui, &sec, lang, pdfs, status);
        }
        changed
    }

    /// Final weapon stats (damage with STR, AP, accuracy, dice pool, ranges).
    fn weapon_summary(&self, ui: &mut egui::Ui, lang: &Language) {
        let weapons = self.doc.items("weapons", "weapon");
        if weapons.is_empty() {
            return;
        }
        let rules = self.settings.as_ref().map(chummer_core::items::weapon::WeaponRules::from_settings).unwrap_or_default();
        egui::CollapsingHeader::new(RichText::new(lang.tr("Combat stats")).strong()).id_salt("combat_stats").default_open(true).show(ui, |ui| {
            egui::Grid::new("weapon_stats").striped(true).num_columns(8).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Weapon", "Pool", "Damage", "AP", "Acc", "RC", "Reach", "Ranges"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for w in weapons {
                    let st = chummer_core::items::weapon::stats_with(&self.doc, &self.sheet, Some(&self.store), w, &rules);
                    ui.label(w.get("name"));
                    ui.strong(st.dice_pool.to_string()).on_hover_text(&st.skill);
                    ui.label(&st.damage);
                    ui.label(&st.ap);
                    ui.label(st.accuracy.to_string());
                    ui.label(&st.rc);
                    ui.label(if st.reach != 0 { st.reach.to_string() } else { String::new() });
                    let r = &st.ranges;
                    let bands: Vec<&str> = [&r.short, &r.medium, &r.long, &r.extreme].into_iter().map(String::as_str).filter(|b| !b.is_empty()).collect();
                    ui.label(bands.join(" / "));
                    ui.end_row();
                }
            });
        });
    }

    /// Vehicle totals after mods.
    fn vehicle_summary(&self, ui: &mut egui::Ui, lang: &Language) {
        let vehicles = self.doc.items("vehicles", "vehicle");
        if vehicles.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(RichText::new(lang.tr("Vehicle stats")).strong()).id_salt("vehicle_stats").default_open(true).show(ui, |ui| {
            egui::Grid::new("vehicle_stats").striped(true).num_columns(10).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Vehicle", "Handling", "Speed", "Accel", "Body", "Armor", "Pilot", "Sensor", "Seats", "Slots"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for v in vehicles {
                    let st = chummer_core::items::vehicle::stats(v);
                    ui.label(v.get("name"));
                    ui.label(&st.handling_text);
                    ui.label(&st.speed_text);
                    ui.label(&st.accel_text);
                    ui.label(st.body.to_string());
                    ui.label(st.armor.to_string());
                    ui.label(st.pilot.to_string());
                    ui.label(st.sensor.to_string());
                    ui.label(st.seats.to_string());
                    if st.is_drone {
                        ui.label(format!("{}/{}", st.drone_mod_slots_used, st.drone_mod_slots));
                    } else {
                        ui.label(format!("{}/{}", st.slots_used, st.slots));
                    }
                    ui.end_row();
                }
            });
        });
    }

    /// "Add …" buttons for the kinds that live in a section's container.
    fn add_buttons(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, container: &str) {
        let tags = add_tags(container);
        ui.horizontal(|ui| {
            for t in tags {
                let label = chummer_core::items::kind(t).map_or(*t, |k| k.label);
                if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr_fmt("Add {0}…", &[&kind_noun(lang, label)]))).clicked() {
                    self.open_select(t, engine);
                }
            }
        });
    }

    /// Street Gear with Chummer's sub-tabs.
    fn street_gear_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let tabs: Vec<(usize, String)> = STREET_GEAR.iter().enumerate().map(|(i, (label, _))| (i, lang.tr(label))).collect();
        crate::theme::tab_strip(ui, &mut self.gear_tab, &tabs);
        let mut changed = false;
        egui::ScrollArea::both().id_salt(("gear_page", self.gear_tab)).auto_shrink(false).show(ui, |ui| {
            changed = match STREET_GEAR.get(self.gear_tab).and_then(|(_, s)| *s) {
                Some(sec) => self.gear_page(ui, engine, lang, pdfs, status, sec),
                None => {
                    if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("🧪")), lang.tr("Build custom drug…"))).clicked() {
                        self.drug_builder.open = true;
                    }
                    crate::drug_ui::existing_drugs(ui, &mut self.doc, lang)
                }
            };
        });
        changed
    }

    /// An equipment section with its add buttons and summaries.
    fn gear_page(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status, sec: Section) -> bool {
        let mut changed = false;
        self.add_buttons(ui, engine, lang, sec.container);
        match sec.container {
            "weapons" => self.weapon_summary(ui, lang),
            "vehicles" => self.vehicle_summary(ui, lang),
            "cyberwares" => {
                ui.label(format!("{} {}", lang.tr("Essence:"), format::essence(self.sheet.essence, self.rules.essence_decimals)));
            }
            "lifestyles" => {
                let cx = crate::magic_ui::Ctx { store: &self.store, engine, sheet: &self.sheet, settings: self.settings.as_ref(), lang, pdfs };
                changed |= self.lifestyle_editor.ui(ui, &mut self.doc, &cx, status);
            }
            _ => {}
        }
        changed |= self.section(ui, &sec, lang, pdfs, status);
        changed
    }

    /// A section's items as a tree table (`tree_table`), grouped and nested
    /// like Chummer's tree view. Returns true if the character changed.
    fn section(&mut self, ui: &mut egui::Ui, sec: &Section, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let count = self.doc.doc.child(sec.container).map_or(0, |c| c.children_named(sec.item).count());
        ui.heading(format!("{} ({count})", lang.tr(sec.label)));
        if count == 0 {
            ui.weak(lang.tr("None."));
            return false;
        }
        let tree = chummer_core::tree::section_tree(&self.doc.doc, sec);
        let headers: Vec<String> = sec.columns.iter().map(|c| lang.tr(c.header)).collect();
        let selected = self.item_editor.as_ref().map(|(g, _)| g.as_str());
        let mut remove = None;
        let marks = self.item_marks(lang);
        let out = crate::tree_table::TreeTable::new(sec.container, &headers).selected(selected).show(ui, &tree, |n| tree_row(sec, n, lang, &marks), |ui, n| {
            let Entry::Item { el, top } = n.value else { return };
            pdf_ui::source_icon(ui, pdfs, SourceRef::of(el), status);
            if top && ui.small_button(crate::theme::glyph("🗑")).on_hover_text(lang.tr("Remove (also removes its improvements)")).clicked() {
                remove = Some((sec.container.to_owned(), el.get("guid"), display_name(sec, el, lang)));
            }
        });
        if remove.is_some() {
            self.confirm_remove = remove;
        }
        if let Some(g) = out.clicked {
            self.item_editor = Some((g, crate::item_editor::ItemEditor::default()));
        }
        false
    }

    /// The item detail pane, when an item is selected (see `item_editor`).
    fn item_editor_panel(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, status: &mut Status) -> bool {
        if self.item_editor.is_none() {
            return false;
        }
        let mut changed = false;
        egui::SidePanel::right("item_editor").resizable(true).default_width(300.0).show(ctx, |ui| {
            changed = self.item_pane(ui, engine, lang, status, true);
        });
        changed
    }

    /// The item detail pane's contents; `header` adds the "Item ✖" line
    /// (the Workspace inspector draws its own).
    fn item_pane(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, status: &mut Status, header: bool) -> bool {
        let Some((guid, mut ed)) = self.item_editor.take() else { return false };
        let store = self.store.clone();
        let mut res = crate::item_editor::EditorResult::default();
        let mut close = false;
        if header {
            ui.horizontal(|ui| {
                ui.strong(lang.tr("Item"));
                close = ui.small_button(crate::theme::glyph("✖")).on_hover_text(lang.tr("Close")).clicked();
            });
        }
        egui::ScrollArea::vertical().id_salt("item_pane").show(ui, |ui| res = ed.ui(ui, &mut self.doc, &store, engine, lang, &guid));
        if let Some(s) = res.status.take() {
            *status = Some(s);
        }
        if let Some((tag, parent)) = res.add_child.take() {
            self.open_select(&tag, engine);
            self.select = self.select.take().map(|d| d.with_parent(Some(parent)));
        }
        if let Some(g) = res.select.take() {
            self.item_editor = Some((g, crate::item_editor::ItemEditor::default()));
        } else if !close && !res.removed {
            self.item_editor = Some((guid, ed));
        }
        res.changed
    }

    /// Confirmation dialog for item removal. Returns true if an item went.
    fn confirm_dialog(&mut self, ctx: &egui::Context, lang: &Language) -> bool {
        let Some((container, guid, name)) = self.confirm_remove.clone() else { return false };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm_remove")).show(ctx, |ui| {
            ui.heading(lang.tr("Remove item"));
            ui.label(lang.tr_fmt("Remove {0}? Its improvements are removed too. This cannot be undone.", &[&name]));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Remove")).clicked() {
                    choice = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(yes) => {
                self.confirm_remove = None;
                if yes && container == "qualities" && self.doc.created {
                    // Career mode: buying off a negative quality costs karma.
                    self.action = Some(CareerAction::RemoveQuality(guid));
                    return false;
                }
                yes && self.doc.set(Command::RemoveItem { container, guid })
            }
            None => false,
        }
    }

    /// Creation-mode budgets in the sidebar, with Finish creation.
    fn budget_panel(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let Some(b) = self.budget.clone() else { return false };
        let changed = false;
        let row = |ui: &mut egui::Ui, label: &str, total: i32, used: i32| {
            let left = total - used;
            ui.label(label);
            let t = RichText::new(format!("{left} / {total}"));
            ui.label(if left < 0 { t.color(ui.visuals().error_fg_color) } else if left == 0 { t.weak() } else { t.color(crate::theme::accent(ui)) });
            ui.end_row();
        };
        egui::Grid::new("budget").num_columns(2).striped(true).show(ui, |ui| {
            row(ui, &lang.tr("Karma"), b.karma.0, b.karma.1);
            row(ui, &lang.tr("Attribute Points"), b.attribute_points.0, b.attribute_points.1);
            row(ui, &lang.tr("Special points"), b.special_points.0, b.special_points.1);
            row(ui, &lang.tr("Skill Points"), b.skill_points.0, b.skill_points.1);
            row(ui, &lang.tr("Skill Group Points"), b.skill_group_points.0, b.skill_group_points.1);
            row(ui, &lang.tr("Knowledge Points"), b.knowledge_points.0, b.knowledge_points.1);
            row(ui, &lang.tr("Contact Points"), b.contact_points.0, b.contact_points.1);
            if b.free_spells.0 > 0 {
                row(ui, &lang.tr("Free Spells"), b.free_spells.0, b.free_spells.1);
            }
            ui.label(lang.tr("Positive Qualities"));
            ui.label(format!("{} / {}", b.positive_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label(lang.tr("Negative Qualities"));
            ui.label(format!("{} / {}", b.negative_quality_karma, b.quality_limit));
            ui.end_row();
            ui.label(lang.tr("Nuyen left"));
            let left = b.nuyen_left();
            let t = RichText::new(chummer_core::format::nuyen(left));
            ui.label(if left < 0.0 { t.color(ui.visuals().error_fg_color) } else { t });
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.heading(lang.tr("Issues"));
        self.issue_list(ui, lang);
        ui.add_space(6.0);
        let ok = !self.issues.iter().any(chargen::issues::Issue::is_error);
        let r = ui.add_enabled(ok, crate::theme::primary_button(ui, lang.tr("Finish creation")));
        if r.on_disabled_hover_text(lang.tr("Fix the problems above first")).clicked() {
            self.confirm_finish = true;
        }
        changed
    }

    fn finish_dialog(&mut self, ctx: &egui::Context, lang: &Language) -> bool {
        if !self.confirm_finish {
            return false;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("finish_creation")).show(ctx, |ui| {
            ui.heading(lang.tr("Finish creation?"));
            ui.label(lang.tr("The character switches to career mode. Creation budgets go away; karma and nuyen become plain resources."));
            // Everything still open, as Chummer's "are you sure?" prompts.
            for i in self.issues.iter().filter(|i| i.severity != chargen::issues::Severity::Info) {
                let color = if i.is_error() { ui.visuals().error_fg_color } else { crate::theme::warn(ui) };
                ui.colored_label(color, format!("• {}", issues_ui::message(lang, i)));
            }
            ui.horizontal(|ui| {
                if ui.button(lang.tr("Finish")).clicked() {
                    choice = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.confirm_finish = false;
                self.settings.is_some() && self.doc.set(Command::FinishCreation)
            }
            Some(false) => {
                self.confirm_finish = false;
                false
            }
            None => false,
        }
    }

    /// Open the add dialog for an item kind (see `items::KINDS`).
    fn open_select(&mut self, tag: &str, _engine: &Engine) {
        let books = self.settings.as_ref().map(|s| s.books()).unwrap_or_default();
        let max_avail = self.settings.as_ref().map_or(12, |s| s.max_availability());
        let nuyen_left = self.budget.as_ref().map(|b| b.nuyen_left());
        self.select = SelectDialog::new(tag, &self.store, books, max_avail, nuyen_left);
    }

    fn select_dialog(&mut self, ctx: &egui::Context, lang: &Language, pdfs: &SourcebookLibrary, status: &mut Status) -> bool {
        let Some(dlg) = self.select.as_mut() else { return false };
        let tag = dlg.kind.tag;
        let ch = &self.doc;
        let store_arc = self.store.clone();
        let store = &*store_arc;
        let choices_for = |rec: &chummer_core::xml::Element, p: &chummer_core::items::Purchase| chummer_core::items::choices(tag, ch, store, data::Record(rec), p);
        match dlg.show(ctx, ch, &self.sheet, lang, pdfs, status, &choices_for) {
            select::Outcome::None => false,
            select::Outcome::Cancel => {
                self.select = None;
                false
            }
            select::Outcome::Done { index, purchase } => {
                let Some(rec) = dlg.record(store, index) else { return false };
                let record = RecordRef::of(data::Record(&rec));
                match self.doc.apply(Command::AddItem { tag: tag.to_owned(), record, purchase }) {
                    Ok(r) => {
                        *status = r.message.map(|m| (m, false));
                        self.select = None;
                        true
                    }
                    Err(e) => {
                        *status = Some((e.reason, true));
                        false
                    }
                }
            }
        }
    }

    fn life_module_picker(&mut self, ui: &mut egui::Ui, _engine: &Engine, lang: &Language, status: &mut Status) -> bool {
        let (stages, modules) = chargen::life_modules(&self.store);
        let mut added = false;
        ui.group(|ui| {
            ui.label(RichText::new(lang.tr("Life Modules")).strong());
            ui.horizontal(|ui| {
                if self.life.0.is_empty() {
                    self.life.0 = stages.first().cloned().unwrap_or_default();
                }
                crate::combo::Combo::from_id_salt("lm_stage").selected_text(self.life.0.clone()).show_ui(ui, |ui| {
                    for st in &stages {
                        if crate::combo::selectable_label(ui, self.life.0 == *st, st).clicked() {
                            self.life = (st.clone(), String::new(), String::new());
                        }
                    }
                });
                let in_stage: Vec<&chargen::LifeModule> = modules.iter().filter(|m| m.stage == self.life.0).collect();
                let cur = in_stage.iter().find(|m| m.id == self.life.1).map(|m| format!("{} ({} {})", m.name, m.karma, lang.tr("karma"))).unwrap_or_else(|| lang.tr("Choose a module…"));
                crate::combo::Combo::from_id_salt("lm_module").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                    for m in &in_stage {
                        if crate::combo::selectable_label(ui, self.life.1 == m.id, format!("{} ({} {})", m.name, m.karma, lang.tr("karma"))).clicked() {
                            self.life.1 = m.id.clone();
                            self.life.2 = m.versions.first().map(|v| v.0.clone()).unwrap_or_default();
                        }
                    }
                });
                if let Some(m) = in_stage.iter().find(|m| m.id == self.life.1) {
                    if m.versions.len() > 1 {
                        let cur = m.versions.iter().find(|v| v.0 == self.life.2).map(|v| v.1.clone()).unwrap_or_default();
                        crate::combo::Combo::from_id_salt("lm_version").selected_text(cur).show_ui(ui, |ui| {
                            for (id, n) in &m.versions {
                                crate::combo::selectable_value(ui, &mut self.life.2, id.clone(), n);
                            }
                        });
                    }
                }
                if ui.add_enabled(!self.life.1.is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                    let v = (!self.life.2.is_empty()).then(|| self.life.2.clone());
                    added |= self.doc.run(Command::AddLifeModule { module: self.life.1.clone(), version: v }, status).is_some();
                }
            });
        });
        added
    }

    fn improvements_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let changed = self.custom_improvements.tab(ui, &mut self.doc, &self.store, lang);
        ui.separator();
        let imps = &self.doc.improvements;
        ui.label(lang.tr_fmt(
            "{0} improvements ({1} active). These modifiers come from qualities, ware, powers and gear.",
            &[&imps.list.len(), &imps.active().count()],
        ));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("imps").striped(true).num_columns(7).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Type", "Target", "Value", "Aug", "Min/Max", "Source", "Condition"]) {
                    ui.strong(h);
                }
                ui.end_row();
                for i in &imps.list {
                    let label = if imps.applies(i) { RichText::new(&i.kind) } else { RichText::new(&i.kind).weak() };
                    ui.label(label);
                    ui.label(&i.improved_name);
                    ui.label(fmt_opt(i.val));
                    ui.label(fmt_opt(i.aug));
                    ui.label(if i.min != 0.0 || i.max != 0.0 { format!("{}/{}", i.min, i.max) } else { String::new() });
                    ui.weak(&i.source);
                    ui.weak(&i.condition);
                    ui.end_row();
                }
            });
        });
        changed
    }

    fn run_action(&mut self, a: CareerAction, status: &mut Status) -> bool {
        match self.doc.apply(a.command()) {
            Ok(_) => {
                *status = Some((format!("Done: {}", describe_action(&a)), false));
                true
            }
            Err(e) => {
                *status = Some((e.to_string(), true));
                false
            }
        }
    }

    /// Career calendar: in-game weeks with notes (`CalendarWeek`).
    fn calendar_ui(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        use chummer_core::calendar;
        let mut changed = false;
        ui.vertical(|ui| {
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add Week"))).clicked() {
                changed |= self.doc.set(Command::AddWeek);
            }
            let mut weeks = calendar::weeks(&self.doc);
            weeks.sort_by_key(|w| std::cmp::Reverse((w.year, w.week)));
            let mut remove = None;
            egui::Grid::new("calendar_weeks").striped(true).num_columns(3).show(ui, |ui| {
                for w in &weeks {
                    ui.label(w.label());
                    let mut notes = w.notes.clone();
                    if ui.add(egui::TextEdit::singleline(&mut notes).desired_width(420.0)).changed() {
                        changed |= self.doc.set(Command::SetWeekNotes { week: w.guid.clone(), notes });
                    }
                    if ui.small_button(crate::theme::glyph("🗑")).clicked() {
                        remove = Some(w.guid.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(g) = remove {
                changed |= self.doc.set(Command::RemoveWeek { week: g });
            }
        });
        changed
    }

    fn log_tab(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut changed = false;
        let entries = career::entries(&self.doc);
        let totals = career::totals(&self.doc);
        if self.doc.created {
            let rep = career::reputation_for(engine, &self.doc);
            ui.label(format!(
                "{} {} · {} {} · {} {} · {} {}",
                lang.tr("Career Karma"),
                career::career_karma(&self.doc),
                lang.tr("Street Cred"),
                rep.street_cred,
                lang.tr("Notoriety"),
                rep.notoriety,
                lang.tr("Public Awareness"),
                rep.public_awareness
            ));
            changed |= crate::career_ui::actions_ui(ui, &mut self.doc, engine, lang);
            ui.horizontal(|ui| {
                crate::combo::Combo::from_id_salt("manual_kind").selected_text(if self.manual.0 { lang.tr("Karma") } else { lang.tr("Nuyen") }).show_ui(ui, |ui| {
                    crate::combo::selectable_value(ui, &mut self.manual.0, true, lang.tr("Karma"));
                    crate::combo::selectable_value(ui, &mut self.manual.0, false, lang.tr("Nuyen"));
                });
                ui.add(egui::DragValue::new(&mut self.manual.1).range(0.0..=1_000_000.0).max_decimals(2));
                ui.add(egui::TextEdit::singleline(&mut self.manual.2).hint_text(lang.tr("Reason (e.g. run payout)")).desired_width(240.0));
                let ok = self.manual.1 > 0.0;
                let mut gain = None;
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Gain"))).clicked() {
                    gain = Some(true);
                }
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Spend"))).clicked() {
                    gain = Some(false);
                }
                if let Some(g) = gain {
                    match self.apply_manual(g) {
                        Ok(()) => changed = true,
                        Err(e) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                    }
                }
            });
        }
        ui.label(lang.tr_fmt(
            "{0} entries · karma earned {1} · spent {2} · nuyen earned {3}",
            &[&entries.len(), &totals.career_karma, &totals.karma_spent, &format::nuyen(totals.career_nuyen)],
        ));
        ui.add_space(4.0);
        if entries.is_empty() {
            ui.weak(lang.tr("No entries yet. Career-mode spending and income appear here."));
            return changed;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("log").striped(true).num_columns(5).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in lang.tr_all(["Date", "Type", "Amount", "Reason", ""]) {
                    ui.strong(h);
                }
                ui.end_row();
                for e in entries.iter().rev() {
                    ui.label(e.date.replace('T', " "));
                    let karma = e.kind == career::ExpenseType::Karma;
                    ui.label(if karma { lang.tr("Karma") } else { lang.tr("Nuyen") });
                    let text = if karma { chummer_core::improvement::fmt_num(e.amount) } else { format::nuyen(e.amount) };
                    ui.colored_label(if e.amount < 0.0 { crate::theme::warn(ui) } else { crate::theme::accent(ui) }, text);
                    ui.label(&e.reason);
                    if self.doc.created && e.undo.is_some() {
                        if ui.small_button(lang.tr("Undo")).on_hover_text(lang.tr("Reverse this and refund it")).clicked() {
                            self.action = Some(CareerAction::Undo(e.guid.clone()));
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        });
        changed
    }

    /// The attributes the attribute table lists: the special ones only
    /// when the character has them, Essence never.
    fn shown_attributes(&self) -> Vec<&'static str> {
        attributes::PHYSICAL
            .iter()
            .chain(attributes::MENTAL)
            .chain(attributes::SPECIAL)
            .copied()
            .filter(|n| match *n {
                "ESS" => false,
                "MAG" => self.doc.mag_enabled(),
                "MAGAdept" => self.doc.mag_enabled() && self.doc.is_adept() && self.doc.is_magician(),
                "RES" => self.doc.res_enabled(),
                "DEP" => self.doc.dep_enabled(),
                _ => true,
            })
            .collect()
    }

    /// Log the manual karma or nuyen entry being typed (`self.manual`)
    /// as a gain or an expense, and clear it.
    fn apply_manual(&mut self, gain: bool) -> Result<(), String> {
        let expense = career::ManualExpense { amount: self.manual.1, reason: self.manual.2.clone(), ..Default::default() };
        self.doc.apply(Command::ManualExpense { karma: self.manual.0, gain, expense }).map_err(|e| e.to_string())?;
        self.manual.1 = 0.0;
        self.manual.2.clear();
        Ok(())
    }

    fn notes_tab(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (key, label) in TEXT_FIELDS.iter().filter(|(k, _)| *k == "gamenotes") {
                ui.label(RichText::new(lang.tr(label)).strong());
                let mut v = self.doc.field(key);
                if ui.add(egui::TextEdit::multiline(&mut v).desired_width(f32::INFINITY).desired_rows(24)).changed() {
                    changed |= self.doc.set(Command::SetField { key: (*key).to_owned(), value: v });
                }
                ui.add_space(8.0);
            }
        });
        changed
    }
}

/// The kinds the "Add …" buttons of a section's container offer (select
/// dialog tags).
fn add_tags(container: &str) -> &'static [&'static str] {
    match container {
        "gears" => &["gear"],
        "cyberwares" => &["cyberware", "bioware"],
        "armors" => &["armor", "armormod"],
        "weapons" => &["weapon", "accessory"],
        "vehicles" => &["vehicle", "mod"],
        "lifestyles" => &["lifestyle"],
        "spells" => &[], // magic_ui: spell options and career karma
        "powers" => &["power"],
        "complexforms" => &["complexform"],
        "spirits" => &["spirit"],
        "metamagics" => &[], // magic_ui: one per grade, echoes for technomancers
        "martialarts" => &["martialart"],
        "critterpowers" => &["critterpower"],
        "aiprograms" => &["aiprogram"],
        _ => &[],
    }
}

/// An item kind's label for use inside a sentence ("Add weapon…"). Only
/// English lowercases it; other languages (German nouns) keep their case.
pub fn kind_noun(lang: &Language, label: &str) -> String {
    let t = lang.tr(label);
    if lang.code.starts_with("en") { t.to_lowercase() } else { t }
}

/// Whether a skill passes the Skills tab's filter (name contains the
/// text; with `only_rated`, a rating above 0).
fn skill_matches(filter: &str, only_rated: bool, name: &str, rating: i32) -> bool {
    (filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())) && (!only_rated || rating > 0)
}

/// Creation: the highest base (attribute points) an attribute can take.
fn attribute_base_max(v: &calc::AttributeValues, base: i32, karma: i32) -> i32 {
    (v.total_max - v.total_min - v.free_base - karma).max(base)
}

/// Creation: the highest karma levels an attribute can take.
fn attribute_karma_max(v: &calc::AttributeValues, karma: i32) -> i32 {
    (v.total_max - v.total_base).max(karma)
}

fn fmt_opt(v: f64) -> String {
    if v == 0.0 {
        String::new()
    } else {
        chummer_core::improvement::fmt_num(v)
    }
}

fn display_name(sec: &Section, it: &Element, lang: &Language) -> String {
    let name = it.get("name");
    let id = it.child_text("sourceid").or_else(|| it.child_text("id")).unwrap_or_default();
    let mut shown = if sec.data_file.is_empty() { name.clone() } else { lang.data_name(sec.data_file, &id, &name) };
    for custom in ["gearname", "weaponname", "armorname", "vehiclename", "crittername"] {
        let c = it.get(custom);
        if !c.is_empty() && c != name {
            shown = format!("{shown} (“{c}”)");
        }
    }
    shown
}

fn cell(it: &Element, field: &str) -> String {
    use chummer_core::expr;
    let v = it.get(field);
    let rating = it.get_i32("rating").unwrap_or(0);
    let min_rating = it.get_i32("minrating").unwrap_or(0);
    // Data expressions such as "Rating * 250" are stored as written; show
    // the value at the item's rating. Strings that need a parent item or a
    // vehicle stay as written.
    let eval = |s: &str| -> Option<f64> {
        let s = expr::fixed_values(s.trim(), rating).replace("MinRating", &min_rating.to_string());
        let r = rating.to_string();
        let s = s.replace("{Rating}", &r).replace("Rating", &r);
        if expr::needs_evaluation(&s) { expr::evaluate_num(&s).ok() } else { expr::parse_plain(&s) }
    };
    match field {
        "equipped" | "bound" => if chummer_core::xml::parse_bool(&v) { "✓".into() } else { String::new() },
        "cost" => eval(&v).map(format::nuyen).unwrap_or(v),
        "avail" if !v.trim().is_empty() && !v.contains("Gear") && !v.contains('{') => {
            expr::Availability::parse(&v, rating, min_rating, &expr::NoAttributes).to_string()
        }
        "ess" => eval(&v).map(|e| format!("{e:.2}")).unwrap_or(v),
        _ => v,
    }
}

/// A tree-table row for a section node: a group's label, or an item's
/// name and column cells.
fn tree_row(sec: &Section, n: &chummer_core::tree::ItemNode, lang: &Language, marks: &std::collections::HashMap<String, (String, bool)>) -> crate::tree_table::RowView {
    use chummer_core::tree::Label;
    match &n.value {
        Entry::Group(label) => {
            let text = match label {
                Label::Ui(s) => lang.tr(s),
                Label::Text(s) => s.clone(),
                Label::Grade(g) => format!("{} {g}", lang.tr("Grade")),
            };
            crate::tree_table::RowView { cells: vec![text], group: true, ..Default::default() }
        }
        Entry::Item { el, .. } => {
            let mut cells = vec![display_name(sec, el, lang)];
            cells.extend(sec.columns.iter().skip(1).map(|c| cell(el, c.field)));
            crate::tree_table::RowView { cells, group: false, clickable: chummer_core::items::edit::is_item(el), hover: el.get("notes"), warning: marks.get(&el.get("guid")).cloned() }
        }
    }
}

/// Clickable condition-monitor boxes. Clicking box N sets damage to N, or
/// clears it if N was the last filled box. Classic lays them out like
/// Chummer (rows of `threshold` boxes, the penalty in the last box of a
/// row); Graphite in one wrapped row.
pub(crate) fn cm_track(ui: &mut egui::Ui, id: &str, boxes: i32, threshold: i32, filled: &mut i32, color: egui::Color32) -> bool {
    let theme = crate::theme::current(ui.ctx());
    let p = theme.palette;
    let classic = theme.kind == crate::theme::ThemeKind::Classic;
    let radius = theme.widget_radius.min(3);
    let size = if classic { 20.0 } else { 18.0 };
    let per_row = if classic && threshold > 0 { threshold } else { boxes.max(1) };
    let mut changed = false;
    let mut draw = |ui: &mut egui::Ui, n: i32| {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
        let on = n <= *filled;
        let painter = ui.painter();
        let fill = if on { color } else if resp.hovered() { p.surface_hover } else { p.field };
        painter.rect_filled(rect, radius, fill);
        let edge = if resp.hovered() { p.stroke_focus } else { p.stroke };
        painter.rect_stroke(rect, radius, egui::Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
        if threshold > 0 && n % threshold == 0 {
            let text = if on { p.panel } else { p.weak };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("-{}", n / threshold), egui::FontId::proportional(9.5), text);
        }
        if resp.clicked() {
            *filled = if *filled == n { n - 1 } else { n };
            changed = true;
        }
    };
    ui.push_id(id, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
        if classic {
            let mut n = 1;
            while n <= boxes {
                ui.horizontal(|ui| {
                    for k in n..(n + per_row).min(boxes + 1) {
                        draw(ui, k);
                    }
                });
                n += per_row;
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for n in 1..=boxes.max(0) {
                    draw(ui, n);
                }
            });
        }
    });
    changed
}

fn describe_action(a: &CareerAction) -> String {
    match a {
        CareerAction::RaiseAttribute(n) => format!("raised {n}"),
        CareerAction::RaiseSkill(_) => "raised skill".into(),
        CareerAction::RaiseGroup(g) => format!("raised {g}"),
        CareerAction::Specialize(_, n) => format!("specialized in {n}"),
        CareerAction::LearnKnowledge(n, _) => format!("learned {n}"),
        CareerAction::Undo(_) => "undone".into(),
        CareerAction::Initiate(_) => "initiated".into(),
        CareerAction::RemoveQuality(_) => "quality removed".into(),
    }
}
