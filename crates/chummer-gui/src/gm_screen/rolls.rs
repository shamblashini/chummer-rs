//! The GM's dice rolls: what a roll was (who, what, the dice), the
//! result card both layouts show under the card's dice pools, the
//! Workspace's Dice rolls tray (`Panel::Rolls`, filtered by who rolled)
//! and the last result on the member's encounter row.
//!
//! Rolls are the GM's own (the card's pools and weapons, with Push the
//! Limit). They are not saved and not sent to players; players roll on
//! their own Play screen, and those rolls do not reach the feed.

use chummer_core::campaign::MemberId;
use chummer_core::dice::{self, Glitch};
use chummer_core::lang::Language;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, StrokeKind};

use super::GmScreen;
use crate::theme;
use crate::view::CharacterView;
use crate::workspace::icons;
use crate::workspace::widgets;

/// Rolls kept (newest first).
const KEPT: usize = 50;

/// Rolls the tray lists below the newest one.
const LISTED: usize = 12;

/// One roll of the GM's.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GmRoll {
    /// Unix ms.
    pub at: i64,
    /// Who rolled (the member's name).
    pub who: String,
    pub member: Option<MemberId>,
    /// What was rolled ("Defense", a weapon).
    pub label: String,
    /// The pool, before Edge.
    pub pool: i32,
    /// Push the Limit: the Edge dice added (with the Rule of Six).
    pub edge: Option<i32>,
    pub roll: dice::Roll,
}

impl GmRoll {
    /// The roll log line: "Who: Label 9d6 → 3 hits  [6 5 …]".
    pub fn line(&self, lang: &Language) -> String {
        let mut l = crate::campaign_ui::roll_line(lang, &self.who, &self.label, self.pool + self.edge.unwrap_or(0), &self.roll);
        if let Some(e) = self.edge {
            l.push_str(&format!("  ({})", lang.tr_fmt("Push the Limit +{0}", &[&e])));
        }
        l
    }

    /// The glitch as words, `None` without one.
    fn glitch(&self, lang: &Language) -> Option<String> {
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
        Glitch::Critical => ws.error,
        Glitch::Glitch => ws.warning,
        Glitch::None => ok,
    }
}

/// "12:03" of Unix ms.
fn clock(at: i64) -> String {
    let t = crate::history_ui::short_time(at);
    t.get(t.len().saturating_sub(5)..).unwrap_or("").to_owned()
}

impl GmScreen {
    /// Roll `pool` for `who` (member `member`): with Push the Limit when
    /// the GM chose it for that member (its Edge added, the Rule of Six;
    /// spends 1 Edge in Career Mode).
    pub(crate) fn roll_for(&mut self, member: Option<MemberId>, who: &str, label: &str, pool: i32, views: &mut [CharacterView]) {
        let push = member.is_some() && self.push == member;
        let edge = push.then(|| member.and_then(|m| self.live.get(&m)).map_or(0, |l| l.sheet.attr("EDG").max(0)));
        let n = (pool + edge.unwrap_or(0)).max(0) as u32;
        let roll = dice::roll(&mut self.rng, n, push, None);
        self.rolls.insert(0, GmRoll { at: chummer_core::campaign::now_ms(), who: who.to_owned(), member, label: label.to_owned(), pool, edge, roll });
        self.rolls.truncate(KEPT);
        if push {
            self.push = None;
            self.spend_edge(member, views);
        }
    }

