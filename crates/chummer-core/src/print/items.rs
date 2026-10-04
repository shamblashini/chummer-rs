//! Equipment print elements: gear, weapons, armor, cyberware and drugs.
//!
//! Costs come from `items::{gear, weapon, armor, cyberware, drug}::cost`,
//! weapon combat values from `items::weapon::stats_with`. Weights are the
//! saved `<weight>` or, for saves that do not keep it, the data record's.

use std::borrow::Cow;

use super::{add, bool_text, copy, copy_bool, eval, full_name, num, own_avail, signed, total_avail, weight, Ctx};
use crate::expr::{self, standard_round, Availability};
use crate::items::{armor as armor_calc, cyberware as ware_calc, gear as gear_calc, weapon as weapon_calc};
use crate::xml::{Element, Node};

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

/// `cost`, `owncost` (nuyen format), `weight`, `ownweight` (weight format).
fn cost_weight(ctx: &Ctx, out: &mut Element, total_cost: f64, own_cost: f64, total_weight: f64, own_weight: f64) {
    add(out, "cost", ctx.nuyen(total_cost));
    add(out, "owncost", ctx.nuyen(own_cost));
    add(out, "weight", weight(total_weight));
    add(out, "ownweight", weight(own_weight));
}

fn qty(item: &Element) -> f64 {
    item.get_f64("qty").unwrap_or(1.0)
}

fn equipped(e: &Element) -> bool {
    e.get_bool("equipped").unwrap_or(true)
}

fn included(e: &Element) -> bool {
    e.get_bool("included").unwrap_or(false)
}

/// A field of the item's data record (`GetNodeCore`), by sourceid then name.
pub(crate) fn record_field(ctx: &Ctx, file: &str, container: &str, tag: &str, saved: &Element, field: &str) -> Option<String> {
    let doc = ctx.store.doc(file).ok()?;
    let id = source_id(saved);
    let rec = (!id.is_empty())
        .then(|| crate::data::find(&doc, container, tag, &id))
        .flatten()
        .or_else(|| crate::data::find(&doc, container, tag, &saved.get("name")))?;
    rec.el().child_text(field)
}

/// The saved `<weight>`, else the data record's (saves from before
/// Chummer kept weights load them from the data file).
fn weight_expr(ctx: &Ctx, item: &Element, file: &str, container: &str, tag: &str) -> String {
    item.child_text("weight").filter(|w| !w.trim().is_empty()).or_else(|| record_field(ctx, file, container, tag, item, "weight")).unwrap_or_default()
}

/// Evaluated weight expression at the item's rating.
fn own_weight_of(ctx: &Ctx, item: &Element, file: &str, container: &str, tag: &str) -> f64 {
    eval(&weight_expr(ctx, item, file, container, tag), item.get_i32("rating").unwrap_or(0))
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

/// Find an element named `tag` with this guid anywhere under `e`.
pub(crate) fn find_tagged<'a>(e: &'a Element, tag: &str, guid: &str) -> Option<&'a Element> {
    if e.name == tag && e.get("guid").eq_ignore_ascii_case(guid) {
        return Some(e);
    }
    e.elements().filter(|c| c.name != "improvements").find_map(|c| find_tagged(c, tag, guid))
}

/// `e` without the direct children named in `names`.
fn without(e: &Element, names: &[&str]) -> Element {
    let mut c = e.clone();
    c.children.retain(|n| !matches!(n, Node::Element(x) if names.contains(&x.name.as_str())));
    c
}

// ---------------------------------------------------------------------------
// Gear
// ---------------------------------------------------------------------------

/// The parent a gear's cost is computed against.
#[derive(Clone, Copy)]
pub enum GearParent<'a> {
    /// Top level of a gear list.
    None,
    /// A gear or armor (`IHasChildrenAndCost<Gear>`): its rating and child
    /// cost multiplier apply.
    CostParent(&'a Element),
    /// Cyberware, an armor mod, a weapon accessory, a vehicle: the parent
    /// rating applies, but `Gear.OwnCost` falls back to `1 / CostFor`.
    Other(&'a Element),
}

impl<'a> GearParent<'a> {
    fn element(self) -> Option<&'a Element> {
        match self {
            GearParent::None => None,
            GearParent::CostParent(p) | GearParent::Other(p) => Some(p),
        }
    }
}

/// `Gear.TotalAvailTuple`.
pub fn gear_avail(ctx: &Ctx, item: &Element) -> Availability {
    let guid = item.get("guid");
    let kids: Vec<Availability> =
        children(item, "children", "gear").into_iter().filter(|k| !k.get("parentid").eq_ignore_ascii_case(&guid)).map(|k| gear_avail(ctx, k)).collect();
    let mut own = own_avail(item);
    own.add_to_parent = own.add_to_parent && !item.get_bool("includedinparent").unwrap_or(false);
    total_avail(ctx, item, own, &kids)
}

/// A gear element with its weight filled from the data record when the
/// save has none (children too).
fn with_weight<'a>(ctx: &Ctx, item: &'a Element) -> Cow<'a, Element> {
    let has = item.child_text("weight").is_some_and(|w| !w.trim().is_empty());
    let from_data = if has { None } else { record_field(ctx, "gear.xml", "gears", "gear", item, "weight").filter(|w| !w.trim().is_empty()) };
    let kids_need = children(item, "children", "gear").iter().any(|k| matches!(with_weight(ctx, k), Cow::Owned(_)));
    if from_data.is_none() && !kids_need {
        return Cow::Borrowed(item);
    }
    let mut e = item.clone();
    if let Some(w) = from_data {
        e.remove_children("weight");
        e.push(Element::with_text("weight", w));
    }
    if let Some(kids) = e.child_mut("children") {
        for k in kids.children.iter_mut() {
            if let Node::Element(k) = k {
                *k = with_weight(ctx, k).into_owned();
            }
        }
    }
    Cow::Owned(e)
}

/// `Gear.TotalWeight` of a gear and its children, data-record weights included.
pub fn gear_total_weight(ctx: &Ctx, item: &Element, parent: Option<&Element>) -> f64 {
    let w = with_weight(ctx, item);
    gear_calc::total_weight(&w, parent)
}

