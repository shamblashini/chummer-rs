//! The `<character>` element (`Character.PrintToXmlTextWriterCore`):
//! identity, attributes, derived values and the order of all sections.

use super::{add, bool_text, items, magic, skills, social, vehicles, Ctx};
use crate::calc::{attribute_karma_cost, AttributeValues};
use crate::xml::Element;

/// `Character.PrintToXmlTextWriterCore`.
pub fn print(ctx: &Ctx) -> Element {
    let mut out = Element::new("character");
    identity(ctx, &mut out);
    descriptions(ctx, &mut out);
    points(ctx, &mut out);
    flags(ctx, &mut out);
    if let Some(t) = magic::tradition(ctx) {
        out.push(t);
    }
    out.push(attributes(ctx));
    defenses(ctx, &mut out);
    monitors(ctx, &mut out);
    initiative(ctx, &mut out);
    magic_flags(ctx, &mut out);
    derived(ctx, &mut out);
    resistances(ctx, &mut out);
    out.push(skills::skills(ctx));
    out.push(social::contacts(ctx));
    for (wrap, limit) in [("limitmodifiersphys", "Physical"), ("limitmodifiersment", "Mental"), ("limitmodifierssoc", "Social")] {
        out.push(magic::limit_modifiers(ctx, wrap, limit));
    }
    sections(ctx, &mut out);
    out
}

/// Item lists, in Chummer's order.
fn sections(ctx: &Ctx, out: &mut Element) {
    let ch = ctx.ch;
    out.push(magic::mentor_spirits(ctx));
    out.push(list(ctx, "spells", "spell", magic::spell));
    out.push(list(ctx, "powers", "power", magic::power));
    out.push(list(ctx, "spirits", "spirit", magic::spirit));
    out.push(list(ctx, "complexforms", "complexform", magic::complex_form));
    out.push(list(ctx, "aiprograms", "aiprogram", magic::ai_program));
    out.push(list(ctx, "martialarts", "martialart", magic::martial_art));
    out.push(list(ctx, "armors", "armor", items::armor));
    out.push(list(ctx, "weapons", "weapon", items::weapon));
    out.push(list(ctx, "cyberwares", "cyberware", items::cyberware));
    out.push(social::qualities(ctx));
    out.push(list(ctx, "lifestyles", "lifestyle", social::lifestyle));
    out.push(items::gear_list(ctx, &ch.items("gears", "gear")));
    out.push(list(ctx, "drugs", "drug", items::drug));
    out.push(list(ctx, "vehicles", "vehicle", vehicles::vehicle));
    out.push(magic::initiation_grades(ctx));
    out.push(list(ctx, "metamagics", "metamagic", magic::metamagic));
    out.push(list(ctx, "arts", "art", magic::art));
    out.push(list(ctx, "enhancements", "enhancement", magic::enhancement));
    out.push(list(ctx, "critterpowers", "critterpower", magic::critter_power));
    out.push(Element::new("sustainedobjects"));
    out.push(social::other_armors(ctx));
    out.push(social::calendar(ctx));
    if ctx.opts.expenses {
        out.push(social::expenses(ctx));
    }
}

/// `<container>` with one printed element per saved item.
fn list(ctx: &Ctx, container: &str, item: &str, f: fn(&Ctx, &Element) -> Element) -> Element {
    let mut out = Element::new(container);
    for e in ctx.ch.items(container, item) {
        out.push(f(ctx, e));
    }
    out
}

fn field(ctx: &Ctx, out: &mut Element, name: &str) {
    add(out, name, ctx.ch.field(name));
}