    /// The member's newest roll.
    pub(crate) fn last_roll(&self, m: MemberId) -> Option<&GmRoll> {
        self.rolls.iter().find(|r| r.member == Some(m))
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
        if self.rolls.is_empty() {
            ui.label(RichText::new(lang.tr("Click a pool or a weapon's Roll on a combatant's card: the dice show here.")).size(11.5).color(ws.muted));
            return;
        }
        let mut who: Vec<String> = Vec::new();
        for r in &self.rolls {
            if !who.contains(&r.who) {
                who.push(r.who.clone());
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
        let shown: Vec<&GmRoll> = self.rolls.iter().filter(|r| filter.as_ref().is_none_or(|f| *f == r.who)).collect();
        let Some((newest, rest)) = shown.split_first() else { return };
        roll_card(ui, lang, newest, true);
        ui.spacing_mut().item_spacing.y = 0.0;
        for r in rest.iter().take(LISTED) {
            roll_row(ui, lang, r);
        }
    }
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
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(icons::icon(icons::DICE_FIVE, 14.0, ws.accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::mono(clock(r.at), 11.0, ws.muted)).on_hover_text(crate::history_ui::short_time(r.at));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&r.who, 0.0, egui::TextFormat { font_id: widgets::bold(12.5), color: ws.text, ..Default::default() });
                    job.append(&format!("  {}", r.label), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: ws.muted, ..Default::default() });
                    ui.add(egui::Label::new(job).truncate());
                });
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for d in &r.roll.dice {
                widgets::die_face(ui, *d);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(widgets::mono(r.roll.hits.to_string(), 26.0, tone(r, &ws, ws.accent)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let dice = r.pool + r.edge.unwrap_or(0);
                ui.label(RichText::new(format!("{} · {}", lang.tr("hits"), lang.tr_fmt("{0} dice", &[&dice]))).font(widgets::bold(12.5)).color(ws.text));
                ui.label(RichText::new(lang.tr_fmt("{0} ones", &[&r.roll.ones])).size(11.0).color(ws.muted));
            });
            if let Some(g) = r.glitch(lang) {
                let color = tone(r, &ws, ws.muted);
                let galley = ui.painter().layout_no_wrap(g, widgets::bold(11.5), ws.on_badge);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 14.0, 20.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(10), color);
                ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ws.on_badge);
            }
        });
        if let Some(e) = r.edge {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(icons::icon(icons::LIGHTNING, 12.0, ws.accent));
                ui.label(RichText::new(format!("{} · {}", lang.tr_fmt("Push the Limit +{0}", &[&e]), lang.tr("Rule of Six"))).size(11.5).color(ws.accent));
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
        let time = painter.layout_no_wrap(clock(r.at), egui::FontId::monospace(10.5), ws.muted);
        let hits = painter.layout_no_wrap(lang.tr_fmt("{0} hits", &[&r.roll.hits]), widgets::bold(12.0), tone(r, &ws, ws.text));
        let hx = rect.right() - hits.size().x;
        // The dice, small, left of the hits (as many as fit).
        let die = 9.0;
        let room = ((rect.width() - 150.0 - hits.size().x).max(0.0) / (die + 2.0)) as usize;
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
        job.append(&r.who, 0.0, egui::TextFormat { font_id: widgets::bold(12.0), color: ws.text, ..Default::default() });
        job.append(&format!(" {} {}", r.label, r.pool + r.edge.unwrap_or(0)), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: ws.muted, ..Default::default() });
        let what = painter.layout_job(job);
        let clip = egui::Rect::from_min_max(rect.min, egui::pos2(dx - 6.0, rect.bottom()));
        painter.with_clip_rect(clip).galley(egui::pos2(rect.left() + 40.0, y(what.size().y)), what, ws.text);
        painter.galley(egui::pos2(hx, y(hits.size().y)), hits, ws.text);
        if r.roll.glitch != Glitch::None {
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
    let text = format!("{} {}", icons::DICE_FIVE, r.roll.hits);
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
        ui.label(RichText::new(&r.who).strong());
        ui.label(&r.label);
        ui.weak(format!("{}d6", r.pool + r.edge.unwrap_or(0)));
        if let Some(e) = r.edge {
            ui.label(RichText::new(lang.tr_fmt("Push the Limit +{0}", &[&e])).color(p.accent));
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
    let text = crate::dice_ui::hits_text(lang, r.roll.hits, r.roll.glitch);
    let color = match r.roll.glitch {
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
    fn roll_lines_name_the_edge() {
        let lang = Language::default();
        let r = GmRoll { at: 0, who: "Ganger 2".into(), member: None, label: "Pistols".into(), pool: 4, edge: Some(2), roll: dice::evaluate(vec![6, 5, 1, 2, 6, 3, 4], 5) };
        assert_eq!(r.line(&lang), "Ganger 2: Pistols 6d6 → 3 hits  [6 5 1 2 6 3 4]  (Push the Limit +2)");
        assert_eq!(r.glitch(&lang), None);
        let g = GmRoll { edge: None, roll: dice::evaluate(vec![1, 1, 2], 5), ..r };
        assert_eq!(g.glitch(&lang).as_deref(), Some("CRITICAL GLITCH"));
    }
}