/// `Gear.TotalCost`.
pub fn gear_total_cost(item: &Element, parent: GearParent) -> f64 {
    match parent.element() {
        Some(p) => gear_calc::cost_in(item, p),
        None => gear_calc::cost(item),
    }
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
pub fn gear(ctx: &Ctx, item: &Element, parent: GearParent) -> Element {
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
    ctx.add_avail(&mut out, gear_avail(ctx, item), true);
    // `Gear.OwnCost` is `(pre * Parent?.ChildCostMultiplier ?? 1) / CostFor`:
    // without a gear or armor parent the product is null, so Chummer
    // prints `1 / CostFor`.
    let own_cost = match parent {
        GearParent::CostParent(p) => {
            gear_calc::own_cost_pre_multipliers(item, Some(p)) * f64::from(p.get_i32("childcostmultiplier").unwrap_or(1)) / gear_calc::cost_for_units(item)
        }
        _ => 1.0 / gear_calc::cost_for_units(item),
    };
    let w = with_weight(ctx, item);
    let own_w = gear_calc::own_weight(&w, parent.element());
    cost_weight(ctx, &mut out, gear_total_cost(item, parent), own_cost, gear_calc::total_weight(&w, parent.element()), own_w);
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
        kids.push(gear(ctx, g, GearParent::CostParent(item)));
    }
    out.push(kids);
    weapon_bonus(ctx, &mut out, item, false);
    ctx.notes(&mut out, item);
    out
}

/// `Gear.PrintWeaponBonusEntries`: the damage, AP, accuracy, range and
/// pool bonuses of ammunition. `force` prints both blocks even when the
/// gear has none (`Clip.Print`).
fn weapon_bonus(ctx: &Ctx, out: &mut Element, item: &Element, force: bool) {
    for prefix in ["weaponbonus", "flechetteweaponbonus"] {
        let b = item.child(prefix).filter(|b| b.elements().next().is_some());
        if b.is_none() && !force {
            continue;
        }
        let get = |f: &str| b.map(|b| b.get(f)).unwrap_or_default();
        add(out, &format!("{prefix}damage"), bonus_damage(ctx, &get("damage"), &get("damagetype"), &get("damagereplace"), false));
        add(out, &format!("{prefix}damage_english"), bonus_damage(ctx, &get("damage"), &get("damagetype"), &get("damagereplace"), true));
        let ap = Some(get("apreplace")).filter(|s| !s.is_empty()).unwrap_or_else(|| signed_text(&get("ap")));
        add(out, &format!("{prefix}ap"), localize_damage(ctx, &ap, false));
        add(out, &format!("{prefix}ap_english"), ap);
        add(out, &format!("{prefix}acc"), signed_text(&get("accuracy")));
        add(out, &format!("{prefix}range"), signed_text(&get("rangebonus")));
        add(out, &format!("{prefix}pool"), signed_text(&get("pool")));
        add(out, &format!("{prefix}smartlinkpool"), signed_text(&get("smartlinkpool")));
    }
}

/// `+#,0.##;-#,0.##;0` of a saved number, or the text as is.
fn signed_text(s: &str) -> String {
    match expr::parse_plain(s.trim()) {
        Some(v) => signed(v),
        None if s.trim().is_empty() => "0".into(),
        None => s.to_owned(),
    }
}

/// `Gear.WeaponBonusDamage`: `damagereplace`, else signed damage plus the
/// damage type.
fn bonus_damage(ctx: &Ctx, damage: &str, kind: &str, replace: &str, english: bool) -> String {
    let s = if !replace.is_empty() {
        replace.to_owned()
    } else {
        let mut s = signed_text(damage);
        if s == "0" && !kind.is_empty() {
            s.clear();
        }
        s.push_str(kind);
        s
    };
    localize_damage(ctx, &s, english)
}

/// Every `<gear>` of a list.
pub fn gear_list(ctx: &Ctx, items: &[&Element], parent: GearParent) -> Element {
    let mut out = Element::new("gears");
    for g in items {
        out.push(gear(ctx, g, parent));
    }
    out
}

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

/// `Weapon.ReplaceStrings` for a non-English print language.
fn replace_strings(ctx: &Ctx, s: &str) -> String {
    let l = ctx.lang;
    let mut out = s.to_owned();
    for (from, key) in [
        ("Special", "String_DamageSpecial"),
        ("P or S", "String_DamagePOrS"),
        ("Chemical", "String_DamageChemical"),
        ("(e)", "String_DamageElectric"),
        ("(f)", "String_DamageFlechette"),
        ("(fire)", "String_DamageFire"),
        ("Grenade", "String_DamageGrenade"),
        ("Missile", "String_DamageMissile"),
        ("Mortar", "String_DamageMortar"),
        ("Rocket", "String_DamageRocket"),
        ("Torpedo", "String_DamageTorpedo"),
        ("Radius", "String_DamageRadius"),
        ("As Drug/Toxin", "String_DamageAsDrugToxin"),
        ("as round", "String_DamageAsRound"),
    ] {
        if out.contains(from) {
            out = out.replace(from, &l.s(key));
        }
    }
    if out.contains("/m") {
        out = out.replace("/m", &format!("/{}", l.s("String_DamageMeter")));
    }
    if out.contains("(M)") {
        out = out.replace("(M)", &l.s("String_DamageMatrix"));
    }
    out
}

/// `Weapon.ReplaceDamageStrings`: [`replace_strings`] plus the `nS`/`nP`
/// damage codes. English text is returned unchanged.
fn localize_damage(ctx: &Ctx, s: &str, english: bool) -> String {
    if english || ctx.is_english() {
        return s.to_owned();
    }
    let mut out = replace_strings(ctx, s);
    let stun = ctx.s("String_DamageStun");
    let phys = ctx.s("String_DamagePhysical");
    for d in 0..=9 {
        out = out.replace(&format!("{d}S"), &format!("{d}{stun}"));
    }
    for d in 0..=9 {
        out = out.replace(&format!("{d}P"), &format!("{d}{phys}"));
    }
    out
}

