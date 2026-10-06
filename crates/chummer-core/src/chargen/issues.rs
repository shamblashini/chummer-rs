//! What is wrong or unfinished with a character in creation, as a list the
//! GUI can show all the time instead of only when finishing.
//!
//! Errors are the checks of `CharacterCreate.CheckCharacterValidity` (they
//! block finishing creation); warnings are the "are you sure?" prompts it
//! shows for unspent points, karma and nuyen above the carry-over, plus
//! choices still pending (mentor spirit, stream). Each issue names the
//! part of the character it concerns ([`Area`]), which maps to a GUI tab
//! ([`IssueTab`]), and the item it concerns when there is one.

use crate::calc::Sheet;
use crate::character::Character;
use crate::data::DataStore;
use crate::expr::{Availability, NoAttributes};
use crate::settings::CharacterSettings;
use crate::xml::Element;

use super::{karma, Budget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Blocks finishing creation.
    Error,
    /// Allowed, but probably not what the player wants (points left over).
    Warning,
    /// Something still to fill in that no rule requires.
    Info,
}

/// The part of the character an issue concerns. Finer than the tabs so a
/// step-by-step guide can tell attributes from qualities on one tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    Metatype,
    Attributes,
    SpecialAttributes,
    Qualities,
    LifeModules,
    ActiveSkills,
    SkillGroups,
    KnowledgeSkills,
    MartialArts,
    Spells,
    AdeptPowers,
    ComplexForms,
    Cyberware,
    Gear,
    Armor,
    Weapons,
    Lifestyles,
    Vehicles,
    Contacts,
    CharacterInfo,
    /// The karma total as a whole.
    Karma,
    /// Starting nuyen as a whole.
    Nuyen,
}

/// The character tabs, named after Chummer's, without any UI type. The
/// GUI maps these to its own tab enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueTab {
    Common,
    Skills,
    MartialArts,
    Magician,
    Adept,
    Technomancer,
    Cyberware,
    StreetGear,
    Vehicles,
    Relationships,
    CharacterInfo,
}

impl Area {
    /// The tab that shows this area; `None` for character-wide totals
    /// (karma, nuyen), which belong to the Karma Summary.
    pub fn tab(self) -> Option<IssueTab> {
        Some(match self {
            Area::Metatype | Area::Attributes | Area::SpecialAttributes | Area::Qualities | Area::LifeModules => IssueTab::Common,
            Area::ActiveSkills | Area::SkillGroups | Area::KnowledgeSkills => IssueTab::Skills,
            Area::MartialArts => IssueTab::MartialArts,
            Area::Spells => IssueTab::Magician,
            Area::AdeptPowers => IssueTab::Adept,
            Area::ComplexForms => IssueTab::Technomancer,
            Area::Cyberware => IssueTab::Cyberware,
            Area::Gear | Area::Armor | Area::Weapons | Area::Lifestyles => IssueTab::StreetGear,
            Area::Vehicles => IssueTab::Vehicles,
            Area::Contacts => IssueTab::Relationships,
            Area::CharacterInfo => IssueTab::CharacterInfo,
            Area::Karma | Area::Nuyen => return None,
        })
    }
}

/// What kind of problem. Each kind has one message template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueKind {
    AttributePointsOver,
    AttributePointsLeft,
    SpecialPointsOver,
    SpecialPointsLeft,
    TooManyAttributesAtMax,
    SkillPointsOver,
    SkillPointsLeft,
    SkillGroupPointsOver,
    SkillGroupPointsLeft,
    KnowledgePointsLeft,
    MultipleSpecializations,
    TooManyNativeLanguages,
    NativeLanguagesLeft,
    KarmaOver,
    KarmaCarryOver,
    NuyenOver,
    NuyenCarryOver,
    PositiveQualityLimit,
    NegativeQualityLimit,
    MentorPending,
    PowerPointsOver,
    PowerPointsLeft,
    FreeSpellsLeft,
    NoTradition,
    NoStream,
    EssenceTooLow,
    MartialArtsCount,
    TechniquesCount,
    HighContact,
    ContactPointsLeft,
    AvailabilityTooHigh,
    RestrictedGearUsed,
    BannedGrade,
    OverCapacity,
    NoMetatype,
    NoAlias,
}

