//! What each [`Command`] does: the calls the GUI used to make directly.

use super::{Command, ImprovementRef, RecordRef, Rejected};
use crate::career::{self, CareerRules, NuyenExpenseType};
use crate::character::Character;
use crate::data::{self, DataStore, Record};
use crate::engine::Engine;
use crate::items::magic::{account, martialart, mentor, metamagic, power, spell, spirit};
use crate::items::{self, edit, lifestyle};
use crate::play::{ai, ammo, matrix, vehicle};
use crate::{calendar, chargen, contacts, custom_improvement as custom, format};

/// How a command ended.
pub(super) enum Done {
    /// Nothing to change (the value was already set, the item is gone).
    Unchanged,
    Changed { message: Option<String>, count: Option<usize> },
}

type R = Result<Done, Rejected>;

fn changed() -> R {
    Ok(Done::Changed { message: None, count: None })
}

fn said(message: impl Into<String>) -> R {
    Ok(Done::Changed { message: Some(message.into()), count: None })
}

/// The usual "returns true if it changed anything".
fn flag(b: bool) -> R {
    if b {
        changed()
    } else {
        Ok(Done::Unchanged)
    }
}

fn reject<E: std::fmt::Display>(e: E) -> Rejected {
    Rejected::new(e.to_string())
}

/// A field a command names becomes an element name in the save.
fn field_name(key: &str) -> Result<(), Rejected> {
    if crate::xml::is_name(key) {
        Ok(())
    } else {
        Err(Rejected::new(format!("{key:?} is not a field name")))
    }
}

/// A record of kind `tag` (`items::KINDS`).
fn with_record<T>(store: &DataStore, file: &str, container: &str, item: &str, r: &RecordRef, f: impl FnOnce(Record<'_>) -> Result<T, Rejected>) -> Result<T, Rejected> {
    let doc = store.doc(file).map_err(reject)?;
    let rec = (!r.id.is_empty()).then(|| data::find(&doc, container, item, &r.id)).flatten().or_else(|| data::find(&doc, container, item, &r.name));
    match rec {
        Some(rec) => f(rec),
        None => Err(Rejected::new(format!("{} is not in {file}", if r.name.is_empty() { &r.id } else { &r.name }))),
    }
}

/// The improvement `at` names, if it is still where the command saw it.
fn improvement(ch: &Character, at: &ImprovementRef) -> Result<usize, Rejected> {
    let i = at.index as usize;
    match ch.improvements.list.get(i) {
        Some(imp) if imp.source_name == at.source => Ok(i),
        _ => Err(Rejected::new("the improvement has changed; try again")),
    }
}

fn skill_mut<'a>(ch: &'a mut Character, guid: &str) -> Result<&'a mut crate::skills::Skill, Rejected> {
    ch.skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(guid)).ok_or_else(|| Rejected::new(format!("no skill {guid}")))
}

fn knowledge_mut<'a>(ch: &'a mut Character, guid: &str) -> Result<&'a mut crate::skills::KnowledgeSkill, Rejected> {
    ch.knowledge_skills.iter_mut().find(|s| s.guid.eq_ignore_ascii_case(guid)).ok_or_else(|| Rejected::new(format!("no knowledge skill {guid}")))
}

fn group_mut<'a>(ch: &'a mut Character, name: &str) -> Result<&'a mut crate::skills::SkillGroup, Rejected> {
    ch.skill_groups.iter_mut().find(|g| g.name == name).ok_or_else(|| Rejected::new(format!("no skill group {name}")))
}

fn attribute_mut<'a>(ch: &'a mut Character, name: &str) -> Result<&'a mut crate::attributes::Attribute, Rejected> {
    ch.attribute_mut(name).ok_or_else(|| Rejected::new(format!("no attribute {name}")))
}

