//! Cyberware and bioware (`Cyberware.Create`, `Cyberware.CreateChildren`,
//! `Cyberware.Save`, plus the cost rules of `Cyberware.CalculatedTotalCost`
//! and the grade/rating rules of `SelectCyberware`).
//!
//! Both kinds are saved as `<cyberware>` in `<cyberwares>`; the data file
//! they came from is recorded in `<improvementsource>` (`Cyberware` or
//! `Bioware`). Expressions (`ess`, `capacity`, `avail`, `cost`) are saved
//! as the raw data strings and evaluated on use.
//!
//! Not modelled yet (documented, not silently wrong):
//! - `<addweapon>`, `<addvehicle>`, `<addparentweaponaccessory>` and data
//!   `<gears><usegear>`: these need the weapon, vehicle and gear modules.
//!   They are reported in [`Outcome::unsupported`].
//! - Automatic side choice from requirements and mount blockers
//!   (`GetValidLimbSlot`): the side comes from the purchase answer.
//! - `<wirelesspairbonus>` (4 records): the pair count across wireless
//!   ware is not applied.

use crate::bonus::{self, BonusSource, Choice, Outcome};
use crate::calc::{self, AttributeValues, Rules, SheetAttributes};
use crate::character::Character;
use crate::data::{DataStore, Record};
use crate::expr::{self, AttributeSource};
use crate::improvement::{bool_str, fmt_num, Improvement};
use crate::xml::Element;

use super::{new_guid, Purchase};

/// `Cyberware.EssenceHoleGuidString`.
pub const ESSENCE_HOLE_ID: &str = "b57eadaa-7c3b-4b80-8d79-cbbd922c1196";
/// `Cyberware.EssenceAntiHoleGuidString`.
pub const ESSENCE_ANTIHOLE_ID: &str = "961eac53-0c43-4b19-8741-2872177a3a4c";

/// Fields the oracle does not compare for cyberware.
pub const IGNORE: &[&str] = &[
    // Career-mode snapshot of the non-retroactive essence multipliers at
    // purchase time (`SaveNonRetroactiveEssenceModifiers`); old saves also
    // write "1.0" where current code writes "1".
    "extraessadditivemultiplier", "extraessmultiplicativemultiplier",
    // User toggle, and only meaningful with the Prototype Transhuman quality.
    "prototypetranshuman",
    // Links to the weapon/vehicle instance the ware created (per-instance GUIDs).
    "weaponguid", "vehicleguid",
    // User state: stolen flag, overclocked matrix attribute.
    "stolen", "overclocked",
];

// ---------------------------------------------------------------------------
// Data lookup
// ---------------------------------------------------------------------------

/// `(file, container, item, ImprovementSource)` for cyberware or bioware.
pub fn data_source(bioware: bool) -> (&'static str, &'static str, &'static str, &'static str) {
    if bioware {
        ("bioware.xml", "biowares", "bioware", "Bioware")
    } else {
        ("cyberware.xml", "cyberwares", "cyberware", "Cyberware")
    }
}

/// Whether a saved `<cyberware>` came from bioware.xml.
pub fn is_bioware(saved: &Element) -> bool {
    saved.get("improvementsource") == "Bioware"
}

/// A grade record (`<grades><grade>`) by name.
pub fn grade_record<'a>(doc: &'a Element, name: &str) -> Option<&'a Element> {
    doc.child("grades")?.children_named("grade").find(|g| g.get("name") == name)
}

/// `Character.GetGradeByName`: the named grade, else "Standard".
fn resolve_grade(store: &DataStore, bioware: bool, name: &str) -> String {
    let Ok(doc) = store.doc(data_source(bioware).0) else { return "Standard".into() };
    if grade_record(&doc, name).is_some() {
        name.to_owned()
    } else {
        "Standard".into()
    }
}

/// A grade's cost multiplier (`Grade.Cost`), 1 when unknown.
fn grade_cost(store: &DataStore, bioware: bool, name: &str) -> f64 {
    store
        .doc(data_source(bioware).0)
        .ok()
        .and_then(|d| grade_record(&d, name).and_then(|g| g.get_f64("cost")))
        .unwrap_or(1.0)
}

/// Grades offered for a record (`SelectCyberware.PopulateGrades` with the
/// record's `forcegrade` / `bannedgrades`). Adapsin and Burnout's Way grades
/// are shown only when the character has the matching improvement, and
/// then replace their plain counterparts; "None" only when forced.
pub fn grades(ch: &Character, store: &DataStore, bioware: bool, rec: Record<'_>) -> Vec<String> {
    let Ok(doc) = store.doc(data_source(bioware).0) else { return Vec::new() };
    let names: Vec<String> = doc.child("grades").map(|g| g.children_named("grade").map(|e| e.get("name")).collect()).unwrap_or_default();
    let force = rec.get("forcegrade");
    if !force.is_empty() {
        return names.into_iter().filter(|n| *n == force).collect();
    }
    let banned: Vec<String> = rec.el().child("bannedgrades").map(|b| b.children_named("grade").map(Element::text).collect()).unwrap_or_default();
    let disabled_kind = if bioware { "DisableBiowareGrade" } else { "DisableCyberwareGrade" };
    let disabled: Vec<String> = ch.improvements.of_kind(disabled_kind).map(|i| i.improved_name.clone()).collect();
    let adapsin = !bioware && ch.improvements.has("Adapsin");
    let burnout = ch.improvements.has("BurnoutsWay");
    let is_adapsin = |n: &str| n.contains("(Adapsin)");
    let is_burnout = |n: &str| n.contains("Burnout");
    // A plain grade is hidden when a variant of it is active ("Alphaware" vs
    // "Alphaware (Adapsin)").
    let has_variant = |n: &str, f: &dyn Fn(&str) -> bool| names.iter().any(|m| f(m) && m != n && m.contains(n));
    names
        .iter()
        .filter(|n| n.as_str() != "None" && !banned.contains(n))
        .filter(|n| !disabled.iter().any(|d| n.contains(d.as_str())))
        .filter(|n| if adapsin { is_adapsin(n) || !has_variant(n, &is_adapsin) } else { !is_adapsin(n) })
        .filter(|n| if burnout { is_burnout(n) || !has_variant(n, &is_burnout) } else { !is_burnout(n) })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// The saved element
// ---------------------------------------------------------------------------

/// Per-instance values of a piece of ware: what the buyer chose, or what
/// the containing object set.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub grade: String,
    pub rating: i32,
    pub extra: String,
    /// `Left` / `Right` for sided ware.
    pub location: String,
    /// `<forced>`: a forced bonus answer or side, from a data subsystem.
    pub forced: String,
    pub parent_id: String,
    /// Replaces the data cost: the chosen `Variable(..)` amount, or "0" for
    /// ware included in something else.
    pub cost: Option<String>,
    pub suite: bool,
    pub stolen: bool,
    pub ess_discount: i32,
    pub prototype_transhuman: bool,
    pub ess_additive: f64,
    pub ess_multiplicative: f64,
}

