//! The dice rolls at the table: what a roll was (who, what for, the
//! dice), the result card both layouts show under the card's dice pools,
//! the Workspace's Dice rolls tray (`Panel::Rolls`, filtered by who
//! rolled), the last result on the member's encounter row, and the
//! Play screen's Table rolls (players).
//!
//! The GM's own rolls come from the card's pools and weapons (with Push
//! the Limit). In an online campaign the players' rolls arrive too (made
//! on their own Play screen, marked with the player's name), and the GM's
//! go to the campaign's roll log: private unless the campaign shows the
//! GM's rolls or the GM rolls one openly ([`GmScreen::next_open`]). The
//! last [`KEPT`] are saved with the campaign file.

use chummer_core::campaign::{LoggedRoll, MemberId, RollSettings};
use chummer_core::dice::{self, Glitch, RollRecord};
use chummer_core::lang::Language;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, StrokeKind};

use super::GmScreen;
use crate::theme;
use crate::view::CharacterView;
use crate::workspace::icons;
use crate::workspace::widgets;

/// Rolls kept (newest first), as many as the campaign file keeps.
pub(crate) const KEPT: usize = chummer_core::campaign::ROLLS_KEPT;

/// Rolls the tray lists below the newest one.
const LISTED: usize = 12;

/// One roll at the table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GmRoll {
    /// The online campaign's id for it ([`chummer_sync::msg::RollId`] as
    /// text); empty for a roll made offline.
    pub id: String,
    /// Whose roll it is (the member's or character's name).
    pub who: String,
    pub member: Option<MemberId>,
    /// A player's roll: the player ("Anna"). Made on their machine.
    pub player: Option<String>,
    /// A GM's roll shown to players.
    pub open: bool,
    /// Ours, not answered by the GM's app yet (Play screen).
    pub waiting: bool,
    /// The dice and how they were rolled (what for, the pool, Edge, the
    /// limit, when).
    pub rec: RollRecord,
    /// Worked out from `rec`.
    pub roll: dice::Roll,
}

impl GmRoll {
    pub fn new(who: &str, member: Option<MemberId>, rec: RollRecord) -> GmRoll {
        GmRoll { id: String::new(), who: who.to_owned(), member, player: None, open: false, waiting: false, roll: rec.outcome(), rec }
    }

    /// A roll of the campaign's roll log; `member` from its character.
    pub fn from_table(t: &chummer_sync::msg::TableRoll, player: Option<String>) -> GmRoll {
        let member = t.character.as_ref().and_then(chummer_sync::hosted::member_id);
        GmRoll { id: t.id.to_string(), player, open: t.open, ..GmRoll::new(&t.who, member, t.roll.clone()) }
    }

    pub fn from_logged(l: &LoggedRoll) -> GmRoll {
        let player = Some(l.player.clone()).filter(|p| !p.is_empty());
        GmRoll { id: l.id.clone(), player, open: l.open, ..GmRoll::new(&l.who, l.member, l.roll.clone()) }
    }

    pub fn to_logged(&self) -> LoggedRoll {
        LoggedRoll { id: self.id.clone(), who: self.who.clone(), member: self.member, player: self.player.clone().unwrap_or_default(), open: self.open, roll: self.rec.clone() }
    }

    pub fn at(&self) -> i64 {
        self.rec.at
    }

    /// Whose roll, with the player for a player's: "Raven (Anna)".
    pub fn who_text(&self) -> String {
        match &self.player {
            Some(p) if !p.is_empty() && *p != self.who => format!("{} ({p})", self.who),
            _ => self.who.clone(),
        }
    }

    /// What it was rolled for ("Roll" for a free roll).
    pub fn label(&self, lang: &Language) -> String {
        if self.rec.label.is_empty() {
            lang.tr("Roll")
        } else {
            self.rec.label.clone()
        }
    }