/// `Weapon.TotalAP` localized: only non-numeric AP is translated.
fn localize_ap(ctx: &Ctx, ap: &str, english: bool) -> String {
    if english || ctx.is_english() || ap == "-" || expr::parse_plain(ap.trim_start_matches('+')).is_some() {
        return ap.to_owned();
    }
    replace_strings(ctx, &ap.replace("-half", &ctx.s("String_APHalf")))
}

/// `Weapon.GetAccuracy`: `"raw (total)"` when a numeric raw accuracy differs.
fn accuracy_text(ctx: &Ctx, raw: &str, total: i32, english: bool) -> String {
    match raw.trim().parse::<i32>() {
        Ok(r) if r != total => format!("{r}{}({total})", ctx.space(english)),
        _ => total.to_string(),
    }
}

/// Mode codes in a `firemode`/`modereplace` value.
fn split_modes(s: &str) -> impl Iterator<Item = &str> {
    s.split('/').map(str::trim).filter(|m| !m.is_empty())
}

/// Apply `firemode` (added) and `replace_tag` (replacing all modes) of
/// one bonus or accessory node.
fn apply_modes(node: &Element, replace_tag: &str, modes: &mut Vec<String>, new_modes: &mut Vec<String>) {
    new_modes.extend(split_modes(&node.get("firemode")).map(str::to_owned));
    let replace = node.get(replace_tag);
    if !replace.trim().is_empty() {
        modes.clear();
        modes.extend(split_modes(&replace).map(str::to_owned));
    }
}

/// The gear loaded in the weapon's active clip (`Weapon.AmmoLoaded`).
fn loaded_ammo<'a>(ctx: &'a Ctx, w: &Element) -> Option<&'a Element> {
    let slot = w.get_i32("activeammoslot").unwrap_or(1).max(1) as usize;
    let clip = w.child("clips")?.children_named("clip").nth(slot - 1)?;
    let id = clip.get("id");
    if id.is_empty() || id.starts_with("00000000") {
        return None;
    }
    find_tagged(&ctx.ch.doc, "gear", &id)
}

/// `Weapon.CalculatedMode`: base modes plus wireless, accessory and
/// ammunition changes, in SS/SA/BF/FA/Special order.
fn calculated_mode(ctx: &Ctx, w: &Element, include_ammo: bool, english: bool) -> String {
    let mut modes: Vec<String> = split_modes(&w.get("mode")).map(str::to_owned).collect();
    let mut new_modes = Vec::new();
    let wireless = w.get_bool("wirelesson").unwrap_or(true);
    if wireless {
        if let Some(b) = w.child("wirelessweaponbonus") {
            apply_modes(b, "modereplace", &mut modes, &mut new_modes);
        }
    }
    for a in children(w, "accessories", "accessory").into_iter().filter(|a| equipped(a)) {
        apply_modes(a, "firemodereplace", &mut modes, &mut new_modes);
        if wireless && a.get_bool("wirelesson").unwrap_or(false) {
            if let Some(b) = a.child("wirelessweaponbonus") {
                apply_modes(b, "modereplace", &mut modes, &mut new_modes);
            }
        }
    }
    if include_ammo {
        if let Some(g) = loaded_ammo(ctx, w) {
            let flechette = w.get("damage").contains("(f)") && w.get("ammocategory") != "Gear";
            let pick = |g: &Element| -> Option<Element> {
                let fb = g.child("flechetteweaponbonus").filter(|b| b.elements().next().is_some());
                if flechette && fb.is_some() {
                    return fb.cloned();
                }
                g.child("weaponbonus").filter(|b| b.elements().next().is_some()).cloned()
            };
            if let Some(b) = pick(g) {
                apply_modes(&b, "modereplace", &mut modes, &mut new_modes);
            }
            for k in children(g, "children", "gear").into_iter().filter(|k| equipped(k)) {
                if let Some(b) = pick(k) {
                    apply_modes(&b, "modereplace", &mut modes, &mut new_modes);
                }
            }
        }
    }
    modes.extend(new_modes);
    let l = ctx.strings(english);
    [("SS", "String_ModeSingleShot"), ("SA", "String_ModeSemiAutomatic"), ("BF", "String_ModeBurstFire"), ("FA", "String_ModeFullAutomatic"), ("Special", "String_ModeSpecial")]
        .into_iter()
        .filter(|(m, _)| modes.iter().any(|x| x == m))
        .map(|(_, k)| l.s(k))
        .collect::<Vec<_>>()
        .join("/")
}

/// Byte length of the char at `p` (for `x` / `×`).
fn char_len(s: &str, p: usize) -> usize {
    s[p..].chars().next().map_or(1, char::len_utf8)
}

/// `Weapon.AmmoCapacity`: strip a numeric `Nx` prefix, an `xN` suffix
/// and the clip type.
fn ammo_capacity(ammo: &str) -> String {
    let mut s = ammo.to_owned();
    if let Some(p) = s.find(['x', '×']) {
        if s[..p].chars().all(|c| c.is_ascii_digit()) {
            s = s[p + char_len(&s, p)..].to_owned();
        }
        if let Some(p) = s.rfind(['x', '×']) {
            if s[p + char_len(&s, p)..].chars().all(|c| c.is_ascii_digit()) {
                s.truncate(p);
            }
        }
    }
    match s.find('(') {
        Some(p) => s[..p].to_owned(),
        None => s,
    }
}

