//! The command palette (Ctrl+K): one search over commands (the menu's
//! actions), the open document's sections, the open character's items,
//! the open documents and every game-data record (the Master Index).
//!
//! The shell gives the palette its [`Entry`] list each frame it is open;
//! the palette filters and ranks them with [`rank`] (built on
//! `crate::combo`'s word matching), and returns the [`Target`] picked with
//! Enter or a click. Arrow keys move the selection; the line under the
//! list previews the selected entry.

use std::sync::Arc;

use chummer_core::data::{self, DataStore};
use chummer_core::lang::Language;
use eframe::egui::{self, Color32, CornerRadius, FontId, Key, RichText, Sense, Stroke};

use super::{icons, widgets, DocKey, Section};
use crate::theme;

/// A menu action the palette (and the Workspace buttons) can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cmd {
    NewCharacter,
    NewCritter,
    Open,
    Save,
    SaveAs,
    Print,
    Export,
    Close,
    NewCampaign,
    OpenCampaign,
    JoinCampaign,
    Undo,
    Redo,
    DiceRoller,
    Initiative,
    MasterIndex,
    Roster,
    CharacterSettings,
    Sourcebooks,
    OnlineSettings,
    Dark,
    Light,
    ClassicLayout,
    GuidedCreation,
    About,
    Exit,
}

impl Cmd {
    pub const ALL: [Cmd; 26] = [
        Cmd::NewCharacter,
        Cmd::NewCritter,
        Cmd::Open,
        Cmd::Save,
        Cmd::SaveAs,
        Cmd::Print,
        Cmd::Export,
        Cmd::Close,
        Cmd::NewCampaign,
        Cmd::OpenCampaign,
        Cmd::JoinCampaign,
        Cmd::Undo,
        Cmd::Redo,
        Cmd::DiceRoller,
        Cmd::Initiative,
        Cmd::MasterIndex,
        Cmd::Roster,
        Cmd::CharacterSettings,
        Cmd::Sourcebooks,
        Cmd::OnlineSettings,
        Cmd::Dark,
        Cmd::Light,
        Cmd::ClassicLayout,
        Cmd::GuidedCreation,
        Cmd::About,
        Cmd::Exit,
    ];

