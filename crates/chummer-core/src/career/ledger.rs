//! The expense log (`ExpenseLogEntry`, `ExpenseUndo`), manual karma and
//! nuyen entries (the `CreateExpense` dialog), totals and reputation.

use super::{require_career, require_karma, require_nuyen, CareerError, CareerRules};
use crate::character::Character;
use crate::engine::Engine;
use crate::expr::standard_round;
use crate::improvement::{bool_str, fmt_num};
use crate::xml::{Element, Node};

/// Define a C# enum that saves by name, with `ConvertTo…` parsing.
macro_rules! saved_enum {
    ($(#[$m:meta])* $name:ident, fallback $fb:ident, [$($v:ident),* $(,)?]) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($v),* }

        impl $name {
            /// Every value, in declaration (numeric) order.
            pub const ALL: &'static [$name] = &[$($name::$v),*];

            /// The name Chummer saves (`ToString()`).
            pub fn as_str(self) -> &'static str {
                match self { $($name::$v => stringify!($v)),* }
            }

            /// `Enum.TryParse`: a name or a number; anything else gives the
            /// fallback Chummer uses.
            pub fn parse(s: &str) -> Self {
                let s = s.trim();
                if let Ok(n) = s.parse::<usize>() {
                    return Self::ALL.get(n).copied().unwrap_or($name::$fb);
                }
                Self::ALL.iter().copied().find(|v| v.as_str() == s).unwrap_or($name::$fb)
            }
        }

        /// The zero value, which C# gives an unset field.
        impl Default for $name {
            fn default() -> Self {
                Self::ALL[0]
            }
        }
    };
}

saved_enum!(
    /// `ExpenseType`.
    ExpenseType, fallback Karma, [Karma, Nuyen]
);

impl ExpenseType {
    /// `ExpenseLogEntry.ConvertToExpenseType`: case-insensitive, Karma otherwise.
    fn parse_saved(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("nuyen") {
            ExpenseType::Nuyen
        } else {
            ExpenseType::Karma
        }
    }
}

saved_enum!(
    /// `KarmaExpenseType`: what a karma entry bought, for undo.
    KarmaExpenseType, fallback ManualAdd, [
        ImproveAttribute, AddQuality, ImproveSkillGroup, AddSkill, ImproveSkill, SkillSpec,
        AddMartialArt, AddSpell, AddComplexForm, AddMetamagic, ImproveInitiateGrade, RemoveQuality,
        ManualAdd, ManualSubtract, BindFocus, JoinGroup, LeaveGroup, QuickeningMetamagic,
        AddPowerPoint, AddSpecialization, AddAIProgram, AddAIAdvancedProgram, AddCritterPower,
        SpiritFettering, AddMartialArtTechnique,
    ]
);

saved_enum!(
    /// `NuyenExpenseType`: what a nuyen entry bought, for undo.
    NuyenExpenseType, fallback ManualAdd, [
        AddCyberware, IncreaseLifestyle, AddArmor, AddArmorMod, AddWeapon, AddWeaponMod,
        AddWeaponAccessory, AddGear, AddVehicle, AddVehicleMod, AddVehicleGear, AddVehicleWeapon,
        AddVehicleWeaponMod, AddVehicleWeaponAccessory, AddVehicleWeaponMount, ManualAdd,
        ManualSubtract, AddArmorGear, AddVehicleModCyberware, AddCyberwareGear, AddWeaponGear,
        ImproveInitiateGrade, AddVehicleWeaponMountMod, ModifyVehicleWeaponMount,
    ]
);

/// Undo information of an entry (`ExpenseUndo`). Only the type matching the
/// entry's [`ExpenseType`] means anything; the other keeps its zero value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExpenseUndo {
    pub karma_type: KarmaExpenseType,
    pub nuyen_type: NuyenExpenseType,
    /// GUID (or attribute abbreviation, skill group name...) of the object.
    pub object_id: String,
    /// Quantity bought (gear) or nuyen paid (initiation schooling).
    pub qty: f64,
    /// `Extra` of a removed quality, to restore it on undo.
    pub extra: String,
}

