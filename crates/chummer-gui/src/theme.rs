//! Look of the GUI: two layouts (see [`Layout`]) and their themes.
//!
//! The Classic layout is Chummer5a's (menu, toolbar, tab pages) with two
//! themes:
//!
//! * Classic imitates Chummer5a's WinForms look: SystemColors.Control
//!   greys, white input fields, square corners, Windows-blue selection and
//!   a Segoe-UI-like font (Selawik).
//! * Graphite is the dark "pro tool" style: neutral greys, one teal accent,
//!   IBM Plex Sans and Plex Mono.
//!
//! The Workspace layout (`crate::workspace`) has a dark and a light theme
//! whose colours come from the chummer-rs logo (#151515, #738FFF,
//! #B3C2FF); see [`WsPalette`].
//!
//! The active [`Theme`] lives in the egui context (see [`apply`]); widgets
//! read their colours with [`palette`] (or [`ws`] for Workspace roles) or
//! the small helpers ([`accent`], [`warn`], ...) so switching themes
//! recolours everything. The choice is kept in
//! `$XDG_CONFIG_HOME/chummer-rs/gui.ini` (see [`Appearance`]).

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    Classic,
    #[default]
    Graphite,
    /// The Workspace layout, dark.
    WorkspaceDark,
    /// The Workspace layout, light.
    WorkspaceLight,
}

/// Where things are: Chummer5a's menu, toolbar and tab pages, or the
/// Workspace shell (sidebar, inspector, command palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Classic,
    Workspace,
}

impl Layout {
    pub const ALL: [Layout; 2] = [Layout::Classic, Layout::Workspace];

    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Classic => "classic",
            Layout::Workspace => "workspace",
        }
    }

    pub fn parse(s: &str) -> Option<Layout> {
        Layout::ALL.into_iter().find(|l| l.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// Menu label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            Layout::Classic => "Classic",
            Layout::Workspace => "Workspace",
        }
    }
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 4] = [ThemeKind::Classic, ThemeKind::Graphite, ThemeKind::WorkspaceDark, ThemeKind::WorkspaceLight];
    /// The Classic layout's themes.
    pub const CLASSIC: [ThemeKind; 2] = [ThemeKind::Classic, ThemeKind::Graphite];
    /// The Workspace layout's themes.
    pub const WORKSPACE: [ThemeKind; 2] = [ThemeKind::WorkspaceDark, ThemeKind::WorkspaceLight];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeKind::Classic => "classic",
            ThemeKind::Graphite => "graphite",
            ThemeKind::WorkspaceDark => "dark",
            ThemeKind::WorkspaceLight => "light",
        }
    }

    /// `classic`, `graphite`, `dark` or `light` (also `workspace-dark`,
    /// `workspace-light`).
    pub fn parse(s: &str) -> Option<ThemeKind> {
        let s = s.trim();
        let s = s.strip_prefix("workspace-").unwrap_or(s);
        ThemeKind::ALL.into_iter().find(|k| k.as_str().eq_ignore_ascii_case(s))
    }

    /// Menu label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Classic => "Classic",
            ThemeKind::Graphite => "Graphite",
            ThemeKind::WorkspaceDark => "Dark",
            ThemeKind::WorkspaceLight => "Light",
        }
    }

    pub fn layout(self) -> Layout {
        match self {
            ThemeKind::Classic | ThemeKind::Graphite => Layout::Classic,
            ThemeKind::WorkspaceDark | ThemeKind::WorkspaceLight => Layout::Workspace,
        }
    }

    /// The WinForms look (square tabs, warning triangles); every other
    /// theme draws like Graphite.
    pub fn is_classic(self) -> bool {
        self == ThemeKind::Classic
    }
}

/// The saved look: the layout, and the last theme picked for each layout
/// so switching layouts brings it back. gui.ini keeps them as `layout=`,
/// `theme=` (Classic layout; as before Workspace existed) and
/// `workspace=` (dark or light).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub layout: Layout,
    /// Classic or Graphite.
    pub classic: ThemeKind,
    /// WorkspaceDark or WorkspaceLight.
    pub workspace: ThemeKind,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { layout: Layout::Classic, classic: ThemeKind::default(), workspace: ThemeKind::WorkspaceDark }
    }
}

impl Appearance {
    /// The theme in use.
    pub fn kind(&self) -> ThemeKind {
        match self.layout {
            Layout::Classic => self.classic,
            Layout::Workspace => self.workspace,
        }
    }

    /// Use `kind`, switching to its layout.
    pub fn with_kind(mut self, kind: ThemeKind) -> Appearance {
        self.layout = kind.layout();
        match self.layout {
            Layout::Classic => self.classic = kind,
            Layout::Workspace => self.workspace = kind,
        }
        self
    }

    pub fn with_layout(mut self, layout: Layout) -> Appearance {
        self.layout = layout;
        self
    }

    /// The appearance saved in a gui.ini text.
    pub fn from_config(text: &str) -> Appearance {
        let d = Appearance::default();
        let pick = |key: &str, layout: Layout, fallback: ThemeKind| config_get(text, key).and_then(|v| ThemeKind::parse(&v)).filter(|k| k.layout() == layout).unwrap_or(fallback);
        Appearance {
            layout: config_get(text, "layout").and_then(|v| Layout::parse(&v)).unwrap_or(d.layout),
            classic: pick("theme", Layout::Classic, d.classic),
            workspace: pick("workspace", Layout::Workspace, d.workspace),
        }
    }