fn split_ammo(s: &str) -> Vec<String> {
    s.split(' ').filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// `Weapon.CalculatedAmmo`: each ammo entry with accessory and weapon
/// mount bonuses applied, then translated.
fn calculated_ammo(ctx: &Ctx, w: &Element, mount: Option<&Element>, english: bool) -> String {
    let base = w.get("ammo");
    let mut ammos = split_ammo(&base);
    let accessories: Vec<&Element> = children(w, "accessories", "accessory").into_iter().filter(|a| equipped(a)).collect();
    let mut bonus = 0.0;
    for a in &accessories {
        let replace = a.get("ammoreplace");
        if !replace.is_empty() {
            ammos = split_ammo(&replace);
        }
        bonus += eval(&a.get("ammobonus"), a.get_i32("rating").unwrap_or(0));
    }
    let mut flat = 0.0;
    let mut percent = 1.0;
    if let Some(m) = mount {
        for x in children(m, "mods", "mod").into_iter().filter(|x| equipped(x)) {
            let replace = x.get("ammoreplace");
            if !replace.is_empty() {
                ammos = split_ammo(&replace);
            }
            let p = x.get_f64("ammobonuspercent").unwrap_or(0.0);
            if p != 0.0 {
                percent *= p / 100.0;
            }
            flat += x.get_f64("ammobonus").unwrap_or(0.0);
        }
    }
    let modifiers: Vec<String> = accessories.iter().map(|a| a.get("modifyammocapacity")).filter(|m| !m.is_empty()).collect();
    let mut parts = Vec::new();
    for ammo in &ammos {
        let Some(paren) = ammo.find('(') else {
            parts.push(ammo.clone());
            continue;
        };
        let mut this = ammo[..paren].to_owned();
        let mut prepend = String::new();
        if let Some(p) = this.find(['x', '×']) {
            let end = p + char_len(&this, p);
            prepend = this[..end].to_owned();
            this = this[end..].to_owned();
        }
        if !modifiers.is_empty() {
            let mut s = format!("({this}");
            for m in &modifiers {
                s.push_str(m);
                s.push(')');
                let extra = m.matches(')').count() as i64 - m.matches('(').count() as i64 + 1;
                if extra > 0 {
                    s.insert_str(0, &"(".repeat(extra as usize));
                } else if extra < 0 {
                    s.push_str(&")".repeat((-extra) as usize));
                }
            }
            s.push(')');
            this = s;
        }
        let value = match expr::parse_plain(this.trim()) {
            Some(v) => Some(v),
            None => expr::evaluate_num(&this.replace("Weapon", &ammo_capacity(&base))).ok(),
        };
        let mut text = match value {
            Some(v) => {
                let mut a = v + flat;
                if bonus != 0.0 {
                    a += a * bonus / 100.0;
                }
                if percent != 1.0 {
                    a *= percent;
                }
                format!("{}{}", standard_round(a), &ammo[paren..])
            }
            None => this,
        };
        if !prepend.is_empty() {
            text = format!("{prepend}{text}");
        }
        parts.push(text);
    }
    let space = ctx.space(english);
    let s = parts.join(space);
    if english || ctx.is_english() {
        return s;
    }
    let l = ctx.lang;
    let mut s = replace_ci(&s, " or ", &format!("{space}{}{space}", l.s("String_Or")));
    for (from, key) in [(" Belt", "String_AmmoBelt"), (" Energy", "String_AmmoEnergy"), (" External Source", "String_AmmoExternalSource"), (" Special", "String_AmmoSpecial")] {
        s = replace_ci(&s, from, &l.s(key));
    }
    for (from, key) in [
        ("(b)", "String_AmmoBreakAction"),
        ("(belt)", "String_AmmoBelt"),
        ("(box)", "String_AmmoBox"),
        ("(c)", "String_AmmoClip"),
        ("(cy)", "String_AmmoCylinder"),
        ("(d)", "String_AmmoDrum"),
        ("(m)", "String_AmmoMagazine"),
        ("(ml)", "String_AmmoMuzzleLoad"),
    ] {
        s = s.replace(from, &format!("({})", l.s(key)));
    }
    s
}

/// Case-insensitive replace (`StringComparison.OrdinalIgnoreCase`), for
/// ASCII patterns.
fn replace_ci(s: &str, from: &str, to: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find(&needle) {
        out.push_str(&s[i..i + p]);
        out.push_str(to);
        i += p + needle.len();
    }
    out.push_str(&s[i..]);
    out
}

/// `Weapon.WeaponType`: saved, data record, category type, category.
fn weapon_type(ctx: &Ctx, w: &Element) -> String {
    if let Some(t) = w.child_text("weapontype").filter(|t| !t.is_empty()) {
        return t;
    }
    if let Some(t) = record_field(ctx, "weapons.xml", "weapons", "weapon", w, "weapontype").filter(|t| !t.is_empty()) {
        return t;
    }
    let cat = w.get("category");
    ctx.store
        .doc("weapons.xml")
        .ok()
        .and_then(|d| d.child("categories").and_then(|c| c.children_named("category").find(|c| c.text() == cat).and_then(|c| c.attr("type").map(str::to_owned))))
        .unwrap_or_else(|| cat.to_lowercase())
}

/// A gear's `ammoforweapontype` / `isflechetteammo`, saved or from data.
fn gear_field(ctx: &Ctx, g: &Element, field: &str) -> String {
    g.child_text(field).filter(|s| !s.is_empty()).or_else(|| record_field(ctx, "gear.xml", "gears", "gear", g, field)).unwrap_or_default()
}

/// `Weapon.GetAmmoReloadable` over `gears` (recursing into equipped
/// children): equipped, unloaded ammunition that fits the weapon.
fn reloadable<'a>(ctx: &Ctx, w: &Element, gears: &[&'a Element], skill: &str, loaded: &[String], out: &mut Vec<&'a Element>) {
    let cat = w.get("ammocategory");
    let wtype = weapon_type(ctx, w);
    let flechette = w.get("damage").contains("(f)");
    for g in gears.iter().filter(|g| equipped(g)) {
        let fits = qty(g) > 0.0 && !loaded.contains(&g.get("guid").to_ascii_lowercase()) && {
            let extra = g.get("extra");
            let for_type = gear_field(ctx, g, "ammoforweapontype").split(',').map(str::trim).any(|t| t == wtype);
            let is_flechette = crate::xml::parse_bool(&gear_field(ctx, g, "isflechetteammo"));
            if cat == "Gear" {
                g.get("name") == w.get("name") && (extra.is_empty() || extra == cat)
            } else if skill == "Throwing Weapons" {
                (!flechette || is_flechette) && for_type && (extra.is_empty() || extra == cat || g.get("name") == w.get("name"))
            } else {
                (!flechette || is_flechette) && for_type && (extra.is_empty() || extra == cat)
            }
        };
        if fits {
            out.push(g);
        }
        reloadable(ctx, w, &children(g, "children", "gear"), skill, loaded, out);
    }
}

