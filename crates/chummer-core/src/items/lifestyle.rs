//! Lifestyles (`Lifestyle.Create` / `Lifestyle.Save`), lifestyle qualities
//! (`LifestyleQuality.Create` / `Save`) and the monthly cost
//! (`Lifestyle.GetTotalMonthlyCost`).

use crate::bonus::{self, BonusSource, Choice};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr;
use crate::improvement::{bool_str, fmt_num, Improvement};
use crate::xml::Element;

use super::{new_guid, Purchase};

const FILE: &str = "lifestyles.xml";

/// Days per month Chummer uses to convert lifestyle increments.
const WEEKS_PER_MONTH: f64 = 4.34812;

/// Fields the oracle does not compare for lifestyles.
pub const IGNORE: &[&str] = &[
    // chosen in the lifestyle dialog one quality at a time
    "lifestylequalities",
    // 5.202 kept free grid subscriptions in their own list, and saved a
    // `primarytenant` flag current Chummer no longer has
    "freegrids", "primarytenant",
    // 5.202 saved 0 here; current Chummer stores the table minimum of the
    // base lifestyle (`comforts`/`neighborhoods`/`securities`)
    "basearea", "basecomforts", "basesecurity",
];

/// What the lifestyle dialog (`SelectLifestyle.AcceptForm`) sets on top of
/// the data record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    /// The player's name for this lifestyle; the record name when empty.
    pub name: String,
    /// Months (or weeks, days) paid for.
    pub months: i32,
    pub roommates: i32,
    pub percentage: f64,
    /// Points bought above the base (advanced lifestyles only).
    pub area: i32,
    pub comforts: i32,
    pub security: i32,
    pub bonus_lp: i32,
    pub trust_fund: bool,
    pub split_cost_with_roommates: bool,
    /// `LifestyleType`: `"Standard"`, `"Advanced"`, `"BoltHole"` or `"Safehouse"`.
    pub style: String,
    pub city: String,
    pub district: String,
    pub borough: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            name: String::new(),
            months: 1,
            roommates: 0,
            percentage: 100.0,
            area: 0,
            comforts: 0,
            security: 0,
            bonus_lp: 0,
            trust_fund: false,
            split_cost_with_roommates: false,
            style: "Standard".into(),
            city: String::new(),
            district: String::new(),
            borough: String::new(),
        }
    }
}

impl Options {
    /// The kind-generic purchase: `answer` names the lifestyle, `qty` is
    /// the number of months.
    pub fn from_purchase(p: &Purchase) -> Options {
        Options {
            name: p.answer.clone().unwrap_or_default(),
            months: if p.qty > 0.0 { p.qty as i32 } else { 1 },
            ..Options::default()
        }
    }

    /// The choices stored in a saved `<lifestyle>`.
    pub fn from_saved(e: &Element) -> Options {
        let int = |k: &str| e.get_i32(k).unwrap_or(0);
        let flag = |k: &str| e.get_bool(k).unwrap_or(false);
        Options {
            name: e.get("name"),
            months: e.get_i32("months").unwrap_or(1),
            roommates: int("roommates"),
            percentage: e.get_f64("percentage").unwrap_or(100.0),
            area: int("area"),
            comforts: int("comforts"),
            security: int("security"),
            bonus_lp: int("bonuslp"),
            trust_fund: flag("trustfund"),
            split_cost_with_roommates: split_cost_with_roommates(e),
            style: e.child_text("type").filter(|t| !t.is_empty()).unwrap_or_else(|| "Standard".into()),
            city: e.get("city"),
            district: e.get("district"),
            borough: e.get("borough"),
        }
    }
}

/// `Lifestyle.Load`: `splitcostwithroommates`, else the opposite of the
/// legacy `primarytenant`, else whether there are roommates.
fn split_cost_with_roommates(e: &Element) -> bool {
    e.get_bool("splitcostwithroommates")
        .or_else(|| e.get_bool("primarytenant").map(|p| !p))
        .unwrap_or_else(|| e.get_i32("roommates").unwrap_or(0) > 0)
}