    /// `text` with this appearance's lines set; other lines are kept.
    pub fn to_config(self, text: &str) -> String {
        let s = config_set(text, "layout", self.layout.as_str());
        let s = config_set(&s, "theme", self.classic.as_str());
        config_set(&s, "workspace", self.workspace.as_str())
    }
}

/// Colour roles. Text roles (`text`, `weak`, `accent`, `warning`,
/// `physical`, `stun`, `good`, `bad`) stay readable (4.5:1) on `panel`,
/// `window` and `field`; see the contrast test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Panels and tab pages.
    pub panel: Color32,
    /// Windows, menus and pop-ups.
    pub window: Color32,
    /// Text fields and other "sunken" backgrounds.
    pub field: Color32,
    /// Button faces.
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    /// Frame and separator lines.
    pub stroke: Color32,
    /// Hovered or focused widget outlines.
    pub stroke_focus: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    /// Text drawn on an `accent` fill.
    pub on_accent: Color32,
    /// Fill of the main action of a form ([`primary_button`]); `accent`
    /// is for text.
    pub primary: Color32,
    /// Text drawn on a `primary` fill.
    pub on_primary: Color32,
    pub selection: Color32,
    pub selection_text: Color32,
    /// Alternate table rows.
    pub stripe: Color32,
    pub physical: Color32,
    pub stun: Color32,
    pub edge: Color32,
    pub matrix: Color32,
    pub warning: Color32,
    pub good: Color32,
    pub bad: Color32,
}

/// The Workspace roles, named as in the mockups. Text roles (`text`,
/// `muted`, `accent`, `physical`, `stun`, `warning`, `error`) stay readable
/// (4.5:1) on `ground`, `chrome`, `raised` and `well`; see the contrast
/// test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WsPalette {
    /// The page behind the content.
    pub ground: Color32,
    /// Top bar, sidebar, inspector and status bar.
    pub chrome: Color32,
    /// Cards, buttons, menus and dialogs.
    pub raised: Color32,
    /// Inputs and other sunken fields.
    pub well: Color32,
    /// Selected rows and the current sidebar entry.
    pub selection: Color32,
    /// Lines between areas and around cards.
    pub divider: Color32,
    /// Borders of inputs, toggles and empty boxes.
    pub control: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Filled buttons, the current-entry bar, filled Edge boxes.
    pub primary: Color32,
    pub on_primary: Color32,
    /// Accent text (values, links, the sync state).
    pub accent: Color32,
    pub physical: Color32,
    pub stun: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// Text on a filled badge or damage box.
    pub on_badge: Color32,
    /// Hovered rows and buttons.
    pub hover: Color32,
}

impl WsPalette {
    pub const DARK: WsPalette = WsPalette {
        ground: hex(0x1A1C22),
        chrome: hex(0x15161B),
        raised: hex(0x21242C),
        well: hex(0x101115),
        selection: hex(0x252C48),
        divider: hex(0x2D313C),
        control: hex(0x4A5065),
        text: hex(0xECEEF6),
        muted: hex(0xA3A9BD),
        primary: hex(0x738FFF),
        on_primary: hex(0x0F1220),
        accent: hex(0xB3C2FF),
        physical: hex(0xF2727C),
        stun: hex(0x5FCFDC),
        warning: hex(0xE8B55C),
        error: hex(0xFF7B84),
        on_badge: hex(0x121318),
        hover: hex(0x262A34),
    };

    /// The light mockup (A-light); physical, stun and warning text is a
    /// shade darker than drawn there so it reaches 4.5:1 on every
    /// background.
    pub const LIGHT: WsPalette = WsPalette {
        ground: hex(0xF6F7FB),
        chrome: hex(0xEEF0F6),
        raised: hex(0xFFFFFF),
        well: hex(0xFFFFFF),
        selection: hex(0xE3E8FF),
        divider: hex(0xD7DBE7),
        control: hex(0xA9B0C3),
        text: hex(0x16171D),
        muted: hex(0x535A6E),
        primary: hex(0x738FFF),
        on_primary: hex(0x0F1220),
        accent: hex(0x3A54CF),
        physical: hex(0xC8303F),
        stun: hex(0x097783),
        warning: hex(0x8F5B00),
        error: hex(0xC42B3A),
        on_badge: hex(0xFFFFFF),
        hover: hex(0xE6E9F2),
    };