/// `Clip.DisplayAmmoName`.
fn clip_name(ctx: &Ctx, gear: Option<&Element>, count: i32, english: bool) -> String {
    match gear {
        Some(g) if english => g.get("name"),
        Some(g) => ctx.tr_name("gear.xml", g),
        None => ctx.strings(english).s(if count > 0 { "String_MountInternal" } else { "String_None" }),
    }
}

/// `Clip.Print` (empty clips are not printed).
fn clip(ctx: &Ctx, c: &Element) -> Option<Element> {
    let id = c.get("id");
    let gear = (!id.is_empty() && !id.starts_with("00000000")).then(|| find_tagged(&ctx.ch.doc, "gear", &id)).flatten();
    let count = c.get_i32("count").unwrap_or(0);
    if gear.is_none() && count == 0 {
        return None;
    }
    let mut out = Element::new("clip");
    add(&mut out, "name", clip_name(ctx, gear, count, false));
    add(&mut out, "english_name", clip_name(ctx, gear, count, true));
    add(&mut out, "count", count.to_string());
    copy(&mut out, c, "location");
    match gear {
        Some(g) => {
            add(&mut out, "id", g.get("guid"));
            let mut t = Element::new("ammotype");
            weapon_bonus(ctx, &mut t, g, true);
            if g.child("children").is_some_and(|c| c.elements().next().is_some()) {
                let mut kids = Element::new("children");
                let mut stack: Vec<&Element> = children(g, "children", "gear").into_iter().filter(|k| equipped(k)).collect();
                while let Some(k) = stack.pop() {
                    if k.child("weaponbonus").is_some() || k.child("flechetteweaponbonus").is_some() {
                        let mut kt = Element::new("ammotype");
                        weapon_bonus(ctx, &mut kt, k, true);
                        kids.push(kt);
                    }
                    stack.extend(children(k, "children", "gear").into_iter().filter(|x| equipped(x)));
                }
                t.push(kids);
            }
            let b = g.child("weaponbonus");
            let get = |f: &str| b.map(|b| b.get(f)).unwrap_or_default();
            add(&mut t, "DV", bonus_damage(ctx, &get("damage"), &get("damagetype"), &get("damagereplace"), false));
            add(&mut t, "BonusRange", get("rangebonus"));
            out.push(t);
        }
        None => add(&mut out, "id", "00000000-0000-0000-0000-000000000000"),
    }
    Some(out)
}

/// `Weapon.DisplayRange` / `DisplayAlternateRange`: the range category
/// translated from ranges.xml or the weapon categories, with the loaded
/// ammunition when asked.
fn display_range(ctx: &Ctx, key: &str, english: bool, ammo: Option<&Element>) -> String {
    let mut s = if english || ctx.is_english() || key.trim().is_empty() {
        key.to_owned()
    } else {
        let tr = ctx.lang.data_name("ranges.xml", "", key);
        if tr != key { tr } else { ctx.tr_category("weapons.xml", key) }
    };
    if let Some(g) = ammo {
        s.push_str(&format!(" ({})", clip_name(ctx, Some(g), 0, english)));
    }
    s
}

fn range_block(name: String, english_name: String, element: &str, bands: [&str; 4]) -> Element {
    let mut out = Element::new(element);
    add(&mut out, "name", name);
    add(&mut out, "name_english", english_name);
    for (k, v) in ["short", "medium", "long", "extreme"].into_iter().zip(bands) {
        add(&mut out, k, v);
    }
    out
}

/// The four `x`, `x_noammo`, `x_english`, `x_english_noammo` variants
/// (`damage` writes `x_noammo_english`).
fn four(out: &mut Element, field: &str, v: [String; 4], damage_order: bool) {
    let [loc, loc_noammo, en, en_noammo] = v;
    add(out, field, loc);
    add(out, &format!("{field}_noammo"), loc_noammo);
    add(out, &format!("{field}_english"), en);
    add(out, &format!("{field}{}", if damage_order { "_noammo_english" } else { "_english_noammo" }), en_noammo);
}

/// `Weapon.TotalAvailTuple`: own, underbarrels and equipped accessories.
fn weapon_avail(ctx: &Ctx, w: &Element) -> Availability {
    let guid = w.get("guid");
    let mut kids = Vec::new();
    for u in children(w, "underbarrel", "weapon") {
        if !u.get("parentid").eq_ignore_ascii_case(&guid) {
            kids.push(weapon_avail(ctx, u));
        }
    }
    for a in children(w, "accessories", "accessory").into_iter().filter(|a| !included(a) && equipped(a)) {
        kids.push(accessory_avail(ctx, a));
    }
    total_avail(ctx, w, own_avail(w), &kids)
}

/// `WeaponAccessory.TotalAvailTuple`: own plus its gear.
fn accessory_avail(ctx: &Ctx, a: &Element) -> Availability {
    let kids: Vec<Availability> = children(a, "gears", "gear").into_iter().map(|g| gear_avail(ctx, g)).collect();
    total_avail(ctx, a, own_avail(a), &kids)
}

/// `Weapon.OwnWeight`: 0 for cyberware, gear and included weapons.
fn weapon_own_weight(ctx: &Ctx, w: &Element) -> f64 {
    if w.get_bool("cyberware").unwrap_or(false) || w.get("category") == "Gear" || (included(w) && !w.get("parentid").is_empty()) {
        return 0.0;
    }
    own_weight_of(ctx, w, "weapons.xml", "weapons", "weapon")
}

fn accessory_own_weight(ctx: &Ctx, a: &Element) -> f64 {
    if included(a) { 0.0 } else { own_weight_of(ctx, a, "weapons.xml", "accessories", "accessory") }
}

fn accessory_total_weight(ctx: &Ctx, a: &Element) -> f64 {
    accessory_own_weight(ctx, a) + children(a, "gears", "gear").into_iter().filter(|g| equipped(g)).map(|g| gear_total_weight(ctx, g, Some(a))).sum::<f64>()
}

