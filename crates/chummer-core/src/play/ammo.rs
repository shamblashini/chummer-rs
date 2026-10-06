//! Weapon ammunition at the table: `Weapon.Clips`, `AmmoRemaining`,
//! `Reload` / `Unload` and the fire buttons of `CharacterCareer`
//! (`cmsAmmoSingleShot_Click` …).
//!
//! As in Chummer 5.222.61 and later, the rounds in a clip are a gear item
//! in the inventory: `<clip><id>` is that gear's guid and its quantity is
//! the clip's `<count>`. Reloading splits the rounds off the stack they
//! come from; firing lowers both and deletes the gear once it is empty.
//! Weapons with `<requireammo>False</requireammo>` have one internal clip
//! of charges and no gear.
//!
//! Chummer saves only the clips that hold something and reassigns them to
//! slots in order on load. This port writes every slot up to the last
//! loaded one (empty slots with the empty guid, which Chummer loads as
//! empty clips), so a loaded second magazine stays the second magazine.

use crate::character::Character;
use crate::data::DataStore;
use crate::improvement::fmt_num;
use crate::xml::{Element, Node};

/// `Utils.GuidEmptyString`: a clip without ammunition gear.
pub const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// One magazine of a weapon (`Clip`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    /// Rounds (or charges) in it.
    pub count: i32,
    /// Guid of the loaded ammunition gear; `None` when empty, for the
    /// internal clip and for an external source.
    pub ammo: Option<String>,
    /// `Clip.AmmoLocation` (always `loaded`).
    pub location: String,
    /// Name of the accessory that provides this slot, if any.
    pub owner: Option<String>,
}

impl Clip {
    fn empty() -> Clip {
        Clip { location: "loaded".into(), ..Default::default() }
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0 && self.ammo.is_none()
    }
}

/// The fire buttons (`cmsAmmo…_Click`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FireMode {
    SingleShot,
    ShortBurst,
    LongBurst,
    FullBurst,
    Suppressive,
}

impl FireMode {
    pub const ALL: [FireMode; 5] = [FireMode::SingleShot, FireMode::ShortBurst, FireMode::LongBurst, FireMode::FullBurst, FireMode::Suppressive];

    /// Saved element with the rounds used (`<singleshot>` …).
    pub fn field(self) -> &'static str {
        match self {
            FireMode::SingleShot => "singleshot",
            FireMode::ShortBurst => "shortburst",
            FireMode::LongBurst => "longburst",
            FireMode::FullBurst => "fullburst",
            FireMode::Suppressive => "suppressive",
        }
    }

    fn default_rounds(self) -> i32 {
        match self {
            FireMode::SingleShot => 1,
            FireMode::ShortBurst => 3,
            FireMode::LongBurst => 6,
            FireMode::FullBurst => 10,
            FireMode::Suppressive => 20,
        }
    }

    /// The en-us `String_SingleShot` … text: `"Single Shot ({0} {1})"`.
    pub fn label(self) -> &'static str {
        match self {
            FireMode::SingleShot => "Single Shot ({0} {1})",
            FireMode::ShortBurst => "Short Burst ({0} {1})",
            FireMode::LongBurst => "Long Burst ({0} {1})",
            FireMode::FullBurst => "Full Burst ({0} {1})",
            FireMode::Suppressive => "Suppressive Fire ({0} {1})",
        }
    }
}

/// What a fire button did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fired {
    /// The rounds were used.
    Fired(i32),
    /// `Message_OutOfAmmo`: nothing happened.
    OutOfAmmo,
    /// Not enough rounds; Chummer asks this (en-us text) and, on Yes,
    /// empties the clip ([`set_remaining`] with 0).
    Confirm(&'static str),
    /// Not enough rounds for this mode (en-us text); nothing happened.
    Cannot(&'static str),
}

pub const OUT_OF_AMMO: &str = "Out of Ammunition!";
const TREAT_SINGLE: &str = "Not enough Ammunition. Treat as Single Shot?";
const TREAT_SHORT_BURST_SHORT: &str = "Not enough Ammunition. Treat as shortened Short Burst?";
const TREAT_SHORT_BURST: &str = "Not enough Ammunition. Treat as Short Burst?";
const TREAT_LONG_BURST_SHORT: &str = "Not enough Ammunition. Treat as shortened Long Burst?";
const NO_FULL_BURST: &str = "Not enough Ammunition. Cannot fire Full Burst.";
const NO_SUPPRESSIVE: &str = "Not enough Ammunition. Cannot fire Suppressive Fire.";

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn weapon<'a>(ch: &'a Character, guid: &str) -> Option<&'a Element> {
    super::find(&ch.doc, guid).filter(|e| e.name == "weapon")
}