/// Settings, metatype, movement, priorities, name.
fn identity(ctx: &Ctx, out: &mut Element) {
    let ch = ctx.ch;
    field(ctx, out, "settings");
    field(ctx, out, "buildmethod");
    add(out, "imageformat", "jpeg");
    let metatype = ch.field("metatype");
    add(out, "metatype", ctx.lang.data_name("metatypes.xml", &ch.field("metatypeid"), &metatype));
    add(out, "metatype_english", metatype);
    add(out, "metatype_guid", ch.field("metatypeid"));
    let metavariant = ch.field("metavariant");
    add(out, "metavariant", ctx.lang.data_name("metatypes.xml", &ch.field("metavariantid"), &metavariant));
    add(out, "metavariant_english", metavariant);
    add(out, "metavariant_guid", ch.field("metavariantid"));
    let movement = full_movement(ctx);
    for f in ["movement", "walk", "run", "sprint"] {
        add(out, f, movement.clone());
    }
    add(out, "movementwalk", movement_for(ctx, 0));
    add(out, "movementswim", movement_for(ctx, 1));
    add(out, "movementfly", movement_for(ctx, 2));
    for f in ["prioritymetatype", "priorityattributes", "priorityspecial", "priorityskills", "priorityresources"] {
        field(ctx, out, f);
    }
    let mut ps = Element::new("priorityskills");
    for s in ch.doc.children_named("priorityskills").flat_map(|p| p.children_named("priorityskill")) {
        add(&mut ps, "priorityskill", s.text());
    }
    out.push(ps);
    let arm = match ch.field("primaryarm").as_str() {
        _ if ch.improvements.has("Ambidextrous") => ctx.s("String_Ambidextrous"),
        "Left" => ctx.s("String_Improvement_SideLeft"),
        _ => ctx.s("String_Improvement_SideRight"),
    };
    add(out, "primaryarm", arm);
    let name = ch.name();
    add(out, "name", if name.trim().is_empty() { ctx.s("String_UnnamedCharacter") } else { name });
    mugshots(ctx, out);
}

/// `Character.PrintMugshots`.
fn mugshots(ctx: &Ctx, out: &mut Element) {
    let shots: Vec<String> = ctx.ch.doc.child("mugshots").map(|m| m.children_named("mugshot").map(Element::text).collect()).unwrap_or_default();
    if shots.is_empty() {
        return;
    }
    let main = ctx.ch.doc.get_i32("mainmugshotindex").filter(|i| *i >= 0 && (*i as usize) < shots.len()).map(|i| i as usize);
    if let Some(i) = main {
        add(out, "mainmugshotbase64", shots[i].clone());
    }
    add(out, "hasothermugshots", bool_text(main.is_none() || shots.len() > 1));
    let mut others = Element::new("othermugshots");
    for (_, s) in shots.iter().enumerate().filter(|(i, _)| Some(*i) != main) {
        let mut m = Element::new("mugshot");
        add(&mut m, "stringbase64", s.clone());
        others.push(m);
    }
    out.push(others);
}

/// Rates of one movement kind from `<walk>`/`<run>`/`<sprint>`
/// (`"ground/swim/fly"`).
fn rate(ctx: &Ctx, key: &str, kind: usize) -> f64 {
    ctx.ch.field(key).split('/').nth(kind).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0)
}

/// `Character.CalculatedMovement`.
fn movement_for(ctx: &Ctx, kind: usize) -> String {
    let agi = f64::from(ctx.sheet.attr("AGI"));
    let mult = if kind == 1 { (agi + f64::from(ctx.sheet.attr("STR"))) / 2.0 } else { agi };
    let walk = rate(ctx, "walk", kind) * mult;
    let run = rate(ctx, "run", kind) * mult;
    let sprint = rate(ctx, "sprint", kind);
    if walk == 0.0 && run == 0.0 && sprint == 0.0 {
        return "0".into();
    }
    let mut s = match (walk == 0.0, run == 0.0) {
        (false, false) => format!("{}/{}", super::num(walk), super::num(run)),
        (false, true) => super::num(walk),
        _ => super::num(run),
    };
    if sprint != 0.0 {
        s.push_str(&format!("; {}{}", super::num(sprint), ctx.s("String_MetersPerHit")));
    }
    s
}

/// `Character.FullMovement`.
fn full_movement(ctx: &Ctx) -> String {
    let parts = [
        movement_for(ctx, 0),
        prefixed(ctx, "Label_OtherSwim", movement_for(ctx, 1)),
        prefixed(ctx, "Label_OtherFly", movement_for(ctx, 2)),
    ];
    parts.into_iter().filter(|p| !p.is_empty() && p != "0").collect::<Vec<_>>().join(", ")
}