/// `Weapon.TotalWeight`.
pub fn weapon_total_weight(ctx: &Ctx, w: &Element) -> f64 {
    weapon_own_weight(ctx, w)
        + children(w, "accessories", "accessory").into_iter().filter(|a| equipped(a)).map(|a| accessory_total_weight(ctx, a)).sum::<f64>()
        + children(w, "underbarrel", "weapon").into_iter().filter(|u| equipped(u)).map(|u| weapon_total_weight(ctx, u)).sum::<f64>()
}

/// The gear that created a weapon (`ParentID` naming a gear anywhere).
fn parent_gear<'a>(ctx: &'a Ctx, w: &Element) -> Option<&'a Element> {
    let pid = w.get("parentid");
    if pid.trim().is_empty() || pid.starts_with("00000000") {
        return None;
    }
    find_tagged(&ctx.ch.doc, "gear", &pid)
}

/// Where a weapon sits: on the character, or in a vehicle (whose gear
/// supplies ammunition) and possibly a weapon mount.
#[derive(Clone, Copy, Default)]
pub struct WeaponPlace<'a> {
    pub vehicle: Option<&'a Element>,
    pub mount: Option<&'a Element>,
}

/// Combat values of a weapon, with and without its loaded ammunition
/// (`blnIncludeAmmo`): ammunition only enters through `<clips>`.
pub fn weapon_stats(ctx: &Ctx, item: &Element) -> (weapon_calc::WeaponStats, weapon_calc::WeaponStats) {
    let stats = weapon_calc::stats_with(ctx.ch, &ctx.sheet, Some(&ctx.store), item, &ctx.weapon_rules);
    let noammo = if item.child("clips").is_some_and(|c| c.elements().next().is_some()) {
        weapon_calc::stats_with(ctx.ch, &ctx.sheet, Some(&ctx.store), &without(item, &["clips"]), &ctx.weapon_rules)
    } else {
        stats.clone()
    };
    (stats, noammo)
}

/// `Weapon.Print`.
pub fn weapon(ctx: &Ctx, item: &Element, place: WeaponPlace) -> Element {
    let file = "weapons.xml";
    let mut out = Element::new("weapon");
    let (stats, stats_noammo) = weapon_stats(ctx, item);
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, &item.get("weaponname"));
    category(ctx, &mut out, item, file);
    copy(&mut out, item, "type");
    add(&mut out, "reach", stats.reach.to_string());
    add(&mut out, "rawreach", item.get("reach"));
    let raw_acc = item.get("accuracy");
    add(&mut out, "accuracy", accuracy_text(ctx, &raw_acc, stats.accuracy, false));
    add(&mut out, "accuracy_noammo", accuracy_text(ctx, &raw_acc, stats_noammo.accuracy, false));
    add(&mut out, "accuracy_english", accuracy_text(ctx, &raw_acc, stats.accuracy, true));
    add(&mut out, "accuracy_english_noammo", accuracy_text(ctx, &raw_acc, stats_noammo.accuracy, true));
    add(&mut out, "rawaccuracy", raw_acc);
    let dmg = |s: &str| localize_damage(ctx, s, false);
    four(&mut out, "damage", [dmg(&stats.damage), dmg(&stats_noammo.damage), stats.damage.clone(), stats_noammo.damage.clone()], true);
    add(&mut out, "rawdamage", item.get("damage"));
    let ap = |s: &str| localize_ap(ctx, s, false);
    four(&mut out, "ap", [ap(&stats.ap), ap(&stats_noammo.ap), stats.ap.clone(), stats_noammo.ap.clone()], false);
    add(&mut out, "rawap", item.get("ap"));
    let mode = |ammo: bool, en: bool| calculated_mode(ctx, item, ammo, en);
    four(&mut out, "mode", [mode(true, false), mode(false, false), mode(true, true), mode(false, true)], false);
    four(&mut out, "rc", [stats.rc.clone(), stats_noammo.rc.clone(), stats.rc.clone(), stats_noammo.rc.clone()], false);
    add(&mut out, "rawrc", item.get("rc"));
    add(&mut out, "ammo", calculated_ammo(ctx, item, place.mount, false));
    add(&mut out, "ammo_english", calculated_ammo(ctx, item, place.mount, true));
    add(&mut out, "maxammo", item.get("ammo"));
    let rating = item.get_i32("rating").unwrap_or(0);
    let accessories = children(item, "accessories", "accessory");
    let conceal = eval(&item.get("conceal"), rating)
        + accessories.iter().filter(|a| equipped(a)).map(|a| eval(&a.get("conceal"), a.get_i32("rating").unwrap_or(0))).sum::<f64>()
        + ctx.ch.improvements.val("Concealability", None);
    add(&mut out, "conceal", signed(conceal));
    add(&mut out, "rawconceal", item.get("conceal"));
    add(&mut out, "availablemounts", item.get("mount"));
    add(&mut out, "availablemounts_english", item.get("mount"));
    if let Some(g) = parent_gear(ctx, item) {
        // Weapons made by gear print the gear's availability, cost and weight.
        ctx.add_avail(&mut out, gear_avail(ctx, g), true);
        let own_c = 1.0 / gear_calc::cost_for_units(g);
        let w = with_weight(ctx, g);
        cost_weight(ctx, &mut out, gear_calc::cost(g), own_c, gear_calc::total_weight(&w, None), gear_calc::own_weight(&w, None));
    } else {
        ctx.add_avail(&mut out, weapon_avail(ctx, item), true);
        let parent = item.get("parentid");
        let parent_weapon = (!parent.is_empty() && included(item)).then(|| find_tagged(&ctx.ch.doc, "weapon", &parent)).flatten();
        let own = weapon_calc::own_cost(item, parent_weapon);
        let total = weapon_calc::cost(item) - weapon_calc::own_cost(item, None) + own;
        cost_weight(ctx, &mut out, total, own, weapon_total_weight(ctx, item), weapon_own_weight(ctx, item));
    }
    source_page(&mut out, item);
    copy(&mut out, item, "weaponname");
    add(&mut out, "location", ctx.location(&item.get("location")));
    matrix(&mut out, item);
    if !accessories.is_empty() {
        let mut list = Element::new("accessories");
        for x in &accessories {
            list.push(accessory(ctx, x, item));
        }
        out.push(list);
    }
    let ammo = loaded_ammo(ctx, item);
    let (r, rn) = (&stats.ranges, &stats_noammo.ranges);
    let ammo_changes = r != rn;
    let range_key = [item.get("range"), item.get("category")].into_iter().find(|s| !s.trim().is_empty()).unwrap_or_default();
    let alt_key = item.get("alternaterange");
    let alt_name = || (display_range(ctx, &alt_key, false, None), display_range(ctx, &alt_key, true, None));
    let shown_ammo = ammo.filter(|_| ammo_changes);
    out.push(range_block(display_range(ctx, &range_key, false, shown_ammo), display_range(ctx, &range_key, true, None), "ranges", [&r.short, &r.medium, &r.long, &r.extreme]));
    let (an, ane) = alt_name();
    out.push(range_block(an, ane, "alternateranges", [&r.alt_short, &r.alt_medium, &r.alt_long, &r.alt_extreme]));
    if ammo_changes {
        out.push(range_block(display_range(ctx, &range_key, false, None), display_range(ctx, &range_key, true, ammo), "ranges", [&rn.short, &rn.medium, &rn.long, &rn.extreme]));
        let (an, ane) = alt_name();
        out.push(range_block(an, ane, "alternateranges", [&rn.alt_short, &rn.alt_medium, &rn.alt_long, &rn.alt_extreme]));
    }
    for u in children(item, "underbarrel", "weapon") {
        let mut wrap = Element::new("underbarrel");
        wrap.push(weapon(ctx, u, place));
        out.push(wrap);
    }
    // `GetAvailableAmmo`: ammunition the character (or vehicle) carries.
    let mut ammo_gear = Vec::new();
    if item.get_bool("requireammo").unwrap_or(true) {
        let pool: Vec<&Element> = match place.vehicle {
            Some(v) => children(v, "gears", "gear"),
            None => ctx.ch.items("gears", "gear"),
        };
        let mut clips = Vec::new();
        ctx.ch.doc.descendants("clip", &mut clips);
        let loaded: Vec<String> = clips.iter().map(|c| c.get("id").to_ascii_lowercase()).collect();
        reloadable(ctx, item, &pool, &stats.skill, &loaded, &mut ammo_gear);
    }
    add(&mut out, "availableammo", num(ammo_gear.iter().map(|g| qty(g)).sum()));
    let slot = item.get_i32("activeammoslot").unwrap_or(1).max(1) as usize;
    let active = item.child("clips").and_then(|c| c.children_named("clip").nth(slot - 1));
    let active_count = active.and_then(|c| c.get_i32("count")).unwrap_or(0);
    add(&mut out, "currentammo", clip_name(ctx, ammo, active_count, false));
    add(&mut out, "currentammo_english", clip_name(ctx, ammo, active_count, true));
    let mut clips = Element::new("clips");
    if item.get_bool("requireammo").unwrap_or(true) {
        for c in children(item, "clips", "clip") {
            if let Some(e) = clip(ctx, c) {
                clips.push(e);
            }
        }
    } else if let Some(e) = active.and_then(|c| clip(ctx, c)) {
        clips.push(e);
    }
    out.push(clips);
    add(&mut out, "dicepool", stats.dice_pool.to_string());
    add(&mut out, "dicepool_noammo", stats_noammo.dice_pool.to_string());
    // `GetSkill` is null when the character lacks the skill (exotic skills).
    let has_skill = ctx.sheet.skills.iter().any(|s| s.name == stats.skill);
    add(&mut out, "skill", if has_skill { stats.skill.clone() } else { String::new() });
    add(&mut out, "wirelesson", bool_text(item.get_bool("wirelesson").unwrap_or(true)));
    ctx.notes(&mut out, item);
    out
}