    /// Workspace roles for a Classic-layout theme, so Workspace widgets
    /// drawn there still fit.
    fn from_palette(p: &Palette) -> WsPalette {
        WsPalette {
            ground: p.panel,
            chrome: p.panel,
            raised: p.window,
            well: p.field,
            selection: p.selection,
            divider: p.stroke,
            control: p.stroke,
            text: p.text,
            muted: p.weak,
            primary: p.primary,
            on_primary: p.on_primary,
            accent: p.accent,
            physical: p.physical,
            stun: p.stun,
            warning: p.warning,
            error: p.bad,
            on_badge: p.on_primary,
            hover: p.surface_hover,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub kind: ThemeKind,
    pub palette: Palette,
    /// Workspace roles (derived from `palette` for Classic and Graphite).
    pub ws: WsPalette,
    /// Corner radius of buttons, fields and condition boxes.
    pub widget_radius: u8,
    /// Corner radius of windows, menus and group frames.
    pub frame_radius: u8,
    pub item_spacing: egui::Vec2,
    pub button_padding: egui::Vec2,
    pub stroke_width: f32,
    /// Body text size; headings and small text derive from it.
    pub body_size: f32,
    pub heading_size: f32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    pub fn of(kind: ThemeKind) -> Theme {
        match kind {
            ThemeKind::Classic => Theme::classic(),
            ThemeKind::Graphite => Theme::graphite(),
            ThemeKind::WorkspaceDark => Theme::workspace(false),
            ThemeKind::WorkspaceLight => Theme::workspace(true),
        }
    }

    fn with_ws(mut self) -> Theme {
        self.ws = WsPalette::from_palette(&self.palette);
        self
    }

    /// The Workspace layout's theme (from the owner's approved mockups):
    /// compact (13px body, 26px controls), IBM Plex, 5px corners.
    pub fn workspace(light: bool) -> Theme {
        let w = if light { WsPalette::LIGHT } else { WsPalette::DARK };
        Theme {
            kind: if light { ThemeKind::WorkspaceLight } else { ThemeKind::WorkspaceDark },
            palette: Palette {
                panel: w.ground,
                window: w.raised,
                field: w.well,
                surface: w.raised,
                surface_hover: w.hover,
                surface_active: w.selection,
                stroke: w.divider,
                stroke_focus: w.primary,
                text: w.text,
                weak: w.muted,
                accent: w.accent,
                on_accent: w.ground,
                primary: w.primary,
                on_primary: w.on_primary,
                selection: w.selection,
                selection_text: w.accent,
                stripe: if light { hex(0xF0F2F8) } else { hex(0x1E2027) },
                physical: w.physical,
                stun: w.stun,
                edge: w.primary,
                matrix: if light { hex(0x1E7A4C) } else { hex(0x7FD1A8) },
                warning: w.warning,
                good: if light { hex(0x1E7A4C) } else { hex(0x7FD1A8) },
                bad: w.error,
            },
            ws: w,
            widget_radius: 5,
            frame_radius: 7,
            item_spacing: egui::vec2(6.0, 4.0),
            button_padding: egui::vec2(10.0, 3.0),
            stroke_width: 1.0,
            body_size: 13.0,
            heading_size: 17.0,
        }
    }

    /// Chummer5a on Windows 10: SystemColors.Control and friends.
    pub fn classic() -> Theme {
        Theme {
            kind: ThemeKind::Classic,
            palette: Palette {
                panel: hex(0xF0F0F0),
                window: hex(0xF0F0F0),
                field: hex(0xFFFFFF),
                surface: hex(0xE1E1E1),
                surface_hover: hex(0xE5F1FB),
                surface_active: hex(0xCCE4F7),
                stroke: hex(0xADADAD),
                stroke_focus: hex(0x0078D7),
                text: hex(0x000000),
                weak: hex(0x5F5F5F),
                accent: hex(0x0063B1),
                on_accent: hex(0xFFFFFF),
                primary: hex(0x0063B1),
                on_primary: hex(0xFFFFFF),
                selection: hex(0x0072CE),
                selection_text: hex(0xFFFFFF),
                stripe: hex(0xFAFAFA),
                physical: hex(0xC42B1C),
                stun: hex(0x0063B1),
                edge: hex(0x8A5A00),
                matrix: hex(0x0F7B0F),
                warning: hex(0x9D5D00),
                good: hex(0x107C10),
                bad: hex(0xC42B1C),
            },
            ws: WsPalette::DARK,
            widget_radius: 0,
            frame_radius: 0,
            item_spacing: egui::vec2(6.0, 4.0),
            button_padding: egui::vec2(6.0, 2.0),
            stroke_width: 1.0,
            body_size: 12.0,
            heading_size: 14.0,
        }
        .with_ws()
    }

    /// The "Graphite" direction of the style study.
    pub fn graphite() -> Theme {
        Theme {
            kind: ThemeKind::Graphite,
            palette: Palette {
                panel: hex(0x191C20),
                window: hex(0x21252B),
                field: hex(0x121417),
                surface: hex(0x2A2F36),
                surface_hover: hex(0x323840),
                surface_active: hex(0x163430),
                stroke: hex(0x3A414A),
                stroke_focus: hex(0x3FCFB3),
                text: hex(0xE6E8EB),
                weak: hex(0x9BA4AE),
                accent: hex(0x3FCFB3),
                on_accent: hex(0x04201A),
                primary: hex(0x3FCFB3),
                on_primary: hex(0x04201A),
                selection: hex(0x163430),
                selection_text: hex(0x3FCFB3),
                stripe: hex(0x1E2227),
                physical: hex(0xF0767B),
                stun: hex(0x79ACF2),
                edge: hex(0xE6B45A),
                matrix: hex(0x7FD1A8),
                warning: hex(0xE6B45A),
                good: hex(0x7FD1A8),
                bad: hex(0xF0767B),
            },
            ws: WsPalette::DARK,
            widget_radius: 4,
            frame_radius: 6,
            item_spacing: egui::vec2(8.0, 6.0),
            button_padding: egui::vec2(12.0, 6.0),
            stroke_width: 1.0,
            body_size: 13.0,
            heading_size: 17.0,
        }
        .with_ws()
    }

    pub fn dark(&self) -> bool {
        matches!(self.kind, ThemeKind::Graphite | ThemeKind::WorkspaceDark)
    }

    pub fn workspace_layout(&self) -> bool {
        self.kind.layout() == Layout::Workspace
    }

    pub fn visuals(&self) -> egui::Visuals {
        let p = &self.palette;
        let mut v = if self.dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
        let wr = CornerRadius::same(self.widget_radius);
        let fr = CornerRadius::same(self.frame_radius);
        let sw = self.stroke_width;
        v.override_text_color = None;
        v.weak_text_color = Some(p.weak);
        v.hyperlink_color = p.accent;
        v.faint_bg_color = p.stripe;
        v.extreme_bg_color = p.field;
        v.text_edit_bg_color = Some(p.field);
        v.code_bg_color = p.field;
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.bad;
        v.panel_fill = p.panel;
        v.window_fill = p.window;
        v.window_stroke = Stroke::new(sw, p.stroke);
        v.window_corner_radius = fr;
        v.menu_corner_radius = fr;
        let shadow = |blur: u8, alpha: u8| egui::epaint::Shadow { offset: [0, 2], blur, spread: 0, color: Color32::from_black_alpha(alpha) };
        v.window_shadow = if self.dark() { shadow(16, 110) } else { shadow(8, 40) };
        v.popup_shadow = if self.dark() { shadow(10, 90) } else { shadow(4, 40) };
        v.selection.bg_fill = p.selection;
        v.selection.stroke = Stroke::new(sw, p.selection_text);
        v.striped = false;
        v.indent_has_left_vline = self.dark();
        v.slider_trailing_fill = true;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.panel;
        w.noninteractive.weak_bg_fill = p.panel;
        w.noninteractive.bg_stroke = Stroke::new(sw, p.stroke);
        w.noninteractive.fg_stroke = Stroke::new(sw, p.text);
        w.noninteractive.corner_radius = wr;
        // bg_fill is the inside of check boxes, sliders and drag values.
        w.inactive.bg_fill = p.field;
        w.inactive.weak_bg_fill = p.surface;
        w.inactive.bg_stroke = Stroke::new(sw, p.stroke);
        w.inactive.fg_stroke = Stroke::new(sw, p.text);
        w.inactive.corner_radius = wr;
        w.inactive.expansion = 0.0;
        w.hovered.bg_fill = p.surface_hover;
        w.hovered.weak_bg_fill = p.surface_hover;
        w.hovered.bg_stroke = Stroke::new(sw, p.stroke_focus);
        w.hovered.fg_stroke = Stroke::new(sw, p.text);
        w.hovered.corner_radius = wr;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = p.surface_active;
        w.active.weak_bg_fill = p.surface_active;
        w.active.bg_stroke = Stroke::new(sw, p.stroke_focus);
        w.active.fg_stroke = Stroke::new(sw, p.text);
        w.active.corner_radius = wr;
        w.active.expansion = 0.0;
        w.open.bg_fill = p.field;
        w.open.weak_bg_fill = p.surface;
        w.open.bg_stroke = Stroke::new(sw, p.stroke_focus);
        w.open.fg_stroke = Stroke::new(sw, p.text);
        w.open.corner_radius = wr;
        if self.workspace_layout() {
            // Inputs, toggles and buttons have the stronger `control`
            // border of the mockups; dividers stay `divider`.
            let ws = &self.ws;
            v.widgets.inactive.bg_stroke = Stroke::new(sw, ws.control);
            v.window_stroke = Stroke::new(sw, ws.control);
            let shadow = |y: i8, blur: u8, alpha: u8| egui::epaint::Shadow { offset: [0, y], blur, spread: 0, color: Color32::from_black_alpha(alpha) };
            v.window_shadow = if self.dark() { shadow(12, 32, 115) } else { shadow(8, 24, 40) };
            v.popup_shadow = if self.dark() { shadow(6, 16, 90) } else { shadow(4, 12, 32) };
            v.menu_corner_radius = CornerRadius::same(self.widget_radius + 1);
            v.window_corner_radius = CornerRadius::same(8);
            v.indent_has_left_vline = false;
        }
        v
    }

    pub fn style(&self) -> egui::Style {
        let mut s = egui::Style { visuals: self.visuals(), ..Default::default() };
        s.spacing.item_spacing = self.item_spacing;
        s.spacing.button_padding = self.button_padding;
        let roomy = self.kind != ThemeKind::Classic;
        s.spacing.interact_size.y = if self.workspace_layout() { 26.0 } else if roomy { 22.0 } else { 20.0 };
        s.spacing.menu_margin = egui::Margin::same(if roomy { 6 } else { 3 });
        s.spacing.window_margin = egui::Margin::same(if roomy { 10 } else { 6 });
        s.spacing.indent = if roomy { 16.0 } else { 14.0 };
        let body = self.body_size;
        let small = if self.workspace_layout() { body - 1.5 } else { body - 2.0 };
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(small)),
            (TextStyle::Body, FontId::proportional(body)),
            (TextStyle::Button, FontId::proportional(body)),
            (TextStyle::Monospace, FontId::monospace(body)),
            (TextStyle::Heading, FontId::new(self.heading_size, FontFamily::Name(BOLD.into()))),
        ]
        .into();
        s
    }