impl IssueKind {
    /// English message with Chummer's `{0}` placeholders. Chummer's own
    /// en-us text where it has a whole sentence for it, so translations
    /// apply; short plain wording otherwise.
    pub fn template(self) -> &'static str {
        use IssueKind::*;
        match self {
            AttributePointsOver => "{0} over allotted Attribute point limit",
            AttributePointsLeft => "{0} Attribute points left to spend",
            SpecialPointsOver => "{0} over allotted Special Attribute point limit",
            SpecialPointsLeft => "{0} Special Attribute points left to spend",
            TooManyAttributesAtMax => "{0} Attribute(s) is/are at their metatype maximum when only {1} is/are allowed.",
            SkillPointsOver => "{0} over allotted Active Skill point limit",
            SkillPointsLeft => "{0} Active Skill points left to spend",
            SkillGroupPointsOver => "{0} over allotted Skill Group point limit",
            SkillGroupPointsLeft => "{0} Skill Group points left to spend",
            KnowledgePointsLeft => "{0} Knowledge Skill points left to spend",
            MultipleSpecializations => "{0} has more than one specialization",
            TooManyNativeLanguages => "Too many Language Knowledge Skills listed as Native. Current: {0} Maximum: {1}",
            NativeLanguagesLeft => "{0} more native language(s) can be chosen",
            KarmaOver => "{0} Karma overspent",
            KarmaCarryOver => "{0} Karma left, only {1} carries over to career mode",
            NuyenOver => "{0} over your Nuyen total",
            NuyenCarryOver => "{0} left, only {1} carries over to career mode",
            PositiveQualityLimit => "Positive qualities cost {0} Karma, the limit is {1}",
            NegativeQualityLimit => "Negative qualities give {0} Karma, the limit is {1}",
            MentorPending => "{0}: no mentor spirit chosen yet",
            PowerPointsOver => "{0} over your Adept Power Point maximum ({1})",
            PowerPointsLeft => "{0} Power Points left to spend",
            FreeSpellsLeft => "{0} free spells left to choose",
            NoTradition => "No Magic Tradition has been selected",
            NoStream => "No Technomancer Stream has been selected",
            EssenceTooLow => "{0} over the Essence limit",
            MartialArtsCount => "{0} Martial Arts, only {1} allowed",
            TechniquesCount => "{0} martial arts techniques, only {1} allowed",
            HighContact => "You cannot have a contact worth more than 7 points at character generation.",
            ContactPointsLeft => "{0} free Contact points left to spend",
            AvailabilityTooHigh => "{0}: Availability {1} is above the allowed {2}",
            RestrictedGearUsed => "{0} uses Restricted Gear (Availability {1})",
            BannedGrade => "{0}: grade {1} is not allowed",
            OverCapacity => "{0} is over its capacity ({1} of {2})",
            NoMetatype => "No metatype has been selected",
            NoAlias => "No street name (alias) yet",
        }
    }

    pub const ALL: [IssueKind; 36] = {
        use IssueKind::*;
        [
            AttributePointsOver, AttributePointsLeft, SpecialPointsOver, SpecialPointsLeft, TooManyAttributesAtMax, SkillPointsOver, SkillPointsLeft,
            SkillGroupPointsOver, SkillGroupPointsLeft, KnowledgePointsLeft, MultipleSpecializations, TooManyNativeLanguages, NativeLanguagesLeft,
            KarmaOver, KarmaCarryOver, NuyenOver, NuyenCarryOver, PositiveQualityLimit, NegativeQualityLimit, MentorPending, PowerPointsOver,
            PowerPointsLeft, FreeSpellsLeft, NoTradition, NoStream, EssenceTooLow, MartialArtsCount, TechniquesCount, HighContact,
            ContactPointsLeft, AvailabilityTooHigh, RestrictedGearUsed, BannedGrade, OverCapacity, NoMetatype, NoAlias,
        ]
    };
}

/// Every message template, for translation coverage.
pub fn templates() -> Vec<&'static str> {
    IssueKind::ALL.iter().map(|k| k.template()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub kind: IssueKind,
    pub area: Area,
    /// GUID of the item (skill, quality, gear, contact...) it concerns.
    pub item: Option<String>,
    /// Values for the template's placeholders.
    pub args: Vec<String>,
}

