//! Item lists as trees, the way Chummer5a's tree views show them.
//!
//! The `.chum5` already nests items (gear in gear, ware in cyberlimbs, mods
//! and gear in armor, accessories on weapons, mods, mounts, weapons and gear
//! in vehicles). [`section_tree`] turns a [`Section`] into that tree plus
//! Chummer's group nodes (`Selected Gear` and the gear locations, `Positive
//! Qualities`, `Combat Spells`, `Cyberware` / `Bioware` and so on), in the
//! order the `RefreshXxx` methods of `CharacterShared.cs` build them.
//! [`flatten`] then lists the rows a tree table shows, given which nodes are
//! open.
//!
//! Nothing here is stored: the tree is rebuilt from the document each frame.

use crate::sections::Section;
use crate::xml::Element;

/// A tree node: an item or a group.
#[derive(Debug, Clone)]
pub struct Node<T> {
    /// Stable key for expansion state: the item guid, or a group key.
    pub key: String,
    pub value: T,
    pub children: Vec<Node<T>>,
}

impl<T> Node<T> {
    pub fn new(key: impl Into<String>, value: T) -> Self {
        Node { key: key.into(), value, children: Vec::new() }
    }

    /// This node and everything below it.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Node::count).sum::<usize>()
    }
}

/// One visible row of a flattened tree.
#[derive(Debug)]
pub struct Row<'a, T> {
    pub node: &'a Node<T>,
    pub depth: usize,
    /// Whether the node is open (only meaningful when it has children).
    pub open: bool,
    /// Whether this is its parent's last child (for └ guide lines).
    pub last: bool,
    /// For each ancestor level 0..depth, whether that level's line goes on
    /// below this row (the ancestor at that level has later siblings).
    pub guides: Vec<bool>,
}

/// The visible rows: depth-first, skipping the children of closed nodes.
pub fn flatten<'a, T>(roots: &'a [Node<T>], is_open: &dyn Fn(&Node<T>) -> bool) -> Vec<Row<'a, T>> {
    fn walk<'a, T>(nodes: &'a [Node<T>], depth: usize, guides: &mut Vec<bool>, is_open: &dyn Fn(&Node<T>) -> bool, out: &mut Vec<Row<'a, T>>) {
        for (i, n) in nodes.iter().enumerate() {
            let last = i + 1 == nodes.len();
            let open = !n.children.is_empty() && is_open(n);
            out.push(Row { node: n, depth, open, last, guides: guides.clone() });
            if open {
                guides.push(!last);
                walk(&n.children, depth + 1, guides, is_open, out);
                guides.pop();
            }
        }
    }
    let mut out = Vec::new();
    walk(roots, 0, &mut Vec::new(), is_open, &mut out);
    out
}

/// Text of a group node.
#[derive(Debug, Clone, PartialEq)]
pub enum Label {
    /// English UI text, to be translated (`Selected Gear`).
    Ui(&'static str),
    /// User or data text shown as it is (a location's name, a mod category).
    Text(String),
    /// An initiation or submersion grade (`Grade 2`).
    Grade(i32),
}

/// What a node of a section tree shows.
#[derive(Debug, Clone)]
pub enum Entry<'a> {
    Group(Label),
    /// An item. `top` is set for items that sit directly in the section's
    /// container (the ones that can be removed from the list).
    Item { el: &'a Element, top: bool },
}

pub type ItemNode<'a> = Node<Entry<'a>>;

/// Nested item containers of a saved element, by element name, in the order
/// Chummer's `CreateTreeNode` adds them. Vehicles are handled by
/// [`vehicle_children`].
pub fn children_of(tag: &str) -> &'static [(&'static str, &'static str)] {
    match tag {
        "gear" => &[("children", "gear")],
        "cyberware" => &[("children", "cyberware"), ("gears", "gear")],
        "armor" => &[("armormods", "armormod"), ("gears", "gear")],
        "armormod" => &[("gears", "gear")],
        "weapon" => &[("underbarrel", "weapon"), ("accessories", "accessory")],
        "accessory" => &[("gears", "gear")],
        "mod" => &[("cyberwares", "cyberware"), ("weapons", "weapon")],
        "weaponmount" => &[("mods", "mod"), ("weapons", "weapon")],
        "martialart" => &[("martialarttechniques", "martialarttechnique")],
        _ => &[],
    }
}