    /// Bundled fonts in front of egui's defaults, which stay as fallbacks
    /// for symbols (➕ 🗑 🎲 ...) and scripts our fonts lack.
    pub fn fonts(&self) -> FontDefinitions {
        let mut f = FontDefinitions::default();
        let mut add = |name: &str, bytes: &'static [u8]| {
            f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        };
        let (regular, bold, mono): (&str, &str, Option<&str>) = match self.kind {
            ThemeKind::Classic => {
                add("Selawik", include_bytes!("../assets/fonts/Selawik-Regular.ttf"));
                add("Selawik Bold", include_bytes!("../assets/fonts/Selawik-Bold.ttf"));
                ("Selawik", "Selawik Bold", None)
            }
            ThemeKind::Graphite | ThemeKind::WorkspaceDark | ThemeKind::WorkspaceLight => {
                add("IBM Plex Sans", include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"));
                add("IBM Plex Sans SemiBold", include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"));
                add("IBM Plex Mono", include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"));
                ("IBM Plex Sans", "IBM Plex Sans SemiBold", Some("IBM Plex Mono"))
            }
        };
        let defaults = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        f.families.entry(FontFamily::Proportional).or_default().insert(0, regular.to_owned());
        if let Some(m) = mono {
            f.families.entry(FontFamily::Monospace).or_default().insert(0, m.to_owned());
        }
        let mut b = vec![bold.to_owned(), regular.to_owned()];
        b.extend(defaults);
        f.families.insert(FontFamily::Name(BOLD.into()), b);
        // Phosphor icons (`crate::workspace::icons`). Workspace puts them
        // right after the text font, ahead of egui's emoji fonts; the
        // Classic layout's themes only get them as a last fallback, so
        // their glyphs stay as they were.
        f.font_data.insert(PHOSPHOR.to_owned(), Arc::new(egui_phosphor::Variant::Regular.font_data()));
        let at = |list: &Vec<String>| if self.workspace_layout() { 1.min(list.len()) } else { list.len() };
        for family in [FontFamily::Proportional, FontFamily::Name(BOLD.into())] {
            let list = f.families.entry(family).or_default();
            let i = at(list);
            list.insert(i, PHOSPHOR.to_owned());
        }
        f
    }
}

/// Font family name of the bold/semibold face (headings, [`strong`]).
pub const BOLD: &str = "bold";

/// Font data name of the Phosphor icon font.
pub const PHOSPHOR: &str = "phosphor";

fn theme_id() -> egui::Id {
    egui::Id::new("chummer-rs-theme")
}

/// Whether [`glyph`] gives Phosphor icons (the Workspace layout).
static PHOSPHOR_GLYPHS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// An icon of the Classic views: the emoji itself in the Classic layout,
/// the matching Phosphor icon in the Workspace layout (which uses no
/// emoji).
pub fn glyph(emoji: &'static str) -> &'static str {
    if !PHOSPHOR_GLYPHS.load(std::sync::atomic::Ordering::Relaxed) {
        return emoji;
    }
    phosphor_for(emoji)
}

/// The Phosphor icon standing in for an emoji (the emoji when none does).
fn phosphor_for(emoji: &'static str) -> &'static str {
    use egui_phosphor::regular as ph;
    match emoji {
        "➕" => ph::PLUS,
        "🗑" => ph::TRASH,
        "📖" => ph::BOOK_OPEN,
        "🎲" => ph::DICE_FIVE,
        "✖" => ph::X,
        "✨" => ph::SPARKLE,
        "📂" => ph::FOLDER_OPEN,
        "📝" => ph::NOTE_PENCIL,
        "🔍" => ph::MAGNIFYING_GLASS,
        "💾" => ph::FLOPPY_DISK,
        "⟲" => ph::ARROW_COUNTER_CLOCKWISE,
        "⟳" => ph::ARROWS_CLOCKWISE,
        "✏" => ph::PENCIL_SIMPLE,
        "🔗" => ph::LINK,
        "📎" => ph::PAPERCLIP,
        "📁" => ph::FOLDER,
        "🔥" => ph::FIRE,
        "🧪" => ph::FLASK,
        "☰" => ph::DOTS_SIX_VERTICAL,
        "⚠" => ph::WARNING,
        "✔" => ph::CHECK,
        "🎭" => ph::MASK_HAPPY,
        other => other,
    }
}

/// Make `theme` the active one: style for both egui themes (so a system
/// light/dark switch changes nothing), fonts, and the copy in ctx data.
pub fn apply(ctx: &egui::Context, theme: &Theme) {
    PHOSPHOR_GLYPHS.store(theme.workspace_layout(), std::sync::atomic::Ordering::Relaxed);
    ctx.set_theme(if theme.dark() { egui::Theme::Dark } else { egui::Theme::Light });
    let style = Arc::new(theme.style());
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_fonts(theme.fonts());
    ctx.data_mut(|d| d.insert_temp(theme_id(), *theme));
}

/// The active theme (Graphite before [`apply`] ran).
pub fn current(ctx: &egui::Context) -> Theme {
    ctx.data(|d| d.get_temp(theme_id())).unwrap_or_else(Theme::graphite)
}

pub fn palette(ui: &egui::Ui) -> Palette {
    current(ui.ctx()).palette
}

/// The Workspace roles of the active theme.
pub fn ws(ui: &egui::Ui) -> WsPalette {
    current(ui.ctx()).ws
}

pub fn accent(ui: &egui::Ui) -> Color32 {
    palette(ui).accent
}

pub fn warn(ui: &egui::Ui) -> Color32 {
    palette(ui).warning
}

/// Bold text in the theme's bold face.
pub fn strong(ui: &egui::Ui, text: impl Into<String>) -> egui::RichText {
    let size = current(ui.ctx()).body_size;
    egui::RichText::new(text).font(FontId::new(size, FontFamily::Name(BOLD.into())))
}

/// The main action of a form ("Finish creation", "Create character"):
/// an accent-filled button in Graphite, a Windows default button (blue
/// outline) in Classic.
pub fn primary_button(ui: &egui::Ui, text: impl Into<String>) -> egui::Button<'static> {
    let t = current(ui.ctx());
    let p = t.palette;
    if t.kind.is_classic() {
        egui::Button::new(egui::RichText::new(text)).stroke(Stroke::new(1.0_f32, p.stroke_focus))
    } else {
        egui::Button::new(strong(ui, text).color(p.on_primary)).fill(p.primary).stroke(Stroke::new(1.0_f32, p.primary))
    }
}

/// A dice pool as a clickable chip (Graphite) or bold number (Classic).
pub fn pool_chip(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let t = current(ui.ctx());
    let p = t.palette;
    match t.kind {
        ThemeKind::Graphite | ThemeKind::WorkspaceDark | ThemeKind::WorkspaceLight => {
            let b = egui::Button::new(egui::RichText::new(text).monospace().color(p.accent))
                .fill(p.selection)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(3))
                .min_size(egui::vec2(30.0, 0.0));
            ui.scope(|ui| {
                ui.spacing_mut().button_padding = egui::vec2(6.0, 1.0);
                ui.add(b)
            })
            .inner
        }
        ThemeKind::Classic => ui.add(egui::Button::new(strong(ui, text)).frame(false)),
    }
}

/// One tab of a tab strip, drawn like a WinForms tab (Classic) or an
/// underlined text tab (Graphite). `close` adds a "×" whose click is
/// reported separately.
pub fn tab(ui: &mut egui::Ui, selected: bool, label: &str, close: bool) -> (egui::Response, bool) {
    tab_with(ui, selected, label, close, TabDeco::default())
}

/// Extra marks on a tab: an issue badge, and whether it is de-emphasised
/// (still clickable) because the guided-creation step is elsewhere.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TabDeco {
    pub badge: Option<Badge>,
    pub dim: bool,
}

/// How many problems a tab (or row) has; `error` when any blocks
/// finishing creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub count: usize,
    pub error: bool,
}