impl Default for Install {
    fn default() -> Self {
        Install {
            grade: "Standard".into(),
            rating: 0,
            extra: String::new(),
            location: String::new(),
            forced: String::new(),
            parent_id: String::new(),
            cost: None,
            suite: false,
            stolen: false,
            ess_discount: 0,
            prototype_transhuman: false,
            ess_additive: 0.0,
            ess_multiplicative: 1.0,
        }
    }
}

/// A data flag as Chummer reads it: present and not "False".
fn data_flag(e: &Element, k: &str) -> bool {
    e.child(k).is_some_and(|c| c.text() != "False")
}

/// The data `cost`, with `Variable(min-max)` resolved to its minimum
/// (`Cyberware.Create`, skip-forms path).
fn data_cost(e: &Element) -> String {
    let c = e.child_text("cost").unwrap_or_else(|| "0".into());
    match c.strip_prefix("Variable(") {
        Some(rest) => {
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            inner.split('-').next().unwrap_or("0").to_owned()
        }
        None => c,
    }
}

/// `<allowsubsystems><category>` joined with commas.
fn allowed_subsystems(e: &Element) -> String {
    e.child("allowsubsystems").map(|a| a.children_named("category").map(Element::text).collect::<Vec<_>>().join(",")).unwrap_or_default()
}

/// `<pairinclude>` / `<wirelesspairinclude>` names (`_lstIncludeInPairBonus`).
fn pair_names(e: &Element, node: &str) -> Vec<String> {
    let name = e.get("name");
    match e.child(node) {
        None => vec![name],
        Some(p) => {
            let mut v = Vec::new();
            if p.attr("includeself") != Some("False") {
                v.push(name);
            }
            v.extend(p.children_named("name").map(Element::text));
            v
        }
    }
}

fn names_element(tag: &str, names: &[String]) -> Element {
    let mut e = Element::new(tag);
    for n in names {
        e.push(Element::with_text("name", n.clone()));
    }
    e
}

/// A copy of data node `k` (bonus-like), or an empty element.
fn node_or_empty(e: &Element, k: &str) -> Element {
    e.child(k).cloned().unwrap_or_else(|| Element::new(k))
}

/// Matrix attributes: `attributearray` or the four separate fields.
fn matrix_attributes(e: &Element) -> ([String; 4], String, bool) {
    if e.child("attributearray").is_some() {
        let arr = e.get("attributearray");
        let mut p = arr.split(',').map(str::to_owned);
        let mut next = || p.next().unwrap_or_default();
        let m = [next(), next(), next(), next()];
        (m, arr, true)
    } else {
        ([e.get("attack"), e.get("sleaze"), e.get("dataprocessing"), e.get("firewall")], String::new(), false)
    }
}

