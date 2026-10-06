//! Quick damage at the table: "8P, AP −2" against a character.
//!
//! SR5 damage resistance (Core p. 173): the soak pool is Body + armor
//! modified by the attack's AP (armor never below 0). Physical damage
//! whose modified DV is less than the modified armor becomes Stun. Each
//! soak hit takes one box off. Damage then fills the condition monitors:
//! Physical past its track runs into the overflow boxes; Stun past its
//! track carries over to Physical (see [`STUN_PER_PHYSICAL`]).
//!
//! The result is box counts; the GUI applies them with
//! `SetPhysicalDamage` / `SetStunDamage` commands.

/// Excess Stun boxes that make one Physical box once the Stun track is
/// full. SR5's carry-over rate; a table that plays 1:1 changes it here.
pub const STUN_PER_PHYSICAL: i32 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attack {
    /// Damage value after net hits.
    pub dv: i32,
    /// Physical (P) or Stun (S).
    pub physical: bool,
    /// Armor penetration, usually 0 or negative.
    pub ap: i32,
}

impl Attack {
    /// Parse "8P", "6S AP-2", "10P -4", "7 P, AP +1".
    pub fn parse(s: &str) -> Option<Attack> {
        let s = s.to_ascii_uppercase().replace(',', " ");
        let digits: String = s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        let dv: i32 = digits.parse().ok()?;
        let rest = s.trim_start()[digits.len()..].trim_start();
        let (physical, rest) = match rest.chars().next() {
            Some('P') => (true, &rest[1..]),
            Some('S') => (false, &rest[1..]),
            _ => (true, rest),
        };
        let rest = rest.trim().trim_start_matches("AP").trim().replace(' ', "");
        let ap = if rest.is_empty() { 0 } else { rest.replace('−', "-").parse().ok()? };
        Some(Attack { dv, physical, ap })
    }
}

/// The defender's side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Defender {
    pub body: i32,
    pub armor: i32,
    /// Extra soak dice (damage resistance improvements).
    pub bonus: i32,
}

/// What an attack does before soaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Incoming {
    pub dv: i32,
    pub physical: bool,
    /// Physical damage that became Stun because the DV was below the
    /// modified armor.
    pub converted: bool,
    pub soak_pool: i32,
}

pub fn modified_armor(armor: i32, ap: i32) -> i32 {
    (armor + ap).max(0)
}

pub fn incoming(a: Attack, d: Defender) -> Incoming {
    let armor = modified_armor(d.armor, a.ap);
    let converted = a.physical && a.dv < armor;
    Incoming { dv: a.dv.max(0), physical: a.physical && !converted, converted, soak_pool: (d.body + armor + d.bonus).max(0) }
}

/// Boxes after `soak_hits` (or none, when the GM skips the roll).
pub fn after_soak(i: Incoming, soak_hits: u32) -> i32 {
    (i.dv - soak_hits as i32).max(0)
}

/// Condition monitors as they are, and their sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tracks {
    pub physical: i32,
    pub stun: i32,
    pub overflow: i32,
    pub physical_filled: i32,
    pub stun_filled: i32,
}

/// The filled boxes after `boxes` of damage. Physical is capped at its
/// track plus the overflow boxes (beyond that the character is dead).
pub fn apply(t: Tracks, boxes: i32, physical: bool) -> (i32, i32) {
    let cap = t.physical + t.overflow;
    let mut p = t.physical_filled;
    let mut s = t.stun_filled;
    if physical || t.stun <= 0 {
        p += boxes;
    } else {
        s += boxes;
        if s > t.stun {
            p += (s - t.stun) / STUN_PER_PHYSICAL;
            s = t.stun;
        }
    }
    (p.min(cap.max(0)), s)
}

/// SR5's wound modifier for filled boxes: −1 per `threshold` boxes of
/// each track.
pub fn wound_modifier(physical_filled: i32, stun_filled: i32, physical: i32, threshold: i32) -> i32 {
    if threshold <= 0 {
        return 0;
    }
    -(physical_filled.min(physical).max(0) / threshold + stun_filled.max(0) / threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_damage_codes() {
        assert_eq!(Attack::parse("8P AP-2"), Some(Attack { dv: 8, physical: true, ap: -2 }));
        assert_eq!(Attack::parse("6s"), Some(Attack { dv: 6, physical: false, ap: 0 }));
        assert_eq!(Attack::parse("10P, -4"), Some(Attack { dv: 10, physical: true, ap: -4 }));
        assert_eq!(Attack::parse("7 P AP +1"), Some(Attack { dv: 7, physical: true, ap: 1 }));
        assert_eq!(Attack::parse("5"), Some(Attack { dv: 5, physical: true, ap: 0 }));
        assert_eq!(Attack::parse("P"), None);
    }

    #[test]
    fn armor_and_conversion() {
        let d = Defender { body: 4, armor: 12, bonus: 0 };
        let i = incoming(Attack { dv: 8, physical: true, ap: -2 }, d);
        assert_eq!((i.physical, i.converted, i.soak_pool), (false, true, 14), "8 < 10: stun");
        let i = incoming(Attack { dv: 8, physical: true, ap: -6 }, d);
        assert_eq!((i.physical, i.soak_pool), (true, 10));
        let i = incoming(Attack { dv: 8, physical: true, ap: -20 }, d);
        assert_eq!(i.soak_pool, 4, "armor never below 0");
        assert_eq!(after_soak(i, 3), 5);
        assert_eq!(after_soak(i, 30), 0);
    }

    #[test]
    fn tracks_overflow() {
        let t = Tracks { physical: 10, stun: 10, overflow: 4, physical_filled: 8, stun_filled: 7 };
        assert_eq!(apply(t, 5, true), (13, 7));
        assert_eq!(apply(t, 20, true), (14, 7), "capped at track + overflow");
        assert_eq!(apply(t, 3, false), (8, 10));
        assert_eq!(apply(t, 8, false), (10, 10), "5 excess stun → 2 physical");
        let ai = Tracks { stun: 0, ..t };
        assert_eq!(apply(ai, 2, false), (10, 7), "no stun track: physical");
        assert_eq!(wound_modifier(4, 6, 10, 3), -3);
    }
}