/// Colour of a warning mark: Classic's yellow WinForms warning sign or a
/// red error sign; Graphite's warning or bad text colour.
fn mark_color(t: &Theme, error: bool) -> Color32 {
    match (t.kind, error) {
        (ThemeKind::Classic, false) => Color32::from_rgb(0xFF, 0xCC, 0x00),
        (ThemeKind::Classic, true) => Color32::from_rgb(0xD0, 0x21, 0x21),
        (_, false) => t.palette.warning,
        (_, true) => t.palette.bad,
    }
}

/// Paint the warning mark centred in `rect`: Classic a small warning
/// triangle with "!", Graphite a dot.
pub fn paint_mark(painter: &egui::Painter, rect: egui::Rect, t: &Theme, error: bool) {
    let c = rect.center();
    let fill = mark_color(t, error);
    match t.kind {
        ThemeKind::Classic => {
            let h = rect.height().min(rect.width()).min(13.0);
            let pts = vec![egui::pos2(c.x, c.y - h / 2.0), egui::pos2(c.x + h / 2.0 + 1.0, c.y + h / 2.0), egui::pos2(c.x - h / 2.0 - 1.0, c.y + h / 2.0)];
            painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0_f32, Color32::from_rgb(0x6B, 0x55, 0x00))));
            let ink = if error { Color32::WHITE } else { Color32::BLACK };
            painter.line_segment([egui::pos2(c.x, c.y - h / 2.0 + 4.0), egui::pos2(c.x, c.y + h / 2.0 - 4.0)], Stroke::new(1.5_f32, ink));
            painter.circle_filled(egui::pos2(c.x, c.y + h / 2.0 - 2.0), 0.9, ink);
        }
        ThemeKind::Graphite | ThemeKind::WorkspaceDark | ThemeKind::WorkspaceLight => {
            painter.circle_filled(c, 3.5, fill);
        }
    }
}