impl ExpenseUndo {
    /// `ExpenseUndo.CreateKarma`.
    pub fn karma(kind: KarmaExpenseType, object_id: impl Into<String>) -> Self {
        ExpenseUndo { karma_type: kind, object_id: object_id.into(), ..Default::default() }
    }

    /// `ExpenseUndo.CreateNuyen`.
    pub fn nuyen(kind: NuyenExpenseType, object_id: impl Into<String>, qty: f64) -> Self {
        ExpenseUndo { nuyen_type: kind, object_id: object_id.into(), qty, ..Default::default() }
    }

    /// `ExpenseUndo.Load`.
    pub fn from_xml(e: &Element) -> Self {
        let mut u = ExpenseUndo::default();
        if let Some(t) = e.child_text("karmatype") {
            u.karma_type = KarmaExpenseType::parse(&t);
        }
        if let Some(t) = e.child_text("nuyentype") {
            u.nuyen_type = NuyenExpenseType::parse(&t);
        }
        u.object_id = e.get("objectid");
        u.qty = e.get_f64("qty").unwrap_or(0.0);
        u.extra = e.get("extra");
        u
    }

    /// `ExpenseUndo.Save`.
    pub fn to_xml(&self) -> Element {
        let mut e = Element::new("undo");
        e.push(Element::with_text("karmatype", self.karma_type.as_str()));
        e.push(Element::with_text("nuyentype", self.nuyen_type.as_str()));
        e.push(Element::with_text("objectid", self.object_id.clone()));
        e.push(Element::with_text("qty", fmt_num(self.qty)));
        e.push(Element::with_text("extra", self.extra.clone()));
        e
    }
}

/// One karma or nuyen entry (`ExpenseLogEntry`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExpenseEntry {
    pub guid: String,
    /// Sortable ISO date as saved, e.g. `2018-10-13T20:58:35`.
    pub date: String,
    /// Positive for gains, negative for spending.
    pub amount: f64,
    pub reason: String,
    pub kind: ExpenseType,
    /// A refund: the amount is not counted as karma or nuyen earned.
    pub refund: bool,
    /// Count a negative amount toward career karma/nuyen anyway.
    pub force_career_visible: bool,
    pub undo: Option<ExpenseUndo>,
}

impl ExpenseEntry {
    /// `ExpenseLogEntry.Create`, dated now with a fresh guid.
    pub fn new(amount: f64, reason: impl Into<String>, kind: ExpenseType) -> Self {
        ExpenseEntry {
            guid: crate::items::new_guid(),
            date: crate::chargen::now_iso(),
            amount,
            reason: reason.into(),
            kind,
            ..Default::default()
        }
    }

    pub fn with_undo(mut self, undo: ExpenseUndo) -> Self {
        self.undo = Some(undo);
        self
    }

    /// `ExpenseLogEntry.Load`. Old files append " (Refund)" to the reason
    /// and use "🡒" for arrows; both are normalised as Chummer does.
    pub fn from_xml(e: &Element) -> Self {
        let reason = e.get("reason");
        let reason = reason.strip_suffix(" (Refund)").unwrap_or(&reason).replace('🡒', "->");
        ExpenseEntry {
            guid: e.get("guid"),
            date: e.get("date"),
            amount: e.get_f64("amount").unwrap_or(0.0),
            reason,
            kind: ExpenseType::parse_saved(&e.get("type")),
            refund: e.get_bool("refund").unwrap_or(false),
            force_career_visible: e.get_bool("forcecareervisible").unwrap_or(false),
            undo: e.child("undo").map(ExpenseUndo::from_xml),
        }
    }

    /// `ExpenseLogEntry.Save`.
    pub fn to_xml(&self) -> Element {
        let mut e = Element::new("expense");
        e.push(Element::with_text("guid", self.guid.clone()));
        e.push(Element::with_text("date", self.date.clone()));
        e.push(Element::with_text("amount", fmt_num(self.amount)));
        e.push(Element::with_text("reason", self.reason.clone()));
        e.push(Element::with_text("type", self.kind.as_str()));
        e.push(Element::with_text("refund", bool_str(self.refund)));
        e.push(Element::with_text("forcecareervisible", bool_str(self.force_career_visible)));
        if let Some(u) = &self.undo {
            e.push(u.to_xml());
        }
        e
    }

