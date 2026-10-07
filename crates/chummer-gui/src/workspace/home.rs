//! Home in the Workspace layout: what to continue (the recent
//! characters as cards), every character of the roster folders with a
//! filter, the rulesets, the tools, and on the right the online
//! campaigns (sync state, characters, Sync now, Leave), the open
//! campaign, a field for an invite link and the campaigns' recent
//! activity. Classic keeps its Character Roster tab (`App::welcome`).
//!
//! Everything runs through what Classic uses: `App::open`,
//! `App::open_player`, the wizards, the settings windows and the Join
//! Campaign window (`online::Online::join`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use chummer_core::lang::Language;
use chummer_core::roster::{self, Entry};
use chummer_sync::SyncMode;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke};

use super::palette::Cmd;
use super::widgets::{self, Look};
use super::{icons, DocKey, Section};
use crate::theme;
use crate::App;

/// Continue cards shown.
const CONTINUE: usize = 4;
/// Feed lines shown under Recent activity.
const ACTIVITY: usize = 8;
const CAMPAIGNS_WIDTH: f32 = 340.0;

/// Home's state between frames.
#[derive(Default)]
pub struct HomeState {
    /// The recent files the summaries are for.
    recent_for: Vec<PathBuf>,
    recent: Vec<Entry>,
    /// Modification times of the recent and roster files.
    modified: HashMap<PathBuf, Option<SystemTime>>,
    /// Roster entries the times were read for.
    roster_for: usize,
    filter: String,
    /// 0 all, 1 creation, 2 career.
    status: usize,
    /// The invite link typed in the Join card.
    link: String,
    /// Leave was clicked once for this campaign key.
    leave: Option<String>,
}

/// "3 min ago", "yesterday", "12 days ago" or the date, for `then`
/// seen at `now`.
fn ago(lang: &Language, then: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => lang.tr("just now"),
        60..3600 => lang.tr_fmt("{0} min ago", &[&(secs / 60)]),
        3600..86_400 => lang.tr_fmt("{0} hours ago", &[&(secs / 3600)]),
        86_400..172_800 => lang.tr("yesterday"),
        172_800..2_592_000 => lang.tr_fmt("{0} days ago", &[&(secs / 86_400)]),
        _ => {
            let unix = then.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            chummer_core::chargen::iso_from_unix(unix).get(..10).unwrap_or("").to_owned()
        }
    }
}

fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.chars().count() < 2 {
        out = name.chars().filter(|c| c.is_alphanumeric()).take(2).collect();
    }
    out.to_uppercase()
}

/// "Elf · Street samurai".
fn about(e: &Entry) -> String {
    let concept = e.concept.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    [e.metatype.as_str(), concept].into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" · ")
}

/// "Career · 14 karma" or "Creation · Priority".
fn mode(lang: &Language, e: &Entry) -> String {
    if e.career {
        format!("{} · {}", lang.tr("Career"), lang.tr_fmt("{0} karma", &[&e.karma]))
    } else if e.build_method.is_empty() {
        lang.tr("Creation")
    } else {
        format!("{} · {}", lang.tr("Creation"), e.build_method)
    }
}

/// A section heading on Home: title and a muted note.
fn heading(ui: &mut egui::Ui, title: &str, note: &str) {
    let ws = theme::ws(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(widgets::title(title, &ws));
        if !note.is_empty() {
            ui.label(RichText::new(note).size(11.5).color(ws.muted));
        }
    });
}

/// A campaign card's frame.
fn card(ws: &theme::WsPalette) -> egui::Frame {
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(7)).inner_margin(Margin::same(10))
}

/// What Home asks the app for this frame.
enum Do {
    Open(PathBuf),
    Player(usize, chummer_sync::CharacterId),
    Run(Cmd),
    Section(Section),
    Campaign,
    AddFolder,
    Refresh,
    RemoveFolder(usize),
    Join(String),
    Leave(usize),
    Sources,
    Initiative,
}

