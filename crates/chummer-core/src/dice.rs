//! Shadowrun dice: pools of d6, hits on 5+, glitches when half or more of
//! the dice show 1.

use std::time::{SystemTime, UNIX_EPOCH};

/// Small non-cryptographic RNG (xorshift64*). Good enough for dice.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn from_time() -> Self {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9);
        Rng::seeded(t ^ 0x2545_F491_4F6C_DD1D)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `1..=6`, without modulo bias.
    pub fn d6(&mut self) -> u8 {
        loop {
            let v = self.next_u64() >> 61; // 0..8
            if v < 6 {
                return v as u8 + 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic scope (see `command`)
// ---------------------------------------------------------------------------

/// What a change may generate while [`deterministic`] is in effect: random
/// values (new GUIDs) from a seeded generator, and a fixed "now".
struct Scope {
    rng: Rng,
    now: String,
}

thread_local! {
    static SCOPE: std::cell::RefCell<Option<Scope>> = const { std::cell::RefCell::new(None) };
}

/// Ends the scope set up by [`deterministic`] when dropped.
pub struct ScopeGuard {
    prev: Option<Scope>,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        SCOPE.with(|s| *s.borrow_mut() = prev);
    }
}

/// SplitMix64, to spread nearby seeds before they seed the xorshift.
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Until the guard drops, [`crate::items::new_guid`] draws from a generator
/// seeded with `seed` and [`crate::chargen::now_iso`] returns `now`, on this
/// thread. Used by `command::apply` so a change gives the same result on
/// every machine.
pub fn deterministic(seed: u64, now: String) -> ScopeGuard {
    let scope = Scope { rng: Rng::seeded(splitmix(seed)), now };
    let prev = SCOPE.with(|s| s.borrow_mut().replace(scope));
    ScopeGuard { prev }
}

/// The next value of the deterministic scope's generator, if one is set.
pub fn scoped_u64() -> Option<u64> {
    SCOPE.with(|s| s.borrow_mut().as_mut().map(|sc| sc.rng.next_u64()))
}

/// The deterministic scope's "now", if one is set.
pub fn scoped_now() -> Option<String> {
    SCOPE.with(|s| s.borrow().as_ref().map(|sc| sc.now.clone()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glitch {
    None,
    Glitch,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roll {
    pub dice: Vec<u8>,
    pub hits: u32,
    pub ones: u32,
    pub glitch: Glitch,
}

/// Count hits and glitches for a set of dice. `rule_of_six` re-rolls (adds)
/// a die for every 6, as with Edge's Push the Limit.
pub fn evaluate(dice: Vec<u8>, threshold: u8) -> Roll {
    let hits = dice.iter().filter(|&&d| d >= threshold).count() as u32;
    let ones = dice.iter().filter(|&&d| d == 1).count() as u32;
    let glitch = if !dice.is_empty() && ones * 2 >= dice.len() as u32 {
        if hits == 0 { Glitch::Critical } else { Glitch::Glitch }
    } else {
        Glitch::None
    };
    Roll { dice, hits, ones, glitch }
}

pub fn roll(rng: &mut Rng, pool: u32, rule_of_six: bool, limit: Option<u32>) -> Roll {
    let mut dice = Vec::with_capacity(pool as usize);
    let mut todo = pool;
    while todo > 0 && dice.len() < 1000 {
        todo -= 1;
        let d = rng.d6();
        if rule_of_six && d == 6 {
            todo += 1;
        }
        dice.push(d);
    }
    let mut r = evaluate(dice, 5);
    if let Some(l) = limit {
        r.hits = r.hits.min(l);
    }
    r
}

/// A roll as it is logged and sent (the GM's roll log, a player's roll
/// reaching the GM): the dice and how they were rolled. The result is
/// worked out from the dice ([`RollRecord::outcome`]), never taken from
/// whoever made the roll, so the two always agree.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RollRecord {
    /// Unix ms, on the machine that rolled.
    pub at: i64,
    /// What it was rolled for ("Pistols · Ares Predator V", "Soak",
    /// "Initiative"); empty for a free roll.
    pub label: String,
    /// The dice asked for, before Edge (initiative: the d6 count).
    pub pool: u32,
    /// Push the Limit: the Edge dice added.
    pub edge: Option<u32>,
    /// Every 6 rolled added a die (Edge).
    pub rule_of_six: bool,
    /// The limit on hits, when one applied.
    pub limit: Option<u32>,
    /// An initiative roll: its base (the score is base + the dice).
    pub initiative: Option<i32>,
    pub dice: Vec<u8>,
}

impl RollRecord {
    /// The hits, ones and glitch of the dice (hits capped by the limit).
    pub fn outcome(&self) -> Roll {
        let mut r = evaluate(self.dice.clone(), 5);
        if let Some(l) = self.limit {
            r.hits = r.hits.min(l);
        }
        r
    }

    /// An initiative roll's score.
    pub fn score(&self) -> Option<i32> {
        self.initiative.map(|b| b.saturating_add(self.dice.iter().map(|&d| i32::from(d)).sum::<i32>()))
    }

    /// The dice rolled, with Edge.
    pub fn dice_count(&self) -> u32 {
        self.pool + self.edge.unwrap_or(0)
    }

    /// Whether the dice fit how they were rolled: faces 1 to 6, as many
    /// as the pool and Edge (and one more per 6 with the Rule of Six).
    pub fn check(&self) -> Result<(), String> {
        if self.dice.iter().any(|&d| !(1..=6).contains(&d)) {
            return Err("a die shows no face of a d6".into());
        }
        let asked = self.dice_count() as usize;
        let sixes = if self.rule_of_six { self.dice.iter().filter(|&&d| d == 6).count() } else { 0 };
        // `roll` stops at 1000 dice.
        if self.dice.len() != (asked + sixes).min(1000) {
            return Err(format!("{} dice for a pool of {asked}", self.dice.len()));
        }
        if self.initiative.is_some() && (self.rule_of_six || self.limit.is_some()) {
            return Err("an initiative roll has no limit or Rule of Six".into());
        }
        Ok(())
    }
}

/// Initiative: base + Nd6.
pub fn initiative(rng: &mut Rng, base: i32, dice: u32) -> (i32, Vec<u8>) {
    let rolled: Vec<u8> = (0..dice).map(|_| rng.d6()).collect();
    (base.saturating_add(rolled.iter().map(|&d| i32::from(d)).sum::<i32>()), rolled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_and_glitches() {
        let r = evaluate(vec![5, 6, 1, 2], 5);
        assert_eq!((r.hits, r.ones, r.glitch), (2, 1, Glitch::None));
        let r = evaluate(vec![1, 1, 5, 3], 5);
        assert_eq!(r.glitch, Glitch::Glitch);
        let r = evaluate(vec![1, 1, 2, 3], 5);
        assert_eq!(r.glitch, Glitch::Critical);
    }

    #[test]
    fn d6_is_in_range_and_covers_all_faces() {
        let mut rng = Rng::seeded(42);
        let mut seen = [0u32; 7];
        for _ in 0..6000 {
            seen[rng.d6() as usize] += 1;
        }
        assert_eq!(seen[0], 0);
        assert!(seen[1..].iter().all(|&n| n > 800), "{seen:?}");
    }

    #[test]
    fn records_work_out_their_result_from_the_dice() {
        let r = RollRecord { label: "Pistols".into(), pool: 4, limit: Some(1), dice: vec![6, 5, 1, 2], ..Default::default() };
        assert_eq!(r.outcome().hits, 1, "capped by the limit");
        assert_eq!(r.check(), Ok(()));
        assert!(RollRecord { dice: vec![6, 5, 1], ..r.clone() }.check().is_err(), "a die short");
        assert!(RollRecord { dice: vec![6, 5, 1, 7], ..r.clone() }.check().is_err());
        // Push the Limit: Edge dice, and one more for each 6.
        let p = RollRecord { pool: 2, edge: Some(1), rule_of_six: true, dice: vec![6, 3, 2, 5], ..Default::default() };
        assert_eq!(p.check(), Ok(()));
        let i = RollRecord { label: "Initiative".into(), pool: 2, initiative: Some(9), dice: vec![3, 4], ..Default::default() };
        assert_eq!((i.score(), i.check()), (Some(16), Ok(())));
    }

    #[test]
    fn limit_caps_hits() {
        let mut rng = Rng::seeded(7);
        let r = roll(&mut rng, 30, false, Some(3));
        assert!(r.hits <= 3);
        assert_eq!(r.dice.len(), 30);
    }
}
