//! Career actions outside the attribute, skill and magic editors: Edge
//! spent and regained, burning Edge and street cred, joining or leaving a
//! magical group and quickening a spell (`CharacterCareer.cs`,
//! `AttributeControl.cmdBurnEdge_Click`, `CharacterAttrib.Degrade`).

use super::ledger::{book_karma, ExpenseUndo, KarmaExpenseType};
use super::{require_career, require_karma, CareerError, CareerRules};
use crate::calc::{self, Rules};
use crate::character::Character;
use crate::engine::Engine;
use crate::improvement::Improvement;

/// `ImprovementSource` of the lowered Edge minimum that burning creates.
pub const BURNED_EDGE_SOURCE: &str = "BurnedEdge";
/// `KarmaJoinGroup` default.
const KARMA_JOIN_GROUP: i32 = 5;
/// `KarmaLeaveGroup` default.
const KARMA_LEAVE_GROUP: i32 = 1;

/// `CharacterAttrib.Degrade` by one point: karma levels first, then base
/// points. With neither left, career-mode Edge burns into its metatype
/// minimum: the `BurnedEdge` improvement lowers the minimum by one more
/// point. Returns false when nothing could be lowered.
pub(super) fn degrade_attribute(ch: &mut Character, rules: &Rules, abbrev: &str) -> bool {
    let total_min = calc::attribute_values(ch, abbrev, rules).total_min;
    let Some((karma, base)) = ch.attribute(abbrev).map(|a| (a.karma, a.base)) else { return false };
    if karma > 0 {
        ch.attribute_mut(abbrev).expect("checked").karma -= 1;
    } else if base > 0 {
        ch.attribute_mut(abbrev).expect("checked").base -= 1;
    } else if abbrev == "EDG" && ch.created && total_min > 0 {
        let list = &mut ch.improvements.list;
        let burned: f64 = list.iter().filter(|i| i.source == BURNED_EDGE_SOURCE && i.kind == "Attribute" && i.improved_name == "EDG").map(|i| i.min * f64::from(i.rating)).sum();
        let burned = 1 - burned as i32;
        list.retain(|i| i.source != BURNED_EDGE_SOURCE);
        list.push(Improvement {
            improved_name: "EDG".into(),
            kind: "Attribute".into(),
            source: BURNED_EDGE_SOURCE.into(),
            min: f64::from(-burned),
            rating: 1,
            enabled: true,
            ..Default::default()
        });
    } else {
        return false;
    }
    ch.dirty = true;
    true
}

/// Permanently burn a point of Edge (`cmdBurnEdge_Click`). Chummer logs
/// no expense for it and nothing is refunded: the point comes off the
/// karma levels, then the base points, then the metatype minimum.
pub fn burn_edge(ch: &mut Character, engine: &Engine) -> Result<(), CareerError> {
    require_career(ch)?;
    let rules = CareerRules::for_character(engine, ch).rules;
    if calc::attribute_values(ch, "EDG", &rules).value <= 0 {
        return Err(CareerError::Refused("you do not have any Edge left to burn".into()));
    }
    if !degrade_attribute(ch, &rules, "EDG") {
        return Err(CareerError::Refused("you do not have any Edge left to burn".into()));
    }
    Ok(())
}

/// Spend a point of Edge (`cmdEdgeSpent_Click`): `<edgeused>` goes up, to
/// at most the Edge total.
pub fn spend_edge(ch: &mut Character, engine: &Engine) -> Result<i32, CareerError> {
    require_career(ch)?;
    let rules = CareerRules::for_character(engine, ch).rules;
    let used = ch.doc.get_i32("edgeused").unwrap_or(0);
    if used >= calc::attribute_values(ch, "EDG", &rules).total {
        return Err(CareerError::Refused("you do not have any Edge left to spend".into()));
    }
    ch.doc.set_child_text("edgeused", (used + 1).to_string());
    ch.dirty = true;
    Ok(used + 1)
}