fn prefixed(ctx: &Ctx, label: &str, v: String) -> String {
    if v == "0" { String::new() } else { format!("{} {v}", ctx.s(label)) }
}

/// Appearance and the long text fields.
fn descriptions(ctx: &Ctx, out: &mut Element) {
    add(out, "gender", ctx.ch.field("gender"));
    for f in ["age", "eyes", "height", "weight", "skin", "hair", "description", "background", "concept", "notes", "alias", "playername", "gamenotes"] {
        field(ctx, out, f);
    }
}

/// Limits, point budgets, karma, reputation, nuyen.
fn points(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    let ch = ctx.ch;
    for (k, v) in [("limitphysical", s.limit_physical), ("limitmental", s.limit_mental), ("limitsocial", s.limit_social), ("limitastral", s.limit_astral)] {
        add(out, k, v.to_string());
    }
    add(out, "contactpoints", s.contact_points.to_string());
    let used: i32 = ch.items("contacts", "contact").iter().filter(|c| !c.get_bool("free").unwrap_or(false)).map(|c| c.get_i32("connection").unwrap_or(0) + c.get_i32("loyalty").unwrap_or(0)).sum();
    add(out, "contactpointsused", used.to_string());
    for f in ["cfplimit", "ainormalprogramlimit", "aiadvancedprogramlimit", "spelllimit"] {
        add(out, f, ch.doc.get_i32(f).unwrap_or(0).to_string());
    }
    add(out, "karma", ch.karma.to_string());
    add(out, "totalkarma", career_karma(ctx).to_string());
    for f in ["special", "totalspecial", "attributes", "totalattributes"] {
        add(out, f, ch.doc.get_i32(f).unwrap_or(0).to_string());
    }
    let edge = s.attr("EDG");
    let used_edge = ch.improvements.val_int("Attribute", Some("EDG")).min(0).abs();
    add(out, "edgeused", used_edge.to_string());
    add(out, "edgeremaining", edge.to_string());
    reputation(ctx, out);
    add(out, "created", bool_text(ch.created));
    add(out, "nuyen", ctx.nuyen(ch.nuyen));
}

/// `Character.CareerKarma`: karma earned in career mode.
fn career_karma(ctx: &Ctx) -> i32 {
    ctx.ch
        .items("expenses", "expense")
        .iter()
        .filter(|e| e.get("type") == "Karma" && !e.get_bool("refund").unwrap_or(false))
        .filter_map(|e| e.get_f64("amount"))
        .filter(|a| *a > 0.0)
        .sum::<f64>() as i32
}

/// Street cred, notoriety, public awareness, astral and wild reputation.
fn reputation(ctx: &Ctx, out: &mut Element) {
    let ch = ctx.ch;
    let imps = &ch.improvements;
    let cred = ch.doc.get_i32("streetcred").unwrap_or(0);
    let burnt = ch.doc.get_i32("burntstreetcred").unwrap_or(0);
    let calc_cred = career_karma(ctx) / 10 + imps.val_int("StreetCred", None);
    add(out, "streetcred", cred.to_string());
    add(out, "calculatedstreetcred", calc_cred.to_string());
    add(out, "totalstreetcred", (cred + calc_cred - burnt).max(0).to_string());
    add(out, "burntstreetcred", burnt.to_string());
    let noto = ch.doc.get_i32("notoriety").unwrap_or(0);
    let calc_noto = imps.val_int("Notoriety", None);
    add(out, "notoriety", noto.to_string());
    add(out, "calculatednotoriety", calc_noto.to_string());
    add(out, "totalnotoriety", (noto + calc_noto + burnt).to_string());
    let pa = ch.doc.get_i32("publicawareness").unwrap_or(0);
    let calc_pa = imps.val_int("PublicAwareness", None);
    add(out, "publicawareness", pa.to_string());
    add(out, "calculatedpublicawareness", calc_pa.to_string());
    add(out, "totalpublicawareness", (pa + calc_pa).to_string());
    for (k, imp) in [("astralreputation", "AstralReputation"), ("wildreputation", "WildReputation")] {
        let v = ch.doc.get_i32(k).unwrap_or(0);
        add(out, k, v.to_string());
        add(out, &format!("total{k}"), (v + imps.val_int(imp, None)).to_string());
    }
}