/// An item node with its nested items. `fallback` lists child containers
/// for kinds [`children_of`] does not know (a section's own
/// `child_containers`).
fn item<'a>(el: &'a Element, key: String, top: bool, fallback: &[(&'static str, &'static str)], depth: usize) -> ItemNode<'a> {
    let mut n = Node::new(key, Entry::Item { el, top });
    if depth > 12 {
        return n;
    }
    if el.name == "vehicle" {
        n.children = vehicle_children(el, &n.key, depth);
        return n;
    }
    let spec = children_of(&el.name);
    let spec = if spec.is_empty() { fallback } else { spec };
    for (container, tag) in spec {
        if let Some(c) = el.child(container) {
            for (i, child) in c.children_named(tag).enumerate() {
                let k = item_key(child, &n.key, container, i);
                n.children.push(item(child, k, false, fallback, depth + 1));
            }
        }
    }
    n
}

fn item_key(el: &Element, parent: &str, container: &str, i: usize) -> String {
    let g = el.get("guid");
    if g.is_empty() { format!("{parent}/{container}/{i}") } else { g }
}

fn group<'a>(key: String, label: Label) -> ItemNode<'a> {
    Node::new(key, Entry::Group(label))
}

/// Vehicle mod category groups, in `VehicleMod.s_CategoryGroupOrder`
/// (vehicles.xml `modcategories`, then General).
const MOD_CATEGORY_ORDER: &[&str] = &["Body", "Cosmetic", "Electromagnetic", "Model-Specific", "Powertrain", "Protection", "Weapons"];