fn weapon_mut<'a>(ch: &'a mut Character, guid: &str) -> Option<&'a mut Element> {
    super::find_mut(&mut ch.doc, guid).filter(|e| e.name == "weapon")
}

/// `Weapon.RequireAmmo`.
pub fn requires_ammo(w: &Element) -> bool {
    w.get_bool("requireammo").unwrap_or(true)
}

/// Whether the weapon holds ammunition or charges at all (`Ammo` is set
/// and not `0`).
pub fn uses_ammo(w: &Element) -> bool {
    let a = w.get("ammo");
    !a.trim().is_empty() && a.trim() != "0"
}

/// `GetClipProvidingAccessories`: accessories, once per ammo slot each.
fn clip_accessories(w: &Element) -> Vec<String> {
    let mut out = Vec::new();
    for a in w.child("accessories").into_iter().flat_map(|c| c.children_named("accessory")) {
        for _ in 0..a.get_i32("ammoslots").unwrap_or(0).max(0) {
            out.push(a.get("name"));
        }
    }
    out
}

/// The weapon's clips, one per slot (`CreateClips` filled by
/// `AddClipNodes`): `ammoslots` of its own, then the accessories' slots.
/// A weapon without `requireammo` has its one internal clip.
pub fn clips(w: &Element) -> Vec<Clip> {
    let saved: Vec<Clip> = w
        .child("clips")
        .into_iter()
        .flat_map(|c| c.children_named("clip"))
        .filter(|c| c.child("id").is_some() && c.child("count").is_some())
        .map(|c| {
            let id = c.get("id");
            let ammo = (!id.trim().is_empty() && !id.starts_with("00000000")).then_some(id);
            let location = c.child_text("location").unwrap_or_else(|| "loaded".into());
            Clip { count: c.get_i32("count").unwrap_or(0), ammo, location, owner: None }
        })
        .collect();
    if !requires_ammo(w) {
        let mut c = saved.into_iter().next().unwrap_or_else(Clip::empty);
        c.ammo = None;
        return vec![c];
    }
    let own = w.get_i32("ammoslots").unwrap_or(1).max(0) as usize;
    let mut owners: Vec<Option<String>> = vec![None; own];
    owners.extend(clip_accessories(w).into_iter().map(Some));
    let n = owners.len().max(saved.len());
    let mut out: Vec<Clip> = saved;
    out.resize_with(n, Clip::empty);
    for (c, o) in out.iter_mut().zip(owners) {
        c.owner = o;
    }
    out
}

/// `ActiveAmmoSlot` (1-based), within the weapon's slots.
pub fn active_slot(w: &Element) -> usize {
    let n = clips(w).len().max(1);
    (w.get_i32("activeammoslot").unwrap_or(1).max(1) as usize).min(n)
}

/// `AmmoRemaining`: rounds in the active clip.
pub fn remaining(w: &Element) -> i32 {
    clips(w).get(active_slot(w) - 1).map_or(0, |c| c.count)
}

/// `AmmoLoaded`: the gear in the active clip.
pub fn loaded<'a>(ch: &'a Character, w: &Element) -> Option<&'a Element> {
    let id = clips(w).get(active_slot(w) - 1)?.ammo.clone()?;
    super::find(&ch.doc, &id).filter(|g| g.name == "gear")
}

/// `SingleShot` … : the weapon's rounds per mode, or an equipped
/// accessory's when larger.
pub fn rounds(w: &Element, mode: FireMode) -> i32 {
    let mut v = w.get_i32(mode.field()).unwrap_or(mode.default_rounds());
    for a in w.child("accessories").into_iter().flat_map(|c| c.children_named("accessory")) {
        if a.get_bool("equipped").unwrap_or(true) {
            v = v.max(a.get_i32(mode.field()).unwrap_or(0));
        }
    }
    v
}