    /// The result in short: "3 hits", an initiative's score.
    fn result(&self, lang: &Language) -> String {
        match self.rec.score() {
            Some(s) => lang.tr_fmt("initiative {0}", &[&s]),
            None => lang.tr_fmt("{0} hits", &[&self.roll.hits]),
        }
    }

    /// The roll log line: "Who: Label 9d6 → 3 hits  [6 5 …]".
    pub fn line(&self, lang: &Language) -> String {
        let who = self.who_text();
        let mut l = match self.rec.score() {
            Some(s) => {
                let d: Vec<String> = self.rec.dice.iter().map(u8::to_string).collect();
                format!("{who}: {} {}d6 → {s}  [{}]", self.label(lang), self.rec.dice_count(), d.join(" "))
            }
            None => crate::campaign_ui::roll_line(lang, &who, &self.label(lang), self.rec.dice_count() as i32, &self.roll),
        };
        if let Some(lim) = self.rec.limit {
            l.push_str(&format!("  ({})", lang.tr_fmt("Limit {0}", &[&lim])));
        }
        match self.rec.edge {
            Some(e) => l.push_str(&format!("  ({})", lang.tr_fmt("Push the Limit +{0}", &[&e]))),
            None if self.rec.rule_of_six => l.push_str(&format!("  ({})", lang.tr("Rule of Six"))),
            None => {}
        }
        l
    }

    /// The glitch as words, `None` without one.
    fn glitch(&self, lang: &Language) -> Option<String> {
        if self.rec.initiative.is_some() {
            return None;
        }
        match self.roll.glitch {
            Glitch::None => None,
            Glitch::Glitch => Some(lang.tr("GLITCH")),
            Glitch::Critical => Some(lang.tr("CRITICAL GLITCH")),
        }
    }
}

/// The colour that marks a roll: critical glitch error, glitch warning,
/// else `ok`.
fn tone(r: &GmRoll, ws: &theme::WsPalette, ok: Color32) -> Color32 {
    match r.roll.glitch {
        _ if r.rec.initiative.is_some() => ok,
        Glitch::Critical => ws.error,
        Glitch::Glitch => ws.warning,
        Glitch::None => ok,
    }
}

/// Why the GM can trust a player's dice only so far (a tooltip and a
/// line under the tray).
pub(crate) const PLAYER_DICE: &str = "Players roll on their own machines: these are the dice their app sent. The hits are worked out from the dice.";

/// "12:03" of Unix ms.
/// "21:01" for a roll today, "10-08 21:01" for an older one.
fn clock(at: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    clock_at(at, now)
}

fn clock_at(at: i64, now: i64) -> String {
    let t = crate::history_ui::short_time(at);
    if t.get(..5) == crate::history_ui::short_time(now).get(..5) {
        t.get(t.len().saturating_sub(5)..).unwrap_or("").to_owned()
    } else {
        t
    }
}

impl GmScreen {
    /// Roll `pool` for `who` (member `member`): with Push the Limit when
    /// the GM chose it for that member (its Edge added, the Rule of Six;
    /// spends 1 Edge in Career Mode). Online, the roll goes to the
    /// campaign's roll log, shown to players when the campaign shows the
    /// GM's rolls or the GM chose to roll this one openly.
    pub(crate) fn roll_for(&mut self, member: Option<MemberId>, who: &str, label: &str, pool: i32, views: &mut [CharacterView]) {
        let push = member.is_some() && self.push == member;
        let edge = push.then(|| member.and_then(|m| self.live.get(&m)).map_or(0, |l| l.sheet.attr("EDG").max(0)));
        let (pool, edge) = (pool.max(0) as u32, edge.map(|e| e as u32));
        let r = dice::roll(&mut self.rng, pool + edge.unwrap_or(0), push, None);
        let rec = RollRecord { at: chummer_core::campaign::now_ms(), label: label.to_owned(), pool, edge, rule_of_six: push, dice: r.dice, ..Default::default() };
        let mut roll = GmRoll::new(who, member, rec);
        if let Some(o) = &self.online {
            roll.open = self.next_open();
            let t = o.hosted.host.gm_roll(member.map(chummer_sync::hosted::character_id), who, roll.open, roll.rec.clone());
            roll.id = t.id.to_string();
        }
        self.next_open = None;
        self.add_roll(roll);
        if push {
            self.push = None;
            self.spend_edge(member, views);
        }
    }