/// A warning mark as a widget, for rows of tables and lists.
pub fn warning_mark(ui: &mut egui::Ui, error: bool) -> egui::Response {
    let t = current(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_mark(ui.painter(), rect, &t, error);
    }
    resp
}

/// [`tab`] with a badge and de-emphasis.
pub fn tab_with(ui: &mut egui::Ui, selected: bool, label: &str, close: bool, deco: TabDeco) -> (egui::Response, bool) {
    let t = current(ui.ctx());
    let p = t.palette;
    let classic = t.kind == ThemeKind::Classic;
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let pad = if classic { egui::vec2(6.0, 3.0) } else { egui::vec2(9.0, 6.0) };
    let close_w = if close { font.size + 4.0 } else { 0.0 };
    let badge_font = FontId::proportional((font.size - 2.0).max(9.0));
    let badge_galley = deco.badge.map(|b| ui.painter().layout_no_wrap(b.count.to_string(), badge_font.clone(), Color32::PLACEHOLDER));
    let badge_w = badge_galley.as_ref().map_or(0.0, |g| 14.0 + g.size().x + 3.0);
    let size = egui::vec2(galley.size().x + 2.0 * pad.x + close_w + badge_w, galley.size().y + 2.0 * pad.y);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let close_rect = egui::Rect::from_min_max(egui::pos2(rect.max.x - close_w - pad.x * 0.5, rect.min.y), egui::pos2(rect.max.x - pad.x * 0.5, rect.max.y));
    let over_close = close && resp.hover_pos().is_some_and(|pos| close_rect.contains(pos));
    let closed = close && resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| close_rect.contains(pos));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = resp.hovered();
        let text_color = if classic {
            // Selected tab: lighter and 2px taller, open towards the page.
            let r = if selected { rect } else { rect.shrink2(egui::vec2(0.0, 1.0)).translate(egui::vec2(0.0, 1.0)) };
            let fill = if selected { p.field } else if hovered { p.surface_hover } else { p.panel };
            painter.rect_filled(r, CornerRadius::ZERO, fill);
            let s = Stroke::new(1.0_f32, if hovered && !selected { p.stroke_focus } else { p.stroke });
            painter.line_segment([r.left_bottom(), r.left_top()], s);
            painter.line_segment([r.left_top(), r.right_top()], s);
            painter.line_segment([r.right_top(), r.right_bottom()], s);
            p.text
        } else {
            if hovered && !selected {
                painter.rect_filled(rect, CornerRadius::same(t.widget_radius), p.surface);
            }
            if selected {
                let y = rect.bottom() - 1.0;
                painter.line_segment([egui::pos2(rect.left() + 4.0, y), egui::pos2(rect.right() - 4.0, y)], Stroke::new(2.0_f32, p.accent));
            }
            if selected || hovered { p.text } else { p.weak }
        };
        let text_color = if deco.dim && !selected && !hovered { p.weak.gamma_multiply(if classic { 1.0 } else { 0.7 }) } else { text_color };
        let label_w = galley.size().x;
        painter.galley(rect.min + pad, galley, text_color);
        if let (Some(b), Some(g)) = (deco.badge, badge_galley) {
            let x = rect.min.x + pad.x + label_w + 3.0;
            let mark = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), egui::vec2(14.0, 14.0));
            paint_mark(painter, mark, &t, b.error);
            let color = if classic { p.text } else { mark_color(&t, b.error) };
            painter.galley(egui::pos2(mark.max.x + 1.0, rect.center().y - g.size().y / 2.0), g, color);
        }
        if close {
            let c = if over_close { p.bad } else { p.weak };
            painter.text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", font, c);
        }
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), closed)
}

