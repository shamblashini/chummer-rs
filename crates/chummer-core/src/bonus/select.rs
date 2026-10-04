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

/// Limits a `<selectlimit>` offers: its `<limit>`s (default all three)
/// minus `<excludelimit>`s.
pub fn limit_options(node: &Element) -> Vec<String> {
    let mut v: Vec<String> = node.children_named("limit").map(Element::text).collect();
    if v.is_empty() {
        v = ["Physical", "Mental", "Social"].iter().map(|s| s.to_string()).collect();
    }
    let ex: Vec<String> = node.children_named("excludelimit").map(Element::text).collect();
    v.retain(|l| !ex.contains(l));
    v
}

/// options.xml `blackmarketpipelinecategories/category`.
pub fn black_market_categories(store: &DataStore) -> Vec<String> {
    store.doc("options.xml").ok().and_then(|d| d.child("blackmarketpipelinecategories").map(|c| c.children_named("category").map(Element::text).collect())).unwrap_or_default()
}

/// `<dealerconnection>` categories not already taken by a Dealer
/// Connection improvement.
pub fn dealer_options(ch: &Character, node: &Element) -> Vec<String> {
    node.children_named("category")
        .map(Element::text)
        .filter(|c| !c.is_empty() && !ch.improvements.list.iter().any(|i| i.kind == "DealerConnection" && i.unique_name == *c))
        .collect()
}

