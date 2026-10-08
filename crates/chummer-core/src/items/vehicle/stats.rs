//! Vehicle derived values: the `Vehicle.Total*`, `Max*` and slot
//! properties, computed from a saved `<vehicle>` element and the
//! `<bonus>` of its mods.

use std::cell::Cell;

use crate::expr::{evaluate_num, fixed_values, needs_evaluation, parse_plain, standard_round};
use crate::settings::CharacterSettings;
use crate::xml::Element;

/// The Rigger 5 mod slot categories (`Vehicle.ModCategoryStrings`).
pub const SLOT_CATEGORIES: &[&str] = &["Powertrain", "Protection", "Weapons", "Body", "Electromagnetic", "Cosmetic"];

/// House rules that change vehicle values.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VehicleRules {
    /// `CharacterSettings.DroneMods`: drones use the Rigger 5 drone mod rules.
    pub drone_mods: bool,
    /// `DroneModsMaximumPilot`: drone mods may raise Pilot to twice its base.
    pub drone_mods_max_pilot: bool,
    /// `DroneArmorMultiplier` when `DroneArmorMultiplierEnabled`.
    pub drone_armor_multiplier: Option<f64>,
    /// `Character.IgnoreRules`: no maximums.
    pub ignore_rules: bool,
}

impl VehicleRules {
    /// Read the drone options from a settings preset.
    pub fn from_settings(s: &CharacterSettings, ignore_rules: bool) -> VehicleRules {
        VehicleRules {
            drone_mods: s.flag("dronemods"),
            drone_mods_max_pilot: s.flag("dronemodsmaximumpilot"),
            drone_armor_multiplier: s.flag("dronearmormultiplierenabled").then(|| f64::from(s.int("dronearmorflatnumber", 2))),
            ignore_rules,
        }
    }
}

/// Used and total slots of one Rigger 5 mod category (`<X>ModSlotsUsed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategorySlots {
    pub category: &'static str,
    pub used: i32,
    pub total: i32,
}

/// A vehicle's values after its mods.
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleStats {
    /// `Vehicle.IsDrone`: the category contains "Drone".
    pub is_drone: bool,
    pub handling: i32,
    pub offroad_handling: i32,
    /// `TotalHandling` as Chummer shows it: "4" or "4/2".
    pub handling_text: String,
    pub speed: i32,
    pub offroad_speed: i32,
    pub speed_text: String,
    pub accel: i32,
    pub offroad_accel: i32,
    pub accel_text: String,
    pub body: i32,
    pub armor: i32,
    pub pilot: i32,
    pub sensor: i32,
    pub seats: i32,
    pub device_rating: i32,
    /// `Slots` and `SlotsUsed`: all mod slots, for vehicles.
    pub slots: i32,
    pub slots_used: i32,
    /// `DroneModSlots` and `DroneModSlotsUsed`, for drones under drone mod rules.
    pub drone_mod_slots: i32,
    pub drone_mod_slots_used: i32,
    /// Rigger 5 slots per category.
    pub categories: Vec<CategorySlots>,
    pub max_armor: i32,
    pub max_handling: i32,
    pub max_speed: i32,
    pub max_accel: i32,
    pub max_sensor: i32,
    pub max_pilot: i32,
}

/// Values of a vehicle with standard rules.
pub fn stats(vehicle: &Element) -> VehicleStats {
    stats_with(vehicle, &VehicleRules::default())
}

/// Values of a vehicle under `rules`.
pub fn stats_with(vehicle: &Element, rules: &VehicleRules) -> VehicleStats {
    let v = Veh::new(vehicle, rules);
    let (handling, offroad_handling, handling_text) = v.total_handling(None);
    let (speed, offroad_speed, speed_text) = v.total_speed(None);
    let (accel, offroad_accel, accel_text) = v.total_accel(None);
    VehicleStats {
        is_drone: v.is_drone,
        handling,
        offroad_handling,
        handling_text,
        speed,
        offroad_speed,
        speed_text,
        accel,
        offroad_accel,
        accel_text,
        body: v.total_body(None),
        armor: v.total_armor(None),
        pilot: v.pilot(None),
        sensor: v.sensor(None),
        seats: v.total_seats(None),
        device_rating: v.device_rating(),
        slots: v.slots(),
        slots_used: v.slots_used(),
        drone_mod_slots: v.drone_mod_slots(),
        drone_mod_slots_used: v.drone_mod_slots_used(),
        categories: SLOT_CATEGORIES.iter().map(|c| CategorySlots { category: c, used: v.category_used(c), total: v.category_total(c) }).collect(),
        max_armor: v.max_armor(),
        max_handling: v.max_handling(),
        max_speed: v.max_speed(),
        max_accel: v.max_accel(),
        max_sensor: v.max_sensor(),
        max_pilot: v.max_pilot(),
    }
}

