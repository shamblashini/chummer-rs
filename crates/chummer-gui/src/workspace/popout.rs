//! Pop-out panels: any Workspace panel can move into its own OS window
//! and back.
//!
//! A panel is named by a [`PopKey`]: the document it shows and which
//! panel ([`PanelId`]). [`PopOuts`] remembers which panels are out (for
//! this session only). Draw a panel with [`Panel::show`]: docked, it is a
//! card (or an inspector section) whose header has a pop-out button;
//! popped out, it leaves a placeholder with a dock-back button, and the
//! shell draws its contents in a native window with [`window`] (an egui
//! immediate viewport). Closing that window docks the panel back.

use eframe::egui::{self, Ui};

use super::{icons, widgets, DocKey, PanelId};
use crate::theme;
use chummer_core::lang::Language;

/// A panel of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PopKey {
    pub doc: DocKey,
    pub panel: PanelId,
}

impl PopKey {
    pub fn new(doc: DocKey, panel: PanelId) -> PopKey {
        PopKey { doc, panel }
    }
}

/// The panels in their own windows, in the order they were popped out,
/// with where each window first opens (screen points; `None` lets the
/// system place it).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PopOuts {
    out: Vec<(PopKey, Option<egui::Pos2>)>,
}

impl PopOuts {
    pub fn is_out(&self, key: PopKey) -> bool {
        self.out.iter().any(|(k, _)| *k == key)
    }

    #[cfg(test)]
    pub fn pop_out(&mut self, key: PopKey) {
        self.pop_out_at(key, None);
    }

    /// Pop out with the window opening at `at`.
    pub fn pop_out_at(&mut self, key: PopKey, at: Option<egui::Pos2>) {
        if !self.is_out(key) {
            self.out.push((key, at));
        }
    }

    /// Pop out next to the main window (staggered, so windows do not
    /// open on top of each other).
    pub fn pop_out_near(&mut self, key: PopKey, ctx: &egui::Context) {
        let main = ctx.input(|i| i.viewport().outer_rect);
        let step = 28.0 * (self.out.len() as f32 + 1.0);
        self.pop_out_at(key, main.map(|r| r.min + egui::vec2(80.0 + step, 60.0 + step)));
    }

    pub fn dock(&mut self, key: PopKey) {
        self.out.retain(|(k, _)| *k != key);
    }

    /// Pop out or dock back.
    pub fn toggle(&mut self, key: PopKey, ctx: &egui::Context) {
        if self.is_out(key) {
            self.dock(key);
        } else {
            self.pop_out_near(key, ctx);
        }
    }

    /// Every panel out, in order.
    pub fn keys(&self) -> Vec<PopKey> {
        self.out.iter().map(|(k, _)| *k).collect()
    }

    /// Where a panel's window first opens.
    pub fn origin(&self, key: PopKey) -> Option<egui::Pos2> {
        self.out.iter().find(|(k, _)| *k == key).and_then(|(_, at)| *at)
    }

    /// Dock the panels of documents that were closed.
    pub fn retain_docs(&mut self, open: impl Fn(DocKey) -> bool) {
        self.out.retain(|(k, _)| open(k.doc));
    }

    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }
}

/// How a docked [`Panel`] is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// A raised card (page sections).
    Card,
    /// A section of the inspector: padding and a divider under it.
    Inspector,
    /// A titled block straight on the page, without a card (the Play
    /// screen's quick rolls and notes).
    Bare,
}

/// A panel with a header (title, extra header widgets, the pop-out
/// button) that can be popped out.
pub struct Panel<'a> {
    pub key: PopKey,
    pub title: &'a str,
    pub frame: Frame,
}

impl<'a> Panel<'a> {
    pub fn card(key: PopKey, title: &'a str) -> Self {
        Panel { key, title, frame: Frame::Card }
    }

    pub fn inspector(key: PopKey, title: &'a str) -> Self {
        Panel { key, title, frame: Frame::Inspector }
    }

    pub fn bare(key: PopKey, title: &'a str) -> Self {
        Panel { key, title, frame: Frame::Bare }
    }

    /// Draw the panel docked: `header` adds widgets right of the title
    /// (right to left), `body` the contents. When the panel is out, a
    /// placeholder replaces the body and `None` is returned.
    pub fn show<R>(self, ui: &mut Ui, pops: &mut PopOuts, lang: &Language, header: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let ws = theme::ws(ui);
        let out = pops.is_out(self.key);
        let mut toggle = false;
        let contents = |ui: &mut Ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                ui.label(widgets::title(self.title, &ws));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (glyph, tip) = if out { (icons::ARROW_SQUARE_IN, lang.tr("Dock back")) } else { (icons::ARROW_SQUARE_OUT, lang.tr("Pop out into its own window")) };
                    toggle = widgets::icon_button(ui, glyph, 22.0).on_hover_text(tip).clicked();
                    if !out {
                        header(ui);
                    }
                });
            });
            if out {
                toggle |= placeholder(ui, lang);
                None
            } else {
                ui.add_space(4.0);
                Some(body(ui))
            }
        };
        let r = match self.frame {
            Frame::Card => widgets::card_frame(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                contents(ui)
            }),
            // No `set_width` here: in a resizable side panel, rounding
            // would widen the panel a little every frame.
            Frame::Inspector => egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 12)).show(ui, contents),
            Frame::Bare => egui::Frame::new().show(ui, |ui| {
                ui.set_width(ui.available_width());
                contents(ui)
            }),
        };
        if self.frame == Frame::Inspector {
            let y = r.response.rect.bottom();
            ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0_f32, ws.divider));
        }
        if toggle {
            pops.toggle(self.key, ui.ctx());
        }
        r.inner
    }
}

