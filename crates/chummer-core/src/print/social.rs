//! Contacts, qualities, lifestyles, the expense log, calendar and the
//! improvement-made armor list.

use std::collections::HashMap;

use super::{add, bool_text, copy, copy_bool, num, Ctx};
use crate::xml::Element;

/// `<contacts>` (`Contact.Print`).
pub fn contacts(ctx: &Ctx) -> Element {
    let mut out = Element::new("contacts");
    for c in ctx.ch.items("contacts", "contact") {
        out.push(contact(ctx, c));
    }
    out
}

fn contact(ctx: &Ctx, c: &Element) -> Element {
    let mut out = Element::new("contact");
    copy(&mut out, c, "guid");
    copy(&mut out, c, "name");
    add(&mut out, "role", ctx.lang.data_name("contacts.xml", "", &c.get("role")));
    copy(&mut out, c, "location");
    let connection = c.get_i32("connection").unwrap_or(0);
    let shown = if c.get_bool("group").unwrap_or(false) { format!("{}({connection})", ctx.s("String_Group")) } else { connection.to_string() };
    add(&mut out, "connection", shown);
    add(&mut out, "loyalty", c.get_i32("loyalty").unwrap_or(0).to_string());
    add(&mut out, "metatype", c.get("metatype"));
    add(&mut out, "gender", [c.get("gender"), c.get("sex")].into_iter().find(|s| !s.is_empty()).unwrap_or_default());
    for f in ["age", "contacttype", "preferredpayment", "hobbiesvice", "personallife"] {
        add(&mut out, f, ctx.lang.data_name("contacts.xml", "", &c.get(f)));
    }
    let kind = c.child_text("type").filter(|t| !t.is_empty()).unwrap_or_else(|| "Contact".into());
    add(&mut out, "type", ctx.s(&format!("String_{kind}")));
    add(&mut out, "forcedloyalty", c.get_i32("forcedloyalty").unwrap_or(0).to_string());
    copy_bool(&mut out, c, "blackmail");
    copy_bool(&mut out, c, "family");
    ctx.notes(&mut out, c);
    out
}

/// `<qualities>`: one `Quality.Print` per distinct quality, with the
/// number of copies as its rating (`Character.PrintToXmlTextWriterCore`).
pub fn qualities(ctx: &Ctx) -> Element {
    let mut out = Element::new("qualities");
    let all = ctx.ch.items("qualities", "quality");
    let key = |q: &Element| format!("{}|{}|{}", q.get("id").to_ascii_lowercase(), q.get("sourcename"), q.get("extra"));
    let mut counts: HashMap<String, i32> = HashMap::new();
    for q in &all {
        *counts.entry(key(q)).or_default() += 1;
    }
    let mut done = Vec::new();
    for q in all {
        let k = key(q);
        if done.contains(&k) || !q.get_bool("print").unwrap_or(true) {
            continue;
        }
        out.push(quality(ctx, q, counts[&k]));
        done.push(k);
    }
    out
}

/// `Quality.Print`.
fn quality(ctx: &Ctx, q: &Element, count: i32) -> Element {
    let mut out = Element::new("quality");
    copy(&mut out, q, "guid");
    add(&mut out, "sourceid", q.get("id"));
    let name = ctx.tr_name("qualities.xml", q);
    add(&mut out, "name", name.clone());
    add(&mut out, "name_english", q.get("name"));
    let extra = q.get("extra");
    let decorate = |n: &str| {
        let mut s = n.to_owned();
        if !extra.is_empty() {
            s.push_str(&format!(" ({extra})"));
        }
        if count > 1 {
            s.push_str(&format!(" {count}"));
        }
        s
    };
    add(&mut out, "fullname", decorate(&name));
    add(&mut out, "fullname_english", decorate(&q.get("name")));
    let mut ex = extra.clone();
    if count > 1 {
        ex.push_str(&format!(" {count}"));
    }
    let source_name = q.get("sourcename");
    if !source_name.trim().is_empty() {
        ex.push_str(&format!(" ({source_name})"));
    }
    add(&mut out, "extra", ex.clone());
    add(&mut out, "extra_english", ex);
    add(&mut out, "bp", q.get_i32("bp").unwrap_or(0).to_string());
    let kind = q.get("qualitytype");
    add(&mut out, "qualitytype", ctx.tr_category("qualities.xml", &kind));
    add(&mut out, "qualitytype_english", kind);
    copy(&mut out, q, "qualitysource");
    add(&mut out, "metagenic", bool_text(q.get_bool("metagenic").or(q.get_bool("metagenetic")).unwrap_or(false)));
    copy(&mut out, q, "source");
    copy(&mut out, q, "page");
    ctx.notes(&mut out, q);
    out
}

