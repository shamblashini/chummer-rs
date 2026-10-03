//! Option lists for bonus nodes that ask the user to choose
//! (`DoSelectSkill`, `selectattribute`, `selecttradition`, ...).

use super::{BonusSource, Choice};
use crate::character::Character;
use crate::data::{self, DataStore};
use crate::xml::Element;

fn csv(s: Option<&str>) -> Vec<String> {
    s.map(|v| v.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect()).unwrap_or_default()
}

/// Skills a `<selectskill>`-style node offers (`DoSelectSkill` filters).
pub fn skill_options(ch: &Character, store: &DataStore, node: &Element) -> Vec<String> {
    let knowledge = ["knowledgeskills", "knowledgeskill"].iter().any(|a| node.attr(a).is_some_and(|v| v.eq_ignore_ascii_case("true")));
    let categories: Vec<String> = {
        let mut c = csv(node.attr("skillcategory"));
        if let Some(cats) = node.child("skillcategories") {
            c.extend(cats.children_named("category").map(Element::text));
        }
        c
    };
    let exclude_cat = csv(node.attr("excludecategory"));
    let groups = csv(node.attr("skillgroup"));
    let exclude_groups = csv(node.attr("excludeskillgroup"));
    let limit = csv(node.attr("limittoskill"));
    let exclude = csv(node.attr("excludeskill"));
    let attrs = csv(node.attr("limittoattribute"));
    let Ok(doc) = store.doc("skills.xml") else { return Vec::new() };
    let container = if knowledge { "knowledgeskills" } else { "skills" };
    let mut out: Vec<String> = data::records(&doc, container, "skill")
        .into_iter()
        .filter(|r| !r.hidden())
        .filter(|r| categories.is_empty() || categories.contains(&r.category()))
        .filter(|r| !exclude_cat.contains(&r.category()))
        .filter(|r| groups.is_empty() || groups.contains(&r.get("skillgroup")))
        .filter(|r| !exclude_groups.contains(&r.get("skillgroup")))
        .filter(|r| limit.is_empty() || limit.contains(&r.name()))
        .filter(|r| !exclude.contains(&r.name()))
        .filter(|r| attrs.is_empty() || attrs.contains(&r.get("attribute")))
        .map(|r| r.name())
        .collect();
    if knowledge {
        for k in &ch.knowledge_skills {
            if !out.contains(&k.name) && (limit.is_empty() || limit.contains(&k.name)) && (categories.is_empty() || categories.contains(&k.kind)) {
                out.push(k.name.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Attributes an `<options>`/`<attribute>` list offers, minus disabled ones.
pub fn attribute_options(ch: &Character, names: impl Iterator<Item = String>) -> Vec<String> {
    names
        .filter(|n| n != "ESS")
        .filter(|n| match n.as_str() {
            "MAG" => ch.mag_enabled(),
            "MAGAdept" => ch.mag_enabled() && ch.is_adept() && ch.is_magician(),
            "RES" => ch.res_enabled(),
            "DEP" => ch.dep_enabled(),
            _ => true,
        })
        .collect()
}

fn names_of(store: &DataStore, file: &str, container: &str, item: &str) -> Vec<String> {
    store
        .doc(file)
        .map(|d| {
            let mut v: Vec<String> = data::records(&d, container, item).into_iter().filter(|r| !r.hidden()).map(|r| r.name()).collect();
            v.sort();
            v
        })
        .unwrap_or_default()
}

/// The selection a bonus node needs, if any.
pub fn choice_for(ch: &Character, store: &DataStore, node: &Element, src: &BonusSource) -> Option<Choice> {
    let mk = |prompt: String, options: Vec<String>| Some(Choice { node: node.name.clone(), prompt, options });
    match node.name.as_str() {
        "selecttext" => {
            let options = match (node.attr("xml"), node.attr("xpath")) {
                (Some(file), Some(xpath)) => {
                    // Support the common "/chummer/<container>/<item>" form.
                    let parts: Vec<&str> = xpath.trim_start_matches('/').split('/').collect();
                    match parts.as_slice() {
                        ["chummer", container, item] if !item.contains('[') => {
                            let doc = store.doc(file).ok()?;
                            let c = doc.child(container)?;
                            let mut v: Vec<String> = c
                                .elements()
                                .filter(|e| *item == "*" || e.name == *item)
                                .map(|e| e.child_text("name").unwrap_or_else(|| e.text()))
                                .filter(|s| !s.trim().is_empty())
                                .collect();
                            v.dedup();
                            v
                        }
                        _ => Vec::new(),
                    }
                }
                _ => Vec::new(),
            };
            mk(format!("Enter a value for {}", src.name), options)
        }
        "selectskill" => mk(format!("Choose a skill for {}", src.name), skill_options(ch, store, node)),
        "skilllevel" | "knowledgeskilllevel" | "skillgrouplevel" => {
            let sel = node.child("selectskill")?;
            if node.name == "knowledgeskilllevel" {
                let mut n = sel.clone();
                n.set_attr("knowledgeskills", "True");
                return mk(format!("Choose a knowledge skill for {}", src.name), skill_options(ch, store, &n));
            }
            mk(format!("Choose a skill for {}", src.name), skill_options(ch, store, sel))
        }
        "attributelevel" => {
            let opts = node.child("options")?;
            mk(format!("Choose an attribute for {}", src.name), attribute_options(ch, opts.elements().map(Element::text)))
        }
        "selectattribute" => {
            let mut names: Vec<String> = node.children_named("attribute").map(Element::text).collect();
            if names.is_empty() {
                names = ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES"].iter().map(|s| s.to_string()).collect();
            }
            let excluded: Vec<String> = node.children_named("excludeattribute").map(Element::text).collect();
            mk(format!("Choose an attribute for {}", src.name), attribute_options(ch, names.into_iter().filter(|n| !excluded.contains(n))))
        }
        "unlockskills" => {
            let opts = csv(Some(&node.text()));
            (opts.len() > 1).then(|| Choice { node: node.name.clone(), prompt: format!("Choose skills to unlock for {}", src.name), options: opts })
        }
        "selecttradition" => mk("Choose a tradition".into(), names_of(store, "traditions.xml", "traditions", "tradition")),
        "selectmentorspirit" => mk("Choose a mentor spirit".into(), names_of(store, "mentors.xml", "mentors", "mentor")),
        "selectparagon" => mk("Choose a paragon".into(), names_of(store, "paragons.xml", "mentors", "mentor")),
        "selectrestricted" => mk(format!("Name the restricted item for {}", src.name), Vec::new()),
        "selectweapon" => mk(format!("Choose a weapon for {}", src.name), names_of(store, "weapons.xml", "weapons", "weapon")),
        "selectarmor" => mk(format!("Choose armor for {}", src.name), names_of(store, "armor.xml", "armors", "armor")),
        _ => None,
    }
}
