//! Encounters: initiative order, passes and the per-round Edge actions.
//!
//! SR5 initiative (Core p. 159): each combat round everyone rolls
//! initiative (base + Nd6, at most 5 dice). Characters act from the
//! highest score down; ties go to the higher Edge, then Reaction, then
//! Intuition. After a pass everyone's score drops by 10 and those still
//! above 0 act again. Seize the Initiative (spend Edge) acts first in
//! every pass of the round; Blitz (spend Edge) rolls 5d6. A character may
//! delay; a delayed character keeps its place until the GM marks it acted.
//!
//! Initiative adjustments ([`InitAdjust`]) are bonuses that are not part
//! of the sheet's initiative, or come later than the roll: Adrenaline
//! Boost, an activated Adrenaline Pump, a drug taken mid-fight, a
//! sustained Increase Reflexes, or anything the GM types in. Each adds to
//! the Initiative Score, adds Initiative Dice, or both, for this pass,
//! this combat turn, N turns or until removed. Changing initiative during
//! a turn (our reading of SR5 p. 159, whose text is not in Chummer's
//! data): a score change raises or lowers the current score at once;
//! dice gained mid-turn are rolled at once and added (never past
//! [`MAX_DICE`] in all); taking an adjustment away (removed, or its pass
//! ended) takes back exactly what it added, its own dice included. A dice
//! adjustment can instead wait for the next roll, and fewer dice (a
//! crash) always wait for it. At every roll the active adjustments add
//! their score and roll their dice again.
//!
//! Dice are rolled here, never inside a command: a roll's result is state
//! of the encounter, and damage reaches characters as commands with fixed
//! values.

use serde::{Deserialize, Serialize};

use super::{CombatantId, MemberId};
use crate::dice::Rng;

/// The most initiative dice anyone rolls (SR5 p. 159).
pub const MAX_DICE: u32 = 5;

/// What initiative needs from a combatant's sheet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InitStats {
    pub base: i32,
    pub dice: u32,
    pub edge: i32,
    pub reaction: i32,
    pub intuition: i32,
}

/// How long an initiative adjustment lasts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lasts {
    /// Until the end of this initiative pass.
    Pass,
    /// Until the next Roll initiative (the end of this combat turn).
    #[default]
    Turn,
    /// This combat turn and the next ones, `n` turns in all.
    Turns(u32),
    /// Until removed.
    Removed,
}

/// A bonus (or penalty) to a combatant's initiative that the sheet does
/// not count: see the module documentation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InitAdjust {
    /// What the board shows: "Adrenaline Boost 2".
    pub label: String,
    /// Where it comes from: a suggestion's key ("power:<guid>"), or empty
    /// for one typed in.
    pub source: String,
    /// Added to the Initiative Score.
    pub score: i32,
    /// Initiative Dice added (fewer when negative).
    pub dice: i32,
    /// Added dice are rolled at once (mid-turn), not only from the next
    /// roll.
    pub roll_now: bool,
    pub lasts: Lasts,
    /// The combat round it started in (the first roll's when added before
    /// it) and the pass; [`InitAdjust::begin`] fills them in.
    pub round: u32,
    pub pass: u32,
    /// Its own dice this round.
    pub rolled: Vec<u8>,
    /// What it adds to the score now: `score` and its dice.
    pub applied: i32,
}

impl InitAdjust {
    pub fn new(label: impl Into<String>, score: i32, dice: i32, lasts: Lasts) -> InitAdjust {
        InitAdjust { label: label.into(), score, dice, roll_now: true, lasts, ..Default::default() }
    }

    pub fn from_source(mut self, source: impl Into<String>) -> InitAdjust {
        self.source = source.into();
        self
    }

    /// The last combat round it lasts in; `None` until removed.
    pub fn last_round(&self) -> Option<u32> {
        match self.lasts {
            Lasts::Pass | Lasts::Turn => Some(self.round),
            Lasts::Turns(n) => Some(self.round.saturating_add(n.max(1) - 1)),
            Lasts::Removed => None,
        }
    }

    /// Whether it still applies in combat round `round`.
    pub fn lasts_into(&self, round: u32) -> bool {
        self.last_round().is_none_or(|r| r >= round)
    }