    /// The karma this entry changed the balance by (`Amount.ToInt32()`).
    pub fn karma_delta(&self) -> i32 {
        self.amount.trunc() as i32
    }

    /// Counts toward career karma/nuyen (`CareerKarma` filter).
    pub fn counts_as_earned(&self) -> bool {
        (self.amount > 0.0 || self.force_career_visible) && !self.refund
    }

    /// Manual entries are the only ones whose amount can be edited.
    pub fn is_manual(&self) -> bool {
        let Some(u) = &self.undo else { return false };
        match self.kind {
            ExpenseType::Karma => matches!(u.karma_type, KarmaExpenseType::ManualAdd | KarmaExpenseType::ManualSubtract),
            ExpenseType::Nuyen => matches!(u.nuyen_type, NuyenExpenseType::ManualAdd | NuyenExpenseType::ManualSubtract),
        }
    }
}

// ---------------------------------------------------------------------------
// Reading and writing the log
// ---------------------------------------------------------------------------

/// Every entry, in file order (`Character.ExpenseEntries`).
pub fn entries(ch: &Character) -> Vec<ExpenseEntry> {
    ch.items("expenses", "expense").into_iter().map(ExpenseEntry::from_xml).collect()
}

/// Karma or nuyen entries, newest first, as the career tabs list them
/// (`ExpenseLogEntry.CompareTo` sorts by date descending).
pub fn entries_of(ch: &Character, kind: ExpenseType) -> Vec<ExpenseEntry> {
    let mut v: Vec<ExpenseEntry> = entries(ch).into_iter().filter(|e| e.kind == kind).collect();
    v.sort_by(|a, b| b.date.cmp(&a.date));
    v
}

pub fn find_entry(ch: &Character, guid: &str) -> Option<ExpenseEntry> {
    ch.items("expenses", "expense").into_iter().find(|e| e.get("guid").eq_ignore_ascii_case(guid)).map(ExpenseEntry::from_xml)
}

/// Add an entry to the log in date order (`ExpenseEntries.AddWithSort`).
/// Does not change the karma or nuyen balance. Returns the entry's guid.
pub fn push_entry(ch: &mut Character, entry: &ExpenseEntry) -> String {
    let log = ch.items_mut("expenses");
    let pos = log
        .children
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name == "expense" && e.get("date") > entry.date))
        .unwrap_or(log.children.len());
    log.children.insert(pos, Node::Element(entry.to_xml()));
    entry.guid.clone()
}

/// Remove an entry from the log without touching the balance or the
/// object it bought. See [`super::undo`] for reversing a spend.
pub fn remove_entry(ch: &mut Character, guid: &str) -> Option<ExpenseEntry> {
    let log = ch.doc.child_mut("expenses")?;
    let pos = log.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "expense" && e.get("guid").eq_ignore_ascii_case(guid)))?;
    let Node::Element(e) = log.children.remove(pos) else { return None };
    ch.dirty = true;
    Some(ExpenseEntry::from_xml(&e))
}

fn entry_element_mut<'a>(ch: &'a mut Character, guid: &str) -> Option<&'a mut Element> {
    ch.doc.child_mut("expenses")?.elements_mut().find(|e| e.name == "expense" && e.get("guid").eq_ignore_ascii_case(guid))
}

/// Log a karma change and apply it to the balance (`ModifyKarma`).
pub(super) fn book_karma(ch: &mut Character, amount: i32, reason: impl Into<String>, undo: ExpenseUndo) -> String {
    let entry = ExpenseEntry::new(f64::from(amount), reason, ExpenseType::Karma).with_undo(undo);
    ch.karma += amount;
    ch.dirty = true;
    push_entry(ch, &entry)
}

