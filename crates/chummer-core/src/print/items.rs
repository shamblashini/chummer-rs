//! Equipment print elements: gear, weapons, armor, cyberware and drugs.
//!
//! Cost, weight and availability totals are not ported yet, so they are
//! evaluated from the saved fields at the item's rating (own value plus
//! children). Weapon damage and accuracy resolve `STR` and `Physical`.

use super::{add, avail, bool_text, copy, copy_bool, eval, full_name, num, signed, Ctx};
use crate::expr::{self, standard_round};
use crate::xml::Element;

fn children<'a>(e: &'a Element, container: &str, item: &'a str) -> Vec<&'a Element> {
    e.child(container).map(|c| c.children_named(item).collect()).unwrap_or_default()
}

/// `guid`, `sourceid`, `name`, `name_english`: the opening of every item.
fn head(ctx: &Ctx, out: &mut Element, item: &Element, file: &str) {
    copy(out, item, "guid");
    add(out, "sourceid", source_id(item));
    add(out, "name", ctx.tr_name(file, item));
    add(out, "name_english", item.get("name"));
}

fn source_id(item: &Element) -> String {
    [item.get("sourceid"), item.get("id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default()
}

/// `fullname` and `fullname_english`.
fn full_names(ctx: &Ctx, out: &mut Element, item: &Element, file: &str, qty: Option<f64>, custom: &str) {
    let rating = item.get_i32("rating").unwrap_or(0);
    let extra = item.get("extra");
    add(out, "fullname", full_name(ctx, &ctx.tr_name(file, item), qty, rating, &extra, custom));
    add(out, "fullname_english", full_name(ctx, &item.get("name"), qty, rating, &extra, custom));
}

/// `category` and `category_english`.
fn category(ctx: &Ctx, out: &mut Element, item: &Element, file: &str) {
    let cat = item.get("category");
    add(out, "category", ctx.tr_category(file, &cat));
    add(out, "category_english", cat);
}

/// `source` (`LanguageBookShort`) and `page` (`DisplayPage`).
fn source_page(out: &mut Element, item: &Element) {
    copy(out, item, "source");
    copy(out, item, "page");
}

/// `cost`, `owncost`, `weight`, `ownweight` from the saved expressions.
fn cost_weight(ctx: &Ctx, out: &mut Element, total_cost: f64, own_cost: f64, total_weight: f64, own_weight: f64) {
    add(out, "cost", ctx.nuyen(total_cost));
    add(out, "owncost", ctx.nuyen(own_cost));
    add(out, "weight", num(total_weight));
    add(out, "ownweight", num(own_weight));
}

fn own(item: &Element, field: &str) -> f64 {
    eval(&item.get(field), item.get_i32("rating").unwrap_or(0))
}

fn qty(item: &Element) -> f64 {
    item.get_f64("qty").unwrap_or(1.0)
}

/// The twelve matrix elements (`IHasMatrixAttributes` print block).
fn matrix(out: &mut Element, item: &Element) {
    for f in ["attack", "sleaze", "dataprocessing", "firewall"] {
        add(out, f, item.get_i32(f).unwrap_or(0).to_string());
    }
    let dr = item.get_i32("devicerating").unwrap_or(0);
    add(out, "devicerating", dr.to_string());
    add(out, "programlimit", item.get_i32("programlimit").unwrap_or(0).to_string());
    add(out, "iscommlink", bool_text(is_commlink(item)));
    add(out, "isprogram", bool_text(item.get("category").ends_with("Programs")));
    copy_bool(out, item, "active");
    copy_bool(out, item, "homenode");
    add(out, "conditionmonitor", (8 + (dr + 1) / 2).to_string());
    add(out, "matrixcmfilled", item.get_i32("matrixcmfilled").unwrap_or(0).to_string());
}

fn is_commlink(item: &Element) -> bool {
    matches!(item.get("category").as_str(), "Commlinks" | "Cyberdecks" | "Rigger Command Consoles" | "Commlink Accessories")
        || item.get_bool("iscommlink").unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Gear
// ---------------------------------------------------------------------------

/// `(own, total)` cost of a gear and its children, times quantity.
fn gear_cost(item: &Element) -> (f64, f64) {
    let own_cost = own(item, "cost");
    let kids: f64 = children(item, "children", "gear").iter().map(|g| gear_cost(g).1).sum();
    (own_cost, (own_cost + kids) * qty(item))
}

fn gear_weight(item: &Element) -> (f64, f64) {
    let w = own(item, "weight");
    let kids: f64 = children(item, "children", "gear").iter().map(|g| gear_weight(g).1).sum();
    (w, (w + kids) * qty(item))
}

/// `Gear.DisplayQuantity`.
fn display_qty(ctx: &Ctx, item: &Element) -> String {
    let q = qty(item);
    if item.get("name").starts_with("Nuyen") {
        ctx.nuyen(q)
    } else if item.get("category") == "Currency" {
        format!("{q:.2}")
    } else {
        num(q)
    }
}

/// `Gear.Print`.
pub fn gear(ctx: &Ctx, item: &Element) -> Element {
    let file = "gear.xml";
    let mut out = Element::new("gear");
    head(ctx, &mut out, item, file);
    let name = item.get("name");
    let cat = item.get("category");
    full_names(ctx, &mut out, item, file, Some(qty(item)), &item.get("gearname"));
    category(ctx, &mut out, item, file);
    add(&mut out, "ispersona", bool_text(name == "Living Persona"));
    add(&mut out, "isammo", bool_text(cat == "Ammunition" || !item.get("ammoforweapontype").is_empty()));
    add(&mut out, "issin", bool_text(name == "Fake SIN" || name == "Credstick, Fake (2050)"));
    for f in ["capacity", "armorcapacity", "maxrating"] {
        copy(&mut out, item, f);
    }
    add(&mut out, "rating", item.get_i32("rating").unwrap_or(0).to_string());
    add(&mut out, "qty", display_qty(ctx, item));
    let a = avail(item);
    add(&mut out, "avail", a.clone());
    add(&mut out, "avail_english", a);
    let (oc, tc) = gear_cost(item);
    let (ow, tw) = gear_weight(item);
    cost_weight(ctx, &mut out, tc, oc, tw, ow);
    copy(&mut out, item, "extra");
    for f in ["bonded", "equipped", "wirelesson"] {
        copy_bool(&mut out, item, f);
    }
    add(&mut out, "location", ctx.location(&item.get("location")));
    copy(&mut out, item, "gearname");
    source_page(&mut out, item);
    matrix(&mut out, item);
    let mut kids = Element::new("children");
    for g in children(item, "children", "gear") {
        kids.push(gear(ctx, g));
    }
    out.push(kids);
    weapon_bonus(&mut out, item);
    ctx.notes(&mut out, item);
    out
}

/// `Gear.PrintWeaponBonusEntries` for gear that has a `<weaponbonus>`.
fn weapon_bonus(out: &mut Element, item: &Element) {
    for (node, prefix) in [("weaponbonus", "weaponbonus"), ("flechetteweaponbonus", "flechetteweaponbonus")] {
        let Some(b) = item.child(node) else { continue };
        add(out, &format!("{prefix}damage"), b.get("damage"));
        add(out, &format!("{prefix}damage_english"), b.get("damage"));
        add(out, &format!("{prefix}ap"), b.get("ap"));
        add(out, &format!("{prefix}ap_english"), b.get("ap"));
        add(out, &format!("{prefix}acc"), b.get("accuracy"));
        add(out, &format!("{prefix}range"), b.get("rangebonus"));
        add(out, &format!("{prefix}pool"), b.get("pool"));
        add(out, &format!("{prefix}smartlinkpool"), b.get("smartlinkpool"));
    }
}

/// Every `<gear>` of a list.
pub fn gear_list(ctx: &Ctx, items: &[&Element]) -> Element {
    let mut out = Element::new("gears");
    for g in items {
        out.push(gear(ctx, g));
    }
    out
}

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

/// `Weapon.GetSkillDictionaryKey`.
fn weapon_skill_key(category: &str, spec: &str) -> String {
    match category {
        "Bows" | "Crossbows" => "Archery".into(),
        "Assault Rifles" | "Carbines" | "Machine Pistols" | "Submachine Guns" => "Automatics".into(),
        "Blades" => "Blades".into(),
        "Clubs" | "Improvised Weapons" => "Clubs".into(),
        "Exotic Melee Weapons" => format!("Exotic Melee Weapon ({spec})"),
        "Exotic Ranged Weapons" | "Special Weapons" => format!("Exotic Ranged Weapon ({spec})"),
        "Flamethrowers" => "Exotic Ranged Weapon (Flamethrowers)".into(),
        "Laser Weapons" => "Exotic Ranged Weapon (Laser Weapons)".into(),
        "Assault Cannons" | "Grenade Launchers" | "Missile Launchers" | "Light Machine Guns" | "Medium Machine Guns" | "Heavy Machine Guns" => {
            "Heavy Weapons".into()
        }
        "Shotguns" | "Sniper Rifles" | "Sporting Rifles" => "Longarms".into(),
        "Throwing Weapons" => "Throwing Weapons".into(),
        "Unarmed" => "Unarmed Combat".into(),
        _ => "Pistols".into(),
    }
}

/// `Weapon.Skill`: `<useskill>` if set, else by category.
fn weapon_skill(item: &Element) -> String {
    let use_skill = item.get("useskill");
    if !use_skill.is_empty() {
        return use_skill;
    }
    let cat = item.get("category");
    let cat = if cat.is_empty() || item.get("type") == "Melee" && cat == "Gear" { "Clubs".to_owned() } else { cat };
    weapon_skill_key(&cat, &item.get("useskillspec"))
}

/// `Weapon.GetDicePool`: the skill's pool, plus its specialization bonus
/// when the weapon matches a specialization.
fn dice_pool(ctx: &Ctx, item: &Element, skill: &str) -> i32 {
    let Some(sv) = ctx.sheet.skills.iter().find(|s| s.name == skill) else { return 0 };
    let spec_hit = sv.specs.iter().any(|s| *s == item.get("category") || *s == item.get("name") || *s == item.get("spec"));
    let mut pool = sv.pool;
    if spec_hit {
        pool += sv.spec_bonus;
    }
    pool + standard_round(item.child("weaponbonus").and_then(|b| b.get_f64("pool")).unwrap_or(0.0))
}

/// Substitute attribute names in a weapon formula: `(STR+2)` -> `(5+2)`.
fn substitute(ctx: &Ctx, s: &str) -> String {
    let mut out = s.replace(['{', '}'], "");
    for a in ["STR", "AGI", "BOD", "REA", "LOG", "WIL", "INT", "CHA", "MAG", "RES"] {
        out = out.replace(a, &ctx.sheet.attr(a).to_string());
    }
    out.replace("Rating", "0")
}

/// `Weapon.CalculatedDamage`, common case: evaluate a leading
/// parenthesised expression (`(STR+2)P` -> `7P`).
fn damage(ctx: &Ctx, raw: &str) -> String {
    let raw = raw.trim();
    if !raw.starts_with('(') {
        return raw.to_owned();
    }
    let Some(close) = raw.find(')') else { return raw.to_owned() };
    match expr::evaluate_num(&substitute(ctx, &raw[1..close])) {
        Ok(v) => format!("{}{}", standard_round(v), &raw[close + 1..]),
        Err(_) => raw.to_owned(),
    }
}

/// `Weapon.GetAccuracy`: `Physical`/`Missile` resolve to the limit.
fn accuracy(ctx: &Ctx, raw: &str) -> String {
    let s = raw.trim();
    let s = s.replace("Physical", &ctx.sheet.limit_physical.to_string()).replace("Missile", &ctx.sheet.limit_physical.to_string());
    if expr::needs_evaluation(&s) {
        return expr::evaluate_num(&substitute(ctx, &s)).map(|v| standard_round(v).to_string()).unwrap_or(s);
    }
    s
}

/// Range bands from `ranges.xml` (`Weapon.RangeShort` and so on).
fn ranges(ctx: &Ctx, item: &Element, element: &str, alternate: bool) -> Element {
    let mut out = Element::new(element);
    let key = if alternate { item.get("alternaterange") } else { [item.get("range"), item.get("category")].into_iter().find(|s| !s.is_empty()).unwrap_or_default() };
    let rec = ctx.engine.store.doc("ranges.xml").ok().and_then(|d| {
        d.child("ranges").and_then(|r| r.children_named("range").find(|x| x.get("name") == key).cloned())
    });
    let mult = item.get_f64("rangemultiply").unwrap_or(1.0).max(1.0);
    let band = |f: &str| -> i32 {
        rec.as_ref().map_or(-1, |r| {
            let s = r.get(f);
            if s.trim().is_empty() {
                return -1;
            }
            standard_round(expr::evaluate_num(&substitute(ctx, &s)).unwrap_or(-1.0) * if f == "min" { 1.0 } else { mult })
        })
    };
    let [min, short, medium, long, extreme] = ["min", "short", "medium", "long", "extreme"].map(band);
    add(&mut out, "name", ctx.lang.data_name("ranges.xml", "", &key));
    add(&mut out, "name_english", key);
    let fmt = |lo: i32, hi: i32| if lo < 0 || hi < 0 { String::new() } else { format!("{lo}-{hi}") };
    add(&mut out, "short", fmt(min, short));
    add(&mut out, "medium", fmt(short + 1, medium));
    add(&mut out, "long", fmt(medium + 1, long));
    add(&mut out, "extreme", fmt(long + 1, extreme));
    out
}

/// The four `x`, `x_noammo`, `x_english`, `x_english_noammo` variants.
fn four(out: &mut Element, field: &str, v: &str, damage_order: bool) {
    add(out, field, v);
    add(out, &format!("{field}_noammo"), v);
    if damage_order {
        add(out, &format!("{field}_english"), v);
        add(out, &format!("{field}_noammo_english"), v);
    } else {
        add(out, &format!("{field}_english"), v);
        add(out, &format!("{field}_english_noammo"), v);
    }
}

/// `Weapon.Print`.
pub fn weapon(ctx: &Ctx, item: &Element) -> Element {
    let file = "weapons.xml";
    let mut out = Element::new("weapon");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, &item.get("weaponname"));
    category(ctx, &mut out, item, file);
    copy(&mut out, item, "type");
    let reach = item.get_i32("reach").unwrap_or(0) + ctx.ch.improvements.val_int("Reach", None) * i32::from(item.get("type") == "Melee");
    add(&mut out, "reach", reach.to_string());
    add(&mut out, "rawreach", item.get("reach"));
    let acc = accuracy(ctx, &item.get("accuracy"));
    for f in ["accuracy", "accuracy_noammo", "accuracy_english", "accuracy_english_noammo"] {
        add(&mut out, f, acc.clone());
    }
    add(&mut out, "rawaccuracy", item.get("accuracy"));
    four(&mut out, "damage", &damage(ctx, &item.get("damage")), true);
    add(&mut out, "rawdamage", item.get("damage"));
    four(&mut out, "ap", &item.get("ap"), false);
    add(&mut out, "rawap", item.get("ap"));
    four(&mut out, "mode", &item.get("mode"), false);
    four(&mut out, "rc", &item.get("rc"), false);
    add(&mut out, "rawrc", item.get("rc"));
    add(&mut out, "ammo", item.get("ammo"));
    add(&mut out, "ammo_english", item.get("ammo"));
    add(&mut out, "maxammo", item.get("ammo"));
    let conceal = item.get_f64("conceal").unwrap_or(0.0);
    add(&mut out, "conceal", signed(conceal));
    add(&mut out, "rawconceal", item.get("conceal"));
    add(&mut out, "availablemounts", item.get("mount"));
    add(&mut out, "availablemounts_english", item.get("mount"));
    let a = avail(item);
    add(&mut out, "avail", a.clone());
    add(&mut out, "avail_english", a);
    let accessories = children(item, "accessories", "accessory");
    let own_cost = own(item, "cost");
    let acc_cost: f64 = accessories.iter().filter(|x| !x.get_bool("included").unwrap_or(false)).map(|x| own(x, "cost")).sum();
    let own_weight = own(item, "weight");
    cost_weight(ctx, &mut out, own_cost + acc_cost, own_cost, own_weight, own_weight);
    source_page(&mut out, item);
    copy(&mut out, item, "weaponname");
    add(&mut out, "location", ctx.location(&item.get("location")));
    matrix(&mut out, item);
    if !accessories.is_empty() {
        let mut list = Element::new("accessories");
        for x in accessories {
            list.push(accessory(ctx, x));
        }
        out.push(list);
    }
    out.push(ranges(ctx, item, "ranges", false));
    out.push(ranges(ctx, item, "alternateranges", true));
    for u in children(item, "underbarrel", "weapon") {
        let mut wrap = Element::new("underbarrel");
        wrap.push(weapon(ctx, u));
        out.push(wrap);
    }
    add(&mut out, "availableammo", "0");
    add(&mut out, "currentammo", "");
    add(&mut out, "currentammo_english", "");
    out.push(Element::new("clips"));
    let skill = weapon_skill(item);
    let pool = dice_pool(ctx, item, &skill).to_string();
    add(&mut out, "dicepool", pool.clone());
    add(&mut out, "dicepool_noammo", pool);
    add(&mut out, "skill", skill);
    copy_bool(&mut out, item, "wirelesson");
    ctx.notes(&mut out, item);
    out
}

/// `WeaponAccessory.Print`.
fn accessory(ctx: &Ctx, item: &Element) -> Element {
    let file = "weapons.xml";
    let mut out = Element::new("accessory");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, "");
    for f in ["mount", "extramount", "addmount"] {
        copy(&mut out, item, f);
    }
    for f in ["damage", "rc", "ap", "conceal"] {
        add(&mut out, f, signed(item.get_f64(f).unwrap_or(0.0)));
    }
    add(&mut out, "avail", avail(item));
    copy(&mut out, item, "ratinglabel");
    let c = own(item, "cost");
    let w = own(item, "weight");
    cost_weight(ctx, &mut out, c, c, w, w);
    copy_bool(&mut out, item, "included");
    source_page(&mut out, item);
    add(&mut out, "accuracy", signed(item.get_f64("accuracy").unwrap_or(0.0)));
    let gears = children(item, "gears", "gear");
    if !gears.is_empty() {
        out.push(gear_list(ctx, &gears));
    }
    ctx.notes(&mut out, item);
    out
}

// ---------------------------------------------------------------------------
// Armor
// ---------------------------------------------------------------------------

/// `Armor.GetDisplayArmorValue`: own value plus equipped mods.
fn armor_value(item: &Element) -> String {
    let raw = item.child_text("armoroverride").filter(|s| !s.trim().is_empty() && s.trim() != "0").unwrap_or_else(|| item.get("armor"));
    let stacking = raw.trim().starts_with('+');
    let mut v = expr::parse_plain(raw.trim().trim_start_matches('+')).unwrap_or(0.0) as i32 - item.get_i32("damage").unwrap_or(0);
    for m in children(item, "armormods", "armormod").into_iter().filter(|m| m.get_bool("equipped").unwrap_or(true)) {
        v += expr::value_to_int(&m.get("armor"), m.get_i32("rating").unwrap_or(0), &expr::NoAttributes);
    }
    if stacking { format!("+{v}") } else { v.to_string() }
}

/// `Armor.Print`.
pub fn armor(ctx: &Ctx, item: &Element) -> Element {
    let file = "armor.xml";
    let mut out = Element::new("armor");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, &item.get("armorname"));
    category(ctx, &mut out, item, file);
    add(&mut out, "armor", armor_value(item));
    let mods = children(item, "armormods", "armormod");
    let capacity = item.get("armorcapacity");
    add(&mut out, "totalarmorcapacity", capacity.clone());
    add(&mut out, "calculatedcapacity", capacity.clone());
    let used: f64 = mods.iter().filter(|m| !m.get_bool("included").unwrap_or(false)).map(|m| own(m, "armorcapacity").abs()).sum();
    add(&mut out, "capacityremaining", num(eval(&capacity, 0) - used));
    add(&mut out, "avail", avail(item));
    let own_cost = own(item, "cost");
    let mod_cost: f64 = mods.iter().filter(|m| !m.get_bool("included").unwrap_or(false)).map(|m| own(m, "cost")).sum();
    let w = own(item, "weight");
    cost_weight(ctx, &mut out, own_cost + mod_cost, own_cost, w, w);
    source_page(&mut out, item);
    copy(&mut out, item, "armorname");
    copy_bool(&mut out, item, "equipped");
    copy(&mut out, item, "ratinglabel");
    copy_bool(&mut out, item, "wirelesson");
    let mut list = Element::new("armormods");
    for m in &mods {
        list.push(armor_mod(ctx, m));
    }
    out.push(list);
    out.push(gear_list(ctx, &children(item, "gears", "gear")));
    copy(&mut out, item, "extra");
    add(&mut out, "location", ctx.location(&item.get("location")));
    matrix(&mut out, item);
    ctx.notes(&mut out, item);
    out
}