/// `WeaponAccessory.Print`.
fn accessory(ctx: &Ctx, item: &Element, weapon: &Element) -> Element {
    let file = "weapons.xml";
    let mut out = Element::new("accessory");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, "");
    for f in ["mount", "extramount", "addmount"] {
        copy(&mut out, item, f);
    }
    let rating = item.get_i32("rating").unwrap_or(0);
    for f in ["damage", "rc", "ap", "conceal"] {
        add(&mut out, f, signed(eval(&item.get(f), rating)));
    }
    ctx.add_avail(&mut out, accessory_avail(ctx, item), false);
    copy(&mut out, item, "ratinglabel");
    let gears = children(item, "gears", "gear");
    let own = weapon_calc::accessory_cost(item, weapon);
    let gear_cost: f64 = gears.iter().map(|g| gear_calc::cost_in(g, item)).sum();
    cost_weight(ctx, &mut out, own + gear_cost, own, accessory_total_weight(ctx, item), accessory_own_weight(ctx, item));
    copy_bool(&mut out, item, "included");
    source_page(&mut out, item);
    add(&mut out, "accuracy", signed(eval(&item.get("accuracy"), rating)));
    if !gears.is_empty() {
        out.push(gear_list(ctx, &gears, GearParent::Other(item)));
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
    for m in children(item, "armormods", "armormod").into_iter().filter(|m| equipped(m)) {
        v += expr::value_to_int(&m.get("armor"), m.get_i32("rating").unwrap_or(0), &expr::NoAttributes);
    }
    if stacking { format!("+{v}") } else { v.to_string() }
}

/// `ArmorMod.TotalAvailTuple`: own plus its gear.
fn armor_mod_avail(ctx: &Ctx, m: &Element) -> Availability {
    let kids: Vec<Availability> = children(m, "gears", "gear").into_iter().map(|g| gear_avail(ctx, g)).collect();
    total_avail(ctx, m, own_avail(m), &kids)
}

