//! Workspace widgets, drawn as in the owner's approved mockups:
//! buttons, badges, the sidebar entry, cards, budget chips, the
//! dark/light switch, a check box, and the condition monitor and Edge
//! boxes of the Play screen. Colours come from [`crate::theme::ws`].
//!
//! Poppable panels (a card or inspector section with a pop-out button)
//! are in [`super::popout`].

use eframe::egui::{self, Align2, Color32, CornerRadius, FontFamily, FontId, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::icons;
use crate::theme::{self, Badge, WsPalette};

/// The semibold face at `size`.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(theme::BOLD.into()))
}

/// Run `add` and clip what it paints on this layer to `rect`. Classic
/// pages put panels inside the page, and a panel clips to its own rect:
/// a row too wide for the page would otherwise paint over the inspector.
pub fn clip_to<R>(ui: &mut Ui, rect: egui::Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let layer = ui.layer_id();
    let start = ui.ctx().graphics_mut(|g| g.entry(layer).next_idx());
    let r = add(ui);
    ui.ctx().graphics_mut(|g| {
        let list = g.entry(layer);
        let end = list.next_idx();
        for i in start.0..end.0 {
            list.mutate_shape(egui::layers::ShapeIdx(i), |s| s.clip_rect = s.clip_rect.intersect(rect));
        }
    });
    r
}

/// Small uppercase caption ("BUILD", "ATTRIBUTE PTS").
pub fn overline(text: &str, ws: &WsPalette) -> RichText {
    RichText::new(text.to_uppercase()).size(10.5).color(ws.muted)
}

/// Monospace value text.
pub fn mono(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text).font(FontId::monospace(size)).color(color)
}