    /// "+4", "+1d6", "+2 +2d6", "−1d6 (next roll)".
    pub fn effect(&self) -> String {
        let mut parts = Vec::new();
        if self.score != 0 {
            parts.push(format!("{:+}", self.score));
        }
        if self.dice != 0 {
            parts.push(format!("{:+}d6", self.dice));
        }
        if parts.is_empty() {
            parts.push("+0".into());
        }
        parts.join(" ").replace('-', "−")
    }
}

/// Damage tracks for a combatant without a character sheet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdHocTrack {
    pub physical: i32,
    pub stun: i32,
    pub physical_filled: i32,
    pub stun_filled: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Combatant {
    pub id: CombatantId,
    /// The campaign member, or `None` for an ad-hoc combatant.
    pub member: Option<MemberId>,
    pub name: String,
    /// Stats used for the last roll (members: from the sheet at that time;
    /// ad-hoc: entered by the GM).
    pub base: i32,
    pub dice: u32,
    pub edge: i32,
    pub reaction: i32,
    pub intuition: i32,
    /// The current initiative score (after passes and actions).
    pub score: i32,
    /// The dice of the last roll.
    pub rolled: Vec<u8>,
    pub acted: bool,
    pub delayed: bool,
    /// Seize the Initiative this round.
    pub seized: bool,
    /// Blitz this round (rolled 5d6).
    pub blitzed: bool,
    /// Ad-hoc combatants' condition monitors.
    pub track: Option<AdHocTrack>,
    pub notes: String,
    /// Initiative adjustments in effect, in the order added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub adjust: Vec<InitAdjust>,
}

impl Combatant {
    pub fn for_member(member: MemberId, name: impl Into<String>) -> Combatant {
        Combatant { id: CombatantId::random(), member: Some(member), name: name.into(), ..Default::default() }
    }

    /// An ad-hoc combatant ("Security guard") with its own tracks.
    pub fn ad_hoc(name: impl Into<String>, base: i32, dice: u32, physical: i32, stun: i32) -> Combatant {
        Combatant { id: CombatantId::random(), member: None, name: name.into(), base, dice, track: Some(AdHocTrack { physical, stun, ..Default::default() }), ..Default::default() }
    }

    fn take(&mut self, s: InitStats) {
        self.base = s.base;
        self.dice = s.dice;
        self.edge = s.edge;
        self.reaction = s.reaction;
        self.intuition = s.intuition;
    }

    fn stats(&self) -> InitStats {
        InitStats { base: self.base, dice: self.dice, edge: self.edge, reaction: self.reaction, intuition: self.intuition }
    }

    /// The dice the next roll takes: its own (5 for a Blitz; fewer for
    /// adjustments that take dice away, at least 1), then each
    /// adjustment's added dice, never more than [`MAX_DICE`] in all.
    pub fn dice_plan(&self) -> (u32, Vec<u32>) {
        let fewer: i64 = self.adjust.iter().map(|a| i64::from(a.dice.min(0))).sum();
        let own = if self.blitzed { MAX_DICE } else { (i64::from(self.dice.clamp(1, MAX_DICE)) + fewer).clamp(1, i64::from(MAX_DICE)) as u32 };
        let mut room = MAX_DICE - own;
        let extra = self
            .adjust
            .iter()
            .map(|a| {
                let n = (a.dice.max(0) as u32).min(room);
                room -= n;
                n
            })
            .collect();
        (own, extra)
    }

    /// Roll initiative: its own dice and the adjustments' ([`Combatant::dice_plan`]).
    pub fn roll(&mut self, rng: &mut Rng) {
        let (own, extra) = self.dice_plan();
        let n = own + extra.iter().sum::<u32>();
        let dice: Vec<u8> = (0..n).map(|_| rng.d6()).collect();
        self.take_roll(dice);
    }

    /// Take a roll made elsewhere (the Play screen's roller): the dice in
    /// [`Combatant::dice_plan`]'s order, own dice first. Sets the score:
    /// base, own dice and every active adjustment.
    pub fn take_roll(&mut self, mut dice: Vec<u8>) {
        let (own, extra) = self.dice_plan();
        let rest = dice.split_off((own as usize).min(dice.len()));
        let mut total = self.base.saturating_add(dice.iter().map(|&d| i32::from(d)).sum::<i32>());
        self.rolled = dice;
        let mut rest = rest.into_iter();
        for (a, n) in self.adjust.iter_mut().zip(extra) {
            a.rolled = rest.by_ref().take(n as usize).collect();
            a.applied = a.score.saturating_add(a.rolled.iter().map(|&d| i32::from(d)).sum::<i32>());
            total = total.saturating_add(a.applied);
        }
        self.score = total;
    }