pub(super) fn run(ch: &mut Character, engine: &Engine, cmd: &Command) -> R {
    use Command::*;
    let store = engine.store_for_character(ch);
    let store = &*store;
    match cmd {
        // ----- fields and settings -----
        SetField { key, value } => {
            field_name(key)?;
            ch.set_field(key, value.clone());
            changed()
        }
        SetKarma { value } => {
            ch.karma = *value;
            changed()
        }
        SetNuyen { value } => {
            ch.nuyen = *value;
            changed()
        }
        ChangeMetatype { choice } => {
            let before = super::canonical(ch);
            chargen::rebuild::apply(ch, engine, choice).map_err(Rejected::new)?;
            flag(super::canonical(ch) != before)
        }
        SwitchSettings { key } => {
            let preset = engine.settings.find(key).cloned().ok_or_else(|| Rejected::new(format!("no settings preset {key}")))?;
            crate::settings::switch_character(ch, &preset).map_err(Rejected::new)?;
            changed()
        }

        // ----- creation -----
        SetAttributeBase { attribute, value } => {
            attribute_mut(ch, attribute)?.base = *value;
            changed()
        }
        SetAttributeKarma { attribute, value } => {
            attribute_mut(ch, attribute)?.karma = *value;
            changed()
        }
        SetSkillBase { skill, value } => {
            skill_mut(ch, skill)?.base = *value;
            changed()
        }
        SetSkillKarma { skill, value } => {
            skill_mut(ch, skill)?.karma = *value;
            changed()
        }
        SetKnowledgeBase { skill, value } => {
            knowledge_mut(ch, skill)?.base = *value;
            changed()
        }
        SetKnowledgeKarma { skill, value } => {
            knowledge_mut(ch, skill)?.karma = *value;
            changed()
        }
        SetGroupBase { group, value } => {
            group_mut(ch, group)?.base = *value;
            changed()
        }
        SetGroupKarma { group, value } => {
            group_mut(ch, group)?.karma = *value;
            changed()
        }
        AddKnowledgeSkill { name, kind, native } => {
            chargen::add_knowledge_skill(ch, name, kind, *native);
            changed()
        }
        RemoveKnowledgeSkill { skill } => {
            chargen::remove_knowledge_skill(ch, skill);
            changed()
        }
        AddSpecialization { skill, name } => {
            chargen::add_specialization(ch, skill, name);
            changed()
        }
        SetTradition { name } => {
            chargen::set_tradition(ch, store, name).map_err(Rejected::new)?;
            changed()
        }
        AddLifeModule { module, version } => {
            chargen::add_life_module(ch, store, module, version.as_deref()).map_err(Rejected::new)?;
            changed()
        }
        FinishCreation => {
            let settings = engine.settings.resolve(&ch.field("settings")).cloned().ok_or_else(|| Rejected::new("the character's settings preset is missing"))?;
            let rules = engine.rules_for(ch);
            let sheet = crate::calc::compute(ch, &rules, Some(store), Some(&engine.catalog));
            let budget = chargen::budget_with(ch, &sheet, &rules, &settings, Some(store));
            chargen::finalize(ch, &budget, &settings);
            changed()
        }

        // ----- career karma -----
        RaiseAttribute { attribute } => said(career::improve_attribute(ch, engine, attribute).map_err(reject)?),
        RaiseSkill { skill } => said(career::improve_skill(ch, engine, skill).map_err(reject)?),
        RaiseSkillGroup { group } => said(career::improve_skill_group(ch, engine, group).map_err(reject)?),
        BuySpecialization { skill, name } => said(career::buy_specialization(ch, engine, skill, name).map_err(reject)?),
        LearnKnowledgeSkill { name, kind } => said(career::learn_knowledge_skill(ch, engine, name, kind).map_err(reject)?),
        Initiate { options } => said(career::add_initiation_grade(ch, engine, *options).map_err(reject)?),
        UndoExpense { entry } => {
            career::undo_expense(ch, engine, entry).map_err(reject)?;
            changed()
        }
        ManualExpense { karma, gain, expense } => {
            let rules = CareerRules::for_character(engine, ch);
            let r = match (karma, gain) {
                (true, true) => career::karma_gained(ch, &rules, expense),
                (true, false) => career::karma_spent(ch, &rules, expense),
                (false, true) => career::nuyen_gained(ch, &rules, expense),
                (false, false) => career::nuyen_spent(ch, &rules, expense),
            };
            r.map_err(reject)?;
            changed()
        }

        // ----- career actions -----
        SpendEdge => {
            career::spend_edge(ch, engine).map_err(reject)?;
            changed()
        }
        RegainEdge => {
            career::regain_edge(ch).map_err(reject)?;
            changed()
        }
        BurnEdge => {
            career::burn_edge(ch, engine).map_err(reject)?;
            changed()
        }
        SetEdgeUsed { used } => {
            let total = engine.sheet(ch).attr("EDG").max(0);
            flag(career::set_edge_used(ch, total, *used))
        }
        RefreshEdge => flag(career::refresh_edge(ch)),
        BurnStreetCred => {
            career::burn_street_cred(ch, engine).map_err(reject)?;
            changed()
        }
        SetGroupMember { member } => {
            career::set_group_member(ch, engine, *member).map_err(reject)?;
            changed()
        }

        // ----- items -----
        AddItem { tag, record, purchase } => add_item(ch, engine, store, tag, record, purchase),
        RemoveItem { container, guid } => match container.as_str() {
            // Career mode: buying off a negative quality costs karma.
            "qualities" if ch.created => {
                let msg = career::remove_quality(ch, engine, guid).map_err(reject)?;
                Ok(Done::Changed { message: msg, count: None })
            }
            "qualities" => {
                chargen::remove_quality(ch, guid);
                changed()
            }
            "cyberwares" => flag(items::cyberware::remove(ch, guid)),
            c => flag(ch.remove_item(c, guid)),
        },
        DeleteItem { guid } => flag(edit::remove(ch, guid)),
        SellItem { guid, fraction } => {
            let v = edit::sell(ch, store, guid, *fraction).map_err(reject)?;
            said(format!("Sold for {}", format::nuyen(v)))
        }
        SetItemRating { guid, rating } => {
            edit::apply_rating_change(ch, store, guid, *rating).map_err(Rejected::new)?;
            changed()
        }
        SetItemQuantity { guid, qty } => flag(edit::set_quantity(ch, guid, *qty)),
        SetItemEquipped { guid, on } => flag(edit::set_equipped(ch, store, guid, *on)),
        SetItemWireless { guid, on } => flag(edit::set_wireless(ch, store, guid, *on)),
        SetItemText { guid, field, value } => {
            field_name(field)?;
            flag(edit::set_text(ch, guid, field, value))
        }
        AddItemLocation { guid, name } => match edit::add_location(ch, guid, name) {
            Some(loc) => flag(edit::set_text(ch, guid, "location", &loc)),
            None => Ok(Done::Unchanged),
        },
        MoveItem { item, to } => {
            let enforce = items::place::enforces_capacity(ch, engine);
            match items::place::move_item(ch, store, item, to, enforce) {
                Ok(true) => changed(),
                Ok(false) => Ok(Done::Unchanged),
                Err(m) => {
                    let name = edit::find(ch, item).map(|e| e.get("name")).unwrap_or_else(|| "That item".into());
                    Err(Rejected::new(m.message(&name, &items::place::place_name(ch, to))))
                }
            }
        }
        AddWeaponMount { vehicle, size } => {
            edit::add_weapon_mount(ch, store, vehicle, size).map_err(Rejected::new)?;
            changed()
        }
        AddCustomDrug { name, grade, components } => add_custom_drug(ch, store, name, grade, components),
        ApplyKit { kit } => {
            let kit = crate::xml::parse(kit).map_err(reject)?;
            let settings = engine.settings.resolve(&ch.field("settings")).cloned();
            let report = crate::gm::packs::apply(ch, store, settings.as_ref(), &kit);
            let message = (!report.skipped.is_empty()).then(|| report.skipped.join("\n"));
            Ok(Done::Changed { message, count: Some(report.added.len()) })
        }
        RemoveAiProgram { guid } => {
            items::aiprogram::remove(ch, guid).map_err(Rejected::new)?;
            changed()
        }

        // ----- lifestyles -----
        UpdateLifestyle { guid, options } => flag(lifestyle::update(ch, guid, options)),
        PayLifestyleMonth { guid } => {
            let l = ch.items("lifestyles", "lifestyle").into_iter().find(|l| l.get("guid") == *guid).cloned().ok_or_else(|| Rejected::new("no such lifestyle"))?;
            let cost = lifestyle::monthly_cost(ch, &l);
            career::spend_nuyen(ch, cost, &format!("Lifestyle {}", l.get("name")), NuyenExpenseType::IncreaseLifestyle, guid, 0.0).map_err(reject)?;
            let mut o = lifestyle::Options::from_saved(&l);
            o.months += 1;
            lifestyle::update(ch, guid, &o);
            said(format!("Paid {} for {}", format::nuyen(cost), l.get("name")))
        }
        AddLifestyleQuality { lifestyle: lguid, quality, answer, free } => {
            let r = RecordRef { id: String::new(), name: quality.clone() };
            with_record(store, "lifestyles.xml", "qualities", "quality", &r, |rec| lifestyle::add_quality(ch, store, lguid, rec, answer.as_deref(), *free).map_err(Rejected::new))?;
            said(format!("Added {quality}"))
        }
        RemoveLifestyleQuality { lifestyle: lguid, quality } => flag(lifestyle::remove_quality(ch, lguid, quality)),

        // ----- magic -----
        AddSpell { record, answer, options } => {
            let name = with_record(store, "spells.xml", "spells", "spell", record, |rec| {
                let name = rec.name();
                if ch.created {
                    career::learn_spell_with(ch, engine, store, rec, answer.as_deref(), options).map_err(reject)?;
                } else {
                    spell::add(ch, store, rec, answer.as_deref(), options);
                }
                Ok(name)
            })?;
            said(if ch.created { format!("Learned {name}") } else { format!("Added {name}") })
        }
        AddCustomSpell { design } => {
            if !ch.created && crate::gm::custom_spell::creation_limit_reached(ch, &engine.sheet(ch)) {
                return Err(Rejected::new("You cannot have more spells, rituals or alchemical preparations than twice your MAG score. Ref: Page 69, SR5 Core."));
            }
            crate::gm::custom_spell::add(ch, engine, design).map_err(reject)?;
            changed()
        }
        QuickenSpell { spell, karma } => {
            career::quicken_spell(ch, spell, *karma).map_err(reject)?;
            changed()
        }
        SetMentorChoices { mentor: guid, choice1, choice2 } => {
            mentor::set_mentor_choices(ch, store, guid, choice1.as_deref(), choice2.as_deref()).map_err(Rejected::new)?;
            changed()
        }
        ChooseMentor { quality, mentor_type, name } => {
            mentor::add_mentor_for_quality(ch, store, quality, mentor_type, name, None, None).map_err(Rejected::new)?;
            said(format!("{name} is now your mentor; pick its choices below"))
        }
        AddMetamagic { kind, name, answer, grade } => {
            let (file, container, item) = metamagic::data_path(kind);
            let r = RecordRef { id: String::new(), name: name.clone() };
            with_record(store, file, container, item, &r, |rec| {
                if ch.created {
                    career::learn_metamagic(ch, engine, rec, answer.as_deref(), *grade).map(|_| ()).map_err(reject)
                } else {
                    metamagic::add(ch, store, rec, answer.as_deref()).map(|_| ()).map_err(Rejected::new)
                }
            })?;
            said(format!("Added {name}"))
        }
        LearnTechnique { art, technique } => {
            if ch.created {
                career::learn_technique(ch, engine, store, art, technique).map_err(reject)?;
            } else {
                martialart::add_technique(ch, store, art, technique).map_err(Rejected::new)?;
            }
            said(format!("Learned {technique}"))
        }
        BuyPowerPoint => {
            career::buy_power_point(ch, engine).map_err(reject)?;
            said("Bought a power point")
        }
        SetPowerRating { power: guid, rating } => set_power_rating(ch, engine, store, guid, *rating),
        SetSpiritState { spirit: guid, force, services, bound, fettered } => {
            let s = ch.items("spirits", "spirit").into_iter().find(|s| s.get("guid") == *guid).cloned().ok_or_else(|| Rejected::new("no such spirit"))?;
            let was = s.get_bool("fettered").unwrap_or(false);
            if ch.created && *fettered != was {
                // Career mode: fettering costs karma (Spirit.Fettered).
                career::set_spirit_fettered(ch, engine, guid, *fettered).map_err(reject)?;
                said(if *fettered { format!("Fettered {}", s.get("name")) } else { format!("Released {}", s.get("name")) })
            } else {
                flag(spirit::set_state(ch, guid, *force, *services, *bound, *fettered))
            }
        }
        BindFocus { gear } => {
            let message = if ch.created {
                let name = ch.items("gears", "gear").into_iter().find(|g| g.get("guid") == *gear).map(|g| g.get("name")).unwrap_or_default();
                let cost = ch.karma;
                career::bind_focus(ch, engine, gear).map_err(reject)?;
                Some(format!("Bound {name} for {} karma", cost - ch.karma))
            } else {
                account::bind_focus(ch, gear).ok_or_else(|| Rejected::new("this focus cannot be bound"))?;
                None
            };
            account::set_focus_bonded(ch, store, gear, true);
            Ok(Done::Changed { message, count: None })
        }
        UnbindFocus { gear } => {
            account::unbind_focus(ch, gear);
            account::set_focus_bonded(ch, store, gear, false);
            changed()
        }

        // ----- custom improvements -----
        CreateImprovement { form, group, edit } => {
            custom::create(ch, store, form, group, edit.as_deref()).map_err(reject)?;
            changed()
        }
        RemoveImprovement { source } => flag(custom::remove(ch, source)),
        SetImprovementEnabled { at, on } => {
            let i = improvement(ch, at)?;
            flag(custom::set_enabled(ch, i, *on))
        }
        SetImprovementNotes { at, notes } => {
            let i = improvement(ch, at)?;
            custom::set_notes(ch, i, notes);
            changed()
        }
        SetImprovementGroup { at, group } => {
            let i = improvement(ch, at)?;
            custom::set_group(ch, i, group);
            changed()
        }
        SetImprovementGroupEnabled { group, on } => flag(custom::set_group_enabled(ch, group, *on) > 0),
        AddImprovementGroup { name } => flag(custom::add_group(ch, name)),
        RenameImprovementGroup { old, new } => flag(custom::rename_group(ch, old, new)),
        RemoveImprovementGroup { name } => flag(custom::remove_group(ch, name)),

        // ----- contacts -----
        AddContact { kind } => {
            contacts::add(ch, *kind);
            changed()
        }
        ImportContacts { xml } => {
            let n = contacts::import(ch, xml).map_err(Rejected::new)?;
            if n == 0 {
                return Ok(Done::Unchanged);
            }
            Ok(Done::Changed { message: None, count: Some(n) })
        }
        SetContactField { contact, key, value } => {
            field_name(key)?;
            flag(contacts::set_field(ch, contact, key, value))
        }
        SetContactNotes { contact, notes, color } => {
            let text = notes.as_ref().is_some_and(|n| contacts::set_field(ch, contact, "notes", n));
            flag(text | contacts::set_notes_color(ch, contact, *color))
        }
        MoveContact { contact, target, after } => flag(contacts::move_contact(ch, contact, target, *after)),
        MoveContactStep { contact, up } => flag(contacts::move_step(ch, contact, *up)),
        SortContacts { kind, by } => flag(contacts::sort(ch, *kind, *by)),
        RemoveContact { contact } => flag(contacts::remove(ch, contact)),
        LinkContact { contact, file, startup } => flag(contacts::link(ch, contact, std::path::Path::new(file), std::path::Path::new(startup))),
        UnlinkContact { contact } => flag(contacts::unlink(ch, contact)),

        // ----- calendar -----
        AddWeek => {
            calendar::add_next_week(ch, None);
            changed()
        }
        SetWeekNotes { week, notes } => {
            calendar::set_notes(ch, week, notes);
            changed()
        }
        RemoveWeek { week } => flag(calendar::remove_week(ch, week)),

        // ----- play -----
        SetPhysicalDamage { filled } => flag(ai::set_physical_filled(ch, *filled)),
        SetStunDamage { filled } => flag(ai::set_stun_filled(ch, *filled)),
        SetActiveClip { weapon, slot } => flag(ammo::set_active_slot(ch, weapon, *slot as usize)),
        Fire { weapon, mode } => match ammo::fire(ch, weapon, *mode) {
            ammo::Fired::Fired(_) => changed(),
            ammo::Fired::OutOfAmmo => Err(Rejected::new(ammo::OUT_OF_AMMO)),
            ammo::Fired::Cannot(m) => Err(Rejected::new(m)),
            ammo::Fired::Confirm(m) => Err(Rejected { reason: m.to_owned(), confirm: true }),
        },
        SetAmmoRemaining { weapon, count } => flag(ammo::set_remaining(ch, weapon, *count)),
        Reload { weapon, ammo: a, count } => {
            ammo::reload(ch, weapon, a.as_deref(), *count).map_err(Rejected::new)?;
            changed()
        }
        Unload { weapon } => flag(ammo::unload(ch, weapon)),
        SetCharges { weapon, count } => flag(ammo::set_charges(ch, weapon, *count)),
        SetVehicleDamage { vehicle: guid, filled } => flag(vehicle::set_filled(ch, guid, *filled, &Default::default())),
        SetMatrixDamage { device, filled } => flag(matrix::set_filled(ch, device, *filled)),
        SetActiveCommlink { device, on } => flag(matrix::set_active(ch, device, *on)),
        SetHomeNode { device, on } => flag(matrix::set_home_node(ch, device, *on)),
        Revert { snapshot, .. } => {
            let mut state = super::restore(snapshot).map_err(reject)?;
            if super::canonical(&state) == super::canonical(ch) {
                return Ok(Done::Unchanged);
            }
            state.file = ch.file.clone();
            *ch = state;
            changed()
        }
    }
}