/// Log a nuyen change and apply it to the balance (`ModifyNuyen`).
pub(super) fn book_nuyen(ch: &mut Character, amount: f64, reason: impl Into<String>, undo: Option<ExpenseUndo>) -> String {
    let mut entry = ExpenseEntry::new(amount, reason, ExpenseType::Nuyen);
    entry.undo = undo;
    ch.nuyen += amount;
    ch.dirty = true;
    push_entry(ch, &entry)
}

// ---------------------------------------------------------------------------
// Manual entries (the "Add Karma/Nuyen" buttons and the CreateExpense form)
// ---------------------------------------------------------------------------

/// What the `CreateExpense` form collects.
#[derive(Debug, Clone, Default)]
pub struct ManualExpense {
    /// Always positive; the button decides the sign.
    pub amount: f64,
    pub reason: String,
    /// ISO date; `None` means now.
    pub date: Option<String>,
    pub refund: bool,
    pub force_career_visible: bool,
    /// Also convert karma and nuyen at the settings' exchange rate
    /// ("working for the man/the people").
    pub exchange: bool,
}

impl ManualExpense {
    fn entry(&self, amount: f64, kind: ExpenseType, undo: ExpenseUndo) -> ExpenseEntry {
        let mut e = ExpenseEntry::new(amount, self.reason.clone(), kind).with_undo(undo);
        if let Some(d) = self.date.as_ref().filter(|d| !d.trim().is_empty()) {
            e.date = d.trim().to_owned();
        }
        e
    }
}

fn manual_amount(m: &ManualExpense) -> Result<f64, CareerError> {
    if m.amount <= 0.0 || !m.amount.is_finite() {
        return Err(CareerError::Refused("the amount must be positive".into()));
    }
    Ok(m.amount)
}

/// Gain karma (`cmdKarmaGained_Click`). With `exchange`, the karma is paid
/// for in nuyen at `NuyenPerBPWftP`. Returns the karma entry's guid.
///
/// Chummer logs the nuyen at `WftP` but deducts it at `WftM`; this port
/// uses `WftP` for both so the log always matches the balance.
pub fn karma_gained(ch: &mut Character, cr: &CareerRules, m: &ManualExpense) -> Result<String, CareerError> {
    require_career(ch)?;
    let amount = manual_amount(m)?;
    let nuyen = amount * cr.nuyen_per_bp_wftp;
    if m.exchange {
        require_nuyen(ch, nuyen)?;
    }
    let mut k = m.entry(amount, ExpenseType::Karma, ExpenseUndo::karma(KarmaExpenseType::ManualAdd, ""));
    k.refund = m.refund;
    ch.karma += amount.trunc() as i32;
    let guid = push_entry(ch, &k);
    if m.exchange {
        let mut n = m.entry(-nuyen, ExpenseType::Nuyen, ExpenseUndo::nuyen(NuyenExpenseType::ManualSubtract, "", 0.0));
        n.force_career_visible = m.force_career_visible;
        ch.nuyen -= nuyen;
        push_entry(ch, &n);
    }
    ch.dirty = true;
    Ok(guid)
}

/// Spend karma (`cmdKarmaSpent_Click`). With `exchange`, the karma buys
/// nuyen at `NuyenPerBPWftM`. Returns the karma entry's guid.
pub fn karma_spent(ch: &mut Character, cr: &CareerRules, m: &ManualExpense) -> Result<String, CareerError> {
    require_career(ch)?;
    let amount = manual_amount(m)?;
    require_karma(ch, amount.ceil() as i32)?;
    let mut k = m.entry(-amount, ExpenseType::Karma, ExpenseUndo::karma(KarmaExpenseType::ManualSubtract, ""));
    k.refund = m.refund;
    k.force_career_visible = m.force_career_visible;
    ch.karma -= amount.trunc() as i32;
    let guid = push_entry(ch, &k);
    if m.exchange {
        let nuyen = amount * cr.nuyen_per_bp_wftm;
        let mut n = m.entry(nuyen, ExpenseType::Nuyen, ExpenseUndo::nuyen(NuyenExpenseType::ManualSubtract, "", 0.0));
        n.force_career_visible = m.force_career_visible;
        ch.nuyen += nuyen;
        push_entry(ch, &n);
    }
    ch.dirty = true;
    Ok(guid)
}

