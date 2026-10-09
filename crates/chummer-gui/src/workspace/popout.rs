//! Pop-out panels: any Workspace panel can move into its own OS window
//! and back.
//!
//! A panel is named by a [`PopKey`]: the document it shows and which
//! panel ([`PanelId`]). [`PopOuts`] remembers which panels are out and
//! where their windows were; both are kept between sessions (eframe
//! storage), and a panel comes back out when its document opens again.
//! Draw a panel with [`Panel::show`]: docked, it is a card (or an
//! inspector section) whose header has a pop-out button; popped out, it
//! leaves a placeholder with a dock-back button, and the shell draws its
//! contents in a native window with [`window`] (an egui immediate
//! viewport). Closing that window docks the panel back. The dialogs a
//! panel opens show in its window ([`DialogHome`]).

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

/// Where a pop-out window was and how big (screen points). `pos` is
/// `None` where the system does not tell (Wayland).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub pos: Option<egui::Pos2>,
    pub size: egui::Vec2,
}

/// A document as saved between sessions: the pop-outs of a character or
/// campaign come back when that file is opened again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedDoc {
    Home,
    Campaign(std::path::PathBuf),
    Character(std::path::PathBuf),
}

impl SavedDoc {
    fn to_fields(&self) -> String {
        match self {
            SavedDoc::Home => "home".into(),
            SavedDoc::Campaign(p) => format!("campaign\t{}", p.display()),
            SavedDoc::Character(p) => format!("character\t{}", p.display()),
        }
    }

    fn from_fields(kind: &str, path: Option<&str>) -> Option<SavedDoc> {
        let path = || path.filter(|p| !p.is_empty()).map(std::path::PathBuf::from);
        match kind {
            "home" => Some(SavedDoc::Home),
            "campaign" => path().map(SavedDoc::Campaign),
            "character" => path().map(SavedDoc::Character),
            _ => None,
        }
    }
}

/// A panel out: its window's first position and size (`None`: the
/// system places it, the panel's default size).
#[derive(Debug, Clone, PartialEq)]
struct Out {
    key: PopKey,
    at: Option<egui::Pos2>,
    size: Option<egui::Vec2>,
}

/// The panels in their own windows, in the order they were popped out;
/// where each kind of panel's window was last (kept between sessions
/// with [`PopOuts::to_text`] and [`PopOuts::restore`]); and the panels
/// that were out when the app closed, popped out again when their
/// document opens.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PopOuts {
    out: Vec<Out>,
    /// By [`panel_code`].
    geometry: std::collections::BTreeMap<String, Geometry>,
    pending: Vec<(SavedDoc, PanelId)>,
}

impl PopOuts {
    pub fn is_out(&self, key: PopKey) -> bool {
        self.out.iter().any(|o| o.key == key)
    }

    #[cfg(test)]
    pub fn pop_out(&mut self, key: PopKey) {
        self.pop_out_at(key, None);
    }

    /// Pop out with the window opening at `at` (or where it was last).
    pub fn pop_out_at(&mut self, key: PopKey, at: Option<egui::Pos2>) {
        if self.is_out(key) {
            return;
        }
        let saved = self.geometry.get(&panel_code(key.panel)).copied();
        let mut at = saved.and_then(|g| g.pos).or(at);
        // Two windows of the same kind (two characters' rolls) do not
        // open on top of each other.
        if at.is_some() && self.out.iter().any(|o| o.at == at) {
            at = at.map(|p| p + egui::vec2(28.0, 28.0) * self.out.len() as f32);
        }
        self.out.push(Out { key, at, size: saved.map(|g| g.size) });
    }