/// `Vehicle.IsDrone`.
pub fn is_drone(vehicle: &Element) -> bool {
    vehicle.get("category").contains("Drone")
}

/// Base values as saved (the `_int*` fields of `Vehicle`).
#[derive(Debug, Clone, Copy, Default)]
struct Base {
    handling: i32,
    offroad_handling: i32,
    accel: i32,
    offroad_accel: i32,
    speed: i32,
    offroad_speed: i32,
    pilot: i32,
    body: i32,
    armor: i32,
    sensor: i32,
    seats: i32,
}

impl Base {
    fn from(e: &Element) -> Base {
        let i = |k: &str| e.get_i32(k).unwrap_or(0);
        let (handling, offroad_handling) = split_pair(e, "handling", "offroadhandling");
        let (accel, offroad_accel) = split_pair(e, "accel", "offroadaccel");
        let (speed, offroad_speed) = split_pair(e, "speed", "offroadspeed");
        Base { handling, offroad_handling, accel, offroad_accel, speed, offroad_speed, pilot: i("pilot"), body: i("body"), armor: i("armor"), sensor: i("sensor"), seats: i("seats") }
    }
}

/// `Vehicle.Load` for handling/accel/speed: "4/2" splits, otherwise the
/// off-road value is read from its own field (or equals the on-road one).
pub(crate) fn split_pair(e: &Element, key: &str, offroad_key: &str) -> (i32, i32) {
    let t = e.get(key);
    if let Some((a, b)) = t.split_once('/') {
        return (int(a), int(b));
    }
    let on = int(&t);
    (on, e.child_text(offroad_key).filter(|s| !s.trim().is_empty()).map_or(on, |s| int(&s)))
}

fn int(s: &str) -> i32 {
    crate::xml::parse_int(s).unwrap_or(0)
}

/// One mod of the vehicle, with what the totals need.
pub(super) struct ModRef<'a> {
    pub e: &'a Element,
    /// Counts toward totals: neither included in the vehicle nor unequipped.
    pub active: bool,
    pub wireless: bool,
}

impl<'a> ModRef<'a> {
    fn new(e: &'a Element) -> Self {
        ModRef {
            e,
            active: !e.get_bool("included").unwrap_or(false) && e.get_bool("equipped").unwrap_or(true),
            wireless: e.get_bool("wirelesson").unwrap_or(false),
        }
    }
    fn bonus(&self, key: &str) -> Option<String> {
        self.e.child("bonus")?.child_text(key)
    }
    fn wireless_bonus(&self, key: &str) -> Option<String> {
        self.e.child("wirelessbonus")?.child_text(key)
    }
    /// Wireless bonus when wireless is on and has the key, else the bonus.
    fn preferred(&self, key: &str) -> Option<String> {
        if self.wireless {
            self.wireless_bonus(key).or_else(|| self.bonus(key))
        } else {
            self.bonus(key)
        }
    }
    /// Wireless bonus only when wireless is on.
    fn wireless_only(&self, key: &str) -> Option<String> {
        if self.wireless { self.wireless_bonus(key) } else { None }
    }
    /// `VehicleMod.Downgrade`. The `<downgrade />` flag is not saved; in
    /// the data every downgrade mod, and only those, has it in its name.
    fn downgrade(&self) -> bool {
        self.e.get("name").contains("Downgrade")
    }
}

/// A mod being evaluated: on the vehicle (by index) or on a weapon mount.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ModAt {
    Vehicle(usize),
    Mount(usize, usize),
}

/// Recursion guard for expressions that reference totals that reference
/// the same expression.
const MAX_DEPTH: u32 = 8;