/// `LifestyleQuality.Load`: `uselpcost`, else the legacy
/// `contributetolimit`, else true.
fn use_lp_cost(q: &Element) -> bool {
    q.get_bool("uselpcost").or_else(|| q.get_bool("contributetolimit")).unwrap_or(true)
}

/// `LifestyleIncrement` from its data/save name.
fn increment(s: &str) -> &'static str {
    match s.to_ascii_uppercase().as_str() {
        "DAY" => "Day",
        "WEEK" => "Week",
        _ => "Month",
    }
}

/// `minimum` and `limit` of the base lifestyle in a comforts, neighborhoods
/// or securities table.
fn base_and_limit(doc: &Element, table: &str, item: &str, base: &str) -> (i32, i32) {
    doc.child(table)
        .and_then(|t| t.children_named(item).find(|e| e.get("name") == base))
        .map(|e| (e.get_i32("minimum").unwrap_or(0), e.get_i32("limit").unwrap_or(0)))
        .unwrap_or((0, 0))
}

/// Free grid subscriptions need Hard Targets or the free grids option.
fn free_grids_enabled(ch: &Character, store: &DataStore) -> bool {
    let Ok(lib) = crate::settings::SettingsLibrary::load(store, crate::settings::user_settings_dir().as_deref()) else { return false };
    lib.resolve(&ch.doc.get("settings")).is_some_and(|s| s.flag("allowfreegrids") || s.books().iter().any(|b| b == "HT"))
}

/// `Lifestyle.Create` (via `SetBaseLifestyle`) + the dialog's choices +
/// `Lifestyle.Save`, without qualities.
fn build(doc: &Element, rec: Record<'_>, o: &Options, guid: &str) -> Element {
    let d = rec.el();
    let base = rec.name();
    let dec = |k: &str| d.get_f64(k).unwrap_or(0.0);
    let (base_comforts, max_comforts) = base_and_limit(doc, "comforts", "comfort", &base);
    let (base_area, max_area) = base_and_limit(doc, "neighborhoods", "neighborhood", &base);
    let (base_security, max_security) = base_and_limit(doc, "securities", "security", &base);
    let advanced = o.style != "Standard";
    let pick = |v: i32| if advanced { v } else { 0 };
    let mut l = Element::new("lifestyle");
    let mut put = |k: &str, v: String| l.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", if o.name.is_empty() { base.clone() } else { o.name.clone() });
    put("cost", fmt_num(dec("cost")));
    put("dice", d.get_i32("dice").unwrap_or(0).to_string());
    put("lp", d.get_i32("lp").unwrap_or(0).to_string());
    put("baselifestyle", base.clone());
    put("multiplier", fmt_num(dec("multiplier")));
    put("months", o.months.to_string());
    put("roommates", if o.trust_fund { 0 } else { o.roommates }.to_string());
    put("percentage", fmt_num(o.percentage));
    put("area", pick(o.area).to_string());
    put("comforts", pick(o.comforts).to_string());
    put("security", pick(o.security).to_string());
    put("basearea", base_area.to_string());
    put("basecomforts", base_comforts.to_string());
    put("basesecurity", base_security.to_string());
    put("maxarea", max_area.to_string());
    put("maxcomforts", max_comforts.to_string());
    put("maxsecurity", max_security.to_string());
    put("costforearea", fmt_num(dec("costforarea")));
    put("costforcomforts", fmt_num(dec("costforcomforts")));
    put("costforsecurity", fmt_num(dec("costforsecurity")));
    put("allowbonuslp", bool_str(d.get_bool("allowbonuslp").unwrap_or(false)));
    put("bonuslp", pick(o.bonus_lp).to_string());
    put("source", rec.source());
    put("page", rec.page());
    put("trustfund", bool_str(o.trust_fund));
    put("splitcostwithroommates", bool_str(o.split_cost_with_roommates));
    put("type", o.style.clone());
    put("increment", increment(&d.get("increment")).into());
    put("sourceid", rec.id());
    put("city", o.city.clone());
    put("district", o.district.clone());
    put("borough", o.borough.clone());
    l.push(Element::new("lifestylequalities"));
    let mut put = |k: &str, v: String| l.push(Element::with_text(k, v));
    put("notes", d.child_text("altnotes").unwrap_or_else(|| d.get("notes")));
    put("notesColor", "Chocolate".into());
    put("sortorder", "0".into());
    l
}

