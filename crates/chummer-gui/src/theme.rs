//! Look of the GUI: two themes on the same (Chummer5a) layout.
//!
//! * Classic imitates Chummer5a's WinForms look: SystemColors.Control
//!   greys, white input fields, square corners, Windows-blue selection and
//!   a Segoe-UI-like font (Selawik).
//! * Graphite is the dark "pro tool" style: neutral greys, one teal accent,
//!   IBM Plex Sans and Plex Mono.
//!
//! The active [`Theme`] lives in the egui context (see [`apply`]); widgets
//! read their colours with [`palette`] or the small helpers ([`accent`],
//! [`warn`], ...) so switching themes recolours everything. The choice is
//! kept in `$XDG_CONFIG_HOME/chummer-rs/gui.ini`.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    Classic,
    #[default]
    Graphite,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Classic, ThemeKind::Graphite];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeKind::Classic => "classic",
            ThemeKind::Graphite => "graphite",
        }
    }

    pub fn parse(s: &str) -> Option<ThemeKind> {
        ThemeKind::ALL.into_iter().find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// Menu label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Classic => "Classic",
            ThemeKind::Graphite => "Graphite",
        }
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub kind: ThemeKind,
    pub palette: Palette,
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
            widget_radius: 0,
            frame_radius: 0,
            item_spacing: egui::vec2(6.0, 4.0),
            button_padding: egui::vec2(6.0, 2.0),
            stroke_width: 1.0,
            body_size: 12.5,
            heading_size: 15.0,
        }
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
            widget_radius: 4,
            frame_radius: 6,
            item_spacing: egui::vec2(8.0, 6.0),
            button_padding: egui::vec2(12.0, 6.0),
            stroke_width: 1.0,
            body_size: 13.0,
            heading_size: 17.0,
        }
    }

    pub fn dark(&self) -> bool {
        self.kind == ThemeKind::Graphite
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
        v
    }

    pub fn style(&self) -> egui::Style {
        let mut s = egui::Style { visuals: self.visuals(), ..Default::default() };
        s.spacing.item_spacing = self.item_spacing;
        s.spacing.button_padding = self.button_padding;
        s.spacing.interact_size.y = if self.dark() { 22.0 } else { 20.0 };
        s.spacing.menu_margin = egui::Margin::same(if self.dark() { 6 } else { 3 });
        s.spacing.window_margin = egui::Margin::same(if self.dark() { 10 } else { 6 });
        s.spacing.indent = if self.dark() { 16.0 } else { 14.0 };
        let body = self.body_size;
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(body - 2.0)),
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
            ThemeKind::Graphite => {
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
        f
    }
}

/// Font family name of the bold/semibold face (headings, [`strong`]).
pub const BOLD: &str = "bold";

fn theme_id() -> egui::Id {
    egui::Id::new("chummer-rs-theme")
}

/// Make `theme` the active one: style for both egui themes (so a system
/// light/dark switch changes nothing), fonts, and the copy in ctx data.
pub fn apply(ctx: &egui::Context, theme: &Theme) {
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
    match t.kind {
        ThemeKind::Graphite => egui::Button::new(strong(ui, text).color(p.on_accent)).fill(p.accent).stroke(Stroke::new(1.0_f32, p.accent)),
        ThemeKind::Classic => egui::Button::new(egui::RichText::new(text)).stroke(Stroke::new(1.0_f32, p.stroke_focus)),
    }
}

/// A dice pool as a clickable chip (Graphite) or bold number (Classic).
pub fn pool_chip(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let t = current(ui.ctx());
    let p = t.palette;
    match t.kind {
        ThemeKind::Graphite => {
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
    let t = current(ui.ctx());
    let p = t.palette;
    let classic = t.kind == ThemeKind::Classic;
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let pad = if classic { egui::vec2(6.0, 3.0) } else { egui::vec2(10.0, 6.0) };
    let close_w = if close { font.size + 4.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + 2.0 * pad.x + close_w, galley.size().y + 2.0 * pad.y);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let close_rect = egui::Rect::from_min_max(egui::pos2(rect.max.x - close_w - pad.x * 0.5, rect.min.y), egui::pos2(rect.max.x - pad.x * 0.5, rect.max.y));
    let over_close = close && resp.hover_pos().is_some_and(|pos| close_rect.contains(pos));
    let closed = close && resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| close_rect.contains(pos));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = resp.hovered();
        let text_color;
        if classic {
            // Selected tab: lighter and 2px taller, open towards the page.
            let r = if selected { rect } else { rect.shrink2(egui::vec2(0.0, 1.0)).translate(egui::vec2(0.0, 1.0)) };
            let fill = if selected { p.field } else if hovered { p.surface_hover } else { p.panel };
            painter.rect_filled(r, CornerRadius::ZERO, fill);
            let s = Stroke::new(1.0_f32, if hovered && !selected { p.stroke_focus } else { p.stroke });
            painter.line_segment([r.left_bottom(), r.left_top()], s);
            painter.line_segment([r.left_top(), r.right_top()], s);
            painter.line_segment([r.right_top(), r.right_bottom()], s);
            text_color = p.text;
        } else {
            if hovered && !selected {
                painter.rect_filled(rect, CornerRadius::same(t.widget_radius), p.surface);
            }
            if selected {
                let y = rect.bottom() - 1.0;
                painter.line_segment([egui::pos2(rect.left() + 4.0, y), egui::pos2(rect.right() - 4.0, y)], Stroke::new(2.0_f32, p.accent));
            }
            text_color = if selected || hovered { p.text } else { p.weak };
        }
        painter.galley(rect.min + pad, galley, text_color);
        if close {
            let c = if over_close { p.bad } else { p.weak };
            painter.text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", font, c);
        }
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), closed)
}

/// A row of tabs over a page. Returns true when the selection changed.
pub fn tab_strip<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, tabs: &[(T, String)]) -> bool {
    let mut changed = false;
    strip_frame(ui, |ui| {
        for (v, label) in tabs {
            if tab(ui, *current == *v, label, false).0.clicked() && *current != *v {
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

/// The `theme=` line of a gui.ini.
pub fn parse_config(text: &str) -> Option<ThemeKind> {
    text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == "theme").and_then(|(_, v)| ThemeKind::parse(v))
}

/// `text` with its `theme=` line set to `kind`; other lines are kept.
pub fn set_config(text: &str, kind: ThemeKind) -> String {
    let line = format!("theme={}", kind.as_str());
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.split_once('=').is_some_and(|(k, _)| k.trim() == "theme") {
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

/// The saved theme; Graphite for new installs.
pub fn load_kind() -> ThemeKind {
    config_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse_config(&t)).unwrap_or_default()
}

pub fn save_kind(kind: ThemeKind) -> std::io::Result<()> {
    let Some(path) = config_path() else { return Ok(()) };
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, set_config(&old, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

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
