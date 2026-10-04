//! Reversing an expense entry (`tsUndoKarmaExpense_Click` and
//! `tsUndoNuyenExpense_Click` in `CharacterCareer.cs`): reverse what was
//! bought, give back the amount and drop the entry.

use super::karma::{group_karma_unbroken, sync_grades};
use super::ledger::{find_entry, remove_entry, ExpenseEntry, ExpenseType, KarmaExpenseType as K, NuyenExpenseType as N};
use super::nuyen::{find_item_deep_mut, take_item};
use super::{require_career, CareerError};
use crate::character::Character;
use crate::chargen;
use crate::engine::Engine;
use crate::improvement::fmt_num;

/// Undo an entry by guid: reverse its purchase, refund the amount and
/// remove it from the log.
pub fn undo_expense(ch: &mut Character, engine: &Engine, entry_guid: &str) -> Result<(), CareerError> {
    require_career(ch)?;
    let e = find_entry(ch, entry_guid).ok_or_else(|| CareerError::NotFound(format!("expense {entry_guid}")))?;
    let Some(u) = e.undo.clone() else {
        return Err(CareerError::Refused("this entry has no undo history".into()));
    };
    if u.karma_type == K::ImproveInitiateGrade || (e.kind == ExpenseType::Nuyen && u.nuyen_type == N::ImproveInitiateGrade) {
        check_highest_grade(ch, &u.object_id)?;
    }
    match e.kind {
        ExpenseType::Karma => undo_karma_object(ch, engine, &e)?,
        ExpenseType::Nuyen => undo_nuyen_object(ch, &e)?,
    }
    match e.kind {
        ExpenseType::Karma => ch.karma -= e.karma_delta(),
        ExpenseType::Nuyen => ch.nuyen -= e.amount,
    }
    remove_entry(ch, entry_guid);
    ch.dirty = true;
    Ok(())
}

/// Only the highest grade can be undone.
fn check_highest_grade(ch: &Character, grade_guid: &str) -> Result<(), CareerError> {
    let grades = ch.items("initiationgrades", "initiationgrade");
    let max = grades.iter().filter_map(|g| g.get_i32("grade")).max().unwrap_or(0);
    match grades.iter().find(|g| g.get("guid").eq_ignore_ascii_case(grade_guid)) {
        Some(g) if g.get_i32("grade").unwrap_or(0) < max => Err(CareerError::Refused("only the highest grade can be undone".into())),
        _ => Ok(()),
    }
}

/// The per-type part of `tsUndoKarmaExpense_Click`.
fn undo_karma_object(ch: &mut Character, engine: &Engine, e: &ExpenseEntry) -> Result<(), CareerError> {
    let u = e.undo.as_ref().expect("checked by caller");
    let id = u.object_id.as_str();
    match u.karma_type {
        K::ImproveAttribute => degrade_attribute(ch, id),
        K::AddQuality => chargen::remove_quality(ch, id),
        K::AddSpell => {
            ch.remove_item("spells", id);
        }
        K::AddComplexForm => {
            ch.remove_item("complexforms", id);
        }
        K::AddMetamagic => {
            if !ch.remove_item("metamagics", id) {
                ch.remove_item("arts", id);
            }
        }
        K::AddMartialArt => {
            ch.remove_item("martialarts", id);
        }
        K::AddCritterPower => {
            ch.remove_item("critterpowers", id);
        }
        K::SkillSpec | K::AddSpecialization => remove_specialization(ch, id),
        K::ImproveSkillGroup => lower_skill_group(ch, engine, id)?,
        K::AddSkill => undo_add_skill(ch, engine, id)?,
        K::ImproveSkill => lower_skill(ch, id),
        K::ImproveInitiateGrade => remove_grade(ch, id),
        K::RemoveQuality => restore_quality(ch, engine, id, &u.extra)?,
        K::JoinGroup => ch.doc.set_child_text("groupmember", "False"),
        K::LeaveGroup => ch.doc.set_child_text("groupmember", "True"),
        K::AddPowerPoint => {
            let pp = ch.doc.get_i32("magsplitadept").unwrap_or(0);
            ch.doc.set_child_text("magsplitadept", (pp - 1).max(0).to_string());
        }
        K::BindFocus | K::AddMartialArtTechnique => return Err(CareerError::Unsupported(u.karma_type.as_str().into())),
        K::ManualAdd | K::ManualSubtract | K::QuickeningMetamagic | K::AddAIProgram | K::AddAIAdvancedProgram | K::SpiritFettering => {}
    }
    Ok(())
}

/// `CharacterAttrib.Degrade`: karma points first, then base points.
fn degrade_attribute(ch: &mut Character, abbrev: &str) {
    if let Some(a) = ch.attribute_mut(abbrev) {
        if a.karma > 0 {
            a.karma -= 1;
        } else if a.base > 0 {
            a.base -= 1;
        }
    }
}