/// Build the saved `<lifestyle>` for a record and the dialog's choices.
pub fn element_with(ch: &Character, store: &DataStore, rec: Record<'_>, o: &Options, guid: &str) -> Result<Element, String> {
    let doc = store.doc(FILE).map_err(|e| e.to_string())?;
    let mut l = build(&doc, rec, o, guid);
    if free_grids_enabled(ch, store) {
        for q in free_grid_qualities(&doc, rec) {
            l.child_or_insert("lifestylequalities").push(q);
        }
    }
    Ok(l)
}

/// Build the saved `<lifestyle>` for a generic purchase (see
/// [`Options::from_purchase`]).
pub fn element(ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase, guid: &str) -> Result<Element, String> {
    element_with(ch, store, rec, &Options::from_purchase(p), guid)
}

/// The built-in free grid subscriptions of a lifestyle record (`<freegrids>`).
fn free_grid_qualities(doc: &Element, rec: Record<'_>) -> Vec<Element> {
    let Some(grids) = rec.el().child("freegrids") else { return Vec::new() };
    grids
        .children_named("freegrid")
        .filter_map(|g| {
            let q = crate::data::find(doc, "qualities", "quality", &g.text())?;
            let mut e = quality_element(q, &new_guid(), "BuiltIn", g.attr("select").unwrap_or(""));
            e.set_child_text("isfreegrid", "True");
            Some(e)
        })
        .collect()
}

/// The extra a lifestyle quality keeps: the text in parentheses, if any
/// (`LifestyleQuality.Create`).
fn quality_extra(extra: &str) -> String {
    match extra.find('(') {
        Some(i) => extra[i + 1..].strip_suffix(')').unwrap_or(&extra[i + 1..]).to_owned(),
        None => extra.to_owned(),
    }
}

/// `LifestyleQuality.Create` + `LifestyleQuality.Save`. `source` is the
/// `QualitySource` ("Selected", "BuiltIn"...).
pub fn quality_element(rec: Record<'_>, guid: &str, source: &str, extra: &str) -> Element {
    let d = rec.el();
    let int = |k: &str| d.get_i32(k).unwrap_or(0).to_string();
    let category = rec.category();
    let kind = match category.to_ascii_uppercase().as_str() {
        "NEGATIVE" => "Negative",
        "POSITIVE" => "Positive",
        "CONTRACTS" => "Contracts",
        _ => "Entertainment",
    };
    let mut q = Element::new("lifestylequality");
    let mut put = |k: &str, v: String| q.push(Element::with_text(k, v));
    put("sourceid", rec.id());
    put("guid", guid.to_owned());
    put("name", rec.name());
    put("category", category.clone());
    put("extra", quality_extra(extra));
    put("cost", d.get("cost"));
    put("multiplier", fmt_num(d.get_f64("multiplier").unwrap_or(0.0)));
    put("basemultiplier", fmt_num(d.get_f64("multiplierbaseonly").unwrap_or(0.0)));
    put("lp", int("lp"));
    put("areamaximum", int("areamaximum"));
    put("comfortsmaximum", int("comfortsmaximum"));
    put("securitymaximum", int("securitymaximum"));
    put("area", int("area"));
    put("comforts", int("comforts"));
    put("security", int("security"));
    put("uselpcost", bool_str(true));
    put("print", bool_str(d.get_bool("print").unwrap_or(true)));
    put("lifestylequalitytype", kind.into());
    put("lifestylequalitysource", source.to_owned());
    put("free", bool_str(source == "BuiltIn"));
    put("isfreegrid", bool_str(false));
    put("source", rec.source());
    put("page", rec.page());
    put("allowed", d.get("allowed").split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(","));
    let mut bonus = Element::new("bonus");
    if let Some(b) = d.child("bonus") {
        bonus.children = b.children.clone();
    }
    q.push(bonus);
    let mut put = |k: &str, v: String| q.push(Element::with_text(k, v));
    put("notes", d.child_text("altnotes").unwrap_or_else(|| d.get("notes")));
    put("notesColor", "Chocolate".into());
    q
}