impl Issue {
    fn new(severity: Severity, kind: IssueKind, area: Area, args: Vec<String>) -> Issue {
        Issue { severity, kind, area, item: None, args }
    }
    fn on(mut self, guid: impl Into<String>) -> Issue {
        let g = guid.into();
        self.item = (!g.is_empty()).then_some(g);
        self
    }
    pub fn template(&self) -> &'static str {
        self.kind.template()
    }
    /// The English message.
    pub fn message(&self) -> String {
        let mut out = self.template().to_owned();
        for (i, a) in self.args.iter().enumerate() {
            out = out.replace(&format!("{{{i}}}"), a);
        }
        out
    }
    pub fn tab(&self) -> Option<IssueTab> {
        self.area.tab()
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Issues of a character in creation; empty in career mode or with
/// "Ignore Rules". Sorted errors first.
pub fn issues(ch: &Character, b: &Budget, sheet: &Sheet, settings: &CharacterSettings, store: Option<&DataStore>) -> Vec<Issue> {
    use IssueKind::*;
    use Severity::*;
    let mut out = Vec::new();
    if ch.created || ch.flag("ignorerules") {
        return out;
    }
    let n = |v: i32| v.to_string();
    // Point pools: overspent is an error, unspent a warning (only for pools
    // the build method gives; karma builds have none).
    let mut pool = |p: (i32, i32), over: IssueKind, left: IssueKind, area: Area| {
        let rest = Budget::left(p);
        if rest < 0 {
            out.push(Issue::new(Error, over, area, vec![n(-rest)]));
        } else if rest > 0 && p.0 > 0 {
            out.push(Issue::new(Warning, left, area, vec![n(rest)]));
        }
    };
    pool(b.attribute_points, AttributePointsOver, AttributePointsLeft, Area::Attributes);
    pool(b.special_points, SpecialPointsOver, SpecialPointsLeft, Area::SpecialAttributes);
    pool(b.skill_points, SkillPointsOver, SkillPointsLeft, Area::ActiveSkills);
    pool(b.skill_group_points, SkillGroupPointsOver, SkillGroupPointsLeft, Area::SkillGroups);
    let kno_left = Budget::left(b.knowledge_points);
    if kno_left > 0 {
        out.push(Issue::new(Warning, KnowledgePointsLeft, Area::KnowledgeSkills, vec![n(kno_left)]));
    }

    if ch.field("metatype").trim().is_empty() {
        out.push(Issue::new(Error, NoMetatype, Area::Metatype, vec![]));
    }

    // Attributes at their natural maximum.
    let at_max = ch
        .attributes
        .iter()
        .filter(|a| a.category == "Standard" && a.metatype_max > 0)
        .filter(|a| a.metatype_min + a.base + a.karma >= a.metatype_max)
        .count() as i32;
    let allowed = settings.int("maxnumbermaxattributescreate", 1);
    if at_max > allowed {
        out.push(Issue::new(Error, TooManyAttributesAtMax, Area::Attributes, vec![n(at_max), n(allowed)]));
    }

    // Specializations: one per skill at creation.
    let skill_name = |suid: &str| {
        ch.doc.child("newskills").and_then(|n| n.child("skills")).and_then(|s| s.elements().find(|e| e.get("suid").eq_ignore_ascii_case(suid))).map(|e| e.get("name")).unwrap_or_default()
    };
    for s in ch.skills.iter().filter(|s| s.specs.iter().filter(|x| !x.free).count() > 1) {
        out.push(Issue::new(Error, MultipleSpecializations, Area::ActiveSkills, vec![skill_name(&s.suid)]).on(&s.guid));
    }
    for k in ch.knowledge_skills.iter().filter(|k| k.specs.len() > 1) {
        out.push(Issue::new(Error, MultipleSpecializations, Area::KnowledgeSkills, vec![k.name.clone()]).on(&k.guid));
    }

    // Native languages.
    let natives = ch.knowledge_skills.iter().filter(|k| k.native_language).count() as i32;
    let native_limit = 1 + crate::expr::standard_round(ch.improvements.val("NativeLanguageLimit", None));
    if natives > native_limit {
        out.push(Issue::new(Error, TooManyNativeLanguages, Area::KnowledgeSkills, vec![n(natives), n(native_limit)]));
    } else if natives < native_limit {
        out.push(Issue::new(Info, NativeLanguagesLeft, Area::KnowledgeSkills, vec![n(native_limit - natives)]));
    }

    // Karma and nuyen.
    let karma_left = b.karma_left();
    let carry_k = settings.karma("karmacarryover", 7).max(0);
    if karma_left < 0 {
        out.push(Issue::new(Error, KarmaOver, Area::Karma, vec![n(-karma_left)]));
    } else if karma_left > carry_k {
        out.push(Issue::new(Warning, KarmaCarryOver, Area::Karma, vec![n(karma_left), n(carry_k)]));
    }
    let nuyen_left = b.nuyen_left();
    let carry_n = f64::from(settings.int("nuyencarryover", 5000));
    if nuyen_left < -1e-9 {
        out.push(Issue::new(Error, NuyenOver, Area::Nuyen, vec![crate::format::nuyen(-nuyen_left)]));
    } else if nuyen_left > carry_n + 1e-9 {
        out.push(Issue::new(Warning, NuyenCarryOver, Area::Nuyen, vec![crate::format::nuyen(nuyen_left), crate::format::nuyen(carry_n)]));
    }

    // Qualities.
    if b.positive_quality_karma > b.quality_limit {
        out.push(Issue::new(Error, PositiveQualityLimit, Area::Qualities, vec![n(b.positive_quality_karma), n(b.quality_limit)]));
    }
    if b.negative_quality_karma > b.quality_limit {
        out.push(Issue::new(Error, NegativeQualityLimit, Area::Qualities, vec![n(b.negative_quality_karma), n(b.quality_limit)]));
    }
    for q in ch.items("qualities", "quality") {
        if let Some(kind) = q.child("bonus").and_then(|bn| bn.elements().find(|e| e.name == "selectmentorspirit" || e.name == "selectparagon")).map(|e| if e.name == "selectparagon" { "Paragon" } else { "MentorSpirit" }) {
            let guid = q.get("guid");
            if !ch.improvements.list.iter().any(|i| i.kind == kind && i.source_name.eq_ignore_ascii_case(&guid)) {
                out.push(Issue::new(Warning, MentorPending, Area::Qualities, vec![q.get("name")]).on(guid));
            }
        }
    }

    // Magic and resonance.
    if let Some((total, used)) = b.power_points {
        if used > total + 1e-9 {
            out.push(Issue::new(Error, PowerPointsOver, Area::AdeptPowers, vec![fmt_num(used - total), fmt_num(total)]));
        } else if total - used > 1e-9 {
            out.push(Issue::new(Warning, PowerPointsLeft, Area::AdeptPowers, vec![fmt_num(total - used)]));
        }
    }
    let free_left = Budget::left(b.free_spells);
    if free_left > 0 {
        let area = if ch.is_technomancer() && !ch.is_magician() { Area::ComplexForms } else { Area::Spells };
        out.push(Issue::new(Warning, FreeSpellsLeft, area, vec![n(free_left)]));
    }
    // Files from before 5.190 keep the tradition's name as the element text.
    let tradition = ch.doc.child("tradition").map(|t| if t.child("name").is_some() { t.get("name") } else { t.text() }).unwrap_or_default();
    if ch.is_magician() && ch.mag_enabled() && tradition.trim().is_empty() {
        out.push(Issue::new(Error, NoTradition, Area::Spells, vec![]));
    }
    // An error in Chummer; a warning here because the GUI has no stream
    // picker yet, so it must not block finishing.
    if ch.is_technomancer() && ch.res_enabled() && ch.doc.get("stream").trim().is_empty() && ch.doc.child("tradition").is_none_or(|t| t.get("traditiontype") != "RES") {
        out.push(Issue::new(Warning, NoStream, Area::ComplexForms, vec![]));
    }

    // Essence above zero (to the displayed precision).
    if ch.attribute("ESS").is_none_or(|a| a.metatype_max > 0) {
        let min = 10f64.powi(-(settings.essence_decimals() as i32));
        if sheet.essence < min - 1e-9 {
            out.push(Issue::new(Error, EssenceTooLow, Area::Cyberware, vec![crate::format::essence(min - sheet.essence, settings.essence_decimals())]));
        }
    }

    // Martial arts.
    let arts = ch.items("martialarts", "martialart");
    let own_arts = arts.iter().filter(|a| !a.get_bool("isquality").unwrap_or(false)).count() as i32;
    let max_arts = settings.int("maximummartialarts", 1);
    if own_arts > max_arts {
        out.push(Issue::new(Error, MartialArtsCount, Area::MartialArts, vec![n(own_arts), n(max_arts)]));
    }
    let techniques: i32 = arts.iter().map(|a| a.child("martialarttechniques").or_else(|| a.child("techniques")).map_or(0, |t| t.elements().count() as i32)).sum();
    let max_tech = settings.int("maximummartialtechniques", 5);
    if techniques > max_tech {
        out.push(Issue::new(Error, TechniquesCount, Area::MartialArts, vec![n(techniques), n(max_tech)]));
    }

    // Contacts.
    let high = ch.items("contacts", "contact").into_iter().filter(|c| crate::contacts::ContactType::of(c) == crate::contacts::ContactType::Contact).find(|c| karma::contact_points(ch, c) > 7);
    if let Some(c) = high {
        if !ch.improvements.has("FriendsInHighPlaces") {
            out.push(Issue::new(Error, HighContact, Area::Contacts, vec![]).on(c.get("guid")));
        }
    }
    pool_left(&mut out, b.contact_points, ContactPointsLeft, Area::Contacts);

    // Gear: availability, banned ware grades, capacity.
    gear_checks(ch, settings, store, &mut out);

    if ch.field("alias").trim().is_empty() {
        out.push(Issue::new(Info, NoAlias, Area::CharacterInfo, vec![]));
    }

    out.sort_by_key(|i| i.severity);
    out
}

fn pool_left(out: &mut Vec<Issue>, p: (i32, i32), kind: IssueKind, area: Area) {
    let rest = Budget::left(p);
    if rest > 0 && p.0 > 0 {
        out.push(Issue::new(Severity::Warning, kind, area, vec![rest.to_string()]));
    }
}

fn fmt_num(v: f64) -> String {
    crate::improvement::fmt_num((v * 100.0).round() / 100.0)
}

/// The containers whose items have availability, and their area.
const GEAR_LISTS: [(&str, &str, Area); 5] = [
    ("gears", "gear", Area::Gear),
    ("cyberwares", "cyberware", Area::Cyberware),
    ("armors", "armor", Area::Armor),
    ("weapons", "weapon", Area::Weapons),
    ("vehicles", "vehicle", Area::Vehicles),
];

/// Child lists that hold sub-items with their own availability.
const CHILD_LISTS: [&str; 7] = ["children", "gears", "armormods", "accessories", "mods", "cyberwares", "weapons"];

fn included(e: &Element) -> bool {
    e.get_bool("includedinparent").unwrap_or(false) || e.get_bool("included").unwrap_or(false)
}

/// Own availability of a saved item; `None` when it has none or it needs
/// context we cannot evaluate.
fn own_avail(ch: &Character, store: Option<&DataStore>, e: &Element) -> Option<Availability> {
    let v = e.get("avail");
    if v.trim().is_empty() {
        return None;
    }
    if e.name == "cyberware" {
        if let Some(st) = store {
            return Some(crate::items::cyberware::availability(ch, st, e));
        }
    }
    if v.contains('{') || v.contains("Parent") || v.contains("Gear") {
        return None;
    }
    let rating = e.get_i32("rating").unwrap_or(0);
    let min = e.get_i32("minrating").unwrap_or(0);
    Some(Availability::parse(&v, rating, min, &NoAttributes))
}

/// `TotalAvail`: own value plus the `+N` of children that add to it.
fn total_avail(ch: &Character, store: Option<&DataStore>, e: &Element) -> Option<Availability> {
    let mut a = own_avail(ch, store, e)?;
    if a.add_to_parent {
        return Some(a);
    }
    for list in CHILD_LISTS {
        for c in e.child(list).into_iter().flat_map(Element::elements) {
            if included(c) {
                continue;
            }
            if let Some(ca) = own_avail(ch, store, c).filter(|x| x.add_to_parent) {
                a.value += ca.value;
            }
        }
    }
    Some(a)
}

fn gear_checks(ch: &Character, settings: &CharacterSettings, store: Option<&DataStore>, out: &mut Vec<Issue>) {
    use IssueKind::*;
    let max = settings.max_availability();
    // Restricted Gear: (availability allowed, how many items).
    let mut restricted: Vec<(i32, i32)> = ch.improvements.of_kind("RestrictedGear").map(|i| (crate::expr::standard_round(i.val), i.rating)).filter(|(_, c)| *c > 0).collect();
    restricted.sort();
    let banned: Vec<String> = settings.raw.child("bannedwaregrades").map(|b| b.children_named("grade").map(Element::text).filter(|g| !g.is_empty()).collect()).unwrap_or_default();
    let enforce_capacity = settings.flag("enforcecapacity");
    let mut stack: Vec<(&Element, Area, bool)> = Vec::new();
    for (container, item, area) in GEAR_LISTS {
        for e in ch.items(container, item) {
            stack.push((e, area, true));
        }
    }
    while let Some((e, area, top)) = stack.pop() {
        for list in CHILD_LISTS {
            for c in e.child(list).into_iter().flat_map(Element::elements) {
                stack.push((c, area, false));
            }
        }
        if included(e) {
            continue;
        }
        let guid = e.get("guid");
        let name = e.get("name");
        if let Some(a) = total_avail(ch, store, e).filter(|a| !a.add_to_parent && a.value > max) {
            let qty = crate::expr::standard_round(e.get_f64("qty").unwrap_or(1.0)).max(1);
            match restricted.iter_mut().find(|(avail, count)| *avail >= a.value && *count >= qty) {
                Some(slot) => {
                    slot.1 -= qty;
                    out.push(Issue::new(Severity::Info, RestrictedGearUsed, area, vec![name.clone(), a.to_string()]).on(&guid));
                }
                None => out.push(Issue::new(Severity::Error, AvailabilityTooHigh, area, vec![name.clone(), a.to_string(), max.to_string()]).on(&guid)),
            }
        }
        if e.name == "cyberware" && top {
            let grade = e.get("grade");
            if !grade.is_empty() && banned.iter().any(|b| grade.contains(b.as_str())) {
                out.push(Issue::new(Severity::Error, BannedGrade, area, vec![name.clone(), grade]).on(&guid));
            }
        }
        if enforce_capacity {
            if let Some((used, total)) = capacity(e) {
                if used > total + 1e-9 {
                    out.push(Issue::new(Severity::Error, OverCapacity, area, vec![name, fmt_num(used), fmt_num(total)]).on(&guid));
                }
            }
        }
    }
}

/// What a capacity string consumes of its parent: `[m]` or `n/[m]`, with
/// `Capacity` (the parent's) and `Rating` evaluated. `None` when it cannot
/// be evaluated.
fn consumed(s: &str, rating: i32, parent: f64) -> Option<f64> {
    let s = s.trim();
    let part = match s.split_once("/[") {
        Some((_, b)) => b,
        None if s.starts_with('[') => &s[1..],
        None => return Some(0.0),
    };
    let t = part.trim_end_matches(']').trim();
    if t.is_empty() || t == "*" {
        return Some(0.0);
    }
    let t = crate::expr::fixed_values(t, rating).replace("Capacity", &parent.to_string()).replace("Rating", &rating.to_string());
    if crate::expr::needs_evaluation(&t) {
        crate::expr::evaluate_num(&t).ok()
    } else {
        crate::expr::parse_plain(&t)
    }
}

/// (used, total) capacity of gear, ware and armor (`CapacityRemaining`);
/// `None` when the item has none or a child's cost cannot be evaluated.
fn capacity(e: &Element) -> Option<(f64, f64)> {
    let rating = e.get_i32("rating").unwrap_or(0);
    let (field, lists): (&str, &[(&str, &str)]) = match e.name.as_str() {
        "gear" => ("capacity", &[("children", "capacity")]),
        "cyberware" => ("capacity", &[("children", "capacity"), ("gears", "capacity")]),
        "armor" => ("armorcapacity", &[("armormods", "armorcapacity"), ("gears", "armorcapacity")]),
        "armormod" => ("gearcapacity", &[("gears", "armorcapacity")]),
        _ => return None,
    };
    let total = crate::items::edit::parse_capacity(&e.get(field), rating).0;
    if total <= 0.0 {
        return None;
    }
    let mut used = 0.0;
    for (list, f) in lists {
        for c in e.child(list).into_iter().flat_map(Element::elements) {
            used += consumed(&c.get(f), c.get_i32("rating").unwrap_or(0), total)?;
        }
    }
    Some((used, total))
}

/// Issues per tab: (errors, warnings, infos).
pub fn count_for(issues: &[Issue], tab: IssueTab) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for i in issues.iter().filter(|i| i.tab() == Some(tab)) {
        match i.severity {
            Severity::Error => c.0 += 1,
            Severity::Warning => c.1 += 1,
            Severity::Info => c.2 += 1,
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::consumed;

    #[test]
    fn capacity_costs_evaluate_parent_capacity() {
        assert_eq!(consumed("[5]", 6, 3.0), Some(5.0));
        assert_eq!(consumed("2/[1]", 0, 3.0), Some(1.0));
        assert_eq!(consumed("[*]", 0, 3.0), Some(0.0));
        assert_eq!(consumed("4", 0, 3.0), Some(0.0), "provides, consumes nothing");
        assert_eq!(consumed("[Rating]", 3, 6.0), Some(3.0));
        // YNT Softweave frees half the armor's capacity, rounded up.
        assert_eq!(consumed("[-(Capacity * 0.5 + 0.5*number((Capacity mod 2) = 1))]", 0, 3.0), Some(-2.0));
    }
}