/// Gain nuyen (`cmdNuyenGained_Click`). With `exchange`, karma pays for it
/// at `NuyenPerBPWftM`. Returns the nuyen entry's guid.
pub fn nuyen_gained(ch: &mut Character, cr: &CareerRules, m: &ManualExpense) -> Result<String, CareerError> {
    require_career(ch)?;
    let amount = manual_amount(m)?;
    let karma = (amount / cr.nuyen_per_bp_wftm).trunc() as i32;
    if m.exchange {
        require_karma(ch, karma)?;
    }
    let mut n = m.entry(amount, ExpenseType::Nuyen, ExpenseUndo::nuyen(NuyenExpenseType::ManualAdd, "", 0.0));
    n.refund = m.refund;
    ch.nuyen += amount;
    let guid = push_entry(ch, &n);
    if m.exchange {
        let mut k = m.entry(f64::from(-karma), ExpenseType::Karma, ExpenseUndo::karma(KarmaExpenseType::ManualSubtract, ""));
        k.refund = m.refund;
        k.force_career_visible = m.force_career_visible;
        ch.karma -= karma;
        push_entry(ch, &k);
    }
    ch.dirty = true;
    Ok(guid)
}

/// Spend nuyen (`cmdNuyenSpent_Click`). With `exchange`, the nuyen buys
/// karma at `NuyenPerBPWftP`. Returns the nuyen entry's guid.
pub fn nuyen_spent(ch: &mut Character, cr: &CareerRules, m: &ManualExpense) -> Result<String, CareerError> {
    require_career(ch)?;
    let amount = manual_amount(m)?;
    require_nuyen(ch, amount)?;
    let n = m.entry(-amount, ExpenseType::Nuyen, ExpenseUndo::nuyen(NuyenExpenseType::ManualSubtract, "", 0.0));
    ch.nuyen -= amount;
    let guid = push_entry(ch, &n);
    if m.exchange {
        let karma = (amount / cr.nuyen_per_bp_wftp).trunc() as i32;
        let mut k = m.entry(f64::from(karma), ExpenseType::Karma, ExpenseUndo::karma(KarmaExpenseType::ManualSubtract, ""));
        k.refund = m.refund;
        k.force_career_visible = m.force_career_visible;
        ch.karma += karma;
        push_entry(ch, &k);
    }
    ch.dirty = true;
    Ok(guid)
}

/// Changes from the edit dialog (`lstKarma_DoubleClick`, `lstNuyen_DoubleClick`).
#[derive(Debug, Clone, Default)]
pub struct EntryEdit {
    pub reason: Option<String>,
    pub date: Option<String>,
    /// New signed amount. Applied to manual entries only.
    pub amount: Option<f64>,
}

/// Edit an entry. Reason and date can always change; the amount only for
/// manual entries, and the balance follows it. Returns false if no entry
/// has this guid.
pub fn edit_entry(ch: &mut Character, guid: &str, edit: &EntryEdit) -> bool {
    let Some(old) = find_entry(ch, guid) else { return false };
    let new_amount = edit.amount.filter(|_| old.is_manual());
    let Some(e) = entry_element_mut(ch, guid) else { return false };
    if let Some(r) = &edit.reason {
        e.set_child_text("reason", r.clone());
    }
    if let Some(d) = &edit.date {
        e.set_child_text("date", d.clone());
    }
    if let Some(a) = new_amount {
        match old.kind {
            ExpenseType::Karma => {
                let a = a.trunc();
                e.set_child_text("amount", fmt_num(a));
                ch.karma += a as i32 - old.karma_delta();
            }
            ExpenseType::Nuyen => {
                e.set_child_text("amount", fmt_num(a));
                ch.nuyen += a - old.amount;
            }
        }
    }
    ch.dirty = true;
    true
}

// ---------------------------------------------------------------------------
// Totals
// ---------------------------------------------------------------------------