/// Selections needed before adding a lifestyle: its name.
pub fn choices(_ch: &Character, _store: &DataStore, rec: Record<'_>, _p: &Purchase) -> Vec<Choice> {
    vec![Choice { node: "lifestylename".into(), prompt: format!("Name of the {} lifestyle", rec.name()), options: Vec::new() }]
}

/// Selections a lifestyle quality's bonus needs.
pub fn quality_choices(ch: &Character, store: &DataStore, rec: Record<'_>) -> Vec<Choice> {
    let src = BonusSource { kind: "Quality".into(), guid: String::new(), name: rec.name(), rating: 1 };
    rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default()
}

/// Add a lifestyle with the dialog's choices. Returns its guid.
pub fn add_with(ch: &mut Character, store: &DataStore, rec: Record<'_>, o: &Options) -> Result<String, String> {
    let guid = new_guid();
    let mut l = element_with(ch, store, rec, o, &guid)?;
    let mut improvements = Vec::new();
    if let Some(list) = l.child_mut("lifestylequalities") {
        for q in list.elements_mut() {
            let extra = q.get("extra");
            improvements.extend(apply_quality_bonus(ch, store, q, Some(extra.as_str())));
        }
    }
    ch.items_mut("lifestyles").push(l);
    ch.improvements.list.extend(improvements);
    Ok(guid)
}

/// Add a lifestyle from a generic purchase (see [`Options::from_purchase`]).
pub fn add(ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    add_with(ch, store, rec, &Options::from_purchase(p))
}

/// Run a lifestyle quality's data bonus (source "Quality", as Chummer does).
fn apply_quality_bonus(ch: &Character, store: &DataStore, q: &mut Element, answer: Option<&str>) -> Vec<Improvement> {
    let Ok(doc) = store.doc(FILE) else { return Vec::new() };
    let key = q.get("sourceid");
    let Some(rec) = crate::data::find(&doc, "qualities", "quality", &key) else { return Vec::new() };
    let Some(b) = rec.el().child("bonus") else { return Vec::new() };
    let src = BonusSource { kind: "Quality".into(), guid: q.get("guid"), name: q.get("name"), rating: 1 };
    let out = bonus::apply(ch, store, b, &src, answer);
    if let Some(sel) = out.selected.filter(|s| !s.is_empty()) {
        q.set_child_text("extra", sel);
    }
    out.improvements
}

/// Add a lifestyle quality to the lifestyle with guid `lifestyle`.
/// `free` makes it cost nothing (the dialog's "free" box).
pub fn add_quality(ch: &mut Character, store: &DataStore, lifestyle: &str, rec: Record<'_>, extra: Option<&str>, free: bool) -> Result<String, String> {
    let guid = new_guid();
    let mut q = quality_element(rec, &guid, "Selected", extra.unwrap_or(""));
    if free {
        q.set_child_text("free", "True");
    }
    let improvements = apply_quality_bonus(ch, store, &mut q, extra);
    let parent = super::find_by_guid_mut(&mut ch.doc, lifestyle).ok_or_else(|| format!("no lifestyle with guid {lifestyle}"))?;
    parent.child_or_insert("lifestylequalities").push(q);
    ch.improvements.list.extend(improvements);
    ch.dirty = true;
    Ok(guid)
}

