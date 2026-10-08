//! Human descriptions of applied commands, for the history panel and the
//! future activity feed: "Raised Pistols to 5 (10 karma)".

use super::Command;
use crate::character::Character;
use crate::engine::Engine;
use crate::format;

/// What `cmd` did, given the character before and after it.
pub(super) fn describe(cmd: &Command, before: &Character, after: &Character, engine: &Engine) -> String {
    let mut s = what(cmd, before, after, engine);
    let karma = after.karma - before.karma;
    let nuyen = after.nuyen - before.nuyen;
    // Only commands that spend or earn mention it; setting the karma or
    // nuyen box already says the new value.
    if !matches!(cmd, Command::SetKarma { .. } | Command::SetNuyen { .. }) {
        let mut cost = Vec::new();
        if karma != 0 {
            cost.push(if karma < 0 { format!("{} karma", -karma) } else { format!("+{karma} karma") });
        }
        if nuyen.abs() > 1e-9 {
            cost.push(if nuyen < 0.0 { format::nuyen(-nuyen) } else { format!("+{}", format::nuyen(nuyen)) });
        }
        if !cost.is_empty() {
            s = format!("{s} ({})", cost.join(", "));
        }
    }
    s
}

/// A short excerpt of a text value.
fn excerpt(v: &str) -> String {
    let line = v.lines().next().unwrap_or("");
    let short: String = line.chars().take(40).collect();
    if short.len() < v.len() {
        format!("“{short}…”")
    } else {
        format!("“{short}”")
    }
}

fn item_name(ch: &Character, guid: &str) -> String {
    let Some(e) = crate::items::edit::find(ch, guid).or_else(|| find_any(&ch.doc, guid)) else { return "an item".into() };
    let name = e.get("name");
    if name.is_empty() { e.name.clone() } else { name }
}

fn find_any<'a>(e: &'a crate::xml::Element, guid: &str) -> Option<&'a crate::xml::Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_any(c, guid))
}

fn skill_name(ch: &Character, engine: &Engine, guid: &str) -> String {
    if let Some(s) = ch.skills.iter().find(|s| s.guid.eq_ignore_ascii_case(guid)) {
        let name = engine.catalog.get(&s.suid).map(|d| d.name.clone()).unwrap_or_else(|| "a skill".into());
        return if s.specific.is_empty() { name } else { format!("{name} ({})", s.specific) };
    }
    ch.knowledge_skills.iter().find(|s| s.guid.eq_ignore_ascii_case(guid)).map(|s| s.name.clone()).unwrap_or_else(|| "a skill".into())
}

fn skill_rating(ch: &Character, guid: &str) -> Option<i32> {
    ch.skills
        .iter()
        .find(|s| s.guid.eq_ignore_ascii_case(guid))
        .map(|s| s.base + s.karma)
        .or_else(|| ch.knowledge_skills.iter().find(|s| s.guid.eq_ignore_ascii_case(guid)).map(|s| s.base + s.karma))
}

fn contact_name(ch: &Character, guid: &str) -> String {
    let n = crate::contacts::of_type(ch, crate::contacts::ContactType::Contact)
        .into_iter()
        .chain(crate::contacts::of_type(ch, crate::contacts::ContactType::Enemy))
        .chain(crate::contacts::of_type(ch, crate::contacts::ContactType::Pet))
        .find(|c| c.get("guid") == guid)
        .map(|c| c.get("name"))
        .unwrap_or_default();
    if n.trim().is_empty() { "a contact".into() } else { n }
}

