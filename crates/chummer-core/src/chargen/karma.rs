//! Creation karma spent, by category (`CharacterCreate.CalculateBP`).
//!
//! Not ported (no creation fixture exercises them): quality `<costdiscount>`
//! nodes, Mastery qualities bought with spell points, stacked foci and the
//! `CompensateSkillGroupKarmaDifference` house rule.

use crate::calc::{Rules, Sheet};
use crate::character::Character;
use crate::data::DataStore;
use crate::expr::standard_round;
use crate::settings::CharacterSettings;
use crate::xml::Element;

/// Karma spent at creation, one entry per `CalculateBP` section, in the
/// order Chummer subtracts them.
pub fn karma_breakdown(ch: &Character, sheet: &Sheet, rules: &Rules, settings: &CharacterSettings, _store: Option<&DataStore>) -> Vec<(&'static str, i32)> {
    let q = quality_karma(ch, rules, settings);
    let pp = mystic_adept_power_points(ch, settings);
    vec![
        ("metatype", ch.doc.get_i32("metatypebp").unwrap_or(0)),
        ("contacts", contact_karma(ch, sheet)),
        ("qualities", q.positive - q.negative + q.life_modules + q.metagenic_balance),
        ("attributes", sheet.attribute_karma_spent),
        ("martial arts", martial_arts_karma(ch, settings)),
        ("skill groups", sheet.skill_group_karma_spent),
        ("skills", sheet.skill_karma_spent),
        ("nuyen", ch.doc.get_i32("nuyenbp").unwrap_or(0)),
        ("spells", crate::items::magic::account::spell_karma_with_extra(ch, sheet, rules, pp.spells)),
        ("power points", pp.karma),
        ("foci", foci_karma(ch, settings)),
        ("spirits", spirit_karma(ch, settings)),
        ("forms", crate::items::magic::complex_form_karma(ch, rules)),
        ("programs", crate::items::aiprogram::creation_karma(ch, rules)),
        ("initiation", initiation_karma(ch, rules, settings)),
        ("critter powers", ch.items("critterpowers", "critterpower").iter().map(|p| p.get_i32("karma").unwrap_or(0)).sum()),
    ]
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

fn improved_for(ch: &Character, kind: &str, c: &Element) -> bool {
    let guid = c.get("guid");
    ch.improvements.of_kind(kind).any(|i| i.improved_name.eq_ignore_ascii_case(&guid))
}

/// `Contact.Free`: flagged free or made free by a ContactMakeFree improvement.
fn contact_free(ch: &Character, c: &Element) -> bool {
    c.get_bool("free").unwrap_or(false) || improved_for(ch, "ContactMakeFree", c)
}

/// `Contact.IsGroup`, including ContactForceGroup improvements.
fn contact_is_group(ch: &Character, c: &Element) -> bool {
    c.get_bool("group").unwrap_or(false) || improved_for(ch, "ContactForceGroup", c)
}

/// `Contact.Loyalty`: the highest ContactForcedLoyalty, else 1 for groups.
fn contact_loyalty(ch: &Character, c: &Element) -> i32 {
    let guid = c.get("guid");
    let forced = ch.improvements.of_kind("ContactForcedLoyalty").filter(|i| i.improved_name.eq_ignore_ascii_case(&guid)).map(|i| standard_round(i.val)).max().unwrap_or(0);
    if forced > 0 {
        forced
    } else if contact_is_group(ch, c) {
        1
    } else {
        c.get_i32("loyalty").unwrap_or(1)
    }
}

/// `Contact.Connection`, capped by `ConnectionMaximum` (6 at creation,
/// 12 with Friends in High Places).
fn contact_connection(ch: &Character, c: &Element) -> i32 {
    let max = if ch.created || friends_in_high_places(ch) { 12 } else { 6 };
    c.get_i32("connection").unwrap_or(1).min(max)
}

/// `Character.FriendsInHighPlaces`.
fn friends_in_high_places(ch: &Character) -> bool {
    ch.improvements.has("FriendsInHighPlaces")
}

/// `Contact.ContactPoints`: connection + loyalty, +1 family, +2 blackmail,
/// with ContactKarmaDiscount and the ContactKarmaMinimum floor.
pub fn contact_points(ch: &Character, c: &Element) -> i32 {
    if contact_free(ch, c) {
        return 0;
    }
    let mut v = f64::from(contact_connection(ch, c) + contact_loyalty(ch, c));
    if c.get_bool("family").unwrap_or(false) {
        v += 1.0;
    }
    if c.get_bool("blackmail").unwrap_or(false) {
        v += 2.0;
    }
    v += ch.improvements.val("ContactKarmaDiscount", None);
    v = v.max(2.0 + ch.improvements.val("ContactKarmaMinimum", None));
    standard_round(v)
}

/// `Contact.EntityType == ContactType.Contact`.
fn is_contact(c: &Element) -> bool {
    let t = c.get("type");
    t.is_empty() || t == "Contact"
}

/// `Character.ContactPoints`. Files from before 5.214 have a gameplay
/// option `<contactmultiplier>` (6 for Prime Runner) that the expression
/// cannot reproduce; for those use the saved `<contactpoints>`, which
/// Chummer reads into its cache on load.
pub fn free_contact_points(ch: &Character, sheet: &Sheet) -> i32 {
    match (ch.doc.get_i32("contactmultiplier"), ch.doc.get_i32("contactpoints")) {
        (Some(_), Some(saved)) => saved,
        _ => sheet.contact_points,
    }
}

/// Contact points spent: (ordinary, Friends in High Places), as
/// `Character.GetContactPointsUsed` counts them (no groups, no enemies).
pub fn contact_points_used(ch: &Character) -> (i32, i32) {
    let fihp = friends_in_high_places(ch);
    let (mut used, mut high) = (0, 0);
    for c in ch.items("contacts", "contact") {
        if !is_contact(c) || contact_is_group(ch, c) {
            continue;
        }
        let cost = contact_points(ch, c);
        if fihp && contact_connection(ch, c) >= 8 {
            high += cost;
        } else {
            used += cost;
        }
    }
    (used, high)
}

/// `CalculateBP` contacts section: points beyond the free ones, and Friends
/// in High Places contacts beyond CHA x 4.
fn contact_karma(ch: &Character, sheet: &Sheet) -> i32 {
    let (used, high) = contact_points_used(ch);
    let high_free = if friends_in_high_places(ch) { sheet.attr("CHA") * 4 } else { 0 };
    (used - free_contact_points(ch, sheet)).max(0) + (high - high_free).max(0)
}

/// `Character.EnemyKarma`.
fn enemy_karma(ch: &Character, settings: &CharacterSettings) -> i32 {
    let per = settings.karma("karmaenemy", 1);
    if !settings.flag("enableenemytracking") || per <= 0 {
        return 0;
    }
    let sum: i32 = ch
        .items("contacts", "contact")
        .iter()
        .filter(|c| c.get("type") == "Enemy" && !contact_free(ch, c))
        .map(|c| c.get_i32("connection").unwrap_or(0) + c.get_i32("loyalty").unwrap_or(0))
        .sum();
    sum * per
}

/// Group contacts are paid as positive qualities (`PositiveQualityKarma`).
fn group_contact_karma(ch: &Character, rules: &Rules) -> i32 {
    ch.items("contacts", "contact").iter().filter(|c| is_contact(c) && contact_is_group(ch, c)).map(|c| contact_points(ch, c)).sum::<i32>() * rules.karma_contact
}

// ---------------------------------------------------------------------------
// Qualities
// ---------------------------------------------------------------------------

/// Quality karma at creation (`Character.PositiveQualityKarma`,
/// `NegativeQualityKarma` and the life-module part of `CalculateBP`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QualityKarma {
    /// Positive qualities, group contacts, free-quality deductions and the
    /// doubling house rule.
    pub positive: i32,
    /// Negative qualities and enemies as a positive number, after the
    /// no-bonus cap.
    pub negative: i32,
    /// Positive and negative quality karma that counts toward the limit.
    pub positive_limit: i32,
    pub negative_limit: i32,
    pub life_modules: i32,
    /// Changelings pay 1 karma when metagenic qualities are off by one.
    pub metagenic_balance: i32,
}