/// Regain a point of spent Edge (`cmdEdgeGained_Click`).
pub fn regain_edge(ch: &mut Character) -> Result<i32, CareerError> {
    require_career(ch)?;
    let used = ch.doc.get_i32("edgeused").unwrap_or(0);
    if used <= 0 {
        return Err(CareerError::Refused("you have not spent any Edge".into()));
    }
    ch.doc.set_child_text("edgeused", (used - 1).to_string());
    ch.dirty = true;
    Ok(used - 1)
}

/// Burn 2 points of street cred (`cmdBurnStreetCred_Click`), allowed with
/// a total street cred of 2 or more (`CanBurnStreetCred`).
pub fn burn_street_cred(ch: &mut Character, engine: &Engine) -> Result<(), CareerError> {
    require_career(ch)?;
    if super::reputation_for(engine, ch).street_cred < 2 {
        return Err(CareerError::Refused("burning street cred needs a street cred of 2 or more".into()));
    }
    let burnt = ch.doc.get_i32("burntstreetcred").unwrap_or(0);
    ch.doc.set_child_text("burntstreetcred", (burnt + 2).to_string());
    ch.dirty = true;
    Ok(())
}

/// Karma to join (`KarmaJoinGroup`) or leave (`KarmaLeaveGroup`) a
/// magical group. Technomancer networks cost nothing.
pub fn group_karma_cost(engine: &Engine, ch: &Character, join: bool) -> i32 {
    if !ch.mag_enabled() {
        return 0;
    }
    let s = engine.settings.resolve(&ch.field("settings"));
    match (join, s) {
        (true, Some(s)) => s.karma("karmajoingroup", KARMA_JOIN_GROUP),
        (true, None) => KARMA_JOIN_GROUP,
        (false, Some(s)) => s.karma("karmaleavegroup", KARMA_LEAVE_GROUP),
        (false, None) => KARMA_LEAVE_GROUP,
    }
}

/// Join or leave a magical group (`chkJoinGroup_CheckedChanged`): sets
/// `<groupmember>`, and for characters with MAG pays the karma. Returns
/// the expense guid, if one was logged.
pub fn set_group_member(ch: &mut Character, engine: &Engine, member: bool) -> Result<Option<String>, CareerError> {
    require_career(ch)?;
    if ch.flag("groupmember") == member {
        return Ok(None);
    }
    let mut guid = None;
    if ch.mag_enabled() {
        let cost = group_karma_cost(engine, ch, member);
        require_karma(ch, cost)?;
        let (reason, kind) = if member { ("Joined a Group", KarmaExpenseType::JoinGroup) } else { ("Left a Group", KarmaExpenseType::LeaveGroup) };
        guid = Some(book_karma(ch, -cost, reason, ExpenseUndo::karma(kind, "")));
    }
    ch.doc.set_child_text("groupmember", crate::improvement::bool_str(member));
    ch.dirty = true;
    Ok(guid)
}

/// Spend karma to quicken a spell (`cmdQuickenSpell_Click`); the player
/// picks the amount (at least 1). Returns the expense guid.
pub fn quicken_spell(ch: &mut Character, spell_guid: &str, karma: i32) -> Result<String, CareerError> {
    require_career(ch)?;
    let spell = ch.items("spells", "spell").into_iter().find(|s| s.get("guid").eq_ignore_ascii_case(spell_guid)).cloned().ok_or_else(|| CareerError::NotFound(format!("spell {spell_guid}")))?;
    if karma < 1 {
        return Err(CareerError::Refused("quickening costs at least 1 karma".into()));
    }
    require_karma(ch, karma)?;
    let extra = spell.get("extra");
    let name = if extra.is_empty() { spell.get("name") } else { format!("{} ({extra})", spell.get("name")) };
    Ok(book_karma(ch, -karma, format!("Quickened {name}"), ExpenseUndo::karma(KarmaExpenseType::QuickeningMetamagic, "")))
}
