//! Bonus node handlers. Simple handlers come from the generated table in
//! `generic.rs`; the rest are written out here, each named after its
//! method in `AddImprovementCollection.cs`.

use super::generic::GENERIC;
use super::Ctx;
use crate::items::new_guid;
use crate::xml::{parse_bool, Element};

/// Where a generated row takes the improved name from.
pub enum N {
    Empty,
    Text,
    Child(&'static str),
    Lit(&'static str),
}

/// Unique name: the `<bonus unique>` attribute or a literal.
pub enum U {
    Bonus,
    Lit(&'static str),
}

/// A numeric field: zero, the node text, a child's text, or a literal.
#[allow(dead_code)] // `Lit` is emitted by the generator when the C# passes a literal.
pub enum V {
    Zero,
    Text,
    Child(&'static str),
    Lit(i32),
}

/// One generated handler.
pub struct G {
    pub node: &'static str,
    pub kind: &'static str,
    pub name: N,
    pub unique: U,
    pub val: V,
    pub rating: i32,
    pub min: V,
    pub max: V,
    pub aug: V,
    pub aug_max: V,
    pub condition: Option<&'static str>,
}

fn num(ctx: &Ctx<'_>, node: &Element, v: &V, int: bool) -> f64 {
    let s = match v {
        V::Zero => return 0.0,
        V::Lit(n) => return f64::from(*n),
        V::Text => node.text(),
        V::Child(k) => node.get(k),
    };
    if int {
        f64::from(ctx.int(&s))
    } else {
        ctx.dec(&s)
    }
}

fn generic(ctx: &mut Ctx<'_>, node: &Element, g: &G) {
    let name = match g.name {
        N::Empty => String::new(),
        N::Text => node.text(),
        N::Child(k) => node.get(k),
        N::Lit(s) => s.to_owned(),
    };
    let mut i = ctx.imp(g.kind, &name);
    if let U::Lit(u) = g.unique {
        i.unique_name = u.to_owned();
    }
    i.val = num(ctx, node, &g.val, false);
    i.rating = g.rating;
    i.min = num(ctx, node, &g.min, true);
    i.max = num(ctx, node, &g.max, true);
    i.aug = num(ctx, node, &g.aug, false);
    i.aug_max = num(ctx, node, &g.aug_max, true);
    if let Some(c) = g.condition {
        i.condition = node.get(c);
    }
    ctx.push(i);
}

/// `precedence="N"` on the node (or its `<name>`) gives unique `precedenceN`.
fn precedence(node: &Element) -> Option<String> {
    node.attr("precedence")
        .or_else(|| node.child("name").and_then(|n| n.attr("precedence")))
        .map(|p| format!("precedence{p}"))
}

fn child_bool(node: &Element, k: &str) -> bool {
    node.child_text(k).is_some_and(|t| parse_bool(&t))
}

/// Plain decimal parse used by the `*level` handlers (no Rating substitution).
fn plain_dec(node: &Element, k: &str, default: f64) -> f64 {
    node.child_text(k).and_then(|t| t.trim().parse().ok()).unwrap_or(default)
}

/// Returns false when the node type is not handled.
pub fn apply_node(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let name = node.name.as_str();
    match name {
        "pushtext" | "selecttext" => {}
        "skilllevel" | "skillgrouplevel" => {
            let val = plain_dec(node, "val", 1.0);
            let kind = if name == "skilllevel" { "SkillLevel" } else { "SkillGroupLevel" };
            let target = if let Some(n) = node.child_text("name") {
                n
            } else if node.child("selectskill").is_some() || node.child("selectskillgroup").is_some() {
                match ctx.answer() {
                    Some(a) => a,
                    None => return false,
                }
            } else {
                return true;
            };
            let mut i = ctx.imp(kind, &target);
            i.val = val;
            ctx.push(i);
        }
        "knowledgeskilllevel" => {
            let val = node.child_text("val").map(|v| ctx.dec(&v)).unwrap_or(1.0);
            if node.child("selectskill").is_some() {
                let Some(a) = ctx.answer() else { return false };
                let mut i = ctx.imp("SkillLevel", &a);
                i.val = val;
                ctx.push(i);
            } else {
                let mut i = ctx.imp("FreeKnowledgeSkills", "");
                i.val = val;
                ctx.push(i);
            }
        }
        "attributelevel" => {
            let val = plain_dec(node, "val", 1.0);
            let target = match node.child_text("name") {
                Some(n) => n,
                None if node.child("options").is_some() => match ctx.answer() {
                    Some(a) => a,
                    None => return false,
                },
                None => return true,
            };
            let mut i = ctx.imp("Attributelevel", &target);
            i.val = val;
            ctx.push(i);
        }
        "enabletab" | "disabletab" => {
            for n in node.children_named("name") {
                let t = n.text().to_ascii_uppercase();
                let (canon, flag) = match (name, t.as_str()) {
                    ("enabletab", "MAGICIAN") => ("Magician", "magician"),
                    ("enabletab", "ADEPT") => ("Adept", "adept"),
                    ("enabletab", "TECHNOMANCER") => ("Technomancer", "technomancer"),
                    ("enabletab", "ADVANCED PROGRAMS") => ("Advanced Programs", "ainode"),
                    ("enabletab", "CRITTER") => ("Critter", "critter"),
                    ("disabletab", "CYBERWARE") => ("Cyberware", "cyberwaredisabled"),
                    ("disabletab", "INITIATION") => ("Initiation", "initiationdisabled"),
                    _ => continue,
                };
                let mut i = ctx.imp("SpecialTab", canon);
                i.unique_name = name.to_owned();
                i.rating = 0;
                ctx.push(i);
                if name == "enabletab" {
                    ctx.out.flags.push((flag.to_owned(), "True".to_owned()));
                }
            }
        }
        "enableattribute" => {
            let n = node.get("name").to_ascii_uppercase();
            let flag = match n.as_str() {
                "MAG" => "magenabled",
                "RES" => "resenabled",
                "DEP" => "depenabled",
                _ => return true,
            };
            let mut i = ctx.imp("Attribute", &n);
            i.unique_name = "enableattribute".into();
            i.rating = 0;
            ctx.push(i);
            ctx.out.flags.push((flag.to_owned(), "True".to_owned()));
            // EssenceAtSpecialStart is fixed when the special attribute is
            // first enabled during creation.
            if !ctx.ch.created && ctx.ch.doc.get_f64("essenceatspecialstart").is_none_or(|v| v < -1e20) {
                let ess = ctx.attrs.iter().find(|a| a.name == "ESS").map_or(6, |a| a.metatype_max);
                ctx.out.flags.push(("essenceatspecialstart".into(), ess.to_string()));
            }
        }
        "specificskill" => {
            let skill = node.get("name");
            let atr = child_bool(node, "applytorating");
            let cond = node.get("condition");
            let uniq = precedence(node);
            let base = |ctx: &Ctx<'_>, kind: &str| {
                let mut i = ctx.imp(kind, &skill);
                if let Some(u) = &uniq {
                    i.unique_name = u.clone();
                }
                i.condition = cond.clone();
                i
            };
            if let Some(b) = node.child_text("bonus") {
                let mut i = base(ctx, "Skill");
                i.val = ctx.dec(&b);
                i.add_to_rating = atr;
                ctx.push(i);
            }
            if node.child("disablespecializationeffects").is_some() {
                let i = base(ctx, "DisableSpecializationEffects");
                ctx.push(i);
            }
            if let Some(m) = node.child_text("max") {
                let mut i = base(ctx, "Skill");
                i.max = f64::from(ctx.int(&m));
                i.add_to_rating = atr;
                ctx.push(i);
            }
            if let Some(m) = node.child_text("misceffect") {
                let mut i = base(ctx, "Skill");
                i.target = m;
                ctx.push(i);
            }
        }
        "specificattribute" => {
            let mut target = node.get("name");
            if node.child("affectbase").is_some() {
                target.push_str("Base");
            }
            let mut i = ctx.imp("Attribute", &target);
            if let Some(u) = precedence(node) {
                i.unique_name = u;
            }
            if let Some(v) = node.child_text("min") {
                i.min = f64::from(ctx.int(&v));
            }
            if let Some(v) = node.child_text("val") {
                i.aug = ctx.dec(&v);
            }
            if let Some(v) = node.child_text("max") {
                i.max = f64::from(ctx.int(v.trim_end_matches("-natural")));
            }
            if let Some(v) = node.child_text("aug") {
                i.aug_max = f64::from(ctx.int(&v));
            }
            ctx.push(i);
        }
        "selectattribute" => {
            let Some(a) = ctx.answer() else { return false };
            let mut target = a;
            if node.child("affectbase").is_some() {
                target.push_str("Base");
            }
            let mut i = ctx.imp("Attribute", &target);
            i.min = f64::from(ctx.int(&node.get("min")));
            i.aug = ctx.dec(&node.get("val"));
            i.max = f64::from(ctx.int(&node.get("max")));
            i.aug_max = f64::from(ctx.int(&node.get("aug")));
            ctx.push(i);
        }
        "selectattributes" => {
            let Some(a) = ctx.answer() else { return false };
            // A single attribute answer applies to every child; the
            // "BOD (2), INT (1)" form lists each pick with its count.
            let mut picks: Vec<String> = Vec::new();
            for part in a.split(',') {
                let part = part.trim();
                match part.split_once(" (") {
                    Some((attr, n)) => {
                        let n: usize = n.trim_end_matches(')').parse().unwrap_or(1);
                        picks.extend(std::iter::repeat_n(attr.to_owned(), n));
                    }
                    None => picks.push(part.to_owned()),
                }
            }
            for (idx, sel) in node.children_named("selectattribute").enumerate() {
                let attr = picks.get(idx).or(picks.first()).cloned().unwrap_or_default();
                let p = |k: &str| sel.get(k).trim().parse::<f64>().unwrap_or(0.0);
                let mut i = ctx.imp("Attribute", &attr);
                i.min = p("min");
                i.aug = p("val");
                i.max = p("max");
                i.aug_max = p("aug");
                ctx.push(i);
            }
        }
        "replaceattributes" => {
            for r in node.children_named("replaceattribute") {
                let n = r.get("name");
                if n.is_empty() {
                    continue;
                }
                let p = |k: &str| r.get_i32(k).unwrap_or(0) as f64;
                let mut i = ctx.imp("ReplaceAttribute", &n);
                i.min = p("min");
                i.max = p("max");
                i.aug_max = p("aug");
                ctx.push(i);
            }
        }
        "initiativepass" | "initiativedice" => {
            let mut i = ctx.imp("InitiativeDice", "");
            i.val = ctx.dec(&node.text());
            i.unique_name = precedence(node).unwrap_or_else(|| name.to_owned());
            ctx.push(i);
        }
        "matrixinitiativedice" | "matrixinitiativepass" => {
            let mut i = ctx.imp("MatrixInitiativeDice", "");
            i.val = ctx.dec(&node.text());
            i.unique_name = "matrixinitiativepass".into();
            ctx.push(i);
        }
        "lifestylecost" => {
            let mut i = ctx.imp("LifestyleCost", node.attr("lifestyle").unwrap_or(""));
            i.val = ctx.dec(&node.text());
            i.condition = node.attr("condition").unwrap_or("").to_owned();
            ctx.push(i);
        }
        "reach" => {
            let mut i = ctx.imp("Reach", node.attr("name").unwrap_or(""));
            i.val = ctx.dec(&node.text());
            ctx.push(i);
        }
        "armor" | "firearmor" | "coldarmor" | "electricityarmor" | "acidarmor" | "fallingarmor" | "dodge" => {
            let kind = match name {
                "armor" => "Armor",
                "firearmor" => "FireArmor",
                "coldarmor" => "ColdArmor",
                "electricityarmor" => "ElectricityArmor",
                "acidarmor" => "AcidArmor",
                "fallingarmor" => "FallingArmor",
                _ => "Dodge",
            };
            let mut i = ctx.imp(kind, "");
            i.val = ctx.dec(&node.text());
            if let Some(u) = precedence(node).or_else(|| node.attr("group").map(|g| format!("group{g}"))) {
                i.unique_name = u;
            }
            ctx.push(i);
        }
        "conditionmonitor" => {
            for (child, kind, prec) in [
                ("physical", "PhysicalCM", false),
                ("stun", "StunCM", false),
                ("threshold", "CMThreshold", true),
                ("thresholdoffset", "CMThresholdOffset", true),
                ("sharedthresholdoffset", "CMSharedThresholdOffset", true),
                ("overflow", "CMOverflow", false),
            ] {
                if let Some(c) = node.child(child) {
                    let mut i = ctx.imp(kind, "");
                    i.val = ctx.dec(&c.text());
                    if prec {
                        if let Some(u) = precedence(c) {
                            i.unique_name = u;
                        }
                    }
                    ctx.push(i);
                }
            }
        }
        "skillcategory" | "skillgroup" | "skillattribute" | "skilllinkedattribute" => {
            let target = node.get("name");
            if target.is_empty() {
                return true;
            }
            let kind = match name {
                "skillcategory" => "SkillCategory",
                "skillgroup" => "SkillGroup",
                "skillattribute" => "SkillAttribute",
                _ => "SkillLinkedAttribute",
            };
            let mut i = ctx.imp(kind, &target);
            i.val = ctx.dec(&node.get("bonus"));
            i.exclude = node.get("exclude");
            i.add_to_rating = child_bool(node, "applytorating");
            i.condition = node.get("condition");
            if matches!(name, "skillattribute" | "skilllinkedattribute") {
                if let Some(u) = precedence(node) {
                    i.unique_name = u;
                }
            }
            ctx.push(i);
        }
        "spellcategory" => {
            let mut i = ctx.imp("SpellCategory", &node.get("name"));
            i.val = ctx.dec(&node.get("val"));
            i.condition = node.get("condition");
            ctx.push(i);
        }
        "focusbindingkarmacost" | "focusbindingkarmamultiplier" => {
            let kind = if name == "focusbindingkarmacost" { "FocusBindingKarmaCost" } else { "FocusBindingKarmaMultiplier" };
            let mut i = ctx.imp(kind, &node.get("name"));
            i.val = ctx.dec(&node.get("val"));
            i.target = node.get("extracontains");
            ctx.push(i);
        }
        "walkmultiplier" | "runmultiplier" | "sprintbonus" => {
            let cat = node.get("category");
            if cat.is_empty() {
                return true;
            }
            let (k1, k2) = match name {
                "walkmultiplier" => ("WalkMultiplier", "WalkMultiplierPercent"),
                "runmultiplier" => ("RunMultiplier", "RunMultiplierPercent"),
                _ => ("SprintBonus", "SprintBonusPercent"),
            };
            for (child, kind) in [("val", k1), ("percent", k2)] {
                if let Some(t) = node.child_text(child) {
                    let v = ctx.dec(&t);
                    if v != 0.0 {
                        let mut i = ctx.imp(kind, &cat);
                        i.val = v;
                        ctx.push(i);
                    }
                }
            }
        }
        "movementreplace" => {
            let kind = match node.get("speed").to_ascii_uppercase().as_str() {
                "RUN" => "RunSpeed",
                "SPRINT" => "SprintSpeed",
                _ => "WalkSpeed",
            };
            let val = ctx.dec(&node.get("val"));
            let cats: Vec<String> = match node.child_text("category") {
                Some(c) => vec![c],
                None => vec!["Ground".into(), "Swim".into(), "Fly".into()],
            };
            for c in cats {
                let mut i = ctx.imp(kind, &c);
                i.val = val;
                ctx.push(i);
            }
        }
        "qualitylevel" => {
            let mut i = ctx.imp("QualityLevel", node.attr("group").unwrap_or(""));
            i.val = f64::from(ctx.int(&node.text()));
            ctx.push(i);
        }
        "limitmodifier" => {
            let limit = node.get("limit");
            let value = ctx.dec(&node.get("value"));
            let cond = node.get("condition");
            let guid = new_guid();
            let mut lm = Element::new("limitmodifier");
            lm.push(Element::with_text("guid", guid.clone()));
            lm.push(Element::with_text("name", ctx.src.name.clone()));
            lm.push(Element::with_text("limit", limit));
            lm.push(Element::with_text("bonus", crate::improvement::fmt_num(value.round())));
            lm.push(Element::with_text("condition", cond.clone()));
            lm.push(Element::with_text("candelete", "False"));
            lm.push(Element::new("notes"));
            ctx.out.added.push(("limitmodifiers".into(), lm));
            let mut i = ctx.imp("LimitModifier", &guid);
            i.val = value;
            i.rating = 0;
            i.condition = cond;
            ctx.push(i);
        }
        "selectskill" => {
            let Some(skill) = ctx.answer() else { return false };
            let atr = child_bool(node, "applytorating");
            if let Some(v) = node.child_text("val") {
                let mut i = ctx.imp("Skill", &skill);
                i.val = ctx.dec(&v);
                i.add_to_rating = atr;
                ctx.push(i);
            }
            if child_bool(node, "disablespecializationeffects") {
                let i = ctx.imp("DisableSpecializationEffects", &skill);
                ctx.push(i);
            }
            if let Some(m) = node.child_text("max") {
                let mut i = ctx.imp("Skill", &skill);
                i.max = f64::from(ctx.int(&m));
                i.add_to_rating = atr;
                ctx.push(i);
            }
        }
        "weaponskillaccuracy" => {
            let skill = match ctx.answer().or_else(|| node.child_text("name")) {
                Some(s) => s,
                None => return false,
            };
            ctx.selected = Some(skill.clone());
            let mut i = ctx.imp("WeaponSkillAccuracy", &skill);
            i.val = ctx.dec(&node.get("value"));
            ctx.push(i);
        }
        "swapskillattribute" => {
            let attrs: Vec<String> = node.children_named("attribute").map(Element::text).collect();
            let attr = match ctx.answer().or_else(|| (attrs.len() == 1).then(|| attrs[0].clone())) {
                Some(a) => a,
                None => return false,
            };
            let mut i = ctx.imp("SwapSkillAttribute", &attr);
            i.target = node.get("limittoskill");
            ctx.push(i);
        }
        "unlockskills" => {
            let opts: Vec<String> = node.text().split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            let choice = if opts.len() == 1 { opts[0].clone() } else { match ctx.answer() { Some(a) => a, None => return false } };
            let mut i = ctx.imp("SpecialSkills", &choice);
            i.target = node.attr("name").unwrap_or("").to_owned();
            ctx.push(i);
        }
        "livingpersona" => {
            for (child, kind) in [
                ("devicerating", "LivingPersonaDeviceRating"),
                ("programlimit", "LivingPersonaProgramLimit"),
                ("attack", "LivingPersonaAttack"),
                ("sleaze", "LivingPersonaSleaze"),
                ("dataprocessing", "LivingPersonaDataProcessing"),
                ("firewall", "LivingPersonaFirewall"),
                ("matrixcm", "LivingPersonaMatrixCM"),
            ] {
                if let Some(t) = node.child_text(child) {
                    let mut v = crate::expr::fixed_values(&t, ctx.src.rating).replace("Rating", &ctx.src.rating.to_string());
                    if v.trim().parse::<i32>().is_ok_and(|n| n > 0) {
                        v = format!("+{}", v.trim());
                    }
                    let i = ctx.imp(kind, &v);
                    ctx.push(i);
                }
            }
        }
        "selectweapon" | "selectarmor" => {
            let Some(a) = ctx.answer() else { return false };
            let i = ctx.imp("Text", &a);
            ctx.push(i);
        }
        "addqualities" => {
            let Ok(doc) = ctx.store.doc("qualities.xml") else { return false };
            for aq in node.children_named("addquality") {
                let forced = aq.attr("select").map(str::to_owned);
                let count = aq.attr("rating").map(|r| ctx.int(r)).unwrap_or(1).max(1);
                let free = !aq.attr("contributetobp").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                let Some(rec) = crate::data::find(&doc, "qualities", "quality", &aq.text()) else { return false };
                add_quality(ctx, rec, forced.as_deref(), count, free);
            }
        }
        // Bonus types that create objects. Each is implemented next to the
        // object it creates; they return false until then.
        "addgear" => return crate::items::gear::bonus_addgear(ctx, node),
        "naturalweapon" => return crate::items::weapon::bonus_naturalweapon(ctx, node),
        "addweapon" => return crate::items::weapon::bonus_addweapon(ctx, node),
        "addware" => return crate::items::cyberware::bonus_addware(ctx, node),
        "specificpower" | "selectpowers" => return crate::items::magic::bonus_power(ctx, node),
        "critterpowers" | "optionalpowers" => return crate::items::magic::bonus_critterpowers(ctx, node),
        "addspell" | "addcomplexform" | "addart" | "addmetamagic" | "addecho" => return crate::items::magic::bonus_add_magic(ctx, node),
        "addspirit" | "limitspiritcategory" => return crate::items::magic::bonus_spirit(ctx, node),
        "selectquality" => return select_quality(ctx, node),
        "addcontact" => add_contact(ctx, node),
        "selectcontact" => return select_contact(ctx, node),
        "selectrestricted" => {
            let Some(a) = ctx.answer() else { return false };
            let i = ctx.imp("Restricted", &a);
            ctx.push(i);
        }
        "selecttradition" => {
            let Some(a) = ctx.answer() else { return false };
            let i = ctx.imp("Tradition", &a);
            ctx.push(i);
        }
        "selectmentorspirit" | "selectparagon" => return select_mentor(ctx, node),
        "addskillspecializationoption" => {
            let spec = node.get("spec");
            let mut skills: Vec<String> = node.child("skills").map(|s| s.children_named("skill").map(Element::text).collect()).unwrap_or_default();
            if skills.is_empty() {
                skills.push(node.get("skill"));
            }
            for s in skills.into_iter().filter(|s| !s.is_empty()) {
                let mut i = ctx.imp("SkillSpecializationOption", &s);
                i.unique_name = spec.clone();
                ctx.push(i);
            }
        }
        "fadingvalue" | "drainvalue" => {
            let kind = if name == "fadingvalue" { "FadingValue" } else { "DrainValue" };
            let mut i = ctx.imp(kind, node.attr("specific").unwrap_or(""));
            i.val = ctx.dec(&node.text());
            ctx.push(i);
        }
        "weaponcategorydice" | "weaponcategorydv" | "weaponcategoryap" | "weaponcategoryaccuracy" | "weaponcategoryreach" => {
            let kind = match name {
                "weaponcategorydice" => "WeaponCategoryDice",
                "weaponcategorydv" => "WeaponCategoryDV",
                "weaponcategoryap" => "WeaponCategoryAP",
                "weaponcategoryaccuracy" => "WeaponCategoryAccuracy",
                _ => "WeaponCategoryReach",
            };
            // Older data lists <category><name/><value/></category> pairs.
            if node.child("category").is_some_and(|c| c.child("name").is_some()) {
                for c in node.children_named("category") {
                    let mut i = ctx.imp(kind, &c.get("name"));
                    i.val = ctx.dec(&c.get("value"));
                    ctx.push(i);
                }
                return true;
            }
            let target = if node.child("selectskill").is_some() || node.child("selectcategory").is_some() {
                match ctx.answer() {
                    Some(a) => a,
                    None => return false,
                }
            } else {
                node.get("name")
            };
            let mut i = ctx.imp(kind, &target);
            i.val = ctx.dec(&node.get("bonus"));
            ctx.push(i);
        }
        "addlimb" => {
            let mut i = ctx.imp("AddLimb", &node.get("limbslot"));
            i.val = ctx.dec(&node.get("val"));
            ctx.push(i);
        }
        "prototypetranshuman" => {
            let mut i = ctx.imp("PrototypeTranshuman", &node.text());
            i.rating = ctx.src.rating;
            ctx.push(i);
        }
        "cyberseeker" => {
            let mut i = ctx.imp("Seeker", &node.text());
            i.rating = 0;
            ctx.push(i);
        }
        "restrictedgear" => {
            let (value, count) = match node.child_text("amount") {
                Some(c) => (node.get("availability"), c),
                None => (node.child_text("availability").unwrap_or_else(|| node.text()), "1".to_owned()),
            };
            let mut i = ctx.imp("RestrictedGear", "");
            i.val = ctx.dec(&value);
            i.rating = ctx.int(&count);
            ctx.push(i);
        }
        "metageneticlimit" => {
            // Old name of metageniclimit.
            let mut i = ctx.imp("MetageneticLimit", "");
            i.val = ctx.dec(&node.text());
            ctx.push(i);
        }
        "smartlink" => {
            let mut i = ctx.imp("Smartlink", "");
            i.unique_name = "smartlink".into();
            i.val = ctx.dec(&node.text());
            ctx.push(i);
        }
        n if IGNORED.iter().any(|(i, _)| *i == n) => {}
        // No AddImprovementCollection method exists for these (data errors
        // upstream); Chummer5a logs "Tried to get unknown bonus" and rolls
        // the whole bonus back, so they stay unsupported here.
        "astralreputation" | "defensetest" | "addquality" => return false,
        "disablecyberwaregrade" | "disablebiowaregrade" => disable_grade(ctx, node),
        "nuyenamt" => nuyen_amt(ctx, node),
        "skillwire" | "skillsoftaccess" => skillwire(ctx, node),
        "availability" | "newspellkarmacost" => text_value_with_condition(ctx, node),
        "knowledgeskillpoints" => knowledge_skill_points(ctx, node),
        "penaltyfreesustain" => penalty_free_sustain(ctx, node),
        "metamagiclimit" => metamagic_limit(ctx, node),
        "critterpowerlevels" => critter_power_levels(ctx, node),
        "spelldicepool" => spell_dice_pool(ctx, node),
        "spellcategorydrain" => return spell_category_drain(ctx, node),
        "freespells" => free_spells(ctx, node),
        "allowspellrange" | "allowspellcategory" | "limitspellcategory" | "blockspelldescriptor" => return spell_restriction(ctx, node),
        "selectlimit" => return select_limit(ctx, node),
        "selectside" => {
            let Some(a) = ctx.answer() else { return false };
            ctx.selected = Some(a);
        }
        "selectcyberware" => {
            let Some(a) = ctx.answer() else { return false };
            let i = ctx.imp("Text", &a);
            ctx.push(i);
            ctx.selected = Some(a);
        }
        "blackmarketdiscount" => return black_market_discount(ctx),
        "dealerconnection" => return dealer_connection(ctx, node),
        "skillgroupdisablechoice" => {
            let Some(a) = ctx.answer() else { return false };
            let i = ctx.imp("SkillGroupDisable", &a);
            ctx.push(i);
            ctx.selected = Some(a);
        }
        "swapskillspecattribute" => return swap_skill_spec_attribute(ctx, node),
        "actiondicepool" => return action_dice_pool(ctx, node),
        "hardwires" => return hardwires(ctx, node),
        "activesoft" => return activesoft(ctx, node),
        "skillsoft" => return skillsoft(ctx, node),
        "addskillspecialization" => add_skill_specialization(ctx, node),
        "selectexpertise" => return select_expertise(ctx, node),
        "selectinherentaiprogram" => return select_ai_program(ctx),
        "weaponspecificdice" => return weapon_specific_dice(ctx, node),
        "martialart" => return martial_art(ctx, node),
        "selectspell" => return select_spell(ctx, node),
        "selectcomplexform" => return select_complex_form(ctx),
        "selectart" | "selectmetamagic" | "selectecho" => return select_metamagic(ctx, node),
        "selectsprite" | "addsprite" => return sprite(ctx, node),
        _ => {
            if let Some(g) = GENERIC.iter().find(|g| g.node == name) {
                generic(ctx, node, g);
                return true;
            }
            // Empty unknown elements are skipped, as in Chummer5a.
            return node.children.is_empty();
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Ignored node types
// ---------------------------------------------------------------------------

/// Bonus node types that are not character improvements, with the reason.
/// `apply` accepts them without creating anything, as Chummer5a does.
pub const IGNORED: &[(&str, &str)] = &[
    // Vehicle mod stats: `VehicleMod.Create` runs its bonus with
    // `blnAddImprovementsToCharacter = false`, which skips unknown methods;
    // `Vehicle` reads these nodes from the mod itself.
    ("handling", "vehicle mod stat"),
    ("offroadhandling", "vehicle mod stat"),
    ("accel", "vehicle mod stat"),
    ("offroadaccel", "vehicle mod stat"),
    ("speed", "vehicle mod stat"),
    ("offroadspeed", "vehicle mod stat"),
    ("seats", "vehicle mod stat"),
    ("sensor", "vehicle mod stat"),
    ("pilot", "vehicle mod stat"),
    ("body", "vehicle mod stat"),
    ("devicerating", "vehicle mod stat"),
    // Drug component effects, read by `Drug` (items/drug.rs), never passed
    // to the improvement manager.
    ("attribute", "drug effect"),
    ("limit", "drug effect"),
    ("quality", "drug effect"),
];

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// `XmlNode.InnerText`: the text of all descendants.
fn inner_text(e: &Element) -> String {
    let mut out = String::new();
    for n in &e.children {
        match n {
            crate::xml::Node::Text(t) | crate::xml::Node::CData(t) => out.push_str(t),
            crate::xml::Node::Element(c) => out.push_str(&inner_text(c)),
            crate::xml::Node::Comment(_) => {}
        }
    }
    out.trim().to_owned()
}

/// `ValueToDec` at an explicit rating.
fn dec_at(ctx: &Ctx<'_>, s: &str, rating: i32) -> f64 {
    crate::expr::value_to_dec(s.trim(), rating, &crate::calc::SheetAttributes(&ctx.attrs))
}

/// The two-argument `CreateImprovement` overload appends its name to the
/// selected value ("A, B"). The shared answer counts as already selected.
fn append_selected(ctx: &mut Ctx<'_>, v: &str) {
    ctx.selected = Some(match ctx.selected.take().filter(|s| !s.is_empty()) {
        Some(s) if s == v => s,
        Some(s) => format!("{s}, {v}"),
        None => v.to_owned(),
    });
}

/// The answer, or the only option when there is one.
fn answer_or_single(ctx: &Ctx<'_>, options: &[String]) -> Option<String> {
    ctx.answer().or_else(|| (options.len() == 1).then(|| options[0].clone()))
}

/// "Name (Extra)" for a record that is not itself named so.
fn record_and_extra<'a>(doc: &'a Element, container: &str, item: &'a str, answer: &str) -> Option<(crate::data::Record<'a>, String)> {
    if let Some(r) = crate::data::find(doc, container, item, answer) {
        return Some((r, String::new()));
    }
    let (base, rest) = answer.rsplit_once(" (")?;
    let extra = rest.strip_suffix(')')?;
    crate::data::find(doc, container, item, base).map(|r| (r, extra.to_owned()))
}

/// Apply a created object's own bonus, sourced to the object.
fn own_bonus(ctx: &Ctx<'_>, kind: &str, guid: &str, rec: crate::data::Record<'_>, forced: Option<&str>) -> super::Outcome {
    let src = super::BonusSource { kind: kind.into(), guid: guid.into(), name: rec.name(), rating: 1 };
    rec.el().child("bonus").map(|b| super::apply(ctx.ch, ctx.store, b, &src, forced)).unwrap_or_default()
}

/// Collect a created object and its own outcome, and link it to the
/// granting item with an improvement of `kind` named by its guid.
fn absorb(ctx: &mut Ctx<'_>, container: &str, el: Element, own: super::Outcome, kind: &str) {
    let guid = el.get("guid");
    ctx.out.improvements.extend(own.improvements);
    ctx.out.added.extend(own.added);
    ctx.out.flags.extend(own.flags);
    ctx.out.unsupported.extend(own.unsupported);
    ctx.out.added.push((container.into(), el));
    let i = ctx.imp(kind, &guid);
    ctx.push(i);
}

// ---------------------------------------------------------------------------
// Single-improvement handlers the generator cannot express
// ---------------------------------------------------------------------------

/// `disablecyberwaregrade` / `disablebiowaregrade`: unique is the node name.
fn disable_grade(ctx: &mut Ctx<'_>, node: &Element) {
    let kind = if node.name == "disablecyberwaregrade" { "DisableCyberwareGrade" } else { "DisableBiowareGrade" };
    let mut i = ctx.imp(kind, &node.text());
    i.unique_name = node.name.clone();
    ctx.push(i);
}

/// `nuyenamt`: the `condition` attribute is the improved name.
fn nuyen_amt(ctx: &mut Ctx<'_>, node: &Element) {
    let mut i = ctx.imp("Nuyen", node.attr("condition").unwrap_or(""));
    i.val = ctx.dec(&node.text());
    ctx.push(i);
}

/// `skillwire` / `skillsoftaccess`, with `precedence` as unique name.
fn skillwire(ctx: &mut Ctx<'_>, node: &Element) {
    let kind = if node.name == "skillwire" { "Skillwire" } else { "SkillsoftAccess" };
    let mut i = ctx.imp(kind, "");
    if let Some(p) = node.attr("precedence").filter(|p| !p.is_empty()) {
        i.unique_name = format!("precedence{p}");
    }
    i.val = ctx.dec(&node.text());
    ctx.push(i);
}

/// `availability` (name from `id`) and `newspellkarmacost` (name from
/// `type`): a value with a `condition` attribute.
fn text_value_with_condition(ctx: &mut Ctx<'_>, node: &Element) {
    let (kind, key) = if node.name == "availability" { ("Availability", "id") } else { ("NewSpellKarmaCost", "type") };
    let mut i = ctx.imp(kind, node.attr(key).unwrap_or(""));
    i.val = ctx.dec(&node.text());
    i.condition = node.attr("condition").unwrap_or("").to_owned();
    ctx.push(i);
}

/// `knowledgeskillpoints`: the inner text at rating 0 (the C# passes
/// `ValueToInt(bonusNode.Value)`, which is null for an element).
fn knowledge_skill_points(ctx: &mut Ctx<'_>, node: &Element) {
    let mut i = ctx.imp("FreeKnowledgeSkills", "");
    i.val = dec_at(ctx, &inner_text(node), 0);
    ctx.push(i);
}

/// `penaltyfreesustain`: value is the maximum force, rating the count.
fn penalty_free_sustain(ctx: &mut Ctx<'_>, node: &Element) {
    let count = node.child_text("count").map_or(1, |c| ctx.int(&c));
    let force = node.child_text("force").map_or(i32::MAX, |f| ctx.int(&f));
    let mut i = ctx.imp("PenaltyFreeSustain", &inner_text(node));
    i.val = f64::from(force);
    i.rating = count;
    ctx.push(i);
}

/// `metamagiclimit`: one improvement per `<metamagic>`, rating its grade.
fn metamagic_limit(ctx: &mut Ctx<'_>, node: &Element) {
    for m in node.children_named("metamagic") {
        let grade = m.attr("grade").filter(|g| !g.is_empty()).map_or(-1, |g| ctx.int(g));
        let mut i = ctx.imp("MetamagicLimit", &m.text());
        i.rating = grade;
        ctx.push(i);
    }
}

/// `critterpowerlevels`: one improvement per `<power>`.
fn critter_power_levels(ctx: &mut Ctx<'_>, node: &Element) {
    for p in node.children_named("power") {
        let mut i = ctx.imp("CritterPowerLevel", &p.get("name"));
        i.val = ctx.dec(&p.get("val"));
        ctx.push(i);
    }
}

/// `spelldicepool`: named by the spell `id`, else `name`.
fn spell_dice_pool(ctx: &mut Ctx<'_>, node: &Element) {
    let target = node.child_text("id").unwrap_or_else(|| node.get("name"));
    let mut i = ctx.imp("SpellDicePool", &target);
    i.val = ctx.dec(&node.get("val"));
    ctx.push(i);
}

/// `spellcategorydrain`: the `<category>`, else the value selected so far
/// (e.g. by a preceding `limitspellcategory`).
fn spell_category_drain(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(cat) = node.child_text("category").or_else(|| ctx.selected.clone()).filter(|c| !c.trim().is_empty()) else { return false };
    let mut i = ctx.imp("SpellCategoryDrain", &cat);
    i.val = ctx.dec(&node.get("val"));
    ctx.push(i);
    true
}

/// `freespells`: per attribute (`FreeSpellsATT`), per skill
/// (`FreeSpellsSkill`), or a plain count. `limit` becomes the unique name.
fn free_spells(ctx: &mut Ctx<'_>, node: &Element) {
    let limit = node.attr("limit").unwrap_or("").to_owned();
    if let Some(a) = node.attr("attribute") {
        if ctx.attrs.iter().any(|x| x.name == a) {
            let mut i = ctx.imp("FreeSpellsATT", a);
            i.unique_name = limit;
            ctx.push(i);
        }
    } else if let Some(s) = node.attr("skill") {
        let mut i = ctx.imp("FreeSpellsSkill", s);
        i.unique_name = limit;
        ctx.push(i);
    } else {
        let mut i = ctx.imp("FreeSpells", "");
        i.val = ctx.dec(&node.text());
        ctx.push(i);
    }
}

/// `allowspellrange`, `allowspellcategory`, `limitspellcategory`,
/// `blockspelldescriptor`: the node text, or the answer when it is empty
/// (the GUI asks for a spell category / descriptor).
fn spell_restriction(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let kind = match node.name.as_str() {
        "allowspellrange" => "AllowSpellRange",
        "allowspellcategory" => "AllowSpellCategory",
        "limitspellcategory" => "LimitSpellCategory",
        _ => "BlockSpellDescriptor",
    };
    let text = node.text().trim().to_owned();
    let value = if text.is_empty() && kind != "AllowSpellRange" {
        match ctx.answer() {
            Some(a) => a,
            None => return false,
        }
    } else {
        text
    };
    append_selected(ctx, &value);
    let i = ctx.imp(kind, &value);
    ctx.push(i);
    true
}

// ---------------------------------------------------------------------------
// Selecting handlers
// ---------------------------------------------------------------------------

/// `selectlimit`: a Physical/Mental/Social limit bonus (`val` is passed as
/// both value and augmented value).
fn select_limit(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let opts = super::select::limit_options(node);
    let Some(limit) = answer_or_single(ctx, &opts).filter(|a| opts.contains(a)) else { return false };
    let kind = match limit.to_ascii_uppercase().as_str() {
        "MENTAL" => "MentalLimit",
        "SOCIAL" => "SocialLimit",
        "PHYSICAL" => "PhysicalLimit",
        _ => return false,
    };
    let mut target = limit.clone();
    if node.child("affectbase").is_some() {
        target.push_str("Base");
    }
    let mut i = ctx.imp(kind, &target);
    let aug = node.child_text("val").filter(|v| !v.is_empty()).map_or(0.0, |v| ctx.dec(&v));
    i.val = aug;
    i.aug = aug;
    i.rating = 0;
    i.min = node.child_text("min").filter(|v| !v.is_empty()).map_or(0.0, |v| f64::from(ctx.int(&v)));
    i.max = node.child_text("max").filter(|v| !v.is_empty()).map_or(0.0, |v| f64::from(ctx.int(&v)));
    i.aug_max = node.child_text("aug").filter(|v| !v.is_empty()).map_or(0.0, |v| f64::from(ctx.int(&v)));
    ctx.push(i);
    ctx.selected = Some(limit);
    true
}

/// `blackmarketdiscount`: a black market pipeline category, when
/// options.xml defines any.
fn black_market_discount(ctx: &mut Ctx<'_>) -> bool {
    let cats = super::select::black_market_categories(ctx.store);
    let pick = if cats.is_empty() {
        String::new()
    } else {
        match ctx.answer() {
            Some(a) => a,
            None => return false,
        }
    };
    let i = ctx.imp("BlackMarketDiscount", &pick);
    ctx.push(i);
    ctx.selected = Some(pick);
    true
}

/// `dealerconnection`: a vehicle category not taken by another Dealer
/// Connection; the category is also the unique name.
fn dealer_connection(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let opts = super::select::dealer_options(ctx.ch, node);
    let Some(a) = ctx.answer().filter(|a| opts.contains(a)) else { return false };
    let mut i = ctx.imp("DealerConnection", &a);
    i.unique_name = a.clone();
    ctx.push(i);
    ctx.selected = Some(a);
    true
}

/// `swapskillspecattribute`: attribute (answer, or the only one listed)
/// replacing the linked attribute of one specialization. The skill comes
/// from `limittoskill`; a second skill prompt is not supported (no data
/// uses one).
fn swap_skill_spec_attribute(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let attrs = super::select::swap_attribute_options(ctx.ch, node);
    let Some(attr) = answer_or_single(ctx, &attrs) else { return false };
    let Some(skill) = node.child_text("limittoskill").filter(|s| !s.is_empty()) else { return false };
    let mut i = ctx.imp("SwapSkillSpecAttribute", &attr);
    i.exclude = node.get("spec");
    i.target = skill;
    ctx.push(i);
    ctx.selected = Some(attr);
    true
}

/// `actiondicepool`: the answer, else `<name>`; an answered action name
/// is stored by its actions.xml id, as the C# list does.
fn action_dice_pool(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let value = match (ctx.answer(), node.child_text("name")) {
        (Some(a), _) => ctx.store.doc("actions.xml").ok().and_then(|d| crate::data::find(&d, "actions", "action", &a).map(|r| r.id())).filter(|id| !id.is_empty()).unwrap_or(a),
        (None, Some(n)) => n,
        (None, None) => return false,
    };
    let mut i = ctx.imp("ActionDicePool", &value);
    i.val = ctx.dec(&node.get("val"));
    ctx.push(i);
    ctx.selected = Some(value);
    true
}

/// Exotic skills are named "Base (Specific)" with `<exotic>` on the base.
fn is_exotic(ctx: &Ctx<'_>, skill: &str) -> bool {
    let base = skill.split_once(" (").map_or(skill, |(b, _)| b);
    ctx.store.doc("skills.xml").ok().and_then(|d| crate::data::find(&d, "skills", "skill", base).map(|r| r.el().get_bool("exotic").unwrap_or(false))).unwrap_or(false)
}

/// `hardwires`: the answer, else the `select` attribute, else a skill
/// prompt. Non-exotic skills use the skill as unique name. Missing exotic
/// skills are not created here (the caller adds the skill).
fn hardwires(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(skill) = ctx.answer().or_else(|| node.attr("select").filter(|s| !s.is_empty()).map(str::to_owned)) else { return false };
    ctx.selected = Some(skill.clone());
    let exotic = is_exotic(ctx, &skill);
    if exotic && node.text().trim().is_empty() {
        return true;
    }
    let mut i = ctx.imp("Hardwire", &skill);
    if !exotic {
        i.unique_name = skill.clone();
    }
    i.val = ctx.dec(&node.text());
    ctx.push(i);
    true
}

/// A knowsoft knowledge skill (`<skilljackknowledgeskills>`), minimal:
/// guid and name. The caller adds it to the knowledge skills too when the
/// character has skillsoft access.
fn knowsoft_skill(name: &str) -> Element {
    let mut k = Element::new("skill");
    k.push(Element::with_text("guid", new_guid()));
    k.push(Element::with_text("name", name));
    k.push(Element::with_text("isknowledge", "True"));
    k
}

/// `activesoft`: an Activesoft rating for the chosen active skill, and
/// with `<addknowledge>` a knowsoft knowledge skill of the same name.
/// Missing exotic skills are not created here.
fn activesoft(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(skill) = ctx.answer() else { return false };
    ctx.selected = Some(skill.clone());
    let val = node.get("val");
    if !val.is_empty() {
        let mut i = ctx.imp("Activesoft", &skill);
        i.val = ctx.dec(&val);
        ctx.push(i);
    }
    if node.child("addknowledge").is_some() {
        add_knowsoft(ctx, &skill, &val);
    }
    true
}

fn add_knowsoft(ctx: &mut Ctx<'_>, skill: &str, val: &str) {
    let k = knowsoft_skill(skill);
    let mut i = ctx.imp("Skillsoft", &k.get("guid"));
    i.val = ctx.dec(val);
    ctx.push(i);
    ctx.out.added.push(("skilljackknowledgeskills".into(), k));
}

/// `skillsoft`: a knowsoft knowledge skill with its rating.
fn skillsoft(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(skill) = ctx.answer() else { return false };
    ctx.selected = Some(skill.clone());
    add_knowsoft(ctx, &skill, &node.get("val"));
    true
}

/// A `<spec>` for skill `skill`. `Outcome.added` has no per-skill
/// container, so the skill name rides along in `<skill>`; the caller
/// moves the spec onto that skill.
fn spec_element(skill: &str, spec: &str, expertise: bool) -> Element {
    let mut s = Element::new("spec");
    s.push(Element::with_text("guid", new_guid()));
    s.push(Element::with_text("name", spec));
    s.push(Element::with_text("free", "False"));
    s.push(Element::with_text("expertise", crate::improvement::bool_str(expertise)));
    s.push(Element::with_text("skill", skill));
    s
}

/// `addskillspecialization`: a named specialization on an active skill.
/// Nothing happens when the skill does not exist, as in Chummer5a.
fn add_skill_specialization(ctx: &mut Ctx<'_>, node: &Element) {
    let skill = node.get("skill");
    let exists = ctx.store.doc("skills.xml").ok().is_some_and(|d| crate::data::find(&d, "skills", "skill", &skill).is_some());
    if !exists {
        return;
    }
    let s = spec_element(&skill, &node.get("spec"), false);
    let mut i = ctx.imp("SkillSpecialization", &skill);
    i.unique_name = s.get("guid");
    ctx.push(i);
    ctx.out.added.push(("skillspecializations".into(), s));
}

/// `selectexpertise`: an expertise specialization. The skill must be the
/// only one the node allows (all data uses `limittoskill`); the answer is
/// the specialization.
fn select_expertise(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let skills = super::select::skill_options(ctx.ch, ctx.store, node);
    let [skill] = skills.as_slice() else { return false };
    let Some(spec) = ctx.answer() else { return false };
    let s = spec_element(skill, &spec, true);
    let mut i = ctx.imp("SkillSpecialization", skill);
    i.unique_name = s.get("guid");
    ctx.push(i);
    ctx.out.added.push(("skillspecializations".into(), s));
    ctx.selected = Some(spec);
    true
}

/// `selectinherentaiprogram`: an AI program (`AIProgram.Create` with
/// `blnCanDelete = false`). A program with `<selecttext>` takes its text
/// as "Name (Text)".
fn select_ai_program(ctx: &mut Ctx<'_>) -> bool {
    let Some(a) = ctx.answer() else { return false };
    let Ok(doc) = ctx.store.doc("programs.xml") else { return false };
    let Some((rec, extra)) = record_and_extra(&doc, "programs", "program", &a) else { return false };
    let guid = new_guid();
    let own = own_bonus(ctx, "AIProgram", &guid, rec, Some(&extra));
    let e = rec.el();
    let mut p = Element::new("aiprogram");
    for (k, v) in [
        ("sourceid", rec.id()),
        ("guid", guid),
        ("name", rec.name()),
        ("candelete", "False".into()),
        ("isadvancedprogram", crate::improvement::bool_str(rec.category() == "Advanced Programs")),
        ("requiresprogram", e.get("require")),
        ("extra", extra.clone()),
        ("source", rec.source()),
        ("page", rec.page()),
        ("notes", e.get("notes")),
    ] {
        p.push(Element::with_text(k, v));
    }
    ctx.selected = Some(if extra.is_empty() { rec.name() } else { format!("{} ({extra})", rec.name()) });
    absorb(ctx, "aiprograms", p, own, "AIProgram");
    true
}

/// `weaponspecificdice` (`CreateWeaponSpecificImprovement`): one of the
/// character's weapons, of range type `type` when given. The answer is
/// the weapon's guid or name.
fn weapon_specific_dice(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(a) = ctx.answer() else { return false };
    let ty = node.attr("type").unwrap_or("");
    let weapons = ctx.ch.items("weapons", "weapon");
    let Some(w) = weapons.into_iter().filter(|w| ty.is_empty() || w.get("type") == ty).find(|w| w.get("guid").eq_ignore_ascii_case(&a) || w.get("name") == a) else { return false };
    let (guid, wname) = (w.get("guid"), w.get("name"));
    let mut i = ctx.imp("WeaponSpecificDice", &guid);
    i.val = ctx.dec(&node.text());
    ctx.push(i);
    ctx.selected = Some(wname);
    true
}

// ---------------------------------------------------------------------------
// Object-creating handlers
// ---------------------------------------------------------------------------

/// `martialart`: a martial art granted by a quality (`IsQuality`).
fn martial_art(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Ok(doc) = ctx.store.doc("martialarts.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "martialarts", "martialart", node.text().trim()) else { return false };
    let guid = new_guid();
    let own = own_bonus(ctx, "MartialArt", &guid, rec, None);
    let mut el = crate::items::magic::martialart::element(rec, &guid, Vec::new());
    el.set_child_text("isquality", "True");
    absorb(ctx, "martialarts", el, own, "MartialArt");
    true
}

/// `selectspell`: the answered spell, limited to the `category`
/// attribute, granted at grade -1. A spell with `<selecttext>` takes its
/// text as "Name (Text)". Requirements are not checked.
fn select_spell(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    use crate::items::magic::spell;
    let Some(a) = ctx.answer() else { return false };
    let Ok(doc) = ctx.store.doc("spells.xml") else { return false };
    let Some((rec, extra)) = record_and_extra(&doc, "spells", "spell", &a) else { return false };
    if node.attr("category").is_some_and(|c| !c.is_empty() && c != rec.category()) {
        return false;
    }
    let guid = new_guid();
    let own = own_bonus(ctx, "Spell", &guid, rec, Some(&extra));
    let extra = own.selected.clone().filter(|s| !s.is_empty()).unwrap_or(extra);
    let o = spell::SpellOptions { grade: -1, ..Default::default() };
    ctx.selected = Some(rec.name());
    absorb(ctx, "spells", spell::element(rec, &guid, &extra, &o), own, "Spell");
    true
}

/// `selectcomplexform`: the answered complex form at grade -1. (The C#
/// looks it up under `complexforms/complexforms` and so always aborts;
/// this follows the evident intent.) No data uses this node.
fn select_complex_form(ctx: &mut Ctx<'_>) -> bool {
    let Some(a) = ctx.answer() else { return false };
    let Ok(doc) = ctx.store.doc("complexforms.xml") else { return false };
    let Some(rec) = crate::data::find(&doc, "complexforms", "complexform", &a) else { return false };
    let guid = new_guid();
    let own = own_bonus(ctx, "ComplexForm", &guid, rec, None);
    let extra = own.selected.clone().unwrap_or_default();
    ctx.selected = Some(rec.name());
    absorb(ctx, "complexforms", crate::items::magic::complexform::element(rec, &guid, &extra, -1), own, "ComplexForm");
    true
}

/// `selectart`, `selectmetamagic`, `selectecho`: one of the listed
/// children (or any record when none are listed) at grade -1; a listed
/// child's `select` attribute answers the new object's own selection.
/// Requirements are not checked. No data uses these nodes.
fn select_metamagic(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    use crate::items::magic::metamagic;
    let Some(a) = ctx.answer() else { return false };
    let (child, isrc, kind) = match node.name.as_str() {
        "selectart" => ("art", "Metamagic", "Art"),
        "selectecho" => ("echo", "Echo", "Echo"),
        _ => ("metamagic", "Metamagic", "Metamagic"),
    };
    let listed: Vec<&Element> = node.children_named(child).collect();
    let pick = listed.iter().find(|c| c.text() == a);
    if !listed.is_empty() && pick.is_none() {
        return false;
    }
    let (file, container, item) = if kind == "Art" { ("metamagic.xml", "arts", "art") } else { metamagic::data_path(isrc) };
    let Ok(doc) = ctx.store.doc(file) else { return false };
    let Some(rec) = crate::data::find(&doc, container, item, &a) else { return false };
    ctx.selected = Some(rec.name());
    if kind == "Art" {
        let guid = new_guid();
        let own = own_bonus(ctx, "Metamagic", &guid, rec, None);
        absorb(ctx, "arts", metamagic::art_element(rec, &guid, "Metamagic", -1), own, "Art");
    } else {
        let forced = pick.and_then(|c| c.attr("select")).map(str::to_owned);
        let (el, own) = metamagic::create(ctx.ch, ctx.store, rec, isrc, forced.as_deref(), -1);
        absorb(ctx, "metamagics", el, own, kind);
    }
    true
}

/// `selectsprite` (any critters.xml sprite) and `addsprite`
/// (`AddSpiritOrSprite` over streams.xml, limited to the listed
/// `<spirit>`s): an `AddSprite` improvement for the answered sprite.
fn sprite(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let opts = super::select::sprite_options(ctx.store, node);
    let Some(a) = answer_or_single(ctx, &opts).filter(|a| opts.contains(a)) else { return false };
    let i = ctx.imp("AddSprite", &a);
    ctx.push(i);
    let add_to_selected = node.child_text("addtoselected").is_none_or(|v| parse_bool(&v));
    if node.name == "selectsprite" {
        ctx.selected = Some(a);
    } else if add_to_selected {
        append_selected(ctx, &a);
    }
    true
}

/// The `<extra>` of an object of this name the character already has.
/// Re-applying a saved item's bonus finds the answers to prompts that
/// Chummer5a does not store on the item this way.
fn existing_extra(ctx: &Ctx<'_>, container: &str, item: &str, name: &str, keep: impl Fn(&Element) -> bool) -> Option<String> {
    ctx.ch.items(container, item).into_iter().find(|e| e.get("name") == name && keep(e)).map(|e| e.get("extra"))
}

/// `selectmentorspirit` / `selectparagon`: create the mentor
/// (`MentorSpirit.Create`, with its own bonus applied) and link it.
///
/// The answer is the mentor's name or id, as "Name (Extra)" when the
/// mentor's own bonus asks for something (e.g. Dragonslayer's social
/// skill); without the extra, a mentor of that name the character already
/// has supplies it. The two choices are not part of the answer: the GUI
/// asks for them afterwards and calls `items::magic::set_mentor_choices`.
fn select_mentor(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    use crate::items::magic::mentor;
    let Some(a) = ctx.answer() else { return false };
    let mtype = if node.name == "selectmentorspirit" { "MentorSpirit" } else { "Paragon" };
    let Ok(doc) = ctx.store.doc(mentor::data_file(mtype)) else { return false };
    let Some((rec, extra)) = record_and_extra(&doc, "mentors", "mentor", &a) else { return false };
    let extra = Some(extra).filter(|e| !e.is_empty()).or_else(|| existing_extra(ctx, "mentorspirits", "mentorspirit", &rec.name(), |_| true)).filter(|e| !e.is_empty());
    let guid = new_guid();
    let (el, own) = mentor::create(ctx.ch, ctx.store, rec, &guid, mtype, None, None, extra.as_deref());
    absorb(ctx, "mentorspirits", el, own, mtype);
    if let Some(i) = ctx.out.improvements.last_mut() {
        i.unique_name = rec.id();
    }
    ctx.selected = Some(rec.name());
    true
}

/// Add `count` copies of a quality granted by a bonus (`AddQuality`):
/// each runs its own bonus and is linked by a `SpecificQuality`
/// improvement. `free` qualities cost no karma and do not count toward
/// the quality limit.
fn add_quality(ctx: &mut Ctx<'_>, rec: crate::data::Record<'_>, forced: Option<&str>, count: i32, free: bool) {
    for _ in 0..count {
        let guid = new_guid();
        let mut q = crate::items::quality_element(rec, &guid, "Improvement", forced.unwrap_or(""));
        if free {
            q.set_child_text("bp", "0");
            q.set_child_text("contributetolimit", "False");
        }
        let own = own_bonus(ctx, "Quality", &guid, rec, forced);
        if let Some(sel) = &own.selected {
            q.set_child_text("extra", sel.clone());
        }
        absorb(ctx, "qualities", q, own, "SpecificQuality");
    }
}

/// `selectquality`: one of the listed qualities, added free. The answer
/// is its name, as "Name (Extra)" when the quality asks for a text.
/// Without an answer, a quality from the list that the character already
/// has from an improvement stands in for it, with its extra (Chummer5a
/// stores no answer for this prompt, so re-applying an existing item's
/// bonus finds its pick this way). Requirements of unforced options are not checked. The C# reads
/// `contributetobp` from the data record, where it never appears, so the
/// quality is always free. No data uses `<discountqualities>`; it is
/// ignored.
fn select_quality(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let names: Vec<String> = node.children_named("quality").map(Element::text).collect();
    let Ok(doc) = ctx.store.doc("qualities.xml") else { return false };
    let (rec, extra) = match ctx.answer().and_then(|a| record_and_extra(&doc, "qualities", "quality", &a)) {
        Some((r, x)) => (r, Some(x).filter(|x| !x.is_empty())),
        None => {
            let held = ctx.ch.items("qualities", "quality").into_iter().find(|q| q.get("qualitysource") == "Improvement" && names.contains(&q.get("name")));
            let Some(q) = held else { return false };
            let Some(r) = crate::data::find(&doc, "qualities", "quality", &q.get("name")) else { return false };
            // An empty saved extra is a valid (empty) text answer.
            (r, Some(q.get("extra")))
        }
    };
    let Some(child) = node.children_named("quality").find(|q| q.text() == rec.name() || q.text() == rec.id()) else { return false };
    let forced = child.attr("select").map(str::to_owned).or(extra);
    let count = child.attr("rating").filter(|r| !r.is_empty()).map_or(1, |r| ctx.int(r));
    add_quality(ctx, rec, forced.as_deref(), count, true);
    true
}

/// A new `<contact>` (`Contact.Save`) for `addcontact`.
fn contact_element(guid: &str, connection: i32, loyalty: i32, group: bool, read_only: bool) -> Element {
    let mut c = Element::new("contact");
    let mut put = |k: &str, v: &str| c.push(Element::with_text(k, v));
    for k in ["name", "role", "location"] {
        put(k, "");
    }
    put("connection", &connection.to_string());
    put("loyalty", &loyalty.to_string());
    for k in ["metatype", "gender", "age", "contacttype", "preferredpayment", "hobbiesvice", "personallife"] {
        put(k, "");
    }
    put("type", "Contact");
    for k in ["file", "relative", "notes", "groupname"] {
        put(k, "");
    }
    put("colour", "-986896");
    put("group", crate::improvement::bool_str(group).as_str());
    for k in ["family", "blackmail", "free"] {
        put(k, "False");
    }
    put("groupenabled", "True");
    put("readonly", crate::improvement::bool_str(read_only).as_str());
    put("guid", guid);
    put("mainmugshotindex", "-1");
    c.push(Element::new("mugshots"));
    c
}

/// The `forcedloyalty`, `free` and `forcegroup` improvements on a contact
/// (`addcontact`, `selectcontact`).
fn contact_modifiers(ctx: &mut Ctx<'_>, node: &Element, guid: &str) {
    if let Some(l) = node.child_text("forcedloyalty") {
        let mut i = ctx.imp("ContactForcedLoyalty", guid);
        i.val = ctx.dec(&l);
        ctx.push(i);
    }
    if node.child("free").is_some() {
        let i = ctx.imp("ContactMakeFree", guid);
        ctx.push(i);
    }
    if node.child("forcegroup").is_some() {
        let i = ctx.imp("ContactForceGroup", guid);
        ctx.push(i);
    }
}

/// `addcontact`: a new contact (read-only unless `<canwrite>`), linked by
/// an `AddContact` improvement whose unique name is the contact guid.
fn add_contact(ctx: &mut Ctx<'_>, node: &Element) {
    let loyalty = node.child_text("loyalty").map_or(1, |v| ctx.int(&v));
    let connection = node.child_text("connection").map_or(1, |v| ctx.int(&v));
    let guid = new_guid();
    let c = contact_element(&guid, connection, loyalty, node.child("group").is_some(), node.child("canwrite").is_none());
    ctx.out.added.push(("contacts".into(), c));
    let mut i = ctx.imp("AddContact", &guid);
    i.unique_name = guid.clone();
    ctx.push(i);
    contact_modifiers(ctx, node, &guid);
}

/// `selectcontact`: modifiers on one of the character's contacts (all,
/// group or non-group per `<type>`). The answer is its name or guid.
fn select_contact(ctx: &mut Ctx<'_>, node: &Element) -> bool {
    let Some(a) = ctx.answer() else { return false };
    let contacts = super::select::contact_choices(ctx.ch, node);
    let Some((guid, cname)) = contacts.into_iter().find(|(g, n)| g.eq_ignore_ascii_case(&a) || *n == a) else { return false };
    contact_modifiers(ctx, node, &guid);
    ctx.selected = Some(match ctx.selected.take().filter(|s| !s.is_empty() && *s != a) {
        Some(s) => format!("{s}, {cname}"),
        None => cname,
    });
    true
}