/// Build a saved `<cyberware>` from its data record without children
/// (`Cyberware.Create` field reads + `Cyberware.Save`). `children` and
/// `gears` are written empty; callers fill them.
pub fn ware_element(rec: Record<'_>, bioware: bool, inst: &Install, guid: &str) -> Element {
    let e = rec.el();
    let mut w = Element::new("cyberware");
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("guid", guid.to_owned());
    // `Guid.ToString("D")`: lower case, whatever the data file uses.
    put("sourceid", rec.id().to_ascii_lowercase());
    put("name", rec.name());
    put("category", rec.category());
    put("limbslot", e.get("limbslot"));
    put("limbslotcount", e.child_text("limbslotcount").unwrap_or_else(|| "1".into()));
    put("inheritattributes", bool_str(e.child("inheritattributes").is_some()));
    put("ess", e.get("ess"));
    put("capacity", e.get("capacity"));
    put("avail", e.get("avail"));
    put("cost", inst.cost.clone().unwrap_or_else(|| data_cost(e)));
    put("weight", e.get("weight"));
    put("source", rec.source());
    put("page", rec.page());
    put("parentid", inst.parent_id.clone());
    put("hasmodularmount", e.get("modularmount"));
    put("plugsintomodularmount", e.get("mountsto"));
    put("blocksmounts", e.get("blocksmounts"));
    put("forced", inst.forced.clone());
    put("rating", inst.rating.to_string());
    put("minagility", e.get_i32("minagility").unwrap_or(3).to_string());
    put("minstrength", e.get_i32("minstrength").unwrap_or(3).to_string());
    put("minrating", e.get("minrating"));
    put("maxrating", e.get("rating"));
    put("ratinglabel", e.child_text("ratinglabel").unwrap_or_else(|| "String_Rating".into()));
    put("subsystems", allowed_subsystems(e));
    put("wirelesson", bool_str(e.child("wirelessbonus").is_some() || e.child("wirelesspairbonus").is_some()));
    put("grade", inst.grade.clone());
    put("location", inst.location.clone());
    put("extra", inst.extra.clone());
    put("suite", bool_str(inst.suite));
    put("stolen", bool_str(inst.stolen));
    put("essdiscount", inst.ess_discount.to_string());
    put("extraessadditivemultiplier", fmt_num(inst.ess_additive));
    put("extraessmultiplicativemultiplier", fmt_num(inst.ess_multiplicative));
    put("forcegrade", e.get("forcegrade"));
    put("matrixcmfilled", "0".into());
    put("matrixcmbonus", e.get_i32("matrixcmbonus").unwrap_or(0).to_string());
    put("prototypetranshuman", bool_str(inst.prototype_transhuman));
    for k in ["bonus", "pairbonus", "wirelessbonus", "wirelesspairbonus"] {
        w.push(node_or_empty(e, k));
    }
    if let Some(g) = e.child("allowgear") {
        w.push(g.clone());
    }
    w.push(Element::with_text("improvementsource", data_source(bioware).3));
    w.push(names_element("pairinclude", &pair_names(e, "pairinclude")));
    w.push(names_element("wirelesspairinclude", &pair_names(e, "wirelesspairinclude")));
    // Chummer writes these only when non-empty; older saves always wrote
    // `<children />`. Always writing them is harmless on load.
    w.push(Element::new("children"));
    w.push(Element::new("gears"));
    let notes = e.child_text("altnotes").unwrap_or_else(|| e.get("notes"));
    let ([a, s, d, f], arr, swap) = matrix_attributes(e);
    let mut put = |k: &str, v: String| w.push(Element::with_text(k, v));
    put("notes", notes);
    put("notesColor", e.child_text("notesColor").unwrap_or_else(|| "#003FFF".into()));
    put("discountedcost", "False".into());
    put("addtoparentess", bool_str(data_flag(e, "addtoparentess")));
    put("addtoparentcapacity", bool_str(data_flag(e, "addtoparentcapacity")));
    put("isgeneware", bool_str(data_flag(e, "isgeneware")));
    put("active", "False".into());
    put("homenode", "False".into());
    put("devicerating", e.get("devicerating"));
    put("programlimit", e.get("programs"));
    put("overclocked", "None".into());
    put("canformpersona", e.get("canformpersona"));
    put("attack", a);
    put("sleaze", s);
    put("dataprocessing", d);
    put("firewall", f);
    put("attributearray", arr);
    for k in ["modattack", "modsleaze", "moddataprocessing", "modfirewall", "modattributearray"] {
        put(k, e.get(k));
    }
    put("canswapattributes", bool_str(swap));
    put("sortorder", "0".into());
    w
}

/// Creation-mode essence multipliers written by `Cyberware.Save` from the
/// character's non-retroactive improvements: `(additive, multiplicative)`.
pub fn nonretroactive_multipliers(ch: &Character, bioware: bool) -> (f64, f64) {
    let (cost, total) = if bioware {
        ("BiowareEssCostNonRetroactive", "BiowareTotalEssMultiplierNonRetroactive")
    } else {
        ("CyberwareEssCostNonRetroactive", "CyberwareTotalEssMultiplierNonRetroactive")
    };
    let mut add = 0.0;
    if ch.improvements.has(cost) {
        let m = ch.improvements.of_kind(cost).fold(1.0, |m, i| m - (1.0 - i.val / 100.0));
        add -= 1.0 - m;
    }
    let mult = ch.improvements.of_kind(total).fold(1.0, |m, i| m * i.val / 100.0);
    (add, mult)
}

// ---------------------------------------------------------------------------
// Creation (element + bonuses + data subsystems)
// ---------------------------------------------------------------------------

/// Builds ware and collects what its bonuses produce.
struct Builder<'a> {
    ch: &'a Character,
    store: &'a DataStore,
    /// `blnCreateImprovements`.
    apply: bool,
    out: Outcome,
}

