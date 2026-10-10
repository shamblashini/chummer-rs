//! The Workspace's Play screen ("At the table") for a character in
//! career mode: the condition monitor and Edge, initiative, quick rolls,
//! weapons with their ammunition, the Matrix device, vehicles, gear at
//! hand and the session notes; in the inspector, the character's dice
//! roller and its recent rolls. Every block can pop out ([`Panel`]).
//!
//! A child module of `view` (declared there with `#[path]`) so it can
//! use the view's state. It only draws: changes are the commands the
//! Classic tabs and the item pane run (`play_ui` for ammunition), rolls
//! go to the character's own [`DiceRoller`].
//!
//! A character of an online campaign reports every roll to the GM's app
//! ([`CharacterView::report_rolls`]: with what it was rolled for, the
//! dice, the limit and the Rule of Six), and its inspector shows the
//! table's rolls ([`Panel::Table`]): the player's own, the GM's open ones
//! and, when the GM allows it, the other players'.

use std::collections::HashMap;

use chummer_core::campaign::damage::Attack;
use chummer_core::command::Command;
use chummer_core::lang::Language;
use chummer_core::play::{ammo, matrix, vehicle};
use chummer_core::sections;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use super::{CharacterView, Tab};
use crate::campaign_ui::{self, DamageForm};
use crate::dice_ui::{DiceRoller, Outcome};
use crate::doc::Backend;
use crate::gm_screen::rolls::{self, GmRoll};
use crate::pdf_ui::Status;
use crate::theme;
use crate::workspace::popout::{Panel as Block, PopKey, PopOuts};
use crate::workspace::widgets::{self, CmClick, Look, Track};
use crate::workspace::{icons, DocKey, PanelId, Section};

/// A block of the Play screen; each can pop out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// The condition monitor, damage and Edge (pops out as
    /// `PanelId::Condition`).
    Condition,
    Initiative,
    /// The quick-roll tiles.
    Rolls,
    Weapons,
    /// Consumables.
    AtHand,
    Matrix,
    Vehicles,
    Notes,
    /// Inspector: the dice roller.
    Roller,
    /// Inspector: the recent rolls.
    Log,
    /// Inspector, online campaigns: the rolls at the table.
    Table,
}

impl Panel {
    /// The title (English; goes through `lang.tr`).
    pub fn title(self) -> &'static str {
        match self {
            Panel::Condition => "Condition Monitor",
            Panel::Initiative => "Initiative",
            Panel::Rolls => "Quick rolls",
            Panel::Weapons => "Weapons",
            Panel::AtHand => "At hand",
            Panel::Matrix => "Matrix",
            Panel::Vehicles => "Vehicles",
            Panel::Notes => "Session notes",
            Panel::Roller => "Dice Roller",
            Panel::Log => "Recent rolls",
            Panel::Table => "Table rolls",
        }
    }

    /// Its pop-out id.
    pub fn id(self) -> PanelId {
        match self {
            Panel::Condition => PanelId::Condition,
            p => PanelId::Play(p),
        }
    }
}

/// Initiative rolled at the table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Init {
    /// The score now (−10 each pass).
    score: i32,
    rolled: Vec<u8>,
    pass: u32,
}

impl Init {
    /// Whether there is another pass.
    fn has_next(&self) -> bool {
        self.score - 10 > 0
    }

    fn next_pass(&mut self) {
        self.score -= 10;
        self.pass += 1;
    }
}

/// The Play screen's state of a character, for this session.
#[derive(Default)]
pub struct PlayState {
    /// The character's own roller and roll log.
    pub roller: DiceRoller,
    init: Option<Init>,
    /// The weapon last rolled or fired (highlighted).
    weapon: Option<String>,
    /// Fire mode picked per weapon.
    modes: HashMap<String, ammo::FireMode>,
    /// Ammunition choices and the fire question (`play_ui`).
    ammo: crate::play_ui::PlayPanel,
    /// The weapon whose reload choices are open.
    reloading: Option<String>,
    damage: DamageForm,
    /// What the weapon cards show of each weapon's ammunition, by guid,
    /// once per revision.
    ammo_info: crate::memo::Memo<String, AmmoInfo>,
    /// The table's rolls as last read from the campaign replica, with the
    /// replica's roll revision.
    table: Option<(u64, Vec<GmRoll>)>,
}

/// A weapon's ammunition as its card shows it.
#[derive(Clone)]
struct AmmoInfo {
    capacity: i32,
    modes: Vec<ammo::FireMode>,
    loaded: Option<String>,
    loose: f64,
}

impl CharacterView {
    pub(super) fn play_memo_clear(&self) {
        self.play.ammo_info.clear();
    }

    fn ammo_info(&self, w: &Element) -> AmmoInfo {
        let work = || AmmoInfo {
            capacity: ammo::capacity(&self.doc, w),
            modes: ammo::FireMode::ALL.into_iter().filter(|m| ammo::allows(&self.doc, w, *m)).collect(),
            loaded: ammo::loaded(&self.doc, w).map(|g| g.get("name")),
            loose: ammo::reloadable(&self.doc, Some(&self.store), &w.get("guid")).iter().map(|c| c.2).sum(),
        };
        let guid = w.get("guid");
        if guid.is_empty() {
            return work();
        }
        self.play.ammo_info.get(self.doc.revision(), guid, work)
    }
}

/// Gear categories shown "at hand": what is used up at the table.
const CONSUMABLES: &[&str] = &["Biotech", "Drugs", "Toxins", "Chemicals", "Explosives", "Custom Drug"];

