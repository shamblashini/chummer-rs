//! Improvements: the modifiers that qualities, ware, powers and gear apply
//! to a character. They are saved in `.chum5` files, so a loaded character
//! carries all of its modifiers even where this port cannot yet create
//! them from a `<bonus>` node.

use crate::xml::Element;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Improvement {
    pub target: String,
    pub improved_name: String,
    /// GUID of the object that created this improvement.
    pub source_name: String,
    pub min: f64,
    pub max: f64,
    pub aug: f64,
    pub aug_max: f64,
    pub val: f64,
    pub rating: i32,
    pub exclude: String,
    pub condition: String,
    /// `ImprovementType` name, e.g. `"Attribute"`, `"Skill"`, `"PhysicalCM"`.
    pub kind: String,
    /// `ImprovementSource` name, e.g. `"Quality"`, `"Cyberware"`.
    pub source: String,
    pub custom: bool,
    pub custom_name: String,
    pub custom_id: String,
    pub custom_group: String,
    pub add_to_rating: bool,
    pub enabled: bool,
    pub order: i32,
    pub notes: String,
    /// Unique name used by "highest wins" stacking, from `<unique>`.
    pub unique_name: String,
}

impl Improvement {
    pub fn from_xml(e: &Element) -> Self {
        let f = |k: &str| e.get_f64(k).unwrap_or(0.0);
        Improvement {
            target: e.get("target"),
            improved_name: e.get("improvedname"),
            source_name: e.get("sourcename"),
            min: f("min"),
            max: f("max"),
            aug: f("aug"),
            aug_max: f("augmax"),
            val: f("val"),
            rating: e.get_i32("rating").unwrap_or(1),
            exclude: e.get("exclude"),
            condition: e.get("condition"),
            // Chummer's save format misspells this tag.
            kind: e.child_text("improvementttype").unwrap_or_else(|| e.get("improvementtype")),
            source: e.get("improvementsource"),
            custom: e.get_bool("custom").unwrap_or(false),
            custom_name: e.get("customname"),
            custom_id: e.get("customid"),
            custom_group: e.get("customgroup"),
            add_to_rating: int_or_bool(e, "addtorating", false),
            enabled: int_or_bool(e, "enabled", true),
            order: e.get_i32("order").unwrap_or(0),
            notes: e.get("notes"),
            unique_name: e.get("unique"),
        }
    }

    pub fn to_xml(&self) -> Element {
        let mut e = Element::new("improvement");
        let mut put = |k: &str, v: String| e.push(Element::with_text(k, v));
        put("target", self.target.clone());
        put("improvedname", self.improved_name.clone());
        put("sourcename", self.source_name.clone());
        put("min", fmt_num(self.min));
        put("max", fmt_num(self.max));
        put("aug", fmt_num(self.aug));
        put("augmax", fmt_num(self.aug_max));
        put("val", fmt_num(self.val));
        put("rating", self.rating.to_string());
        put("exclude", self.exclude.clone());
        if !self.unique_name.is_empty() {
            put("unique", self.unique_name.clone());
        }
        put("condition", self.condition.clone());
        put("improvementttype", self.kind.clone());
        put("improvementsource", self.source.clone());
        put("custom", bool_str(self.custom));
        put("customname", self.custom_name.clone());
        put("customid", self.custom_id.clone());
        put("customgroup", self.custom_group.clone());
        put("addtorating", bool_str(self.add_to_rating));
        put("enabled", bool_str(self.enabled));
        put("order", self.order.to_string());
        put("notes", self.notes.clone());
        e
    }

}

/// `addtorating` and `enabled` are saved either as `True`/`False` or as ints.
fn int_or_bool(e: &Element, key: &str, default: bool) -> bool {
    match e.child_text(key) {
        Some(t) => match crate::xml::parse_int(&t) {
            Some(n) => n > 0,
            None => crate::xml::parse_bool(&t),
        },
        None => default,
    }
}

pub fn bool_str(b: bool) -> String {
    if b { "True" } else { "False" }.to_owned()
}

/// Format like .NET's invariant decimal `ToString()`: no trailing `.0`.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Which numeric quantity of an improvement a query sums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `Value` (`ValueOf`). Not multiplied by rating.
    Val,
    /// `Augmented * Rating` (`AugmentedValueOf`).
    Aug,
    Min,
    Max,
    AugMax,
}

impl Field {
    pub fn get(self, i: &Improvement) -> f64 {
        match self {
            Field::Val => i.val,
            Field::Aug => i.aug * f64::from(i.rating),
            Field::Min => i.min,
            Field::Max => i.max,
            Field::AugMax => i.aug_max,
        }
    }
}

/// Filter for an improvement query.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    pub kind: &'a str,
    /// `None` or empty: every improvement of the kind, whatever its name.
    pub name: Option<&'a str>,
    /// Also match improvements with an empty improved name.
    pub include_non_improved: bool,
    pub add_to_rating: bool,
}

