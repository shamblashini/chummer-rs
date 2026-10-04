//! Career calendar (`CalendarWeek`, `IsoWeekCalendar`): in-game weeks with
//! notes, saved as `<calendar><week>`.

use crate::character::Character;
use crate::items::new_guid;
use crate::xml::Element;

#[derive(Debug, Clone, PartialEq)]
pub struct Week {
    pub guid: String,
    pub year: i32,
    /// ISO week number, 1..=52 or 53.
    pub week: i32,
    pub notes: String,
}

impl Week {
    fn from_xml(e: &Element) -> Week {
        Week { guid: e.get("guid"), year: e.get_i32("year").unwrap_or(2072), week: e.get_i32("week").unwrap_or(1), notes: e.get("notes") }
    }

    fn to_xml(&self) -> Element {
        let mut e = Element::new("week");
        e.push(Element::with_text("guid", self.guid.clone()));
        e.push(Element::with_text("year", self.year.to_string()));
        e.push(Element::with_text("week", self.week.to_string()));
        e.push(Element::with_text("notes", self.notes.clone()));
        e.push(Element::with_text("notesColor", "Black"));
        e
    }

    /// Monday of the week as (year, month, day).
    pub fn monday(&self) -> (i32, u32, u32) {
        iso_week_start(self.year, self.week)
    }

    /// "Week 12, 2072 (Mar 21)".
    pub fn label(&self) -> String {
        let (_, m, d) = self.monday();
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        format!("Week {}, {} ({} {})", self.week, self.year, MONTHS[(m as usize).saturating_sub(1).min(11)], d)
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Weekday of a day number, Monday = 0.
fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// Monday of ISO week `week` in `year`.
pub fn iso_week_start(year: i32, week: i32) -> (i32, u32, u32) {
    let jan4 = days_from_civil(year, 1, 4);
    let week1_monday = jan4 - weekday(jan4);
    civil_from_days(week1_monday + i64::from(week - 1) * 7)
}

/// ISO years have 53 weeks when they start on a Thursday, or on a
/// Wednesday in a leap year (`IsYearLongYear`).
pub fn weeks_in_year(year: i32) -> i32 {
    let p = |y: i32| -> i64 { (i64::from(y) + i64::from(y).div_euclid(4) - i64::from(y).div_euclid(100) + i64::from(y).div_euclid(400)).rem_euclid(7) };
    if p(year) == 4 || p(year - 1) == 3 {
        53
    } else {
        52
    }
}

pub fn weeks(ch: &Character) -> Vec<Week> {
    ch.items("calendar", "week").into_iter().map(Week::from_xml).collect()
}

/// Add the week after the latest one, or the given start week when the
/// calendar is empty (`SelectCalendarStart`, default 2072 week 1).
pub fn add_next_week(ch: &mut Character, start: Option<(i32, i32)>) -> Week {
    let all = weeks(ch);
    let (year, week) = match all.iter().max_by_key(|w| (w.year, w.week)) {
        Some(last) if last.week >= weeks_in_year(last.year) => (last.year + 1, 1),
        Some(last) => (last.year, last.week + 1),
        None => start.unwrap_or((2072, 1)),
    };
    let w = Week { guid: new_guid(), year, week, notes: String::new() };
    ch.items_mut("calendar").push(w.to_xml());
    w
}

pub fn set_notes(ch: &mut Character, guid: &str, notes: &str) -> bool {
    let Some(cal) = ch.doc.child_mut("calendar") else { return false };
    match cal.elements_mut().find(|e| e.get("guid").eq_ignore_ascii_case(guid)) {
        Some(e) => {
            e.set_child_text("notes", notes);
            ch.dirty = true;
            true
        }
        None => false,
    }
}

pub fn remove_week(ch: &mut Character, guid: &str) -> bool {
    ch.remove_item("calendar", guid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_weeks() {
        assert_eq!(iso_week_start(2072, 1), (2072, 1, 4));
        assert_eq!(iso_week_start(2026, 1), (2025, 12, 29));
        assert_eq!(weeks_in_year(2026), 53);
        assert_eq!(weeks_in_year(2025), 52);
        assert_eq!(weeks_in_year(2020), 53);
    }

    #[test]
    fn add_and_roll_over() {
        let mut ch = Character::from_str("<character><name>x</name></character>").unwrap();
        let w = add_next_week(&mut ch, Some((2075, 52)));
        assert_eq!((w.year, w.week), (2075, 52));
        let w = add_next_week(&mut ch, None);
        // 2075 has 52 ISO weeks.
        assert_eq!((w.year, w.week), (2076, 1));
        assert!(set_notes(&mut ch, &w.guid, "Run in Redmond"));
        assert_eq!(weeks(&ch)[1].notes, "Run in Redmond");
    }
}