/// A saved vehicle with the rules in force: the port of `Vehicle`'s
/// computed properties.
pub(super) struct Veh<'a> {
    pub e: &'a Element,
    base: Base,
    pub is_drone: bool,
    pub mods: Vec<ModRef<'a>>,
    pub mounts: Vec<&'a Element>,
    rules: VehicleRules,
    depth: Cell<u32>,
    /// The owner, to price ware in the vehicle's mods (`None`: saved costs).
    pub pricing: Option<(&'a crate::character::Character, &'a crate::data::DataStore)>,
}

impl<'a> Veh<'a> {
    pub fn new(e: &'a Element, rules: &VehicleRules) -> Self {
        let kids = |c: &str, t: &'a str| e.child(c).map(|x| x.children_named(t).collect::<Vec<_>>()).unwrap_or_default();
        Veh {
            e,
            base: Base::from(e),
            is_drone: is_drone(e),
            mods: kids("mods", "mod").into_iter().map(ModRef::new).collect(),
            mounts: kids("weaponmounts", "weaponmount"),
            rules: *rules,
            depth: Cell::new(0),
            pricing: None,
        }
    }

    fn active(&self, ex: Option<usize>) -> impl Iterator<Item = (usize, &ModRef<'a>)> {
        self.mods.iter().enumerate().filter(move |(i, m)| m.active && Some(*i) != ex)
    }

    fn drone_mods(&self) -> bool {
        self.is_drone && self.rules.drone_mods
    }

    // ---------------------------------------------------------------
    // Expressions
    // ---------------------------------------------------------------

    /// `Vehicle.ParseBonus`. With `bonus`, only `+x`/`-x` expressions count
    /// (additive bonuses); without, only the others (overrides). Plain
    /// numbers count in both, as in Chummer.
    fn parse_bonus(&self, s: Option<&str>, m: usize, total: i32, word: &str, bonus: bool) -> i32 {
        let Some(s) = s.filter(|s| !s.trim().is_empty()) else { return 0 };
        if !needs_evaluation(s) {
            return standard_round(parse_plain(s).unwrap_or(0.0));
        }
        let signed = s.starts_with('+') || s.starts_with('-');
        if signed != bonus {
            return 0;
        }
        let t = total.to_string();
        let s = s.replace(&format!("{{{word}}}"), &t).replace(word, &t);
        standard_round(self.mod_value(ModAt::Vehicle(m), &s))
    }