    /// Keeps a roll, newest arrival first: a play-by-post roll made
    /// yesterday comes in at the top (its card shows its date).
    fn add_roll(&mut self, r: GmRoll) {
        self.rolls.insert(0, r);
        self.rolls.truncate(KEPT);
    }

    /// Whether the GM's next roll is shown to players: the campaign's
    /// setting, unless the GM chose otherwise for this one.
    pub(crate) fn next_open(&self) -> bool {
        self.next_open.unwrap_or(self.campaign.roll_settings.show_gm_rolls)
    }

    /// Shows (or hides) the GM's next roll against the setting.
    pub(crate) fn set_next_open(&mut self, open: bool) {
        self.next_open = (open != self.campaign.roll_settings.show_gm_rolls).then_some(open);
    }

    /// Who sees which rolls (saved with the campaign; online, at once).
    pub(crate) fn set_roll_settings(&mut self, s: RollSettings) {
        if s == self.campaign.roll_settings {
            return;
        }
        self.campaign.roll_settings = s;
        self.next_open = None;
        self.dirty = true;
        if let Some(o) = &self.online {
            o.hosted.host.authority().set_roll_settings(s);
            o.hosted.host.changed();
        }
    }

    /// Online: the rolls the authority took since the last call (the
    /// players', and the GM's from before a restart), each kept once.
    pub(crate) fn take_table_rolls(&mut self) {
        let Some(o) = &self.online else { return };
        let Some(a) = o.hosted.host.try_authority() else { return };
        if a.roll_seq() == self.rolls_seq {
            return;
        }
        let from = self.rolls_seq;
        let new: Vec<GmRoll> = a
            .rolls()
            .iter()
            .filter(|t| t.seq > from)
            .map(|t| {
                // The GM's name for the player (the invite's label).
                let player = (t.author_role == chummer_net::invite::Role::Player).then(|| a.invite_of(&t.author).map(|i| i.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| t.author_name.clone()));
                GmRoll::from_table(t, player)
            })
            .collect();
        self.rolls_seq = a.roll_seq();
        drop(a);
        for mut r in new {
            if self.rolls.iter().any(|x| x.id == r.id) {
                continue;
            }
            // Its character's name as the roster has it now.
            if let Some(m) = r.member.and_then(|m| self.campaign.member(m)) {
                r.who = m.name.clone();
            }
            self.add_roll(r);
        }
    }

    /// The GM's rolls, newest first.
    #[cfg(test)]
    pub(crate) fn rolls(&self) -> &[GmRoll] {
        &self.rolls
    }

    /// The member's newest roll.
    pub(crate) fn last_roll(&self, m: MemberId) -> Option<&GmRoll> {
        self.rolls.iter().find(|r| r.member == Some(m))
    }

    /// Whether the GM's next roll is shown to players, and the
    /// campaign's setting: `None` offline (for [`open_check`]).
    pub(crate) fn open_state(&self) -> Option<(bool, bool)> {
        self.online.as_ref().map(|_| (self.next_open(), self.campaign.roll_settings.show_gm_rolls))
    }