impl App {
    /// Refresh the cached file summaries when the lists changed.
    fn ws_home_cache(&mut self) {
        let h = &mut self.ws.home_page;
        if h.recent_for != self.recent {
            h.recent_for = self.recent.clone();
            // Campaign files are in the list too; Continue shows characters.
            let campaign = |p: &PathBuf| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(chummer_core::campaign::EXTENSION));
            h.recent = self.recent.iter().filter(|p| p.exists() && !campaign(p)).take(CONTINUE).map(|p| roster::summarize(p)).collect();
            for p in &self.recent {
                h.modified.insert(p.clone(), std::fs::metadata(p).and_then(|m| m.modified()).ok());
            }
        }
        if h.roster_for != self.roster.len() || self.roster.iter().any(|e| !h.modified.contains_key(&e.path)) {
            h.roster_for = self.roster.len();
            for e in &self.roster {
                h.modified.insert(e.path.clone(), std::fs::metadata(&e.path).and_then(|m| m.modified()).ok());
            }
        }
    }

    /// The Home page (Workspace): the library in the middle, the
    /// campaigns on the right.
    pub(super) fn ws_home_page(&mut self, ctx: &egui::Context) {
        self.ws_home_cache();
        let ws = theme::current(ctx).ws;
        let mut todo: Vec<Do> = Vec::new();
        egui::SidePanel::right("ws_home_campaigns").exact_width(CAMPAIGNS_WIDTH).resizable(false).frame(egui::Frame::new().fill(ws.chrome).inner_margin(Margin::same(12))).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("ws_home_campaigns").auto_shrink(false).show(ui, |ui| self.ws_home_campaigns(ui, &mut todo));
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 8 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("ws_home").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                self.ws_home_library(ui, &mut todo);
            });
        });
        for d in todo {
            match d {
                Do::Open(p) => self.open(&p),
                Do::Player(c, id) => self.open_player(c, id),
                Do::Run(c) => self.ws_run(ctx, c),
                Do::Section(s) => self.ws_go(DocKey::Home, s),
                Do::Campaign => self.home = Some(crate::Home::Campaign),
                Do::AddFolder => {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.roster_folders.push(d);
                        self.roster = roster::scan(&self.roster_folders);
                    }
                }
                Do::Refresh => self.roster = roster::scan(&self.roster_folders),
                Do::RemoveFolder(i) => {
                    if i < self.roster_folders.len() {
                        self.roster_folders.remove(i);
                        self.roster = roster::scan(&self.roster_folders);
                    }
                }
                Do::Join(link) => self.online.join = Some((link, self.online.display_name())),
                Do::Leave(i) => self.online.leave(i),
                Do::Sources => self.show_sources = true,
                Do::Initiative => self.show_initiative = true,
            }
        }
    }

    /// The middle: welcome, Continue, All characters, Rulesets, Tools.
    fn ws_home_library(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        let ws = theme::ws(ui);
        let lang = &self.lang;
        let now = SystemTime::now();
        // Welcome and the main actions.
        ui.horizontal(|ui| {
            let name = self.online.settings.name.trim().to_owned();
            let hello = if name.is_empty() { lang.tr("Welcome back") } else { lang.tr_fmt("Welcome back, {0}", &[&name]) };
            ui.label(RichText::new(hello).font(widgets::bold(18.0)).color(ws.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if widgets::button(ui, Some(icons::FOLDER_OPEN), &lang.tr("Open"), Look::Secondary, 28.0).on_hover_text("Ctrl+O").clicked() {
                    todo.push(Do::Run(Cmd::Open));
                }
                if widgets::button(ui, Some(icons::PAW_PRINT), &lang.tr("New critter or NPC"), Look::Secondary, 28.0).clicked() {
                    todo.push(Do::Run(Cmd::NewCritter));
                }
                if widgets::button(ui, Some(icons::USER_PLUS), &lang.tr("New character"), Look::Primary, 28.0).on_hover_text("Ctrl+N").clicked() {
                    todo.push(Do::Run(Cmd::NewCharacter));
                }
            });
        });
        ui.add_space(4.0);

        // Continue.
        let h = &self.ws.home_page;
        heading(ui, &lang.tr("Continue"), &lang.tr("most recent first"));
        if h.recent.is_empty() {
            widgets::card_frame(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(lang.tr("No recent files. Create a character, or open one (.chum5 files can be dropped onto this window).")).size(12.0).color(ws.muted));
            });
        } else {
            let n = h.recent.len().max(2) as f32;
            let w = ((ui.available_width() - (n - 1.0) * 8.0) / n).floor() - 1.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for e in &h.recent {
                    let open = self.views.iter().any(|v| v.path().as_deref() == Some(e.path.as_path()));
                    let (r, _) = widgets::click_card(ui, ("continue", &e.path), w, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 9.0;
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::hover());
                            ui.painter().rect_filled(rect, CornerRadius::same(6), ws.selection);
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials(&e.display_name()), widgets::bold(11.5), ws.accent);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(egui::Label::new(RichText::new(e.display_name()).font(widgets::bold(13.5)).color(ws.text)).truncate());
                                ui.add(egui::Label::new(RichText::new(about(e)).size(11.5).color(ws.muted)).truncate());
                            });
                        });
                        match &e.error {
                            Some(err) => {
                                ui.add(egui::Label::new(RichText::new(err).size(12.0).color(ws.error)).truncate());
                            }
                            None => {
                                ui.add(egui::Label::new(RichText::new(mode(lang, e)).size(12.0).color(ws.text)).truncate());
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            let (glyph, text, color) = if open { (icons::CIRCLE, lang.tr("Open now"), ws.accent) } else { (icons::FILE, lang.tr("Local file"), ws.muted) };
                            ui.label(icons::icon(glyph, 11.0, color));
                            ui.label(RichText::new(text).size(11.5).color(color));
                            if let Some(Some(t)) = h.modified.get(&e.path) {
                                ui.label(RichText::new(format!("· {}", ago(lang, *t, now))).size(11.5).color(ws.muted));
                            }
                        });
                    });
                    if r.on_hover_text(e.path.display().to_string()).clicked() {
                        todo.push(Do::Open(e.path.clone()));
                    }
                }
            });
        }
        ui.add_space(6.0);

        // All characters.
        let note = if self.roster_folders.is_empty() { lang.tr("no roster folder yet") } else { lang.tr_fmt("{0} files", &[&self.roster.len()]) };
        let h = &mut self.ws.home_page;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(widgets::title(&lang.tr("All characters"), &ws));
            ui.add(egui::Label::new(RichText::new(note).size(11.5).color(ws.muted)).truncate());
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(egui::TextEdit::singleline(&mut h.filter).hint_text(lang.tr("Filter by name, metatype, concept")).desired_width(240.0));
            let labels = [lang.tr("All statuses"), lang.tr("Creation"), lang.tr("Career")];
            crate::combo::Combo::from_id_salt("ws_home_status").selected_text(labels[h.status].clone()).width(130.0).show_ui(ui, |ui| {
                for (i, l) in labels.iter().enumerate() {
                    crate::combo::selectable_value(ui, &mut h.status, i, l);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if !self.roster_folders.is_empty() && widgets::button(ui, Some(icons::ARROWS_CLOCKWISE), &lang.tr("Refresh"), Look::Ghost, 26.0).clicked() {
                    todo.push(Do::Refresh);
                }
                if widgets::button(ui, Some(icons::FOLDER_PLUS), &lang.tr("Add folder…"), Look::Secondary, 26.0).clicked() {
                    todo.push(Do::AddFolder);
                }
            });
        });
        if !self.roster_folders.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for (i, f) in self.roster_folders.iter().enumerate() {
                    egui::Frame::new().stroke(Stroke::new(1.0_f32, ws.divider)).corner_radius(CornerRadius::same(9)).inner_margin(Margin { left: 8, right: 2, top: 0, bottom: 0 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(icons::icon(icons::FOLDER, 11.0, ws.muted));
                        ui.label(RichText::new(f.display().to_string()).size(11.5).color(ws.muted));
                        if widgets::icon_button(ui, icons::X, 18.0).on_hover_text(lang.tr("Remove folder")).clicked() {
                            todo.push(Do::RemoveFolder(i));
                        }
                    });
                }
            });
        }
        let needle = h.filter.trim().to_lowercase();
        let rows: Vec<&Entry> = self
            .roster
            .iter()
            .filter(|e| match h.status {
                1 => !e.career,
                2 => e.career,
                _ => true,
            })
            .filter(|e| needle.is_empty() || crate::combo::matches(&format!("{} {} {} {}", e.display_name(), e.metatype, e.concept, e.player), &needle))
            .collect();
        widgets::card_frame(&ws).inner_margin(Margin::same(0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            let cols: [(String, f32); 6] = [
                (lang.tr("Name"), 0.22),
                (lang.tr("Metatype · concept"), 0.32),
                (lang.tr("Status"), 0.14),
                (lang.tr("Karma"), 0.08),
                (lang.tr("Essence"), 0.1),
                (lang.tr("Modified"), 0.14),
            ];
            let xs: Vec<f32> = cols.iter().scan(10.0, |x, (_, f)| {
                let at = *x;
                *x += (width - 20.0) * f;
                Some(at)
            }).collect();
            let (head, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), Sense::hover());
            for ((t, _), x) in cols.iter().zip(&xs) {
                ui.painter().text(egui::pos2(head.left() + x, head.center().y), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(10.5), ws.muted);
            }
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0_f32, ws.divider));
            if rows.is_empty() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let text = if self.roster.is_empty() { lang.tr("Add a folder with .chum5 files to list its characters here.") } else { lang.tr("No character matches the filter.") };
                    ui.label(RichText::new(text).size(12.0).color(ws.muted));
                });
                ui.add_space(6.0);
                return;
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            for e in rows {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 30.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, 0.0, ws.hover);
                }
                let cell = |i: usize, t: String, color: Color32, mono: bool| {
                    let x = rect.left() + xs[i];
                    let end = xs.get(i + 1).map_or(rect.right() - 10.0, |n| rect.left() + n - 8.0);
                    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(end, rect.bottom()));
                    let font = if mono { egui::FontId::monospace(12.0) } else { egui::FontId::proportional(12.5) };
                    p.with_clip_rect(clip).text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, t, font, color);
                };
                cell(0, e.display_name(), ws.text, false);
                cell(1, about(e), ws.muted, false);
                match &e.error {
                    Some(err) => cell(2, err.clone(), ws.error, false),
                    None if e.career => cell(2, lang.tr("Career"), ws.text, false),
                    None => cell(2, lang.tr("Creation"), ws.warning, false),
                }
                cell(3, e.karma.clone(), ws.text, true);
                cell(4, e.essence.clone(), ws.text, true);
                let when = h.modified.get(&e.path).copied().flatten().map(|t| ago(lang, t, now)).unwrap_or_default();
                cell(5, when, ws.muted, false);
                if resp.on_hover_text(e.path.display().to_string()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    todo.push(Do::Open(e.path.clone()));
                }
            }
        });
        ui.add_space(6.0);

        // Tools.
        heading(ui, &lang.tr("Tools"), "");
        let linked = self.pdfs.linked_count();
        let tools: [(&str, String, String, Do); 4] = [
            (icons::DATABASE, lang.tr("Master Index"), lang.tr("Search every item, quality and spell"), Do::Section(Section::DataBrowser)),
            (icons::BOOK_OPEN, lang.tr("Sourcebooks"), lang.tr_fmt("{0} PDFs linked", &[&linked]), Do::Sources),
            (icons::DICE_FIVE, lang.tr("Dice Roller"), lang.tr("Quick pools, glitches and Edge"), Do::Run(Cmd::DiceRoller)),
            (icons::TIMER, lang.tr("Initiative tracker"), lang.tr("Combat turns and passes"), Do::Initiative),
        ];
        let w = ((ui.available_width() - 3.0 * 8.0) / 4.0).floor() - 1.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (glyph, title, note, action) in tools {
                let (r, _) = widgets::click_card(ui, ("tool", &title), w, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 9.0;
                        ui.label(icons::icon(glyph, 18.0, ws.accent));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(RichText::new(&title).size(12.5).color(ws.text)).truncate());
                            ui.add(egui::Label::new(RichText::new(&note).size(11.0).color(ws.muted)).truncate());
                        });
                    });
                });
                if r.clicked() {
                    todo.push(action);
                }
            }
        });
        ui.add_space(6.0);

        // Rulesets.
        ui.horizontal(|ui| {
            ui.label(widgets::title(&lang.tr("Rulesets"), &ws));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, Some(icons::SLIDERS), &lang.tr("Character settings…"), Look::Ghost, 22.0).clicked() {
                    todo.push(Do::Run(Cmd::CharacterSettings));
                }
            });
        });
        widgets::card_frame(&ws).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for s in &self.engine.settings.presets {
                ui.horizontal(|ui| {
                    ui.set_min_height(28.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(s.name()).size(12.5).color(ws.text));
                    let books = s.books();
                    let shown: Vec<&str> = books.iter().take(6).map(String::as_str).collect();
                    let more = if books.len() > shown.len() { format!(" +{}", books.len() - shown.len()) } else { String::new() };
                    ui.add(egui::Label::new(RichText::new(format!("{}{more}", shown.join(" · "))).size(11.5).color(ws.muted)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let origin = if s.file.is_some() { lang.tr("your file") } else { lang.tr("built in") };
                        ui.label(RichText::new(format!("{} · {}", s.build_method(), origin)).size(11.5).color(ws.muted));
                    });
                });
            }
        });
    }

    /// The right column: campaigns, joining one, recent activity.
    fn ws_home_campaigns(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        let ws = theme::ws(ui);
        let lang = &self.lang;
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(lang.tr("Campaigns")).font(widgets::bold(15.0)).color(ws.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::icon_button(ui, icons::FOLDER_OPEN, 24.0).on_hover_text(lang.tr("Open campaign…")).clicked() {
                    todo.push(Do::Run(Cmd::OpenCampaign));
                }
                if widgets::button(ui, Some(icons::PLUS), &lang.tr("New campaign"), Look::Secondary, 24.0).clicked() {
                    todo.push(Do::Run(Cmd::NewCampaign));
                }
            });
        });

        // The open campaign (this app is its GM).
        if let Some(gm) = &self.gm {
            card(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(gm.title(&self.views)).font(widgets::bold(13.0)).color(ws.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::tag(ui, &lang.tr("GM"), ws.muted, ws.divider);
                    });
                });
                ui.label(RichText::new(lang.tr_fmt("{0} members", &[&gm.campaign.members.len()])).size(11.5).color(ws.muted));
                for m in gm.campaign.members.iter().take(5) {
                    ui.label(RichText::new(&m.name).size(11.5).color(ws.muted));
                }
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, Some(icons::USERS_THREE), &lang.tr("Open GM screen"), Look::Secondary, 24.0).clicked() {
                            todo.push(Do::Campaign);
                        }
                    });
                });
            });
        }

        // Joined campaigns.
        let mut leave_click = None;
        for (i, c) in self.online.joined.iter().enumerate() {
            let name = c.name();
            let r = c.session.replica();
            let pending = r.outbox_len();
            let refused = r.refused().len();
            let gm = r.membership().and_then(|m| m.members.iter().find(|x| x.role == chummer_net::invite::Role::Gm).map(|x| x.name.clone()));
            let chars: Vec<(chummer_sync::CharacterId, String)> = r.characters().map(|id| (id.clone(), r.name(id).unwrap_or_default().to_owned())).collect();
            drop(r);
            let (glyph, mut state, color) = match c.session.last_mode() {
                Some(SyncMode::Online) => (icons::CLOUD_CHECK, lang.tr("Online"), ws.accent),
                Some(SyncMode::Mailbox) => (icons::ENVELOPE_SIMPLE, lang.tr("Via mailbox"), ws.warning),
                Some(SyncMode::Offline) => (icons::CLOUD_SLASH, lang.tr("Offline"), ws.muted),
                None => (icons::CLOUD_ARROW_UP, lang.tr("Connecting…"), ws.muted),
            };
            let mut color = color;
            if refused > 0 {
                state = format!("{state} · {}", lang.tr_fmt("{0} refused", &[&refused]));
                color = ws.error;
            } else if pending > 0 {
                state = format!("{state} · {}", lang.tr_fmt("{0} pending", &[&pending]));
                color = ws.warning;
            } else if c.session.last_mode() == Some(SyncMode::Online) {
                state = format!("{state} · {}", lang.tr("all synced"));
            }
            card(&ws).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&name).font(widgets::bold(13.0)).color(ws.text)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::tag(ui, &lang.tr("Player"), ws.muted, ws.divider);
                    });
                });
                if let Some(g) = gm {
                    ui.label(RichText::new(lang.tr_fmt("GM {0}", &[&g])).size(11.5).color(ws.muted));
                }
                widgets::icon_line(ui, glyph, &state, color, color);
                if chars.is_empty() {
                    ui.label(RichText::new(lang.tr("The GM has not given you a character yet.")).size(11.5).color(ws.muted));
                }
                for (id, n) in chars {
                    let r = ui.add(egui::Label::new(RichText::new(format!("{n}  ›")).size(12.0).color(ws.accent)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(lang.tr("Open"));
                    if r.clicked() {
                        todo.push(Do::Player(i, id));
                    }
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if widgets::button(ui, Some(icons::ARROWS_CLOCKWISE), &lang.tr("Sync now"), Look::Secondary, 24.0).clicked() {
                        c.session.sync_soon();
                    }
                    if widgets::icon_button(ui, icons::LINK, 24.0).on_hover_text(lang.tr("Copy invite link")).clicked() {
                        ui.ctx().copy_text(c.session.link().to_string());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let confirm = self.ws.home_page.leave.as_deref() == Some(c.key.as_str());
                        let text = if confirm { lang.tr("Click again to leave") } else { lang.tr("Leave") };
                        let r = widgets::button(ui, Some(icons::SIGN_OUT), &text, if confirm { Look::Outline } else { Look::Ghost }, 24.0);
                        if r.on_hover_text(lang.tr("Stop syncing it here; your copy file stays")).clicked() {
                            leave_click = Some((i, c.key.clone(), confirm));
                        }
                    });
                });
            });
        }
        if let Some((i, key, confirm)) = leave_click {
            if confirm {
                self.ws.home_page.leave = None;
                todo.push(Do::Leave(i));
            } else {
                self.ws.home_page.leave = Some(key);
            }
        }
        if self.online.joined.is_empty() && self.gm.is_none() {
            ui.label(RichText::new(lang.tr("No campaigns joined. Ask your GM for an invite link.")).size(12.0).color(ws.muted));
        }

        // Join.
        card(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new(lang.tr("Join a campaign")).font(widgets::bold(12.5)).color(ws.text));
            ui.label(RichText::new(lang.tr("Paste the invite link your GM sent you:")).size(11.5).color(ws.muted));
            let link = &mut self.ws.home_page.link;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(egui::TextEdit::singleline(link).hint_text("chummer-rs://join/…").desired_width(ui.available_width() - 60.0));
                let ok = link.trim().parse::<chummer_net::invite::InviteLink>().is_ok();
                let r = ui.add_enabled_ui(ok, |ui| widgets::button(ui, None, &lang.tr("Join"), Look::Primary, 26.0)).inner;
                if r.clicked() {
                    todo.push(Do::Join(std::mem::take(link)));
                }
            });
            if let Err(e) = link.trim().parse::<chummer_net::invite::InviteLink>().map(|_| ()).or_else(|e| if link.trim().is_empty() { Ok(()) } else { Err(e) }) {
                ui.label(RichText::new(e.to_string()).size(11.5).color(ws.error));
            }
        });

        // Online identity.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let on = self.online.active();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.5, if on { ws.primary } else { ws.muted });
            let text = if on { lang.tr_fmt("Online as {0}", &[&self.online.display_name()]) } else { lang.tr_fmt("Offline · {0}", &[&self.online.display_name()]) };
            ui.label(RichText::new(text).size(12.0).color(ws.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, None, &lang.tr("Online settings"), Look::Ghost, 22.0).clicked() {
                    todo.push(Do::Run(Cmd::OnlineSettings));
                }
            });
        });

        // Recent activity of the joined campaigns.
        widgets::divider(ui);
        ui.label(widgets::title(&lang.tr("Recent activity"), &ws));
        let mut feed: Vec<(i64, String, String, bool)> = Vec::new();
        for c in &self.online.joined {
            let name = c.name();
            let r = c.session.replica();
            for e in r.feed().iter().rev().take(ACTIVITY) {
                feed.push((e.at, name.clone(), chummer_sync::feed::for_owner(e), e.rejected.is_some()));
            }
        }
        feed.sort_by(|a, b| b.0.cmp(&a.0));
        if feed.is_empty() {
            ui.label(RichText::new(lang.tr("Nothing yet. Changes in your campaigns show here.")).size(11.5).color(ws.muted));
        }
        for (at, campaign, text, refused) in feed.into_iter().take(ACTIVITY) {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(widgets::mono(crate::history_ui::short_time(at), 10.5, ws.muted));
                    ui.label(RichText::new(campaign).size(11.0).color(ws.muted));
                });
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if refused { ws.error } else { ws.text })).wrap());
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ago_reads_like_a_person() {
        let lang = Language::default();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let before = |s: u64| now - Duration::from_secs(s);
        assert_eq!(ago(&lang, before(5), now), "just now");
        assert_eq!(ago(&lang, before(180), now), "3 min ago");
        assert_eq!(ago(&lang, before(7_200), now), "2 hours ago");
        assert_eq!(ago(&lang, before(90_000), now), "yesterday");
        assert_eq!(ago(&lang, before(5 * 86_400), now), "5 days ago");
        assert_eq!(ago(&lang, before(400 * 86_400), now).len(), 10, "a date");
        // A file from the future (clock skew) is "just now".
        assert_eq!(ago(&lang, now + Duration::from_secs(60), now), "just now");
    }

    #[test]
    fn initials_and_lines() {
        assert_eq!(initials("Davis Jones"), "DJ");
        let e = Entry { metatype: "Elf".into(), concept: "Street samurai".into(), career: true, karma: "14".into(), ..Default::default() };
        assert_eq!(about(&e), "Elf · Street samurai");
        assert_eq!(mode(&Language::default(), &e), "Career · 14 karma");
    }
}
