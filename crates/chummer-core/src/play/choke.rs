//! Shotgun choke settings (SR5 p. 180, "Shotguns" and "Choke Settings";
//! the Defense Modifiers table on p. 189 lists the same defense
//! modifiers). Shot rounds spread in a cone; the choke sets a narrow,
//! medium or wide spread. A wider spread hits more targets in the cone
//! and is harder to dodge, but does less damage and, at long range, is
//! less accurate.
//!
//! Chummer5a has no choke setting. chummer-rs saves it as `<choke>` in
//! the `<weapon>` (Narrow, Medium or Wide; absent when not set, i.e. the
//! weapon fires slugs). Chummer5a ignores the element when it loads a
//! `.chum5` and drops it when it saves one (docs/deviations.md).

use crate::character::Character;
use crate::xml::Element;

/// A choke setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Choke {
    Narrow,
    Medium,
    Wide,
}

/// A weapon's range band (the length of the cone is the band's range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Band {
    #[default]
    Short,
    Medium,
    Long,
    Extreme,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Short, Band::Medium, Band::Long, Band::Extreme];

    /// "Short" … "Extreme" (English; the GUI translates).
    pub fn label(self) -> &'static str {
        match self {
            Band::Short => "Short",
            Band::Medium => "Medium",
            Band::Long => "Long",
            Band::Extreme => "Extreme",
        }
    }
}

/// What a choke setting does at one range band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    /// Added to the Damage Value.
    pub dv: i32,
    /// Added to the weapon's Accuracy.
    pub accuracy: i32,
    /// Added to each target's defense dice pool (the GM applies it).
    pub defense: i32,
    /// Targets one attack roll can hit.
    pub targets: u32,
    /// Width of the spread in meters within which those targets must be;
    /// 0 for a narrow spread (one target).
    pub spread_m: u32,
}

// ----- rules values (SR5 p. 180, Core Rulebook 2nd printing) -----
//
// Narrow spread: the target gets −1 defense at all ranges.
// Medium spread: defense −3 at all ranges; per band (short, medium, long,
//   extreme): DV −1/−3/−5/−7, Accuracy 0/0/−1/−1, up to 2/3/4/6 targets
//   within a 2/4/6/8 m spread. No Called Shots.
// Wide spread: defense −5 at all ranges; DV −3/−5/−7/−9, Accuracy
//   0/0/−1/−1, up to 2/3/4/6 targets within a 3/6/9/12 m spread. No
//   Called Shots.
// Changing the choke is a Simple Action, or a Free Action when the
// shotgun is smartlinked (p. 180).
const NARROW: [Effect; 4] = [Effect { dv: 0, accuracy: 0, defense: -1, targets: 1, spread_m: 0 }; 4];
const MEDIUM: [Effect; 4] = [
    Effect { dv: -1, accuracy: 0, defense: -3, targets: 2, spread_m: 2 },
    Effect { dv: -3, accuracy: 0, defense: -3, targets: 3, spread_m: 4 },
    Effect { dv: -5, accuracy: -1, defense: -3, targets: 4, spread_m: 6 },
    Effect { dv: -7, accuracy: -1, defense: -3, targets: 6, spread_m: 8 },
];
const WIDE: [Effect; 4] = [
    Effect { dv: -3, accuracy: 0, defense: -5, targets: 2, spread_m: 3 },
    Effect { dv: -5, accuracy: 0, defense: -5, targets: 3, spread_m: 6 },
    Effect { dv: -7, accuracy: -1, defense: -5, targets: 4, spread_m: 9 },
    Effect { dv: -9, accuracy: -1, defense: -5, targets: 6, spread_m: 12 },
];

impl Choke {
    pub const ALL: [Choke; 3] = [Choke::Narrow, Choke::Medium, Choke::Wide];

    /// The saved form, `<choke>Medium</choke>`.
    pub fn key(self) -> &'static str {
        match self {
            Choke::Narrow => "Narrow",
            Choke::Medium => "Medium",
            Choke::Wide => "Wide",
        }
    }

    pub fn from_key(s: &str) -> Option<Choke> {
        Choke::ALL.into_iter().find(|c| c.key().eq_ignore_ascii_case(s.trim()))
    }

    /// "Narrow spread" … (English; the GUI translates).
    pub fn label(self) -> &'static str {
        match self {
            Choke::Narrow => "Narrow spread",
            Choke::Medium => "Medium spread",
            Choke::Wide => "Wide spread",
        }
    }

    /// "Narrow", "Medium", "Wide": the selector's labels.
    pub fn short_label(self) -> &'static str {
        self.key()
    }

    /// The effect at each band, short to extreme.
    pub fn effects(self) -> &'static [Effect; 4] {
        match self {
            Choke::Narrow => &NARROW,
            Choke::Medium => &MEDIUM,
            Choke::Wide => &WIDE,
        }
    }

    pub fn effect(self, band: Band) -> Effect {
        self.effects()[band as usize]
    }

    /// The defense modifier, the same at every range.
    pub fn defense(self) -> i32 {
        self.effects()[0].defense
    }

    /// Medium and wide spreads cannot be used with Called Shots.
    pub fn allows_called_shots(self) -> bool {
        self == Choke::Narrow
    }

    /// Whether the effect changes with the range band (medium and wide).
    pub fn by_range(self) -> bool {
        self != Choke::Narrow
    }
}