    /// The campaign's roll settings as two checkboxes (online). Both
    /// layouts.
    pub(crate) fn roll_settings_ui(&mut self, ui: &mut egui::Ui, lang: &Language, ws_look: bool) {
        if self.online.is_none() {
            return;
        }
        let mut s = self.campaign.roll_settings;
        let check = |ui: &mut egui::Ui, on: &mut bool, label: &str, tip: &str| {
            let r = if ws_look { widgets::check(ui, on, label) } else { ui.checkbox(on, label) };
            r.on_hover_text(tip);
        };
        check(ui, &mut s.show_gm_rolls, &lang.tr("Show my rolls to players"), &lang.tr("Off: your rolls are yours alone, unless you roll one openly (the eye next to the dice pools)."));
        check(ui, &mut s.players_see_each_other, &lang.tr("Players see each other's rolls"), &lang.tr("Each player always sees their own rolls."));
        self.set_roll_settings(s);
    }

    /// Edge a member has left to push the limit with: (left, rating).
    pub(crate) fn edge_left(&self, m: MemberId, views: &[CharacterView]) -> (i32, i32) {
        let rating = self.live.get(&m).map_or(0, |l| l.sheet.attr("EDG").max(0));
        let used = super::doc_ref(&self.live, views, m).filter(|d| d.created).and_then(|d| d.doc.get_i32("edgeused")).unwrap_or(0);
        ((rating - used).max(0), rating)
    }

    /// The Workspace's Dice rolls tray: who to show, the newest roll as a
    /// card, then a list.
    pub(crate) fn ws_rolls(&mut self, ui: &mut egui::Ui, lang: &Language) {
        let ws = theme::ws(ui);
        ui.spacing_mut().item_spacing.y = 6.0;
        if self.online.is_some() {
            egui::CollapsingHeader::new(RichText::new(lang.tr("Who sees rolls")).size(11.5).color(ws.muted)).id_salt("ws_gm_roll_settings").show(ui, |ui| {
                self.roll_settings_ui(ui, lang, true);
            });
        }
        if self.rolls.is_empty() {
            ui.label(RichText::new(lang.tr("Click a pool or a weapon's Roll on a combatant's card: the dice show here.")).size(11.5).color(ws.muted));
            if self.online.is_some() {
                ui.label(RichText::new(lang.tr("Players' rolls show here too.")).size(11.5).color(ws.muted));
            }
            return;
        }
        let mut who: Vec<String> = Vec::new();
        for r in &self.rolls {
            let w = r.who_text();
            if !who.contains(&w) {
                who.push(w);
            }
        }
        if self.roll_filter.as_ref().is_some_and(|f| !who.contains(f)) {
            self.roll_filter = None;
        }
        let everyone = lang.tr("Everyone");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(lang.tr("Show")).size(11.5).color(ws.muted));
            let shown = self.roll_filter.clone().unwrap_or_else(|| everyone.clone());
            crate::combo::Combo::from_id_salt("ws_gm_roll_filter").selected_text(shown).width(ui.available_width().min(200.0)).show_ui(ui, |ui| {
                crate::combo::selectable_value(ui, &mut self.roll_filter, None, &everyone);
                for w in &who {
                    crate::combo::selectable_value(ui, &mut self.roll_filter, Some(w.clone()), w);
                }
            });
        });
        let filter = self.roll_filter.clone();
        let shown: Vec<&GmRoll> = self.rolls.iter().filter(|r| filter.as_ref().is_none_or(|f| *f == r.who_text())).collect();
        let Some((newest, rest)) = shown.split_first() else { return };
        roll_card(ui, lang, newest, true);
        ui.spacing_mut().item_spacing.y = 0.0;
        for r in rest.iter().take(LISTED) {
            roll_row(ui, lang, r);
        }
        if self.rolls.iter().any(|r| r.player.is_some()) {
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr(PLAYER_DICE)).size(11.0).color(ws.muted));
        }
    }
}

/// The rolls of a list: the newest as a card, then rows (the Play
/// screen's Table rolls).
pub(crate) fn rolls_list(ui: &mut egui::Ui, lang: &Language, rolls: &[GmRoll]) {
    let Some((newest, rest)) = rolls.split_first() else { return };
    roll_card(ui, lang, newest, true);
    ui.spacing_mut().item_spacing.y = 0.0;
    for r in rest.iter().take(LISTED) {
        roll_row(ui, lang, r);
    }
}

