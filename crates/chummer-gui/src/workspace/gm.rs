//! The Workspace's GM screen: the roster in the sidebar (grouped by kind,
//! with Physical and Stun damage), the encounter board, the combatant's
//! card in the page, and in the inspector the players (hosting online,
//! invites, the mailbox), the activity feed, the GM award and the GM's
//! notes. The encounter, the card, the players, the award, the notes and
//! the activity feed pop out ([`Panel`], `PanelId::Activity`).
//!
//! A child module of `gm_screen` (declared there with `#[path]`) so it
//! can use the screen's state: it draws the same campaign through the
//! same steps as the Classic GM screen (`GmScreen::roll_initiative`,
//! `damage_member`, `feed_rows`, `revert`, …).

use std::sync::Arc;

use chummer_core::campaign::damage::Attack;
use chummer_core::campaign::{MemberId, MemberKind};
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::play::{matrix, vehicle};
use eframe::egui::{self, RichText};

use super::{doc_mut, doc_ref, Action, Card, GmScreen};
use crate::pdf_ui::Status;
use crate::theme;
use crate::view::CharacterView;
use crate::workspace::popout::{self, Panel as Block, PopKey, PopOuts};
use crate::workspace::widgets::{self, CmClick, Look, Track};
use crate::workspace::{icons, DocKey, PanelId};

/// A part of the GM screen that can pop out (the activity feed is
/// `PanelId::Activity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// The encounter board.
    Encounter,
    /// The combatant's card.
    Card,
    /// Online: hosting, invites, the mailbox and the players.
    Players,
    Award,
    Notes,
}

impl Panel {
    /// The title (English; goes through `lang.tr`).
    pub fn title(self) -> &'static str {
        match self {
            Panel::Encounter => "Encounter",
            Panel::Card => "Combatant",
            Panel::Players => "Players",
            Panel::Award => "GM award",
            Panel::Notes => "GM Notes",
        }
    }
}

/// What the GM screen's Workspace drawing needs from the app.
pub struct Env<'a> {
    pub engine: &'a Arc<Engine>,
    pub lang: &'a Language,
    pub views: &'a mut [CharacterView],
    pub status: &'a mut Status,
    pub net: &'a mut crate::online::Online,
    pub pops: &'a mut PopOuts,
}

fn key(p: Panel) -> PopKey {
    PopKey::new(DocKey::Campaign, PanelId::Gm(p))
}

fn activity_key() -> PopKey {
    PopKey::new(DocKey::Campaign, PanelId::Activity)
}

/// Encounter row actions.
enum RowDo {
    Select,
    Acted(bool),
    Delayed(bool),
    Seize,
    Blitz,
    Score(i32),
    Remove,
}

/// What the card asks for, done after drawing.
enum CardDo {
    Physical(i32),
    Stun(i32),
    EdgeUsed(i32),
    Damage(Attack),
    Matrix(String, i32),
    Vehicle(String, i32),
    Roll(String, i32),
    Open,
    Improve,
}

impl GmScreen {
    // ----- sidebar -----