impl Builder<'_> {
    /// `Cyberware.Create` + `CreateChildren` for one record.
    fn create(&mut self, rec: Record<'_>, bioware: bool, mut inst: Install, guid: &str, answer: Option<&str>, has_parent: bool) -> Element {
        let e = rec.el();
        for k in ["addweapon", "addvehicle", "addparentweaponaccessory"] {
            if e.child(k).is_some() {
                self.out.unsupported.push(format!("cyberware {k}"));
            }
        }
        if self.apply {
            self.apply_bonuses(rec, bioware, &mut inst, guid, answer, has_parent);
        }
        let mut w = ware_element(rec, bioware, &inst, guid);
        let kids = self.create_children(e, &inst, guid);
        push_children(&mut w, kids);
        w
    }

    /// The bonus, pair bonus and wireless bonus of a new piece of ware
    /// (`Cyberware.Create`, `RefreshWirelessBonuses`). Modular ware that is
    /// not plugged into anything is created unequipped, without bonuses
    /// (`ChangeModularEquip(false)`).
    fn apply_bonuses(&mut self, rec: Record<'_>, bioware: bool, inst: &mut Install, guid: &str, answer: Option<&str>, has_parent: bool) {
        let e = rec.el();
        if !e.get("mountsto").is_empty() && !has_parent {
            return;
        }
        let kind = data_source(bioware).3;
        let forced = if !inst.forced.is_empty() && inst.forced != "Left" && inst.forced != "Right" {
            Some(inst.forced.clone())
        } else {
            answer.map(str::to_owned)
        };
        let rating = inst.rating;
        let src = |g: String| BonusSource { kind: kind.into(), guid: g, name: rec.name(), rating };
        if let Some(b) = non_empty(e, "bonus") {
            let o = bonus::apply(self.ch, self.store, b, &src(guid.to_owned()), forced.as_deref());
            self.take_selected(inst, &o);
            self.merge(o);
        }
        if let Some(b) = non_empty(e, "pairbonus") {
            if pair_count(self.ch, &pair_names(e, "pairinclude"), &rec.name(), &inst.extra, &inst.location) & 1 == 1 {
                let f = forced.clone().or_else(|| Some(inst.extra.clone()).filter(|x| !x.is_empty()));
                let o = bonus::apply(self.ch, self.store, b, &src(format!("{guid}Pair")), f.as_deref());
                self.merge(o);
            }
        }
        if let Some(b) = non_empty(e, "wirelessbonus") {
            if b.attr("mode") == Some("replace") {
                for i in self.out.improvements.iter_mut().filter(|i| i.source_name == guid) {
                    i.enabled = false;
                }
            }
            let o = bonus::apply(self.ch, self.store, b, &src(format!("{guid}Wireless")), forced.as_deref());
            self.take_selected(inst, &o);
            self.merge(o);
        }
        if e.child("wirelesspairbonus").is_some() {
            self.out.unsupported.push("cyberware wirelesspairbonus".into());
        }
    }

    /// `if (!string.IsNullOrEmpty(strSelectedValue) && string.IsNullOrEmpty(_strExtra)) _strExtra = strSelectedValue`.
    fn take_selected(&self, inst: &mut Install, o: &Outcome) {
        if inst.extra.is_empty() {
            if let Some(s) = o.selected.as_ref().filter(|s| !s.is_empty()) {
                inst.extra = s.clone();
            }
        }
    }

    fn merge(&mut self, o: Outcome) {
        self.out.improvements.extend(o.improvements);
        self.out.flags.extend(o.flags);
        self.out.added.extend(o.added);
        self.out.unsupported.extend(o.unsupported);
    }

    /// `Cyberware.CreateChildren`: the `<subsystems>` of `node` (a data
    /// record or a nested subsystem entry), each at the parent's grade with
    /// cost 0. Data `<gears><usegear>` need the gear module.
    fn create_children(&mut self, node: &Element, parent: &Install, parent_guid: &str) -> Vec<Element> {
        let mut out = Vec::new();
        if let Some(subs) = node.child("subsystems") {
            for (bio, tag) in [(false, "cyberware"), (true, "bioware")] {
                let (file, container, item, _) = data_source(bio);
                let Ok(doc) = self.store.doc(file) else { continue };
                for sub in subs.children_named(tag) {
                    let Some(rec) = crate::data::find(&doc, container, item, &sub.get("name")) else { continue };
                    out.push(self.create_subsystem(rec, bio, sub, parent, parent_guid));
                }
            }
        }
        if node.path("gears/usegear").is_some() {
            self.out.unsupported.push("cyberware usegear".into());
        }
        out
    }

    /// One `<subsystems>` entry: created at the parent's grade with cost 0,
    /// then the entry's own nested subsystems.
    fn create_subsystem(&mut self, rec: Record<'_>, bioware: bool, sub: &Element, parent: &Install, parent_guid: &str) -> Element {
        let side = if rec.el().child("selectside").is_some() { parent.location.clone() } else { String::new() };
        let inst = Install {
            grade: parent.grade.clone(),
            rating: clamp_rating(self.ch, self.store, rec, sub.get_i32("rating").unwrap_or(0), None),
            forced: sub.get("forced"),
            parent_id: parent_guid.to_owned(),
            cost: Some("0".into()),
            location: side,
            ess_additive: parent.ess_additive,
            ess_multiplicative: parent.ess_multiplicative,
            ..Install::default()
        };
        let guid = new_guid();
        let mut w = self.create(rec, bioware, inst.clone(), &guid, None, true);
        let nested = self.create_children(sub, &inst, &guid);
        push_children(&mut w, nested);
        w
    }
}

/// A bonus-like data node, when present and not empty.
fn non_empty<'a>(e: &'a Element, k: &str) -> Option<&'a Element> {
    e.child(k).filter(|b| b.elements().next().is_some())
}

fn push_children(w: &mut Element, kids: Vec<Element>) {
    let c = w.child_or_insert("children");
    for k in kids {
        c.push(k);
    }
}

/// Every installed piece of ware, children included.
fn all_ware(ch: &Character) -> Vec<&Element> {
    fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
        out.push(e);
        if let Some(c) = e.child("children") {
            for k in c.children_named("cyberware") {
                walk(k, out);
            }
        }
    }
    let mut v = Vec::new();
    for w in ch.items("cyberwares", "cyberware") {
        walk(w, &mut v);
    }
    v
}

/// The pair count of `Cyberware.Create`: other installed ware that pairs
/// with this one. Sided ware that pairs only with itself pairs Left with
/// Right; the result is then 0 or 1.
fn pair_count(ch: &Character, include: &[String], name: &str, extra: &str, location: &str) -> i32 {
    let others: Vec<&Element> = all_ware(ch).into_iter().filter(|w| include.contains(&w.get("name")) && w.get("extra") == extra).collect();
    if !location.is_empty() && include.iter().all(|n| n == name) {
        let n: i32 = others.iter().map(|w| if w.get("location") != location { 1 } else { -1 }).sum();
        i32::from(n > 0)
    } else {
        others.len() as i32
    }
}

// ---------------------------------------------------------------------------
// Ratings and availability
// ---------------------------------------------------------------------------

/// Attribute tokens for ware: `{STRMinimum}` / `{AGIMinimum}` come from the
/// cyberlimb the ware is in (`Cyberware.ProcessAttributesInXPath`), the
/// rest from the character.
struct WareAttributes<'a> {
    sheet: SheetAttributes<'a>,
    limb: Option<&'a Element>,
    vehicle: Option<VehicleAttributes>,
}

/// What a vehicle supplies to ware installed in it (not in a cyberlimb):
/// `STRMinimum` = Body, `STRMaximum` = Body × 2, `AGIMinimum` = Pilot,
/// `AGIMaximum` = MaxPilot, each at least 1
/// (`Cyberware.ProcessAttributesInXPath`, `ParentVehicle` branch).
#[derive(Debug, Clone, Copy)]
pub struct VehicleAttributes {
    pub body: i32,
    pub pilot: i32,
    pub max_pilot: i32,
}