impl<'a> Query<'a> {
    pub fn new(kind: &'a str) -> Self {
        Query { kind, name: None, include_non_improved: false, add_to_rating: false }
    }
    pub fn named(kind: &'a str, name: &'a str) -> Self {
        Query { kind, name: Some(name), include_non_improved: false, add_to_rating: false }
    }
    pub fn with_non_improved(mut self) -> Self {
        self.include_non_improved = true;
        self
    }
    pub fn add_to_rating(mut self) -> Self {
        self.add_to_rating = true;
        self
    }
}

/// The character's improvement list with Chummer's aggregation rules
/// (`ImprovementManager.MetaValueOf`).
#[derive(Debug, Clone, Default)]
pub struct Improvements {
    pub list: Vec<Improvement>,
    /// Career mode. Improvements conditioned on `career`/`create` apply only
    /// in that mode.
    pub career: bool,
}

impl Improvements {
    /// Unconditional for the current mode.
    pub fn applies(&self, i: &Improvement) -> bool {
        if !i.enabled {
            return false;
        }
        let c = i.condition.trim();
        c.is_empty() || c == if self.career { "career" } else { "create" }
    }

    pub fn active(&self) -> impl Iterator<Item = &Improvement> {
        self.list.iter().filter(|i| self.applies(i))
    }

    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Improvement> + 'a {
        self.active().filter(move |i| i.kind == kind)
    }

    fn matching<'a>(&'a self, q: Query<'a>) -> impl Iterator<Item = &'a Improvement> + 'a {
        self.of_kind(q.kind).filter(move |i| {
            if i.add_to_rating != q.add_to_rating {
                return false;
            }
            match q.name {
                Some(n) if !n.is_empty() => i.improved_name == n || (q.include_non_improved && i.improved_name.is_empty()),
                _ => true,
            }
        })
    }

    /// The improvements that count after unique-name and precedence rules,
    /// ranked by `rank`. Chummer's `GetCachedImprovementListForValueOf`.
    pub fn winners<'a>(&'a self, q: Query<'a>, rank: Field) -> Vec<&'a Improvement> {
        let mut by_name: Vec<(&str, Vec<&Improvement>)> = Vec::new();
        for i in self.matching(q) {
            match by_name.iter_mut().find(|(n, _)| *n == i.improved_name) {
                Some((_, v)) => v.push(i),
                None => by_name.push((&i.improved_name, vec![i])),
            }
        }
        let mut out = Vec::new();
        for (_, group) in by_name {
            let (custom, normal): (Vec<&Improvement>, Vec<&Improvement>) = group.into_iter().partition(|i| i.custom);
            out.extend(select(&normal, rank, true));
            out.extend(select(&custom, rank, false));
        }
        out
    }

    /// Sum of `field` over the winners of a query.
    pub fn sum(&self, q: Query<'_>, field: Field) -> f64 {
        self.winners(q, field).iter().map(|i| field.get(i)).sum()
    }

    /// `ValueOf(kind, name)`. An empty or absent name means all names.
    pub fn val(&self, kind: &str, name: Option<&str>) -> f64 {
        self.sum(Query { kind, name, include_non_improved: false, add_to_rating: false }, Field::Val)
    }

    /// `ValueOf` rounded with `StandardRound`.
    pub fn val_int(&self, kind: &str, name: Option<&str>) -> i32 {
        crate::expr::standard_round(self.val(kind, name))
    }

    /// `AugmentedValueOf(kind, name)`: sum of `Augmented * Rating`.
    pub fn aug(&self, kind: &str, name: &str) -> f64 {
        self.sum(Query::named(kind, name), Field::Aug)
    }

    /// Plain sum over all matches, without unique-name handling. Used for
    /// skill pool bonuses.
    pub fn plain_sum(&self, kind: &str, name: &str) -> f64 {
        self.of_kind(kind).filter(|i| !i.add_to_rating && i.improved_name == name).map(|i| i.val).sum()
    }

    pub fn has(&self, kind: &str) -> bool {
        self.of_kind(kind).next().is_some()
    }

    pub fn has_named(&self, kind: &str, name: &str) -> bool {
        self.of_kind(kind).any(|i| i.improved_name == name)
    }

    /// Remove every improvement created by the object with this GUID.
    pub fn remove_from_source(&mut self, source_guid: &str) -> usize {
        let before = self.list.len();
        self.list.retain(|i| !i.source_name.eq_ignore_ascii_case(source_guid));
        before - self.list.len()
    }
}