const ATTRIBUTES: &[&str] = &["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "MAGAdept", "RES", "DEP"];

/// Attributes a `swapskillspecattribute` offers: its `<attribute>`s, else
/// all minus `<excludeattribute>`s.
pub fn swap_attribute_options(ch: &Character, node: &Element) -> Vec<String> {
    let listed: Vec<String> = node.children_named("attribute").map(Element::text).collect();
    let names = if listed.is_empty() {
        let ex: Vec<String> = node.children_named("excludeattribute").map(Element::text).collect();
        ATTRIBUTES.iter().map(|s| s.to_string()).filter(|a| !ex.contains(a)).collect()
    } else {
        listed
    };
    attribute_options(ch, names.into_iter())
}

/// Sprites `selectsprite` (critters.xml "Sprites") or `addsprite`
/// (streams.xml spirits plus critters.xml sprites, limited to the listed
/// `<spirit>`s) offers.
pub fn sprite_options(store: &DataStore, node: &Element) -> Vec<String> {
    let allowed: Vec<String> = node.children_named("spirit").map(Element::text).collect();
    let ok = |n: &String| allowed.is_empty() || allowed.contains(n);
    let mut v: Vec<String> = Vec::new();
    if node.name == "addsprite" {
        if let Ok(d) = store.doc("streams.xml") {
            v.extend(data::records(&d, "spirits", "spirit").iter().map(|r| r.name()).filter(ok));
        }
    }
    if let Ok(d) = store.doc("critters.xml") {
        v.extend(data::records(&d, "metatypes", "metatype").iter().filter(|r| r.category().contains("Sprites")).map(|r| r.name()).filter(ok));
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|n| seen.insert(n.clone()));
    v
}

/// (guid, name) of the contacts `selectcontact` offers: all, or only
/// group / non-group ones per `<type>`.
pub fn contact_choices(ch: &Character, node: &Element) -> Vec<(String, String)> {
    let mode = node.child_text("type").unwrap_or_else(|| "all".into());
    ch.items("contacts", "contact")
        .into_iter()
        .filter(|c| {
            let group = c.get_bool("group").unwrap_or(false);
            match mode.as_str() {
                "group" => group,
                "nongroup" => !group,
                _ => true,
            }
        })
        .map(|c| (c.get("guid"), c.get("name")))
        .collect()
}

/// Spell categories (spells.xml), minus a comma-separated `exclude`.
fn spell_categories(store: &DataStore, node: &Element) -> Vec<String> {
    let ex = csv(node.attr("exclude"));
    store.doc("spells.xml").map(|d| data::categories(&d).into_iter().filter(|c| !ex.contains(c)).collect()).unwrap_or_default()
}

/// Records of `file`, filtered by category.
fn names_where(store: &DataStore, file: &str, container: &str, item: &str, keep: impl Fn(&data::Record<'_>) -> bool) -> Vec<String> {
    store
        .doc(file)
        .map(|d| {
            let mut v: Vec<String> = data::records(&d, container, item).into_iter().filter(|r| !r.hidden() && keep(r)).map(|r| r.name()).collect();
            v.sort();
            v.dedup();
            v
        })
        .unwrap_or_default()
}

/// Specializations offered for an expertise: `limittospecialization`, or
/// the skill's specializations in skills.xml.
fn expertise_options(store: &DataStore, node: &Element, skill: &str) -> Vec<String> {
    let limited = csv(node.attr("limittospecialization"));
    if !limited.is_empty() {
        return limited;
    }
    store
        .doc("skills.xml")
        .ok()
        .and_then(|d| data::find(&d, "skills", "skill", skill).and_then(|r| r.el().child("specs").map(|s| s.children_named("spec").map(Element::text).collect())))
        .unwrap_or_default()
}

/// Options for a selecting `select*` node that lists its candidates as
/// `child` elements, else every record.
fn listed_or_all(store: &DataStore, node: &Element, child: &str, file: &str, container: &str, item: &str) -> Vec<String> {
    let listed: Vec<String> = node.children_named(child).map(Element::text).collect();
    if listed.is_empty() { names_of(store, file, container, item) } else { listed }
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
        "weaponskillaccuracy" => {
            let sel = node.child("selectskill")?;
            mk(format!("Choose a skill for {}", src.name), skill_options(ch, store, sel))
        }
        "weaponcategorydice" | "weaponcategorydv" | "weaponcategoryap" | "weaponcategoryaccuracy" | "weaponcategoryreach" => {
            if let Some(sel) = node.child("selectskill") {
                return mk(format!("Choose a weapon skill for {}", src.name), skill_options(ch, store, sel));
            }
            let cats: Vec<String> = node.child("selectcategory")?.children_named("category").map(Element::text).collect();
            mk(format!("Choose a weapon category for {}", src.name), cats)
        }
        "selectattributes" => {
            // One answer applies to every child; "BOD (2), INT (1)" spreads picks.
            let first = node.child("selectattribute")?;
            let mut names: Vec<String> = first.children_named("attribute").map(Element::text).collect();
            if names.is_empty() {
                names = ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL", "EDG", "MAG", "RES"].iter().map(|s| s.to_string()).collect();
            }
            let excluded: Vec<String> = first.children_named("excludeattribute").map(Element::text).collect();
            mk(format!("Choose attributes for {}", src.name), attribute_options(ch, names.into_iter().filter(|n| !excluded.contains(n))))
        }
        "selectlimit" => mk(format!("Choose a limit for {}", src.name), limit_options(node)),
        "selectside" => mk(format!("Choose a side for {}", src.name), vec!["Left".into(), "Right".into()]),
        "selectcyberware" => {
            let cat = node.get("category");
            mk(format!("Choose cyberware for {}", src.name), names_where(store, "cyberware.xml", "cyberwares", "cyberware", |r| cat.is_empty() || r.category() == cat))
        }
        "blackmarketdiscount" => {
            let cats = black_market_categories(store);
            (!cats.is_empty()).then(|| Choice { node: node.name.clone(), prompt: format!("Choose a black market category for {}", src.name), options: cats })
        }
        "dealerconnection" => mk(format!("Choose a dealer category for {}", src.name), dealer_options(ch, node)),
        "skillgroupdisablechoice" => {
            let mut groups: Vec<String> = node.children_named("skillgroup").map(Element::text).collect();
            if groups.is_empty() {
                groups = ch.skill_groups.iter().map(|g| g.name.clone()).collect();
            }
            groups.sort();
            mk("Choose a skill group to disable".into(), groups)
        }
        "swapskillspecattribute" => {
            let opts = swap_attribute_options(ch, node);
            (opts.len() > 1).then(|| Choice { node: node.name.clone(), prompt: format!("Choose an attribute for {}", src.name), options: opts })
        }
        "actiondicepool" => {
            if node.child("name").is_some() {
                return None;
            }
            let cat = node.attr("category").unwrap_or("").to_owned();
            mk(format!("Choose an action for {}", src.name), names_where(store, "actions.xml", "actions", "action", |r| cat.is_empty() || r.category() == cat))
        }
        "hardwires" | "activesoft" => {
            if node.attr("select").is_some_and(|s| !s.is_empty()) {
                return None;
            }
            mk(format!("Choose a skill for {}", src.name), skill_options(ch, store, node))
        }
        "skillsoft" => {
            let mut n = node.clone();
            n.set_attr("knowledgeskills", "True");
            mk(format!("Choose a knowledge skill for {}", src.name), skill_options(ch, store, &n))
        }
        "selectexpertise" => {
            let skills = skill_options(ch, store, node);
            let [skill] = skills.as_slice() else { return None };
            mk(format!("Choose an expertise in {skill} for {}", src.name), expertise_options(store, node, skill))
        }
        "selectinherentaiprogram" => mk(
            "Choose an inherent program".into(),
            names_where(store, "programs.xml", "programs", "program", |r| matches!(r.category().as_str(), "Common Programs" | "Hacking Programs")),
        ),
        "weaponspecificdice" => {
            let ty = node.attr("type").unwrap_or("");
            let w: Vec<String> = ch.items("weapons", "weapon").into_iter().filter(|w| ty.is_empty() || w.get("type") == ty).map(|w| w.get("name")).collect();
            mk(format!("Choose a weapon for {}", src.name), w)
        }
        "selectquality" => mk(format!("Choose a quality for {}", src.name), node.children_named("quality").map(Element::text).collect()),
        "selectcontact" => mk(format!("Choose a contact for {}", src.name), contact_choices(ch, node).into_iter().map(|(_, n)| n).collect()),
        "selectspell" => {
            let cat = node.attr("category").unwrap_or("").to_owned();
            mk(format!("Choose a spell for {}", src.name), names_where(store, "spells.xml", "spells", "spell", |r| (cat.is_empty() || r.category() == cat) && !r.name().contains(", Extended")))
        }
        "selectcomplexform" => mk(format!("Choose a complex form for {}", src.name), names_of(store, "complexforms.xml", "complexforms", "complexform")),
        "selectart" => mk(format!("Choose an art for {}", src.name), listed_or_all(store, node, "art", "metamagic.xml", "arts", "art")),
        "selectmetamagic" => mk(format!("Choose a metamagic for {}", src.name), listed_or_all(store, node, "metamagic", "metamagic.xml", "metamagics", "metamagic")),
        "selectecho" => mk(format!("Choose an echo for {}", src.name), listed_or_all(store, node, "echo", "echoes.xml", "echoes", "echo")),
        "selectsprite" => mk(format!("Choose a sprite for {}", src.name), sprite_options(store, node)),
        "addsprite" => {
            let opts = sprite_options(store, node);
            (opts.len() > 1).then(|| Choice { node: node.name.clone(), prompt: format!("Choose a sprite type for {}", src.name), options: opts })
        }
        "allowspellcategory" | "limitspellcategory" if node.text().trim().is_empty() => mk(format!("Choose a spell category for {}", src.name), spell_categories(store, node)),
        "blockspelldescriptor" if node.text().trim().is_empty() => mk(format!("Name the spell descriptor to block for {}", src.name), Vec::new()),
        "selectpowers" | "optionalpowers" | "addspirit" | "limitspiritcategory" => crate::items::magic::hooks::choice(ch, store, node, src),
        _ => None,
    }
}
