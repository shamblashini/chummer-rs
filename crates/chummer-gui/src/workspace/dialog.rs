//! Dialog windows in both layouts. In the Workspace a dialog is a raised
//! card with its own header (title, close button) and a rule under it,
//! with Workspace inputs, list rows and buttons; in Classic it is the
//! plain egui window and widgets it always was. Dialogs call these
//! helpers instead of `egui::Window` and the plain widgets, so their logic
//! is written once.

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Ui, Vec2};

use super::icons;
use super::widgets::{self, Look};
use crate::theme;

/// Whether the current theme is a Workspace one.
pub fn ws(ctx: &egui::Context) -> bool {
    theme::current(ctx).workspace_layout()
}

/// Room a tool window keeps from the app window's edges.
const SCREEN_MARGIN: f32 = 16.0;

/// Room a window's frame and title bar take around its contents.
const WINDOW_CHROME: Vec2 = Vec2::new(32.0, 64.0);

/// The largest contents a tool window may have: the app window less a
/// margin and the window's own frame and title bar.
pub fn max_window_size(ctx: &egui::Context) -> Vec2 {
    (ctx.content_rect().size() - Vec2::splat(SCREEN_MARGIN * 2.0) - WINDOW_CHROME).max(egui::vec2(240.0, 160.0))
}

/// Make a tool window resizable and fit it to the app window: it opens at
/// `default` (smaller when the app window is small), cannot be made
/// smaller than `min` nor larger than the app window. With `scroll` its
/// contents scroll when they do not fit; without, the contents arrange
/// their own scrolling (see [`with_footer`]).
pub fn fit<'a>(ctx: &egui::Context, w: egui::Window<'a>, default: Vec2, min: Vec2, scroll: bool) -> egui::Window<'a> {
    let max = max_window_size(ctx);
    let min = min.min(max);
    let default = default.max(min).min(max);
    w.resizable(true).default_size(default).min_size(min).max_size(max).constrain(true).vscroll(scroll)
}

/// A window's body that scrolls above a `footer` that stays put (its
/// action buttons are always visible). Fills the window; use it in a
/// window [`fit`] without `scroll`.
pub fn with_footer<R>(ui: &mut Ui, id: impl std::hash::Hash, footer: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> R {
    let id = egui::Id::new(id);
    egui::TopBottomPanel::bottom(id.with("footer")).frame(egui::Frame::NONE).show_separator_line(false).show_inside(ui, |ui| {
        rule(ui);
        footer(ui);
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| egui::ScrollArea::vertical().id_salt(id.with("body")).auto_shrink([false, false]).show(ui, body).inner)
        .inner
}

/// A dialog window `title`. `open` becomes false when it is closed with
/// its close button (and, in the Workspace, Escape). `size` is the
/// default size; a height of 0 makes the dialog as tall as its contents
/// (it scrolls when they are taller than the app window). Every dialog
/// can be resized and fits the app window ([`fit`]); `fills` dialogs lay
/// out and scroll their contents themselves, the others scroll as a whole.
pub fn window<R>(ctx: &egui::Context, id: impl std::hash::Hash, title: &str, open: &mut bool, size: Vec2, fills: bool, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let id = egui::Id::new(id);
    let auto_height = size.y <= 0.0;
    let min = egui::vec2(size.x.min(320.0), if fills { 240.0 } else { 80.0 });
    let fitted = |w: egui::Window<'static>| {
        if auto_height {
            // As tall as the contents (up to the app window): only the
            // width can be dragged; the contents scroll below.
            fit(ctx, w, egui::vec2(size.x, f32::INFINITY), min, false).resizable([true, false])
        } else {
            fit(ctx, w, size, min, !fills)
        }
    };
    // Contents as tall as they are, up to the app window's height.
    let max_body = max_window_size(ctx).y - 30.0;
    let add = |ui: &mut Ui| {
        if auto_height {
            egui::ScrollArea::vertical().id_salt("dialog_body").max_height(max_body).auto_shrink([false, true]).show(ui, add).inner
        } else {
            add(ui)
        }
    };
    if !ws(ctx) {
        let w = fitted(egui::Window::new(title.to_owned()).id(id).collapsible(false));
        return w.open(open).show(ctx, add).and_then(|r| r.inner);
    }
    let pal = theme::current(ctx).ws;
    let frame = egui::Frame::new()
        .fill(pal.raised)
        .stroke(Stroke::new(1.0_f32, pal.divider))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::same(14))
        .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    let mut close = false;
    let w = fitted(egui::Window::new(title.to_owned()).id(id).title_bar(false).frame(frame).collapsible(false));
    let r = w.show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(widgets::bold(15.0)).color(pal.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::X, 24.0).on_hover_text("Esc").clicked() {
                    close = true;
                }
            });
        });
        widgets::hline(ui, pal.divider);
        ui.add_space(6.0);
        add(ui)
    });
    // Escape closes the topmost Workspace dialog, as a native one would.
    if r.as_ref().is_some_and(|r| ctx.top_layer_id() == Some(r.response.layer_id)) && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if close {
        *open = false;
    }
    r.and_then(|r| r.inner)
}