fn origin(q: &Element) -> String {
    q.get("qualitysource")
}

/// A FreeQuality improvement names the quality's id or name.
fn free_quality(ch: &Character, q: &Element) -> bool {
    let (id, name) = (q.get("id"), q.get("name"));
    ch.improvements.of_kind("FreeQuality").any(|i| (!id.is_empty() && i.improved_name.eq_ignore_ascii_case(&id)) || i.improved_name == name)
}

/// `Quality.Metagenic` (older files save it as `<metagenetic>`).
fn metagenic(q: &Element) -> bool {
    q.get_bool("metagenic").or_else(|| q.get_bool("metagenetic")).unwrap_or(false)
}

/// `Character.MetagenicLimit`: changelings get a MetageneticLimit.
fn metagenic_limit(ch: &Character) -> i32 {
    standard_round(ch.improvements.val("MetageneticLimit", None))
}

/// Metagenic qualities are paid from the changeling's metagenic limit.
fn free_metagenic(ch: &Character, q: &Element) -> bool {
    metagenic(q) && metagenic_limit(ch) > 0
}

/// The Beast's Way and the Spiritual Way include a free Mentor Spirit.
fn free_mentor(ch: &Character, q: &Element) -> bool {
    q.get("name") == "Mentor Spirit" && ch.items("qualities", "quality").iter().any(|o| matches!(o.get("name").as_str(), "The Beast's Way" | "The Spiritual Way"))
}