    /// Pop out where the window was last, else next to the main window
    /// (staggered, so windows do not open on top of each other).
    pub fn pop_out_near(&mut self, key: PopKey, ctx: &egui::Context) {
        let (main, monitor) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().monitor_size));
        // A saved position on a monitor that is gone would open the
        // window out of reach.
        if let (Some(g), Some(m)) = (self.geometry.get_mut(&panel_code(key.panel)), monitor) {
            if g.pos.is_some_and(|p| !on_screen(p, m)) {
                g.pos = None;
            }
        }
        let step = 28.0 * (self.out.len() as f32 + 1.0);
        self.pop_out_at(key, main.map(|r| r.min + egui::vec2(80.0 + step, 60.0 + step)));
    }

    pub fn dock(&mut self, key: PopKey) {
        self.out.retain(|o| o.key != key);
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
        self.out.iter().map(|o| o.key).collect()
    }

    /// Where a panel's window first opens.
    pub fn origin(&self, key: PopKey) -> Option<egui::Pos2> {
        self.out.iter().find(|o| o.key == key).and_then(|o| o.at)
    }

    /// The size a panel's window first opens with (`None`: its default).
    pub fn first_size(&self, key: PopKey) -> Option<egui::Vec2> {
        self.out.iter().find(|o| o.key == key).and_then(|o| o.size)
    }

    /// Note where a window is now (the position only when known).
    pub fn remember(&mut self, key: PopKey, now: Geometry) {
        let g = self.geometry.entry(panel_code(key.panel)).or_insert(now);
        g.size = now.size;
        if now.pos.is_some() {
            g.pos = now.pos;
        }
    }

    /// Where the windows of a kind of panel were last.
    #[cfg(test)]
    pub fn geometry(&self, panel: PanelId) -> Option<Geometry> {
        self.geometry.get(&panel_code(panel)).copied()
    }

    /// Dock the panels of documents that were closed.
    pub fn retain_docs(&mut self, open: impl Fn(DocKey) -> bool) {
        self.out.retain(|o| open(o.key.doc));
    }

    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }

    // ----- between sessions -----

    /// The text kept between sessions: each kind of window's last
    /// geometry, and the panels out (`saved` names their documents; one
    /// never saved is left out), with those still waiting for their
    /// document from the last session.
    pub fn to_text(&self, saved: impl Fn(DocKey) -> Option<SavedDoc>) -> String {
        let mut lines = Vec::new();
        for (code, g) in &self.geometry {
            let (x, y) = g.pos.map_or(("-".to_owned(), "-".to_owned()), |p| (p.x.round().to_string(), p.y.round().to_string()));
            lines.push(format!("geometry\t{code}\t{x}\t{y}\t{}\t{}", g.size.x.round(), g.size.y.round()));
        }
        let out = self.out.iter().filter_map(|o| saved(o.key.doc).map(|d| (d, o.key.panel)));
        for (doc, panel) in out.chain(self.pending.iter().cloned()) {
            lines.push(format!("out\t{}\t{}", panel_code(panel), doc.to_fields()));
        }
        lines.join("\n")
    }

    /// Take back what [`PopOuts::to_text`] saved (unknown lines are
    /// skipped).
    pub fn restore(&mut self, text: &str) {
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["geometry", code, x, y, w, h] => {
                    let (Some(_), Ok(w), Ok(h)) = (parse_panel(code), w.parse::<f32>(), h.parse::<f32>()) else { continue };
                    let pos = match (x.parse::<f32>(), y.parse::<f32>()) {
                        (Ok(x), Ok(y)) => Some(egui::pos2(x, y)),
                        _ => None,
                    };
                    if w >= 1.0 && h >= 1.0 {
                        self.geometry.insert((*code).to_owned(), Geometry { pos, size: egui::vec2(w, h) });
                    }
                }
                ["out", code, kind, rest @ ..] => {
                    if let (Some(panel), Some(doc)) = (parse_panel(code), SavedDoc::from_fields(kind, rest.first().copied())) {
                        if !self.pending.contains(&(doc.clone(), panel)) {
                            self.pending.push((doc, panel));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// The panels of `doc` that were out when the app closed (once).
    pub fn take_pending(&mut self, doc: &SavedDoc) -> Vec<PanelId> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let (mine, rest) = std::mem::take(&mut self.pending).into_iter().partition(|(d, _)| d == doc);
        self.pending = rest;
        mine.into_iter().map(|(_, p)| p).collect()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Whether a saved window position is still on a screen of about
/// `monitor`'s size (generous: other monitors may sit around it).
fn on_screen(p: egui::Pos2, monitor: egui::Vec2) -> bool {
    p.x > -monitor.x && p.y > -monitor.y && p.x < 2.0 * monitor.x && p.y < 2.0 * monitor.y
}

/// The blocks of the Play screen.
const PLAY_PANELS: [crate::view::play::Panel; 10] = {
    use crate::view::play::Panel as P;
    [P::Condition, P::Initiative, P::Rolls, P::Weapons, P::AtHand, P::Matrix, P::Vehicles, P::Notes, P::Roller, P::Log]
};

/// The parts of the GM screen.
const GM_PANELS: [crate::gm_screen::workspace::Panel; 6] = {
    use crate::gm_screen::workspace::Panel as G;
    [G::Encounter, G::Card, G::Players, G::Rolls, G::Award, G::Notes]
};

/// Every panel that can pop out.
fn all_panels() -> Vec<PanelId> {
    use super::Section;
    let mut out: Vec<PanelId> = [Section::Play, Section::History, Section::Home, Section::DataBrowser, Section::Campaign].into_iter().map(PanelId::Section).collect();
    out.extend(crate::view::TABS.iter().map(|(t, _)| PanelId::Section(Section::Page(*t))));
    out.extend((0..8).map(|i| PanelId::Section(Section::Gear(i))));
    out.extend([PanelId::Issues, PanelId::Item, PanelId::Summary, PanelId::Recent, PanelId::Selected, PanelId::Ledger, PanelId::Condition, PanelId::Dice, PanelId::Initiative, PanelId::Activity]);
    out.extend(PLAY_PANELS.iter().map(|p| PanelId::Play(*p)));
    out.extend(GM_PANELS.iter().map(|p| PanelId::Gm(*p)));
    out
}

/// A panel's name in the saved text, e.g. `Section(Page(Skills))`.
pub fn panel_code(p: PanelId) -> String {
    format!("{p:?}")
}

/// The panel [`panel_code`] names.
pub fn parse_panel(code: &str) -> Option<PanelId> {
    all_panels().into_iter().find(|p| panel_code(*p) == code)
}

/// Which window a document's dialogs show in (confirmations, the
/// selection dialog, the editors drawn at the end of its frame).
///
/// A dialog opens where the user was working: each window of the
/// document reports its presses with [`DialogHome::track`], and while
/// no dialog is open the last one wins. An open dialog stays put. The
/// shell checks the window still exists with [`DialogHome::resolve`];
/// dialogs draw only in the window for which [`DialogHome::here`] holds.
/// The default (`None`) is the main window, so the Classic layout draws
/// them where it always did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DialogHome(Option<egui::ViewportId>);

impl DialogHome {
    /// The window the dialogs show in.
    pub fn viewport(self) -> egui::ViewportId {
        self.0.unwrap_or(egui::ViewportId::ROOT)
    }

    /// Whether dialogs draw in the window `ctx` is drawing.
    pub fn here(self, ctx: &egui::Context) -> bool {
        self.viewport() == ctx.viewport_id()
    }

    /// A press (pointer or key) in window `vp`: with no dialog open, the
    /// next one opens there.
    pub fn press(&mut self, vp: egui::ViewportId, dialog_open: bool) {
        if !dialog_open {
            self.0 = Some(vp);
        }
    }

    /// Record a press in the window `ctx` is drawing (call at the start
    /// of the window's frame, before its widgets run).
    pub fn track(&mut self, ctx: &egui::Context, dialog_open: bool) {
        if pressed(ctx) {
            self.press(ctx.viewport_id(), dialog_open);
        }
    }

    /// Keep the window if it is still shown (`shown`), else move the
    /// dialogs to `fallback` (a closed pop-out's dialogs come back to the
    /// main window, or to the document's first pop-out when it is behind
    /// another document).
    pub fn resolve(&mut self, shown: impl Fn(egui::ViewportId) -> bool, fallback: egui::ViewportId) {
        if !shown(self.viewport()) {
            self.0 = Some(fallback);
        }
    }
}

/// Whether the window `ctx` is drawing got a click or a key press this
/// frame.
fn pressed(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })))
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

/// What [`window`] reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    /// The user docked it back (the dock-back button or closing the
    /// window).
    pub docked: bool,
    /// Where the native window is now (`None` when it is an egui window
    /// inside the main one).
    pub geometry: Option<Geometry>,
}