/// Summary of the log for the career panel.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LedgerTotals {
    /// Karma earned over the career (`CareerKarma`).
    pub career_karma: i32,
    /// Karma spent: the negative, non-refund karma entries, as a positive number.
    pub karma_spent: i32,
    /// Sum of every karma entry; equals the balance when the log is complete.
    pub karma_logged: i32,
    /// Nuyen earned over the career (`CareerNuyen`).
    pub career_nuyen: f64,
    /// Nuyen spent, as a positive number.
    pub nuyen_spent: f64,
    /// Sum of every nuyen entry.
    pub nuyen_logged: f64,
}

/// `Character.CareerKarma`: earned karma, i.e. positive non-refund entries
/// (or forced visible ones), each rounded with `StandardRound`.
pub fn career_karma(ch: &Character) -> i32 {
    entries(ch).iter().filter(|e| e.kind == ExpenseType::Karma && e.counts_as_earned()).map(|e| standard_round(e.amount)).sum()
}

/// `Character.CareerNuyen`.
pub fn career_nuyen(ch: &Character) -> f64 {
    entries(ch).iter().filter(|e| e.kind == ExpenseType::Nuyen && e.counts_as_earned()).map(|e| e.amount).sum()
}

/// All totals in one pass.
pub fn totals(ch: &Character) -> LedgerTotals {
    let mut t = LedgerTotals::default();
    for e in entries(ch) {
        let spent = e.amount < 0.0 && !e.refund;
        match e.kind {
            ExpenseType::Karma => {
                t.karma_logged += e.karma_delta();
                if e.counts_as_earned() {
                    t.career_karma += standard_round(e.amount);
                }
                if spent {
                    t.karma_spent -= e.karma_delta();
                }
            }
            ExpenseType::Nuyen => {
                t.nuyen_logged += e.amount;
                if e.counts_as_earned() {
                    t.career_nuyen += e.amount;
                }
                if spent {
                    t.nuyen_spent -= e.amount;
                }
            }
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Street cred, notoriety, public awareness
// ---------------------------------------------------------------------------

/// Reputation values of the career panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reputation {
    /// `CalculatedStreetCred`: career karma / (10 + StreetCredMultiplier) − burnt.
    pub street_cred_calculated: i32,
    /// `TotalStreetCred`.
    pub street_cred: i32,
    /// `BurntStreetCred` (saved).
    pub burnt_street_cred: i32,
    /// `CalculatedNotoriety`: Notoriety improvements − burnt / 2.
    pub notoriety_calculated: i32,
    /// `TotalNotoriety`.
    pub notoriety: i32,
    /// `CalculatedPublicAwareness`.
    pub public_awareness_calculated: i32,
    /// `TotalPublicAwareness` (1 at most while Erased).
    pub public_awareness: i32,
}

/// Street cred, notoriety and public awareness (`Character.cs`).
pub fn reputation(ch: &Character, cr: &CareerRules) -> Reputation {
    let imps = &ch.improvements;
    let saved = |k: &str| ch.doc.get_i32(k).unwrap_or(0);
    let burnt = saved("burntstreetcred");
    let divisor = 10 + imps.val_int("StreetCredMultiplier", None);
    let sc_calc = if divisor == 0 { 0 } else { career_karma(ch) / divisor } - burnt;
    let street_cred = (sc_calc + saved("streetcred") + imps.val_int("StreetCred", None)).max(0);
    let not_calc = imps.val_int("Notoriety", None) - burnt / 2;
    let notoriety = not_calc + saved("notoriety");
    let mut pa_calc = imps.val_int("PublicAwareness", None);
    if cr.use_calculated_public_awareness {
        pa_calc += (street_cred + notoriety) / 3;
    }
    let mut public_awareness = saved("publicawareness") + pa_calc;
    if public_awareness >= 1 && imps.has("Erased") {
        public_awareness = 1;
    }
    Reputation {
        street_cred_calculated: sc_calc,
        street_cred,
        burnt_street_cred: burnt,
        notoriety_calculated: not_calc,
        notoriety,
        public_awareness_calculated: pa_calc,
        public_awareness,
    }
}

/// [`reputation`] with the character's own settings.
pub fn reputation_for(engine: &Engine, ch: &Character) -> Reputation {
    reputation(ch, &CareerRules::for_character(engine, ch))
}