/// `AllowSingleShot` …: the weapon allows the mode (`<allow…>`) and its
/// firing modes include one that can fire it.
pub fn allows(ch: &Character, w: &Element, mode: FireMode) -> bool {
    let allow = |k: &str| w.get_bool(k).unwrap_or(true);
    let modes = crate::print::items::mode_codes(&ch.doc, w, true);
    let any = |m: &[&str]| modes.iter().any(|x| m.contains(x));
    match mode {
        FireMode::SingleShot => (w.get("type") == "Melee" && w.get("ammo") != "0") || (allow("allowsingleshot") && any(&["SS", "SA"])),
        FireMode::ShortBurst => allow("allowshortburst") && any(&["BF", "SA", "FA"]),
        FireMode::LongBurst => allow("allowlongburst") && any(&["BF", "FA"]),
        FireMode::FullBurst => allow("allowfullburst") && any(&["FA"]),
        FireMode::Suppressive => allow("allowsuppressive") && any(&["FA"]),
    }
}

/// The weapon mount a vehicle weapon sits in, for ammo bonuses.
fn mount_of<'a>(doc: &'a Element, guid: &str) -> Option<&'a Element> {
    fn walk<'a>(e: &'a Element, guid: &str, mount: Option<&'a Element>) -> Option<&'a Element> {
        let here = if e.name == "weaponmount" { Some(e) } else { mount };
        for c in e.elements().filter(|c| c.name != "improvements") {
            if c.name == "weapon" && c.get("guid").eq_ignore_ascii_case(guid) {
                return here;
            }
            if let Some(m) = walk(c, guid, here) {
                return Some(m);
            }
        }
        None
    }
    walk(doc, guid, None)
}

/// The reload choices of `Weapon.Reload` (`lstCount`): the capacity of
/// each way to load the weapon, plus `External Source` when it has one.
pub fn reload_counts(ch: &Character, w: &Element) -> Vec<String> {
    let ammo = crate::print::items::ammo_entries(w, mount_of(&ch.doc, &w.get("guid"))).join(" ");
    let lower = ammo.to_ascii_lowercase();
    let mut out = Vec::new();
    let external = lower.contains("external source");
    if ammo.contains(['×', 'x', '+']) || lower.contains(" or ") || lower.contains("special") || external {
        let mut s = replace_ci(&ammo, "External Source", "").to_lowercase();
        if let Some(p) = s.rfind(" + energy") {
            s.replace_range(p..p + " + energy".len(), "");
        }
        let s = s.replace(" or belt", " or 250(belt)");
        for part in s.split(" or ").filter(|p| !p.is_empty()) {
            out.push(crate::print::items::ammo_capacity(part));
        }
    } else {
        out.push(ammo.split('(').next().unwrap_or("").to_owned());
    }
    if external && requires_ammo(w) {
        out.push("External Source".into());
    }
    out
}

/// The largest load: the first count that is a number.
pub fn capacity(ch: &Character, w: &Element) -> i32 {
    reload_counts(ch, w).iter().find_map(|c| c.trim().parse::<i32>().ok()).unwrap_or(0)
}

fn replace_ci(s: &str, from: &str, to: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find(&needle) {
        out.push_str(&s[i..i + p]);
        out.push_str(to);
        i += p + needle.len();
    }
    out.push_str(&s[i..]);
    out
}

// ---------------------------------------------------------------------------
// Reloadable ammunition
// ---------------------------------------------------------------------------

/// The vehicle a weapon is on (its gear is the ammunition pool).
fn vehicle_of(doc: &Element, guid: &str) -> Option<String> {
    fn walk(e: &Element, guid: &str, veh: Option<&Element>) -> Option<Option<String>> {
        let here = if e.name == "vehicle" { Some(e) } else { veh };
        for c in e.elements().filter(|c| c.name != "improvements") {
            if c.get("guid").eq_ignore_ascii_case(guid) {
                return Some(here.map(|v| v.get("guid")));
            }
            if let Some(r) = walk(c, guid, here) {
                return Some(r);
            }
        }
        None
    }
    walk(doc, guid, None).flatten()
}

/// The gear list ammunition comes from: the vehicle's for a vehicle
/// weapon, else the character's.
fn pool<'a>(ch: &'a Character, weapon_guid: &str) -> Option<&'a Element> {
    match vehicle_of(&ch.doc, weapon_guid) {
        Some(v) => super::find(&ch.doc, &v)?.child("gears"),
        None => ch.doc.child("gears"),
    }
}