/// Next to the card's dice pools (online): "Roll openly", whether the
/// next roll is shown to players, from [`GmScreen::open_state`]. `ws_look`:
/// the Workspace's checkbox. Returns the new choice when it was changed
/// (for [`GmScreen::set_next_open`]).
pub(crate) fn open_check(ui: &mut egui::Ui, lang: &Language, ws_look: bool, (mut open, setting): (bool, bool)) -> Option<bool> {
    let label = format!("{} {}", icons::EYE, lang.tr("Roll openly"));
    let r = if ws_look { widgets::check(ui, &mut open, &label) } else { ui.checkbox(&mut open, label) };
    let tip = if setting { lang.tr("Your rolls are shown to players. Untick to keep the next one to yourself.") } else { lang.tr("Your rolls are yours alone. Tick to show the next one to players.") };
    r.on_hover_text(tip).changed().then_some(open)
}

/// A roll as a card: who and what, the dice (hits filled, ones outlined
/// red), the hits large, the glitch, and Push the Limit. `newest`: the
/// edge in the roll's colour.
pub(crate) fn roll_card(ui: &mut egui::Ui, lang: &Language, r: &GmRoll, newest: bool) {
    let ws = theme::ws(ui);
    let edge = if newest { tone(r, &ws, ws.primary) } else { tone(r, &ws, ws.divider) };
    egui::Frame::new().fill(ws.raised).stroke(Stroke::new(if newest { 1.5_f32 } else { 1.0 }, edge)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        // What it was rolled for, then whose roll (a player's marked).
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(icons::icon(icons::DICE_FIVE, 14.0, ws.accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(clock(r.at()), 11.0, ws.muted)).on_hover_text(crate::history_ui::short_time(r.at()));
                if r.waiting {
                    ui.label(icons::icon(icons::CLOCK, 12.0, ws.muted)).on_hover_text(lang.tr("Not at the GM's app yet: sent when it is reachable"));
                }
                if r.open {
                    ui.label(icons::icon(icons::EYE, 12.0, ws.muted)).on_hover_text(lang.tr("Shown to players"));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(r.label(lang)).font(widgets::bold(13.0)).color(ws.text)).truncate());
                });
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if let Some(p) = &r.player {
                ui.label(icons::icon(icons::USER, 12.0, ws.accent)).on_hover_text(lang.tr(PLAYER_DICE));
                let mut job = egui::text::LayoutJob::default();
                job.append(&r.who, 0.0, egui::TextFormat { font_id: widgets::bold(12.0), color: ws.text, ..Default::default() });
                if *p != r.who {
                    job.append(&format!("  {} {p}", lang.tr("rolled by")), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: ws.muted, ..Default::default() });
                }
                ui.add(egui::Label::new(job).truncate());
            } else {
                ui.add(egui::Label::new(RichText::new(&r.who).size(12.0).color(ws.muted)).truncate());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for d in &r.roll.dice {
                widgets::die_face(ui, *d);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let big = r.rec.score().unwrap_or(r.roll.hits as i32);
            ui.label(widgets::mono(big.to_string(), 26.0, tone(r, &ws, ws.accent)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let dice = lang.tr_fmt("{0} dice", &[&r.rec.dice_count()]);
                let what = if r.rec.initiative.is_some() { lang.tr("initiative") } else { lang.tr("hits") };
                let mut head = format!("{what} · {dice}");
                if let Some(l) = r.rec.limit {
                    head = format!("{head} · {}", lang.tr_fmt("Limit {0}", &[&l]));
                }
                ui.label(RichText::new(head).font(widgets::bold(12.5)).color(ws.text));
                let detail = match r.rec.initiative {
                    Some(b) => format!("{b} + {}d6", r.rec.dice.len()),
                    None => lang.tr_fmt("{0} ones", &[&r.roll.ones]),
                };
                ui.label(RichText::new(detail).size(11.0).color(ws.muted));
            });
            if let Some(g) = r.glitch(lang) {
                let color = tone(r, &ws, ws.muted);
                let galley = ui.painter().layout_no_wrap(g, widgets::bold(11.5), ws.on_badge);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 14.0, 20.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(10), color);
                ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ws.on_badge);
            }
        });
        let edge = match r.rec.edge {
            Some(e) => Some(format!("{} · {}", lang.tr_fmt("Push the Limit +{0}", &[&e]), lang.tr("Rule of Six"))),
            None => r.rec.rule_of_six.then(|| lang.tr("Rule of Six")),
        };
        if let Some(e) = edge {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icons::icon(icons::LIGHTNING, 12.0, ws.accent));
                ui.label(RichText::new(e).size(11.5).color(ws.accent));
            });
        }
    });
}