/// `Lifestyle.Print`; totals from the saved cost, qualities, share and
/// months.
pub fn lifestyle(ctx: &Ctx, l: &Element) -> Element {
    let mut out = Element::new("lifestyle");
    copy(&mut out, l, "guid");
    copy(&mut out, l, "sourceid");
    copy(&mut out, l, "name");
    for f in ["city", "district", "borough"] {
        copy(&mut out, l, f);
    }
    let qualities: Vec<&Element> = l.child("lifestylequalities").map(|c| c.children_named("lifestylequality").collect()).unwrap_or_default();
    let cost = l.get_f64("cost").unwrap_or(0.0);
    let extra: f64 = qualities.iter().filter(|q| !q.get_bool("free").unwrap_or(false)).map(|q| q.get_f64("cost").unwrap_or(0.0)).sum();
    let mult: f64 = qualities.iter().map(|q| q.get_f64("multiplier").unwrap_or(0.0)).sum();
    let share = l.get_f64("percentage").unwrap_or(100.0) / 100.0;
    let monthly = ((cost + extra) * (1.0 + mult / 100.0) * share).max(0.0);
    let months = l.get_i32("months").unwrap_or(1);
    add(&mut out, "cost", ctx.nuyen(cost));
    add(&mut out, "totalmonthlycost", ctx.nuyen(monthly));
    add(&mut out, "totalcost", ctx.nuyen(monthly * f64::from(months)));
    add(&mut out, "dice", l.get_i32("dice").unwrap_or(0).to_string());
    add(&mut out, "multiplier", ctx.nuyen(l.get_f64("multiplier").unwrap_or(0.0)));
    add(&mut out, "months", months.to_string());
    copy_bool(&mut out, l, "purchased");
    add(&mut out, "type", l.child_text("type").filter(|t| !t.is_empty()).unwrap_or_else(|| "Standard".into()));
    add(&mut out, "increment", l.child_text("increment").filter(|t| !t.is_empty()).unwrap_or_else(|| "Month".into()));
    add(&mut out, "bonuslp", l.get_i32("bonuslp").unwrap_or(0).to_string());
    let base = l.get("baselifestyle");
    add(&mut out, "baselifestyle", ctx.lang.data_name("lifestyles.xml", "", &base));
    add(&mut out, "baselifestyle_english", base);
    copy_bool(&mut out, l, "trustfund");
    copy(&mut out, l, "source");
    copy(&mut out, l, "page");
    let mut list = Element::new("qualities");
    for q in qualities.into_iter().filter(|q| q.get_bool("print").unwrap_or(true)) {
        list.push(lifestyle_quality(ctx, q));
    }
    out.push(list);
    ctx.notes(&mut out, l);
    out
}

/// `LifestyleQuality.Print`.
fn lifestyle_quality(ctx: &Ctx, q: &Element) -> Element {
    let mut out = Element::new("quality");
    copy(&mut out, q, "guid");
    add(&mut out, "sourceid", q.get("id"));
    let name = ctx.tr_name("lifestyles.xml", q);
    let extra = q.get("extra");
    let full = |n: &str| if extra.is_empty() { n.to_owned() } else { format!("{n} ({extra})") };
    add(&mut out, "name", name.clone());
    add(&mut out, "name_english", q.get("name"));
    add(&mut out, "fullname", full(&name));
    add(&mut out, "fullname_english", full(&q.get("name")));
    add(&mut out, "formattedname", full(&name));
    add(&mut out, "formattedname_english", full(&q.get("name")));
    add(&mut out, "extra", extra.clone());
    add(&mut out, "lp", q.get_i32("lp").unwrap_or(0).to_string());
    add(&mut out, "cost", ctx.nuyen(q.get_f64("cost").unwrap_or(0.0)));
    let kind = q.get("lifestylequalitytype");
    add(&mut out, "lifestylequalitytype", ctx.tr_category("lifestyles.xml", &kind));
    add(&mut out, "lifestylequalitytype_english", kind);
    add(&mut out, "lifestylequalitysource", q.child_text("lifestylequalitysource").filter(|s| !s.is_empty()).unwrap_or_else(|| "Selected".into()));
    for f in ["free", "freebylifestyle", "isfreegrid"] {
        copy_bool(&mut out, q, f);
    }
    copy(&mut out, q, "source");
    copy(&mut out, q, "page");
    ctx.notes(&mut out, q);
    out
}