/// The selection dialog's "Add": in career mode karma (qualities, martial
/// arts, critter powers, programs) or nuyen (equipment) is spent and
/// logged; an item that cannot be paid for is not added.
fn add_item(ch: &mut Character, engine: &Engine, store: &DataStore, tag: &str, record: &RecordRef, purchase: &items::Purchase) -> R {
    let kind = items::kind(tag).ok_or_else(|| Rejected::new(format!("unknown item kind {tag}")))?;
    with_record(store, kind.file, kind.data_container, kind.data_item, record, |rec| {
        let name = rec.name();
        if ch.created && matches!(tag, "quality" | "martialart" | "critterpower" | "aiprogram") {
            let answer = purchase.answer.as_deref();
            match tag {
                "martialart" => career::learn_martial_art(ch, engine, rec, answer),
                "critterpower" => career::learn_critter_power(ch, engine, rec, purchase.rating, answer),
                "aiprogram" => career::learn_ai_program(ch, engine, rec, purchase),
                _ => career::add_quality(ch, engine, rec, answer),
            }
            .map_err(reject)?;
            return said(format!("Added {name}"));
        }
        let guid = items::add(tag, ch, store, rec, purchase).map_err(|e| Rejected::new(format!("Could not add {name}: {e}")))?;
        edit::settle_new_item(ch, &guid);
        // Into a location: a top-level item of a kind with locations.
        if let Some(loc) = purchase.location.as_deref().filter(|l| !l.is_empty()) {
            if !edit::has_location(ch, &guid) || !edit::locations(ch, &guid).iter().any(|(g, _)| g.eq_ignore_ascii_case(loc)) {
                return Err(Rejected::new(format!("{name} can't be put in that location")));
            }
            edit::set_text(ch, &guid, "location", loc);
        }
        let mut msg = format!("Added {name}");
        let nuyen_kind = matches!(tag, "gear" | "cyberware" | "bioware" | "armor" | "armormod" | "weapon" | "accessory" | "vehicle" | "mod" | "weaponmount" | "lifestyle" | "drug");
        if ch.created && nuyen_kind {
            let cost = edit::total_cost(ch, store, &guid);
            let parent_tag = purchase.parent.as_ref().and_then(|p| items::find_by_guid_mut(&mut ch.doc, p).map(|e| e.name.clone()));
            if cost > 0.0 {
                career::pay_for_item(ch, tag, parent_tag.as_deref(), &guid, cost).map_err(reject)?;
                msg = format!("Bought {name} for {}", format::nuyen(cost));
            }
        }
        said(msg)
    })
}