/// A panel's window size when nothing was saved: pages get room for
/// their tables, other panels are narrow.
fn default_size(panel: PanelId) -> egui::Vec2 {
    match panel {
        PanelId::Section(_) => egui::vec2(960.0, 680.0),
        PanelId::Condition => egui::vec2(680.0, 420.0),
        _ => egui::vec2(440.0, 600.0),
    }
}

/// Show a popped-out panel in its own native window (or, where the
/// platform has a single window, an egui window). `at` and `size` are
/// where it first opens (`None`: the system places it; the panel's
/// default size); they only apply when the window is created, so the
/// builder stays the same every frame and egui sends the window no
/// commands. `add` draws the contents with the window's `Context` (its
/// dialogs show in it) and `Ui`.
///
/// Platforms: Wayland ignores `at` and does not report positions (only
/// the size is kept); closing the window from the title bar or the
/// compositor docks the panel (`close_requested`); sizes are in points,
/// so a HiDPI monitor scales them like the main window.
#[allow(clippy::too_many_arguments)]
pub fn window(ctx: &egui::Context, key: PopKey, title: &str, at: Option<egui::Pos2>, size: Option<egui::Vec2>, icon: Option<std::sync::Arc<egui::IconData>>, dock_label: &str, mut add: impl FnMut(&egui::Context, &mut Ui)) -> Shown {
    let size = size.unwrap_or_else(|| default_size(key.panel)).max(egui::vec2(260.0, 180.0));
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
            Shown { docked: dock || !open, geometry: None }
        } else {
            let ws = theme::current(vctx).ws;
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(egui::Margin::same(10))).show(vctx, |ui| draw(ui));
            let (close, outer, inner) = vctx.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
            Shown { docked: dock || close, geometry: inner.map(|r| Geometry { pos: outer.map(|o| o.min), size: r.size() }) }
        }
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
    #[test]
    fn panel_codes_round_trip() {
        let all = all_panels();
        for p in &all {
            assert_eq!(parse_panel(&panel_code(*p)), Some(*p), "{p:?}");
        }
        let codes: std::collections::HashSet<String> = all.iter().map(|p| panel_code(*p)).collect();
        assert_eq!(codes.len(), all.len(), "codes are unique");
        assert_eq!(parse_panel("Section(Page(Nope))"), None);
    }

    #[test]
    fn geometry_is_kept_between_sessions() {
        let doc = DocKey::Character(7);
        let rolls = PopKey::new(doc, PanelId::Play(crate::view::play::Panel::Log));
        let mut p = PopOuts::default();
        p.pop_out(rolls);
        assert_eq!(p.first_size(rolls), None, "the panel's default size the first time");
        p.remember(rolls, Geometry { pos: Some(egui::pos2(1200.0, 80.0)), size: egui::vec2(420.0, 640.0) });
        // Wayland: no position; the size still counts, the last known
        // position stays.
        p.remember(rolls, Geometry { pos: None, size: egui::vec2(430.0, 650.0) });
        let g = Geometry { pos: Some(egui::pos2(1200.0, 80.0)), size: egui::vec2(430.0, 650.0) };
        assert_eq!(p.geometry(rolls.panel), Some(g));

        let path = std::path::PathBuf::from("/tmp/Apex.chum5");
        let text = p.to_text(|d| (d == doc).then(|| SavedDoc::Character(path.clone())));
        let mut next = PopOuts::default();
        next.restore(&text);
        assert_eq!(next.geometry(rolls.panel), Some(g));
        assert!(next.is_empty() && next.has_pending());
        // Another character opens: nothing for it.
        assert!(next.take_pending(&SavedDoc::Character("/tmp/Other.chum5".into())).is_empty());
        // The character opens (with a new id): its window comes back
        // where it was, as big as it was.
        let panels = next.take_pending(&SavedDoc::Character(path.clone()));
        assert_eq!(panels, vec![rolls.panel]);
        assert!(!next.has_pending(), "once");
        let again = PopKey::new(DocKey::Character(9), panels[0]);
        next.pop_out_at(again, Some(egui::pos2(10.0, 10.0)));
        assert_eq!((next.origin(again), next.first_size(again)), (g.pos, Some(g.size)));
    }

    #[test]
    fn pending_panels_survive_a_save_before_their_document_opens() {
        let mut p = PopOuts::default();
        p.restore("out\tDice\thome\nout\tGm(Card)\tcampaign\t/c/Run.chum5campaign\nbogus line\ngeometry\tNope\t1\t2\t3\t4\ngeometry\tDice\t-\t-\t300\t200");
        assert_eq!(p.geometry(PanelId::Dice), Some(Geometry { pos: None, size: egui::vec2(300.0, 200.0) }));
        assert_eq!(p.take_pending(&SavedDoc::Home), vec![PanelId::Dice]);
        // Saved again before the campaign opened: it is still pending.
        let text = p.to_text(|_| None);
        let mut next = PopOuts::default();
        next.restore(&text);
        assert_eq!(next.take_pending(&SavedDoc::Campaign("/c/Run.chum5campaign".into())), vec![PanelId::Gm(crate::gm_screen::workspace::Panel::Card)]);
        // A document never saved has nothing to come back to.
        let mut q = PopOuts::default();
        q.pop_out(PopKey::new(DocKey::Character(1), PanelId::Recent));
        assert!(!q.to_text(|_| None).contains("out\t"));
    }

    #[test]
    fn windows_of_one_kind_do_not_stack() {
        let mut p = PopOuts::default();
        p.restore("geometry\tRecent\t500\t300\t400\t500");
        let a = PopKey::new(DocKey::Character(1), PanelId::Recent);
        let b = PopKey::new(DocKey::Character(2), PanelId::Recent);
        p.pop_out(a);
        p.pop_out(b);
        assert_eq!(p.origin(a), Some(egui::pos2(500.0, 300.0)));
        assert_ne!(p.origin(b), p.origin(a));
        assert!(on_screen(egui::pos2(500.0, 300.0), egui::vec2(1920.0, 1080.0)));
        assert!(!on_screen(egui::pos2(9000.0, 300.0), egui::vec2(1920.0, 1080.0)));
    }

    #[test]
    fn dialogs_open_where_the_user_works() {
        let main = egui::ViewportId::ROOT;
        let pop = viewport_id(PopKey::new(DocKey::Character(1), PanelId::Item));
        let mut h = DialogHome::default();
        assert_eq!(h.viewport(), main, "the main window by default (Classic)");
        // A click in the pop-out: the next dialog opens there...
        h.press(pop, false);
        assert_eq!(h.viewport(), pop);
        // ...and stays there while it is open, whatever is clicked.
        h.press(main, true);
        assert_eq!(h.viewport(), pop);
        // The pop-out is docked: back to the main window.
        h.resolve(|vp| vp == main, main);
        assert_eq!(h.viewport(), main);
        // A document behind another one: its first pop-out.
        h.resolve(|vp| vp == pop, pop);
        assert_eq!(h.viewport(), pop);
        h.resolve(|vp| vp == pop, main);
        assert_eq!(h.viewport(), pop, "a window still shown is kept");
    }
}