    /// The mod element at `at`.
    fn mod_el(&self, at: ModAt) -> Option<&'a Element> {
        match at {
            ModAt::Vehicle(i) => self.mods.get(i).map(|m| m.e),
            ModAt::Mount(w, i) => self.mounts.get(w)?.child("mods")?.children_named("mod").nth(i),
        }
    }

    /// `VehicleMod.ProcessRatingStringAsDec` for the mod at `at`.
    pub fn mod_value(&self, at: ModAt, expr: &str) -> f64 {
        let rating = self.mod_el(at).and_then(|e| e.get_i32("rating")).unwrap_or(0);
        let s = fixed_values(expr, rating);
        let s = s.trim_start_matches('+');
        if s.trim().is_empty() {
            return 0.0;
        }
        if !needs_evaluation(s) {
            return parse_plain(s).unwrap_or(0.0);
        }
        let mut s = s.to_owned();
        if has_tokens(&s) {
            if s.contains("Rating") {
                let r = rating.to_string();
                s = s.replace("{Rating}", &r).replace("Rating", &r);
            }
            if let ModAt::Mount(w, _) = at {
                s = self.replace_parent(&s, w);
            }
            let ex = match at {
                ModAt::Vehicle(i) => Some(i),
                ModAt::Mount(..) => None,
            };
            s = self.process_attrs(&s, ex, None);
        }
        evaluate_num(&s).unwrap_or(0.0)
    }

    /// `Parent Cost` and `Parent Slots` of a weapon mount child.
    fn replace_parent(&self, s: &str, w: usize) -> String {
        let mut s = s.to_owned();
        if s.contains("Parent Cost") {
            let c = super::cost::fmt_dec(self.mount_own_cost(w));
            s = s.replace("{Parent Cost}", &c).replace("Parent Cost", &c);
        }
        if s.contains("Parent Slots") {
            let c = self.mount_slots(w).to_string();
            s = s.replace("{Parent Slots}", &c).replace("Parent Slots", &c);
        }
        s
    }

    /// `Vehicle.ProcessAttributesInXPath`: substitute vehicle values.
    /// Totals exclude the mod `ex` (and mount `exm`) being evaluated.
    pub fn process_attrs(&self, input: &str, ex: Option<usize>, exm: Option<usize>) -> String {
        if !has_tokens(input) {
            return input.to_owned();
        }
        if self.depth.get() >= MAX_DEPTH {
            return input.to_owned();
        }
        self.depth.set(self.depth.get() + 1);
        let out = self.substitute(input, ex, exm);
        self.depth.set(self.depth.get() - 1);
        out
    }

    fn substitute(&self, orig: &str, ex: Option<usize>, exm: Option<usize>) -> String {
        let b = self.base;
        let excluding = ex.is_some() || exm.is_some();
        let body = || {
            let t = self.total_body(ex);
            // For mods, total body is always at least 0.5.
            if excluding && t == 0 { "0.5".to_owned() } else { t.to_string() }
        };
        let handling = || self.total_handling(ex);
        let speed = || self.total_speed(ex);
        let accel = || self.total_accel(ex);
        let max_or = |m: i32| m.to_string();
        let mut s = orig.to_owned();
        let mut rep = |token: &str, value: &dyn Fn() -> String| {
            if orig.contains(token) && s.contains(token) {
                s = s.replace(token, &value());
            }
        };
        rep("{BodyBase}", &|| b.body.to_string());
        rep("{HandlingBase}", &|| b.handling.to_string());
        rep("{OffroadHandlingBase}", &|| b.offroad_handling.to_string());
        rep("{SpeedBase}", &|| b.speed.to_string());
        rep("{OffroadSpeedBase}", &|| b.offroad_speed.to_string());
        rep("{AccelerationBase}", &|| b.accel.to_string());
        rep("{OffroadAccelerationBase}", &|| b.offroad_accel.to_string());
        rep("{SensorBase}", &|| b.sensor.to_string());
        rep("{ArmorBase}", &|| b.armor.to_string());
        rep("{PilotBase}", &|| b.pilot.to_string());
        rep("{SeatsBase}", &|| b.seats.to_string());
        rep("{BodyTotal}", &body);
        rep("{HandlingTotal}", &|| handling().0.to_string());
        rep("{OffroadHandlingTotal}", &|| handling().1.to_string());
        rep("{SpeedTotal}", &|| speed().0.to_string());
        rep("{OffroadSpeedTotal}", &|| speed().1.to_string());
        rep("{AccelerationTotal}", &|| accel().0.to_string());
        rep("{OffroadAccelerationTotal}", &|| accel().1.to_string());
        rep("{SensorTotal}", &|| self.sensor(ex).to_string());
        rep("{ArmorTotal}", &|| self.total_armor(ex).to_string());
        rep("{PilotTotal}", &|| self.pilot(ex).to_string());
        rep("{SeatsTotal}", &|| self.total_seats(ex).to_string());
        rep("{HandlingMax}", &|| max_or(self.max_handling()));
        rep("{SpeedMax}", &|| max_or(self.max_speed()));
        rep("{AccelerationMax}", &|| max_or(self.max_accel()));
        rep("{SensorMax}", &|| max_or(self.max_sensor()));
        rep("{ArmorMax}", &|| max_or(self.max_armor()));
        rep("{PilotMax}", &|| max_or(self.max_pilot()));
        rep("{Body}", &body);
        rep("{Handling}", &|| handling().0.to_string());
        rep("{OffroadHandling}", &|| handling().1.to_string());
        rep("{Speed}", &|| speed().0.to_string());
        rep("{OffroadSpeed}", &|| speed().1.to_string());
        rep("{Acceleration}", &|| accel().0.to_string());
        rep("{AccelTotal}", &|| accel().0.to_string());
        rep("{Accel}", &|| accel().0.to_string());
        rep("{OffroadAcceleration}", &|| accel().1.to_string());
        rep("{OffroadAccelTotal}", &|| accel().1.to_string());
        rep("{OffroadAccel}", &|| accel().1.to_string());
        rep("{Sensor}", &|| self.sensor(ex).to_string());
        rep("{Armor}", &|| self.total_armor(ex).to_string());
        rep("{Pilot}", &|| self.pilot(ex).to_string());
        rep("{Seats}", &|| self.total_seats(ex).to_string());
        rep("Body", &body);
        rep("OffroadHandling", &|| handling().1.to_string());
        // Bare Handling, Speed, Acceleration, Sensor and Armor are the base
        // values "for legacy reasons".
        rep("Handling", &|| b.handling.to_string());
        rep("OffroadSpeed", &|| speed().1.to_string());
        rep("Speed", &|| b.speed.to_string());
        rep("OffroadAcceleration", &|| accel().1.to_string());
        rep("OffroadAccel", &|| accel().1.to_string());
        rep("Acceleration", &|| b.accel.to_string());
        rep("Accel", &|| accel().0.to_string());
        rep("Sensor", &|| b.sensor.to_string());
        rep("Armor", &|| b.armor.to_string());
        rep("Pilot", &|| self.pilot(ex).to_string());
        rep("Seats", &|| self.total_seats(ex).to_string());
        let own_cost = || super::cost::fmt_dec(self.own_cost());
        for t in ["{Parent Cost}", "Parent Cost", "{Vehicle Cost}", "Vehicle Cost", "{Cost}", "Cost"] {
            rep(t, &own_cost);
        }
        let own_slots = || self.slots().to_string();
        for t in ["{Parent Slots}", "Parent Slots", "{Vehicle Slots}", "Vehicle Slots", "{Slots}", "Slots"] {
            rep(t, &own_slots);
        }
        s
    }

    // ---------------------------------------------------------------
    // Totals
    // ---------------------------------------------------------------

    /// `Vehicle.GetTotalBody`.
    pub fn total_body(&self, ex: Option<usize>) -> i32 {
        let b = self.base.body;
        b + self
            .active(ex)
            .map(|(i, m)| self.parse_bonus(m.bonus("body").as_deref(), i, b, "Body", true) + self.parse_bonus(m.wireless_only("body").as_deref(), i, b, "Body", true))
            .sum::<i32>()
    }

    /// `Vehicle.GetTotalSeats`.
    pub fn total_seats(&self, ex: Option<usize>) -> i32 {
        let base = self.base.seats;
        let mut total = base;
        for (i, m) in self.active(ex) {
            if let Some(s) = m.preferred("seats").filter(|s| !s.is_empty()) {
                total = total.max(self.parse_bonus(Some(&s), i, base, "Seats", false));
            }
        }
        let bonus: i32 = self
            .active(ex)
            .map(|(i, m)| self.parse_bonus(m.bonus("seats").as_deref(), i, total, "Seats", true) + self.parse_bonus(m.wireless_only("seats").as_deref(), i, total, "Seats", true))
            .sum();
        total + bonus
    }

    /// `Vehicle.GetPilot`: overrides only.
    pub fn pilot(&self, ex: Option<usize>) -> i32 {
        let base = self.base.pilot;
        self.active(ex).fold(base, |acc, (i, m)| acc.max(self.parse_bonus(m.preferred("pilot").as_deref(), i, base, "Pilot", false)))
    }

    /// `Vehicle.GetCalculatedSensor`.
    pub fn sensor(&self, ex: Option<usize>) -> i32 {
        let (total, bonus) = self.override_then_bonus("sensor", "Sensor", self.base.sensor, ex);
        total + bonus
    }

    /// `Vehicle.GetTotalArmor`.
    pub fn total_armor(&self, ex: Option<usize>) -> i32 {
        let (total, bonus) = self.override_then_bonus("armor", "Armor", self.base.armor, ex);
        self.max_armor().min(total.saturating_add(bonus))
    }

    /// The sensor/armor pattern: overrides (bonus, then wireless bonus)
    /// against the running total, then additive bonuses on that total.
    fn override_then_bonus(&self, key: &str, word: &str, base: i32, ex: Option<usize>) -> (i32, i32) {
        let mut total = base;
        for (i, m) in self.active(ex) {
            total = total.max(self.parse_bonus(m.bonus(key).as_deref(), i, total, word, false));
            total = total.max(self.parse_bonus(m.wireless_only(key).as_deref(), i, total, word, false));
        }
        let bonus = self
            .active(ex)
            .map(|(i, m)| self.parse_bonus(m.bonus(key).as_deref(), i, total, word, true) + self.parse_bonus(m.wireless_only(key).as_deref(), i, total, word, true))
            .sum();
        (total, bonus)
    }

    /// Armor and armor bonus used for the speed/handling/accel penalty:
    /// with drone mod rules mods count, otherwise the base armor.
    fn penalty_armor(&self, ex: Option<usize>) -> i32 {
        if !self.drone_mods() {
            return self.base.armor;
        }
        let (total, bonus) = self.override_then_bonus("armor", "Armor", self.base.armor, ex);
        total + bonus
    }

    /// The armor-over-body penalty: `max((min(armor, MaxArmor) - 3 * Body) / div, 0)`.
    fn penalty(&self, ex: Option<usize>, div: i32) -> i32 {
        ((self.penalty_armor(ex).min(self.max_armor()) - self.total_body(None) * 3) / div).max(0)
    }

    /// `Vehicle.GetTotalHandling`: (on-road, off-road, display text).
    pub fn total_handling(&self, ex: Option<usize>) -> (i32, i32, String) {
        let b = self.base;
        let (mut on, mut off) = (b.handling, b.offroad_handling);
        for (i, m) in self.active(ex) {
            if let Some(s) = m.preferred("handling").filter(|s| !s.is_empty()) {
                on = on.max(self.parse_bonus(Some(&s), i, b.handling, "Handling", false));
            }
            if let Some(s) = m.preferred("offroadhandling").filter(|s| !s.is_empty()) {
                off = off.max(self.parse_bonus(Some(&s), i, b.offroad_handling, "OffroadHandling", false));
            }
        }
        let (mut bon, mut boff) = (0, 0);
        for (i, m) in self.active(ex) {
            for v in [m.bonus("handling"), m.wireless_only("handling")] {
                // chummer-rs deviates from Chummer (LB-08): Chummer evaluates
                // the on-road bonus against the off-road handling. R5 p. 123
                // upgrades each rating from its own value.
                bon += self.parse_bonus(v.as_deref(), i, on, "Handling", true);
            }
            for v in [m.bonus("offroadhandling"), m.wireless_only("offroadhandling")] {
                boff += self.parse_bonus(v.as_deref(), i, off, "OffroadHandling", true);
            }
        }
        let p = self.penalty(ex, 3);
        pair_text(on + bon, off + boff, p, b.handling != b.offroad_handling)
    }

    /// `Vehicle.GetTotalSpeed`.
    pub fn total_speed(&self, ex: Option<usize>) -> (i32, i32, String) {
        let b = self.base;
        self.speed_like(ex, ("speed", "Speed", b.speed), ("offroadspeed", "OffroadSpeed", b.offroad_speed), 3)
    }

    /// `Vehicle.GetTotalAccel`.
    pub fn total_accel(&self, ex: Option<usize>) -> (i32, i32, String) {
        let b = self.base;
        self.speed_like(ex, ("accel", "Accel", b.accel), ("offroadaccel", "OffroadAccel", b.offroad_accel), 6)
    }

    /// Shared body of `GetTotalSpeed` and `GetTotalAccel`.
    // chummer-rs deviates from Chummer (LB-08): Chummer compares an
    // off-road override with the on-road total and evaluates the off-road
    // bonus against the on-road value. R5 p. 123 upgrades each rating from
    // its own value, so the off-road side uses the off-road value.
    fn speed_like(&self, ex: Option<usize>, on_key: (&str, &str, i32), off_key: (&str, &str, i32), div: i32) -> (i32, i32, String) {
        let (mut on, mut off) = (on_key.2, off_key.2);
        for (i, m) in self.active(ex) {
            if let Some(s) = m.preferred(on_key.0).filter(|s| !s.is_empty()) {
                on = on.max(self.parse_bonus(Some(&s), i, on_key.2, on_key.1, false));
            }
            if let Some(s) = m.preferred(off_key.0).filter(|s| !s.is_empty()) {
                off = off.max(self.parse_bonus(Some(&s), i, off_key.2, off_key.1, false));
            }
        }
        let (mut bon, mut boff) = (0, 0);
        for (i, m) in self.active(ex) {
            for v in [m.bonus(on_key.0), m.wireless_only(on_key.0)] {
                bon += self.parse_bonus(v.as_deref(), i, on, on_key.1, true);
            }
            for v in [m.bonus(off_key.0), m.wireless_only(off_key.0)] {
                boff += self.parse_bonus(v.as_deref(), i, off, off_key.1, true);
            }
        }
        let p = self.penalty(ex, div);
        pair_text(on + bon, off + boff, p, on_key.2 != off_key.2)
    }

    /// `GetBaseMatrixAttribute("Device Rating")` + mod bonuses
    /// (`GetBonusMatrixAttribute`, without gear).
    pub fn device_rating(&self) -> i32 {
        let s = self.e.get("devicerating");
        let base = if s.trim().is_empty() {
            self.pilot(None)
        } else if needs_evaluation(&s) {
            standard_round(evaluate_num(&self.process_attrs(&s, None, None)).unwrap_or(0.0))
        } else {
            standard_round(parse_plain(&s).unwrap_or(0.0))
        };
        let overclocked = i32::from(self.e.get("overclocked") == "Device Rating");
        // Every mod counts here, included or not (as in Chummer).
        let bonus: i32 = self
            .mods
            .iter()
            .map(|m| {
                let a = m.bonus("devicerating").and_then(|s| crate::xml::parse_int(&s)).unwrap_or(0);
                let b = m.wireless_only("devicerating").and_then(|s| crate::xml::parse_int(&s)).unwrap_or(0);
                a + b
            })
            .sum();
        base + overclocked + bonus
    }

    // ---------------------------------------------------------------
    // Maximums
    // ---------------------------------------------------------------

    /// `Vehicle.MaxArmor`.
    pub fn max_armor(&self) -> i32 {
        if self.rules.ignore_rules || self.drone_mods() {
            return i32::MAX;
        }
        let b = self.base;
        match self.rules.drone_armor_multiplier {
            Some(mult) if self.is_drone => standard_round((f64::from(b.body).max(0.5) + f64::from(b.armor)) * mult),
            _ => (b.body + b.armor).max(1),
        }
    }

    fn drone_cap(&self, base: i32) -> i32 {
        if self.is_drone && !self.rules.ignore_rules { (base * 2).max(1) } else { i32::MAX }
    }

    /// `Vehicle.MaxHandling`.
    pub fn max_handling(&self) -> i32 {
        self.drone_cap(self.base.handling)
    }
    /// `Vehicle.MaxSpeed`.
    pub fn max_speed(&self) -> i32 {
        self.drone_cap(self.base.speed)
    }
    /// `Vehicle.MaxAcceleration`.
    pub fn max_accel(&self) -> i32 {
        self.drone_cap(self.base.accel)
    }
    /// `Vehicle.MaxSensor`.
    pub fn max_sensor(&self) -> i32 {
        self.drone_cap(self.base.sensor)
    }
    /// `Vehicle.MaxPilot`.
    pub fn max_pilot(&self) -> i32 {
        if self.rules.drone_mods_max_pilot { self.drone_cap(self.base.pilot) } else { i32::MAX }
    }

    // ---------------------------------------------------------------
    // Slots
    // ---------------------------------------------------------------

    /// `VehicleMod.CalculatedSlots`.
    pub fn mod_slots(&self, at: ModAt) -> i32 {
        let s = self.mod_el(at).map(|e| e.get("slots")).unwrap_or_default();
        standard_round(self.mod_value(at, &s))
    }

    /// `WeaponMount.CalculatedSlots`: own slots, options and added mods.
    pub fn mount_slots(&self, w: usize) -> i32 {
        let Some(m) = self.mounts.get(w) else { return 0 };
        if self.depth.get() >= MAX_DEPTH {
            return 0;
        }
        self.depth.set(self.depth.get() + 1);
        let own = m.get_i32("slots").unwrap_or(0);
        let options: i32 = m.child("weaponmountoptions").map_or(0, |o| o.elements().map(|x| x.get_i32("slots").unwrap_or(0)).sum());
        let mods: i32 = mount_mods(m).enumerate().filter(|(_, x)| !x.get_bool("included").unwrap_or(false)).map(|(i, _)| self.mod_slots(ModAt::Mount(w, i))).sum();
        self.depth.set(self.depth.get() - 1);
        own + options + mods
    }

    fn active_mounts(&self) -> impl Iterator<Item = usize> + '_ {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.get_bool("included").unwrap_or(false) && m.get_bool("equipped").or_else(|| m.get_bool("equuipped")).unwrap_or(true))
            .map(|(i, _)| i)
    }

    /// `Vehicle.Slots`: 4 or Body, whichever is higher, plus added slots.
    pub fn slots(&self) -> i32 {
        self.total_body(None).max(4) + self.e.get_i32("addslots").unwrap_or(0)
    }

    /// `Vehicle.SlotsUsed`.
    pub fn slots_used(&self) -> i32 {
        let mods: i32 = self.active(None).map(|(i, _)| self.mod_slots(ModAt::Vehicle(i))).sum();
        mods + self.active_mounts().map(|w| self.mount_slots(w)).sum::<i32>()
    }

    /// `Vehicle.DroneModSlots`: base slots plus slots freed by negative-slot
    /// mods (only the first downgrade counts).
    pub fn drone_mod_slots(&self) -> i32 {
        let base = self.e.get_i32("modslots").unwrap_or(self.base.body);
        let mut downgraded = false;
        let mut extra = 0;
        for (i, m) in self.active(None) {
            let s = self.mod_slots(ModAt::Vehicle(i));
            if s >= 0 {
                continue;
            }
            if !m.downgrade() {
                extra -= s;
            } else if !downgraded {
                downgraded = true;
                extra -= s;
            }
        }
        base + extra
    }

    /// `Vehicle.DroneModSlotsUsed`.
    pub fn drone_mod_slots_used(&self) -> i32 {
        let mods: i32 = self.active(None).filter(|(_, m)| !m.downgrade()).map(|(i, _)| self.mod_slots(ModAt::Vehicle(i))).sum();
        mods + self.active_mounts().map(|w| self.mount_slots(w)).sum::<i32>()
    }

    /// Total slots of a Rigger 5 category: base Body plus added slots.
    pub fn category_total(&self, cat: &str) -> i32 {
        let add = match cat.to_ascii_uppercase().as_str() {
            "POWERTRAIN" => "powertrainmodslots",
            "PROTECTION" => "protectionmodslots",
            "WEAPONS" => "weaponmodslots",
            "BODY" => "bodymodslots",
            "ELECTROMAGNETIC" => "electromagneticmodslots",
            "COSMETIC" => "cosmeticmodslots",
            _ => "",
        };
        self.base.body + if add.is_empty() { 0 } else { self.e.get_i32(add).unwrap_or(0) }
    }

    /// `Vehicle.CalcCategoryUsed`.
    pub fn category_used(&self, cat: &str) -> i32 {
        let mods: i32 = self.active(None).filter(|(_, m)| m.e.get("category") == cat).map(|(i, _)| self.mod_slots(ModAt::Vehicle(i))).sum();
        let mounts = if cat.eq_ignore_ascii_case("Weapons") { self.active_mounts().map(|w| self.mount_slots(w)).sum() } else { 0 };
        mods + mounts
    }
}

/// Mods added to a weapon mount.
pub(super) fn mount_mods(m: &Element) -> impl Iterator<Item = &Element> {
    m.child("mods").into_iter().flat_map(|c| c.children_named("mod"))
}

/// Strings that contain tokens to substitute
/// (`HasValuesNeedingReplacementForXPathProcessing`).
fn has_tokens(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_uppercase() || c == '{')
}

/// Totals as (on-road, off-road, text) after the penalty. The text shows
/// both values when the base values differ or the totals do.
fn pair_text(on: i32, off: i32, penalty: i32, base_differs: bool) -> (i32, i32, String) {
    let (a, b) = (on - penalty, off - penalty);
    let text = if base_differs || on != off { format!("{a}/{b}") } else { a.to_string() };
    (a, b, text)
}