/// A square, frameless icon button (`size` 22, 24 or 26). Add the tooltip
/// with `on_hover_text` (`on_disabled_hover_text` when disabled).
pub fn icon_button(ui: &mut Ui, glyph: &str, size: f32) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(5), ws.hover);
        }
        let color = if !enabled { ws.muted.gamma_multiply(0.45) } else if hovered { ws.text } else { ws.muted };
        icons::paint(ui.painter(), rect, glyph, (size * 0.58).round(), color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Looks of [`button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Filled with `primary`: the main action.
    Primary,
    /// A raised button with a divider border.
    Secondary,
    /// Text only until hovered.
    Ghost,
    /// Accent text and a primary outline ("Next: Qualities").
    Outline,
}

/// A text button with an optional leading icon, `height` 22–28.
pub fn button(ui: &mut Ui, glyph: Option<&str>, text: &str, look: Look, height: f32) -> Response {
    let ws = theme::ws(ui);
    let size = if height <= 22.0 { 11.5 } else { 12.0 };
    let font = if look == Look::Primary { bold(size) } else { FontId::proportional(size) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let icon_w = if glyph.is_some() { 14.0 + 6.0 } else { 0.0 };
    let pad = 10.0;
    let width = pad * 2.0 + icon_w + galley.size().x;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && resp.hovered();
        let (fill, stroke, color) = match look {
            Look::Primary => (if hovered { ws.primary.gamma_multiply(0.9) } else { ws.primary }, Stroke::new(1.0_f32, ws.primary), ws.on_primary),
            Look::Secondary => (if hovered { ws.hover } else { ws.raised }, Stroke::new(1.0_f32, if hovered { ws.control } else { ws.divider }), ws.text),
            Look::Ghost => (if hovered { ws.hover } else { Color32::TRANSPARENT }, Stroke::NONE, if hovered { ws.text } else { ws.muted }),
            Look::Outline => (if hovered { ws.selection } else { Color32::TRANSPARENT }, Stroke::new(1.0_f32, ws.primary), ws.accent),
        };
        let alpha = if enabled { 1.0 } else { 0.5 };
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(5), fill.gamma_multiply(alpha), Stroke::new(stroke.width, stroke.color.gamma_multiply(alpha)), StrokeKind::Inside);
        let mut x = rect.left() + pad;
        if let Some(g) = glyph {
            icons::paint(painter, egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)), g, 14.0, color.gamma_multiply(alpha));
            x += icon_w;
        }
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color.gamma_multiply(alpha));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An issue count pill: warning or error colour.
pub fn badge(ui: &mut Ui, b: Badge) -> Response {
    let ws = theme::ws(ui);
    let fill = if b.error { ws.error } else { ws.warning };
    count_pill(ui, &b.count.to_string(), fill, ws.on_badge)
}

/// A small filled pill with `text` (badges, counts).
pub fn count_pill(ui: &mut Ui, text: &str, fill: Color32, ink: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), bold(10.5), ink);
    let w = (galley.size().x + 8.0).max(16.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 16.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    }
    resp
}

/// A rounded outline tag ("Low-light vision", "Wound modifier −1").
pub fn tag(ui: &mut Ui, text: &str, color: Color32, border: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 14.0, 18.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_stroke(rect, CornerRadius::same(9), Stroke::new(1.0_f32, border), StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp
}

/// A keyboard key ("Ctrl K", "Esc", "↑").
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let ws = theme::ws(ui);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(10.5), ws.muted);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 8.0, 16.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, CornerRadius::same(3), ws.well, Stroke::new(1.0_f32, ws.divider), StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ws.muted);
    }
    resp
}

/// One sidebar entry: icon, label, an issue badge; the current one is
/// highlighted with a bar on the left.
pub fn nav_item(ui: &mut Ui, selected: bool, glyph: &str, label: &str, badge: Option<Badge>) -> Response {
    let ws = theme::ws(ui);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = resp.hovered();
        if selected {
            painter.rect_filled(rect, CornerRadius::same(5), ws.selection);
            let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 5.0), egui::pos2(rect.left() + 2.0, rect.bottom() - 5.0));
            painter.rect_filled(bar, CornerRadius::same(1), ws.primary);
        } else if hovered {
            painter.rect_filled(rect, CornerRadius::same(5), ws.hover);
        }
        let icon_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.center().y - 7.0), Vec2::splat(14.0));
        icons::paint(painter, icon_rect, glyph, 14.0, if selected { ws.accent } else { ws.muted });
        let mut right = rect.right() - 8.0;
        if let Some(b) = badge {
            let text = b.count.to_string();
            let g = painter.layout_no_wrap(text, bold(10.5), ws.on_badge);
            let w = (g.size().x + 8.0).max(16.0);
            let pill = egui::Rect::from_min_size(egui::pos2(right - w, rect.center().y - 8.0), egui::vec2(w, 16.0));
            painter.rect_filled(pill, CornerRadius::same(8), if b.error { ws.error } else { ws.warning });
            painter.galley(pill.center() - g.size() / 2.0, g, ws.on_badge);
            right = pill.left() - 6.0;
        }
        let text_left = icon_rect.right() + 9.0;
        let galley = painter.layout(label.to_owned(), FontId::proportional(12.5), ws.text, (right - text_left).max(10.0));
        let galley = if galley.rows.len() > 1 { painter.layout_no_wrap(label.to_owned(), FontId::proportional(12.5), ws.text) } else { galley };
        let clip = egui::Rect::from_min_max(egui::pos2(text_left, rect.top()), egui::pos2(right, rect.bottom()));
        painter.with_clip_rect(clip).galley(egui::pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, ws.text);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A group heading in the sidebar.
pub fn nav_heading(ui: &mut Ui, text: &str) {
    let ws = theme::ws(ui);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(overline(text, &ws));
    });
    ui.add_space(1.0);
}

/// A raised card (sections of a page).
pub fn card_frame(ws: &WsPalette) -> egui::Frame {
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(12))
}

/// A card's title in the 13px semibold face.
pub fn title(text: &str, ws: &WsPalette) -> RichText {
    RichText::new(text).font(bold(13.0)).color(ws.text)
}

/// Tone of a [`budget_chip`] value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// Points left to spend.
    Warning,
    /// Over budget.
    Error,
}

/// One budget in the strip above the page: caption, value and, with
/// `fill`, a 2px bar (`fill` 0–1).
pub fn budget_chip(ui: &mut Ui, label: &str, value: &str, fill: Option<f32>, tone: Tone) -> Response {
    let ws = theme::ws(ui);
    let color = match tone {
        Tone::Normal => ws.text,
        Tone::Warning => ws.warning,
        Tone::Error => ws.error,
    };
    ui.allocate_ui_with_layout(egui::vec2(78.0, 36.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_width(78.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(overline(label, &ws)).extend());
        ui.add(egui::Label::new(mono(value, 12.5, color)).extend());
        if let Some(f) = fill {
            let w = ui.min_rect().width().max(78.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 2.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(1), ws.divider);
            let bar = match tone {
                Tone::Normal => ws.primary,
                Tone::Warning => ws.warning,
                Tone::Error => ws.error,
            };
            let mut r = rect;
            r.set_width(rect.width() * f.clamp(0.0, 1.0));
            ui.painter().rect_filled(r, CornerRadius::same(1), bar);
        }
    })
    .response
}

/// A segmented switch (the dark/light toggle, filters): returns the
/// index clicked. Items are text or an icon glyph, with a tooltip.
pub fn segmented(ui: &mut Ui, items: &[(&str, &str)], selected: usize, height: f32) -> Option<usize> {
    let ws = theme::ws(ui);
    let inner = height - 4.0;
    let font = FontId::proportional(if height <= 22.0 { 11.0 } else { 12.5 });
    let galleys: Vec<_> = items.iter().map(|(t, _)| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), Color32::PLACEHOLDER)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| (g.size().x + 14.0).max(inner + 4.0)).collect();
    let total = widths.iter().sum::<f32>() + 2.0 + (items.len().saturating_sub(1)) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total, height), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(6), ws.well, Stroke::new(1.0_f32, ws.control), StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.left() + 1.0;
    for (i, ((g, w), (_, tip))) in galleys.into_iter().zip(&widths).zip(items).enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), egui::vec2(*w, inner));
        let resp = ui.interact(r, ui.id().with(("segmented", i)), Sense::click()).on_hover_text(*tip).on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = i == selected;
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(4), ws.primary);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(4), ws.hover);
        }
        ui.painter().galley(r.center() - g.size() / 2.0, g, if on { ws.on_primary } else { ws.text });
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w + 1.0;
    }
    clicked
}

/// A check box like the mockups': a 14px box, filled `primary` with a
/// check mark when on.
pub fn check(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let ws = theme::ws(ui);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(12.5), ws.text);
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(14.0 + 7.0 + galley.size().x, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let b = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 7.0), Vec2::splat(14.0));
        let border = if *on || resp.hovered() { ws.primary } else { ws.control };
        ui.painter().rect(b, CornerRadius::same(3), if *on { ws.primary } else { ws.well }, Stroke::new(1.0_f32, border), StrokeKind::Inside);
        if *on {
            icons::paint(ui.painter(), b, icons::CHECK, 11.0, ws.on_primary);
        }
        ui.painter().galley(egui::pos2(b.right() + 7.0, rect.center().y - galley.size().y / 2.0), galley, ws.text);
    }
    resp
}

/// A label and a monospace value on one 20px line.
pub fn stat_row(ui: &mut Ui, label: &str, value: &str, accent: bool) {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        ui.label(RichText::new(label).size(12.0).color(ws.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(mono(value, 12.5, if accent { ws.accent } else { ws.text }));
        });
    });
}

// ----- condition monitor and Edge -----

/// One damage track of a condition monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub label: String,
    pub color: Color32,
    pub boxes: i32,
    pub filled: i32,
    /// Boxes per wound modifier (the row length too); 0 for none (an
    /// A.I.'s Matrix track).
    pub threshold: i32,
}

/// What a click on the condition monitor asks for: the new number of
/// filled boxes of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmClick {
    Physical(i32),
    Stun(i32),
}

/// Filled boxes after clicking box `n` (1-based): fill up to it, or clear
/// it when it is the last filled one.
pub fn box_click(filled: i32, n: i32) -> i32 {
    if filled == n {
        n - 1
    } else {
        n
    }
}

/// Edge points left after clicking box `n` of the Edge boxes (filled =
/// available): an available box spends it and those after it, a spent one
/// regains up to it.
pub fn edge_click(available: i32, n: i32) -> i32 {
    if n <= available {
        n - 1
    } else {
        n
    }
}

const BOX: f32 = 22.0;
const GAP: f32 = 3.0;

/// One 22px box. `dashed` draws an empty overflow box.
fn damage_box(ui: &mut Ui, on: bool, color: Color32, marker: Option<String>, dashed: bool, tip: String) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(BOX), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = resp.hovered();
        if on {
            painter.rect(rect, CornerRadius::same(3), color, Stroke::new(1.0_f32, color), StrokeKind::Inside);
        } else if dashed {
            let r = rect.shrink(0.5);
            let edge = if hovered { color } else { ws.control };
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            for w in pts.windows(2) {
                painter.extend(egui::Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.0_f32, edge), 3.0, 2.0));
            }
        } else {
            painter.rect(rect, CornerRadius::same(3), ws.well, Stroke::new(1.0_f32, if hovered { color } else { ws.control }), StrokeKind::Inside);
        }
        if let Some(m) = marker {
            painter.text(rect.center(), Align2::CENTER_CENTER, m, FontId::monospace(10.0), if on { ws.on_badge } else { ws.muted });
        }
    }
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A grid of boxes `first..=last`, `per_row` to a row. `f(n)` gives each
/// box's (on, marker, dashed); returns the box clicked.
fn box_grid(ui: &mut Ui, first: i32, last: i32, per_row: i32, mut f: impl FnMut(i32) -> (bool, Option<String>, bool, String), color: Color32) -> Option<i32> {
    let mut clicked = None;
    let per_row = per_row.max(1);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(GAP);
        let mut n = first;
        while n <= last {
            ui.horizontal(|ui| {
                for k in n..(n + per_row).min(last + 1) {
                    let (on, marker, dashed, tip) = f(k);
                    if damage_box(ui, on, color, marker, dashed, tip).clicked() {
                        clicked = Some(k);
                    }
                }
            });
            n += per_row;
        }
    });
    clicked
}

/// Track header ("Physical 3/10") and its box grid; `overflow` boxes
/// follow under it (physical only).
fn track_column(ui: &mut Ui, t: &Track, overflow: Option<(i32, &str)>) -> Option<i32> {
    let ws = theme::ws(ui);
    let per_row = if t.threshold > 0 { t.threshold } else { 3 };
    let mut out = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(&t.label).font(bold(12.5)).color(t.color));
            ui.label(mono(format!("{}/{}", t.filled.min(t.boxes), t.boxes), 11.5, ws.muted));
        });
        let width = per_row as f32 * (BOX + GAP) - GAP;
        ui.set_min_width(width);
        let label = t.label.clone();
        out = box_grid(
            ui,
            1,
            t.boxes,
            per_row,
            |n| {
                let on = n <= t.filled;
                let marker = (t.threshold > 0 && n % t.threshold == 0).then(|| format!("−{}", n / t.threshold));
                let tip = match &marker {
                    Some(m) => format!("{label} {n} · {m}"),
                    None => format!("{label} {n}"),
                };
                (on, marker, false, tip)
            },
            t.color,
        )
        .map(|n| box_click(t.filled, n));
        if let Some((count, caption)) = overflow.filter(|(c, _)| *c > 0) {
            ui.add_space(2.0);
            ui.label(overline(caption, &ws));
            let base = t.boxes;
            let clicked = box_grid(ui, base + 1, base + count, per_row, |n| (n <= t.filled, None, true, format!("{caption} {}", n - base)), t.color);
            if let Some(n) = clicked {
                out = Some(box_click(t.filled, n));
            }
        }
    });
    out
}

/// The condition monitor of the Play screen: Physical and Stun side by
/// side as clickable boxes in rows of the wound threshold, a wound marker
/// in the last box of each row, and Physical's overflow boxes under it.
pub fn condition_monitor(ui: &mut Ui, id: impl std::hash::Hash, physical: &Track, stun: &Track, overflow: i32, overflow_label: &str) -> Option<CmClick> {
    let mut click = None;
    ui.push_id(id, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            if let Some(n) = track_column(ui, physical, Some((overflow, overflow_label))) {
                click = Some(CmClick::Physical(n));
            }
            if let Some(n) = track_column(ui, stun, None) {
                click = Some(CmClick::Stun(n));
            }
        });
    });
    click
}

/// Edge as square boxes up to the attribute: filled `primary` =
/// available, empty = spent. Returns the new number available.
pub fn edge_boxes(ui: &mut Ui, id: impl std::hash::Hash, total: i32, available: i32, tip: impl Fn(i32, bool) -> String) -> Option<i32> {
    let ws = theme::ws(ui);
    let mut out = None;
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for n in 1..=total.max(0) {
                let on = n <= available;
                if damage_box(ui, on, ws.primary, None, false, tip(n, on)).clicked() {
                    out = Some(edge_click(available, n));
                }
            }
        });
    });
    out
}

// ----- the Play screen and the GM screen -----

/// A row of small damage boxes (`size` 14–16: a device's Matrix
/// condition monitor, a vehicle's damage track), `per_row` to a row.
/// `label` names a box in its tooltip. Returns the new number filled.
#[allow(clippy::too_many_arguments)]
pub fn small_track(ui: &mut Ui, id: impl std::hash::Hash, boxes: i32, filled: i32, per_row: i32, size: f32, color: Color32, label: &str) -> Option<i32> {
    let ws = theme::ws(ui);
    let mut out = None;
    ui.push_id(id, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(if size < 16.0 { 2.0 } else { GAP });
            let per_row = per_row.max(1);
            let mut n = 1;
            while n <= boxes {
                ui.horizontal(|ui| {
                    for k in n..(n + per_row).min(boxes + 1) {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
                        let on = k <= filled;
                        let edge = if on || resp.hovered() { color } else { ws.control };
                        ui.painter().rect(rect, CornerRadius::same(2), if on { color } else { ws.well }, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
                        if resp.on_hover_text(format!("{label} {k}")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            out = Some(box_click(filled, k));
                        }
                    }
                });
                n += per_row;
            }
        });
    });
    out
}

/// A quick-roll tile: the label on the left, the pool in accent
/// monospace on the right; `height` 26 or 28.
pub fn roll_tile(ui: &mut Ui, label: &str, value: &str, width: f32, height: f32) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(5), if hovered { ws.hover } else { ws.raised }, Stroke::new(1.0_f32, if hovered { ws.control } else { ws.divider }), StrokeKind::Inside);
        let v = painter.layout_no_wrap(value.to_owned(), FontId::monospace(12.5), ws.accent);
        let vx = rect.right() - 9.0 - v.size().x;
        let l = painter.layout_no_wrap(label.to_owned(), FontId::proportional(if height <= 26.0 { 12.0 } else { 12.5 }), ws.text);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(vx - 4.0, rect.bottom()));
        painter.with_clip_rect(clip).galley(egui::pos2(rect.left() + 9.0, rect.center().y - l.size().y / 2.0), l, ws.text);
        painter.galley(egui::pos2(vx, rect.center().y - v.size().y / 2.0), v, ws.accent);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rounds left in a weapon as 4×12 pips (filled = loaded).
pub fn ammo_pips(ui: &mut Ui, left: i32, total: i32) -> Response {
    let ws = theme::ws(ui);
    let n = total.max(0) as f32;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2((n * 6.0 - 2.0).max(0.0), 12.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        for k in 0..total.max(0) {
            let r = egui::Rect::from_min_size(egui::pos2(rect.left() + k as f32 * 6.0, rect.top()), egui::vec2(4.0, 12.0));
            if k < left {
                ui.painter().rect(r, CornerRadius::same(1), ws.primary, Stroke::new(1.0_f32, ws.primary), StrokeKind::Inside);
            } else {
                ui.painter().rect_stroke(r, CornerRadius::same(1), Stroke::new(1.0_f32, ws.control), StrokeKind::Inside);
            }
        }
    }
    resp
}

/// One die of a roll: hits filled `primary`, ones outlined in `error`.
pub fn die_face(ui: &mut Ui, value: u8) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let (fill, edge, ink) = match value {
            5 | 6 => (ws.primary, ws.primary, ws.on_primary),
            1 => (Color32::TRANSPARENT, ws.error, ws.error),
            _ => (Color32::TRANSPARENT, ws.control, ws.muted),
        };
        ui.painter().rect(rect, CornerRadius::same(4), fill, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, value.to_string(), FontId::monospace(12.0), ink);
    }
    resp
}

/// A small filled circle (online state), `size` 6 or 7.
pub fn dot(ui: &mut Ui, color: Color32, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
    resp
}

/// A 1px divider across the `ui` (`vertical`: a line `height` tall).
pub fn divider(ui: &mut Ui, vertical: Option<f32>) {
    let ws = theme::ws(ui);
    let size = match vertical {
        Some(h) => egui::vec2(1.0, h),
        None => egui::vec2(ui.available_width(), 1.0),
    };
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, ws.divider);
}

/// A row of a list: `text` on the left, a muted monospace `value` on the
/// right, a line above (`height` 20–30).
pub fn list_row(ui: &mut Ui, text: &str, value: &str, height: f32, line: bool) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if line {
            painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0_f32, ws.divider));
        }
        let v = painter.layout_no_wrap(value.to_owned(), FontId::monospace(11.5), ws.muted);
        let vx = rect.right() - v.size().x;
        let t = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), ws.text);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(vx - 6.0, rect.bottom()));
        painter.with_clip_rect(clip).galley(egui::pos2(rect.left(), rect.center().y - t.size().y / 2.0), t, ws.text);
        painter.galley(egui::pos2(vx, rect.center().y - v.size().y / 2.0), v, ws.muted);
    }
    resp
}

/// A [`button`] as wide as the `ui`, its icon and text centred.
pub fn wide_button(ui: &mut Ui, glyph: Option<&str>, text: &str, look: Look, height: f32) -> Response {
    let ws = theme::ws(ui);
    let size = if height <= 22.0 { 11.5 } else { 12.0 };
    let font = if look == Look::Primary { bold(size) } else { FontId::proportional(size) };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && resp.hovered();
        let (fill, stroke, color) = match look {
            Look::Primary => (if hovered { ws.primary.gamma_multiply(0.9) } else { ws.primary }, ws.primary, ws.on_primary),
            Look::Secondary => (if hovered { ws.hover } else { ws.raised }, if hovered { ws.control } else { ws.divider }, ws.text),
            Look::Ghost => (if hovered { ws.hover } else { Color32::TRANSPARENT }, Color32::TRANSPARENT, if hovered { ws.text } else { ws.muted }),
            Look::Outline => (if hovered { ws.selection } else { Color32::TRANSPARENT }, ws.primary, ws.accent),
        };
        let alpha = if enabled { 1.0 } else { 0.5 };
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(5), fill.gamma_multiply(alpha), Stroke::new(1.0_f32, stroke.gamma_multiply(alpha)), StrokeKind::Inside);
        let galley = painter.layout_no_wrap(text.to_owned(), font, color.gamma_multiply(alpha));
        let icon_w = if glyph.is_some() { 20.0 } else { 0.0 };
        let mut x = rect.center().x - (icon_w + galley.size().x) / 2.0;
        if let Some(g) = glyph {
            icons::paint(painter, egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)), g, 14.0, color.gamma_multiply(alpha));
            x += icon_w;
        }
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color.gamma_multiply(alpha));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ----- item pages, catalog and home (added for the gear and home screens) -----

/// A number field with − and + buttons (the catalog's Rating): a
/// [`num_stepper`]. Returns the response of the value; `changed` when a
/// button, a drag or typing moved it.
pub fn rating_stepper(ui: &mut Ui, value: &mut i32, min: i32, max: i32, lower_tip: &str, raise_tip: &str) -> Response {
    let id = ui.next_auto_id();
    num_stepper(ui, id, value, min, max, lower_tip, raise_tip)
}

/// A 24px line: a muted label, a monospace value in `color`, and an
/// optional small note after it ("was 8 + 1d6", "≤ 12 ok").
pub fn value_row(ui: &mut Ui, label: &str, value: &str, color: Color32, note: &str) {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        ui.label(RichText::new(label).size(12.5).color(ws.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !note.is_empty() {
                ui.label(RichText::new(note).size(11.0).color(ws.muted));
            }
            ui.label(mono(value, 12.5, color));
        });
    });
}

/// A thin horizontal rule in the divider colour.
pub fn rule(ui: &mut Ui) {
    let ws = theme::ws(ui);
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, ws.divider);
}

/// An icon and a line of text in `color` (requirement checks, sync
/// states).
pub fn icon_line(ui: &mut Ui, glyph: &str, text: &str, color: Color32, text_color: Color32) -> Response {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(icons::icon(glyph, 13.0, color));
        // Wrap within the space left (a wrapping label in a row would
        // widen a side panel instead).
        let w = ui.available_width();
        ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_max_width(w);
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(text_color)).wrap());
        });
    })
    .response
}

/// Essence before and after a purchase as a bar out of 6: the essence
/// left in `primary`, the part the purchase takes in `accent`.
pub fn essence_bar(ui: &mut Ui, before: f64, after: f64) {
    let ws = theme::ws(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), ws.divider);
    let frac = |v: f64| (v / 6.0).clamp(0.0, 1.0) as f32;
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac(after.min(before)), rect.height()));
    ui.painter().rect_filled(left, CornerRadius::same(4), ws.primary);
    if before > after {
        let taken = egui::Rect::from_min_max(egui::pos2(left.right(), rect.top()), egui::pos2(rect.left() + rect.width() * frac(before), rect.bottom()));
        ui.painter().rect_filled(taken, CornerRadius::same(2), ws.accent);
    }
}

/// A clickable card (Home's Continue and Tools cards): raised, with a
/// divider border that turns `primary` on hover. `add` fills it.
pub fn click_card<R>(ui: &mut Ui, id: impl std::hash::Hash, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let ws = theme::ws(ui);
    let id = ui.id().with(id);
    let hovered = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let inner = egui::Frame::new()
        .fill(if hovered { ws.hover } else { ws.raised })
        .stroke(Stroke::new(1.0_f32, if hovered { ws.primary } else { ws.divider }))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.vertical(|ui| add(ui)).inner
        });
    let resp = ui.interact(inner.response.rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let now = resp.hovered();
    if now != hovered {
        ui.ctx().data_mut(|d| d.insert_temp(id, now));
        ui.ctx().request_repaint();
    }
    (resp, inner.inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_fill_and_clear() {
        assert_eq!(box_click(0, 3), 3);
        assert_eq!(box_click(3, 3), 2);
        assert_eq!(box_click(5, 2), 2);
        assert_eq!(box_click(2, 11), 11, "overflow boxes go past the track");
    }

    #[test]
    fn edge_spends_and_regains() {
        // 3 of 4 available: clicking the 3rd spends it, the 4th regains it.
        assert_eq!(edge_click(3, 3), 2);
        assert_eq!(edge_click(3, 1), 0);
        assert_eq!(edge_click(3, 4), 4);
        assert_eq!(edge_click(0, 2), 2);
    }
}

// ----- build and career pages (tables, steppers, rating pips) -----

/// Widths of a table's columns: `spec` gives fixed widths, with `0.0`
/// for the one column that takes what is left of `total` (at least 80).
pub fn columns(total: f32, spec: &[f32], gap: f32) -> Vec<f32> {
    let fixed: f32 = spec.iter().sum::<f32>() + gap * spec.len().saturating_sub(1) as f32;
    let flex = (total - fixed).max(80.0);
    spec.iter().map(|w| if *w == 0.0 { flex } else { *w }).collect()
}

/// Gap between table columns.
pub const COL_GAP: f32 = 10.0;
/// Padding at the left and right of a table row.
const ROW_PAD: f32 = 10.0;

/// A table in a card: no inner margin, so rows run edge to edge.
pub fn table_frame(ws: &WsPalette) -> egui::Frame {
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(6))
}

/// The column widths for a table drawn in `ui` (inside [`table_frame`]).
pub fn table_columns(ui: &Ui, spec: &[f32]) -> Vec<f32> {
    columns(ui.available_width() - 2.0 * ROW_PAD, spec, COL_GAP)
}

/// One cell `width` wide, its contents centred vertically.
pub fn cell<R>(ui: &mut Ui, width: f32, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(egui::vec2(width, height));
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.x = 6.0;
        add(ui)
    })
    .inner
}

/// A table's header row: small uppercase captions, a line under it.
pub fn table_header(ui: &mut Ui, captions: &[&str], widths: &[f32]) {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = COL_GAP;
        ui.add_space(ROW_PAD);
        for (c, w) in captions.iter().zip(widths) {
            cell(ui, *w, 24.0, |ui| ui.label(overline(c, &ws)));
        }
    });
    hline(ui, ws.divider);
}

/// A 1px line across the `ui`.
pub fn hline(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, color);
}

/// One table row `height` high: selected rows are filled, hovered ones
/// lightly. `add` draws the cells (with [`cell`]); buttons in it take
/// their own clicks. Returns the row's response: a click selects it.
pub fn table_row(ui: &mut Ui, id: impl std::hash::Hash, selected: bool, height: f32, add: impl FnOnce(&mut Ui)) -> Response {
    let ws = theme::ws(ui);
    let width = ui.available_width();
    let top = ui.cursor().min;
    let rect = egui::Rect::from_min_size(top, egui::vec2(width, height));
    let resp = ui.interact(rect, ui.id().with(("table row", id)), Sense::click());
    let bg = ui.painter().add(egui::Shape::Noop);
    ui.horizontal(|ui| {
        ui.set_min_height(height);
        // A click on the text selects the row, not the text.
        ui.style_mut().interaction.selectable_labels = false;
        ui.spacing_mut().item_spacing.x = COL_GAP;
        ui.add_space(ROW_PAD);
        add(ui);
    });
    let fill = if selected {
        ws.selection
    } else if resp.hovered() {
        ws.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::ZERO, fill));
    resp
}

/// Rating pips: `max` small bars, the first `value` filled (6px wide, or
/// 5px for 12-step skill ratings).
pub fn pips(ui: &mut Ui, value: i32, max: i32) -> Response {
    let ws = theme::ws(ui);
    let w = if max > 8 { 5.0 } else { 6.0 };
    let n = max.clamp(0, 30);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(n as f32 * (w + 2.0), 10.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        for i in 0..n {
            let r = egui::Rect::from_min_size(egui::pos2(rect.left() + i as f32 * (w + 2.0), rect.top()), egui::vec2(w, 10.0));
            if i < value {
                ui.painter().rect(r, CornerRadius::same(1), ws.primary, Stroke::new(1.0_f32, ws.primary), StrokeKind::Inside);
            } else {
                ui.painter().rect_stroke(r, CornerRadius::same(1), Stroke::new(1.0_f32, ws.control), StrokeKind::Inside);
            }
        }
    }
    resp
}

/// An inline number field with − and + buttons, clamped to
/// `min..=max` (a [`num_stepper`]). Returns true if it changed.
pub fn stepper(ui: &mut Ui, id: impl std::hash::Hash, value: &mut i32, min: i32, max: i32, lower_tip: &str, raise_tip: &str) -> bool {
    num_stepper(ui, id, value, min, max, lower_tip, raise_tip).changed()
}

/// A career advance button: "+1 · 14 k" with an arrow; disabled when
/// the karma is not there (the tooltip says why).
pub fn cost_button(ui: &mut Ui, text: &str, affordable: bool, tip: &str, short_tip: &str) -> Response {
    let r = ui.add_enabled_ui(affordable, |ui| button(ui, Some(icons::ARROW_UP), text, Look::Secondary, 24.0)).inner;
    r.on_hover_text(tip).on_disabled_hover_text(short_tip)
}

/// An issue mark: a warning (or error) icon with the messages as tooltip.
pub fn issue_mark(ui: &mut Ui, error: bool, tip: &str) -> Response {
    let ws = theme::ws(ui);
    let glyph = if error { icons::WARNING_OCTAGON } else { icons::WARNING };
    ui.label(icons::icon(glyph, 13.0, if error { ws.error } else { ws.warning })).on_hover_text(tip)
}

/// A page or card heading: the title, a muted note after it, and
/// `right` drawn from the right edge (buttons).
pub fn heading(ui: &mut Ui, text: &str, note: &str, size: f32, right: impl FnOnce(&mut Ui)) {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(text).font(bold(size)).color(ws.text));
        if !note.is_empty() {
            ui.label(RichText::new(note).size(11.5).color(ws.muted));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(2.0);
            right(ui);
        });
    });
}

/// A small card with an uppercase caption and label/value rows (the
/// derived values under the attributes). `rows`: (label, value, accent).
pub fn mini_card(ui: &mut Ui, caption: &str, rows: &[(String, String, bool)], width: f32) {
    let ws = theme::ws(ui);
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(width - 22.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(overline(caption, &ws));
            ui.add_space(2.0);
            for (label, value, accent) in rows {
                stat_row(ui, label, value, *accent);
            }
        });
    });
}

/// A tile with a caption over a large value (the career karma summary).
pub fn tile(ui: &mut Ui, caption: &str, value: &str, accent: bool, width: f32) {
    let ws = theme::ws(ui);
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(9, 7)).show(ui, |ui| {
        ui.set_width(width - 20.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(overline(caption, &ws));
            ui.label(mono(value, 14.0, if accent { ws.accent } else { ws.text }));
        });
    });
}

/// A wide button with a title, a muted line under it and a value on the
/// right (career "Other advances"); dimmed when not `enabled`.
pub fn advance_card(ui: &mut Ui, glyph: &str, title: &str, detail: &str, value: &str, enabled: bool, width: f32) -> Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(6), if hovered { ws.hover } else { ws.raised }, Stroke::new(1.0_f32, if hovered { ws.control } else { ws.divider }), StrokeKind::Inside);
        let ink = if enabled { ws.text } else { ws.muted };
        icons::paint(painter, egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0, rect.center().y - 7.0), Vec2::splat(14.0)), glyph, 14.0, if enabled { ws.accent } else { ws.muted });
        let v = painter.layout_no_wrap(value.to_owned(), FontId::monospace(12.0), Color32::PLACEHOLDER);
        let right = rect.right() - 10.0 - v.size().x - 8.0;
        let left = rect.left() + 32.0;
        let clip = egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(right, rect.bottom()));
        let t = painter.layout_no_wrap(title.to_owned(), FontId::proportional(12.5), ink);
        let d = painter.layout_no_wrap(detail.to_owned(), FontId::proportional(11.0), ws.muted);
        let top = rect.center().y - (t.size().y + d.size().y) / 2.0;
        painter.with_clip_rect(clip).galley(egui::pos2(left, top), t.clone(), ink);
        painter.with_clip_rect(clip).galley(egui::pos2(left, top + t.size().y), d, ws.muted);
        painter.galley(egui::pos2(rect.right() - 10.0 - v.size().x, rect.center().y - v.size().y / 2.0), v, if enabled { ws.accent } else { ws.muted });
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A text field in the Workspace style (well, control border).
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let ws = theme::ws(ui);
    let mut r = None;
    egui::Frame::new().fill(ws.well).stroke(Stroke::new(1.0_f32, ws.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        r = Some(ui.add(egui::TextEdit::singleline(text).frame(false).hint_text(hint).desired_width(width - 14.0)));
    });
    r.expect("drawn")
}

// ----- compact number steppers -----

/// Width of a stepper's − and + buttons.
const STEP_BUTTON: f32 = 18.0;
/// Height of a stepper.
const STEP_HEIGHT: f32 = 24.0;

/// Characters a stepper's field is sized for: the widest of the range's
/// ends (at most 4, so a large range does not make a wide field) and the
/// current value (so it always fits).
pub fn stepper_chars(min: &str, max: &str, value: &str) -> usize {
    min.chars().count().max(max.chars().count()).clamp(1, 4).max(value.chars().count())
}

/// Width of a stepper's value field for `chars` characters `char_w` wide.
pub fn stepper_field_width(chars: usize, char_w: f32) -> f32 {
    (chars.max(1) as f32 * char_w + 8.0).ceil()
}

/// One of a stepper's − / + buttons: `STEP_BUTTON` wide, `height` tall.
fn step_button(ui: &mut Ui, glyph: &str, height: f32, enabled: bool, tip: &str) -> Response {
    let ws = theme::ws(ui);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(STEP_BUTTON, height), sense);
    if ui.is_rect_visible(rect) {
        let on = enabled && ui.is_enabled();
        let hovered = on && resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect.shrink(1.0), CornerRadius::same(3), ws.hover);
        }
        let color = if !on { ws.muted.gamma_multiply(0.4) } else if hovered { ws.text } else { ws.muted };
        icons::paint(ui.painter(), rect, glyph, 11.0, color);
    }
    let resp = resp.on_hover_text(tip);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// The body of every stepper: a bordered field with a small − button, the
/// value (drag it, click to type, arrow keys while typing) and a small +
/// button, all inside the border. The field is as wide as the values of
/// the range need, so nothing spills over at any value. `step` is what
/// the buttons add, `speed` the drag speed.
#[allow(clippy::too_many_arguments)]
fn step_field(ui: &mut Ui, id: egui::Id, value: &mut f64, min: f64, max: f64, step: f64, decimals: usize, speed: f64, lower_tip: &str, raise_tip: &str) -> Response {
    let ws = theme::ws(ui);
    let max = max.max(min);
    let old = *value;
    let font = FontId::monospace(12.5);
    let fmt = |v: f64| format!("{v:.decimals$}");
    let char_w = ui.painter().layout_no_wrap("0".to_owned(), font.clone(), Color32::PLACEHOLDER).size().x;
    let field_w = stepper_field_width(stepper_chars(&fmt(min), &fmt(max), &fmt(*value)), char_w);
    let enabled = ui.is_enabled();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(STEP_BUTTON * 2.0 + field_w + 2.0, STEP_HEIGHT), Sense::hover());
    let radius = CornerRadius::same(theme::current(ui.ctx()).widget_radius.min(5));
    ui.painter().rect(rect, radius, ws.well, Stroke::new(1.0_f32, if enabled { ws.control } else { ws.divider }), StrokeKind::Inside);
    let inner = rect.shrink(1.0);
    let resp = clip_to(ui, rect, |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt(("stepper", id)));
        child.spacing_mut().item_spacing.x = 0.0;
        if step_button(&mut child, icons::MINUS, inner.height(), *value > min, lower_tip).clicked() {
            *value -= step;
        }
        {
            let style = child.style_mut();
            style.spacing.interact_size = egui::vec2(field_w, inner.height());
            style.spacing.button_padding = egui::vec2(2.0, 0.0);
            style.drag_value_text_style = egui::TextStyle::Monospace;
            style.text_styles.insert(egui::TextStyle::Monospace, font.clone());
            let v = &mut style.visuals;
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                w.bg_fill = Color32::TRANSPARENT;
                w.weak_bg_fill = Color32::TRANSPARENT;
                w.bg_stroke = Stroke::NONE;
                w.expansion = 0.0;
            }
            v.widgets.hovered.weak_bg_fill = ws.hover;
            v.extreme_bg_color = Color32::TRANSPARENT;
            v.text_edit_bg_color = Some(Color32::TRANSPARENT);
            v.override_text_color = Some(ws.text);
        }
        let r = child.add(egui::DragValue::new(value).range(min..=max).speed(speed).max_decimals(decimals));
        if step_button(&mut child, icons::PLUS, inner.height(), *value < max, raise_tip).clicked() {
            *value += step;
        }
        r
    });
    let mut resp = resp;
    *value = value.clamp(min, max);
    if *value != old {
        resp.mark_changed();
    }
    resp
}

/// A whole-number stepper (attributes, skills, ratings, months), clamped
/// to `min..=max`. Returns the value's response; `changed` when a button,
/// a drag or typing moved it.
pub fn num_stepper(ui: &mut Ui, id: impl std::hash::Hash, value: &mut i32, min: i32, max: i32, lower_tip: &str, raise_tip: &str) -> Response {
    let mut v = f64::from(*value);
    let id = egui::Id::new(id);
    let r = step_field(ui, id, &mut v, f64::from(min), f64::from(max), 1.0, 0, 0.1, lower_tip, raise_tip);
    *value = v.round() as i32;
    r
}

/// A stepper for a quantity that may have decimals: the buttons add
/// `step`, a drag moves it by `step` per point.
#[allow(clippy::too_many_arguments)]
pub fn qty_stepper(ui: &mut Ui, id: impl std::hash::Hash, value: &mut f64, min: f64, max: f64, step: f64, decimals: usize, lower_tip: &str, raise_tip: &str) -> Response {
    let decimals = if step.fract() == 0.0 && value.fract() == 0.0 { 0 } else { decimals };
    step_field(ui, egui::Id::new(id), value, min, max, step, decimals, step.max(0.01) * 0.1, lower_tip, raise_tip)
}

#[cfg(test)]
mod stepper_tests {
    use super::*;

    #[test]
    fn field_fits_the_range_and_the_value() {
        assert_eq!(stepper_chars("0", "6", "3"), 1);
        assert_eq!(stepper_chars("1", "12", "1"), 2);
        assert_eq!(stepper_chars("0", "999", "128"), 3);
        // A huge range does not widen the field past 4 characters...
        assert_eq!(stepper_chars("1", "100000", "1"), 4);
        // ...but a value that needs more gets it.
        assert_eq!(stepper_chars("1", "100000", "25000"), 5);
        assert_eq!(stepper_chars("-12", "12", "0"), 3);
        assert!(stepper_field_width(3, 7.5) >= 3.0 * 7.5 + 8.0);
        assert_eq!(stepper_field_width(0, 7.0), stepper_field_width(1, 7.0));
    }

    /// Steppers drawn headlessly stay inside their border at 1, 12, 128
    /// and 9999, and the buttons move the value.
    #[test]
    fn stays_inside_its_border() {
        let ctx = egui::Context::default();
        for v0 in [1, 12, 128, 9999] {
            let mut v = v0;
            let mut outer = egui::Rect::NOTHING;
            let mut field = egui::Rect::NOTHING;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let before = ui.cursor().min;
                    let r = num_stepper(ui, "t", &mut v, 0, 9999, "-", "+");
                    field = r.rect;
                    outer = egui::Rect::from_min_size(before, egui::vec2(ui.min_rect().right() - before.x, STEP_HEIGHT));
                });
            });
            assert!(field.left() >= outer.left() + STEP_BUTTON && field.right() <= outer.right() - STEP_BUTTON + 0.5, "{v0}: field {field:?} in {outer:?}");
            assert_eq!(v, v0);
        }
    }
}

// ----- text input with presets -----

/// What a [`preset_input`] remembers between frames.
#[derive(Clone, Copy, Default)]
struct PresetState {
    open: bool,
    /// Opened by typing: the list shows the presets matching the text.
    filter: bool,
    /// The entry chosen with the arrow keys (an index into the shown ones).
    highlight: Option<usize>,
}

/// The presets of `presets` (value, label) to show for `text`: all of
/// them, or with `filter` those whose label or value has every word typed
/// (as in [`crate::combo`]'s search).
pub fn preset_matches<'a>(presets: &'a [(String, String)], text: &str, filter: bool) -> Vec<&'a (String, String)> {
    let needle = text.trim().to_lowercase();
    presets.iter().filter(|(v, l)| !filter || needle.is_empty() || crate::combo::matches(&l.to_lowercase(), &needle) || crate::combo::matches(&v.to_lowercase(), &needle)).collect()
}

/// A free-text field with presets: a text input with a small chevron
/// inside its right edge that opens the list of `presets` (value stored,
/// label shown). Typing filters the list; any text is still allowed. Up and
/// Down pick an entry, Enter takes it, Escape closes the list. Returns the
/// text's response, `changed` on typing and on a pick. Drawn in the
/// Workspace style or as a Classic field, after the active layout. With no
/// presets it is a plain text field of the same look.
pub fn preset_input(ui: &mut Ui, id: impl std::hash::Hash, text: &mut String, presets: &[(String, String)], hint: &str, width: f32) -> Response {
    let th = theme::current(ui.ctx());
    let ws = th.ws;
    let wsl = th.workspace_layout();
    let id = ui.id().with(("preset_input", id));
    let edit_id = id.with("edit");
    let mut state: PresetState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let enabled = ui.is_enabled();
    let height = if wsl { 24.0 } else { ui.spacing().interact_size.y };
    let (rect, frame_resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let visuals = ui.visuals().clone();
    let (fill, stroke, radius) = if wsl {
        (ws.well, Stroke::new(1.0_f32, if focused { ws.primary } else if enabled { ws.control } else { ws.divider }), CornerRadius::same(5))
    } else {
        let s = if focused { visuals.selection.stroke } else { visuals.widgets.inactive.bg_stroke };
        (visuals.text_edit_bg_color(), s, visuals.widgets.inactive.corner_radius)
    };
    ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
    let has_list = !presets.is_empty();
    let chevron_w = if has_list { 18.0 } else { 0.0 };
    let chevron = egui::Rect::from_min_max(egui::pos2(rect.right() - chevron_w - 1.0, rect.top() + 1.0), rect.right_bottom() - egui::vec2(1.0, 1.0));
    let mut pick: Option<String> = None;
    let shown_now = if state.open { preset_matches(presets, text, state.filter) } else { Vec::new() };
    // Keys for the list, before the text field sees them.
    if focused && state.open {
        ui.input_mut(|i| {
            let n = shown_now.len();
            if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                state.highlight = Some(state.highlight.map_or(0, |h| (h + 1).min(n - 1)));
            }
            if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                state.highlight = state.highlight.and_then(|h| h.checked_sub(1));
            }
            if let Some(h) = state.highlight.filter(|h| *h < n) {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    pick = Some(shown_now[h].0.clone());
                }
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                state.open = false;
            }
        });
    }
    let text_rect = egui::Rect::from_min_max(rect.min + egui::vec2(if wsl { 7.0 } else { 4.0 }, 0.0), egui::pos2(chevron.left() - 2.0, rect.bottom()));
    let mut edit = {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt(id.with("child")));
        let mut te = egui::TextEdit::singleline(text).id(edit_id).frame(false).margin(egui::Margin::ZERO).desired_width(text_rect.width());
        te = if wsl { te.text_color(ws.text).hint_text(RichText::new(hint).color(ws.muted)) } else { te.hint_text(hint) };
        child.add(te)
    };
    if edit.changed() && has_list {
        state.open = true;
        state.filter = true;
        state.highlight = None;
    }
    if has_list {
        let resp = ui.interact(chevron, id.with("chevron"), if enabled { Sense::click() } else { Sense::hover() });
        let hovered = enabled && resp.hovered();
        if hovered {
            ui.painter().rect_filled(chevron.shrink(1.0), CornerRadius::same(3), if wsl { ws.hover } else { visuals.widgets.hovered.weak_bg_fill });
        }
        let ink = if !enabled {
            ws.muted.gamma_multiply(0.45)
        } else if hovered || state.open {
            if wsl { ws.text } else { visuals.strong_text_color() }
        } else if wsl {
            ws.muted
        } else {
            visuals.weak_text_color()
        };
        icons::paint(ui.painter(), chevron, icons::CARET_DOWN, 11.0, ink);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.open = !state.open;
            state.filter = false;
            state.highlight = None;
            if state.open {
                ui.memory_mut(|m| m.request_focus(edit_id));
            }
        }
    }
    if state.open && has_list && pick.is_none() {
        let shown = preset_matches(presets, text, state.filter);
        // As wide as the field, or as the longest entry (up to 380px).
        let font = egui::TextStyle::Button.resolve(ui.style());
        let widest = shown.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.clone(), font.clone(), Color32::PLACEHOLDER).size().x).fold(0.0_f32, f32::max);
        let list_w = (widest + 36.0).clamp(width, width.max(380.0));
        let mut open = true;
        let popup = egui::Popup::new(id.with("popup"), ui.ctx().clone(), &frame_resp, ui.layer_id())
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(list_w)
            .gap(2.0);
        popup.show(|ui| {
            ui.set_width(list_w - 12.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                if shown.is_empty() {
                    ui.weak("—");
                }
                for (k, (value, label)) in shown.iter().enumerate() {
                    let on = state.highlight == Some(k) || *value == *text;
                    let r = ui.add(egui::Button::selectable(on, label.as_str()));
                    if state.highlight == Some(k) {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        pick = Some(value.clone());
                    }
                }
            });
        });
        // A click in the text field keeps the list (the field is part of it).
        if !open && !ui.input(|i| i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
            state.open = false;
        }
    }
    if let Some(v) = pick {
        if *text != v {
            *text = v;
            edit.mark_changed();
        }
        state = PresetState::default();
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    edit
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn typing_filters_the_presets() {
        let p: Vec<(String, String)> = ["Fixer", "Fence", "Street Doc", "Bartender"].iter().map(|s| (s.to_string(), s.to_string())).collect();
        assert_eq!(preset_matches(&p, "f", true).len(), 2, "Fixer and Fence");
        assert_eq!(preset_matches(&p, "doc street", true)[0].0, "Street Doc");
        assert_eq!(preset_matches(&p, "fixer", false).len(), 4, "opened with the chevron: every preset");
        assert_eq!(preset_matches(&p, "", true).len(), 4);
        assert!(preset_matches(&p, "decker", true).is_empty(), "free text matches nothing");
        // The stored value is searched too ("Troll (Fomori)" shown, "Fomori" stored).
        let m = vec![("Fomori".to_string(), "Troll (Fomori)".to_string())];
        assert_eq!(preset_matches(&m, "fomo", true).len(), 1);
    }
}

#[cfg(test)]
mod build_tests {
    #[test]
    fn columns_share_what_is_left() {
        assert_eq!(super::columns(500.0, &[0.0, 100.0, 50.0], 10.0), vec![330.0, 100.0, 50.0]);
        assert_eq!(super::columns(100.0, &[0.0, 100.0], 10.0)[0], 80.0, "the flexible column keeps a minimum");
    }
}