    /// The menu label (English; goes through `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            Cmd::NewCharacter => "New Character…",
            Cmd::NewCritter => "New Critter…",
            Cmd::Open => "Open…",
            Cmd::Save => "Save",
            Cmd::SaveAs => "Save As…",
            Cmd::Print => "Print…",
            Cmd::Export => "Export…",
            Cmd::Close => "Close",
            Cmd::NewCampaign => "New Campaign",
            Cmd::OpenCampaign => "Open Campaign…",
            Cmd::JoinCampaign => "Join Campaign…",
            Cmd::Undo => "Undo",
            Cmd::Redo => "Redo",
            Cmd::DiceRoller => "Dice Roller",
            Cmd::Initiative => "Initiative tracker",
            Cmd::MasterIndex => "Master Index",
            Cmd::Roster => "Character Roster",
            Cmd::CharacterSettings => "Character Settings…",
            Cmd::Sourcebooks => "Sourcebooks (PDFs)…",
            Cmd::OnlineSettings => "Online Settings…",
            Cmd::Dark => "Dark",
            Cmd::Light => "Light",
            Cmd::ClassicLayout => "Classic",
            Cmd::GuidedCreation => "Guided creation",
            Cmd::About => "About",
            Cmd::Exit => "Exit",
        }
    }

    /// Where the command lives, shown under it (English; `lang.tr`).
    pub fn menu(self) -> &'static str {
        match self {
            Cmd::Undo | Cmd::Redo => "Edit",
            Cmd::DiceRoller | Cmd::Initiative | Cmd::MasterIndex | Cmd::Roster | Cmd::CharacterSettings | Cmd::Sourcebooks | Cmd::OnlineSettings => "Tools",
            Cmd::Dark | Cmd::Light | Cmd::ClassicLayout | Cmd::GuidedCreation => "View",
            Cmd::About => "Help",
            _ => "File",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Cmd::NewCharacter => "Ctrl+N",
            Cmd::Open => "Ctrl+O",
            Cmd::Save => "Ctrl+S",
            Cmd::Print => "Ctrl+P",
            Cmd::Close => "Ctrl+W",
            Cmd::Undo => "Ctrl+Z",
            Cmd::Redo => "Ctrl+Y",
            Cmd::Exit => "Ctrl+Q",
            _ => "",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Cmd::NewCharacter => icons::USER_PLUS,
            Cmd::NewCritter => icons::PAW_PRINT,
            Cmd::Open | Cmd::OpenCampaign => icons::FOLDER_OPEN,
            Cmd::Save | Cmd::SaveAs => icons::FLOPPY_DISK,
            Cmd::Print => icons::PRINTER,
            Cmd::Export => icons::EXPORT,
            Cmd::Close => icons::X,
            Cmd::NewCampaign | Cmd::JoinCampaign => icons::USERS_THREE,
            Cmd::Undo => icons::ARROW_U_UP_LEFT,
            Cmd::Redo => icons::ARROW_U_UP_RIGHT,
            Cmd::DiceRoller => icons::DICE_FIVE,
            Cmd::Initiative => icons::LIST_NUMBERS,
            Cmd::MasterIndex => icons::DATABASE,
            Cmd::Roster => icons::HOUSE,
            Cmd::CharacterSettings => icons::SLIDERS,
            Cmd::Sourcebooks => icons::BOOK_OPEN,
            Cmd::OnlineSettings => icons::CLOUD_CHECK,
            Cmd::Dark => icons::MOON,
            Cmd::Light => icons::SUN,
            Cmd::ClassicLayout => icons::CIRCLE_HALF,
            Cmd::GuidedCreation => icons::LIGHTBULB,
            Cmd::About => icons::INFO,
            Cmd::Exit => icons::SIGN_OUT,
        }
    }
}

/// What an entry does when picked.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Command(Cmd),
    /// Go to a section of the open document.
    Section(Section),
    /// Show an item of the open character in the inspector.
    Item { section: Section, guid: String },
    /// Bring an open document to the front.
    Document(DocKey),
    /// A game-data record in the Master Index: (`data::BROWSABLE` index,
    /// record index).
    Record { kind: usize, index: usize },
}

/// Kinds of entries, in the order they rank on ties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Command,
    Navigate,
    Document,
    Item,
    Record,
}

/// One line of the palette.
#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub icon: &'static str,
    pub title: String,
    /// The second line ("Edit · Ctrl+Z", "Cyberware & Bioware", the
    /// record's category and source).
    pub detail: String,
    /// Right-aligned hint (a shortcut, a rating).
    pub hint: String,
    /// More words that find the entry without being shown.
    pub keywords: String,
    pub enabled: bool,
    pub target: Target,
}

/// How well `text` matches the search: `None` when some word is
/// missing, else lower is better (whole-text prefix, then a word starting
/// with the first search word, then anywhere; earlier is better).
pub fn rank(text: &str, needle: &str) -> Option<u32> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    let text = text.to_lowercase();
    if !crate::combo::matches(&text, &needle) {
        return None;
    }
    let first = needle.split_whitespace().next().unwrap_or_default();
    let quality = if text.starts_with(&needle) {
        0
    } else if text.starts_with(first) {
        1
    } else if text.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(first)) {
        2
    } else {
        3
    };
    Some(quality * 1000 + text.find(first).unwrap_or(0).min(999) as u32)
}