/// `Armor.TotalAvailTuple`: own plus mods and gear.
fn armor_avail(ctx: &Ctx, a: &Element) -> Availability {
    let mut kids: Vec<Availability> = children(a, "armormods", "armormod").into_iter().filter(|m| !included(m)).map(|m| armor_mod_avail(ctx, m)).collect();
    kids.extend(children(a, "gears", "gear").into_iter().map(|g| gear_avail(ctx, g)));
    total_avail(ctx, a, own_avail(a), &kids)
}

fn armor_mod_own_weight(ctx: &Ctx, m: &Element) -> f64 {
    if included(m) { 0.0 } else { own_weight_of(ctx, m, "armor.xml", "mods", "mod") }
}

fn armor_mod_total_weight(ctx: &Ctx, m: &Element) -> f64 {
    armor_mod_own_weight(ctx, m) + children(m, "gears", "gear").into_iter().filter(|g| equipped(g)).map(|g| gear_total_weight(ctx, g, Some(m))).sum::<f64>()
}

/// `Armor.TotalWeight`.
pub fn armor_total_weight(ctx: &Ctx, a: &Element) -> f64 {
    own_weight_of(ctx, a, "armor.xml", "armors", "armor")
        + children(a, "armormods", "armormod").into_iter().filter(|m| equipped(m)).map(|m| armor_mod_total_weight(ctx, m)).sum::<f64>()
        + children(a, "gears", "gear").into_iter().filter(|g| equipped(g)).map(|g| gear_total_weight(ctx, g, Some(a))).sum::<f64>()
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
    let used: f64 = mods.iter().filter(|m| !included(m)).map(|m| eval(&m.get("armorcapacity"), m.get_i32("rating").unwrap_or(0)).abs()).sum();
    add(&mut out, "capacityremaining", num(eval(&capacity, 0) - used));
    ctx.add_avail(&mut out, armor_avail(ctx, item), false);
    let own_w = own_weight_of(ctx, item, file, "armors", "armor");
    cost_weight(ctx, &mut out, armor_calc::cost(item), armor_calc::own_cost(item), armor_total_weight(ctx, item), own_w);
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
    out.push(gear_list(ctx, &children(item, "gears", "gear"), GearParent::CostParent(item)));
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
    ctx.add_avail(&mut out, armor_mod_avail(ctx, item), false);
    cost_weight(ctx, &mut out, armor_calc::cost(item), armor_calc::own_cost(item), armor_mod_total_weight(ctx, item), armor_mod_own_weight(ctx, item));
    source_page(&mut out, item);
    copy_bool(&mut out, item, "included");
    copy_bool(&mut out, item, "equipped");
    copy_bool(&mut out, item, "wirelesson");
    out.push(gear_list(ctx, &children(item, "gears", "gear"), GearParent::Other(item)));
    copy(&mut out, item, "extra");
    ctx.notes(&mut out, item);
    out
}

// ---------------------------------------------------------------------------
// Cyberware
// ---------------------------------------------------------------------------

/// `Cyberware.TotalAvailTuple`: own (with grade) plus non-included
/// children and gear whose availability is a modifier.
fn ware_avail(ctx: &Ctx, item: &Element) -> Availability {
    let mut kids: Vec<Availability> = children(item, "children", "cyberware").into_iter().filter(|k| !included(k)).map(|k| ware_avail(ctx, k)).collect();
    kids.extend(children(item, "gears", "gear").into_iter().map(|g| gear_avail(ctx, g)));
    total_avail(ctx, item, ware_calc::availability(ctx.ch, &ctx.store, item), &kids)
}

fn ware_file(item: &Element) -> (&'static str, &'static str, &'static str) {
    if item.get("improvementsource") == "Bioware" { ("bioware.xml", "biowares", "bioware") } else { ("cyberware.xml", "cyberwares", "cyberware") }
}

/// `Cyberware.TotalWeight` (own, children and gear; no grade modifier).
pub fn ware_total_weight(ctx: &Ctx, item: &Element) -> f64 {
    let (file, container, tag) = ware_file(item);
    own_weight_of(ctx, item, file, container, tag)
        + children(item, "children", "cyberware").into_iter().map(|k| ware_total_weight(ctx, k)).sum::<f64>()
        + children(item, "gears", "gear").into_iter().filter(|g| equipped(g)).map(|g| gear_total_weight(ctx, g, Some(item))).sum::<f64>()
}

/// `Cyberware.Print`.
pub fn cyberware(ctx: &Ctx, item: &Element) -> Element {
    let (file, container, tag) = ware_file(item);
    let bio = file == "bioware.xml";
    let mut out = Element::new("cyberware");
    head(ctx, &mut out, item, file);
    full_names(ctx, &mut out, item, file, None, "");
    category(ctx, &mut out, item, file);
    let ess = ware_calc::essence(ctx.ch, &ctx.store, &ctx.rules, item);
    add(&mut out, "ess", ctx.essence(ess));
    copy(&mut out, item, "capacity");
    ctx.add_avail(&mut out, ware_avail(ctx, item), false);
    let total = ware_calc::cost(ctx.ch, &ctx.store, item);
    // `OwnCost`: the item alone, with its grade.
    let own_cost = ware_calc::cost(ctx.ch, &ctx.store, &without(item, &["children", "gears"]));
    cost_weight(ctx, &mut out, total, own_cost, ware_total_weight(ctx, item), own_weight_of(ctx, item, file, container, tag));
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
    add(&mut out, "improvementsource", if bio { "Bioware" } else { "Cyberware" });
    copy_bool(&mut out, item, "isgeneware");
    matrix(&mut out, item);
    let gears = children(item, "gears", "gear");
    if !gears.is_empty() {
        out.push(gear_list(ctx, &gears, GearParent::Other(item)));
    }
    let mut list = Element::new("children");
    for k in children(item, "children", "cyberware") {
        list.push(cyberware(ctx, k));
    }
    out.push(list);
    ctx.notes(&mut out, item);
    out
}

// ---------------------------------------------------------------------------
// Drugs
// ---------------------------------------------------------------------------

/// `Drug.Print`.
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
    ctx.add_avail(&mut out, own_avail(item), true);
    add(&mut out, "cost", ctx.nuyen(crate::items::drug::cost(item)));
    ctx.notes(&mut out, item);
    out
}