fn pool_mut<'a>(ch: &'a mut Character, weapon_guid: &str) -> &'a mut Element {
    match vehicle_of(&ch.doc, weapon_guid) {
        Some(v) => super::find_mut(&mut ch.doc, &v).expect("vehicle").child_or_insert("gears"),
        None => ch.doc.child_or_insert("gears"),
    }
}

/// Guids of every gear loaded in some clip.
fn loaded_ids(doc: &Element) -> Vec<String> {
    let mut clips = Vec::new();
    doc.descendants("clip", &mut clips);
    clips.iter().map(|c| c.get("id").to_ascii_lowercase()).filter(|id| !id.starts_with("00000000")).collect()
}

/// A gear field, saved or (for old saves) from gear.xml.
fn gear_field(store: Option<&DataStore>, g: &Element, field: &str) -> String {
    if let Some(v) = g.child_text(field).filter(|s| !s.is_empty()) {
        return v;
    }
    let Some(doc) = store.and_then(|s| s.doc("gear.xml").ok()) else { return String::new() };
    let id = g.get("sourceid");
    let rec = (!id.is_empty()).then(|| crate::data::find(&doc, "gears", "gear", &id)).flatten().or_else(|| crate::data::find(&doc, "gears", "gear", &g.get("name")));
    rec.and_then(|r| r.el().child_text(field)).unwrap_or_default()
}

/// `Weapon.AmmoCategory`: the saved one, else the weapon's category.
fn ammo_category(w: &Element) -> String {
    let a = w.get("ammocategory");
    if a.is_empty() { w.get("category") } else { a }
}

/// `Weapon.WeaponType`: saved, else the category's type in weapons.xml.
fn weapon_type(store: Option<&DataStore>, w: &Element) -> String {
    if let Some(t) = w.child_text("weapontype").filter(|t| !t.is_empty()) {
        return t;
    }
    let cat = w.get("category");
    store
        .and_then(|s| s.doc("weapons.xml").ok())
        .and_then(|d| d.child("categories").and_then(|c| c.children_named("category").find(|c| c.text() == cat).and_then(|c| c.attr("type").map(str::to_owned))))
        .unwrap_or_else(|| cat.to_lowercase())
}

fn qty(g: &Element) -> f64 {
    g.get_f64("qty").unwrap_or(1.0)
}

/// `Weapon.GetAmmoReloadable`: equipped, unloaded ammunition the
/// character (or vehicle) carries that fits the weapon, recursing into
/// equipped gear. Returns (guid, name, quantity).
pub fn reloadable(ch: &Character, store: Option<&DataStore>, weapon_guid: &str) -> Vec<(String, String, f64)> {
    let Some(w) = weapon(ch, weapon_guid) else { return Vec::new() };
    if !requires_ammo(w) {
        return Vec::new();
    }
    let Some(pool) = pool(ch, weapon_guid) else { return Vec::new() };
    let loaded = loaded_ids(&ch.doc);
    let cat = ammo_category(w);
    let wtype = weapon_type(store, w);
    let flechette = w.get("damage").contains("(f)");
    let throwing = w.get("useskill") == "Throwing Weapons" || w.get("category") == "Throwing Weapons";
    let mut out = Vec::new();
    let mut stack: Vec<&Element> = pool.children_named("gear").collect();
    stack.reverse();
    while let Some(g) = stack.pop() {
        if !g.get_bool("equipped").unwrap_or(true) {
            continue;
        }
        let extra = g.get("extra");
        let for_type = gear_field(store, g, "ammoforweapontype").split(',').map(str::trim).any(|t| t == wtype);
        let is_flechette = crate::xml::parse_bool(&gear_field(store, g, "isflechetteammo"));
        let fits = qty(g) > 0.0
            && !loaded.contains(&g.get("guid").to_ascii_lowercase())
            && if cat == "Gear" {
                g.get("name") == w.get("name") && (extra.is_empty() || extra == cat)
            } else if throwing {
                (!flechette || is_flechette) && for_type && (extra.is_empty() || extra == cat || g.get("name") == w.get("name"))
            } else {
                (!flechette || is_flechette) && for_type && (extra.is_empty() || extra == cat)
            };
        if fits {
            out.push((g.get("guid"), g.get("name"), qty(g)));
        }
        let kids: Vec<&Element> = g.child("children").into_iter().flat_map(|c| c.children_named("gear")).collect();
        stack.extend(kids.into_iter().rev());
    }
    out
}