/// `<otherarmors>`: Armor improvements not made by armor itself.
pub fn other_armors(ctx: &Ctx) -> Element {
    let mut out = Element::new("otherarmors");
    for i in ctx.ch.improvements.of_kind("Armor").filter(|i| i.source != "Armor" && i.source != "ArmorMod") {
        let mut e = Element::new("otherarmor");
        add(&mut e, "guid", i.source_name.clone());
        add(&mut e, "sourcename", i.source_name.clone());
        let name = super::magic::object_name(ctx, &i.source_name, &i.custom_name);
        add(&mut e, "objectname", name.clone());
        add(&mut e, "objectname_english", name);
        add(&mut e, "armor", num(i.val));
        add(&mut e, "improvesource", i.source.clone());
        add(&mut e, "enabled", bool_text(i.enabled));
        add(&mut e, "customname", i.custom_name.clone());
        add(&mut e, "customgroup", i.custom_group.clone());
        if ctx.opts.notes {
            add(&mut e, "notes", i.notes.clone());
        }
        out.push(e);
    }
    out
}

/// `<calendar>`: weeks that have notes (`CalendarWeek.Print`).
pub fn calendar(ctx: &Ctx) -> Element {
    let mut out = Element::new("calendar");
    for w in ctx.ch.items("calendar", "week").into_iter().filter(|w| !w.get("notes").trim().is_empty()) {
        let mut e = Element::new("week");
        copy(&mut e, w, "guid");
        add(&mut e, "year", w.get_i32("year").unwrap_or(0).to_string());
        add(&mut e, "week", w.get_i32("week").unwrap_or(0).to_string());
        copy(&mut e, w, "notes");
        out.push(e);
    }
    out
}

/// `<expenses>`, newest first (`ExpenseLogEntry.Print`).
pub fn expenses(ctx: &Ctx) -> Element {
    let mut out = Element::new("expenses");
    for x in ctx.ch.items("expenses", "expense").into_iter().rev() {
        let amount = x.get_f64("amount").unwrap_or(0.0);
        if amount == 0.0 && !ctx.opts.free_expenses {
            continue;
        }
        let mut e = Element::new("expense");
        copy(&mut e, x, "guid");
        add(&mut e, "date", general_date(&x.get("date")));
        let kind = x.child_text("type").unwrap_or_else(|| "Karma".into());
        add(&mut e, "amount", if kind == "Nuyen" { ctx.nuyen(amount) } else { num(amount) });
        let refund = x.get_bool("refund").unwrap_or(false);
        let reason = x.get("reason");
        add(&mut e, "reason", if refund { format!("{reason} ({})", ctx.s("String_Expense_Refund")) } else { reason });
        add(&mut e, "type", kind);
        add(&mut e, "refund", bool_text(refund));
        out.push(e);
    }
    out
}

/// `DateTime.ToString(en-US)` ("G"): `9/16/2018 12:00:23 AM` from the
/// saved sortable `2018-09-16T00:00:23`.
fn general_date(iso: &str) -> String {
    let parse = || -> Option<String> {
        let (d, t) = iso.split_once('T')?;
        let mut dp = d.split('-').map(|x| x.parse::<u32>().ok());
        let (y, m, day) = (dp.next()??, dp.next()??, dp.next()??);
        let t = t.split(['.', '+', 'Z']).next()?;
        let mut tp = t.split(':').map(|x| x.parse::<u32>().ok());
        let (h, mi, s) = (tp.next()??, tp.next()??, tp.next().flatten().unwrap_or(0));
        let (h12, ampm) = match h {
            0 => (12, "AM"),
            1..=11 => (h, "AM"),
            12 => (12, "PM"),
            _ => (h - 12, "PM"),
        };
        Some(format!("{m}/{day}/{y} {h12}:{mi:02}:{s:02} {ampm}"))
    };
    parse().unwrap_or_else(|| iso.to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::general_date("2018-09-16T00:00:23"), "9/16/2018 12:00:23 AM");
        assert_eq!(super::general_date("2020-01-02T13:05:00.123"), "1/2/2020 1:05:00 PM");
        assert_eq!(super::general_date("junk"), "junk");
    }
}