/// Oracle: rebuild a saved `<lifestyle>` from its base lifestyle record and
/// the choices saved in it.
pub fn rebuild(_ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let doc = store.doc(FILE).ok()?;
    let id = saved.get("sourceid");
    let rec = crate::data::find(&doc, "lifestyles", "lifestyle", &id)
        .filter(|_| !id.is_empty())
        .or_else(|| crate::data::find(&doc, "lifestyles", "lifestyle", &saved.get("baselifestyle")))?;
    Some(build(&doc, rec, &Options::from_saved(saved), &saved.get("guid")))
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// A saved lifestyle quality, as the cost calculation sees it.
struct Lq {
    builtin: bool,
    kind: String,
    category: String,
    cost: f64,
    multiplier: f64,
    base_multiplier: f64,
}

/// `LifestyleQuality.CanBeFreeByLifestyle`.
fn free_by_lifestyle(q: &Element, base: &str) -> bool {
    let kind = q.get("lifestylequalitytype");
    if kind != "Entertainment" && kind != "Contracts" || base.is_empty() {
        return false;
    }
    let allowed = q.get("allowed");
    let allowed: Vec<&str> = allowed.split(',').filter(|s| !s.is_empty()).collect();
    allowed.contains(&base) || allowed.contains(&equivalent_lifestyle(base))
}

/// `Lifestyle.GetEquivalentLifestyle`.
fn equivalent_lifestyle(s: &str) -> &str {
    match s {
        "BOLTHOLE" | "BOLT HOLE" => "Squatter",
        "TRAVELER" => "Low",
        "COMMERCIAL" => "Medium",
        _ if s.to_ascii_lowercase().starts_with("hospitalized") => "High",
        _ => s,
    }
}

impl Lq {
    fn of(q: &Element, base: &str, attrs: &dyn expr::AttributeSource) -> Lq {
        let builtin = q.get("lifestylequalitysource") == "BuiltIn";
        // `CostFree`: free, built in, or paid with LP where the base allows.
        let free = q.get_bool("free").unwrap_or(false) || builtin || (use_lp_cost(q) && free_by_lifestyle(q, base));
        let num = |k: &str| q.get_f64(k).unwrap_or(0.0);
        let cost = q.get("cost");
        let cost = if expr::needs_evaluation(&cost) {
            expr::evaluate_num(&expr::substitute_attributes(&cost, attrs)).unwrap_or(0.0)
        } else {
            expr::parse_plain(&cost).unwrap_or(0.0)
        };
        Lq {
            builtin,
            kind: q.get("lifestylequalitytype"),
            category: q.get("category"),
            cost: if free { 0.0 } else { cost },
            multiplier: if free { 0.0 } else { num("multiplier") },
            base_multiplier: if free { 0.0 } else { num("basemultiplier") },
        }
    }
}

/// The lifestyle qualities of a saved lifestyle (5.202 kept free grids in a
/// separate list).
fn qualities(e: &Element, attrs: &dyn expr::AttributeSource) -> Vec<Lq> {
    let base = e.get("baselifestyle");
    ["lifestylequalities", "freegrids"]
        .iter()
        .filter_map(|l| e.child(l))
        .flat_map(|l| l.children_named("lifestylequality"))
        .map(|q| Lq::of(q, &base, attrs))
        .collect()
}

/// Multiply up `1 + m/100` over a list of percentages.
fn product(ms: impl Iterator<Item = f64>) -> f64 {
    ms.filter(|m| *m != 0.0).fold(1.0, |acc, m| acc * (1.0 + m / 100.0))
}

/// `Lifestyle.CostPreSplit`.
fn cost_pre_split(e: &Element, qs: &[Lq]) -> f64 {
    let num = |k: &str| e.get_f64(k).unwrap_or(0.0);
    let mut cost = num("cost");
    let bought: Vec<&Lq> = qs.iter().filter(|q| !q.builtin).collect();
    cost *= product(bought.iter().map(|q| q.base_multiplier));
    let (area, comforts, security) = (num("area"), num("comforts"), num("security"));
    if area + comforts + security != 0.0 {
        cost *= 1.0 + 0.1 * (area + comforts + security);
    }
    cost += area * num("costforearea") + comforts * num("costforcomforts") + security * num("costforsecurity");
    let assets: Vec<&&Lq> = bought.iter().filter(|q| q.kind == "Entertainment" && q.category.contains("Asset")).collect();
    cost *= product(assets.iter().map(|q| q.multiplier));
    cost += assets.iter().map(|q| q.cost).sum::<f64>();
    let others: Vec<&&Lq> = bought.iter().filter(|q| q.kind != "Entertainment" && q.kind != "Contracts").collect();
    cost *= product(others.iter().map(|q| q.multiplier));
    cost += others.iter().map(|q| q.cost).sum::<f64>();
    let roommates = num("roommates");
    if roommates > 0.0 {
        cost *= 1.0 + 0.1 * roommates;
    }
    cost.max(0.0)
}

/// Lifestyle cost improvements that apply to a base lifestyle
/// (`LifestyleCost`, and `BasicLifestyleCost` for standard lifestyles).
/// One-off ("once") adjustments are not modelled.
fn cost_improvements<'a>(ch: &'a Character, base: &'a str, standard: bool) -> impl Iterator<Item = &'a Improvement> + 'a {
    ch.improvements.list.iter().filter(move |i| {
        i.enabled
            && (i.kind == "LifestyleCost" || standard && i.kind == "BasicLifestyleCost")
            && i.condition.is_empty()
            && (i.improved_name.is_empty() || i.improved_name == base)
    })
}