// ---------------------------------------------------------------------------
// Changing
// ---------------------------------------------------------------------------

/// Write the clips back, in Chummer's place right after `<activeammoslot>`.
fn write_clips(w: &mut Element, clips: &[Clip]) {
    let last = clips.iter().rposition(|c| !c.is_empty());
    let pos = w.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "clips"));
    let Some(last) = last else {
        // Chummer writes no <clips> when every clip is empty.
        if let Some(p) = pos {
            w.children.remove(p);
        }
        return;
    };
    let mut el = Element::new("clips");
    for c in &clips[..=last] {
        let mut e = Element::new("clip");
        e.push(Element::with_text("count", c.count.to_string()));
        e.push(Element::with_text("location", if c.location.is_empty() { "loaded" } else { &c.location }));
        e.push(Element::with_text("id", c.ammo.clone().unwrap_or_else(|| EMPTY_GUID.into())));
        el.push(e);
    }
    match pos {
        Some(p) => w.children[p] = Node::Element(el),
        None => {
            let at = w.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "activeammoslot")).map_or(w.children.len(), |p| p + 1);
            w.children.insert(at, Node::Element(el));
        }
    }
}

fn update_clips(ch: &mut Character, guid: &str, f: impl FnOnce(&mut Vec<Clip>, usize)) -> bool {
    let Some(w) = weapon_mut(ch, guid) else { return false };
    let mut cs = clips(w);
    if cs.is_empty() {
        return false;
    }
    let slot = active_slot(w);
    f(&mut cs, slot - 1);
    write_clips(w, &cs);
    ch.dirty = true;
    true
}

fn set_gear_qty(ch: &mut Character, guid: &str, q: f64) {
    if let Some(g) = super::find_mut(&mut ch.doc, guid) {
        g.set_child_text("qty", fmt_num(q));
    }
}

/// `ActiveAmmoSlot`: switch to another clip.
pub fn set_active_slot(ch: &mut Character, guid: &str, slot: usize) -> bool {
    let Some(w) = weapon_mut(ch, guid) else { return false };
    let n = clips(w).len().max(1);
    let s = slot.clamp(1, n);
    if active_slot(w) == s && w.child("activeammoslot").is_some() {
        return false;
    }
    w.set_child_text("activeammoslot", s.to_string());
    ch.dirty = true;
    true
}

/// `Weapon.SetAmmoRemaining`: set the active clip's rounds; the loaded
/// gear's quantity follows, and the gear is deleted once it is used up.
pub fn set_remaining(ch: &mut Character, guid: &str, value: i32) -> bool {
    let Some(w) = weapon(ch, guid) else { return false };
    let Some(clip) = clips(w).get(active_slot(w) - 1).cloned() else { return false };
    let value = value.max(0);
    if clip.count == value {
        return false;
    }
    let gear = clip.ammo.as_deref().and_then(|id| super::find(&ch.doc, id)).map(|g| (g.get("guid"), qty(g)));
    let mut emptied = false;
    if let Some((gid, q)) = &gear {
        let new_q = q + f64::from(value - clip.count);
        if new_q > 0.0 {
            set_gear_qty(ch, gid, new_q);
        } else {
            ch.remove_item_anywhere(gid);
            emptied = true;
        }
    }
    update_clips(ch, guid, |cs, i| {
        if emptied {
            cs[i].ammo = None;
            cs[i].count = 0;
        } else {
            cs[i].count = value;
        }
    })
}

/// A fire button: use the mode's rounds from the active clip, following
/// `DoSingleShot`, `DoShortBurst`, `DoLongBurst` and the full burst and
/// suppressive fire handlers.
pub fn fire(ch: &mut Character, guid: &str, mode: FireMode) -> Fired {
    let Some(w) = weapon(ch, guid) else { return Fired::OutOfAmmo };
    let left = remaining(w);
    let need = rounds(w, mode);
    let (ss, sb) = (rounds(w, FireMode::SingleShot), rounds(w, FireMode::ShortBurst));
    if (mode == FireMode::SingleShot && left < need) || left == 0 {
        return Fired::OutOfAmmo;
    }
    if left >= need {
        set_remaining(ch, guid, left - need);
        return Fired::Fired(need);
    }
    match mode {
        FireMode::SingleShot => Fired::OutOfAmmo,
        FireMode::ShortBurst if left == ss => Fired::Confirm(TREAT_SINGLE),
        FireMode::ShortBurst => Fired::Confirm(TREAT_SHORT_BURST_SHORT),
        FireMode::LongBurst if left == ss => Fired::Confirm(TREAT_SINGLE),
        FireMode::LongBurst if left > sb => Fired::Confirm(TREAT_LONG_BURST_SHORT),
        FireMode::LongBurst if left == sb => Fired::Confirm(TREAT_SHORT_BURST),
        FireMode::LongBurst => Fired::Confirm(TREAT_SHORT_BURST_SHORT),
        FireMode::FullBurst => Fired::Cannot(NO_FULL_BURST),
        FireMode::Suppressive => Fired::Cannot(NO_SUPPRESSIVE),
    }
}