/// The entries matching `needle`, best first: by [`rank`] of the title
/// (keywords count, one step worse), then kind, then shorter titles.
/// Disabled entries go last. With nothing typed, entries keep their
/// order, grouped by kind. At most `limit`.
pub fn search(entries: &[Entry], needle: &str, limit: usize) -> Vec<Entry> {
    let mut hits: Vec<(u32, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            let by_title = rank(&e.title, needle);
            let by_all = || rank(&format!("{} {} {}", e.title, e.detail, e.keywords), needle).map(|r| r + 4000);
            by_title.or_else(by_all).map(|r| (r, e))
        })
        .collect();
    if needle.trim().is_empty() {
        // Nothing typed yet: everything in its own order, by kind.
        hits.sort_by_key(|(_, e)| e.kind);
    } else {
        hits.sort_by_key(|(r, e)| (!e.enabled, *r, e.kind, e.title.len()));
    }
    hits.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
}

/// Every game-data record as palette entries (built once, on the first
/// search long enough to need it).
pub fn record_entries(store: &DataStore, lang: &Language) -> Vec<Entry> {
    let mut out = Vec::new();
    for (kind, (label, file, container, item)) in data::BROWSABLE.iter().enumerate() {
        let Ok(doc) = store.doc(file) else { continue };
        let kind_label = lang.tr(label);
        for (index, r) in data::records(&doc, container, item).iter().enumerate() {
            let name = lang.data_name(file, &r.id(), &r.name());
            let cat = r.category();
            let mut detail = kind_label.clone();
            if !cat.is_empty() {
                detail.push_str(" · ");
                detail.push_str(&lang.data_name(file, "", &cat));
            }
            let source = r.source();
            out.push(Entry {
                kind: Kind::Record,
                icon: icons::DATABASE,
                keywords: if name != r.name() { r.name() } else { String::new() },
                title: name,
                detail,
                hint: source,
                enabled: true,
                target: Target::Record { kind, index },
            });
        }
    }
    out
}

/// The palette's state between frames.
#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    /// Focus goes to the search field on the next frame.
    focus: bool,
    /// Game-data records, once built.
    records: Option<Arc<Vec<Entry>>>,
    /// The last search: (query, number of entries) → results.
    cache: Option<((String, usize), Vec<Entry>)>,
}