/// `Lifestyle.GetTotalMonthlyCost`: nuyen per increment (month, week or
/// day) of a saved lifestyle.
pub fn monthly_cost(ch: &Character, e: &Element) -> f64 {
    let qs = qualities(e, &expr::NoAttributes);
    let mut total = 0.0;
    if !e.get_bool("trustfund").unwrap_or(false) {
        total += cost_pre_split(e, &qs);
        if split_cost_with_roommates(e) {
            total /= e.get_f64("roommates").unwrap_or(0.0) + 1.0;
        }
    }
    let base = e.get("baselifestyle");
    let standard = e.child_text("type").is_none_or(|t| t.is_empty() || t == "Standard");
    let dependents: Vec<String> = ch.items("qualities", "quality").iter().filter(|q| q.get("name").contains("Dependent")).map(|q| q.get("guid")).collect();
    let (mut dep, mut meta, mut other) = (0.0, 0.0, 1.0);
    for i in cost_improvements(ch, &base, standard) {
        if i.source == "Quality" && dependents.iter().any(|g| g.eq_ignore_ascii_case(&i.source_name)) {
            dep += i.val;
        } else if matches!(i.source.as_str(), "Heritage" | "Metatype" | "Metavariant") {
            meta += i.val;
        } else {
            other *= 1.0 + i.val / 100.0;
        }
    }
    if dep != 0.0 {
        total *= 1.0 + dep / 100.0;
    }
    if meta != 0.0 {
        total *= 1.0 + meta / 100.0;
    }
    total *= other;

    let bought: Vec<&Lq> = qs.iter().filter(|q| !q.builtin).collect();
    let contracts: f64 = bought.iter().filter(|q| q.kind == "Contracts").map(|q| q.cost).sum();
    let outings: Vec<&&Lq> = bought.iter().filter(|q| q.kind == "Entertainment" && !q.category.contains("Asset")).collect();
    total *= product(outings.iter().map(|q| q.multiplier));
    total += outings.iter().map(|q| q.cost).sum::<f64>();
    let base_multiplier = product(outings.iter().map(|q| q.base_multiplier));
    if base_multiplier != 1.0 {
        total += e.get_f64("cost").unwrap_or(0.0) * base_multiplier;
    }
    total *= e.get_f64("percentage").unwrap_or(100.0) / 100.0;
    total
        + match increment(&e.get("increment")) {
            "Day" => contracts / (WEEKS_PER_MONTH * 7.0),
            "Week" => contracts / WEEKS_PER_MONTH,
            _ => contracts,
        }
}