/// What a docked panel shows while it is in its own window. Returns
/// true when its "Dock back" button was clicked.
pub fn placeholder(ui: &mut Ui, lang: &Language) -> bool {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.label(icons::icon(icons::ARROW_SQUARE_OUT, 13.0, ws.muted));
        ui.label(egui::RichText::new(lang.tr("Shown in its own window")).size(12.0).color(ws.muted));
        ui.add_space(8.0);
        widgets::button(ui, Some(icons::ARROW_SQUARE_IN), &lang.tr("Dock back"), widgets::Look::Secondary, 22.0).clicked()
    })
    .inner
}

/// The viewport of a popped-out panel.
pub fn viewport_id(key: PopKey) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("chummer-rs popout", key))
}

/// Show a popped-out panel in its own native window (or, where the
/// platform has a single window, an egui window). `add` draws the
/// contents with the window's `Context` (for dialogs) and `Ui`. Returns
/// true when the user docked it back (the dock-back button or closing
/// the window).
#[allow(clippy::too_many_arguments)]
pub fn window(ctx: &egui::Context, key: PopKey, title: &str, at: Option<egui::Pos2>, icon: Option<std::sync::Arc<egui::IconData>>, dock_label: &str, mut add: impl FnMut(&egui::Context, &mut Ui)) -> bool {
    // Pages get room for their tables; other panels are narrow.
    let size = match key.panel {
        PanelId::Section(_) => [960.0, 680.0],
        PanelId::Condition => [680.0, 420.0],
        _ => [440.0, 600.0],
    };
    let mut builder = egui::ViewportBuilder::default().with_title(format!("{title} — chummer-rs")).with_app_id("chummer-rs").with_inner_size(size).with_min_inner_size([260.0, 180.0]);
    if let Some(p) = at {
        // Honoured on X11, Windows and macOS; Wayland places windows itself.
        builder = builder.with_position(p);
    }
    if let Some(i) = icon {
        builder = builder.with_icon(i);
    }
    ctx.show_viewport_immediate(viewport_id(key), builder, |vctx, class| {
        let mut dock = false;
        let mut draw = |ui: &mut Ui| {
            let ws = theme::ws(ui);
            ui.horizontal(|ui| {
                ui.label(widgets::title(title, &ws));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dock = widgets::button(ui, Some(icons::ARROW_SQUARE_IN), dock_label, widgets::Look::Ghost, 22.0).clicked();
                });
            });
            ui.separator();
            add(vctx, ui);
        };
        if class == egui::ViewportClass::Embedded {
            let mut open = true;
            egui::Window::new(title).id(egui::Id::new(("popout window", key))).open(&mut open).default_size([420.0, 520.0]).show(vctx, |ui| draw(ui));
            dock |= !open;
        } else {
            let ws = theme::current(vctx).ws;
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(egui::Margin::same(10))).show(vctx, |ui| draw(ui));
            dock |= vctx.input(|i| i.viewport().close_requested());
        }
        dock
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Section;

    #[test]
    fn bookkeeping() {
        let a = PopKey::new(DocKey::Character(1), PanelId::Recent);
        let b = PopKey::new(DocKey::Character(2), PanelId::Recent);
        let c = PopKey::new(DocKey::Character(1), PanelId::Section(Section::Play));
        let mut p = PopOuts::default();
        assert!(p.is_empty());
        p.pop_out(a);
        p.pop_out(c);
        p.pop_out(a);
        assert_eq!(p.keys(), vec![a, c], "popping out twice keeps one window");
        assert!(p.is_out(a) && !p.is_out(b));
        p.pop_out(b);
        p.dock(a);
        assert_eq!(p.keys(), vec![c, b]);
        // Closing character 2 docks its panels.
        p.retain_docs(|d| d != DocKey::Character(2));
        assert_eq!(p.keys(), vec![c]);
        // Where a window opens is kept while it is out.
        let at = Some(egui::pos2(100.0, 50.0));
        p.pop_out_at(a, at);
        p.pop_out_at(a, None);
        assert_eq!((p.origin(a), p.origin(c)), (at, None));
        assert_ne!(viewport_id(a), viewport_id(b));
        assert_eq!(viewport_id(a), viewport_id(PopKey::new(DocKey::Character(1), PanelId::Recent)));
    }
}
