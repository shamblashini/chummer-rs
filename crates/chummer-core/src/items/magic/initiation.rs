//! Initiation and submersion grades (`InitiationGrade`).

use super::Out;
use crate::character::Character;
use crate::xml::Element;

/// How a grade was taken; each option discounts its karma cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct GradeOptions {
    pub group: bool,
    pub ordeal: bool,
    pub schooling: bool,
}

/// Build an `<initiationgrade>` (`InitiationGrade.Save`).
pub fn element(guid: &str, grade: i32, technomancer: bool, o: GradeOptions) -> Element {
    let mut g = Out::new("initiationgrade");
    g.put("guid", guid);
    g.flag("res", technomancer);
    g.put("grade", grade.to_string());
    g.flag("group", o.group);
    g.flag("ordeal", o.ordeal);
    g.flag("schooling", o.schooling);
    g.put("notes", "");
    g.0
}

/// Take the next initiation grade (magicians, adepts) or submersion grade
/// (technomancers). Updates `<initiategrade>` / `<submersiongrade>`.
/// Karma is not deducted here; see `account::initiation_karma`.
pub fn add_grade(ch: &mut Character, o: GradeOptions) -> (String, i32) {
    let techno = ch.is_technomancer() && !ch.mag_enabled();
    let key = if techno { "submersiongrade" } else { "initiategrade" };
    let grade = ch.doc.get_i32(key).unwrap_or(0) + 1;
    let guid = super::super::new_guid();
    ch.items_mut("initiationgrades").push(element(&guid, grade, techno, o));
    ch.set_field(key, grade.to_string());
    (guid, grade)
}

/// Saved grades as (grade, technomancer, options).
pub fn grades(ch: &Character) -> Vec<(i32, bool, GradeOptions)> {
    ch.items("initiationgrades", "initiationgrade")
        .into_iter()
        .map(|g| {
            let b = |k: &str| g.get_bool(k).unwrap_or(false);
            (g.get_i32("grade").unwrap_or(0), b("res"), GradeOptions { group: b("group"), ordeal: b("ordeal"), schooling: b("schooling") })
        })
        .collect()
}