impl AttributeSource for WareAttributes<'_> {
    fn attribute_token(&self, token: &str) -> Option<i32> {
        match (token, self.limb, self.vehicle) {
            ("STRMinimum", Some(l), _) => Some(l.get_i32("minstrength").unwrap_or(3)),
            ("AGIMinimum", Some(l), _) => Some(l.get_i32("minagility").unwrap_or(3)),
            ("STRMinimum", None, Some(v)) => Some(v.body.max(1)),
            ("STRMaximum", None, Some(v)) => Some(v.body.saturating_mul(2).max(1)),
            ("AGIMinimum", None, Some(v)) => Some(v.pilot.max(1)),
            ("AGIMaximum", None, Some(v)) => Some(v.max_pilot.max(1)),
            _ => self.sheet.attribute_token(token),
        }
    }
}

fn sheet_attributes(ch: &Character, store: &DataStore) -> Vec<AttributeValues> {
    let rules = Rules::default();
    ch.attributes.iter().map(|a| calc::attribute_values_with(ch, &a.name, &rules, Some(store))).collect()
}

/// Saves before 5.214 wrote `MinimumAGI`-style tokens; `Cyberware.Load`
/// rewrites them to `{AGIMinimum}`.
fn modern_tokens(s: &str) -> String {
    let mut s = s.to_owned();
    for a in ["STR", "AGI", "BOD", "REA"] {
        s = s.replace(&format!("Minimum{a}"), &format!("{{{a}Minimum}}")).replace(&format!("Maximum{a}"), &format!("{{{a}Maximum}}"));
    }
    s
}

/// Evaluate a rating expression (`GetMinRating` / `GetMaxRating`).
fn eval_rating(s: &str, attrs: &dyn AttributeSource) -> i32 {
    if s.trim().is_empty() {
        return 0;
    }
    expr::standard_round(expr::evaluate_num(&expr::substitute_attributes(&modern_tokens(s), attrs)).unwrap_or(0.0))
}

fn is_limb(e: &Element) -> bool {
    e.get("category") == "Cyberlimb" || !e.get("limbslot").is_empty()
}

/// `(MinRating, MaxRating)` of a record. `limb` is the ware it goes into;
/// a cyberlimb supplies `{AGIMinimum}`-style tokens.
pub fn rating_range(ch: &Character, store: &DataStore, rec: Record<'_>, limb: Option<&Element>) -> (i32, i32) {
    let attrs = sheet_attributes(ch, store);
    let src = WareAttributes { sheet: SheetAttributes(&attrs), limb: limb.filter(|l| is_limb(l)), vehicle: None };
    let min = eval_rating(&rec.get("minrating"), &src);
    let max = eval_rating(&rec.get("rating"), &src);
    (min, max.max(min))
}

/// `Math.Min(Math.Max(rating, MinRating), MaxRating)` from `Cyberware.Create`.
fn clamp_rating(ch: &Character, store: &DataStore, rec: Record<'_>, rating: i32, limb: Option<&Element>) -> i32 {
    let (min, max) = rating_range(ch, store, rec, limb);
    rating.max(min).min(max)
}

/// Availability of saved ware including its grade's modifier
/// (`Cyberware.TotalAvailTuple`, own part, without children).
pub fn availability(ch: &Character, store: &DataStore, e: &Element) -> expr::Availability {
    let attrs = sheet_attributes(ch, store);
    let src = SheetAttributes(&attrs);
    let rating = e.get_i32("rating").unwrap_or(0);
    let min = eval_rating(&e.get("minrating"), &src);
    let mut a = expr::Availability::parse(&e.get("avail"), rating, min, &src);
    let grade_avail = store
        .doc(data_source(is_bioware(e)).0)
        .ok()
        .and_then(|d| grade_record(&d, &e.get("grade")).and_then(|g| g.get_i32("avail")))
        .unwrap_or(0);
    if !a.add_to_parent {
        a.value = (a.value + grade_avail).max(0);
    }
    a
}

// ---------------------------------------------------------------------------
// Module contract
// ---------------------------------------------------------------------------

/// The ware `p.parent` names, if any.
fn parent_ware<'a>(ch: &'a Character, p: &Purchase) -> Option<&'a Element> {
    let g = p.parent.as_deref()?;
    all_ware(ch).into_iter().find(|w| w.get("guid").eq_ignore_ascii_case(g))
}

/// Grade for a purchase: forced by the record, chosen, the parent's, or
/// Standard (`SelectCyberware` forces the parent's grade on children).
fn purchase_grade(store: &DataStore, rec: Record<'_>, bioware: bool, p: &Purchase, parent: Option<&Element>) -> String {
    let forced = rec.get("forcegrade");
    if !forced.is_empty() {
        forced
    } else if let Some(g) = p.grade.as_deref() {
        resolve_grade(store, bioware, g)
    } else if let Some(par) = parent {
        par.get("grade")
    } else {
        "Standard".into()
    }
}

/// Per-instance values for a new purchase: grade, clamped rating, side
/// (the parent's, else a "Left"/"Right" answer), cost.
fn purchase_install(ch: &Character, store: &DataStore, rec: Record<'_>, bioware: bool, p: &Purchase) -> Install {
    let parent = parent_ware(ch, p);
    let side = p.answer.as_deref().filter(|a| *a == "Left" || *a == "Right");
    let location = if rec.el().child("selectside").is_some() {
        parent.map(|w| w.get("location")).filter(|l| !l.is_empty()).or(side.map(str::to_owned)).unwrap_or_default()
    } else {
        String::new()
    };
    let (ess_additive, ess_multiplicative) = if ch.created { (0.0, 1.0) } else { nonretroactive_multipliers(ch, bioware) };
    Install {
        grade: purchase_grade(store, rec, bioware, p, parent),
        rating: clamp_rating(ch, store, rec, p.rating, parent),
        location,
        cost: if p.free { Some("0".into()) } else { None },
        ess_additive,
        ess_multiplicative,
        ..Install::default()
    }
}