/// A roll as one line of the tray: time, who and what, small dice, hits.
fn roll_row(ui: &mut egui::Ui, lang: &Language, r: &GmRoll) {
    let ws = theme::ws(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0_f32, ws.divider));
        let y = |h: f32| rect.center().y - h / 2.0;
        let time = painter.layout_no_wrap(clock(r.at()), egui::FontId::monospace(10.5), ws.muted);
        let hits = painter.layout_no_wrap(r.result(lang), widgets::bold(12.0), tone(r, &ws, ws.text));
        let hx = rect.right() - hits.size().x;
        // The dice, small, left of the hits (as many as fit).
        let die = 9.0;
        let room = ((rect.width() - 200.0 - hits.size().x).max(0.0) / (die + 2.0)) as usize;
        let n = r.roll.dice.len().min(room.min(14));
        let dx = hx - 8.0 - n as f32 * (die + 2.0);
        for (k, d) in r.roll.dice.iter().take(n).enumerate() {
            let b = egui::Rect::from_min_size(egui::pos2(dx + k as f32 * (die + 2.0), y(die)), egui::vec2(die, die));
            match d {
                5 | 6 => painter.rect_filled(b, CornerRadius::same(2), ws.primary),
                1 => painter.rect_stroke(b, CornerRadius::same(2), Stroke::new(1.5_f32, ws.error), StrokeKind::Inside),
                _ => painter.rect_stroke(b, CornerRadius::same(2), Stroke::new(1.0_f32, ws.control), StrokeKind::Inside),
            };
        }
        painter.galley(egui::pos2(rect.left(), y(time.size().y)), time, ws.muted);
        let mut job = egui::text::LayoutJob::default();
        if r.player.is_some() {
            job.append(&format!("{} ", icons::USER), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(11.0), color: ws.accent, ..Default::default() });
        }
        job.append(&r.who_text(), 0.0, egui::TextFormat { font_id: widgets::bold(12.0), color: ws.text, ..Default::default() });
        job.append(&format!(" {} {}", r.label(lang), r.rec.dice_count()), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: ws.text, ..Default::default() });
        let what = painter.layout_job(job);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(dx - 6.0, rect.bottom()));
        painter.with_clip_rect(clip).galley(egui::pos2(rect.left() + 40.0, y(what.size().y)), what, ws.text);
        painter.galley(egui::pos2(hx, y(hits.size().y)), hits, ws.text);
        if r.glitch(lang).is_some() {
            icons::paint(painter, egui::Rect::from_center_size(egui::pos2(dx - 14.0, rect.center().y), egui::vec2(12.0, 12.0)), icons::WARNING, 12.0, tone(r, &ws, ws.muted));
        }
    }
    resp.on_hover_text(r.line(lang));
}