fn add_custom_drug(ch: &mut Character, store: &DataStore, name: &str, grade: &str, components: &[(String, i32)]) -> R {
    let comps: Vec<(&str, i32)> = components.iter().map(|(n, l)| (n.as_str(), *l)).collect();
    let name = if name.trim().is_empty() { "Custom Drug" } else { name.trim() };
    let d = items::drug::custom_drug(store, name, grade, &comps, &items::new_guid()).map_err(Rejected::new)?;
    let guid = d.get("guid");
    let name = d.get("name");
    let cost = items::drug::cost_with(Some(store), &d);
    items::drug::add_element(ch, d);
    if ch.created && cost > 0.0 {
        career::pay_for_item(ch, "drug", None, &guid, cost).map_err(reject)?;
    }
    said(format!("Added {name} ({} per dose)", format::nuyen(cost)))
}

/// An adept power's level, refused when the power points run out.
fn set_power_rating(ch: &mut Character, engine: &Engine, store: &DataStore, guid: &str, r: i32) -> R {
    let p = ch.items("powers", "power").into_iter().find(|p| p.get("guid") == guid).cloned().ok_or_else(|| Rejected::new("no such power"))?;
    if r == p.get_i32("rating").unwrap_or(1) {
        return Ok(Done::Unchanged);
    }
    let settings = engine.settings.resolve(&ch.field("settings")).cloned();
    let sheet = engine.sheet(ch);
    let second = settings.as_ref().is_some_and(|s| s.flag("mysadeptsecondmagattribute"));
    let mag = account::adept_mag(ch, &sheet, second);
    let (total, used) = match &settings {
        Some(s) => account::power_points_with(ch, &sheet, s),
        None => account::power_points(ch, &sheet),
    };
    let ignore = ch.flag("ignorerules");
    let cost_now = power::power_point_cost(ch, &p, mag);
    let mut probe = p.clone();
    probe.set_child_text("rating", r.to_string());
    let cost_new = power::power_point_cost(ch, &probe, mag);
    if !ignore && cost_new > cost_now && used - cost_now + cost_new > total + 1e-9 {
        return Err(Rejected::new(format!("Not enough power points for {} level {r}", p.get("name"))));
    }
    power::set_rating(ch, store, guid, r).map_err(Rejected::new)?;
    changed()
}