/// Build new ware from a record and collect its bonus outcome. `tag` is
/// "cyberware" or "bioware". The answer doubles as the side ("Left" /
/// "Right") for ware with `<selectside>`, as `strForced` does in Chummer.
pub fn element(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase, guid: &str) -> (Element, Outcome) {
    let bioware = tag == "bioware";
    let inst = purchase_install(ch, store, rec, bioware, p);
    let answer = p.answer.as_deref().filter(|a| *a != "Left" && *a != "Right");
    let mut b = Builder { ch, store, apply: true, out: Outcome::default() };
    let w = b.create(rec, bioware, inst, guid, answer, p.parent.is_some());
    (w, b.out)
}

/// Selections needed before adding: the selections of the ware's bonus,
/// plus a side for `<selectside>` ware outside a sided parent.
pub fn choices(tag: &str, ch: &Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Vec<Choice> {
    let src = BonusSource { kind: data_source(tag == "bioware").3.into(), guid: String::new(), name: rec.name(), rating: p.rating };
    let mut v: Vec<Choice> = rec.el().child("bonus").map(|b| bonus::choices(ch, store, b, &src)).unwrap_or_default();
    let parent_side = parent_ware(ch, p).is_some_and(|w| !w.get("location").is_empty());
    if rec.el().child("selectside").is_some() && !parent_side {
        v.push(Choice { node: "selectside".into(), prompt: format!("Choose a side for {}", rec.name()), options: vec!["Left".into(), "Right".into()] });
    }
    v
}

/// Add ware to the character, or into `p.parent`. Returns its guid.
/// Creation mode: call [`crate::essence_loss::refresh`] afterwards.
pub fn add(tag: &str, ch: &mut Character, store: &DataStore, rec: Record<'_>, p: &Purchase) -> Result<String, String> {
    let guid = new_guid();
    let (w, out) = element(tag, ch, store, rec, p, &guid);
    match p.parent.as_deref() {
        Some(pg) => {
            let parent = super::find_by_guid_mut(ch.items_mut("cyberwares"), pg).ok_or_else(|| format!("no ware with guid {pg}"))?;
            parent.child_or_insert("children").push(w);
        }
        None => ch.items_mut("cyberwares").push(w),
    }
    crate::items::place_added(ch, store, &out.added);
    super::apply_outcome(ch, &out);
    Ok(guid)
}

/// Remove ware (top-level or nested) and every improvement it and its
/// children made, with the objects those bonuses created: ware granted
/// through `addware` (`FreeWare`), qualities, limit modifiers and mentor
/// spirits. Creation mode: call [`crate::essence_loss::refresh`] afterwards.
pub fn remove(ch: &mut Character, guid: &str) -> bool {
    let Some(w) = all_ware(ch).into_iter().find(|w| w.get("guid").eq_ignore_ascii_case(guid)).cloned() else { return false };
    let mut guids = Vec::new();
    collect_guids(&w, &mut guids);
    if let Some(c) = ch.doc.child_mut("cyberwares") {
        remove_nested(c, guid);
    }
    for g in &guids {
        let owned: Vec<(&str, String)> = ch.improvements.list.iter().filter(|i| source_of(i, g)).filter_map(|i| granted_object(i).map(|c| (c, i.improved_name.clone()))).collect();
        ch.improvements.list.retain(|i| !source_of(i, g));
        for (container, id) in owned {
            if container == "cyberwares" {
                remove(ch, &id);
            } else {
                ch.remove_item(container, &id);
            }
        }
    }
    ch.dirty = true;
    true
}

/// The container of an object a bonus created, keyed by the improvement
/// that records it (as `chargen::remove_with_children` does for qualities).
fn granted_object(i: &Improvement) -> Option<&'static str> {
    match i.kind.as_str() {
        "FreeWare" => Some("cyberwares"),
        "SpecificQuality" => Some("qualities"),
        "LimitModifier" => Some("limitmodifiers"),
        "MentorSpirit" | "Paragon" => Some("mentorspirits"),
        _ => None,
    }
}

/// Improvements from ware `g`: its bonus, pair and wireless sources.
fn source_of(i: &Improvement, g: &str) -> bool {
    let s = i.source_name.to_ascii_lowercase();
    let g = g.to_ascii_lowercase();
    s == g || s == format!("{g}pair") || s == format!("{g}wireless") || s == format!("{g}wirelesspair")
}

fn collect_guids(e: &Element, out: &mut Vec<String>) {
    out.push(e.get("guid"));
    if let Some(c) = e.child("children") {
        for k in c.children_named("cyberware") {
            collect_guids(k, out);
        }
    }
}

fn remove_nested(e: &mut Element, guid: &str) -> bool {
    let before = e.children.len();
    e.children.retain(|n| !matches!(n, crate::xml::Node::Element(x) if x.name == "cyberware" && x.get("guid").eq_ignore_ascii_case(guid)));
    if e.children.len() != before {
        return true;
    }
    e.elements_mut().any(|c| remove_nested(c, guid))
}

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

/// Per-instance values stored in a saved element.
fn saved_install(saved: &Element) -> Install {
    let flag = |k: &str| saved.get_bool(k).unwrap_or(false);
    Install {
        grade: saved.get("grade"),
        rating: saved.get_i32("rating").unwrap_or(0),
        extra: saved.get("extra"),
        location: saved.get("location"),
        forced: saved.get("forced"),
        parent_id: saved.get("parentid"),
        cost: None,
        suite: flag("suite"),
        stolen: flag("stolen"),
        ess_discount: saved.get_i32("essdiscount").unwrap_or(0),
        prototype_transhuman: flag("prototypetranshuman"),
        ess_additive: saved.get_f64("extraessadditivemultiplier").unwrap_or(0.0),
        ess_multiplicative: saved.get_f64("extraessmultiplicativemultiplier").unwrap_or(1.0),
    }
}

/// The saved ware whose `<children>` contains `guid`.
fn parent_of<'a>(ch: &'a Character, guid: &str) -> Option<&'a Element> {
    all_ware(ch)
        .into_iter()
        .find(|w| w.child("children").is_some_and(|c| c.children_named("cyberware").any(|k| k.get("guid").eq_ignore_ascii_case(guid))))
}