/// A row of tabs over a page. Returns true when the selection changed.
pub fn tab_strip<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, tabs: &[(T, String)]) -> bool {
    let decorated: Vec<(T, String, TabDeco)> = tabs.iter().map(|(v, l)| (*v, l.clone(), TabDeco::default())).collect();
    tab_strip_with(ui, current, &decorated)
}

/// [`tab_strip`] with a badge and de-emphasis per tab.
pub fn tab_strip_with<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, tabs: &[(T, String, TabDeco)]) -> bool {
    let mut changed = false;
    strip_frame(ui, |ui| {
        for (v, label, deco) in tabs {
            if tab_with(ui, *current == *v, label, false, *deco).0.clicked() && *current != *v {
                *current = *v;
                changed = true;
            }
        }
    });
    changed
}

/// Lays out tabs edge to edge with the line under them that the page
/// hangs from.
pub fn strip_frame(ui: &mut egui::Ui, add_tabs: impl FnOnce(&mut egui::Ui)) {
    let t = current(ui.ctx());
    let r = ui
        .horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(if t.kind == ThemeKind::Classic { 0.0 } else { 2.0 }, 2.0);
            add_tabs(ui);
        })
        .response;
    let y = r.rect.bottom();
    ui.painter().line_segment([egui::pos2(ui.max_rect().left(), y), egui::pos2(ui.max_rect().right(), y)], Stroke::new(1.0_f32, t.palette.stroke));
    ui.add_space(if t.kind == ThemeKind::Classic { 2.0 } else { 4.0 });
}

// ----- persistence -----

/// `$XDG_CONFIG_HOME/chummer-rs/gui.ini`, next to `sourcebooks.xml`.
pub fn config_path() -> Option<PathBuf> {
    chummer_core::settings::user_settings_dir().and_then(|d| d.parent().map(|p| p.join("gui.ini")))
}

/// The value of a `key=` line of a gui.ini.
pub fn config_get(text: &str, key: &str) -> Option<String> {
    text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_owned())
}

/// `text` with its `key=` line set to `value`; other lines are kept.
pub fn config_set(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}={value}");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
                found = true;
                line.clone()
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !found {
        out.push(line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// The `theme=` line of a gui.ini.
#[cfg(test)]
pub fn parse_config(text: &str) -> Option<ThemeKind> {
    config_get(text, "theme").and_then(|v| ThemeKind::parse(&v))
}

/// `text` with its `theme=` line set to `kind`; other lines are kept.
#[cfg(test)]
pub fn set_config(text: &str, kind: ThemeKind) -> String {
    config_set(text, "theme", kind.as_str())
}

/// A value saved in gui.ini.
pub fn load_value(key: &str) -> Option<String> {
    config_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| config_get(&t, key))
}

/// Save a gui.ini value, keeping the other lines.
pub fn save_value(key: &str, value: &str) -> std::io::Result<()> {
    let Some(path) = config_path() else { return Ok(()) };
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, config_set(&old, key, value))
}

/// The saved appearance; the Classic layout in Graphite for new installs.
pub fn load_appearance() -> Appearance {
    Appearance::from_config(&config_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default())
}