/// Short fire-mode labels for the segmented switch.
fn mode_code(m: ammo::FireMode) -> &'static str {
    match m {
        ammo::FireMode::SingleShot => "SS",
        ammo::FireMode::ShortBurst => "SB",
        ammo::FireMode::LongBurst => "LB",
        ammo::FireMode::FullBurst => "FB",
        ammo::FireMode::Suppressive => "SF",
    }
}

/// The time of a log entry ("20:41").
fn clock(at: i64) -> String {
    let t = crate::history_ui::short_time(at);
    t.get(t.len().saturating_sub(5)..).unwrap_or("").to_owned()
}

/// What a header button asks for.
enum Ask {
    Skills,
    ResetEdge,
    ClearLog,
}

impl CharacterView {
    fn play_key(&self, p: Panel) -> PopKey {
        PopKey::new(DocKey::Character(self.ws_id), p.id())
    }

    /// The Play screen. Returns true if the character changed.
    pub fn ws_play(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status, pops: &mut PopOuts) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("ws_play").auto_shrink([false, false]).show(ui, |ui| {
            let total = ui.available_width();
            let right = 330.0_f32.min((total * 0.4).max(240.0));
            let left = (total - right - 12.0).max(300.0);
            let top = ui.cursor().min;
            // Two columns, each clipped to its own width.
            let column = |ui: &mut egui::Ui, x: f32, w: f32, gap: f32| {
                let rect = egui::Rect::from_min_size(egui::pos2(top.x + x, top.y), egui::vec2(w, f32::INFINITY));
                let mut c = ui.new_child(egui::UiBuilder::new().id_salt(("ws_play_column", x as i32)).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
                c.set_clip_rect(ui.clip_rect().intersect(rect.expand2(egui::vec2(1.0, 0.0))));
                c.set_width(w);
                c.spacing_mut().item_spacing.y = gap;
                c
            };
            let mut l = column(ui, 0.0, left, 12.0);
            for p in [Panel::Condition, Panel::Initiative, Panel::Rolls] {
                changed |= self.play_block(&mut l, p, lang, status, pops);
            }
            let side: Vec<Panel> = [Panel::AtHand, Panel::Matrix].into_iter().filter(|p| self.play_has(*p, lang)).collect();
            if !side.is_empty() {
                let w = ((left - 12.0 * (side.len() as f32 - 1.0)) / side.len() as f32).floor();
                let row_top = l.cursor().min;
                let mut bottom = row_top.y;
                for (k, p) in side.into_iter().enumerate() {
                    let rect = egui::Rect::from_min_size(egui::pos2(row_top.x + k as f32 * (w + 12.0), row_top.y), egui::vec2(w, f32::INFINITY));
                    let mut c = l.new_child(egui::UiBuilder::new().id_salt(("ws_play_side", k)).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
                    c.set_width(w);
                    changed |= self.play_block(&mut c, p, lang, status, pops);
                    bottom = bottom.max(c.min_rect().bottom());
                }
                l.allocate_space(egui::vec2(left, bottom - row_top.y));
            }
            changed |= self.play_block(&mut l, Panel::Notes, lang, status, pops);
            let mut r = column(ui, left + 12.0, right, 8.0);
            changed |= self.play_block(&mut r, Panel::Weapons, lang, status, pops);
            if self.play_has(Panel::Vehicles, lang) {
                r.add_space(4.0);
                changed |= self.play_block(&mut r, Panel::Vehicles, lang, status, pops);
            }
            let height = l.min_rect().bottom().max(r.min_rect().bottom()) - top.y;
            ui.allocate_space(egui::vec2(total, height));
        });
        self.report_rolls();
        changed
    }

    /// The session of the character's online campaign, for a player.
    fn player_session(&self) -> Option<(chummer_sync::PlayerSession, chummer_sync::CharacterId)> {
        match self.doc.backend() {
            Some(Backend::Player { session, id }) => Some((session.clone(), id.clone())),
            _ => None,
        }
    }

    /// Rolls made since the last call go to the GM's app when the
    /// character is in an online campaign (a log next to the character:
    /// nothing about it changes). Made here, so the GM takes them on
    /// trust; the hits are worked out from the dice. On the GM's own app
    /// (a member's tab) they join the campaign's roll log as the GM's.
    pub fn report_rolls(&mut self) {
        let new = self.play.roller.take_new();
        if new.is_empty() {
            return;
        }
        let name = self.doc.display_name();
        for rec in new.iter().filter_map(|e| e.record()) {
            match self.doc.backend() {
                Some(Backend::Player { session, id }) => {
                    if let Err(err) = session.roll_now(id, rec) {
                        eprintln!("could not report the roll to the GM: {err}");
                    }
                }
                // The GM rolling on a member's own tab: into the
                // campaign's roll log like the GM screen's rolls.
                Some(Backend::Gm { host, id }) => {
                    let open = host.authority().roll_settings().show_gm_rolls;
                    host.gm_roll(Some(id.clone()), &name, open, rec);
                }
                None => {}
            }
        }
    }

    /// Whether a block has something to show.
    fn play_has(&self, p: Panel, lang: &Language) -> bool {
        match p {
            Panel::AtHand => !self.at_hand(lang).is_empty(),
            Panel::Matrix => self.play_device().is_some(),
            Panel::Vehicles => !self.doc.items("vehicles", "vehicle").is_empty(),
            Panel::Table => self.player_session().is_some(),
            _ => true,
        }
    }

    /// One block, docked: its header and, unless it is out, its body.
    fn play_block(&mut self, ui: &mut egui::Ui, p: Panel, lang: &Language, status: &mut Status, pops: &mut PopOuts) -> bool {
        let ws = theme::ws(ui);
        let key = self.play_key(p);
        let title = lang.tr(p.title());
        let block = match p {
            Panel::Rolls | Panel::Notes | Panel::Weapons | Panel::Vehicles => Block::bare(key, &title),
            Panel::Roller | Panel::Log | Panel::Table => Block::inspector(key, &title),
            _ => Block::card(key, &title),
        };
        // Header contents, worked out before the body borrows the view.
        let s = &self.sheet;
        let note = match p {
            Panel::Condition => Some(lang.tr("Click a box to mark it; click again to clear.")),
            Panel::Initiative => self.play.init.as_ref().map(|i| lang.tr_fmt("Pass {0}", &[&i.pass])),
            Panel::Rolls => Some(if s.wound_modifier != 0 { format!("{} · {} {}", lang.tr("click to roll"), lang.tr("CM Penalty:"), s.wound_modifier) } else { lang.tr("click to roll") }),
            _ => None,
        };
        let mono = (p == Panel::Initiative).then(|| format!("{} + {}d6", s.initiative, s.initiative_dice));
        let active = p == Panel::Matrix && self.play_device().is_some_and(|d| d.get_bool("active").unwrap_or(false));
        let has_log = !self.play.roller.history().is_empty();
        let mut ask = None;
        let mut asked = None;
        let mut changed = false;
        block.show(
            ui,
            pops,
            lang,
            |ui| {
                match p {
                    Panel::Rolls => {
                        if widgets::button(ui, Some(icons::LIGHTNING), &lang.tr("Skills"), Look::Ghost, 22.0).clicked() {
                            asked = Some(Ask::Skills);
                        }
                    }
                    Panel::Log if has_log => {
                        if widgets::button(ui, Some(icons::TRASH), &lang.tr("Clear"), Look::Ghost, 22.0).clicked() {
                            asked = Some(Ask::ClearLog);
                        }
                    }
                    Panel::Matrix if active => {
                        widgets::tag(ui, &lang.tr("Active Commlink"), ws.accent, ws.primary);
                    }
                    _ => {}
                }
                if let Some(n) = &note {
                    ui.label(RichText::new(n).size(11.5).color(ws.muted));
                }
                if let Some(m) = &mono {
                    ui.add_space(4.0);
                    ui.label(widgets::mono(m, 12.0, ws.muted));
                }
            },
            |ui| changed |= self.ws_play_body(ui, p, lang, status, &mut ask),
        );
        match ask.or(asked) {
            Some(Ask::Skills) => self.tab = Tab::Skills,
            Some(Ask::ResetEdge) => changed |= self.doc.set(Command::RefreshEdge),
            Some(Ask::ClearLog) => self.play.roller = DiceRoller::default(),
            None => {}
        }
        changed
    }

    /// A block's contents (docked or in its own window). Returns true if
    /// the character changed.
    fn ws_play_body(&mut self, ui: &mut egui::Ui, p: Panel, lang: &Language, status: &mut Status, ask: &mut Option<Ask>) -> bool {
        let _s = crate::trace::span(p.title());
        match p {
            Panel::Condition => self.play_condition(ui, lang, ask),
            Panel::Initiative => {
                self.play_initiative(ui, lang);
                false
            }
            Panel::Rolls => {
                self.play_rolls(ui, lang);
                false
            }
            Panel::Weapons => self.play_weapons(ui, lang, status),
            Panel::AtHand => {
                self.play_at_hand(ui, lang);
                false
            }
            Panel::Matrix => self.play_matrix(ui, lang),
            Panel::Vehicles => self.play_vehicles(ui, lang),
            Panel::Notes => self.play_notes(ui, lang),
            Panel::Roller => {
                self.play_roller(ui, lang);
                false
            }
            Panel::Log => {
                self.play_log(ui, lang);
                false
            }
            Panel::Table => {
                self.play_table(ui, lang);
                false
            }
        }
    }

    /// A block in its own window. Returns true if the character changed.
    pub fn ws_play_panel(&mut self, ui: &mut egui::Ui, p: Panel, lang: &Language, status: &mut Status) -> bool {
        let mut ask = None;
        let changed = self.ws_play_body(ui, p, lang, status, &mut ask);
        self.report_rolls();
        match ask {
            Some(Ask::ResetEdge) => self.doc.set(Command::RefreshEdge) || changed,
            Some(Ask::ClearLog) => {
                self.play.roller = DiceRoller::default();
                changed
            }
            Some(Ask::Skills) | None => changed,
        }
    }

    /// The Play screen's inspector: the dice roller and the recent rolls.
    pub fn ws_play_inspector(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status, pops: &mut PopOuts) -> bool {
        let mut changed = false;
        for p in [Panel::Roller, Panel::Log, Panel::Table] {
            if self.play_has(p, lang) {
                changed |= self.play_block(ui, p, lang, status, pops);
            }
        }
        self.report_rolls();
        changed
    }

    // ----- condition -----

    fn play_condition(&mut self, ui: &mut egui::Ui, lang: &Language, ask: &mut Option<Ask>) -> bool {
        let ws = theme::ws(ui);
        let s = self.sheet.clone();
        let mut changed = false;
        let (plabel, slabel) = crate::ai_ui::cm_labels(&self.doc, lang);
        let physical = Track { label: plabel, color: ws.physical, boxes: s.physical_cm, filled: chummer_core::play::ai::physical_filled(&self.doc), threshold: s.cm_threshold };
        let stun = Track { label: slabel, color: ws.stun, boxes: s.stun_cm, filled: chummer_core::play::ai::stun_filled(&self.doc), threshold: if self.doc.is_ai() { 0 } else { s.cm_threshold } };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            let top = ui.cursor().top();
            match widgets::condition_monitor(ui, "ws_cm", &physical, &stun, s.cm_overflow, &lang.tr("Overflow")) {
                Some(CmClick::Physical(n)) => changed |= self.doc.set(Command::SetPhysicalDamage { filled: n }),
                Some(CmClick::Stun(n)) => changed |= self.doc.set(Command::SetStunDamage { filled: n }),
                None => {}
            }
            let height = (ui.min_rect().bottom() - top).max(100.0);
            widgets::divider(ui, Some(height));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let penalty = format!("{} {}", lang.tr("CM Penalty:"), s.wound_modifier);
                    if s.wound_modifier != 0 {
                        widgets::tag(ui, &penalty, ws.warning, ws.warning);
                    } else {
                        ui.label(RichText::new(penalty).size(11.5).color(ws.muted));
                    }
                    ui.label(RichText::new(format!("{} {}", lang.tr("Armor"), s.armor)).size(11.5).color(ws.muted));
                });
                changed |= self.play_damage(ui, lang);
                if self.doc.created {
                    widgets::divider(ui, None);
                    let total = s.attr("EDG").max(0);
                    let used = self.doc.doc.get_i32("edgeused").unwrap_or(0).clamp(0, total);
                    let available = total - used;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(lang.tr("Edge")).font(widgets::bold(12.5)).color(ws.text));
                        let tip = |n: i32, on: bool| format!("{} {n} / {total}: {}", lang.tr("Edge"), if on { lang.tr("available") } else { lang.tr("spent") });
                        if let Some(a) = widgets::edge_boxes(ui, "ws_edge", total, available, tip) {
                            changed |= self.doc.set(Command::SetEdgeUsed { used: total - a });
                        }
                        ui.label(widgets::mono(format!("{available}/{total}"), 11.5, ws.muted));
                        if widgets::icon_button(ui, icons::ARROWS_CLOCKWISE, 24.0).on_hover_text(lang.tr("Reset")).clicked() {
                            *ask = Some(Ask::ResetEdge);
                        }
                    });
                }
            });
        });
        changed
    }

    /// Damage taken: a code like 6P or 4S AP-2, soaked (rolled or not) as
    /// on the GM screen.
    fn play_damage(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(widgets::overline(&lang.tr("Damage"), &ws));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = ui.add(egui::TextEdit::singleline(&mut self.play.damage.code).hint_text("6P").desired_width(80.0).font(egui::TextStyle::Monospace));
                let parsed = Attack::parse(&self.play.damage.code);
                if parsed.is_none() {
                    r.on_hover_text(lang.tr("Damage code, e.g. 8P AP-2 or 6S"));
                }
                widgets::check(ui, &mut self.play.damage.soak_roll, &lang.tr("Soak roll"));
                let apply = ui.add_enabled_ui(parsed.is_some(), |ui| widgets::button(ui, None, &lang.tr("Apply"), Look::Secondary, 26.0)).inner.clicked();
                if let (true, Some(a)) = (apply, parsed) {
                    let (t, r) = campaign_ui::damage_character(self.play.roller.rng(), a, &self.doc, &self.sheet, self.play.damage.soak_roll);
                    if let Some((pool, roll)) = r.soak.clone() {
                        self.play.roller.logged(&lang.tr("Soak"), pool, roll);
                    }
                    self.play.roller.note(&lang.tr("Damage"), r.text.clone());
                    for c in campaign_ui::damage_commands(&t, &r) {
                        changed |= self.doc.set(c);
                    }
                }
            });
        });
        changed
    }

    // ----- initiative -----

    fn play_initiative(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let (base, dice) = (self.sheet.initiative, self.sheet.initiative_dice.max(1) as u32);
        let mut roll = false;
        let mut next = false;
        let mut reset = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            match &self.play.init {
                Some(i) => {
                    ui.label(widgets::mono(i.score.to_string(), 28.0, ws.accent));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let d: Vec<String> = i.rolled.iter().map(u8::to_string).collect();
                        ui.label(RichText::new(format!("{base} + [{}]", d.join(" "))).size(11.5).color(ws.muted));
                        let after = if i.has_next() { lang.tr_fmt("next pass at {0}", &[&(i.score - 10)]) } else { lang.tr("last pass") };
                        ui.label(RichText::new(after).size(11.5).color(ws.muted));
                    });
                }
                None => {
                    ui.label(widgets::mono("—", 28.0, ws.muted));
                    ui.label(RichText::new(lang.tr("Roll at the start of each combat turn.")).size(11.5).color(ws.muted));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let any = self.play.init.is_some();
                if ui.add_enabled_ui(any, |ui| widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, 26.0)).inner.on_hover_text(lang.tr("Reset")).clicked() {
                    reset = true;
                }
                let can_next = self.play.init.as_ref().is_some_and(Init::has_next);
                if ui.add_enabled_ui(can_next, |ui| widgets::button(ui, None, &format!("{} −10", lang.tr("Next pass")), Look::Secondary, 26.0)).inner.on_hover_text(lang.tr("Everyone loses 10")).clicked() {
                    next = true;
                }
                if widgets::button(ui, Some(icons::DICE_FIVE), &lang.tr("Roll"), Look::Primary, 26.0).clicked() {
                    roll = true;
                }
            });
        });
        if roll {
            let (score, rolled) = self.play.roller.initiative(&lang.tr("Initiative"), base, dice);
            self.play.init = Some(Init { score, rolled, pass: 1 });
        }
        if next {
            if let Some(i) = &mut self.play.init {
                i.next_pass();
            }
        }
        if reset {
            self.play.init = None;
        }
    }

    // ----- quick rolls -----

    /// The tiles: the table's pools, the best skills, the weapons.
    fn play_pools(&self, lang: &Language) -> Vec<(String, i32, String)> {
        let mut out: Vec<(String, i32, String)> = campaign_ui::quick_pools(&self.doc, &self.sheet, lang, 8)
            .into_iter()
            .map(|p| {
                let shown = match p.spec {
                    Some(s) => format!("{} ({s})", p.pool),
                    None => p.pool.to_string(),
                };
                (p.label, p.pool, shown)
            })
            .collect();
        for w in self.doc.items("weapons", "weapon").into_iter().take(4) {
            let st = self.weapon_stats(w, false);
            out.push((super::display_name(&sections::WEAPONS, w, lang), st.dice_pool, st.dice_pool.to_string()));
        }
        out
    }

    fn play_rolls(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let pools = self.play_pools(lang);
        let gap = 5.0;
        let cols = if ui.available_width() < 420.0 { 2 } else { 4 };
        let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let mut roll = None;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            for row in pools.chunks(cols) {
                ui.horizontal(|ui| {
                    for (label, pool, shown) in row {
                        if widgets::roll_tile(ui, label, shown, w, 28.0).on_hover_text(lang.tr_fmt("Roll {0} dice", &[pool])).clicked() {
                            roll = Some((label.clone(), *pool));
                        }
                    }
                });
            }
        });
        if let Some((label, pool)) = roll {
            self.play.roller.roll_as(&label, pool.max(0) as u32, None);
        }
    }

    // ----- weapons -----

    fn play_weapons(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let weapons: Vec<Element> = self.doc.items("weapons", "weapon").into_iter().cloned().collect();
        if weapons.is_empty() {
            ui.label(RichText::new(lang.tr("No weapons.")).size(12.0).color(ws.muted));
        }
        for w in &weapons {
            // Each card has its own ids (two of the same weapon look alike).
            changed |= ui.push_id(("ws_weapon", w.get("guid")), |ui| self.play_weapon(ui, lang, status, w)).inner;
        }
        self.play_armor(ui, lang);
        self.play_ammunition(ui, lang);
        changed
    }

    fn play_weapon(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status, w: &Element) -> bool {
        let ws = theme::ws(ui);
        let guid = w.get("guid");
        let st = self.weapon_stats(w, false);
        let name = super::display_name(&sections::WEAPONS, w, lang);
        let selected = self.play.weapon.as_deref() == Some(guid.as_str());
        let melee = st.ranges.short.is_empty();
        let mut changed = false;
        let frame = egui::Frame::new()
            .fill(if selected { ws.selection } else { ws.raised })
            .stroke(egui::Stroke::new(1.0_f32, if selected { ws.primary } else { ws.divider }))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(10, 9));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&name).font(widgets::bold(13.0)).color(ws.text)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let look = if selected { Look::Primary } else { Look::Secondary };
                    let label = format!("{} {}", lang.tr("Roll"), st.dice_pool);
                    if widgets::button(ui, Some(icons::DICE_FIVE), &label, look, 24.0).on_hover_text(lang.tr_fmt("Limit {0}", &[&st.accuracy])).clicked() {
                        let what = if st.skill.is_empty() { name.clone() } else { format!("{} · {name}", lang.data_name("skills.xml", "", &st.skill)) };
                        self.play.roller.roll_as(&what, st.dice_pool.max(0) as u32, Some(st.accuracy.max(0) as u32));
                        self.play.weapon = Some(guid.clone());
                    }
                });
            });
            let ap = if st.ap.is_empty() { "-".to_owned() } else { st.ap.clone() };
            let mut parts = vec![st.damage.clone(), format!("{} {ap}", lang.tr("AP")), format!("{} {}", lang.tr("Acc"), st.accuracy)];
            if melee {
                parts.push(format!("{} {}", lang.tr("Reach"), st.reach));
            } else {
                let mode = w.get("mode");
                if !mode.is_empty() {
                    parts.push(mode);
                }
                parts.push(format!("{} {}", lang.tr("RC"), st.rc));
            }
            ui.label(RichText::new(parts.join(" · ")).size(11.5).color(ws.muted));
            if self.doc.created && ammo::uses_ammo(w) && !ammo::clips(w).is_empty() {
                changed |= self.play_ammo(ui, lang, status, w);
            }
        });
        changed
    }

    /// Rounds, fire modes, Fire and Reload of a weapon (career mode).
    fn play_ammo(&mut self, ui: &mut egui::Ui, lang: &Language, status: &mut Status, w: &Element) -> bool {
        let ws = theme::ws(ui);
        let guid = w.get("guid");
        let mut changed = false;
        let left = ammo::remaining(w);
        let info = self.ammo_info(w);
        let capacity = info.capacity.max(left);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if capacity > 0 && capacity <= 48 {
                widgets::ammo_pips(ui, left, capacity);
            }
            ui.label(widgets::mono(format!("{left}/{capacity}"), 11.5, ws.muted));
        });
        let modes = info.modes;
        let mode = self.play.modes.get(&guid).copied().filter(|m| modes.contains(m)).or_else(|| modes.first().copied());
        let asking = self.play.ammo.is_for(&guid) && self.play.ammo.asking();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !modes.is_empty() {
                let tips: Vec<String> = modes
                    .iter()
                    .map(|m| {
                        let n = ammo::rounds(w, *m);
                        lang.tr_fmt(m.label(), &[&n, &lang.tr(if n == 1 { "Bullet" } else { "Bullets" })])
                    })
                    .collect();
                let items: Vec<(&str, &str)> = modes.iter().zip(&tips).map(|(m, t)| (mode_code(*m), t.as_str())).collect();
                let at = mode.and_then(|m| modes.iter().position(|x| *x == m)).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, &items, at, 24.0) {
                    self.play.modes.insert(guid.clone(), modes[i]);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let open = self.play.reloading.as_deref() == Some(guid.as_str());
                let look = if open { Look::Outline } else { Look::Secondary };
                if widgets::button(ui, Some(icons::ARROWS_CLOCKWISE), &lang.tr("Reload"), look, 24.0).clicked() {
                    self.play.reloading = if open { None } else { Some(guid.clone()) };
                    self.play.ammo.for_weapon(&guid);
                }
                let can = left > 0 && mode.is_some() && !asking;
                if ui.add_enabled_ui(can, |ui| widgets::button(ui, Some(icons::CROSSHAIR), &lang.tr("Fire"), Look::Secondary, 24.0)).inner.clicked() {
                    if let Some(m) = mode {
                        self.play.ammo.for_weapon(&guid);
                        changed |= self.play.ammo.fire(&mut self.doc, lang, &guid, m, status);
                        self.play.weapon = Some(guid.clone());
                    }
                }
            });
        });
        if self.play.ammo.is_for(&guid) {
            changed |= self.play.ammo.confirm_ui(ui, &mut self.doc, lang, &guid);
        }
        // What is loaded and what is left to load.
        let slot = ammo::active_slot(w);
        let loaded = info.loaded.unwrap_or_else(|| lang.tr(if left > 0 { "External Source" } else { "None" }));
        let spare = ammo::clips(w).iter().enumerate().filter(|(i, c)| i + 1 != slot && c.count > 0).count();
        let loose = info.loose;
        let mut line = vec![loaded];
        if spare > 0 {
            line.push(lang.tr_fmt("{0} spare clips", &[&spare]));
        }
        if loose > 0.0 {
            line.push(lang.tr_fmt("{0} rounds to reload", &[&chummer_core::improvement::fmt_num(loose)]));
        }
        ui.label(RichText::new(line.join(" · ")).size(11.0).color(ws.muted));
        if self.play.reloading.as_deref() == Some(guid.as_str()) {
            self.play.ammo.for_weapon(&guid);
            let w = w.clone();
            changed |= if ammo::requires_ammo(&w) { self.play.ammo.reload_ui(ui, &mut self.doc, &self.store, lang, &w, status) } else { self.play.ammo.charges_ui(ui, &mut self.doc, lang, &w) };
            if changed {
                self.play.reloading = None;
            }
        }
        changed
    }

    /// The armor worn and its total.
    fn play_armor(&self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let worn: Vec<String> = self.doc.items("armors", "armor").into_iter().filter(|a| a.get_bool("equipped").unwrap_or(false)).map(|a| super::display_name(&sections::ARMOR, a, lang)).collect();
        widgets::card_frame(&ws).inner_margin(egui::Margin::same(9)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icons::icon(icons::SHIELD, 14.0, ws.muted));
                let text = if worn.is_empty() { lang.tr("Armor") } else { worn.join(" + ") };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(widgets::mono(self.sheet.armor.to_string(), 13.0, ws.accent));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(ws.text)).truncate());
                    });
                });
            });
        });
    }

    /// Ammunition carried, with the rounds of each.
    fn play_ammunition(&self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let rounds: Vec<(String, String)> = self
            .doc
            .items("gears", "gear")
            .into_iter()
            .filter(|g| g.get("category") == "Ammunition")
            .map(|g| (super::display_name(&sections::GEAR, g, lang), chummer_core::improvement::fmt_num(g.get_f64("qty").unwrap_or(0.0))))
            .collect();
        if rounds.is_empty() {
            return;
        }
        widgets::card_frame(&ws).inner_margin(egui::Margin::same(9)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.label(icons::icon(icons::PACKAGE, 14.0, ws.muted));
                ui.label(RichText::new(lang.tr("Ammunition")).size(12.5).color(ws.text));
            });
            ui.add_space(3.0);
            for (n, q) in rounds {
                widgets::list_row(ui, &n, &q, 20.0, false);
            }
        });
    }

    // ----- at hand -----

    /// Consumables (name, quantity, where they are listed, guid): gear in
    /// the consumable categories, and drugs.
    fn at_hand(&self, lang: &Language) -> Vec<(String, String, Section, String)> {
        let qty = |e: &Element| chummer_core::improvement::fmt_num(e.get_f64("qty").unwrap_or(1.0));
        let mut out: Vec<(String, String, Section, String)> =
            self.doc.items("gears", "gear").into_iter().filter(|g| CONSUMABLES.contains(&g.get("category").as_str())).map(|g| (super::display_name(&sections::GEAR, g, lang), qty(g), Section::Gear(0), g.get("guid"))).collect();
        out.extend(self.doc.items("drugs", "drug").into_iter().map(|d| (d.get("name"), qty(d), Section::Gear(3), d.get("guid"))));
        out
    }

    fn play_at_hand(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let mut show = None;
        for (i, (name, qty, section, guid)) in self.at_hand(lang).into_iter().enumerate() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
                let mut row = ui.new_child(egui::UiBuilder::new().id_salt(("ws_at_hand", i)).max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let ws = theme::ws(&row);
                if i > 0 {
                    row.painter().hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0_f32, ws.divider));
                }
                row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if widgets::button(ui, None, &lang.tr("Show"), Look::Secondary, 22.0).on_hover_text(lang.tr("Open the item")).clicked() {
                        show = Some((section, guid.clone()));
                    }
                    ui.label(widgets::mono(format!("×{qty}"), 11.5, ws.muted));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&name).size(12.5).color(ws.text)).truncate());
                    });
                });
            });
        }
        if let Some((section, guid)) = show {
            self.ws_show_item(section, &guid);
        }
    }

    // ----- matrix -----

    /// The device shown: the active commlink, else the first commlink.
    fn play_device(&self) -> Option<&Element> {
        matrix::active_commlink(&self.doc).or_else(|| matrix::commlinks(&self.doc).into_iter().next())
    }

    fn play_matrix(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let Some(dev) = self.play_device().cloned() else { return false };
        let guid = dev.get("guid");
        let s = &self.sheet;
        let mut changed = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let w = (ui.available_width() - 70.0).max(100.0);
            ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 3.0;
                let small = |ui: &mut egui::Ui, t: String| ui.add(egui::Label::new(RichText::new(t).size(11.5).color(ws.muted)).truncate());
                ui.add(egui::Label::new(RichText::new(super::display_name(&sections::GEAR, &dev, lang)).size(12.5).color(ws.text)).truncate());
                let attr = |l: &str| matrix::total(&dev, l);
                small(ui, format!("{} {} · A{} S{} D{} F{}", lang.tr("Device Rating"), attr("Device Rating"), attr("Attack"), attr("Sleaze"), attr("Data Processing"), attr("Firewall")));
                small(ui, format!("{} {} + {}d6", lang.tr("Matrix cold-sim"), s.matrix_cold_initiative, s.matrix_cold_dice));
                small(ui, format!("{} {} + {}d6", lang.tr("Matrix hot-sim"), s.matrix_hot_initiative, s.matrix_hot_dice));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let mut wireless = dev.get_bool("wirelesson").unwrap_or(false);
                    if widgets::check(ui, &mut wireless, &lang.tr("Wireless")).changed() {
                        changed |= self.doc.set(Command::SetItemWireless { guid: guid.clone(), on: wireless });
                    }
                    if matrix::is_commlink(&dev) {
                        let mut on = dev.get_bool("active").unwrap_or(false);
                        if widgets::check(ui, &mut on, &lang.tr("Active Commlink")).changed() {
                            changed |= self.doc.set(Command::SetActiveCommlink { device: guid.clone(), on });
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(egui::Label::new(widgets::overline(&lang.tr("Matrix CM"), &ws)).extend());
                let label = lang.tr("Matrix Condition Monitor");
                if let Some(f) = widgets::small_track(ui, ("ws_mcm", &guid), matrix::condition_monitor(&dev), matrix::filled(&dev), 3, 16.0, ws.accent, &label) {
                    changed |= self.doc.set(Command::SetMatrixDamage { device: guid.clone(), filled: f });
                }
            });
        });
        changed
    }

    // ----- vehicles -----

    fn play_vehicles(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let ws = theme::ws(ui);
        let mut changed = false;
        let mut open = None;
        let vehicles: Vec<Element> = self.doc.items("vehicles", "vehicle").into_iter().cloned().collect();
        for v in &vehicles {
            let guid = v.get("guid");
            let st = chummer_core::items::vehicle::stats(v);
            widgets::card_frame(&ws).inner_margin(egui::Margin::same(9)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 5.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(icons::icon(if st.is_drone { icons::ROBOT } else { icons::CAR }, 14.0, ws.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, Some(icons::ARROW_SQUARE_OUT), &lang.tr("Open"), Look::Ghost, 22.0).clicked() {
                            open = Some(guid.clone());
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(super::display_name(&sections::VEHICLES, v, lang)).size(12.5).color(ws.text)).truncate());
                        });
                    });
                });
                let line = format!(
                    "{} {} · {} {} · {} {} · {} {} · {} {}",
                    lang.tr("Handling"),
                    st.handling_text,
                    lang.tr("Speed"),
                    st.speed_text,
                    lang.tr("Body"),
                    st.body,
                    lang.tr("Armor"),
                    st.armor,
                    lang.tr("Pilot"),
                    st.pilot
                );
                ui.label(RichText::new(line).size(11.5).color(ws.muted));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(widgets::overline(&lang.tr("Damage"), &ws));
                    let boxes = vehicle::condition_monitor(v, &Default::default());
                    let filled = vehicle::filled(v);
                    let per_row = ((ui.available_width() - 50.0) / 16.0).floor().max(4.0) as i32;
                    if let Some(f) = widgets::small_track(ui, ("ws_vcm", &guid), boxes, filled, per_row, 14.0, ws.physical, &lang.tr("Damage")) {
                        changed |= self.doc.set(Command::SetVehicleDamage { vehicle: guid.clone(), filled: f });
                    }
                    ui.label(widgets::mono(format!("{filled}/{boxes}"), 11.0, ws.muted));
                });
            });
        }
        if let Some(g) = open {
            self.ws_show_item(Section::Page(Tab::Vehicles), &g);
        }
        changed
    }

    // ----- notes -----

    /// The session notes: the character's Game Notes.
    fn play_notes(&mut self, ui: &mut egui::Ui, lang: &Language) -> bool {
        let mut v = self.doc.field("gamenotes");
        let r = ui.add(egui::TextEdit::multiline(&mut v).id_salt("ws_play_notes").hint_text(lang.tr("Game Notes")).desired_width(f32::INFINITY).desired_rows(3));
        r.changed() && self.doc.set(Command::SetField { key: "gamenotes".into(), value: v })
    }

    // ----- inspector -----

    fn play_roller(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        ui.spacing_mut().item_spacing.y = 8.0;
        let roller = &mut self.play.roller;
        if let Some(e) = roller.last().cloned() {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let title = if e.label.is_empty() { lang.tr("Roll") } else { e.label.clone() };
                ui.add(egui::Label::new(RichText::new(title).font(widgets::bold(12.5)).color(ws.text)).wrap());
                let mut detail = lang.tr_fmt("{0} dice", &[&e.pool]);
                if let Some(l) = e.limit {
                    detail = format!("{detail} · {}", lang.tr_fmt("Limit {0}", &[&l]));
                }
                if !matches!(e.outcome, Outcome::Note(_)) {
                    ui.label(RichText::new(detail).size(11.5).color(ws.muted));
                }
            });
            match &e.outcome {
                Outcome::Hits(r) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        for d in &r.dice {
                            widgets::die_face(ui, *d);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(widgets::mono(r.hits.to_string(), 22.0, ws.accent));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new(lang.tr("hits")).font(widgets::bold(12.5)).color(ws.text));
                            let glitch = match r.glitch {
                                chummer_core::dice::Glitch::None => lang.tr("no glitch"),
                                chummer_core::dice::Glitch::Glitch => lang.tr("GLITCH"),
                                chummer_core::dice::Glitch::Critical => lang.tr("CRITICAL GLITCH"),
                            };
                            let color = if r.glitch == chummer_core::dice::Glitch::None { ws.muted } else { ws.error };
                            ui.label(RichText::new(format!("{} · {glitch}", lang.tr_fmt("{0} ones", &[&r.ones]))).size(11.5).color(color));
                        });
                    });
                }
                Outcome::Initiative { score, dice } => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        for d in dice {
                            widgets::die_face(ui, *d);
                        }
                    });
                    ui.label(widgets::mono(score.to_string(), 22.0, ws.accent));
                }
                Outcome::Note(t) => {
                    ui.label(RichText::new(t).size(12.0).color(ws.text));
                }
            }
        } else {
            ui.label(RichText::new(lang.tr("Click a pool or a weapon's Roll to roll it here.")).size(11.5).color(ws.muted));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(lang.tr("Dice")).size(11.5).color(ws.muted));
            ui.add(egui::DragValue::new(&mut roller.pool).range(1..=100));
            widgets::check(ui, &mut roller.use_limit, &lang.tr("Limit"));
            ui.add_enabled(roller.use_limit, egui::DragValue::new(&mut roller.limit).range(0..=50));
        });
        widgets::check(ui, &mut roller.rule_of_six, &lang.tr("Rule of Six (Edge)"));
        let label = lang.tr_fmt("Roll {0} dice", &[&roller.pool]);
        if widgets::wide_button(ui, Some(icons::DICE_FIVE), &label, Look::Primary, 28.0).clicked() {
            roller.roll();
        }
    }

    /// The rolls at the table (online campaigns): ours (those not at the
    /// GM's app yet marked), the GM's open ones and, when the GM allows
    /// it, the other players', newest first.
    fn play_table(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let Some((session, _)) = self.player_session() else { return };
        if let Some(r) = session.try_replica() {
            let rev = r.rolls_rev();
            if self.play.table.as_ref().map(|t| t.0) != Some(rev) {
                let me = r.me();
                let mut list: Vec<GmRoll> = r
                    .table_rolls()
                    .iter()
                    .map(|t| {
                        let player = (t.author_role == chummer_net::invite::Role::Player && Some(t.author) != me).then(|| t.author_name.clone());
                        GmRoll::from_table(t, player)
                    })
                    .collect();
                list.extend(r.roll_outbox().iter().map(|p| {
                    let who = p.report.character.as_ref().and_then(|c| r.name(c)).unwrap_or_default();
                    GmRoll { waiting: true, ..GmRoll::new(who, None, p.report.roll.clone()) }
                }));
                list.sort_by_key(|g| std::cmp::Reverse(g.at()));
                self.play.table = Some((rev, list));
            }
        }
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(lang.tr("Your rolls go to the GM. Here: yours, the GM's open rolls, and the other players' when the GM allows it.")).size(11.5).color(ws.muted));
        match self.play.table.as_ref().map(|t| t.1.as_slice()) {
            Some(list) if !list.is_empty() => rolls::rolls_list(ui, lang, list),
            _ => {
                ui.label(RichText::new(lang.tr("No rolls at the table yet.")).size(11.5).color(ws.muted));
            }
        }
    }

    fn play_log(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        let history = self.play.roller.history();
        if history.is_empty() {
            ui.label(RichText::new(lang.tr("No rolls yet.")).size(11.5).color(ws.muted));
            return;
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        for e in history {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            painter.hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0_f32, ws.divider));
            let time = painter.layout_no_wrap(clock(e.at), egui::FontId::monospace(11.0), ws.muted);
            let what = match &e.outcome {
                Outcome::Hits(_) => {
                    let l = if e.label.is_empty() { lang.tr("Roll") } else { e.label.clone() };
                    format!("{l} {}", e.pool)
                }
                _ => e.label.clone(),
            };
            let result = painter.layout_no_wrap(e.result(lang), egui::FontId::monospace(11.5), ws.text);
            let tx = rect.right() - time.size().x;
            let rx = tx - 8.0 - result.size().x;
            let label = painter.layout_no_wrap(what, egui::FontId::proportional(12.0), ws.text);
            let y = |h: f32| rect.center().y - h / 2.0;
            painter.with_clip_rect(egui::Rect::from_min_max(rect.min, egui::pos2(rx - 8.0, rect.bottom()))).galley(egui::pos2(rect.left(), y(label.size().y)), label, ws.text);
            painter.with_clip_rect(egui::Rect::from_min_max(egui::pos2(rect.left() + 60.0, rect.top()), rect.max)).galley(egui::pos2(rx, y(result.size().y)), result, ws.text);
            painter.galley(egui::pos2(tx, y(time.size().y)), time, ws.muted);
            resp.on_hover_text(e.line(lang));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_lose_ten() {
        let mut i = Init { score: 23, rolled: vec![6, 4, 3], pass: 1 };
        assert!(i.has_next());
        i.next_pass();
        assert_eq!((i.score, i.pass), (13, 2));
        i.next_pass();
        assert_eq!(i.score, 3);
        assert!(!i.has_next(), "no pass at 0 or less");
    }

    #[test]
    fn panels_pop_out_by_id() {
        assert_eq!(Panel::Condition.id(), PanelId::Condition);
        assert_eq!(Panel::Log.id(), PanelId::Play(Panel::Log));
        assert_eq!(clock(0), "00:00");
    }
}
