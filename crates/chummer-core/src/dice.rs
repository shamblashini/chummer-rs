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

/// Initiative: base + Nd6.
pub fn initiative(rng: &mut Rng, base: i32, dice: u32) -> (i32, Vec<u8>) {
    let rolled: Vec<u8> = (0..dice).map(|_| rng.d6()).collect();
    (base + rolled.iter().map(|&d| i32::from(d)).sum::<i32>(), rolled)
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
    fn limit_caps_hits() {
        let mut rng = Rng::seeded(7);
        let r = roll(&mut rng, 30, false, Some(3));
        assert!(r.hits <= 3);
        assert_eq!(r.dice.len(), 30);
    }
}
