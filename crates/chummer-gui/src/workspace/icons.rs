//! Phosphor icons (MIT, <https://phosphoricons.com>) through the
//! `egui-phosphor` font, which `theme::Theme::fonts` loads for every theme.
//! An icon is a one-character string; draw it as text in any label or
//! button, or with [`icon`] for a fixed size and colour.

use eframe::egui::{self, Color32, RichText};

pub use egui_phosphor::regular::*;

/// An icon as text of `size` points.
pub fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).size(size).color(color)
}

/// Paint an icon centred in `rect`.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, glyph: &str, size: f32, color: Color32) {
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(size), color);
}