    /// Add an adjustment in combat round `round`, pass `pass` (0 before
    /// the first roll): during a round its score counts at once and its
    /// added dice (with `roll_now`) are rolled at once; before the first
    /// roll it waits for it.
    pub fn add_adjust(&mut self, mut a: InitAdjust, round: u32, pass: u32, rng: &mut Rng) {
        a.round = round.max(1);
        a.pass = if round == 0 { 1 } else { pass.max(1) };
        a.rolled.clear();
        a.applied = 0;
        if round > 0 {
            if a.roll_now && a.dice > 0 {
                let used = self.rolled.len() + self.adjust.iter().map(|x| x.rolled.len()).sum::<usize>();
                let n = (a.dice as usize).min((MAX_DICE as usize).saturating_sub(used));
                a.rolled = (0..n).map(|_| rng.d6()).collect();
            }
            a.applied = a.score.saturating_add(a.rolled.iter().map(|&d| i32::from(d)).sum::<i32>());
            self.score = self.score.saturating_add(a.applied);
        }
        self.adjust.push(a);
    }

    /// Take adjustment `k` away, and what it added to the score.
    pub fn remove_adjust(&mut self, k: usize) -> Option<InitAdjust> {
        if k >= self.adjust.len() {
            return None;
        }
        let a = self.adjust.remove(k);
        self.score = self.score.saturating_sub(a.applied);
        Some(a)
    }

    /// Whether an adjustment from `source` is in effect.
    pub fn has_adjust(&self, source: &str) -> bool {
        !source.is_empty() && self.adjust.iter().any(|a| a.source == source)
    }

    /// The start of combat round `round`, before its roll: adjustments
    /// that have run out go (the roll gives a fresh score).
    pub fn start_round(&mut self, round: u32) {
        self.adjust.retain(|a| a.lasts_into(round));
        for a in &mut self.adjust {
            a.rolled.clear();
            a.applied = 0;
        }
    }

    /// The next pass: adjustments for one pass end (taking back what
    /// they added), then the score loses 10.
    pub fn next_pass(&mut self) {
        let mut k = 0;
        while k < self.adjust.len() {
            if self.adjust[k].lasts == Lasts::Pass {
                self.remove_adjust(k);
            } else {
                k += 1;
            }
        }
        self.score = self.score.saturating_sub(10);
    }

    /// Back to before the first roll: only adjustments until removed stay.
    pub fn reset(&mut self) {
        self.score = 0;
        self.rolled.clear();
        self.adjust.retain(|a| a.lasts == Lasts::Removed);
        for a in &mut self.adjust {
            a.rolled.clear();
            a.applied = 0;
            a.round = 1;
            a.pass = 1;
        }
    }

