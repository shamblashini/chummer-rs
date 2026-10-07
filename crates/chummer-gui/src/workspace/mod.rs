//! The Workspace layout (View → Appearance → Workspace): a top bar with
//! the open documents and the command palette, a sidebar of sections
//! grouped by domain, a budget strip, the page, an inspector on the right
//! and a status bar. Classic (`crate::view`, `crate::main`'s menus and
//! tab pages) stays as it was; this module draws the same documents a
//! different way.
//!
//! # Adding a screen
//!
//! * A **section** is an entry of the sidebar: add a [`Section`] variant,
//!   give it a label and icon ([`Section::label`], [`Section::icon`]),
//!   list it in the sidebar model (`character::ws_nav` for characters,
//!   `shell::home_nav` / `shell::campaign_nav` otherwise) and draw it in
//!   `CharacterView::ws_page` (characters) or the shell's
//!   `section_page`. Until a screen is rebuilt, `Section::Page(tab)`
//!   draws the Classic tab page.
//! * A **panel** that can pop out is drawn with [`popout::Panel`]; give
//!   it a [`PanelId`] and draw the same contents in `shell::popped` so
//!   its window shows them.
//! * **Inspector** sections are `popout::Panel::inspector` panels drawn in
//!   `shell::inspector`.
//! * Widgets in the mockups' style are in [`widgets`], icons in [`icons`],
//!   colours in `crate::theme::ws`.

pub mod icons;
pub mod palette;
pub mod popout;
mod shell;
pub mod widgets;

use std::collections::HashMap;

use eframe::egui;

use crate::theme::Badge;
use crate::view::Tab;

/// An open document: Home (roster and Master Index), the campaign (GM
/// screen), or a character tab (by `CharacterView::ws_id`, which stays
/// the same while the tab is open).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocKey {
    Home,
    Campaign,
    Character(u64),
}

/// A sidebar entry: what the page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    /// Character, career mode: the table view (condition monitor, Edge).
    Play,
    /// Character: a Classic tab page.
    Page(Tab),
    /// Character: a Street Gear sub-tab (Gear, Clothing & Armor, Weapons,
    /// Drugs, Lifestyles).
    Gear(usize),
    /// Character: this session's changes.
    History,
    /// Home: recent characters, the roster, online campaigns.
    Home,
    /// Home: the Master Index.
    DataBrowser,
    /// Campaign: the GM screen.
    Campaign,
}

impl Section {
    /// The sidebar label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            Section::Play => "At the table",
            Section::Page(Tab::Common) => "Attributes & Qualities",
            Section::Page(t) => crate::view::TABS.iter().find(|(x, _)| *x == t).map_or("", |(_, l)| l),
            Section::Gear(i) => crate::view::workspace::gear_label(i),
            Section::History => "History",
            Section::Home => "Home",
            Section::DataBrowser => "Master Index",
            Section::Campaign => "GM Screen",
        }
    }

    pub fn icon(self) -> &'static str {
        use icons::*;
        match self {
            Section::Play => HEARTBEAT,
            Section::Page(t) => match t {
                Tab::Common => USER_CIRCLE,
                Tab::Skills => LIGHTNING,
                Tab::Limits => GRAPH,
                Tab::MartialArts => HAND_FIST,
                Tab::Magician => MAGIC_WAND,
                Tab::Adept => FLAME,
                Tab::Technomancer => CPU,
                Tab::AdvancedPrograms => ROBOT,
                Tab::Critter => PAW_PRINT,
                Tab::Initiation => EYE,
                Tab::Cyberware => BRAIN,
                Tab::StreetGear => PACKAGE,
                Tab::Vehicles => CAR,
                Tab::CharacterInfo => IDENTIFICATION_CARD,
                Tab::Karma => COINS,
                Tab::Calendar => CALENDAR_BLANK,
                Tab::Notes => NOTEBOOK,
                Tab::Improvements => SLIDERS,
                Tab::Relationships => ADDRESS_BOOK,
            },
            Section::Gear(i) => match i {
                0 => PACKAGE,
                1 => T_SHIRT,
                2 => CROSSHAIR,
                3 => PILL,
                _ => HOUSE_LINE,
            },
            Section::History => CLOCK_COUNTER_CLOCKWISE,
            Section::Home => HOUSE,
            Section::DataBrowser => DATABASE,
            Section::Campaign => USERS_THREE,
        }
    }
}

/// A panel that can pop out into its own window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// A sidebar section's page.
    Section(Section),
    /// Inspector: creation issues.
    Issues,
    /// Inspector: the selected item.
    Item,
    /// Inspector: the Karma Summary (creation) or Other Info (career).
    Summary,
    /// Inspector: recent changes.
    Recent,
    /// Play: the condition monitor and Edge.
    Condition,
    Dice,
    Initiative,
    /// The GM screen's activity feed.
    Activity,
}

/// One sidebar entry.
#[derive(Debug, Clone)]
pub struct NavItem {
    pub section: Section,
    /// Translated.
    pub label: String,
    pub badge: Option<Badge>,
}

/// A titled group of sidebar entries ("Build", "Story", "Records").
#[derive(Debug, Clone)]
pub struct NavGroup {
    /// Translated.
    pub title: String,
    pub items: Vec<NavItem>,
}

/// The Workspace's state (in `App`).
#[derive(Default)]
pub struct Workspace {
    pub pops: popout::PopOuts,
    pub palette: palette::Palette,
    /// The logo for the top bar, once loaded.
    logo: Option<egui::TextureHandle>,
    /// Characters showing a section that is not a tab page (Play,
    /// History): the section, and the tab the view was on when it was
    /// picked (going to a tab, e.g. from an issue, leaves the section).
    special: HashMap<u64, (Section, Tab)>,
    /// Home's section (Home or the Master Index).
    home: Option<Section>,
}

/// The app icon (the logo at 256px), for the window and the pop-outs.
pub fn app_icon() -> Option<std::sync::Arc<egui::IconData>> {
    use std::sync::OnceLock;
    static ICON: OnceLock<Option<std::sync::Arc<egui::IconData>>> = OnceLock::new();
    ICON.get_or_init(|| {
        let img = image::load_from_memory(include_bytes!("../../assets/logo/chummer-rs-256.png")).ok()?.into_rgba8();
        let (width, height) = img.dimensions();
        Some(std::sync::Arc::new(egui::IconData { rgba: img.into_raw(), width, height }))
    })
    .clone()
}