/// Whether the parent's data lists `name` as an included subsystem, so
/// Chummer zeroed its cost (`CreateChildren`).
fn is_data_subsystem(store: &DataStore, parent: &Element, name: &str) -> bool {
    let (file, container, item, _) = data_source(is_bioware(parent));
    let Ok(doc) = store.doc(file) else { return false };
    let key = parent.child_text("sourceid").filter(|s| !s.is_empty()).unwrap_or_else(|| parent.get("name"));
    let Some(rec) = crate::data::find(&doc, container, item, &key) else { return false };
    let mut subs = Vec::new();
    rec.el().descendants("subsystems", &mut subs);
    subs.iter().any(|s| s.elements().any(|w| w.get("name") == name))
}

/// The cost a saved element must have when it is not the data cost: the
/// chosen amount for `Variable(..)` costs, "0" for included subsystems and
/// ware granted by a bonus.
fn rebuilt_cost(ch: &Character, store: &DataStore, rec: Record<'_>, saved: &Element) -> Option<String> {
    let guid = saved.get("guid");
    if rec.get("cost").starts_with("Variable(") {
        return Some(saved.get("cost"));
    }
    let granted = ch.improvements.list.iter().any(|i| i.kind == "FreeWare" && i.improved_name.eq_ignore_ascii_case(&guid));
    let included = parent_of(ch, &guid).is_some_and(|p| is_data_subsystem(store, p, &saved.get("name")));
    (granted || included).then(|| "0".into())
}

/// Oracle: rebuild a saved `<cyberware>` from its data record and the
/// choices stored in it. Children are rebuilt the same way; gear children
/// are kept as saved (they belong to the gear module).
pub fn rebuild(ch: &Character, store: &DataStore, saved: &Element) -> Option<Element> {
    let bioware = is_bioware(saved);
    let (file, container, item, _) = data_source(bioware);
    let doc = store.doc(file).ok()?;
    let key = saved.child_text("sourceid").filter(|s| !s.is_empty()).unwrap_or_else(|| saved.get("name"));
    let rec = crate::data::find(&doc, container, item, &key)?;
    let mut inst = saved_install(saved);
    inst.cost = rebuilt_cost(ch, store, rec, saved);
    let mut w = ware_element(rec, bioware, &inst, &saved.get("guid"));
    if let Some(sc) = saved.child("children") {
        let kids = sc.children_named("cyberware").map(|k| rebuild(ch, store, k).unwrap_or_else(|| k.clone())).collect();
        push_children(&mut w, kids);
    }
    if let Some(sg) = saved.child("gears") {
        *w.child_or_insert("gears") = sg.clone();
    }
    Some(w)
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// Token values for `Cyberware.ProcessCostExpression`.
#[derive(Clone, Copy, Default)]
struct CostTokens {
    children: f64,
    gear: f64,
    parent: f64,
}

/// `Cyberware.ProcessCostExpression` for a saved element: FixedValues,
/// cost tokens, `MinRating`, attributes, `Rating`, then evaluate.
fn cost_expression(e: &Element, s: &str, t: CostTokens, attrs: &dyn AttributeSource) -> f64 {
    let rating = saved_rating(e, attrs);
    let s = expr::fixed_values(s, rating);
    let s = s.trim_start_matches('+');
    if s.is_empty() {
        return 0.0;
    }
    if !expr::needs_evaluation(s) {
        return expr::parse_plain(s).unwrap_or(0.0);
    }
    let min = eval_rating(&e.get("minrating"), attrs).to_string();
    let s = modern_tokens(s);
    let s = s.as_str();
    let r = rating.to_string();
    let mut s = s.to_owned();
    for (token, v) in [("Parent Cost", t.parent), ("Gear Cost", t.gear), ("Children Cost", t.children)] {
        s = s.replace(&format!("{{{token}}}"), &fmt_num(v)).replace(token, &fmt_num(v));
    }
    let s = s.replace("{MinRating}", &min).replace("MinRating", &min);
    let s = expr::substitute_attributes(&s, attrs).replace("{Rating}", &r).replace("Rating", &r);
    expr::evaluate_num(&s).unwrap_or(0.0)
}

/// `Cyberware.GetRating`: the saved rating kept within the ware's
/// `MinRating`..`MaxRating` (a cyberlimb customization saved above the
/// character's attribute maximum is priced at that maximum).
fn saved_rating(e: &Element, attrs: &dyn AttributeSource) -> i32 {
    let rating = e.get_i32("rating").unwrap_or(0);
    let max = eval_rating(&e.get("maxrating"), attrs);
    let min = eval_rating(&e.get("minrating"), attrs);
    rating.min(max).max(min)
}

fn gear_children(e: &Element) -> impl Iterator<Item = &Element> {
    e.child("gears").into_iter().flat_map(|g| g.children_named("gear"))
}

/// The `Gear Cost` token: `CalculatedCost` of the ware's gear.
fn gear_token_cost(e: &Element) -> f64 {
    gear_children(e).map(super::gear::own_calculated_cost).sum()
}

/// `GearChildren.Sum(x => x.TotalCost)`.
fn gear_children_cost(e: &Element) -> f64 {
    gear_children(e).map(|g| super::gear::cost_in(g, e)).sum()
}

fn children(e: &Element) -> Vec<&Element> {
    e.child("children").map(|c| c.children_named("cyberware").collect()).unwrap_or_default()
}

struct CostCtx<'a> {
    ch: &'a Character,
    store: &'a DataStore,
    sheet: SheetAttributes<'a>,
    vehicle: Option<VehicleAttributes>,
}

