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

// ----- item pages, catalog and home (added for the gear and home screens) -----

/// A number field with − and + buttons, 26px high (the catalog's
/// Rating). Returns the response of the value; `changed` when a button
/// or a drag moved it.
pub fn stepper(ui: &mut Ui, value: &mut i32, min: i32, max: i32, lower_tip: &str, raise_tip: &str) -> Response {
    let ws = theme::ws(ui);
    let old = *value;
    let inner = egui::Frame::new().fill(ws.well).stroke(Stroke::new(1.0_f32, ws.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::same(0)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal(|ui| {
            let down = ui.add_enabled_ui(*value > min, |ui| icon_button(ui, icons::MINUS, 24.0)).inner.on_hover_text(lower_tip);
            if down.clicked() {
                *value -= 1;
            }
            let mut r = ui.add_sized([34.0, 24.0], egui::DragValue::new(value).range(min..=max));
            let up = ui.add_enabled_ui(*value < max, |ui| icon_button(ui, icons::PLUS, 24.0)).inner.on_hover_text(raise_tip);
            if up.clicked() {
                *value += 1;
            }
            *value = (*value).clamp(min, max);
            if *value != old {
                r.mark_changed();
            }
            r
        })
        .inner
    });
    inner.inner
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
pub fn divider(ui: &mut Ui) {
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
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(text_color)).wrap());
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