    /// The sidebar below its header line: who is hosting, Add and Invite,
    /// the filter and the roster grouped by kind. `height` is what the
    /// roster may use.
    pub fn ws_sidebar(&mut self, ui: &mut egui::Ui, env: &mut Env, height: f32) -> Option<Action> {
        let ws = theme::ws(ui);
        let lang = env.lang;
        let mut action = None;
        let view = self.online_view(env.net);
        let start = ui.cursor().top();
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 10, bottom: 8 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let name = self.title(env.views);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let out = env.pops.is_out(activity_key());
                    let (glyph, tip) = if out { (icons::ARROW_SQUARE_IN, lang.tr("Dock back")) } else { (icons::ARROW_SQUARE_OUT, lang.tr("Pop out the activity feed")) };
                    if widgets::icon_button(ui, glyph, 22.0).on_hover_text(tip).clicked() {
                        env.pops.toggle(activity_key(), ui.ctx());
                    }
                    let r = widgets::icon_button(ui, icons::DOTS_THREE, 22.0).on_hover_text(lang.tr("Campaign"));
                    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        ui.label(lang.tr("Campaign"));
                        if ui.add(egui::TextEdit::singleline(&mut self.campaign.name).desired_width(200.0)).changed() {
                            self.dirty = true;
                        }
                        if let Some(p) = &self.path {
                            ui.label(RichText::new(p.display().to_string()).size(11.0).color(ws.muted));
                        }
                    });
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(name).font(widgets::bold(13.5)).color(ws.text)).truncate());
                    });
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let connected = view.players.iter().filter(|p| p.connected).count();
                let (color, text) = if view.serving {
                    (ws.primary, lang.tr_fmt("Hosting · {0} of {1} players online", &[&connected, &view.players.len()]))
                } else if view.online {
                    (ws.muted, lang.tr("Offline: changes for players wait in the mailbox"))
                } else {
                    (ws.muted, lang.tr_fmt("{0} members", &[&self.campaign.members.len()]))
                };
                widgets::dot(ui, color, 7.0);
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(ws.muted)).truncate());
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let r = widgets::button(ui, Some(icons::PLUS), &lang.tr("Add"), Look::Secondary, 24.0);
                egui::Popup::menu(&r).show(|ui| self.add_menu(ui, env.engine, lang, env.views, env.status));
                let invite = ui.add_enabled_ui(view.online, |ui| widgets::button(ui, Some(icons::LINK), &lang.tr("Invite"), Look::Secondary, 24.0)).inner;
                let invite = if view.online { invite.on_hover_text(lang.tr("A link for players; it stays valid for the whole group")) } else { invite.on_disabled_hover_text(lang.tr("Host the campaign to invite players. Their characters then sync with yours; every change is logged here.")) };
                if invite.clicked() {
                    self.new_invite(env.net);
                }
                if widgets::icon_button(ui, icons::PAW_PRINT, 24.0).on_hover_text(lang.tr("New Critter…")).clicked() {
                    self.critter = Some(crate::gm_ui::CritterWizard::new());
                }
            });
        });
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 2 }).show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(format!("{}  {}", icons::MAGNIFYING_GLASS, lang.tr("Filter roster"))).desired_width(f32::INFINITY));
        });
        let used = ui.cursor().top() - start;
        let rows = self.roster_rows(env.views);
        let filter = self.filter.trim().to_lowercase();
        let mut pick = None;
        egui::ScrollArea::vertical().id_salt("ws_gm_roster").auto_shrink(false).max_height((height - used).max(60.0)).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for (kind, members) in &rows {
                    let shown: Vec<_> = members.iter().filter(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter) || r.who.to_lowercase().contains(&filter)).collect();
                    if shown.is_empty() {
                        continue;
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(icons::icon(kind_icon(kind), 12.0, ws.muted));
                        ui.label(widgets::overline(&lang.tr(kind.plural()), &ws));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(4.0);
                            ui.label(widgets::mono(shown.len().to_string(), 11.0, ws.muted));
                        });
                    });
                    for r in shown {
                        let selected = self.selected == Some(r.id);
                        let resp = roster_row(ui, r, selected);
                        let resp = if r.notes.is_empty() { resp } else { resp.on_hover_text(&r.notes) };
                        if resp.clicked() {
                            pick = Some(r.id);
                        }
                        if resp.double_clicked() && self.live.contains_key(&r.id) {
                            action = Some(Action::Open(r.id));
                        }
                        resp.context_menu(|ui| {
                            if ui.add_enabled(self.live.contains_key(&r.id), egui::Button::new(lang.tr("Open"))).clicked() {
                                action = Some(Action::Open(r.id));
                            }
                        });
                    }
                }
                if self.campaign.members.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(lang.tr("No characters yet: use Add to bring in player characters, critters and NPCs.")).size(11.5).color(ws.muted));
                }
            });
        });
        if let Some(id) = pick {
            self.select_member(id);
        }
        action
    }

    /// The Add menu (both layouts): character files, a critter, PACKS
    /// NPCs, open characters.
    pub(super) fn add_menu(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status) {
        if ui.button(lang.tr("Character file (copy into the campaign)…")).clicked() {
            ui.close();
            for p in crate::campaign_ui::pick_characters() {
                self.add_file(&p, false, engine, status);
            }
        }
        if ui.button(lang.tr("Character file (link to the file)…")).clicked() {
            ui.close();
            for p in crate::campaign_ui::pick_characters() {
                self.add_file(&p, true, engine, status);
            }
        }
        if ui.button(lang.tr("New Critter…")).clicked() {
            ui.close();
            self.critter = Some(crate::gm_ui::CritterWizard::new());
        }
        if ui.button(lang.tr("NPC from PACKS Kit…")).clicked() {
            ui.close();
            self.kit = Some(crate::campaign_ui::KitForm::new(engine));
        }
        let open: Vec<usize> = (0..views.len()).filter(|&i| views[i].campaign_member.is_none()).collect();
        if !open.is_empty() {
            ui.separator();
            ui.weak(lang.tr("Open characters:"));
            for i in open {
                if ui.button(views[i].ch().display_name()).clicked() {
                    ui.close();
                    self.adopt(&mut views[i]);
                }
            }
        }
    }

    // ----- the page -----

    /// The GM screen's panels (the sidebar is drawn by the shell with
    /// [`GmScreen::ws_sidebar`]), and its dialogs.
    pub fn ws_ui(&mut self, ctx: &egui::Context, env: &mut Env) -> Option<Action> {
        let ws = theme::current(ctx).ws;
        let mut action = None;
        if self.campaign.encounters.is_empty() {
            self.campaign.encounters.push(chummer_core::campaign::Encounter::new("Encounter 1"));
        }
        self.encounter = self.encounter.min(self.campaign.encounters.len() - 1);
        egui::SidePanel::left("ws_gm_encounter").exact_width(310.0).resizable(false).frame(egui::Frame::new().fill(ws.ground).inner_margin(egui::Margin::same(12))).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("ws_gm_encounter_scroll").auto_shrink(false).show(ui, |ui| {
                self.block(ui, Panel::Encounter, env, &mut action);
            });
        });
        egui::SidePanel::right("ws_gm_inspector").default_width(320.0).min_width(260.0).max_width(560.0).resizable(true).frame(egui::Frame::new().fill(ws.chrome)).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("ws_gm_inspector_scroll").auto_shrink(false).show(ui, |ui| {
                self.block(ui, Panel::Players, env, &mut action);
                self.ws_activity_block(ui, env);
                self.block(ui, Panel::Award, env, &mut action);
                self.block(ui, Panel::Notes, env, &mut action);
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(ws.ground).inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 8 })).show(ctx, |ui| {
            if env.pops.is_out(key(Panel::Card)) {
                widgets::card_frame(&ws).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if popout::placeholder(ui, env.lang) {
                        env.pops.dock(key(Panel::Card));
                    }
                });
            } else {
                egui::ScrollArea::vertical().id_salt("ws_gm_card_scroll").auto_shrink(false).show(ui, |ui| {
                    if let Some(a) = self.ws_card(ui, env, true) {
                        action = Some(a);
                    }
                });
            }
        });
        self.windows(ctx, env.engine, env.lang, env.views, env.status);
        action
    }

    /// A popped-out part's contents (`None`: the activity feed).
    pub fn ws_panel(&mut self, ui: &mut egui::Ui, panel: Option<Panel>, env: &mut Env) -> Option<Action> {
        let mut action = None;
        match panel {
            None => self.ws_activity(ui, env),
            Some(Panel::Encounter) => {
                self.ws_encounter_select(ui, env.lang);
                ui.add_space(8.0);
                self.ws_encounter(ui, env);
            }
            Some(p) => self.body(ui, p, env, &mut action),
        }
        action
    }

    /// An inspector or encounter block with its pop-out button.
    fn block(&mut self, ui: &mut egui::Ui, p: Panel, env: &mut Env, action: &mut Option<Action>) {
        let enc = &self.campaign.encounters[self.encounter];
        let title = if p == Panel::Encounter && enc.round > 0 { env.lang.tr_fmt("Round {0}", &[&enc.round]) } else { env.lang.tr(p.title()) };
        if p == Panel::Encounter {
            self.ws_encounter_select(ui, env.lang);
            ui.add_space(8.0);
        }
        let b = match p {
            Panel::Encounter => Block::bare(key(p), &title),
            _ => Block::inspector(key(p), &title),
        };
        let ws = theme::ws(ui);
        let lang = env.lang;
        let mut roll = false;
        let mut invite = false;
        let online = self.is_online();
        let enc = &self.campaign.encounters[self.encounter];
        let (round, pass) = (enc.round, enc.pass);
        // The block borrows the pop-outs; the body gets the rest.
        let mut none = PopOuts::default();
        let mut inner = Env { engine: env.engine, lang: env.lang, views: &mut *env.views, status: &mut *env.status, net: &mut *env.net, pops: &mut none };
        b.show(
            ui,
            env.pops,
            lang,
            |ui| match p {
                Panel::Encounter => {
                    roll = widgets::button(ui, Some(icons::DICE_FIVE), &lang.tr("Roll initiative"), Look::Secondary, 24.0).on_hover_text(lang.tr("Start the next combat round: everyone rolls")).clicked();
                    if round > 0 {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("· {}", lang.tr_fmt("Pass {0}", &[&pass]))).size(12.0).color(ws.muted));
                        });
                    }
                }
                Panel::Players if online => {
                    invite = widgets::button(ui, Some(icons::LINK), &lang.tr("Invite"), Look::Ghost, 22.0).on_hover_text(lang.tr("A link for players; it stays valid for the whole group")).clicked();
                }
                _ => {}
            },
            |ui| self.body(ui, p, &mut inner, action),
        );
        if roll {
            self.roll_initiative(env.views);
        }
        if invite {
            self.new_invite(env.net);
        }
    }

    /// A part's contents.
    fn body(&mut self, ui: &mut egui::Ui, p: Panel, env: &mut Env, action: &mut Option<Action>) {
        match p {
            Panel::Encounter => self.ws_encounter(ui, env),
            Panel::Card => {
                if let Some(a) = self.ws_card(ui, env, false) {
                    *action = Some(a);
                }
            }
            Panel::Players => self.ws_players(ui, env),
            Panel::Award => self.ws_award(ui, env),
            Panel::Notes => {
                let r = ui.add(egui::TextEdit::multiline(&mut self.campaign.gm_notes).id_salt("ws_gm_notes").desired_width(f32::INFINITY).desired_rows(10));
                self.dirty |= r.changed();
            }
        }
    }

    // ----- encounter -----

    /// The encounter: pick one; rename, new, delete and reset in a menu.
    fn ws_encounter_select(&mut self, ui: &mut egui::Ui, lang: &Language) {
        // The encounter: pick, rename, new, delete, reset.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let names: Vec<String> = self.campaign.encounters.iter().map(|e| e.name.clone()).collect();
            let mut new = false;
            let mut delete = false;
            let mut reset = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = widgets::icon_button(ui, icons::DOTS_THREE, 26.0).on_hover_text(lang.tr("Encounter"));
                egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    let e = &mut self.campaign.encounters[self.encounter];
                    self.dirty |= ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(180.0)).changed();
                    new = ui.button(lang.tr("New")).clicked();
                    delete = self.campaign.encounters.len() > 1 && ui.button(lang.tr("Delete")).clicked();
                    reset = ui.add_enabled(self.campaign.encounters[self.encounter].round > 0, egui::Button::new(lang.tr("Reset"))).clicked();
                });
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let w = ui.available_width();
                    crate::combo::Combo::from_id_salt("ws_gm_encounter").selected_text(names[self.encounter].clone()).width(w).show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            crate::combo::selectable_value(ui, &mut self.encounter, i, n);
                        }
                    });
                });
            });
            if new {
                let n = self.campaign.encounters.len() + 1;
                self.campaign.encounters.push(chummer_core::campaign::Encounter::new(format!("Encounter {n}")));
                self.encounter = n - 1;
                self.dirty = true;
            }
            if delete {
                self.campaign.encounters.remove(self.encounter);
                self.encounter = 0;
                self.dirty = true;
            }
            if reset {
                self.campaign.encounters[self.encounter].reset();
                self.dirty = true;
            }
        });
    }

    fn ws_encounter(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ws = theme::ws(ui);
        let lang = env.lang;
        ui.spacing_mut().item_spacing.y = 8.0;
        let order = self.campaign.encounters[self.encounter].order();
        let current = self.campaign.encounters[self.encounter].current();
        let mut todo: Option<(usize, RowDo)> = None;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if order.is_empty() {
                ui.label(RichText::new(lang.tr("Add combatants: characters from the roster, or quick ones by name.")).size(11.5).color(ws.muted));
            }
            for i in order {
                let e = &self.campaign.encounters[self.encounter];
                let c = &e.combatants[i];
                let kind = c.member.and_then(|m| self.campaign.member(m)).map(|m| lang.tr(m.kind.as_str())).unwrap_or_else(|| lang.tr("Other"));
                if let Some(d) = encounter_row(ui, c, kind, current == Some(i), self.combatant == Some(c.id), e.pass, lang) {
                    todo = Some((i, d));
                }
            }
        });
        if let Some((i, d)) = todo {
            let e = &mut self.campaign.encounters[self.encounter];
            match d {
                RowDo::Select => {
                    let (cid, m) = (e.combatants[i].id, e.combatants[i].member);
                    self.select_combatant(cid, m);
                }
                RowDo::Acted(on) => {
                    e.combatants[i].acted = on;
                    self.dirty = true;
                }
                RowDo::Delayed(on) => {
                    e.combatants[i].delayed = on;
                    self.dirty = true;
                }
                RowDo::Score(s) => {
                    e.combatants[i].score = s;
                    self.dirty = true;
                }
                RowDo::Seize => self.seize(i, env.views),
                RowDo::Blitz => self.blitz(i, env.views),
                RowDo::Remove => self.remove_combatant(i),
            }
        }
        // Next and next pass.
        let e = &self.campaign.encounters[self.encounter];
        let (has_current, has_pass) = (e.current().is_some(), e.round > 0 && e.has_next_pass());
        let mut next = false;
        let mut pass = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pass = ui.add_enabled_ui(has_pass, |ui| widgets::button(ui, None, &format!("{} −10", lang.tr("Next pass")), Look::Secondary, 28.0)).inner.on_hover_text(lang.tr("Everyone loses 10")).clicked();
                next = ui.add_enabled_ui(has_current, |ui| widgets::wide_button(ui, Some(icons::SKIP_FORWARD), &lang.tr("Next"), Look::Primary, 28.0)).inner.on_hover_text(lang.tr("Mark the current combatant as acted")).clicked();
            });
        });
        if next {
            self.campaign.encounters[self.encounter].advance();
            self.dirty = true;
        }
        if pass {
            self.campaign.encounters[self.encounter].next_pass();
            self.dirty = true;
        }
        // Adding combatants.
        let mut all = false;
        let mut add = None;
        let mut adhoc = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let r = widgets::button(ui, Some(icons::PLUS), &lang.tr("Add to encounter"), Look::Ghost, 24.0);
            egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                let enc = &self.campaign.encounters[self.encounter];
                for m in self.campaign.members.iter().filter(|m| self.live.contains_key(&m.id) && !enc.has_member(m.id)) {
                    if ui.button(&m.name).clicked() {
                        add = Some(m.id);
                        ui.close();
                    }
                }
                ui.separator();
                ui.label(RichText::new(lang.tr("A combatant without a character sheet")).size(11.5).color(ws.muted));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.adhoc.0).hint_text(lang.tr("Name")).desired_width(110.0));
                    ui.add(egui::DragValue::new(&mut self.adhoc.1).range(0..=40));
                    ui.label("+");
                    ui.add(egui::DragValue::new(&mut self.adhoc.2).range(1..=5).suffix("d6"));
                    if ui.add_enabled(!self.adhoc.0.trim().is_empty(), egui::Button::new(lang.tr("Add"))).clicked() {
                        adhoc = true;
                    }
                });
            });
            let players = self.campaign.members.iter().any(|m| m.kind == MemberKind::Player && self.live.contains_key(&m.id));
            all = ui.add_enabled_ui(players, |ui| widgets::button(ui, Some(icons::USERS_THREE), &lang.tr("Add all players"), Look::Ghost, 24.0)).inner.clicked();
        });
        if let Some(m) = add {
            self.add_to_encounter(m, env.views, true);
        }
        if adhoc {
            self.add_adhoc();
        }
        if all {
            self.add_all_players(env.views);
        }
    }

    // ----- the card -----

    /// The combatant's card. `header`: with its title row (docked).
    fn ws_card(&mut self, ui: &mut egui::Ui, env: &mut Env, header: bool) -> Option<Action> {
        let ws = theme::ws(ui);
        let lang = env.lang;
        match self.card() {
            Card::AdHoc(i) => {
                if header {
                    let name = self.campaign.encounters[self.encounter].combatants[i].name.clone();
                    card_title(ui, env.pops, lang, &name, &lang.tr("A combatant without a character sheet"), |_| {});
                }
                self.ws_adhoc(ui, i, lang);
                None
            }
            Card::Member(id) => self.ws_member(ui, id, env, header),
            Card::None => {
                ui.label(RichText::new(lang.tr("Select a combatant or a character to see its condition monitors and dice pools.")).size(12.0).color(ws.muted));
                None
            }
        }
    }

    fn ws_adhoc(&mut self, ui: &mut egui::Ui, i: usize, lang: &Language) {
        let ws = theme::ws(ui);
        let mut attack = None;
        let mut dirty = false;
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let c = &mut self.campaign.encounters[self.encounter].combatants[i];
            let t = c.track.get_or_insert_with(Default::default);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let physical = Track { label: lang.tr("Physical"), color: ws.physical, boxes: t.physical, filled: t.physical_filled, threshold: 3 };
                let stun = Track { label: lang.tr("Stun"), color: ws.stun, boxes: t.stun, filled: t.stun_filled, threshold: 3 };
                let top = ui.cursor().top();
                match widgets::condition_monitor(ui, "ws_gm_adhoc", &physical, &stun, 0, &lang.tr("Overflow")) {
                    Some(CmClick::Physical(n)) => {
                        t.physical_filled = n;
                        dirty = true;
                    }
                    Some(CmClick::Stun(n)) => {
                        t.stun_filled = n;
                        dirty = true;
                    }
                    None => {}
                }
                let h = (ui.min_rect().bottom() - top).max(100.0);
                widgets::divider(ui, Some(h));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let wm = chummer_core::campaign::damage::wound_modifier(t.physical_filled, t.stun_filled, t.physical, 3);
                    ui.horizontal(|ui| {
                        let (color, edge) = if wm != 0 { (ws.warning, ws.warning) } else { (ws.muted, ws.divider) };
                        widgets::tag(ui, &format!("{} {wm}", lang.tr("CM Penalty:")), color, edge);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(lang.tr("Physical")).size(11.5).color(ws.muted));
                        dirty |= ui.add(egui::DragValue::new(&mut t.physical).range(1..=30)).changed();
                        ui.label(RichText::new(lang.tr("Stun")).size(11.5).color(ws.muted));
                        dirty |= ui.add(egui::DragValue::new(&mut t.stun).range(0..=30)).changed();
                    });
                    ui.label(widgets::overline(&lang.tr("Damage"), &ws));
                    attack = damage_entry(ui, &mut self.damage, lang, &lang.tr("Apply"), false);
                });
            });
        });
        if let Some(a) = attack {
            self.damage_adhoc(i, a);
        }
        ui.add_space(8.0);
        ui.label(widgets::title(&lang.tr("Notes"), &ws));
        let c = &mut self.campaign.encounters[self.encounter].combatants[i];
        dirty |= ui.add(egui::TextEdit::multiline(&mut c.notes).desired_rows(3).desired_width(f32::INFINITY)).changed();
        self.dirty |= dirty;
    }

    fn ws_member(&mut self, ui: &mut egui::Ui, id: MemberId, env: &mut Env, header: bool) -> Option<Action> {
        let ws = theme::ws(ui);
        let lang = env.lang;
        let sheet = self.live.get(&id).map(|l| l.sheet.clone())?;
        let m = self.campaign.member(id).cloned()?;
        let doc = doc_ref(&self.live, env.views, id)?;
        let mut todo: Vec<CardDo> = Vec::new();
        // Everything the card shows, read before drawing.
        let (plabel, slabel) = crate::ai_ui::cm_labels(doc, lang);
        let physical = Track { label: plabel, color: ws.physical, boxes: sheet.physical_cm, filled: chummer_core::play::ai::physical_filled(doc), threshold: sheet.cm_threshold };
        let stun = Track { label: slabel, color: ws.stun, boxes: sheet.stun_cm, filled: chummer_core::play::ai::stun_filled(doc), threshold: if doc.is_ai() { 0 } else { sheet.cm_threshold } };
        let created = doc.created;
        let edge_total = sheet.attr("EDG").max(0);
        let edge_used = doc.doc.get_i32("edgeused").unwrap_or(0).clamp(0, edge_total);
        let device = matrix::active_commlink(doc).cloned();
        let vehicles: Vec<_> = doc.items("vehicles", "vehicle").into_iter().take(4).cloned().collect();
        let pools = crate::campaign_ui::quick_pools(doc, &sheet, lang, 5);
        let weapons: Vec<(String, chummer_core::items::weapon::WeaponStats)> = doc.items("weapons", "weapon").into_iter().take(5).map(|w| (w.get("name"), chummer_core::items::weapon::stats(doc, &sheet, w))).collect();
        let mut rated: Vec<_> = sheet.skills.iter().filter(|s| s.rating > 0 && !s.disabled).collect();
        rated.sort_by(|a, b| b.rating.cmp(&a.rating).then(a.name.cmp(&b.name)));
        let skills: Vec<(String, String)> = rated.into_iter().take(6).map(|s| (lang.data_name("skills.xml", "", &s.name), s.rating.to_string())).collect();
        let gear: Vec<(String, String)> = doc
            .items("armors", "armor")
            .into_iter()
            .filter(|a| a.get_bool("equipped").unwrap_or(false))
            .map(|a| (a.get("name"), format!("{} {}", lang.tr("Armor"), a.get("armor"))))
            .chain(doc.items("gears", "gear").into_iter().map(|g| (g.get("name"), g.get_f64("qty").filter(|q| *q > 1.0).map(|q| format!("×{}", chummer_core::improvement::fmt_num(q))).unwrap_or_default())))
            .take(6)
            .collect();
        let qualities: Vec<(String, String)> = doc.items("qualities", "quality").into_iter().take(6).map(|q| (q.get("name"), String::new())).collect();
        if header {
            let tag = [lang.tr(m.kind.as_str()), m.group.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            card_title(ui, env.pops, lang, &m.name, &tag, |ui| {
                if widgets::button(ui, Some(icons::ARROW_SQUARE_OUT), &lang.tr("Open"), Look::Ghost, 24.0).on_hover_text(lang.tr("Open the character in its own tab")).clicked() {
                    todo.push(CardDo::Open);
                }
                if widgets::button(ui, Some(icons::PLUS), &lang.tr("Add Improvement"), Look::Ghost, 24.0).on_hover_text(lang.tr("A custom improvement: the GM allows it")).clicked() {
                    todo.push(CardDo::Improve);
                }
            });
        }
        // Condition monitors, Edge and damage.
        widgets::card_frame(&ws).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let top = ui.cursor().top();
                match widgets::condition_monitor(ui, ("ws_gm_cm", id), &physical, &stun, sheet.cm_overflow, &lang.tr("Overflow")) {
                    Some(CmClick::Physical(n)) => todo.push(CardDo::Physical(n)),
                    Some(CmClick::Stun(n)) => todo.push(CardDo::Stun(n)),
                    None => {}
                }
                let h = (ui.min_rect().bottom() - top).max(100.0);
                widgets::divider(ui, Some(h));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let penalty = format!("{} {}", lang.tr("CM Penalty:"), sheet.wound_modifier);
                        if sheet.wound_modifier != 0 {
                            widgets::tag(ui, &penalty, ws.warning, ws.warning);
                        } else {
                            ui.label(RichText::new(penalty).size(11.5).color(ws.muted));
                        }
                        ui.label(RichText::new(format!("{} {}", lang.tr("Armor"), sheet.armor)).size(11.5).color(ws.muted));
                    });
                    if created {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(lang.tr("Edge")).font(widgets::bold(12.0)).color(ws.text));
                            let available = edge_total - edge_used;
                            let tip = |n: i32, on: bool| format!("{} {n} / {edge_total}: {}", lang.tr("Edge"), if on { lang.tr("available") } else { lang.tr("spent") });
                            if let Some(a) = widgets::edge_boxes(ui, ("ws_gm_edge", id), edge_total, available, tip) {
                                todo.push(CardDo::EdgeUsed(edge_total - a));
                            }
                            ui.label(widgets::mono(format!("{available}/{edge_total}"), 11.0, ws.muted));
                        });
                    }
                    ui.label(widgets::overline(&lang.tr("Damage"), &ws));
                    if let Some(a) = damage_entry(ui, &mut self.damage, lang, &lang.tr_fmt("Apply to {0}", &[&m.name]), true) {
                        todo.push(CardDo::Damage(a));
                    }
                });
            });
            // Matrix and vehicles (as on the Classic card).
            if device.is_some() || !vehicles.is_empty() {
                ui.add_space(8.0);
                widgets::divider(ui, None);
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(18.0, 6.0);
                    if let Some(dev) = &device {
                        ui.vertical(|ui| {
                            ui.label(widgets::overline(&format!("{}: {}", lang.tr("Matrix"), dev.get("name")), &ws));
                            if let Some(f) = widgets::small_track(ui, ("ws_gm_mcm", id), matrix::condition_monitor(dev), matrix::filled(dev), 12, 14.0, ws.accent, &lang.tr("Matrix Condition Monitor")) {
                                todo.push(CardDo::Matrix(dev.get("guid"), f));
                            }
                        });
                    }
                    for v in &vehicles {
                        ui.vertical(|ui| {
                            ui.label(widgets::overline(&v.get("name"), &ws));
                            let boxes = vehicle::condition_monitor(v, &Default::default());
                            if let Some(f) = widgets::small_track(ui, ("ws_gm_vcm", v.get("guid")), boxes, vehicle::filled(v), 12, 14.0, ws.physical, &lang.tr("Damage")) {
                                todo.push(CardDo::Vehicle(v.get("guid"), f));
                            }
                        });
                    }
                });
            }
        });
        ui.add_space(12.0);
        // Dice pools.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(widgets::title(&lang.tr("Dice pools"), &ws));
            if sheet.wound_modifier != 0 {
                ui.label(RichText::new(format!("{} {}", lang.tr("CM Penalty:"), sheet.wound_modifier)).size(11.5).color(ws.muted));
            }
        });
        ui.add_space(4.0);
        let gap = 4.0;
        let w = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            for row in pools.chunks(3) {
                ui.horizontal(|ui| {
                    for p in row {
                        let shown = p.spec.map_or_else(|| p.pool.to_string(), |s| format!("{} ({s})", p.pool));
                        if widgets::roll_tile(ui, &p.label, &shown, w, 26.0).on_hover_text(lang.tr("Roll")).clicked() {
                            todo.push(CardDo::Roll(p.label.clone(), p.pool));
                        }
                    }
                });
            }
        });
        // Weapons.
        if !weapons.is_empty() {
            ui.add_space(12.0);
            ui.label(widgets::title(&lang.tr("Weapons"), &ws));
            ui.add_space(2.0);
            for (k, (name, st)) in weapons.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
                ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0_f32, ws.divider));
                let mut row = ui.new_child(egui::UiBuilder::new().id_salt(("ws_gm_weapon", k)).max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
                row.spacing_mut().item_spacing.x = 8.0;
                if widgets::button(&mut row, Some(icons::DICE_FIVE), &format!("{} {}", lang.tr("Roll"), st.dice_pool), Look::Secondary, 22.0).clicked() {
                    todo.push(CardDo::Roll(name.clone(), st.dice_pool));
                }
                let ap = if st.ap.is_empty() { "-".to_owned() } else { st.ap.clone() };
                row.label(RichText::new(format!("{} · {} {ap}", st.damage, lang.tr("AP"))).size(11.5).color(ws.muted));
                row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(name).size(12.5).color(ws.text)).truncate());
                });
            }
        }
        // Skills, gear and qualities.
        ui.add_space(12.0);
        let gap = 10.0;
        let w = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (title, rows) in [(lang.tr("Skills"), &skills), (lang.tr("Gear"), &gear), (lang.tr("Qualities"), &qualities)] {
                ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    widgets::card_frame(&ws).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(w - 22.0);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(widgets::overline(&title, &ws));
                        ui.add_space(4.0);
                        if rows.is_empty() {
                            ui.label(RichText::new("—").size(12.0).color(ws.muted));
                        }
                        for (n, v) in rows {
                            widgets::list_row(ui, n, v, 21.0, false);
                        }
                    });
                });
            }
        });
        if !m.notes.trim().is_empty() {
            ui.add_space(12.0);
            ui.label(widgets::title(&lang.tr("Notes"), &ws));
            ui.label(RichText::new(&m.notes).size(12.0).color(ws.muted));
        }
        // The member's campaign entry (type, player, group, copies, …).
        ui.add_space(12.0);
        let mut action = None;
        egui::CollapsingHeader::new(RichText::new(lang.tr("Campaign")).font(widgets::bold(13.0))).id_salt("ws_gm_member_fields").show(ui, |ui| {
            self.member_fields(ui, env.engine, lang, env.views, env.status, &mut action);
        });
        // Do what was asked.
        for d in todo {
            match d {
                CardDo::Damage(a) => self.damage_member(id, a, env.views, env.status),
                CardDo::Roll(label, pool) => self.roll_pool(&m.name, &label, pool, lang),
                CardDo::Open => action = Some(Action::Open(id)),
                CardDo::Improve => self.add_improvement(id, env.engine, env.views, lang),
                CardDo::Physical(n) => self.run_for(id, Command::SetPhysicalDamage { filled: n }, env),
                CardDo::Stun(n) => self.run_for(id, Command::SetStunDamage { filled: n }, env),
                CardDo::EdgeUsed(n) => self.run_for(id, Command::SetEdgeUsed { used: n }, env),
                CardDo::Matrix(g, f) => self.run_for(id, Command::SetMatrixDamage { device: g, filled: f }, env),
                CardDo::Vehicle(g, f) => self.run_for(id, Command::SetVehicleDamage { vehicle: g, filled: f }, env),
            }
        }
        action
    }

    /// Run a command on member `id`'s character.
    fn run_for(&mut self, id: MemberId, cmd: Command, env: &mut Env) {
        if let Some(doc) = doc_mut(&mut self.live, env.views, id) {
            doc.run(cmd, env.status);
        }
    }

    // ----- inspector -----

    fn ws_players(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ws = theme::ws(ui);
        let lang = env.lang;
        let v = self.online_view(env.net);
        ui.spacing_mut().item_spacing.y = 6.0;
        let mut on = v.serving;
        let saved = self.path.is_some();
        let r = ui.add_enabled_ui(saved, |ui| widgets::check(ui, &mut on, &lang.tr("Host online"))).inner;
        let r = if saved { r.on_hover_text(lang.tr("Players connect to this app; changes sync live")) } else { r.on_disabled_hover_text(lang.tr("Save the campaign to a file first")) };
        if r.changed() {
            self.toggle_hosting(env.net, env.engine, env.views, on, env.status);
        }
        if let Some(e) = &self.online_error {
            ui.label(RichText::new(e).size(11.5).color(ws.error));
        }
        let small = |t: String, c| RichText::new(t).size(11.5).color(c);
        if !v.online {
            ui.label(small(lang.tr("Host the campaign to invite players. Their characters then sync with yours; every change is logged here."), ws.muted));
            return;
        }
        match &v.relay {
            Some(relay) => {
                ui.label(small(lang.tr("Online: players can connect"), ws.accent));
                ui.label(small(format!("{} {}", lang.tr("Relay:"), relay.clone().unwrap_or_else(|| lang.tr("connecting to the relay…"))), ws.muted));
            }
            None => {
                ui.label(small(lang.tr("Offline: changes for players wait in the mailbox"), ws.muted));
            }
        }
        if let Some(link) = &v.invite {
            ui.horizontal(|ui| {
                let mut text = link.clone();
                ui.add(egui::TextEdit::singleline(&mut text).desired_width((ui.available_width() - 70.0).max(80.0)).font(egui::TextStyle::Monospace));
                if widgets::button(ui, Some(icons::COPY), &lang.tr("Copy"), Look::Secondary, 24.0).clicked() {
                    ui.ctx().copy_text(link.clone());
                    *env.status = Some((lang.tr("Invite link copied."), false));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let check = ui.add_enabled_ui(!v.mail_busy, |ui| widgets::button(ui, Some(icons::ENVELOPE_SIMPLE), &lang.tr("Check mail"), Look::Secondary, 24.0)).inner;
            if check.on_hover_text(lang.tr("Collect changes players mailed while you were offline, and mail them yours")).clicked() {
                self.ask_mail();
            }
            if v.mail_busy {
                ui.label(small(lang.tr("Checking mail…"), ws.muted));
            }
        });
        if let (false, Some((when, r))) = (v.mail_busy, &v.mail) {
            match r {
                Ok((f, h, s)) => ui.label(small(lang.tr_fmt("Mail at {0}: {1} read, {2} applied, {3} sent", &[when, f, h, s]), ws.muted)),
                Err(e) => ui.label(small(format!("{when}: {e}"), ws.error)),
            };
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        for p in &v.players {
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                widgets::dot(ui, if p.connected { ws.primary } else { ws.control }, 6.0);
                ui.label(RichText::new(&p.name).size(12.0).color(ws.text)).on_hover_text(&p.id);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(small(lang.tr(if p.connected { "connected" } else { "not connected" }), ws.muted));
                });
            });
        }
    }

    fn ws_award(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ws = theme::ws(ui);
        let lang = env.lang;
        let target = match self.card() {
            Card::Member(m) => doc_ref(&self.live, env.views, m).map(|d| (m, d.created, d.display_name())),
            _ => None,
        };
        let career = target.as_ref().is_some_and(|t| t.1);
        let mut give = None;
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.add_enabled_ui(career, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let karma = lang.tr("Karma");
                let nuyen = lang.tr("Nuyen");
                if let Some(i) = widgets::segmented(ui, &[(karma.as_str(), karma.as_str()), (nuyen.as_str(), nuyen.as_str())], usize::from(!self.award.karma), 26.0) {
                    self.award.karma = i == 0;
                }
                let (step, max) = if self.award.karma { (1.0, 1000.0) } else { (100.0, 10_000_000.0) };
                ui.add(egui::DragValue::new(&mut self.award.amount).range(0.0..=max).speed(step));
            });
            ui.add(egui::TextEdit::singleline(&mut self.award.note).hint_text(lang.tr("Reason")).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let ok = self.award.amount > 0.0;
                let name = target.as_ref().map(|t| t.2.clone()).unwrap_or_default();
                if ui.add_enabled_ui(ok, |ui| widgets::button(ui, Some(icons::GIFT), &lang.tr_fmt("Give to {0}", &[&name]), Look::Primary, 26.0)).inner.clicked() {
                    give = Some(true);
                }
                if ui.add_enabled_ui(ok, |ui| widgets::button(ui, None, &lang.tr("Take"), Look::Secondary, 26.0)).inner.clicked() {
                    give = Some(false);
                }
            });
        });
        let note = if target.is_none() {
            lang.tr("Select a character to give it karma or nuyen.")
        } else if !career {
            lang.tr("Awards are for characters in Career Mode.")
        } else {
            lang.tr("Shown in the character's Karma & Nuyen log.")
        };
        ui.label(RichText::new(note).size(11.0).color(ws.muted));
        if let (Some(gain), Some((m, _, _))) = (give, target) {
            self.award_member(m, gain, env.views, env.status);
        }
    }

    /// The activity feed docked in the inspector, or a note that it is in
    /// its own window.
    fn ws_activity_block(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ws = theme::ws(ui);
        let lang = env.lang;
        if env.pops.is_out(activity_key()) {
            let count = self.feed_rows().len() + self.rolls.len();
            let mut dock = false;
            egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, ws.control)).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(icons::icon(icons::ARROW_SQUARE_OUT, 14.0, ws.muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            dock = widgets::button(ui, Some(icons::ARROW_SQUARE_IN), &lang.tr("Dock back"), Look::Secondary, 24.0).clicked();
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(RichText::new(lang.tr("Activity is in its own window")).size(12.5).color(ws.text));
                                ui.label(RichText::new(lang.tr_fmt("{0} entries", &[&count])).size(11.0).color(ws.muted));
                            });
                        });
                    });
                });
            });
            if dock {
                env.pops.dock(activity_key());
            }
            return;
        }
        let title = lang.tr("Activity");
        let mut none = PopOuts::default();
        let mut inner = Env { engine: env.engine, lang: env.lang, views: &mut *env.views, status: &mut *env.status, net: &mut *env.net, pops: &mut none };
        Block::inspector(activity_key(), &title).show(ui, env.pops, lang, |_| {}, |ui| self.ws_activity(ui, &mut inner));
    }

    /// The activity feed: the GM's rolls, every change with its author,
    /// Revert where the campaign is online.
    fn ws_activity(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ws = theme::ws(ui);
        let lang = env.lang;
        let mut rows = self.feed_rows();
        rows.extend(self.rolls.iter().map(|(at, l)| super::FeedRow { at: *at, who: lang.tr("Dice"), text: l.clone(), refused: false, note: true, revert: None }));
        rows.sort_by_key(|r| std::cmp::Reverse(r.at));
        let mut revert = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.label(RichText::new(lang.tr("Changes to the campaign's characters show here.")).size(11.5).color(ws.muted));
        }
        for r in rows.iter().take(200) {
            let resp = egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 5)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let t = crate::history_ui::short_time(r.at);
                    ui.label(widgets::mono(t.get(t.len().saturating_sub(5)..).unwrap_or(""), 11.0, ws.muted)).on_hover_text(&t);
                    let revert_w = if r.revert.is_some() { 56.0 } else { 0.0 };
                    let w = (ui.available_width() - revert_w).max(60.0);
                    ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                        ui.set_width(w);
                        let mut job = egui::text::LayoutJob::default();
                        let font = egui::FontId::proportional(12.0);
                        let who = if !r.who.is_empty() {
                            Some((r.who.as_str(), if r.note { ws.accent } else { ws.stun }))
                        } else if r.note {
                            Some(("GM", ws.accent))
                        } else {
                            None
                        };
                        if let Some((who, color)) = who {
                            job.append(who, 0.0, egui::TextFormat { font_id: widgets::bold(12.0), color, ..Default::default() });
                            job.append(" ", 0.0, egui::TextFormat { font_id: font.clone(), color: ws.text, ..Default::default() });
                        }
                        job.append(&r.text, 0.0, egui::TextFormat { font_id: font, color: if r.refused { ws.error } else { ws.text }, ..Default::default() });
                        ui.add(egui::Label::new(job).wrap());
                    });
                    if let Some(v) = &r.revert {
                        if widgets::button(ui, None, &lang.tr("Revert"), Look::Ghost, 20.0).on_hover_text(lang.tr("Take this change back")).clicked() {
                            revert = Some(v.clone());
                        }
                    }
                });
            });
            ui.painter().hline(resp.response.rect.x_range(), resp.response.rect.bottom(), egui::Stroke::new(1.0_f32, ws.divider));
        }
        if self.is_online() && !rows.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr("Revert takes a change back; later changes are re-applied on top.")).size(11.0).color(ws.muted));
        }
        if let Some((c, v)) = revert {
            self.revert(&c, v, env.views, env.status);
        }
    }
}