/// `Gear.IsIdenticalToOtherGear`.
fn identical(a: &Element, b: &Element, ignore_superficials: bool) -> bool {
    let same = |k: &str| a.get(k) == b.get(k);
    let kids = |e: &Element| -> Vec<Element> { e.child("children").map(|c| c.children_named("gear").cloned().collect()).unwrap_or_default() };
    let (ka, kb) = (kids(a), kids(b));
    same("name")
        && same("category")
        && a.get_i32("rating").unwrap_or(0) == b.get_i32("rating").unwrap_or(0)
        && same("extra")
        && (ignore_superficials || (same("gearname") && same("notes")))
        && ka.len() == kb.len()
        && ka.iter().zip(&kb).all(|(x, y)| qty(x) == qty(y) && identical(x, y, ignore_superficials))
}

/// `Gear.Copy`: the same gear with fresh guids (and children's
/// `parentid`s pointing at them).
fn copy_gear(g: &Element) -> Element {
    fn renew(e: &mut Element, parent: Option<&str>) {
        let guid = crate::items::new_guid();
        e.set_child_text("guid", guid.clone());
        if let Some(p) = parent {
            if e.child("parentid").is_some() {
                e.set_child_text("parentid", p);
            }
        }
        if let Some(c) = e.child_mut("children") {
            for k in c.elements_mut().filter(|k| k.name == "gear") {
                renew(k, Some(&guid));
            }
        }
    }
    let mut c = g.clone();
    renew(&mut c, None);
    c
}

/// Index path from `root` to the gear with this guid.
fn path_to(root: &Element, guid: &str) -> Option<Vec<usize>> {
    for (i, c) in root.elements().enumerate() {
        if c.get("guid").eq_ignore_ascii_case(guid) {
            return Some(vec![i]);
        }
        if let Some(mut p) = path_to(c, guid) {
            p.insert(0, i);
            return Some(p);
        }
    }
    None
}

fn at_path<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    let mut e = root;
    for &i in path {
        e = e.elements().nth(i)?;
    }
    Some(e)
}

/// The gear that holds gear `guid` inside the pool.
fn gear_parent<'a>(pool: &'a Element, guid: &str) -> Option<&'a Element> {
    let p = path_to(pool, guid)?;
    (p.len() >= 3).then(|| at_path(pool, &p[..p.len() - 2])).flatten().filter(|e| e.name == "gear")
}

