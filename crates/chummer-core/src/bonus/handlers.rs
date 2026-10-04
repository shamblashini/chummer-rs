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
            for aq in node.children_named("addquality") {
                let forced = aq.attr("select").map(str::to_owned);
                let count = aq.attr("rating").map(|r| ctx.int(r)).unwrap_or(1).max(1);
                let free = !aq.attr("contributetobp").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                let Ok(doc) = ctx.store.doc("qualities.xml") else { return false };
                let Some(rec) = crate::data::find(&doc, "qualities", "quality", &aq.text()) else { return false };
                for _ in 0..count {
                    let guid = new_guid();
                    let mut q = crate::items::quality_element(rec, &guid, "Improvement", forced.as_deref().unwrap_or(""));
                    if free {
                        q.set_child_text("bp", "0");
                        q.set_child_text("contributetolimit", "False");
                    }
                    // The added quality runs its own bonus, sourced to itself.
                    if let Some(b) = rec.el().child("bonus") {
                        let src = super::BonusSource { kind: "Quality".into(), guid: guid.clone(), name: rec.name(), rating: 1 };
                        let inner = super::apply(ctx.ch, ctx.store, b, &src, forced.as_deref());
                        if let Some(sel) = &inner.selected {
                            q.set_child_text("extra", sel.clone());
                        }
                        ctx.out.improvements.extend(inner.improvements);
                        ctx.out.added.extend(inner.added);
                        ctx.out.flags.extend(inner.flags);
                        ctx.out.unsupported.extend(inner.unsupported);
                    }
                    ctx.out.added.push(("qualities".into(), q));
                    let i = ctx.imp("SpecificQuality", &guid);
                    ctx.push(i);
                }
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
        "selectquality" => return crate::items::quality::bonus_selectquality(ctx, node),
        "addcontact" => return crate::items::quality::bonus_addcontact(ctx, node),
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
        "selectmentorspirit" | "selectparagon" => {
            let Some(a) = ctx.answer() else { return false };
            let (file, kind, tag) = if name == "selectmentorspirit" { ("mentors.xml", "MentorSpirit", "mentorspirit") } else { ("paragons.xml", "Paragon", "mentorspirit") };
            let Ok(doc) = ctx.store.doc(file) else { return false };
            // The extra may carry a choice suffix, e.g. "Raven (Alt)".
            let rec = doc.child("mentors").and_then(|m| {
                m.children_named("mentor")
                    .find(|e| e.get("name") == a || e.get("id").eq_ignore_ascii_case(&a))
                    .or_else(|| m.children_named("mentor").find(|e| a.starts_with(&format!("{} (", e.get("name")))))
            });
            let Some(rec) = rec else { return false };
            let guid = new_guid();
            let mut m = Element::new(tag);
            m.push(Element::with_text("guid", guid.clone()));
            m.push(Element::with_text("id", rec.get("id")));
            m.push(Element::with_text("name", rec.get("name")));
            m.push(Element::with_text("mentortype", kind));
            m.push(Element::with_text("source", rec.get("source")));
            m.push(Element::with_text("page", rec.get("page")));
            m.push(Element::with_text("advantage", rec.get("advantage")));
            m.push(Element::with_text("disadvantage", rec.get("disadvantage")));
            if let Some(b) = rec.child("bonus") {
                m.push(b.clone());
            }
            m.push(Element::new("notes"));
            ctx.out.added.push(("mentorspirits".into(), m));
            let mut i = ctx.imp(kind, &guid);
            i.unique_name = rec.get("id");
            ctx.push(i);
            ctx.selected = Some(rec.get("name"));
        }
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