/// Character type flags and essence.
fn flags(ctx: &Ctx, out: &mut Element) {
    let ch = ctx.ch;
    for f in ["adept", "magician", "technomancer", "ai", "cyberwaredisabled", "critter"] {
        add(out, f, bool_text(ch.flag(f)));
    }
    add(out, "totaless", ctx.essence(ctx.sheet.essence));
}

/// Whether an attribute is printed (`CharacterAttrib.Print` guards).
fn attribute_shown(ctx: &Ctx, name: &str) -> bool {
    let ch = ctx.ch;
    match name {
        "MAGAdept" => {
            let second = ctx.engine.settings.resolve(&ch.field("settings")).is_some_and(|s| s.flag("mysadeptsecondmagattribute"));
            second && ch.mag_enabled() && ch.is_adept() && ch.is_magician()
        }
        "MAG" => ch.mag_enabled(),
        "RES" => ch.res_enabled(),
        "DEP" => ch.dep_enabled(),
        _ => true,
    }
}

/// `AttributeSection.Print`.
fn attributes(ctx: &Ctx) -> Element {
    let mut out = Element::new("attributes");
    let ch = ctx.ch;
    let category = ch.field("metatypecategory");
    if category == "Shapeshifter" {
        add(&mut out, "attributecategory", ctx.lang.data_name("metatypes.xml", "", &ch.field("metatype")));
    }
    add(&mut out, "attributecategory_english", if category == "Shapeshifter" { "Shapeshifter" } else { "Standard" });
    for a in ctx.sheet.attributes.iter().filter(|a| attribute_shown(ctx, &a.name)) {
        let saved_cat = ch.attribute(&a.name).map(|x| x.category.clone()).unwrap_or_default();
        out.push(attribute(ctx, a, &saved_cat));
    }
    out
}

/// `CharacterAttrib.Print`.
fn attribute(ctx: &Ctx, a: &AttributeValues, category: &str) -> Element {
    let mut out = Element::new("attribute");
    add(&mut out, "name_english", a.name.clone());
    let short = if a.name == "MAGAdept" { format!("{} ({})", ctx.s("String_AttributeMAGShort"), ctx.s("String_Adept")) } else { ctx.s(&format!("String_Attribute{}Short", a.name)) };
    add(&mut out, "name", short);
    if a.name == "ESS" {
        add(&mut out, "base", ctx.essence(ctx.sheet.essence));
        add(&mut out, "total", ctx.essence(ctx.sheet.essence));
    } else {
        add(&mut out, "base", a.value.to_string());
        add(&mut out, "total", a.total.to_string());
    }
    add(&mut out, "min", a.total_min.to_string());
    add(&mut out, "max", a.total_max.to_string());
    add(&mut out, "aug", a.total_aug_max.to_string());
    add(&mut out, "bp", attribute_karma_cost(a, &ctx.rules).to_string());
    add(&mut out, "metatypecategory", if category.is_empty() { "Standard" } else { category });
    out
}

/// Dodge and the armor values with their resistance pools.
fn defenses(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    let imps = &ctx.ch.improvements;
    let dodge = s.attr("REA") + s.attr("INT") + imps.val_int("Dodge", None) + s.wound_modifier;
    add(out, "dodge", dodge.to_string());
    let armor = s.armor;
    let kinds = [("armor", 0), ("firearmor", imps.val_int("FireArmor", None)), ("coldarmor", imps.val_int("ColdArmor", None)),
        ("electricityarmor", imps.val_int("ElectricityArmor", None)), ("acidarmor", imps.val_int("AcidArmor", None)), ("fallingarmor", imps.val_int("FallingArmor", None))];
    for (k, extra) in kinds {
        add(out, k, (armor + extra).to_string());
    }
    let resist = s.attr("BOD") + imps.val_int("DamageResistance", None);
    for suffix in ["stun", "physical"] {
        for (k, extra) in kinds {
            add(out, &format!("{k}dice{suffix}"), (resist + armor + extra).to_string());
        }
    }
}