fn field_label(key: &str) -> String {
    crate::character::INFO_FIELDS
        .iter()
        .chain(crate::character::TEXT_FIELDS)
        .find(|(k, _)| *k == key)
        .map(|(_, l)| (*l).to_owned())
        .unwrap_or_else(|| match key {
            "streetcred" => "Street Cred".into(),
            "notoriety" => "Notoriety".into(),
            "publicawareness" => "Public Awareness".into(),
            "nuyenbp" => "karma for nuyen".into(),
            k => k.to_owned(),
        })
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn what(cmd: &Command, b: &Character, a: &Character, engine: &Engine) -> String {
    use Command::*;
    match cmd {
        SetField { key, value } => format!("Set {} to {}", field_label(key), excerpt(value)),
        SetKarma { value } => format!("Set karma to {value}"),
        SetNuyen { value } => format!("Set nuyen to {}", format::nuyen(*value)),
        SwitchSettings { key } => format!("Switched settings to {}", engine.settings.find(key).map_or_else(|| key.clone(), |s| s.name())),
        SetAttributeBase { attribute, value } => format!("Set {attribute} points to {value}"),
        SetAttributeKarma { attribute, value } => format!("Set {attribute} karma to {value}"),
        SetSkillBase { skill, value } | SetKnowledgeBase { skill, value } => format!("Set {} points to {value}", skill_name(b, engine, skill)),
        SetSkillKarma { skill, value } | SetKnowledgeKarma { skill, value } => format!("Set {} karma to {value}", skill_name(b, engine, skill)),
        SetGroupBase { group, value } => format!("Set {group} group points to {value}"),
        SetGroupKarma { group, value } => format!("Set {group} group karma to {value}"),
        AddKnowledgeSkill { name, .. } => format!("Added knowledge skill {name}"),
        RemoveKnowledgeSkill { skill } => format!("Removed knowledge skill {}", skill_name(b, engine, skill)),
        AddSpecialization { skill, name } | BuySpecialization { skill, name } => format!("Added specialization {name} to {}", skill_name(b, engine, skill)),
        SetTradition { name } => format!("Chose the {name} tradition"),
        AddLifeModule { .. } => "Added a life module".into(),
        FinishCreation => "Finished creation".into(),
        RaiseAttribute { attribute } => {
            let v = a.attribute(attribute).map(|x| x.base + x.karma + x.metatype_min);
            match v {
                Some(v) => format!("Raised {attribute} to {v}"),
                None => format!("Raised {attribute}"),
            }
        }
        RaiseSkill { skill } => match skill_rating(a, skill) {
            Some(r) => format!("Raised {} to {r}", skill_name(b, engine, skill)),
            None => format!("Raised {}", skill_name(b, engine, skill)),
        },
        RaiseSkillGroup { group } => match a.skill_groups.iter().find(|g| g.name == *group) {
            Some(g) => format!("Raised the {group} group to {}", g.rating()),
            None => format!("Raised the {group} group"),
        },
        LearnKnowledgeSkill { name, .. } => format!("Learned {name}"),
        Initiate { .. } => {
            let techno = a.res_enabled() && !a.mag_enabled();
            let grade = a.doc.get_i32(if techno { "submersiongrade" } else { "initiategrade" }).unwrap_or(0);
            if techno { format!("Submerged to grade {grade}") } else { format!("Initiated to grade {grade}") }
        }
        UndoExpense { entry } => {
            let reason = crate::career::entries(b).into_iter().find(|e| e.guid == *entry).map(|e| e.reason).unwrap_or_default();
            if reason.is_empty() { "Undid a ledger entry".into() } else { format!("Undid “{reason}”") }
        }
        ManualExpense { karma, gain, expense } => {
            let what = if *karma { "karma" } else { "nuyen" };
            let verb = if *gain { "Gained" } else { "Spent" };
            if expense.reason.is_empty() { format!("{verb} {what}") } else { format!("{verb} {what}: {}", expense.reason) }
        }
        SpendEdge => "Spent a point of Edge".into(),
        RegainEdge => "Regained a point of Edge".into(),
        BurnEdge => "Burnt a point of Edge".into(),
        SetEdgeUsed { used } => format!("Marked {used} Edge spent"),
        RefreshEdge => "Refreshed Edge".into(),
        BurnStreetCred => "Burnt Street Cred".into(),
        SetGroupMember { member } => if *member { "Joined a group".into() } else { "Left a group".into() },
        AddItem { record, purchase, .. } => {
            let mut n = record.name.clone();
            if purchase.rating > 0 {
                n = format!("{n} {}", purchase.rating);
            }
            if let Some(x) = purchase.answer.as_deref().filter(|x| !x.is_empty()) {
                n = format!("{n} ({x})");
            }
            format!("Added {n}")
        }
        RemoveItem { guid, .. } | DeleteItem { guid } | RemoveAiProgram { guid } => format!("Removed {}", item_name(b, guid)),
        SellItem { guid, .. } => format!("Sold {}", item_name(b, guid)),
        SetItemRating { guid, rating } => format!("Set {} to rating {rating}", item_name(b, guid)),
        SetItemQuantity { guid, qty } => format!("Set {} quantity to {}", item_name(b, guid), crate::improvement::fmt_num(*qty)),
        SetItemEquipped { guid, on } => format!("{} {}", if *on { "Equipped" } else { "Unequipped" }, item_name(b, guid)),
        SetItemWireless { guid, on } => format!("Turned wireless {} for {}", on_off(*on), item_name(b, guid)),
        SetItemText { guid, field, value } => format!("Set {} of {} to {}", if field == "notes" { "notes" } else { field.as_str() }, item_name(b, guid), excerpt(value)),
        AddItemLocation { guid, name } => format!("Moved {} to new location {name}", item_name(b, guid)),
        AddWeaponMount { vehicle, .. } => format!("Added a weapon mount to {}", item_name(b, vehicle)),
        AddCustomDrug { name, .. } => format!("Added drug {}", if name.trim().is_empty() { "Custom Drug" } else { name.trim() }),
        ApplyKit { kit } => {
            let name = crate::xml::parse(kit).map(|k| k.get("name")).unwrap_or_default();
            format!("Added kit {name}")
        }
        UpdateLifestyle { guid, .. } => format!("Changed lifestyle {}", item_name(b, guid)),
        PayLifestyleMonth { guid } => format!("Paid a month of {}", item_name(b, guid)),
        AddLifestyleQuality { lifestyle, quality, .. } => format!("Added {quality} to {}", item_name(b, lifestyle)),
        RemoveLifestyleQuality { lifestyle, quality } => format!("Removed {} from {}", item_name(b, quality), item_name(b, lifestyle)),
        AddSpell { record, .. } => format!("Added spell {}", record.name),
        AddCustomSpell { design } => format!("Created spell {}", design.name),
        QuickenSpell { spell, karma } => format!("Quickened {} ({karma} karma)", item_name(b, spell)),
        SetMentorChoices { mentor, .. } => format!("Set choices of {}", item_name(b, mentor)),
        ChooseMentor { name, .. } => format!("Chose mentor {name}"),
        AddMetamagic { name, .. } => format!("Added {name}"),
        LearnTechnique { technique, .. } => format!("Learned {technique}"),
        BuyPowerPoint => "Bought a power point".into(),
        SetPowerRating { power, rating } => format!("Set {} to level {rating}", item_name(b, power)),
        SetSpiritState { spirit, .. } => format!("Changed {}", item_name(b, spirit)),
        BindFocus { gear } => format!("Bound {}", item_name(b, gear)),
        UnbindFocus { gear } => format!("Unbound {}", item_name(b, gear)),
        CreateImprovement { form, edit, .. } => {
            let n = if form.name.is_empty() { form.type_id.clone() } else { form.name.clone() };
            if edit.is_some() { format!("Edited improvement {n}") } else { format!("Added improvement {n}") }
        }
        RemoveImprovement { .. } => "Removed an improvement".into(),
        SetImprovementEnabled { on, .. } => format!("Turned an improvement {}", on_off(*on)),
        SetImprovementNotes { .. } => "Changed improvement notes".into(),
        SetImprovementGroup { group, .. } => format!("Moved an improvement to {}", if group.is_empty() { "Selected Improvements" } else { group }),
        SetImprovementGroupEnabled { group, on } => format!("Turned group {} {}", if group.is_empty() { "Selected Improvements" } else { group }, on_off(*on)),
        AddImprovementGroup { name } => format!("Added improvement group {name}"),
        RenameImprovementGroup { old, new } => format!("Renamed improvement group {old} to {new}"),
        RemoveImprovementGroup { name } => format!("Removed improvement group {name}"),
        AddContact { kind } => format!("Added {}", match kind {
            crate::contacts::ContactType::Contact => "a contact",
            crate::contacts::ContactType::Enemy => "an enemy",
            crate::contacts::ContactType::Pet => "a pet",
        }),
        ImportContacts { .. } => "Imported contacts".into(),
        SetContactField { contact, key, value } => format!("Set {key} of {} to {}", contact_name(b, contact), excerpt(value)),
        SetContactNotes { contact, .. } => format!("Changed notes of {}", contact_name(b, contact)),
        MoveContact { contact, .. } | MoveContactStep { contact, .. } => format!("Moved {}", contact_name(b, contact)),
        SortContacts { kind, by } => {
            let what = match kind {
                crate::contacts::ContactType::Contact => "contacts",
                crate::contacts::ContactType::Enemy => "enemies",
                crate::contacts::ContactType::Pet => "pets",
            };
            format!("Sorted {what} by {}", by.label().to_lowercase())
        }
        RemoveContact { contact } => format!("Removed {}", contact_name(b, contact)),
        LinkContact { contact, file, .. } => format!("Linked {} to {file}", contact_name(b, contact)),
        UnlinkContact { contact } => format!("Unlinked {}", contact_name(b, contact)),
        AddWeek => "Added a calendar week".into(),
        SetWeekNotes { notes, .. } => format!("Set week notes to {}", excerpt(notes)),
        RemoveWeek { .. } => "Removed a calendar week".into(),
        SetPhysicalDamage { filled } => format!("Set physical damage to {filled}"),
        SetStunDamage { filled } => format!("Set stun damage to {filled}"),
        SetActiveClip { weapon, slot } => format!("Switched {} to clip {slot}", item_name(b, weapon)),
        Fire { weapon, .. } => format!("Fired {}", item_name(b, weapon)),
        SetAmmoRemaining { weapon, count } => format!("Set {} ammo to {count}", item_name(b, weapon)),
        Reload { weapon, .. } => format!("Reloaded {}", item_name(b, weapon)),
        Unload { weapon } => format!("Unloaded {}", item_name(b, weapon)),
        SetCharges { weapon, count } => format!("Set {} charges to {count}", item_name(b, weapon)),
        SetVehicleDamage { vehicle, filled } => format!("Set {} damage to {filled}", item_name(b, vehicle)),
        SetMatrixDamage { device, filled } => format!("Set {} matrix damage to {filled}", item_name(b, device)),
        SetActiveCommlink { device, on } => format!("{} {}", if *on { "Activated" } else { "Deactivated" }, item_name(b, device)),
        SetHomeNode { device, on } => format!("{} {} as home node", if *on { "Set" } else { "Unset" }, item_name(b, device)),
        Revert { what, .. } => format!("Reverted: {what}"),
    }
}