/// A vehicle's children as `Vehicle.CreateTreeNode` adds them: its
/// locations, mods grouped by category (`GroupVehicleModsByCategory`, on by
/// default), the Weapon Mounts node, then weapons and gear, which go into
/// their location when they have one.
fn vehicle_children<'a>(v: &'a Element, key: &str, depth: usize) -> Vec<ItemNode<'a>> {
    let mut out: Vec<ItemNode<'a>> = Vec::new();
    let locations = locations(v.child("locations"));
    let n_locations = locations.len();
    for (guid, name) in &locations {
        out.push(group(format!("{key}/loc/{guid}"), Label::Text(name.clone())));
    }
    // Mods by category.
    let mut cats: Vec<(String, ItemNode<'a>)> = Vec::new();
    if let Some(mods) = v.child("mods") {
        for (i, m) in mods.children_named("mod").enumerate() {
            let cat = m.get("category");
            let cat = if cat.is_empty() || cat.eq_ignore_ascii_case("All") { "All".to_owned() } else { cat };
            let pos = match cats.iter().position(|(c, _)| c.eq_ignore_ascii_case(&cat)) {
                Some(p) => p,
                None => {
                    let label = if cat == "All" { Label::Ui("General") } else { Label::Text(cat.clone()) };
                    cats.push((cat.clone(), group(format!("{key}/modcat/{cat}"), label)));
                    cats.len() - 1
                }
            };
            let k = item_key(m, key, "mods", i);
            cats[pos].1.children.push(item(m, k, false, &[], depth + 1));
        }
    }
    let rank = |c: &str| MOD_CATEGORY_ORDER.iter().position(|o| o.eq_ignore_ascii_case(c)).unwrap_or(if c == "All" { MOD_CATEGORY_ORDER.len() } else { MOD_CATEGORY_ORDER.len() + 1 });
    cats.sort_by(|(a, _), (b, _)| rank(a).cmp(&rank(b)).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
    out.extend(cats.into_iter().map(|(_, n)| n));
    // Weapon mounts.
    if let Some(wm) = v.child("weaponmounts") {
        let mut mounts = group(format!("{key}/weaponmounts"), Label::Ui("Weapon Mounts"));
        for (i, m) in wm.children_named("weaponmount").enumerate() {
            let k = item_key(m, key, "weaponmounts", i);
            mounts.children.push(item(m, k, false, &[], depth + 1));
        }
        if !mounts.children.is_empty() {
            out.push(mounts);
        }
    }
    // Weapons and gear, in their location if they have one.
    for (container, tag) in [("weapons", "weapon"), ("gears", "gear")] {
        let Some(c) = v.child(container) else { continue };
        for (i, child) in c.children_named(tag).enumerate() {
            let k = item_key(child, key, container, i);
            let n = item(child, k, false, &[], depth + 1);
            match location_index(&locations, &child.get("location")) {
                Some(l) if l < n_locations => out[l].children.push(n),
                _ => out.push(n),
            }
        }
    }
    out
}

/// `(guid, name)` of the `<location>` elements in a locations container.
fn locations(c: Option<&Element>) -> Vec<(String, String)> {
    c.map(|c| c.children_named("location").map(|l| (l.get("guid"), l.get("name"))).collect()).unwrap_or_default()
}

/// The location an item's `<location>` refers to: by guid, or by name for
/// files that stored the name.
fn location_index(locations: &[(String, String)], loc: &str) -> Option<usize> {
    let loc = loc.trim();
    if loc.is_empty() {
        return None;
    }
    locations.iter().position(|(g, _)| g.eq_ignore_ascii_case(loc)).or_else(|| locations.iter().position(|(_, n)| n == loc))
}

/// Roots made of fixed groups, in display order: items go to the group
/// `pick` names (or to the top level for `None`); empty groups are left out.
fn grouped<'a>(sec: &Section, items: Vec<&'a Element>, groups: &[&'static str], pick: impl Fn(&Element) -> Option<usize>) -> Vec<ItemNode<'a>> {
    let mut roots: Vec<ItemNode<'a>> = groups.iter().map(|g| group(format!("{}/{g}", sec.container), Label::Ui(g))).collect();
    let mut loose = Vec::new();
    for (i, el) in items.into_iter().enumerate() {
        let n = item(el, item_key(el, sec.container, sec.item, i), true, sec.child_containers, 0);
        match pick(el) {
            Some(g) if g < roots.len() => roots[g].children.push(n),
            _ => loose.push(n),
        }
    }
    roots.retain(|r| !r.children.is_empty());
    roots.extend(loose);
    roots
}

/// Gear, armor, weapons and vehicles: the "Selected X" root for items
/// without a location (Chummer inserts it at index 0), then one node per
/// location, shown even when empty.
fn with_locations<'a>(doc: &'a Element, sec: &Section, items: Vec<&'a Element>, root: &'static str, locations_tag: &str) -> Vec<ItemNode<'a>> {
    let locs = locations(doc.child(locations_tag));
    let mut selected = group(format!("{}/{root}", sec.container), Label::Ui(root));
    let mut loc_nodes: Vec<ItemNode<'a>> = locs.iter().map(|(g, name)| group(format!("{}/loc/{g}", sec.container), Label::Text(name.clone()))).collect();
    for (i, el) in items.into_iter().enumerate() {
        let n = item(el, item_key(el, sec.container, sec.item, i), true, sec.child_containers, 0);
        match location_index(&locs, &el.get("location")) {
            Some(l) => loc_nodes[l].children.push(n),
            None => selected.children.push(n),
        }
    }
    let mut roots = Vec::new();
    if !selected.children.is_empty() {
        roots.push(selected);
    }
    roots.extend(loc_nodes);
    roots
}

const ESSENCE_HOLE: &str = "b57eadaa-7c3b-4b80-8d79-cbbd922c1196";
const ESSENCE_ANTIHOLE: &str = "961eac53-0c43-4b19-8741-2872177a3a4c";