/// `Weapon.Reload` for a weapon that needs ammunition: load `count`
/// rounds of the gear `ammo` (or of an external source when `None`) into
/// the active clip.
///
/// Rounds taken from a stack of spare clips or speed loaders take one of
/// them off the stack first. Topping up with the same ammunition adds to
/// the loaded gear; otherwise the rounds are split off into their own
/// gear and the previously loaded gear goes back to the inventory.
pub fn reload(ch: &mut Character, weapon_guid: &str, ammo: Option<&str>, count: i32) -> Result<(), String> {
    let w = weapon(ch, weapon_guid).ok_or("weapon not found")?;
    if !requires_ammo(w) {
        set_charges(ch, weapon_guid, count);
        return Ok(());
    }
    let count = count.max(0);
    let current = loaded(ch, w).map(|g| g.get("guid"));
    let Some(sel_guid) = ammo else {
        // `String_ExternalSource`: not inventory gear, so no id is kept.
        update_clips(ch, weapon_guid, |cs, i| {
            cs[i].ammo = None;
            cs[i].count = count;
        });
        return Ok(());
    };
    let pool_el = pool(ch, weapon_guid).ok_or("no ammunition")?;
    let mut sel = super::find(pool_el, sel_guid).filter(|g| g.name == "gear").ok_or("ammunition not found")?.get("guid");

    // A stack of spare clips: take one off the stack.
    if let Some(parent) = gear_parent(pool_el, &sel) {
        let pname = parent.get("name");
        if (pname.starts_with("Spare Clip") || pname.starts_with("Speed Loader")) && qty(parent) > 1.0 {
            let (pguid, pq) = (parent.get("guid"), qty(parent));
            let rel = path_to(parent, &sel).ok_or("ammunition not found")?;
            let mut dup = copy_gear(parent);
            dup.set_child_text("qty", "1");
            let new_sel = at_path(&dup, &rel).map(|e| e.get("guid")).ok_or("ammunition not found")?;
            pool_mut(ch, weapon_guid).push(dup);
            set_gear_qty(ch, &pguid, pq - 1.0);
            sel = new_sel;
        }
    }

    let find_gear = |ch: &Character, g: &str| super::find(&ch.doc, g).cloned();
    let sel_el = find_gear(ch, &sel).ok_or("ammunition not found")?;
    let mut count = f64::from(count);
    if let Some(cur) = current.as_deref().and_then(|c| find_gear(ch, c)) {
        if identical(&sel_el, &cur, false) {
            // Just top up the currently loaded ammo.
            let top_up = count - qty(&cur);
            let new_count = if top_up > qty(&sel_el) {
                // LIKELY-BUG(LB-20): deviates from Chummer (fixed here). See docs/likely-bugs.md.
                // Chummer subtracts here (`Quantity - selected.Quantity`)
                // while its comment says the stacks merge; they merge.
                let merged = qty(&cur) + qty(&sel_el);
                set_gear_qty(ch, &cur.get("guid"), merged);
                ch.remove_item_anywhere(&sel);
                merged
            } else {
                set_gear_qty(ch, &cur.get("guid"), count);
                let rest = qty(&sel_el) - top_up;
                if rest > 0.0 {
                    set_gear_qty(ch, &sel, rest);
                } else {
                    ch.remove_item_anywhere(&sel);
                }
                count
            };
            update_clips(ch, weapon_guid, |cs, i| cs[i].count = new_count as i32);
            return Ok(());
        }
    }
    if qty(&sel_el) > count {
        // Split the rounds off into their own gear.
        let mut part = copy_gear(&sel_el);
        part.set_child_text("qty", fmt_num(count));
        let part_guid = part.get("guid");
        pool_mut(ch, weapon_guid).push(part);
        set_gear_qty(ch, &sel, qty(&sel_el) - count);
        sel = part_guid;
    } else if count > qty(&sel_el) {
        count = qty(&sel_el);
    }
    update_clips(ch, weapon_guid, |cs, i| {
        cs[i].ammo = Some(sel.clone());
        cs[i].count = count as i32;
    });
    Ok(())
}

/// `Weapon.Unload`: empty the active clip. The rounds go back into an
/// identical stack in the inventory when there is one.
pub fn unload(ch: &mut Character, weapon_guid: &str) -> bool {
    let Some(w) = weapon(ch, weapon_guid) else { return false };
    let Some(g) = loaded(ch, w).cloned() else { return false };
    update_clips(ch, weapon_guid, |cs, i| {
        cs[i].ammo = None;
        cs[i].count = 0;
    });
    let gid = g.get("guid");
    let merge = pool(ch, weapon_guid).and_then(|p| p.children_named("gear").find(|x| !x.get("guid").eq_ignore_ascii_case(&gid) && identical(x, &g, true)).map(|x| (x.get("guid"), qty(x))));
    if let Some((mguid, mq)) = merge {
        set_gear_qty(ch, &mguid, mq + qty(&g));
        ch.remove_item_anywhere(&gid);
    }
    true
}

/// `Weapon.Reload` for a weapon with charges (`requireammo` False): set
/// the internal clip, up to its capacity.
pub fn set_charges(ch: &mut Character, weapon_guid: &str, count: i32) -> bool {
    let Some(w) = weapon(ch, weapon_guid) else { return false };
    if requires_ammo(w) || !uses_ammo(w) {
        return false;
    }
    let max = capacity(ch, w);
    let v = count.clamp(0, max.max(0));
    if remaining(w) == v {
        return false;
    }
    update_clips(ch, weapon_guid, |cs, i| cs[i].count = v)
}