/// Searches shorter than this skip the game data.
const RECORDS_FROM: usize = 2;
const SHOWN: usize = 40;
const ROW: f32 = 42.0;

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
        self.query.clear();
        self.selected = 0;
        self.cache = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the record index is needed and not built yet.
    pub fn wants_records(&self) -> bool {
        self.open && self.records.is_none() && self.query.trim().chars().count() >= RECORDS_FROM
    }

    pub fn set_records(&mut self, records: Vec<Entry>) {
        self.records = Some(Arc::new(records));
        self.cache = None;
    }

    fn results(&mut self, entries: &[Entry]) -> Vec<Entry> {
        let key = (self.query.clone(), entries.len());
        if let Some((k, r)) = &self.cache {
            if *k == key {
                return r.clone();
            }
        }
        let mut r = search(entries, &self.query, SHOWN);
        if self.query.trim().chars().count() >= RECORDS_FROM {
            if let Some(recs) = &self.records {
                // Records after everything else that matched.
                let room = SHOWN.saturating_sub(r.len()).max(12);
                r.extend(search(recs, &self.query, room));
            }
        }
        self.cache = Some((key, r.clone()));
        r
    }

    /// Draw the palette over the window; returns the entry picked.
    pub fn ui(&mut self, ctx: &egui::Context, entries: &[Entry], lang: &Language) -> Option<Target> {
        if !self.open {
            return None;
        }
        let ws = theme::current(ctx).ws;
        let results = self.results(entries);
        self.selected = self.selected.min(results.len().saturating_sub(1));
        let (up, down, enter, esc) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, Key::Enter), i.consume_key(egui::Modifiers::NONE, Key::Escape)));
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if down && self.selected + 1 < results.len() {
            self.selected += 1;
        }
        let mut picked = None;
        if enter {
            picked = results.get(self.selected).filter(|e| e.enabled).map(|e| e.target.clone());
        }
        let screen = ctx.content_rect();
        // The dimmed backdrop; a click on it closes the palette.
        let scrim = egui::Area::new(egui::Id::new("palette scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(8, 9, 12, if theme::current(ctx).dark() { 158 } else { 90 }));
            resp.clicked()
        });
        let width = 580.0_f32.min(screen.width() - 32.0);
        let pos = egui::pos2(screen.center().x - width / 2.0, screen.top() + 80.0);
        let mut moved = up || down;
        egui::Area::new(egui::Id::new("command palette")).order(egui::Order::Tooltip).fixed_pos(pos).show(ctx, |ui| {
            egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.control)).corner_radius(CornerRadius::same(8)).shadow(ui.visuals().window_shadow).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                // Search field.
                egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(42.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(icons::icon(icons::MAGNIFYING_GLASS, 15.0, ws.muted));
                        let field = egui::TextEdit::singleline(&mut self.query).frame(false).font(FontId::proportional(14.0)).hint_text(lang.tr("Search or run a command")).desired_width(width - 90.0).id(egui::Id::new("palette query"));
                        let r = ui.add(field);
                        if self.focus {
                            r.request_focus();
                            self.focus = false;
                        }
                        if r.changed() {
                            self.selected = 0;
                            moved = true;
                        }
                        widgets::kbd(ui, "Esc");
                    });
                });
                let rule = |ui: &mut egui::Ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::ZERO, ws.divider);
                };
                rule(ui);
                ui.add_space(4.0);
                if results.is_empty() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(lang.tr("No matches")).color(ws.muted));
                    });
                    ui.add_space(10.0);
                }
                egui::ScrollArea::vertical().max_height(ROW * 8.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, e) in results.iter().enumerate() {
                        let resp = row(ui, e, i == self.selected, width);
                        if i == self.selected && moved {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() && e.enabled {
                            picked = Some(e.target.clone());
                        }
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                            self.selected = i;
                        }
                    }
                });
                ui.add_space(4.0);
                // Preview of the selected entry.
                if let Some(e) = results.get(self.selected) {
                    rule(ui);
                    egui::Frame::new().fill(ws.chrome).inner_margin(egui::Margin::symmetric(12, 9)).show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(widgets::overline(&lang.tr("Preview"), &ws));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(widgets::mono(e.title.clone(), 12.5, ws.text));
                            ui.label(icons::icon(icons::ARROW_RIGHT, 12.0, ws.muted));
                            ui.label(widgets::mono(preview(e, lang), 12.5, if e.enabled { ws.accent } else { ws.muted }));
                        });
                        if !e.detail.is_empty() {
                            ui.label(RichText::new(&e.detail).size(11.0).color(ws.muted));
                        }
                    });
                }
                rule(ui);
                egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(30.0);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let note = |ui: &mut egui::Ui, t: String| ui.label(RichText::new(t).size(11.0).color(ws.muted));
                        widgets::kbd(ui, "↑");
                        widgets::kbd(ui, "↓");
                        note(ui, lang.tr("select"));
                        ui.add_space(8.0);
                        widgets::kbd(ui, "Enter");
                        note(ui, lang.tr("run"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            note(ui, lang.tr("Commands, items, rules and sections"));
                        });
                    });
                });
            });
        });
        if esc || scrim.inner || picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// What picking the entry does, for the preview line.
fn preview(e: &Entry, lang: &Language) -> String {
    if !e.enabled {
        return lang.tr("Not available now");
    }
    match &e.target {
        Target::Command(_) => lang.tr("Run"),
        Target::Section(_) => lang.tr("Go to"),
        Target::Item { .. } => lang.tr("Show in the inspector"),
        Target::Document(_) => lang.tr("Switch to"),
        Target::Record { .. } => lang.tr("Master Index"),
    }
}