/// A modal dialog (it blocks the window behind it) `width` wide: the
/// plain egui modal in Classic, a raised card in the Workspace.
pub fn modal<R>(ctx: &egui::Context, id: impl std::hash::Hash, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut m = egui::Modal::new(egui::Id::new(id));
    if ws(ctx) {
        let pal = theme::current(ctx).ws;
        m = m.frame(egui::Frame::new().fill(pal.raised).stroke(Stroke::new(1.0_f32, pal.divider)).corner_radius(CornerRadius::same(9)).inner_margin(egui::Margin::same(16)));
    }
    m.show(ctx, |ui| {
        ui.set_width(width);
        add(ui)
    })
    .inner
}

/// A section heading inside a dialog.
pub fn heading(ui: &mut Ui, text: &str) -> egui::Response {
    if ws(ui.ctx()) {
        let pal = theme::ws(ui);
        ui.label(RichText::new(text).font(widgets::bold(13.5)).color(pal.text))
    } else {
        ui.heading(text)
    }
}

/// Muted explanatory text.
pub fn note(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    if ws(ui.ctx()) {
        let pal = theme::ws(ui);
        ui.label(RichText::new(text.into()).size(12.0).color(pal.muted))
    } else {
        ui.weak(text.into())
    }
}

/// A field caption ("Name", "Category").
pub fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    if ws(ui.ctx()) {
        ui.label(widgets::overline(text, &theme::ws(ui)))
    } else {
        ui.label(text)
    }
}

/// A form label ("Name:"): muted in the Workspace.
pub fn label(ui: &mut Ui, text: &str) -> egui::Response {
    if ws(ui.ctx()) {
        let pal = theme::ws(ui);
        ui.label(RichText::new(text).size(12.0).color(pal.muted))
    } else {
        ui.label(text)
    }
}

/// A warning line (unmet requirements).
pub fn warning(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let c = if ws(ui.ctx()) { theme::ws(ui).warning } else { ui.visuals().warn_fg_color };
    ui.colored_label(c, text.into())
}

/// A one-line text input with a hint.
pub fn text_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    if ws(ui.ctx()) {
        widgets::text_field(ui, text, hint, width)
    } else {
        ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width))
    }
}

/// The search field of a picker (with a magnifier in the Workspace).
pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    if !ws(ui.ctx()) {
        return ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width));
    }
    let pal = theme::ws(ui);
    let mut r = None;
    egui::Frame::new().fill(pal.well).stroke(Stroke::new(1.0_f32, pal.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(icons::icon(icons::MAGNIFYING_GLASS, 13.0, pal.muted));
            r = Some(ui.add(egui::TextEdit::singleline(text).frame(false).hint_text(hint).desired_width(width - 32.0)));
        });
    });
    r.expect("drawn")
}