/// `Lifestyle.TotalCost`: the monthly cost times the months paid.
pub fn total_cost(ch: &Character, e: &Element) -> f64 {
    monthly_cost(ch, e) * f64::from(e.get_i32("months").unwrap_or(1))
}

// ---------------------------------------------------------------------------
// Editing a saved lifestyle
// ---------------------------------------------------------------------------

/// Bought (non-built-in) qualities of a saved lifestyle.
fn bought_qualities(e: &Element) -> impl Iterator<Item = &Element> {
    e.child("lifestylequalities").into_iter().flat_map(|l| l.children_named("lifestylequality")).filter(|q| q.get("lifestylequalitysource") != "BuiltIn")
}

/// How many area, comforts and security points can be bought on top of
/// the base (`Lifestyle.AreaDelta`, `ComfortsDelta`, `SecurityDelta`): the
/// total maximum (base lifestyle limit plus quality maximums) less the
/// base and what qualities already give.
pub fn point_limits(e: &Element) -> (i32, i32, i32) {
    let int = |x: &Element, k: &str| x.get_i32(k).unwrap_or(0);
    let delta = |field: &str, max: &str, qmax: &str, base: &str| {
        let total_max = int(e, max) + bought_qualities(e).map(|q| int(q, qmax)).sum::<i32>();
        let given = int(e, base) + bought_qualities(e).map(|q| int(q, field)).sum::<i32>();
        (total_max - given).max(0)
    };
    (
        delta("area", "maxarea", "areamaximum", "basearea"),
        delta("comforts", "maxcomforts", "comfortsmaximum", "basecomforts"),
        delta("security", "maxsecurity", "securitymaximum", "basesecurity"),
    )
}

/// Write the player's choices back into a saved lifestyle (what the
/// lifestyle dialog changes when editing): name, months, roommates,
/// percentage, bought points (advanced lifestyles only, within
/// [`point_limits`]), trust fund and split cost. Returns false when no
/// lifestyle has that guid.
pub fn update(ch: &mut Character, guid: &str, o: &Options) -> bool {
    let Some(l) = super::find_by_guid_mut(ch.items_mut("lifestyles"), guid) else { return false };
    let advanced = o.style != "Standard";
    let (max_area, max_comforts, max_security) = point_limits(l);
    let pick = |v: i32, max: i32| if advanced { v.clamp(0, max) } else { 0 };
    let base = l.get("baselifestyle");
    l.set_child_text("name", if o.name.trim().is_empty() { base } else { o.name.clone() });
    l.set_child_text("months", o.months.max(1).to_string());
    l.set_child_text("roommates", if o.trust_fund { 0 } else { o.roommates.max(0) }.to_string());
    l.set_child_text("percentage", fmt_num(o.percentage.max(0.0)));
    l.set_child_text("area", pick(o.area, max_area).to_string());
    l.set_child_text("comforts", pick(o.comforts, max_comforts).to_string());
    l.set_child_text("security", pick(o.security, max_security).to_string());
    l.set_child_text("bonuslp", if advanced { o.bonus_lp.max(0) } else { 0 }.to_string());
    l.set_child_text("trustfund", bool_str(o.trust_fund));
    l.set_child_text("splitcostwithroommates", bool_str(o.split_cost_with_roommates));
    l.set_child_text("type", o.style.clone());
    ch.dirty = true;
    true
}

/// Remove a lifestyle quality and the improvements it created.
pub fn remove_quality(ch: &mut Character, lifestyle: &str, quality: &str) -> bool {
    let Some(l) = super::find_by_guid_mut(ch.items_mut("lifestyles"), lifestyle) else { return false };
    let Some(list) = l.child_mut("lifestylequalities") else { return false };
    let before = list.children.len();
    list.children.retain(|n| !matches!(n, crate::xml::Node::Element(e) if e.get("guid").eq_ignore_ascii_case(quality)));
    if list.children.len() == before {
        return false;
    }
    ch.improvements.remove_from_source(quality);
    ch.dirty = true;
    true
}