pub fn save_appearance(a: &Appearance) -> std::io::Result<()> {
    let Some(path) = config_path() else { return Ok(()) };
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, a.to_config(&old))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_persists() {
        // Old gui.ini files: only a Classic-layout theme.
        let a = Appearance::from_config("theme=classic\n");
        assert_eq!((a.layout, a.kind()), (Layout::Classic, ThemeKind::Classic));
        assert_eq!(Appearance::from_config(""), Appearance::default());
        assert_eq!(Appearance::default().kind(), ThemeKind::Graphite);
        // Switching to Workspace light keeps the Classic theme for later.
        let a = a.with_kind(ThemeKind::WorkspaceLight);
        assert_eq!((a.layout, a.classic, a.workspace), (Layout::Workspace, ThemeKind::Classic, ThemeKind::WorkspaceLight));
        let text = a.to_config("other=1\ntheme=graphite\n");
        assert_eq!(text, "other=1\ntheme=classic\nlayout=workspace\nworkspace=light\n");
        assert_eq!(Appearance::from_config(&text), a);
        assert_eq!(Appearance::from_config(&text).with_layout(Layout::Classic).kind(), ThemeKind::Classic);
        // A value for the wrong layout is ignored.
        let b = Appearance::from_config("layout=workspace\ntheme=dark\nworkspace=graphite\n");
        assert_eq!((b.classic, b.workspace, b.kind()), (ThemeKind::Graphite, ThemeKind::WorkspaceDark, ThemeKind::WorkspaceDark));
        assert_eq!(Layout::parse(" Workspace "), Some(Layout::Workspace));
        assert_eq!(Layout::parse("tabs"), None);
        assert_eq!(ThemeKind::parse("workspace-light"), Some(ThemeKind::WorkspaceLight));
        for k in ThemeKind::ALL {
            assert_eq!(Theme::of(k).kind, k);
            assert_eq!(Theme::of(k).workspace_layout(), k.layout() == Layout::Workspace);
        }
        assert!(Theme::of(ThemeKind::WorkspaceDark).dark() && !Theme::of(ThemeKind::WorkspaceLight).dark());
    }

    #[test]
    fn workspace_text_is_readable() {
        for w in [WsPalette::DARK, WsPalette::LIGHT] {
            for (name, fg) in [("text", w.text), ("muted", w.muted), ("accent", w.accent), ("physical", w.physical), ("stun", w.stun), ("warning", w.warning), ("error", w.error)] {
                for (bg_name, bg) in [("ground", w.ground), ("chrome", w.chrome), ("raised", w.raised), ("well", w.well)] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{name} on {bg_name} is {c:.2}:1");
                }
                // The current sidebar entry, selected and hovered rows.
                for (bg_name, bg) in [("selection", w.selection), ("hover", w.hover)] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.0, "{name} on {bg_name} is {c:.2}:1");
                }
            }
            assert!(contrast(w.on_primary, w.primary) >= 4.5, "text on primary");
            // Badges and filled damage boxes.
            for (name, fill) in [("warning", w.warning), ("error", w.error), ("physical", w.physical), ("stun", w.stun)] {
                let c = contrast(w.on_badge, fill);
                assert!(c >= 4.5, "badge text on {name} is {c:.2}:1");
            }
            // Empty boxes and inputs must be visible against the cards.
            assert!(contrast(w.control, w.raised) >= 1.9, "control border on raised");
            // Empty boxes also have a `control` border.
            assert!(contrast(w.primary, w.well) >= 2.5, "filled Edge box against an empty one");
        }
    }

    #[test]
    fn glyphs_have_phosphor_icons() {
        for e in ["➕", "🗑", "📖", "🎲", "✖", "✨", "📂", "📝", "🔍", "💾", "⟲", "⟳", "✏", "🔗", "📎", "📁", "🔥", "🧪", "☰", "⚠", "✔", "🎭"] {
            let p = phosphor_for(e);
            assert_ne!(p, e, "{e} has no Phosphor icon");
            assert!(p.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "{e} maps outside the icon font");
        }
        assert_eq!(phosphor_for("x"), "x");
    }

    #[test]
    fn kind_round_trip() {
        for k in ThemeKind::ALL {
            assert_eq!(ThemeKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(ThemeKind::parse(" Classic "), Some(ThemeKind::Classic));
        assert_eq!(ThemeKind::parse("neon"), None);
        assert_eq!(ThemeKind::default(), ThemeKind::Graphite);
    }

    #[test]
    fn config_file() {
        assert_eq!(parse_config(""), None);
        assert_eq!(parse_config("theme = classic\n"), Some(ThemeKind::Classic));
        assert_eq!(parse_config("theme=bogus"), None);
        let s = set_config("", ThemeKind::Classic);
        assert_eq!(s, "theme=classic\n");
        let s = set_config("other=1\ntheme=classic\n", ThemeKind::Graphite);
        assert_eq!(s, "other=1\ntheme=graphite\n");
        assert_eq!(parse_config(&s), Some(ThemeKind::Graphite));
    }

    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_is_readable() {
        for k in ThemeKind::ALL {
            let p = Theme::of(k).palette;
            for (name, fg) in [("text", p.text), ("weak", p.weak), ("accent", p.accent), ("warning", p.warning), ("physical", p.physical), ("stun", p.stun), ("good", p.good), ("bad", p.bad)] {
                for (bg_name, bg) in [("panel", p.panel), ("window", p.window), ("field", p.field)] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{k:?}: {name} on {bg_name} is {c:.2}:1");
                }
            }
            assert!(contrast(p.selection_text, p.selection) >= 4.5, "{k:?}: selected text");
            assert!(contrast(p.on_accent, p.accent) >= 4.5, "{k:?}: text on accent");
            assert!(contrast(p.text, p.surface) >= 4.5, "{k:?}: button text");
        }
    }

    #[test]
    fn fonts_keep_default_fallbacks() {
        for k in ThemeKind::ALL {
            let f = Theme::of(k).fonts();
            let prop = &f.families[&FontFamily::Proportional];
            assert!(prop.len() > 1, "{k:?} dropped egui's fallback fonts");
            assert!(f.families.contains_key(&FontFamily::Name(BOLD.into())));
            for name in prop {
                assert!(f.font_data.contains_key(name), "{name} has no data");
            }
        }
    }
}