/// `Quality.ContributeToBP`.
pub fn quality_contributes_to_bp(ch: &Character, q: &Element) -> bool {
    if matches!(origin(q).as_str(), "Metatype" | "MetatypeRemovable" | "Heritage") || free_metagenic(ch, q) || free_mentor(ch, q) {
        return false;
    }
    q.get_bool("contributetobp").unwrap_or(true) && !free_quality(ch, q)
}

/// `Quality.ContributeToLimit`.
pub fn quality_contributes_to_limit(ch: &Character, q: &Element) -> bool {
    if matches!(origin(q).as_str(), "Metatype" | "MetatypeRemovable" | "MetatypeRemovedAtChargen" | "Heritage") || free_metagenic(ch, q) || free_mentor(ch, q) {
        return false;
    }
    q.get_bool("contributetolimit").unwrap_or(true) && !free_quality(ch, q)
}

/// Karma (`BP` x KarmaQuality) of the qualities of `kind` that contribute
/// to BP: (counting toward the limit, not counting).
fn quality_sums(ch: &Character, rules: &Rules, kind: &str) -> (i32, i32) {
    let (mut limited, mut unlimited) = (0, 0);
    for q in ch.items("qualities", "quality") {
        if q.get("qualitytype") != kind || !quality_contributes_to_bp(ch, q) {
            continue;
        }
        let bp = q.get_i32("bp").unwrap_or(0) * rules.karma_quality;
        if quality_contributes_to_limit(ch, q) {
            limited += bp;
        } else {
            unlimited += bp;
        }
    }
    (limited, unlimited)
}

/// `Quality.Levels`: copies with the same id, extra, source name and type.
fn quality_levels(ch: &Character, q: &Element) -> i32 {
    let key = |x: &Element| (x.get("id"), x.get("extra"), x.get("sourcename"), x.get("qualitytype"));
    ch.items("qualities", "quality").iter().filter(|o| key(o) == key(q)).count() as i32
}