/// Unique-name selection within one improved-name group.
fn select<'a>(items: &[&'a Improvement], rank: Field, precedence: bool) -> Vec<&'a Improvement> {
    let plain: Vec<&Improvement> = items.iter().copied().filter(|i| i.unique_name.is_empty()).collect();
    let plain_sum: f64 = plain.iter().map(|i| rank.get(i)).sum();
    let has = |u: &str| items.iter().any(|i| i.unique_name == u);
    let best = |pred: &dyn Fn(&Improvement) -> bool| -> Option<&'a Improvement> {
        items.iter().copied().filter(|i| pred(i)).max_by(|a, b| rank.get(a).total_cmp(&rank.get(b)))
    };
    if precedence && has("precedence0") {
        let hi = best(&|i| i.unique_name == "precedence0").unwrap();
        let minus: Vec<&Improvement> = items.iter().copied().filter(|i| i.unique_name == "precedence-1").collect();
        let total = rank.get(hi) + minus.iter().map(|i| rank.get(i)).sum::<f64>();
        if plain_sum >= total {
            return plain;
        }
        let mut v = vec![hi];
        v.extend(minus);
        return v;
    }
    if precedence && has("precedence1") {
        let set: Vec<&Improvement> = items.iter().copied().filter(|i| i.unique_name == "precedence1" || i.unique_name == "precedence-1").collect();
        let total: f64 = set.iter().map(|i| rank.get(i)).sum();
        return if plain_sum >= total { plain } else { set };
    }
    let mut out = plain;
    let mut seen: Vec<&str> = Vec::new();
    for i in items {
        if i.unique_name.is_empty() || seen.contains(&i.unique_name.as_str()) {
            continue;
        }
        seen.push(&i.unique_name);
        out.extend(best(&|j| j.unique_name == i.unique_name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(kind: &str, name: &str, val: f64, unique: &str) -> Improvement {
        Improvement { kind: kind.into(), improved_name: name.into(), val, unique_name: unique.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn sums_and_unique_stacking() {
        let set = Improvements {
            list: vec![
                imp("Skill", "Pistols", 1.0, ""),
                imp("Skill", "Pistols", 2.0, ""),
                imp("Skill", "Pistols", 3.0, "Reflexes"),
                imp("Skill", "Pistols", 1.0, "Reflexes"),
                imp("Skill", "Blades", 5.0, ""),
            ],
            career: false,
        };
        assert_eq!(set.val("Skill", Some("Pistols")), 6.0);
        assert_eq!(set.val("Skill", None), 11.0);
        assert_eq!(set.val("Skill", Some("")), 11.0);
    }

    #[test]
    fn precedence0_overrides() {
        let set = Improvements { list: vec![imp("Armor", "", 4.0, ""), imp("Armor", "", 9.0, "precedence0")], career: false };
        assert_eq!(set.val("Armor", None), 9.0);
        // precedence0 replaces only when higher than the plain sum
        let set = Improvements { list: vec![imp("Armor", "", 12.0, ""), imp("Armor", "", 9.0, "precedence0")], career: false };
        assert_eq!(set.val("Armor", None), 12.0);
        let set = Improvements {
            list: vec![imp("Armor", "", 4.0, ""), imp("Armor", "", 3.0, "precedence1"), imp("Armor", "", 3.0, "precedence-1")],
            career: false,
        };
        assert_eq!(set.val("Armor", None), 6.0);
    }

    #[test]
    fn disabled_and_conditional_are_skipped() {
        let mut a = imp("Dodge", "", 2.0, "");
        a.enabled = false;
        let mut b = imp("Dodge", "", 1.0, "");
        b.condition = "only in water".into();
        let mut c = imp("Dodge", "", 10.0, "");
        c.condition = "career".into();
        let mut set = Improvements { list: vec![a, b, c, imp("Dodge", "", 3.0, "")], career: false };
        assert_eq!(set.val("Dodge", None), 3.0);
        set.career = true;
        assert_eq!(set.val("Dodge", None), 13.0);
    }

    #[test]
    fn custom_improvements_add_without_precedence() {
        let mut c = imp("Initiative", "", 2.0, "precedence0");
        c.custom = true;
        let set = Improvements { list: vec![imp("Initiative", "", 1.0, ""), c], career: false };
        assert_eq!(set.val("Initiative", None), 3.0);
    }

    #[test]
    fn augmented_is_scaled_by_rating() {
        let mut i = imp("Attribute", "STR", 0.0, "");
        i.aug = 1.0;
        i.rating = 3;
        let set = Improvements { list: vec![i], career: false };
        assert_eq!(set.aug("Attribute", "STR"), 3.0);
        assert_eq!(set.aug("Attribute", "AGI"), 0.0);
    }

    #[test]
    fn xml_roundtrip() {
        let mut i = imp("Attribute", "STR", 0.0, "");
        i.aug = 2.0;
        i.source = "Cyberware".into();
        i.val = 0.5;
        let back = Improvement::from_xml(&i.to_xml());
        assert_eq!(back, i);
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(-3.0), "-3");
    }
}