// ----- drawing helpers -----

fn kind_icon(kind: &MemberKind) -> &'static str {
    match kind {
        MemberKind::Player => icons::USER,
        MemberKind::Npc => icons::USER_CIRCLE,
        MemberKind::Critter => icons::PAW_PRINT,
        MemberKind::Enemy => icons::SKULL,
        MemberKind::Spirit => icons::FLAME,
        MemberKind::Drone => icons::ROBOT,
        MemberKind::Other(_) => icons::USER,
    }
}

/// Fraction of a track filled.
fn fraction(t: Option<(i32, i32)>) -> f32 {
    match t {
        Some((f, b)) if b > 0 => f as f32 / b as f32,
        _ => 0.0,
    }
}

/// One member of the roster: name, player in small text, damage bars;
/// struck through when the Physical track is full.
fn roster_row(ui: &mut egui::Ui, r: &super::RosterRow, selected: bool) -> egui::Response {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, egui::CornerRadius::same(5), ws.selection);
            let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 4.0), egui::pos2(rect.left() + 2.0, rect.bottom() - 4.0));
            painter.rect_filled(bar, egui::CornerRadius::ZERO, ws.primary);
        } else if resp.hovered() {
            painter.rect_filled(rect, egui::CornerRadius::same(5), ws.hover);
        }
        let down = r.physical.is_some_and(|(f, b)| b > 0 && f >= b);
        let x = rect.left() + 26.0;
        let bars_x = rect.right() - 8.0 - 40.0;
        let name = painter.layout_no_wrap(r.name.clone(), egui::FontId::proportional(12.5), if down { ws.muted } else { ws.text });
        let y = rect.center().y - name.size().y / 2.0;
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(bars_x - 6.0, rect.bottom()));
        let name_w = name.size().x;
        painter.with_clip_rect(clip).galley(egui::pos2(x, y), name, ws.text);
        if down {
            painter.with_clip_rect(clip).hline(x..=x + name_w, rect.center().y, egui::Stroke::new(1.0_f32, ws.muted));
        }
        if !r.who.is_empty() {
            let who = painter.layout_no_wrap(r.who.clone(), egui::FontId::proportional(11.0), ws.muted);
            painter.with_clip_rect(clip).galley(egui::pos2(x + name_w + 6.0, rect.center().y - who.size().y / 2.0 + 1.0), who, ws.muted);
        }
        if r.error.is_some() {
            icons::paint(painter, egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), egui::vec2(12.0, 12.0)), icons::WARNING, 12.0, ws.warning);
        }
        if r.physical.is_some() {
            for (k, (f, c)) in [(fraction(r.physical), ws.physical), (fraction(r.stun), ws.stun)].into_iter().enumerate() {
                let bar = egui::Rect::from_min_size(egui::pos2(bars_x, rect.center().y - 4.0 + k as f32 * 5.0), egui::vec2(40.0, 3.0));
                painter.rect_filled(bar, egui::CornerRadius::same(2), ws.divider);
                if f > 0.0 {
                    let mut fill = bar;
                    fill.set_width(40.0 * f.clamp(0.0, 1.0));
                    painter.rect_filled(fill, egui::CornerRadius::same(2), c);
                }
            }
        }
    }
    let resp = match (&r.error, r.physical) {
        (Some(e), _) => resp.on_hover_text(e),
        (None, Some(_)) => resp.on_hover_text(r.condition()),
        _ => resp,
    };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One combatant of the encounter: a marker on the current one, the
