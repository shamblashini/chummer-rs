//! Game-master content tools: critters and NPCs (`File → New Critter`),
//! PACKS kits (`Add PACKS Kit` / `Create PACKS Kit`) and custom spells
//! (`Create Spell`).

pub mod critter;
pub mod custom_spell;
pub mod packs;

/// `CommonFunctions.ExpressionToInt`: replace `F`, `1D6` and `2D6` with the
/// force, evaluate, round, add `offset`. With a force, the result is at
/// least `min_from_force`; without one, at least 0.
pub fn expression_to_int(s: Option<&str>, force: i32, offset: i32, min_from_force: i32) -> i32 {
    let Some(s) = s.filter(|s| !s.trim().is_empty()) else { return offset };
    let f = force.to_string();
    let replaced = s.replace('F', &f).replace("1D6", &f).replace("2D6", &f);
    // A failed evaluation leaves Chummer's starting value of 1.
    let v = crate::expr::evaluate_num(&replaced).map_or(1, crate::expr::standard_round) + offset;
    if force > 0 {
        v.max(min_from_force)
    } else {
        v.max(0)
    }
}

/// `CommonFunctions.ExpressionToDecimal` with no offset: like
/// [`expression_to_int`] but unrounded, never below 0.
pub fn expression_to_dec(s: Option<&str>, force: i32) -> f64 {
    let Some(s) = s.filter(|s| !s.trim().is_empty()) else { return 0.0 };
    let f = force.to_string();
    let replaced = s.replace('F', &f).replace("1D6", &f).replace("2D6", &f);
    crate::expr::evaluate_num(&replaced).unwrap_or(1.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_expressions() {
        assert_eq!(expression_to_int(Some("F-2"), 6, 0, 1), 4);
        assert_eq!(expression_to_int(Some("F/2"), 5, 0, 1), 3, "2.5 rounds away from zero");
        assert_eq!(expression_to_int(Some("(F*2)+4"), 3, 0, 1), 10);
        assert_eq!(expression_to_int(Some("F-3"), 1, 0, 1), 1, "at least 1 with a force");
        assert_eq!(expression_to_int(Some("0"), 4, 0, 1), 1, "even a 0 limit is raised to 1");
        assert_eq!(expression_to_int(Some("0"), 4, 0, 0), 0);
        assert_eq!(expression_to_int(Some("3"), 0, 0, 1), 3);
        assert_eq!(expression_to_int(Some("F-3"), 0, 0, 1), 0, "no force: at least 0");
        assert_eq!(expression_to_int(None, 6, 0, 1), 0);
        assert_eq!(expression_to_dec(Some("F"), 6), 6.0);
    }
}
