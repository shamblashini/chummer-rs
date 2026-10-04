//! Character sheets: the *print XML* and its XSLT rendering.
//!
//! Chummer5a prints a character by writing a separate XML document
//! (`Character.PrintToXmlTextWriter` and the `Print` methods of every
//! object it owns), wrapping it in `<characters>`
//! (`CommonFunctions.GenerateCharactersExportXml`) and transforming it
//! with one of the XSLT sheets in `resources/sheets`. This module builds
//! the same document and runs the sheets through `xsltproc`.
//!
//! The print XML differs from the `.chum5` save: it carries computed
//! totals (attribute totals, dice pools, limits, initiative strings,
//! condition monitors), display names in the print language beside the
//! English ones (`name` / `name_english`), and formatted nuyen. Values
//! the port does not compute yet (item cost/avail totals, weapon
//! accuracy modifiers and similar) are taken from the saved fields.
//!
//! Numbers are formatted with the invariant culture: a German sheet gets
//! German labels and names but `1,000.5¥` rather than `1.000,5¥`.

mod character;
mod items;
mod magic;
mod render;
mod skills;
mod social;
mod vehicles;

pub use render::{
    available_sheets, available_sheets_in, find_sheet, html_to_pdf, pdf_converter, render, render_report, RenderError, RenderReport, DEFAULT_SHEET,
};

use crate::calc::{Rules, Sheet};
use crate::character::Character;
use crate::engine::Engine;
use crate::expr::NoAttributes;
use crate::lang::Language;
use crate::xml::Element;

/// Global print options (`GlobalSettings.PrintNotes`,
/// `PrintExpenses`, `PrintFreeExpenses`). Defaults match Chummer5a.
#[derive(Debug, Clone, Copy)]
pub struct PrintOptions {
    /// Print the notes of items, skills and improvements.
    pub notes: bool,
    /// Print the karma and nuyen expense log.
    pub expenses: bool,
    /// With `expenses`, also print entries of zero amount.
    pub free_expenses: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions { notes: false, expenses: false, free_expenses: true }
    }
}

/// The print XML for one character: `<characters><character>...`
/// with Chummer5a's default print options.
pub fn print_xml(ch: &Character, engine: &Engine, lang: &Language) -> Element {
    print_xml_with(ch, engine, lang, PrintOptions::default())
}

/// As [`print_xml`], with explicit options.
pub fn print_xml_with(ch: &Character, engine: &Engine, lang: &Language, opts: PrintOptions) -> Element {
    let mut root = Element::new("characters");
    root.push(print_character(ch, engine, lang, opts));
    root
}

/// The `<character>` element alone (`Character.PrintToXmlTextWriterCore`).
pub fn print_character(ch: &Character, engine: &Engine, lang: &Language, opts: PrintOptions) -> Element {
    let rules = engine.rules_for(ch);
    let sheet = crate::calc::compute(ch, &rules, Some(&engine.store), Some(&engine.catalog));
    let ctx = Ctx { ch, engine, lang, sheet, rules, opts };
    character::print(&ctx)
}

/// Everything a `Print` method needs.
pub(crate) struct Ctx<'a> {
    pub ch: &'a Character,
    pub engine: &'a Engine,
    pub lang: &'a Language,
    pub sheet: Sheet,
    pub rules: Rules,
    pub opts: PrintOptions,
}

impl Ctx<'_> {
    /// `LanguageManager.GetString(key, strLanguageToPrint)`.
    pub fn s(&self, key: &str) -> String {
        self.lang.s(key)
    }

    /// Translated display name of a saved item (`DisplayNameShort`).
    pub fn tr_name(&self, file: &str, item: &Element) -> String {
        let id = [item.get("sourceid"), item.get("id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        self.lang.data_name(file, &id, &item.get("name"))
    }

    /// Translated category (`DisplayCategory`).
    pub fn tr_category(&self, file: &str, category: &str) -> String {
        self.lang.data_name(file, "", category)
    }

    /// `ToString(Settings.NuyenFormat, objCulture)`: `#,0.##`, no symbol
    /// (the sheets append `¥`).
    pub fn nuyen(&self, v: f64) -> String {
        crate::format::nuyen(v).trim_end_matches('¥').to_owned()
    }

    /// `Settings.EssenceFormat`.
    pub fn essence(&self, v: f64) -> String {
        crate::format::essence(v, self.rules.essence_decimals)
    }

    /// Append `<notes>` when notes are printed.
    pub fn notes(&self, out: &mut Element, item: &Element) {
        if self.opts.notes {
            add(out, "notes", item.get("notes"));
        }
    }

    /// Name of a saved location by guid (`Location.DisplayName`); plain
    /// text is returned as is.
    pub fn location(&self, value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }
        for list in ["gearlocations", "armorlocations", "weaponlocations", "vehiclelocations"] {
            if let Some(loc) = self.ch.items(list, "location").into_iter().find(|l| l.get("guid").eq_ignore_ascii_case(value)) {
                return loc.get("name");
            }
        }
        if looks_like_guid(value) { String::new() } else { value.to_owned() }
    }
}

fn looks_like_guid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Append `<name>value</name>`.
pub(crate) fn add(out: &mut Element, name: &str, value: impl Into<String>) {
    out.push(Element::with_text(name, value));
}

/// Copy a saved child's text under the same name.
pub(crate) fn copy(out: &mut Element, item: &Element, name: &str) {
    add(out, name, item.get(name));
}

/// `bool.ToString(CultureInfo.InvariantCulture)`.
pub(crate) fn bool_text(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

/// A saved boolean, printed as `True`/`False`.
pub(crate) fn copy_bool(out: &mut Element, item: &Element, name: &str) {
    add(out, name, bool_text(item.get_bool(name).unwrap_or(false)));
}

/// `#,0.##`: up to two decimals, no trailing zeros.
pub(crate) fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_owned() }
}

/// `+#,0.##;-#,0.##;0.##`: signed number.
pub(crate) fn signed(v: f64) -> String {
    if v > 0.0 { format!("+{}", num(v)) } else { num(v) }
}

/// Evaluate a saved cost/weight expression at a rating (`Rating * 2500`).
pub(crate) fn eval(expr: &str, rating: i32) -> f64 {
    let e = expr.trim();
    if e.is_empty() {
        return 0.0;
    }
    crate::expr::value_to_dec(e, rating, &NoAttributes)
}

/// Saved availability evaluated at the item's rating (`TotalAvail`,
/// own value only).
pub(crate) fn avail(item: &Element) -> String {
    let raw = item.get("avail");
    if raw.trim().is_empty() {
        return "0".into();
    }
    let rating = item.get_i32("rating").unwrap_or(0);
    let min = item.get_i32("minrating").unwrap_or(0);
    crate::expr::Availability::parse(&raw, rating, min, &NoAttributes).to_string()
}

/// `DisplayName`: `"[qty ]Name[ (Rating N)][ (extra)][ ("custom")]"`.
pub(crate) fn full_name(ctx: &Ctx, name: &str, qty: Option<f64>, rating: i32, extra: &str, custom: &str) -> String {
    let mut s = String::new();
    if let Some(q) = qty.filter(|q| *q != 1.0) {
        s.push_str(&num(q));
        s.push(' ');
    }
    s.push_str(name);
    if rating > 0 {
        s.push_str(&format!(" ({} {rating})", ctx.s("String_Rating")));
    }
    if !extra.is_empty() {
        s.push_str(&format!(" ({extra})"));
    }
    if !custom.is_empty() {
        s.push_str(&format!(" (\"{custom}\")"));
    }
    s
}