impl CostCtx<'_> {
    /// `CalculatedOwnCostPreMultipliers`.
    /// `limb` is the cyberlimb the ware sits in: it supplies the
    /// `{AGIMinimum}`-style tokens (`Cyberware.ProcessAttributesInXPath`).
    fn own_pre(&self, e: &Element, grade: &str, parent_cost: f64, limb: Option<&Element>) -> f64 {
        let cost = e.get("cost");
        let kids = if cost.contains("Children Cost") { children(e).iter().map(|k| self.total(k, grade, 0.0, Some(e))).sum() } else { 0.0 };
        let attrs = WareAttributes { sheet: SheetAttributes(self.sheet.0), limb: limb.filter(|l| is_limb(l)), vehicle: self.vehicle };
        cost_expression(e, &cost, CostTokens { children: kids, gear: gear_token_cost(e), parent: parent_cost }, &attrs)
    }

    /// `CalculatedTotalCostWithoutModifiers` at `grade` (children are priced
    /// at their parent's grade).
    fn without_modifiers(&self, e: &Element, grade: &str, parent_cost: f64, limb: Option<&Element>) -> f64 {
        let base = self.own_pre(e, grade, parent_cost, limb);
        let mut total = base * grade_cost(self.store, is_bioware(e), grade);
        if e.get_bool("discountedcost").unwrap_or(false) {
            total *= 0.9;
        }
        if e.get_bool("isgeneware").unwrap_or(false) {
            total *= self.ch.improvements.of_kind("GenetechCostMultiplier").fold(1.0, |m, i| m - (1.0 - i.val / 100.0));
        }
        for k in children(e) {
            if k.get("capacity") == "[*]" {
                continue;
            }
            match k.get("cost").strip_prefix('*') {
                Some(factor) => {
                    let f = cost_expression(k, factor, CostTokens { parent: base, ..CostTokens::default() }, &self.sheet);
                    let mut plugin = base * (f - 1.0);
                    if k.get_bool("discountedcost").unwrap_or(false) {
                        plugin *= 0.9;
                    }
                    total += plugin;
                }
                None => total += self.without_modifiers(k, grade, base, Some(e)),
            }
        }
        total + gear_children_cost(e)
    }

    /// `CalculatedTotalCost`: the suite discount on top.
    fn total(&self, e: &Element, grade: &str, parent_cost: f64, limb: Option<&Element>) -> f64 {
        let t = self.without_modifiers(e, grade, parent_cost, limb);
        if e.get_bool("suite").unwrap_or(false) {
            t * 0.9
        } else {
            t
        }
    }
}

/// Total nuyen cost of saved ware with its children (`Cyberware.TotalCost`):
/// own cost × grade cost multiplier, black-market discount, genetech
/// multiplier, plus children (at the parent's grade) and gear, then the
/// suite discount.
pub fn cost(ch: &Character, store: &DataStore, e: &Element) -> f64 {
    cost_with(ch, store, e, None)
}

/// [`cost`] for ware installed in a vehicle mod (a drone arm or leg),
/// whose rating tokens come from the vehicle.
pub fn cost_in_vehicle(ch: &Character, store: &DataStore, e: &Element, vehicle: VehicleAttributes) -> f64 {
    cost_with(ch, store, e, Some(vehicle))
}

fn cost_with(ch: &Character, store: &DataStore, e: &Element, vehicle: Option<VehicleAttributes>) -> f64 {
    let attrs = sheet_attributes(ch, store);
    let ctx = CostCtx { ch, store, sheet: SheetAttributes(&attrs), vehicle };
    ctx.total(e, &e.get("grade"), 0.0, None)
}

/// Essence cost of saved ware (`Cyberware.CalculatedESS`), see
/// [`calc::ware_essence`].
pub fn essence(ch: &Character, store: &DataStore, rules: &Rules, e: &Element) -> f64 {
    let attrs = sheet_attributes(ch, store);
    calc::ware_essence(ch, e, &SheetAttributes(&attrs), Some(store), rules, None)
}

// ---------------------------------------------------------------------------
// Bonus hook
// ---------------------------------------------------------------------------

/// `AddImprovementCollection.addware`: install a free piece of ware owned
/// by the bonus source and record it with a `FreeWare` improvement.
/// Essence (anti)holes become a hole element at the rating; merging with an
/// existing hole (`Character.IncreaseEssenceHole`) needs a mutable
/// character and is not done here.
pub fn bonus_addware(ctx: &mut crate::bonus::Ctx<'_>, node: &Element) -> bool {
    let name = node.get("name");
    if name.is_empty() {
        return false;
    }
    let bioware = node.get("type") == "bioware";
    let (file, container, item, _) = data_source(bioware);
    let Ok(doc) = ctx.store.doc(file) else { return false };
    let Some(rec) = crate::data::find(&doc, container, item, &name) else { return false };
    let rating = node.child_text("rating").filter(|r| !r.is_empty()).map_or(1, |r| ctx.int(&r));
    let id = rec.id().to_ascii_lowercase();
    let is_hole = id == ESSENCE_HOLE_ID || id == ESSENCE_ANTIHOLE_ID;
    let inst = Install {
        grade: if is_hole { "None".to_owned() } else { resolve_grade(ctx.store, bioware, &node.get("grade")) },
        rating: if is_hole { rating } else { clamp_rating(ctx.ch, ctx.store, rec, rating, None) },
        parent_id: ctx.src.guid.clone(),
        cost: Some("0".into()),
        ..Install::default()
    };
    let rating = inst.rating;
    let guid = new_guid();
    let mut b = Builder { ch: ctx.ch, store: ctx.store, apply: !is_hole, out: Outcome::default() };
    let forced = ctx.selected.clone();
    let w = b.create(rec, bioware, inst, &guid, forced.as_deref(), false);
    let o = b.out;
    ctx.out.improvements.extend(o.improvements);
    ctx.out.flags.extend(o.flags);
    ctx.out.added.extend(o.added);
    ctx.out.unsupported.extend(o.unsupported);
    ctx.out.added.push(("cyberwares".into(), w));
    let mut i = ctx.imp("FreeWare", &guid);
    i.rating = rating;
    ctx.push(i);
    true
}