fn remove_specialization(ch: &mut Character, spec_guid: &str) {
    let hit = |s: &crate::skills::Specialization| s.guid.eq_ignore_ascii_case(spec_guid);
    for k in &mut ch.knowledge_skills {
        k.specs.retain(|s| !hit(s));
    }
    for s in &mut ch.skills {
        s.specs.retain(|s| !hit(s));
    }
}

fn lower_skill(ch: &mut Character, guid: &str) {
    if let Some(s) = ch.skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(guid)) {
        s.karma = (s.karma - 1).max(0);
    } else if let Some(k) = ch.knowledge_skills.iter_mut().find(|k| k.guid.eq_ignore_ascii_case(guid)) {
        k.karma = (k.karma - 1).max(0);
    }
}

/// Lower a skill group found by id or (old files) name, if not broken.
fn lower_skill_group(ch: &mut Character, engine: &Engine, id: &str) -> Result<(), CareerError> {
    let Some(name) = ch.skill_groups.iter().find(|g| g.id.eq_ignore_ascii_case(id) || g.name == id).map(|g| g.name.clone()) else {
        return Ok(());
    };
    if !group_karma_unbroken(ch, engine, &name) {
        return Err(CareerError::Refused(format!("the {name} skill group is broken; lower its skills first")));
    }
    if let Some(g) = ch.skill_groups.iter_mut().find(|g| g.name == name) {
        g.karma = (g.karma - 1).max(0);
    }
    Ok(())
}

/// `AddSkill`: active skills drop to 0 (they cannot be deleted), knowledge
/// skills are removed. Old files used `AddSkill` for skill groups too.
fn undo_add_skill(ch: &mut Character, engine: &Engine, id: &str) -> Result<(), CareerError> {
    if let Some(s) = ch.skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(id)) {
        s.base = 0;
        s.karma = 0;
    } else if ch.knowledge_skills.iter().any(|k| k.guid.eq_ignore_ascii_case(id)) {
        chargen::remove_knowledge_skill(ch, id);
    } else {
        lower_skill_group(ch, engine, id)?;
    }
    Ok(())
}

/// `InitiationGrade.Remove`: drop the grade and the arts, metamagics and
/// enhancements learned at it.
fn remove_grade(ch: &mut Character, grade_guid: &str) {
    let Some(g) = ch.items("initiationgrades", "initiationgrade").into_iter().find(|g| g.get("guid").eq_ignore_ascii_case(grade_guid)).cloned() else {
        return;
    };
    let grade = g.get("grade");
    ch.remove_item("initiationgrades", grade_guid);
    for (container, tag) in [("arts", "art"), ("metamagics", "metamagic"), ("enhancements", "enhancement")] {
        let gone: Vec<String> = ch.items(container, tag).into_iter().filter(|e| e.get("grade") == grade).map(|e| e.get("guid")).collect();
        for guid in gone {
            ch.remove_item(container, &guid);
        }
    }
    sync_grades(ch);
}

/// `RemoveQuality` undo: add the quality back from data.
fn restore_quality(ch: &mut Character, engine: &Engine, source_id: &str, extra: &str) -> Result<(), CareerError> {
    let doc = engine.store.doc("qualities.xml").map_err(|e| CareerError::Refused(e.to_string()))?;
    let rec = crate::data::find(&doc, "qualities", "quality", source_id).ok_or_else(|| CareerError::NotFound(format!("quality {source_id}")))?;
    let answer = Some(extra).filter(|x| !x.is_empty());
    chargen::add_quality(ch, &engine.store, rec, answer);
    Ok(())
}

/// The per-type part of `tsUndoNuyenExpense_Click`.
fn undo_nuyen_object(ch: &mut Character, e: &ExpenseEntry) -> Result<(), CareerError> {
    let u = e.undo.as_ref().expect("checked by caller");
    let id = u.object_id.as_str();
    if id.is_empty() {
        return Ok(());
    }
    match u.nuyen_type {
        N::ManualAdd | N::ManualSubtract | N::ImproveInitiateGrade => {}
        N::IncreaseLifestyle => {
            if let Some(l) = find_item_deep_mut(&mut ch.doc, id) {
                let months = l.get_i32("months").unwrap_or(1);
                l.set_child_text("months", (months - 1).to_string());
            }
        }
        N::AddGear | N::AddArmorGear | N::AddCyberwareGear | N::AddWeaponGear | N::AddVehicleGear => reduce_gear(ch, id, u.qty),
        N::ModifyVehicleWeaponMount => return Err(CareerError::Unsupported(u.nuyen_type.as_str().into())),
        _ => {
            take_item(ch, id);
        }
    }
    Ok(())
}

/// Take back the quantity bought; delete the gear when none is left.
fn reduce_gear(ch: &mut Character, guid: &str, qty: f64) {
    let Some(g) = find_item_deep_mut(&mut ch.doc, guid) else { return };
    let left = g.get_f64("qty").unwrap_or(1.0) - qty;
    if qty > 0.0 && left > 0.0 {
        g.set_child_text("qty", fmt_num(left));
        ch.dirty = true;
    } else {
        take_item(ch, guid);
    }
}