/// One result row: icon, title over detail, hint; the selected one is
/// highlighted with a bar on the left.
fn row(ui: &mut egui::Ui, e: &Entry, selected: bool, width: f32) -> egui::Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, CornerRadius::ZERO, ws.selection);
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 6.0), egui::pos2(rect.left() + 2.0, rect.bottom() - 6.0)), CornerRadius::ZERO, ws.primary);
        }
        let ink = if e.enabled { ws.text } else { ws.muted };
        let icon_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.center().y - 8.0), egui::Vec2::splat(16.0));
        icons::paint(painter, icon_rect, e.icon, 16.0, if selected { ws.accent } else { ws.muted });
        let left = icon_rect.right() + 10.0;
        let hint = painter.layout_no_wrap(e.hint.clone(), FontId::monospace(12.0), Color32::PLACEHOLDER);
        let right = rect.right() - 12.0 - hint.size().x - if hint.size().x > 0.0 { 10.0 } else { 0.0 };
        let clip = egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(right, rect.bottom()));
        let title = painter.layout_no_wrap(e.title.clone(), FontId::proportional(13.0), ink);
        let detail = painter.layout_no_wrap(e.detail.clone(), FontId::proportional(11.5), ws.muted);
        let top = rect.center().y - (title.size().y + detail.size().y) / 2.0;
        painter.with_clip_rect(clip).galley(egui::pos2(left, top), title.clone(), ink);
        painter.with_clip_rect(clip).galley(egui::pos2(left, top + title.size().y), detail, ws.muted);
        painter.galley(egui::pos2(rect.right() - 12.0 - hint.size().x, rect.center().y - hint.size().y / 2.0), hint, if selected { ws.accent } else { ws.muted });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, title: &str) -> Entry {
        Entry { kind, icon: "", title: title.into(), detail: String::new(), hint: String::new(), keywords: String::new(), enabled: true, target: Target::Command(Cmd::Save) }
    }

    #[test]
    fn rank_prefers_prefixes_and_word_starts() {
        assert_eq!(rank("Save", ""), Some(0));
        assert!(rank("Save As…", "sa").unwrap() < rank("Dice: Edge saves", "sa").unwrap(), "a prefix beats a later word");
        assert!(rank("Raise Pistols", "pis").unwrap() < rank("Kapistols", "pis").unwrap(), "a word start beats the middle of a word");
        assert_eq!(rank("Ares Predator V", "pred ares"), rank("Ares Predator V", "pred ares"));
        assert!(rank("Ares Predator V", "pred ares").is_some(), "words in any order");
        assert_eq!(rank("Ares Predator V", "pred colt"), None, "every word must match");
        assert!(rank("save", "SAVE").is_some(), "case does not matter");
    }

    #[test]
    fn search_orders_results() {
        let mut off = entry(Kind::Command, "Save");
        off.enabled = false;
        let entries = vec![
            entry(Kind::Record, "Skillwires"),
            entry(Kind::Navigate, "Skills"),
            entry(Kind::Command, "Save As…"),
            off,
            Entry { keywords: "abilities".into(), ..entry(Kind::Navigate, "Martial Arts") },
        ];
        let titles = |v: Vec<Entry>| v.into_iter().map(|e| e.title).collect::<Vec<_>>();
        // Same rank: the section before the record; shorter first.
        assert_eq!(titles(search(&entries, "skill", 10)), vec!["Skills", "Skillwires"]);
        // Disabled entries last; keywords find entries after title hits.
        assert_eq!(titles(search(&entries, "sa", 10)), vec!["Save As…", "Save"]);
        assert_eq!(titles(search(&entries, "abil", 10)), vec!["Martial Arts"]);
        assert_eq!(search(&entries, "", 2).len(), 2, "limit");
        assert_eq!(titles(search(&entries, "", 10)), vec!["Save As…", "Save", "Skills", "Martial Arts", "Skillwires"], "kinds in order, entries as given");
    }
}