/// The last roll on an encounter row: "3" with a die, in the glitch's
/// colour when it glitched. Hover: the whole roll.
pub(crate) fn roll_chip(ui: &mut egui::Ui, lang: &Language, r: &GmRoll) {
    let ws = theme::ws(ui);
    let color = tone(r, &ws, ws.accent);
    let text = format!("{} {}", icons::DICE_FIVE, r.rec.score().unwrap_or(r.roll.hits as i32));
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::monospace(11.0), color);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 10.0, 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_stroke(rect, CornerRadius::same(9), Stroke::new(1.0_f32, color), StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp.on_hover_text(r.line(lang));
}

/// A roll in the Classic layout: the dice in colour (hits green, ones
/// red), the hits and the glitch.
pub(crate) fn classic_roll(ui: &mut egui::Ui, lang: &Language, r: &GmRoll) {
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(r.label(lang)).strong());
        ui.label(r.who_text());
        ui.weak(format!("{}d6", r.rec.dice_count()));
        if let Some(l) = r.rec.limit {
            ui.weak(lang.tr_fmt("Limit {0}", &[&l]));
        }
        if let Some(e) = r.rec.edge {
            ui.label(RichText::new(lang.tr_fmt("Push the Limit +{0}", &[&e])).color(p.accent));
        }
        if r.player.is_some() {
            ui.weak(crate::theme::glyph("👤")).on_hover_text(lang.tr(PLAYER_DICE));
        }
        if r.open {
            ui.weak(lang.tr("shown to players"));
        }
    });
    ui.horizontal_wrapped(|ui| {
        for d in &r.roll.dice {
            let color = match d {
                5 | 6 => p.good,
                1 => p.bad,
                _ => ui.visuals().weak_text_color(),
            };
            ui.label(RichText::new(d.to_string()).monospace().size(17.0).color(color));
        }
    });
    let text = match r.rec.score() {
        Some(s) => lang.tr_fmt("initiative {0}", &[&s]),
        None => crate::dice_ui::hits_text(lang, r.roll.hits, r.roll.glitch),
    };
    let color = match r.roll.glitch {
        _ if r.rec.initiative.is_some() => p.text,
        Glitch::None => p.text,
        Glitch::Glitch => p.warning,
        Glitch::Critical => p.bad,
    };
    ui.label(RichText::new(text).size(16.0).strong().color(color)).on_hover_text(r.line(lang));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_rolls_show_their_date() {
        // 2026-10-09 21:01 UTC and the next day.
        let at = 1_791_579_660_000;
        let day = 24 * 60 * 60 * 1000;
        assert_eq!(clock_at(at, at + 60_000), "21:01");
        assert_eq!(clock_at(at, at + day), "10-09 21:01");
    }

    #[test]
    fn roll_lines_name_the_edge() {
        let lang = Language::default();
        let rec = RollRecord { label: "Pistols".into(), pool: 4, edge: Some(2), rule_of_six: true, dice: vec![6, 5, 1, 2, 6, 3, 4], ..Default::default() };
        let r = GmRoll::new("Ganger 2", None, rec.clone());
        assert_eq!(r.line(&lang), "Ganger 2: Pistols 6d6 → 3 hits  [6 5 1 2 6 3 4]  (Push the Limit +2)");
        assert_eq!(r.glitch(&lang), None);
        let g = GmRoll::new("Ganger 2", None, RollRecord { edge: None, rule_of_six: false, pool: 3, dice: vec![1, 1, 2], ..rec });
        assert_eq!(g.glitch(&lang).as_deref(), Some("CRITICAL GLITCH"));
        // A player's roll names the player; an initiative roll its score.
        let p = GmRoll { player: Some("Anna".into()), ..GmRoll::new("Raven", None, RollRecord { label: "Initiative".into(), pool: 2, initiative: Some(9), dice: vec![6, 1], ..Default::default() }) };
        assert_eq!(p.line(&lang), "Raven (Anna): Initiative 2d6 → 16  [6 1]");
        assert_eq!(p.glitch(&lang), None, "initiative does not glitch");
        assert_eq!(GmRoll::from_logged(&p.to_logged()), p, "saved and loaded with the campaign");
    }
}