/// Condition monitors.
fn monitors(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    let ch = ctx.ch;
    let ai = ch.flag("ai");
    add(out, "physicalcm", s.physical_cm.to_string());
    add(out, "physicalcmiscorecm", bool_text(ai));
    add(out, "stuncm", s.stun_cm.to_string());
    add(out, "stuncmismatrixcm", bool_text(ai));
    add(out, "physicalcmfilled", ch.physical_cm_filled.to_string());
    add(out, "stuncmfilled", ch.stun_cm_filled.to_string());
    add(out, "cmthreshold", s.cm_threshold.to_string());
    let offset = ch.improvements.val_int("CMThresholdOffset", None);
    add(out, "physicalcmthresholdoffset", offset.min(s.physical_cm).to_string());
    add(out, "stuncmthresholdoffset", offset.min(s.stun_cm).to_string());
    add(out, "cmoverflow", if ai { 0 } else { s.cm_overflow }.to_string());
    add(out, "psyche", bool_text(false));
}

/// `String_Initiative` formatted: `"10 + 1d6"`.
fn init_string(ctx: &Ctx, value: i32, dice: i32) -> String {
    ctx.s("String_Initiative").replace("{0}", &value.to_string()).replace("{1}", &dice.to_string())
}

fn init_triplet(ctx: &Ctx, out: &mut Element, prefix: &str, value: i32, dice: i32) {
    add(out, prefix, init_string(ctx, value, dice));
    add(out, &format!("{prefix}dice"), dice.to_string());
    add(out, &format!("{prefix}value"), value.to_string());
}

/// Physical, astral and matrix initiative.
fn initiative(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    add(out, "init", init_string(ctx, s.initiative, s.initiative_dice));
    add(out, "initdice", s.initiative_dice.to_string());
    add(out, "initvalue", s.initiative.to_string());
    add(out, "initbonus", ctx.ch.improvements.val_int("Initiative", None).max(0).to_string());
    if ctx.ch.mag_enabled() {
        init_triplet(ctx, out, "astralinit", s.astral_initiative, s.astral_initiative_dice);
    }
    init_triplet(ctx, out, "matrixarinit", s.initiative, s.initiative_dice);
    init_triplet(ctx, out, "matrixcoldinit", s.matrix_cold_initiative, s.matrix_cold_dice);
    init_triplet(ctx, out, "matrixhotinit", s.matrix_hot_initiative, s.matrix_hot_dice);
    add(out, "riggerinit", init_string(ctx, s.initiative, s.initiative_dice));
}

fn magic_flags(ctx: &Ctx, out: &mut Element) {
    let ch = ctx.ch;
    add(out, "magenabled", bool_text(ch.mag_enabled()));
    add(out, "initiategrade", ch.doc.get_i32("initiategrade").unwrap_or(0).to_string());
    add(out, "resenabled", bool_text(ch.res_enabled()));
    add(out, "submersiongrade", ch.doc.get_i32("submersiongrade").unwrap_or(0).to_string());
    add(out, "depenabled", bool_text(ch.dep_enabled()));
    add(out, "groupmember", bool_text(ch.flag("groupmember")));
    field(ctx, out, "groupname");
    field(ctx, out, "groupnotes");
}

/// Surprise, composure, judge intentions, lift/carry, memory.
fn derived(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    let imps = &ctx.ch.improvements;
    let surprise = s.attr("REA") + s.attr("INT") + imps.val_int("Surprise", None) + s.wound_modifier;
    add(out, "surprise", surprise.to_string());
    add(out, "composure", s.composure.to_string());
    add(out, "judgeintentions", s.judge_intentions.to_string());
    let ji_resist = s.attr("CHA") + s.attr("WIL") + crate::expr::standard_round(imps.val("JudgeIntentions", None) + imps.val("JudgeIntentionsDefense", None));
    add(out, "judgeintentionsresist", ji_resist.to_string());
    add(out, "liftandcarry", s.lift_carry.to_string());
    add(out, "memory", s.memory.to_string());
    let str_ = f64::from(s.attr("STR"));
    add(out, "liftweight", super::num(str_ * 15.0));
    add(out, "carryweight", super::num(str_ * 10.0));
    add(out, "totalcarriedweight", "0");
}