/// A multi-line text input, its text in `color`.
pub fn text_area(ui: &mut Ui, text: &mut String, rows: usize, color: Color32) -> egui::Response {
    if !ws(ui.ctx()) {
        return ui.add(egui::TextEdit::multiline(text).text_color(color).desired_rows(rows).desired_width(f32::INFINITY));
    }
    let pal = theme::ws(ui);
    let mut r = None;
    egui::Frame::new().fill(pal.well).stroke(Stroke::new(1.0_f32, pal.control)).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        r = Some(ui.add(egui::TextEdit::multiline(text).frame(false).text_color(color).desired_rows(rows).desired_width(f32::INFINITY)));
    });
    r.expect("drawn")
}

/// A colour swatch button (opens a colour picker).
pub fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
    if !ws(ui.ctx()) {
        return ui.button(RichText::new("⏹").color(color));
    }
    let pal = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let border = if enabled && resp.hovered() { pal.control } else { pal.divider };
        ui.painter().rect(rect, CornerRadius::same(5), pal.raised, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
        ui.painter().rect_filled(rect.shrink(5.0), CornerRadius::same(3), if enabled { color } else { color.gamma_multiply(0.4) });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One entry of a picker's list: `text`, a muted `extra` (a cost), dimmed
/// when it cannot be taken. Click selects, double click takes it.
pub fn list_row(ui: &mut Ui, selected: bool, text: &str, extra: &str, dimmed: bool) -> egui::Response {
    if !ws(ui.ctx()) {
        let mut t = RichText::new(if extra.is_empty() { text.to_owned() } else { format!("{text}   {extra}") });
        if dimmed {
            t = t.weak();
        }
        return crate::combo::selectable_label(ui, selected, t);
    }
    let pal = theme::ws(ui);
    let height = 24.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            pal.selection
        } else if resp.hovered() {
            pal.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
        if selected {
            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(3.0, height)), CornerRadius::same(1), pal.primary);
        }
        let ink = if dimmed { pal.muted.gamma_multiply(0.7) } else { pal.text };
        let left = egui::pos2(rect.left() + 10.0, rect.center().y);
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(left, egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), ink);
        if !extra.is_empty() {
            painter.text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, extra, egui::FontId::monospace(11.0), pal.muted);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A dialog button: filled when `primary` (the main action).
pub fn button(ui: &mut Ui, text: &str, primary: bool) -> egui::Response {
    if ws(ui.ctx()) {
        widgets::button(ui, None, text, if primary { Look::Primary } else { Look::Secondary }, 26.0)
    } else {
        ui.button(text)
    }
}

/// A check box.
pub fn check(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    if ws(ui.ctx()) {
        widgets::check(ui, on, label)
    } else {
        ui.checkbox(on, label)
    }
}

/// The row of buttons at the bottom of a dialog, after a rule.
pub fn buttons<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if ws(ui.ctx()) {
        ui.add_space(6.0);
        widgets::hline(ui, theme::ws(ui).divider);
        ui.add_space(6.0);
    } else {
        ui.separator();
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui)
    })
    .inner
}

/// A rule between parts of a dialog.
pub fn rule(ui: &mut Ui) {
    if ws(ui.ctx()) {
        ui.add_space(4.0);
        widgets::hline(ui, theme::ws(ui).divider);
        ui.add_space(4.0);
    } else {
        ui.separator();
    }
}

/// A sunken panel for a list (the Workspace well), or nothing in Classic.
pub fn list_frame(ui: &Ui) -> egui::Frame {
    if ws(ui.ctx()) {
        let pal = theme::ws(ui);
        egui::Frame::new().fill(pal.well).stroke(Stroke::new(1.0_f32, pal.divider)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(4))
    } else {
        egui::Frame::NONE
    }
}