/// Whether a weapon has a choke: shotguns (category or ammunition
/// category Shotguns, or shotgun ranges, which covers underbarrel
/// shotguns), and the Remington Roomsweeper, a heavy pistol that fires
/// shot rounds "with Heavy Pistol ranges but shotgun rules" (SR5 p. 426).
/// Nothing in the data marks the Roomsweeper, so it is named here.
/// Energy weapons with flechette-shotgun ranges (sonic and laser
/// weapons) have none.
pub fn has_choke(w: &Element) -> bool {
    if w.name != "weapon" || w.get("type") == "Melee" {
        return false;
    }
    w.get("category") == "Shotguns"
        || w.get("ammocategory") == "Shotguns"
        || w.get("range") == "Shotguns"
        || w.get("name").starts_with("Remington Roomsweeper")
}

/// The weapon's saved choke; `None` when not set (slugs).
pub fn choke(w: &Element) -> Option<Choke> {
    if !has_choke(w) {
        return None;
    }
    Choke::from_key(&w.get("choke"))
}

/// `SetChoke`: set or (with `None`) clear a weapon's choke. Returns
/// whether it changed; false for a weapon without a choke.
pub fn set_choke(ch: &mut Character, guid: &str, choke: Option<Choke>) -> bool {
    let Some(w) = super::find_mut(&mut ch.doc, guid) else { return false };
    if !has_choke(w) || self::choke(w) == choke {
        return false;
    }
    match choke {
        Some(c) => w.set_child_text("choke", c.key()),
        None => {
            w.remove_children("choke");
        }
    }
    ch.dirty = true;
    true
}

/// A damage code with `dv` added to its leading number: "12P(f)" with −3
/// is "9P(f)". A code without a leading number gets the modifier
/// appended ("(STR+2)P −1").
pub fn adjust_damage(damage: &str, dv: i32) -> String {
    if dv == 0 {
        return damage.to_owned();
    }
    let t = damage.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    match digits.parse::<i32>() {
        Ok(n) => format!("{}{}", (n + dv).max(0), &t[digits.len()..]),
        Err(_) => format!("{damage} {}", signed(dv)),
    }
}

/// "−3", "+1", "0" with a real minus sign.
pub fn signed(n: i32) -> String {
    match n {
        0 => "0".into(),
        n if n < 0 => format!("−{}", -n),
        n => format!("+{n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapon(fields: &[(&str, &str)]) -> Element {
        let mut w = Element::new("weapon");
        w.set_child_text("type", "Ranged");
        for (k, v) in fields {
            w.set_child_text(k, *v);
        }
        w
    }

    #[test]
    fn which_weapons_have_a_choke() {
        let yes = [
            weapon(&[("name", "Remington 990"), ("category", "Shotguns"), ("alternaterange", "Shotguns (flechette)")]),
            weapon(&[("name", "Defiance T-250, Short-Barrel"), ("category", "Shotguns"), ("range", "Heavy Pistols")]),
            weapon(&[("name", "Remington Roomsweeper"), ("category", "Heavy Pistols")]),
            weapon(&[("name", "Remington Roomsweeper (2050)"), ("category", "Heavy Pistols")]),
            weapon(&[("name", "Underbarrel Shotgun"), ("category", "Underbarrel Weapons"), ("range", "Shotguns")]),
            weapon(&[("name", "Lemat 2072 (Shotgun Barrel)"), ("category", "Heavy Pistols"), ("ammocategory", "Shotguns")]),
        ];
        for w in &yes {
            assert!(has_choke(w), "{}", w.get("name"));
        }
        let no = [
            weapon(&[("name", "Ares Predator V"), ("category", "Heavy Pistols")]),
            weapon(&[("name", "Ares Screech Sonic Rifle"), ("category", "Exotic Ranged Weapons"), ("range", "Shotguns (flechette)"), ("weapontype", "energy")]),
            weapon(&[("name", "Ares Armatus"), ("category", "Laser Weapons"), ("range", "Shotguns (flechette)")]),
            weapon(&[("name", "Mannlicher Dirmingen SX BD"), ("category", "Sporting Rifles"), ("range", "Shotguns (slug)"), ("alternaterange", "Shotguns (flechette)")]),
        ];
        for w in &no {
            assert!(!has_choke(w), "{}", w.get("name"));
        }
        let mut melee = weapon(&[("name", "Shotgun bayonet"), ("category", "Shotguns")]);
        melee.set_child_text("type", "Melee");
        assert!(!has_choke(&melee));
    }

    #[test]
    fn modifiers_per_setting_and_band() {
        // SR5 p. 180.
        for b in Band::ALL {
            assert_eq!(Choke::Narrow.effect(b), Effect { dv: 0, accuracy: 0, defense: -1, targets: 1, spread_m: 0 });
        }
        let row = |c: Choke| c.effects().map(|e| (e.dv, e.accuracy, e.defense, e.targets, e.spread_m));
        assert_eq!(row(Choke::Medium), [(-1, 0, -3, 2, 2), (-3, 0, -3, 3, 4), (-5, -1, -3, 4, 6), (-7, -1, -3, 6, 8)]);
        assert_eq!(row(Choke::Wide), [(-3, 0, -5, 2, 3), (-5, 0, -5, 3, 6), (-7, -1, -5, 4, 9), (-9, -1, -5, 6, 12)]);
        // The Defense Modifiers table (p. 189) agrees.
        assert_eq!(Choke::ALL.map(Choke::defense), [-1, -3, -5]);
        assert_eq!(Choke::ALL.map(Choke::allows_called_shots), [true, false, false]);
        assert_eq!(adjust_damage("12P(f)", -3), "9P(f)");
        assert_eq!(adjust_damage("11P", 0), "11P");
        assert_eq!(adjust_damage("2S", -5), "0S");
        assert_eq!(adjust_damage("(STR+2)P", -1), "(STR+2)P −1");
        assert_eq!(Choke::from_key(" medium "), Some(Choke::Medium));
        assert_eq!(Choke::from_key(""), None);
    }
}
