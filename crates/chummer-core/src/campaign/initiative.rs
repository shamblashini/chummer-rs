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
//! Dice are rolled here, never inside a command: a roll's result is state
//! of the encounter, and damage reaches characters as commands with fixed
//! values.

use serde::{Deserialize, Serialize};

use super::{CombatantId, MemberId};
use crate::dice::{self, Rng};

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

    fn roll(&mut self, rng: &mut Rng) {
        let n = if self.blitzed { MAX_DICE } else { self.dice.clamp(1, MAX_DICE) };
        let (score, rolled) = dice::initiative(rng, self.base, n);
        self.score = score;
        self.rolled = rolled;
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
            c.score = c.score.saturating_sub(10);
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

    /// Back to before the first roll.
    pub fn reset(&mut self) {
        self.round = 0;
        self.pass = 0;
        for c in &mut self.combatants {
            c.score = 0;
            c.rolled.clear();
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
}