    /// Still in this pass: a score above 0.
    pub fn in_pass(&self) -> bool {
        self.score > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Encounter {
    pub id: CombatantId,
    pub name: String,
    pub combatants: Vec<Combatant>,
    /// Combat round, 0 before the first roll.
    pub round: u32,
    /// Initiative pass within the round, 1-based (0 before the first roll).
    pub pass: u32,
    pub notes: String,
}

impl Encounter {
    pub fn new(name: impl Into<String>) -> Encounter {
        Encounter { id: CombatantId::random(), name: name.into(), ..Default::default() }
    }

    pub fn index(&self, id: CombatantId) -> Option<usize> {
        self.combatants.iter().position(|c| c.id == id)
    }

    /// Whether a member is in the encounter.
    pub fn has_member(&self, m: MemberId) -> bool {
        self.combatants.iter().any(|c| c.member == Some(m))
    }

    /// Start the next combat round: clear the round's flags, take fresh
    /// stats (`stats` gives a member's from its sheet; `None` keeps the
    /// combatant's own) and roll for everyone.
    pub fn new_round(&mut self, rng: &mut Rng, stats: impl Fn(&Combatant) -> Option<InitStats>) {
        // Saturating: the counters and scores come from the campaign file.
        self.round = self.round.saturating_add(1);
        self.pass = 1;
        for c in &mut self.combatants {
            c.acted = false;
            c.delayed = false;
            c.seized = false;
            c.blitzed = false;
            if let Some(s) = stats(c) {
                c.take(s);
            }
            c.start_round(self.round);
            c.roll(rng);
        }
    }

    /// Add a combatant (with `stats` from its sheet, when it has one).
    /// Before the first round it waits for Roll initiative; during a
    /// round it rolls at once, as one joining late does. Returns its id.
    pub fn join(&mut self, mut c: Combatant, rng: &mut Rng, stats: Option<InitStats>) -> CombatantId {
        if let Some(s) = stats {
            c.take(s);
        }
        let id = c.id;
        self.combatants.push(c);
        if self.round > 0 {
            let i = self.combatants.len() - 1;
            self.reroll(i, rng, stats);
        }
        id
    }

    /// Give a member's combatants its new name (after a rename).
    pub fn rename_member(&mut self, m: MemberId, name: &str) {
        for c in self.combatants.iter_mut().filter(|c| c.member == Some(m) && c.name != name) {
            c.name = name.to_owned();
        }
    }

    /// Roll again for one combatant (joined late, or the GM rerolls).
    pub fn reroll(&mut self, i: usize, rng: &mut Rng, stats: Option<InitStats>) {
        let Some(c) = self.combatants.get_mut(i) else { return };
        if let Some(s) = stats {
            c.take(s);
        }
        c.roll(rng);
        // Later passes have already taken their 10s.
        let taken = i32::try_from(self.pass.saturating_sub(1)).unwrap_or(i32::MAX).saturating_mul(10);
        c.score = c.score.saturating_sub(taken);
    }

    /// Blitz: roll 5d6 for this round (the caller spends the Edge).
    pub fn blitz(&mut self, i: usize, rng: &mut Rng) {
        let Some(c) = self.combatants.get_mut(i) else { return };
        c.blitzed = true;
        let stats = c.stats();
        self.reroll(i, rng, Some(stats));
    }

    /// Seize the Initiative: first in every pass of this round.
    pub fn seize(&mut self, i: usize) {
        if let Some(c) = self.combatants.get_mut(i) {
            c.seized = true;
        }
    }

    /// The order to act in: combatants still in the pass first (seized,
    /// then by score, Edge, Reaction, Intuition, then as added), then the
    /// rest by score.
    pub fn order(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.combatants.len()).collect();
        idx.sort_by(|&a, &b| {
            let (x, y) = (&self.combatants[a], &self.combatants[b]);
            y.in_pass()
                .cmp(&x.in_pass())
                .then((y.seized && y.in_pass()).cmp(&(x.seized && x.in_pass())))
                .then(y.score.cmp(&x.score))
                .then(y.edge.cmp(&x.edge))
                .then(y.reaction.cmp(&x.reaction))
                .then(y.intuition.cmp(&x.intuition))
                .then(a.cmp(&b))
        });
        idx
    }

    /// Who acts now: the first in [`Encounter::order`] still in the pass
    /// that has neither acted nor delayed.
    pub fn current(&self) -> Option<usize> {
        (self.pass > 0).then(|| self.order().into_iter().find(|&i| {
            let c = &self.combatants[i];
            c.in_pass() && !c.acted && !c.delayed
        }))?
    }

    /// Mark the current combatant as acted; returns the next one.
    pub fn advance(&mut self) -> Option<usize> {
        if let Some(i) = self.current() {
            self.combatants[i].acted = true;
        }
        self.current()
    }

    /// Whether anyone would still be in the next pass.
    pub fn has_next_pass(&self) -> bool {
        self.combatants.iter().any(|c| c.score > 10)
    }

    /// Next initiative pass: everyone loses 10 and may act again. Returns
    /// false (and changes nothing) when nobody would be left; the round is
    /// over then.
    pub fn next_pass(&mut self) -> bool {
        if !self.has_next_pass() {
            return false;
        }
        self.pass = self.pass.saturating_add(1);
        for c in &mut self.combatants {
            c.next_pass();
            c.acted = false;
            c.delayed = false;
        }
        true
    }

    /// An action that costs initiative (e.g. an interrupt: −5 or −10).
    pub fn spend(&mut self, i: usize, points: i32) {
        if let Some(c) = self.combatants.get_mut(i) {
            c.score = c.score.saturating_sub(points);
        }
    }

    /// Add an initiative adjustment to combatant `i` (see
    /// [`Combatant::add_adjust`]).
    pub fn adjust(&mut self, i: usize, a: InitAdjust, rng: &mut Rng) {
        let (round, pass) = (self.round, self.pass);
        if let Some(c) = self.combatants.get_mut(i) {
            c.add_adjust(a, round, pass, rng);
        }
    }

    /// Take combatant `i`'s adjustment `k` away.
    pub fn unadjust(&mut self, i: usize, k: usize) -> Option<InitAdjust> {
        self.combatants.get_mut(i)?.remove_adjust(k)
    }

    /// Back to before the first roll.
    pub fn reset(&mut self) {
        self.round = 0;
        self.pass = 0;
        for c in &mut self.combatants {
            c.reset();
            c.acted = false;
            c.delayed = false;
            c.seized = false;
            c.blitzed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(scores: &[(&str, i32, i32)]) -> Encounter {
        let mut e = Encounter::new("t");
        for (n, s, edge) in scores {
            let mut c = Combatant::ad_hoc(*n, 0, 1, 10, 10);
            c.score = *s;
            c.edge = *edge;
            e.combatants.push(c);
        }
        e.round = 1;
        e.pass = 1;
        e
    }

    fn names(e: &Encounter) -> Vec<&str> {
        e.order().into_iter().map(|i| e.combatants[i].name.as_str()).collect()
    }

    #[test]
    fn order_by_score_then_edge() {
        let e = enc(&[("a", 12, 1), ("b", 20, 1), ("c", 12, 4), ("d", -3, 9)]);
        assert_eq!(names(&e), ["b", "c", "a", "d"]);
    }

    #[test]
    fn seize_goes_first_while_in_pass() {
        let mut e = enc(&[("a", 25, 1), ("b", 8, 1)]);
        e.seize(1);
        assert_eq!(names(&e), ["b", "a"]);
        assert!(e.next_pass());
        // b is out at -2; a acts alone in pass 2.
        assert_eq!(names(&e), ["a", "b"]);
    }

    #[test]
    fn passes_take_ten_and_end() {
        let mut e = enc(&[("a", 23, 1), ("b", 12, 1), ("c", 9, 1)]);
        assert_eq!(e.current(), Some(0));
        assert_eq!(e.advance(), Some(1));
        e.combatants[1].delayed = true;
        assert_eq!(e.current(), Some(2), "a delayed combatant is skipped");
        assert!(e.next_pass());
        assert_eq!(e.pass, 2);
        assert_eq!(e.combatants.iter().map(|c| c.score).collect::<Vec<_>>(), [13, 2, -1]);
        assert!(!e.combatants[0].acted && !e.combatants[1].delayed);
        assert_eq!(names(&e), ["a", "b", "c"]);
        assert!(e.next_pass());
        assert_eq!(e.current(), Some(0));
        assert!(!e.next_pass(), "nobody above 10: the round is over");
        assert_eq!(e.pass, 3);
    }

    #[test]
    fn rounds_roll_and_blitz() {
        let mut e = Encounter::new("t");
        e.combatants.push(Combatant::ad_hoc("guard", 8, 1, 10, 10));
        let m = MemberId::random();
        e.combatants.push(Combatant::for_member(m, "Ghost"));
        let mut rng = Rng::seeded(5);
        e.new_round(&mut rng, |c| (c.member == Some(m)).then_some(InitStats { base: 10, dice: 9, edge: 3, reaction: 5, intuition: 5 }));
        assert_eq!(e.round, 1);
        let g = &e.combatants[1];
        assert_eq!(g.rolled.len(), MAX_DICE as usize, "dice capped at 5");
        assert_eq!(g.score, 10 + g.rolled.iter().map(|&d| i32::from(d)).sum::<i32>());
        let guard = &e.combatants[0];
        assert_eq!((guard.base, guard.rolled.len()), (8, 1));
        e.blitz(0, &mut rng);
        assert_eq!(e.combatants[0].rolled.len(), 5);
        assert!(e.combatants[0].blitzed);
        e.new_round(&mut rng, |_| None);
        assert_eq!((e.round, e.pass), (2, 1));
        assert!(!e.combatants[0].blitzed);
        assert_eq!(e.combatants[0].rolled.len(), 1);
    }

    #[test]
    fn joining_rolls_only_during_a_round() {
        let mut e = Encounter::new("t");
        let mut rng = Rng::seeded(9);
        let stats = InitStats { base: 9, dice: 2, edge: 2, reaction: 4, intuition: 5 };
        let (a, b) = (MemberId::random(), MemberId::random());
        let first = e.join(Combatant::for_member(a, "Ganger 1"), &mut rng, Some(stats));
        let c = &e.combatants[0];
        assert_eq!((c.id, c.base, c.dice, c.edge), (first, 9, 2, 2), "the sheet's stats");
        assert!(c.rolled.is_empty() && c.score == 0, "waits for Roll initiative");
        e.new_round(&mut rng, |_| None);
        e.combatants[0].score = 30;
        assert!(e.next_pass());
        // Joining in pass 2: rolls now, and has already lost the pass's 10.
        e.join(Combatant::for_member(b, "Ganger 2"), &mut rng, Some(stats));
        let late = &e.combatants[1];
        assert_eq!(late.rolled.len(), 2);
        assert_eq!(late.score, 9 + late.rolled.iter().map(|&d| i32::from(d)).sum::<i32>() - 10);
        assert!(e.has_member(b));
        // A rename reaches the member's combatants only.
        e.rename_member(a, "Boss");
        assert_eq!(names(&e).len(), 2);
        assert_eq!((e.combatants[0].name.as_str(), e.combatants[1].name.as_str()), ("Boss", "Ganger 2"));
    }

    fn sum(d: &[u8]) -> i32 {
        d.iter().map(|&x| i32::from(x)).sum()
    }

    #[test]
    fn adjustments_change_score_and_order() {
        let mut e = enc(&[("a", 14, 1), ("b", 12, 1)]);
        let mut rng = Rng::seeded(3);
        assert_eq!(names(&e), ["a", "b"]);
        // Mid-turn: +4 counts at once and b goes first.
        e.adjust(1, InitAdjust::new("Adrenaline Boost 2", 4, 0, Lasts::Turn), &mut rng);
        assert_eq!(e.combatants[1].score, 16);
        assert_eq!(names(&e), ["b", "a"]);
        // Dice gained mid-turn are rolled now and added.
        e.adjust(0, InitAdjust::new("Jazz", 1, 2, Lasts::Removed), &mut rng);
        let a = &e.combatants[0];
        assert_eq!(a.adjust[0].rolled.len(), 2);
        assert_eq!(a.score, 14 + 1 + sum(&a.adjust[0].rolled));
        assert_eq!(a.adjust[0].applied, a.score - 14);
        // Removing takes back exactly what it added.
        let gone = e.unadjust(0, 0).unwrap();
        assert_eq!(gone.label, "Jazz");
        assert_eq!(e.combatants[0].score, 14);
        assert!(e.unadjust(0, 0).is_none());
        // Dice for the next roll only change nothing now.
        let mut later = InitAdjust::new("Kamikaze", 0, 2, Lasts::Removed);
        later.roll_now = false;
        e.adjust(0, later, &mut rng);
        assert_eq!(e.combatants[0].score, 14);
        assert!(!e.combatants[0].has_adjust(""), "typed-in ones have no source");
        e.adjust(1, InitAdjust::new("Jazz", 1, 2, Lasts::Removed).from_source("gear:x"), &mut rng);
        assert!(e.combatants[1].has_adjust("gear:x"));
    }

    #[test]
    fn adjustments_expire_by_pass_turn_and_turns() {
        let mut e = Encounter::new("t");
        e.combatants.push(Combatant::ad_hoc("a", 10, 1, 10, 10));
        let mut rng = Rng::seeded(11);
        // Before the first roll: they wait for it, and the roll counts them.
        e.adjust(0, InitAdjust::new("turn", 3, 0, Lasts::Turn), &mut rng);
        e.adjust(0, InitAdjust::new("two turns", 5, 1, Lasts::Turns(2)), &mut rng);
        e.adjust(0, InitAdjust::new("pass", 7, 0, Lasts::Pass), &mut rng);
        e.adjust(0, InitAdjust::new("kept", 2, 0, Lasts::Removed), &mut rng);
        assert_eq!(e.combatants[0].score, 0);
        e.new_round(&mut rng, |_| None);
        let c = &e.combatants[0];
        assert_eq!(c.rolled.len(), 1);
        assert_eq!(c.adjust[1].rolled.len(), 1, "an adjustment's dice are its own");
        assert_eq!(c.score, 10 + sum(&c.rolled) + 3 + 5 + sum(&c.adjust[1].rolled) + 7 + 2);
        // The pass ends: its +7 goes with it, then everyone loses 10.
        let before = c.score;
        assert!(e.next_pass());
        let c = &e.combatants[0];
        assert_eq!(c.score, before - 7 - 10);
        assert_eq!(c.adjust.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["turn", "two turns", "kept"]);
        // Round 2: "turn" is over; "two turns" lasts it out.
        e.new_round(&mut rng, |_| None);
        let c = &e.combatants[0];
        assert_eq!(c.adjust.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["two turns", "kept"]);
        assert_eq!(c.score, 10 + sum(&c.rolled) + 5 + sum(&c.adjust[0].rolled) + 2);
        // Round 3: only the one until removed.
        e.new_round(&mut rng, |_| None);
        assert_eq!(e.combatants[0].adjust.len(), 1);
        assert_eq!(e.combatants[0].score, 10 + sum(&e.combatants[0].rolled) + 2);
        // Added in round 3 for this turn: gone at round 4.
        e.adjust(0, InitAdjust::new("boost", 4, 0, Lasts::Turn), &mut rng);
        e.new_round(&mut rng, |_| None);
        assert_eq!(e.combatants[0].adjust.len(), 1);
        // Reset keeps only those until removed.
        e.adjust(0, InitAdjust::new("boost", 4, 0, Lasts::Turns(5)), &mut rng);
        e.reset();
        assert_eq!(e.combatants[0].adjust.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["kept"]);
        assert_eq!(e.combatants[0].score, 0);
    }

    #[test]
    fn adjustment_dice_stay_within_five() {
        let mut e = Encounter::new("t");
        e.combatants.push(Combatant::ad_hoc("a", 10, 4, 10, 10));
        let mut rng = Rng::seeded(2);
        e.adjust(0, InitAdjust::new("Jazz", 1, 2, Lasts::Removed), &mut rng);
        e.adjust(0, InitAdjust::new("crash", 0, -1, Lasts::Removed), &mut rng);
        assert_eq!(e.combatants[0].dice_plan(), (3, vec![2, 0]), "4 − 1 own dice, +2");
        e.new_round(&mut rng, |_| None);
        let c = &e.combatants[0];
        assert_eq!(c.rolled.len() + c.adjust.iter().map(|a| a.rolled.len()).sum::<usize>(), 5);
        // Mid-turn dice beyond five are not rolled.
        e.adjust(0, InitAdjust::new("more", 0, 3, Lasts::Turn), &mut rng);
        assert!(e.combatants[0].adjust[2].rolled.is_empty());
        // Blitz: five dice of its own, nothing for the adjustments.
        e.blitz(0, &mut rng);
        let c = &e.combatants[0];
        assert_eq!(c.rolled.len(), 5);
        assert!(c.adjust.iter().all(|a| a.rolled.is_empty()));
        // An old file without adjustments loads; one with them round-trips.
        let json = serde_json::to_string(&e).unwrap();
        let back: Encounter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        let plain = serde_json::to_string(&Combatant::ad_hoc("b", 1, 1, 1, 1)).unwrap();
        assert!(!plain.contains("adjust"));
    }

    #[test]
    fn effects_read_short() {
        assert_eq!(InitAdjust::new("x", 4, 0, Lasts::Turn).effect(), "+4");
        assert_eq!(InitAdjust::new("x", 1, 2, Lasts::Turn).effect(), "+1 +2d6");
        assert_eq!(InitAdjust::new("x", 0, -1, Lasts::Turn).effect(), "−1d6");
    }
}