/// The tree of a section, from the character document (`Character::doc`).
pub fn section_tree<'a>(doc: &'a Element, sec: &Section) -> Vec<ItemNode<'a>> {
    let items: Vec<&'a Element> = doc.child(sec.container).map(|c| c.children_named(sec.item).collect()).unwrap_or_default();
    match sec.container {
        "gears" => with_locations(doc, sec, items, "Selected Gear", "gearlocations"),
        "armors" => with_locations(doc, sec, items, "Selected Armor", "armorlocations"),
        "weapons" => with_locations(doc, sec, items, "Selected Weapons", "weaponlocations"),
        "vehicles" => with_locations(doc, sec, items, "Selected Vehicles", "vehiclelocations"),
        "qualities" => grouped(sec, items, &["Positive Qualities", "Negative Qualities", "Life Modules"], |q| match q.get("qualitytype").as_str() {
            "Positive" => Some(0),
            "Negative" => Some(1),
            "LifeModule" => Some(2),
            _ => None,
        }),
        // RefreshCyberware: modular ware that is not plugged in gets its own
        // roots; the Essence Hole and Antihole stand alone.
        "cyberwares" => {
            let groups = ["Cyberware", "Bioware", "Unequipped Modular Cyberware", "Unequipped Modular Bioware"];
            grouped(sec, items, &groups, |w| {
                let id = w.get("sourceid");
                if id.eq_ignore_ascii_case(ESSENCE_HOLE) || id.eq_ignore_ascii_case(ESSENCE_ANTIHOLE) {
                    return None;
                }
                let bio = crate::items::cyberware::is_bioware(w);
                let equipped = w.get("plugsintomodularmount").trim().is_empty();
                Some(usize::from(bio) + if equipped { 0 } else { 2 })
            })
        }
        // RefreshSpells: one root per category; others stay at the top.
        "spells" => {
            const CATS: [&str; 7] = ["COMBAT", "DETECTION", "HEALTH", "ILLUSION", "MANIPULATION", "RITUALS", "ENCHANTMENTS"];
            let groups = ["Combat Spells", "Detection Spells", "Health Spells", "Illusion Spells", "Manipulation Spells", "Rituals", "Enchantments"];
            grouped(sec, items, &groups, |s| CATS.iter().position(|c| s.get("category").eq_ignore_ascii_case(c)))
        }
        "critterpowers" => grouped(sec, items, &["Critter Powers", "Weaknesses"], |p| Some(usize::from(p.get("category").eq_ignore_ascii_case("Weakness")))),
        "martialarts" => grouped(sec, items, &["Martial Arts", "Selected Qualities"], |m| Some(usize::from(m.get_bool("isquality").unwrap_or(false)))),
        "lifestyles" => grouped(sec, items, &["Selected Lifestyles"], |_| Some(0)),
        // RefreshComplexForms puts every form under this root.
        "complexforms" => grouped(sec, items, &["Selected Advanced Complex Forms"], |_| Some(0)),
        // Chummer keeps contacts and enemies on separate tabs.
        "contacts" => grouped(sec, items, &["Contacts", "Enemies", "Pets"], |c| match c.get("type").as_str() {
            "Enemy" => Some(1),
            "Pet" => Some(2),
            _ => Some(0),
        }),
        // RefreshInitiationGrades: metamagics and echoes under their grade.
        "metamagics" => {
            let mut grades: Vec<i32> = items.iter().map(|m| m.get_i32("grade").unwrap_or(0)).collect();
            grades.sort_unstable();
            grades.dedup();
            let mut roots: Vec<ItemNode<'a>> = grades.iter().map(|g| group(format!("metamagics/grade/{g}"), Label::Grade(*g))).collect();
            for (i, el) in items.into_iter().enumerate() {
                let g = el.get_i32("grade").unwrap_or(0);
                let n = item(el, item_key(el, sec.container, sec.item, i), true, sec.child_containers, 0);
                if let Ok(p) = grades.binary_search(&g) {
                    roots[p].children.push(n);
                }
            }
            roots
        }
        _ => items.into_iter().enumerate().map(|(i, el)| item(el, item_key(el, sec.container, sec.item, i), true, sec.child_containers, 0)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sections;
    use crate::xml::parse;
    use std::collections::HashSet;

    fn names(rows: &[Row<Entry>]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                let t = match &r.node.value {
                    Entry::Group(Label::Ui(s)) => format!("[{s}]"),
                    Entry::Group(Label::Text(s)) => format!("[{s}]"),
                    Entry::Group(Label::Grade(g)) => format!("[Grade {g}]"),
                    Entry::Item { el, .. } => el.get("name"),
                };
                format!("{}{t}", "  ".repeat(r.depth))
            })
            .collect()
    }

    const DOC: &str = r#"<character>
      <gears>
        <gear><guid>g1</guid><name>Commlink</name><location></location>
          <children><gear><guid>g2</guid><name>Sim Module</name><children><gear><guid>g3</guid><name>Hot-Sim</name></gear></children></gear></children>
        </gear>
        <gear><guid>g4</guid><name>Rope</name><location>L1</location></gear>
        <gear><guid>g5</guid><name>Ammo</name><location>Bag</location></gear>
      </gears>
      <gearlocations>
        <location><guid>L1</guid><name>Car</name></location>
        <location><guid>L2</guid><name>Bag</name></location>
        <location><guid>L3</guid><name>Empty</name></location>
      </gearlocations>
      <qualities>
        <quality><guid>q1</guid><name>Allergy</name><qualitytype>Negative</qualitytype></quality>
        <quality><guid>q2</guid><name>Ambidextrous</name><qualitytype>Positive</qualitytype></quality>
      </qualities>
      <cyberwares>
        <cyberware><guid>c1</guid><name>Cyberarm</name><improvementsource>Cyberware</improvementsource>
          <children><cyberware><guid>c2</guid><name>Gyromount</name></cyberware></children>
          <gears><gear><guid>c3</guid><name>Spare Clip</name></gear></gears>
        </cyberware>
        <cyberware><guid>c4</guid><name>Muscle Toner</name><improvementsource>Bioware</improvementsource></cyberware>
        <cyberware><guid>c5</guid><name>Cyber Hand</name><improvementsource>Cyberware</improvementsource><plugsintomodularmount>wrist</plugsintomodularmount></cyberware>
        <cyberware><guid>c6</guid><name>Essence Hole</name><sourceid>B57EADAA-7C3B-4B80-8D79-CBBD922C1196</sourceid><improvementsource>Cyberware</improvementsource></cyberware>
      </cyberwares>
      <vehicles>
        <vehicle><guid>v1</guid><name>Bulldog</name>
          <mods>
            <mod><guid>m1</guid><name>Gecko Tips</name><category>All</category></mod>
            <mod><guid>m2</guid><name>Armor</name><category>Protection</category></mod>
            <mod><guid>m3</guid><name>Rigger Cocoon</name><category>Body</category>
              <weapons><weapon><guid>w0</guid><name>Taser</name></weapon></weapons></mod>
          </mods>
          <weaponmounts><weaponmount><guid>wm1</guid><name>Standard</name>
            <weapons><weapon><guid>w1</guid><name>LMG</name>
              <accessories><accessory><guid>a1</guid><name>Smartgun</name><gears><gear><guid>a2</guid><name>Ammo</name></gear></gears></accessory></accessories>
            </weapon></weapons></weaponmount></weaponmounts>
          <weapons><weapon><guid>w2</guid><name>Pistol</name><location>VL1</location></weapon></weapons>
          <gears><gear><guid>vg1</guid><name>Toolkit</name></gear></gears>
          <locations><location><guid>VL1</guid><name>Trunk</name></location></locations>
        </vehicle>
      </vehicles>
      <weapons>
        <weapon><guid>w3</guid><name>Rifle</name>
          <accessories><accessory><guid>a3</guid><name>Scope</name></accessory></accessories>
          <underbarrel><weapon><guid>w4</guid><name>Grenade Launcher</name></weapon></underbarrel>
        </weapon>
      </weapons>
    </character>"#;

    fn all_open(_: &ItemNode) -> bool {
        true
    }

    #[test]
    fn gear_goes_under_selected_gear_and_locations() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::GEAR);
        let rows = flatten(&tree, &all_open);
        assert_eq!(names(&rows), ["[Selected Gear]", "  Commlink", "    Sim Module", "      Hot-Sim", "[Car]", "  Rope", "[Bag]", "  Ammo", "[Empty]"]);
        // Only items directly in <gears> can be removed from the list.
        let tops: Vec<bool> = rows.iter().filter_map(|r| if let Entry::Item { top, .. } = r.node.value { Some(top) } else { None }).collect();
        assert_eq!(tops, [true, false, false, true, true]);
    }

    #[test]
    fn collapsed_nodes_hide_their_children() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::GEAR);
        let closed: HashSet<&str> = ["g2", "gears/loc/L1"].into();
        let rows = flatten(&tree, &|n: &ItemNode| !closed.contains(n.key.as_str()));
        assert_eq!(names(&rows), ["[Selected Gear]", "  Commlink", "    Sim Module", "[Car]", "[Bag]", "  Ammo", "[Empty]"]);
        assert!(!rows[2].open);
        // A closed leaf is still a leaf; an empty group is not "open".
        assert!(!rows[6].open);
    }

    #[test]
    fn guide_lines_follow_siblings() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::GEAR);
        let rows = flatten(&tree, &all_open);
        // Hot-Sim: Selected Gear has later siblings, Commlink and Sim Module do not.
        assert_eq!(rows[3].guides, [true, false, false]);
        assert!(rows[3].last);
        assert!(!rows[0].last);
        assert!(rows[8].last);
    }

    #[test]
    fn qualities_split_positive_negative() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::QUALITIES);
        let rows = flatten(&tree, &all_open);
        assert_eq!(names(&rows), ["[Positive Qualities]", "  Ambidextrous", "[Negative Qualities]", "  Allergy"]);
    }

    #[test]
    fn cyberware_roots_and_nesting() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::CYBERWARE);
        let rows = flatten(&tree, &all_open);
        assert_eq!(
            names(&rows),
            ["[Cyberware]", "  Cyberarm", "    Gyromount", "    Spare Clip", "[Bioware]", "  Muscle Toner", "[Unequipped Modular Cyberware]", "  Cyber Hand", "Essence Hole"]
        );
    }

    #[test]
    fn vehicle_children_in_chummer_order() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::VEHICLES);
        let rows = flatten(&tree, &all_open);
        assert_eq!(
            names(&rows),
            [
                "[Selected Vehicles]",
                "  Bulldog",
                "    [Trunk]",
                "      Pistol",
                "    [Body]",
                "      Rigger Cocoon",
                "        Taser",
                "    [Protection]",
                "      Armor",
                "    [General]",
                "      Gecko Tips",
                "    [Weapon Mounts]",
                "      Standard",
                "        LMG",
                "          Smartgun",
                "            Ammo",
                "    Toolkit",
            ]
        );
    }

    #[test]
    fn underbarrel_before_accessories() {
        let doc = parse(DOC).unwrap();
        let tree = section_tree(&doc, &sections::WEAPONS);
        let rows = flatten(&tree, &all_open);
        assert_eq!(names(&rows), ["[Selected Weapons]", "  Rifle", "    Grenade Launcher", "    Scope"]);
    }

    #[test]
    fn unknown_sections_stay_flat_with_their_child_containers() {
        let doc = parse("<character><things><thing><name>A</name><subs><sub><name>B</name></sub></subs></thing></things></character>").unwrap();
        let sec = Section { label: "Things", container: "things", item: "thing", columns: &[], child_containers: &[("subs", "sub")], data_file: "" };
        let tree = section_tree(&doc, &sec);
        let rows = flatten(&tree, &all_open);
        assert_eq!(names(&rows), ["A", "  B"]);
        // Items without a guid get a positional key.
        assert_eq!(tree[0].key, "things/thing/0");
        assert_eq!(tree[0].children[0].key, "things/thing/0/subs/0");
    }
}