/// score, the name and roll, a tag and a menu (acted, delay, seize,
/// blitz, interrupt, score, remove).
fn encounter_row(ui: &mut egui::Ui, c: &chummer_core::campaign::Combatant, kind: String, current: bool, selected: bool, pass: u32, lang: &Language) -> Option<RowDo> {
    let ws = theme::ws(ui);
    let mut out = None;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::hover());
    let resp = ui.interact(rect, ui.id().with(("ws_gm_row", c.id)), egui::Sense::click());
    if resp.clicked() {
        out = Some(RowDo::Select);
    }
    let dim = (pass > 0 && !c.in_pass()) || c.acted;
    let fill = if current { ws.selection } else if resp.hovered() { ws.hover } else { ws.raised };
    let edge = if current || selected { ws.primary } else { ws.divider };
    ui.painter().rect(rect, egui::CornerRadius::same(5), fill, egui::Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
    let mut row = ui.new_child(egui::UiBuilder::new().id_salt(("ws_gm_row_ui", c.id)).max_rect(rect.shrink2(egui::vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // A click anywhere on the row selects it: the labels must not take it.
    row.style_mut().interaction.selectable_labels = false;
    if dim {
        row.set_opacity(0.6);
    }
    row.spacing_mut().item_spacing.x = 8.0;
    if current {
        row.label(icons::icon(icons::CARET_RIGHT, 11.0, ws.accent));
    } else {
        row.add_space(11.0);
    }
    let (score_rect, _) = row.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
    row.painter().text(score_rect.left_center(), egui::Align2::LEFT_CENTER, c.score.to_string(), egui::FontId::monospace(15.0), if current { ws.accent } else { ws.text });
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let menu = widgets::icon_button(ui, icons::DOTS_THREE_VERTICAL, 22.0).on_hover_text(lang.tr("Delay, seize or remove"));
        egui::Popup::menu(&menu).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let mut acted = c.acted;
            if widgets::check(ui, &mut acted, &lang.tr("Acted")).changed() {
                out = Some(RowDo::Acted(acted));
            }
            let mut delayed = c.delayed;
            if widgets::check(ui, &mut delayed, &lang.tr("Delay")).changed() {
                out = Some(RowDo::Delayed(delayed));
            }
            let mut seized = c.seized;
            if ui.add_enabled_ui(!c.seized, |ui| widgets::check(ui, &mut seized, &lang.tr("Seize"))).inner.on_hover_text(lang.tr("Seize the Initiative (spends 1 Edge)")).changed() && seized {
                out = Some(RowDo::Seize);
            }
            let mut blitzed = c.blitzed;
            if ui.add_enabled_ui(pass > 0 && !c.blitzed, |ui| widgets::check(ui, &mut blitzed, &lang.tr("Blitz"))).inner.on_hover_text(lang.tr("Blitz: roll 5d6 (spends 1 Edge)")).changed() && blitzed {
                out = Some(RowDo::Blitz);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(lang.tr("Score"));
                let mut score = c.score;
                if ui.add(egui::DragValue::new(&mut score).range(-40..=60)).changed() {
                    out = Some(RowDo::Score(score));
                }
                if ui.button("−5").on_hover_text(lang.tr("Interrupt action")).clicked() {
                    out = Some(RowDo::Score(c.score - 5));
                }
                if ui.button("−10").clicked() {
                    out = Some(RowDo::Score(c.score - 10));
                }
            });
            ui.separator();
            if ui.button(lang.tr("Remove")).clicked() {
                out = Some(RowDo::Remove);
                ui.close();
            }
        });
        let tag = if c.acted {
            lang.tr("Acted")
        } else if c.delayed {
            lang.tr("Delay")
        } else {
            kind
        };
        widgets::tag(ui, &tag, ws.muted, ws.divider);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(egui::Label::new(RichText::new(&c.name).size(12.5).color(ws.text)).truncate());
            let rolled = if c.rolled.is_empty() { format!("{} + {}d6", c.base, c.dice) } else { format!("{} + [{}]", c.base, c.rolled.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")) };
            ui.add(egui::Label::new(widgets::mono(rolled, 10.5, ws.muted)).truncate());
        });
    });
    out
}