/// Life module qualities cost BP x Levels x KarmaQuality (`CalculateBP`).
fn life_module_karma(ch: &Character, rules: &Rules) -> i32 {
    ch.items("qualities", "quality")
        .iter()
        .filter(|q| q.get("qualitytype") == "LifeModule" && quality_contributes_to_bp(ch, q))
        .map(|q| q.get_i32("bp").unwrap_or(0) * rules.karma_quality * quality_levels(ch, q))
        .sum()
}

/// `Quality.ContributeToMetagenicLimit`.
fn contributes_to_metagenic_limit(ch: &Character, q: &Element) -> bool {
    !matches!(origin(q).as_str(), "Metatype" | "MetatypeRemovable" | "MetatypeRemovedAtChargen" | "Heritage") && free_metagenic(ch, q)
}

/// `MetagenicPositiveQualityKarma + MetagenicNegativeQualityKarma` (raw
/// BP; negative BP values are negative).
fn metagenic_sum(ch: &Character) -> i32 {
    let bp: i32 = ch
        .items("qualities", "quality")
        .iter()
        .filter(|q| matches!(q.get("qualitytype").as_str(), "Positive" | "Negative") && contributes_to_metagenic_limit(ch, q))
        .map(|q| q.get_i32("bp").unwrap_or(0))
        .sum();
    bp - standard_round(ch.improvements.val("FreeNegativeQualities", None))
}

/// `QualityKarmaLimit`. Files from before 5.214 store their gameplay
/// option's limit as `<gameplayoptionqualitylimit>` (as with `<buildkarma>`).
pub fn quality_limit(ch: &Character, settings: &CharacterSettings) -> i32 {
    ch.doc.get_i32("gameplayoptionqualitylimit").unwrap_or_else(|| settings.int("qualitykarmalimit", 25))
}

/// Positive and negative quality karma (`PositiveQualityKarma`,
/// `NegativeQualityKarma`) and the parts `CalculateBP` adds around them.
pub fn quality_karma(ch: &Character, rules: &Rules, settings: &CharacterSettings) -> QualityKarma {
    let limit = quality_limit(ch, settings);
    let free = |kind: &str| standard_round(ch.improvements.val(kind, None) * f64::from(rules.karma_quality));
    let (pos_limited, pos_unlimited) = quality_sums(ch, rules, "Positive");
    let mut positive = pos_limited + group_contact_karma(ch, rules) - free("FreePositiveQualities");
    if settings.flag("exceedpositivequalitiescostdoubled") && positive > limit {
        positive += positive - limit;
    }
    positive += pos_unlimited;

    // Negative BP values are negative; Chummer flips the sign at the end.
    let (neg_limited, neg_unlimited) = quality_sums(ch, rules, "Negative");
    let mut negative = neg_limited + enemy_karma(ch, settings) - free("FreeNegativeQualities");
    if settings.flag("exceednegativequalitiesnobonus") {
        negative = negative.max(-limit);
    }
    negative += neg_unlimited;

    QualityKarma {
        positive,
        negative: -negative,
        positive_limit: pos_limited,
        negative_limit: -neg_limited,
        life_modules: life_module_karma(ch, rules),
        metagenic_balance: i32::from(metagenic_sum(ch) == 1),
    }
}

// ---------------------------------------------------------------------------
// Martial arts, magic and resonance
// ---------------------------------------------------------------------------

/// `CharacterSettings.KarmaTechnique` (older files call it `karmamaneuver`).
fn karma_technique(settings: &CharacterSettings) -> i32 {
    settings.karma("karmatechnique", settings.karma("karmamaneuver", 5))
}

/// Martial arts not granted by a quality cost their `<cost>`, and every
/// technique after the first costs KarmaTechnique.
fn martial_arts_karma(ch: &Character, settings: &CharacterSettings) -> i32 {
    let per = karma_technique(settings);
    ch.items("martialarts", "martialart")
        .iter()
        .filter(|a| !a.get_bool("isquality").unwrap_or(false))
        .map(|a| {
            let techniques = a.child("martialarttechniques").map_or(0, |t| t.elements().count() as i32);
            a.get_i32("cost").unwrap_or(7) + (techniques - 1).max(0) * per
        })
        .sum()
}