/// `ArmorMod.Print`.
fn armor_mod(ctx: &Ctx, item: &Element) -> Element {
    let file = "armor.xml";
    let mut out = Element::new("armormod");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, "");
    category(ctx, &mut out, item, file);
    add(&mut out, "armor", item.get_i32("armor").unwrap_or(0).to_string());
    add(&mut out, "maxrating", item.get_i32("maxrating").unwrap_or(0).to_string());
    add(&mut out, "rating", item.get_i32("rating").unwrap_or(0).to_string());
    copy(&mut out, item, "ratinglabel");
    add(&mut out, "avail", avail(item));
    let c = own(item, "cost");
    let w = own(item, "weight");
    cost_weight(ctx, &mut out, c, c, w, w);
    source_page(&mut out, item);
    copy_bool(&mut out, item, "included");
    copy_bool(&mut out, item, "equipped");
    copy_bool(&mut out, item, "wirelesson");
    out.push(gear_list(ctx, &children(item, "gears", "gear")));
    copy(&mut out, item, "extra");
    ctx.notes(&mut out, item);
    out
}

// ---------------------------------------------------------------------------
// Cyberware
// ---------------------------------------------------------------------------

/// `Cyberware.Print`.
pub fn cyberware(ctx: &Ctx, item: &Element) -> Element {
    let file = if item.get("improvementsource") == "Bioware" { "bioware.xml" } else { "cyberware.xml" };
    let mut out = Element::new("cyberware");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, "");
    category(ctx, &mut out, item, file);
    let attrs = crate::calc::SheetAttributes(&ctx.sheet.attributes);
    let ess = crate::calc::ware_essence(ctx.ch, item, &attrs, Some(&ctx.engine.store), &ctx.rules, None);
    add(&mut out, "ess", ctx.essence(ess));
    copy(&mut out, item, "capacity");
    add(&mut out, "avail", avail(item));
    let own_cost = own(item, "cost");
    let kids = children(item, "children", "cyberware");
    let kid_cost: f64 = kids.iter().map(|k| own(k, "cost")).sum();
    let w = own(item, "weight");
    cost_weight(ctx, &mut out, own_cost + kid_cost, own_cost, w, w);
    source_page(&mut out, item);
    for f in ["rating", "minrating", "maxrating"] {
        add(&mut out, f, item.get_i32(f).unwrap_or(0).to_string());
    }
    copy(&mut out, item, "ratinglabel");
    add(&mut out, "allowsubsystems", item.get("subsystems"));
    copy_bool(&mut out, item, "wirelesson");
    let grade = item.get("grade");
    add(&mut out, "grade", ctx.lang.data_name(file, "", &grade));
    copy(&mut out, item, "location");
    copy(&mut out, item, "extra");
    add(&mut out, "improvementsource", if file == "bioware.xml" { "Bioware" } else { "Cyberware" });
    copy_bool(&mut out, item, "isgeneware");
    matrix(&mut out, item);
    let gears = children(item, "gears", "gear");
    if !gears.is_empty() {
        out.push(gear_list(ctx, &gears));
    }
    let mut list = Element::new("children");
    for k in kids {
        list.push(cyberware(ctx, k));
    }
    out.push(list);
    ctx.notes(&mut out, item);
    out
}

// ---------------------------------------------------------------------------
// Drugs
// ---------------------------------------------------------------------------

/// `Drug.Print`, saved values only.
pub fn drug(ctx: &Ctx, item: &Element) -> Element {
    let file = "drugcomponents.xml";
    let mut out = Element::new("drug");
    head(ctx, &mut out, item, file);
    category(ctx, &mut out, item, file);
    if !item.get("grade").is_empty() {
        copy(&mut out, item, "grade");
    }
    add(&mut out, "qty", num(qty(item)));
    for f in ["addictionthreshold", "addictionrating", "initiative", "initiativedice", "speed"] {
        add(&mut out, f, item.get_i32(f).unwrap_or(0).to_string());
    }
    copy(&mut out, item, "duration");
    add(&mut out, "duration_english", item.get("duration"));
    add(&mut out, "crashdamage", item.get_i32("crashdamage").unwrap_or(0).to_string());
    let a = avail(item);
    add(&mut out, "avail", a.clone());
    add(&mut out, "avail_english", a);
    add(&mut out, "cost", ctx.nuyen(own(item, "cost")));
    ctx.notes(&mut out, item);
    out
}