/// The card's title row: the name, a tag, `extra` buttons and the
/// pop-out button.
fn card_title(ui: &mut egui::Ui, pops: &mut PopOuts, lang: &Language, name: &str, tag: &str, extra: impl FnOnce(&mut egui::Ui)) {
    let ws = theme::ws(ui);
    let mut toggle = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(name).font(widgets::bold(17.0)).color(ws.text));
        if !tag.is_empty() {
            widgets::tag(ui, tag, ws.muted, ws.divider);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            toggle = widgets::icon_button(ui, icons::ARROW_SQUARE_OUT, 22.0).on_hover_text(lang.tr("Pop out into its own window")).clicked();
            extra(ui);
        });
    });
    ui.add_space(8.0);
    if toggle {
        pops.toggle(key(Panel::Card), ui.ctx());
    }
}

/// The damage entry: a code (6P, 8P AP-2, 4S), Soak roll (`soak`) and
/// the Apply button. `Some(attack)` when applied.
fn damage_entry(ui: &mut egui::Ui, form: &mut crate::campaign_ui::DamageForm, lang: &Language, apply: &str, soak: bool) -> Option<Attack> {
    let mut out = None;
    let parsed = Attack::parse(&form.code);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let r = ui.add(egui::TextEdit::singleline(&mut form.code).hint_text("6P").desired_width(70.0).font(egui::TextStyle::Monospace));
        if parsed.is_none() {
            r.on_hover_text(lang.tr("Damage code, e.g. 8P AP-2 or 6S"));
        }
        if soak {
            widgets::check(ui, &mut form.soak_roll, &lang.tr("Soak roll"));
        } else if ui.add_enabled_ui(parsed.is_some(), |ui| widgets::button(ui, None, apply, Look::Secondary, 26.0)).inner.clicked() {
            out = parsed;
        }
    });
    if soak && ui.add_enabled_ui(parsed.is_some(), |ui| widgets::button(ui, None, apply, Look::Primary, 26.0)).inner.clicked() {
        out = parsed;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions() {
        assert_eq!(fraction(Some((3, 10))), 0.3);
        assert_eq!(fraction(Some((0, 0))), 0.0);
        assert_eq!(fraction(None), 0.0);
        assert_eq!(Panel::Award.title(), "GM award");
    }
}