/// Resistance tests (`FatigueResist`, `ToxinContactResist`, ...).
fn resistances(ctx: &Ctx, out: &mut Element) {
    let s = &ctx.sheet;
    let imps = &ctx.ch.improvements;
    let at = |a: &str| s.attr(a);
    let bw = at("BOD") + at("WIL");
    let list: Vec<(&str, i32)> = vec![
        ("fatigueresist", bw + imps.val_int("FatigueResist", None)),
        ("radiationresist", bw + imps.val_int("RadiationResist", None)),
        ("sonicresist", at("WIL") + imps.val_int("SonicResist", None)),
        ("toxincontactresist", bw + imps.val_int("ToxinContactResist", None)),
        ("toxiningestionresist", bw + imps.val_int("ToxinIngestionResist", None)),
        ("toxininhalationresist", bw + imps.val_int("ToxinInhalationResist", None)),
        ("toxininjectionresist", bw + imps.val_int("ToxinInjectionResist", None)),
        ("pathogencontactresist", bw + imps.val_int("PathogenContactResist", None)),
        ("pathogeningestionresist", bw + imps.val_int("PathogenIngestionResist", None)),
        ("pathogeninhalationresist", bw + imps.val_int("PathogenInhalationResist", None)),
        ("pathogeninjectionresist", bw + imps.val_int("PathogenInjectionResist", None)),
        ("physiologicaladdictionresistfirsttime", bw + imps.val_int("PhysiologicalAddictionFirstTime", None)),
        ("physiologicaladdictionresistalreadyaddicted", bw + imps.val_int("PhysiologicalAddictionAlreadyAddicted", None)),
        ("psychologicaladdictionresistfirsttime", at("LOG") + at("WIL") + imps.val_int("PsychologicalAddictionFirstTime", None)),
        ("psychologicaladdictionresistalreadyaddicted", at("LOG") + at("WIL") + imps.val_int("PsychologicalAddictionAlreadyAddicted", None)),
        ("physicalcmnaturalrecovery", at("BOD") * 2 + imps.val_int("PhysicalCMRecovery", None)),
        ("stuncmnaturalrecovery", bw + imps.val_int("StunCMRecovery", None)),
        ("indirectdefenseresist", at("REA") + at("INT") + imps.val_int("SpellDefenseIndirectDodge", None)),
        ("directmanaresist", at("WIL") + imps.val_int("SpellDefenseDirectSoakMana", None)),
        ("directphysicalresist", at("BOD") + imps.val_int("SpellDefenseDirectSoakPhysical", None)),
        ("detectionspellresist", at("LOG") + at("WIL") + imps.val_int("SpellDefenseDetection", None)),
        ("decreasebodresist", at("BOD") + at("WIL")),
        ("decreaseagiresist", at("AGI") + at("WIL")),
        ("decreaserearesist", at("REA") + at("WIL")),
        ("decreasestrresist", at("STR") + at("WIL")),
        ("decreasecharesist", at("CHA") + at("WIL")),
        ("decreaseintresist", at("INT") + at("WIL")),
        ("decreaselogresist", at("LOG") + at("WIL")),
        ("decreasewilresist", at("WIL") * 2),
        ("illusionmanaresist", at("LOG") + at("WIL") + imps.val_int("SpellDefenseIllusionMana", None)),
        ("illusionphysicalresist", at("LOG") + at("INT") + imps.val_int("SpellDefenseIllusionPhysical", None)),
        ("manipulationmentalresist", at("LOG") + at("WIL") + imps.val_int("SpellDefenseManipulationMental", None)),
        ("manipulationphysicalresist", at("STR") + at("BOD") + imps.val_int("SpellDefenseManipulationPhysical", None)),
    ];
    for (k, v) in list {
        add(out, k, v.to_string());
    }
}