/// Mystic adept power points bought at creation: the karma they cost and
/// the free spells they use instead.
struct PowerPointPurchase {
    karma: i32,
    spells: i32,
}

/// Mystic adepts buy power points (`MysticAdeptPowerPoints`) with karma
/// (`KarmaMysticAdeptPowerPoint` each), or first with free spells under
/// `PrioritySpellsAsAdeptPowers`. Not with MAGAdept as a second attribute.
fn mystic_adept_power_points(ch: &Character, settings: &CharacterSettings) -> PowerPointPurchase {
    let mut bought = ch.doc.get_i32("magsplitadept").unwrap_or(0);
    if !(ch.is_adept() && ch.is_magician()) || settings.flag("mysadeptsecondmagattribute") || bought <= 0 {
        return PowerPointPurchase { karma: 0, spells: 0 };
    }
    let mut spells = 0;
    if settings.flag("priorityspellsasadeptpowers") {
        let free = ch.doc.get_i32("spelllimit").unwrap_or(0);
        spells = free.min(bought);
        bought = (bought - free).max(0);
    }
    PowerPointPurchase { karma: bought * settings.karma("karmamysadpp", 5), spells }
}

fn find_guid<'a>(e: &'a Element, guid: &str) -> Option<&'a Element> {
    if e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().find_map(|c| find_guid(c, guid))
}

/// The gear item a `<focus>` binds (`Focus.GearObject`).
fn focus_gear<'a>(ch: &'a Character, focus: &Element) -> Option<&'a Element> {
    let id = focus.get("gearid");
    ["gears", "armors", "weapons", "cyberwares", "vehicles"].iter().find_map(|c| ch.doc.child(c).and_then(|x| find_guid(x, &id)))
}

/// `Focus.BindingKarmaCost` summed over the bound foci.
fn foci_karma(ch: &Character, settings: &CharacterSettings) -> i32 {
    ch.items("foci", "focus").iter().filter_map(|f| focus_gear(ch, f)).map(|g| crate::items::magic::focus_binding_karma(ch, settings, g)).sum()
}

/// Spirits and sprites cost KarmaSpirit per service owed; fettered spirits
/// add Force x KarmaSpiritFettering.
fn spirit_karma(ch: &Character, settings: &CharacterSettings) -> i32 {
    let per = settings.karma("karmaspirit", 1);
    let fetter = settings.karma("karmaspiritfettering", 3);
    ch.items("spirits", "spirit")
        .iter()
        .map(|s| {
            let mut k = s.get_i32("services").unwrap_or(0) * per;
            if s.get("type") != "Sprite" && s.get_bool("fettered").unwrap_or(false) {
                k += s.get_i32("force").unwrap_or(0) * fetter;
            }
            k
        })
        .sum()
}

/// Initiation grades taken at creation plus extra metamagics
/// (KarmaMetamagic each after the first per grade), enhancements (2 each)
/// and joining a magical group.
fn initiation_karma(ch: &Character, rules: &Rules, settings: &CharacterSettings) -> i32 {
    let metamagic_grades: Vec<i32> = ch.items("metamagics", "metamagic").iter().map(|m| m.get_i32("grade").unwrap_or(0)).collect();
    let mut total: i32 = crate::items::magic::initiation::grades(ch)
        .into_iter()
        .map(|(grade, techno, o)| {
            let extra = (metamagic_grades.iter().filter(|g| **g == grade).count() as i32 - 1).max(0);
            crate::items::magic::initiation_karma(rules, settings, grade, techno, o) + extra * rules.karma_metamagic
        })
        .sum();
    let power_enhancements: usize = ch.items("powers", "power").iter().map(|p| p.child("enhancements").map_or(0, |e| e.elements().count())).sum();
    total += (ch.items("enhancements", "enhancement").len() + power_enhancements) as i32 * 2;
    if ch.flag("groupmember") && ch.mag_enabled() {
        total += settings.karma("karmajoingroup", 5);
    }
    total
}
