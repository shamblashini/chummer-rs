//! The GM screen's forms: quick-add (character files, critters, PACKS kit
//! NPCs), copies, GM awards, quick damage and dice rolls. `gm_screen`
//! lays them out; this file draws them and turns them into campaign
//! changes or commands.

use chummer_core::campaign::damage::{self, Attack, Defender, Tracks};
use chummer_core::career::ManualExpense;
use chummer_core::command::Command;
use chummer_core::dice::{self, Rng};
use chummer_core::engine::Engine;
use chummer_core::gm::packs;
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

/// Character files picked and loaded on another thread.
pub type Picked = Vec<(std::path::PathBuf, Result<chummer_core::character::Character, String>)>;

/// Pick character files and load them (on another thread); the answer
/// comes from [`picked_characters`] with the same `link`.
pub fn pick_characters(ctx: &egui::Context, link: bool) {
    crate::bg::dialog(ctx, format!("dialog:gm-add:{link}"), || -> Picked {
        let files = rfd::FileDialog::new().add_filter("Chummer character", &["chum5", "chum5lz"]).add_filter("All files", &["*"]).pick_files().unwrap_or_default();
        files.into_iter().map(|p| {
            let ch = chummer_core::character::Character::load(&p).map_err(|e| e.to_string());
            (p, ch)
        }).collect()
    });
}

/// The files picked with [`pick_characters`], once there.
pub fn picked_characters(link: bool) -> Picked {
    crate::bg::take::<Picked>(&format!("dialog:gm-add:{link}")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// NPC from a PACKS kit
// ---------------------------------------------------------------------------

pub struct KitForm {
    /// `packs.xml` with custom kits.
    doc: Element,
    category: String,
    kit: Option<(String, String)>,
    metatype: String,
    name: String,
    count: u32,
    error: Option<String>,
}

/// What the kit window asks for.
pub struct KitRequest {
    pub kit_xml: String,
    pub metatype: String,
    pub name: String,
    pub count: u32,
}

impl KitForm {
    pub fn new(engine: &Engine) -> KitForm {
        KitForm {
            doc: packs::load(&engine.store, packs::packs_dir().as_deref()),
            category: String::new(),
            kit: None,
            metatype: "Human".into(),
            name: String::new(),
            count: 1,
            error: None,
        }
    }

    pub fn fail(&mut self, e: String) {
        self.error = Some(e);
    }

    /// Draw the window. `Some(None)` closes it; `Some(Some(r))` asks for
    /// NPCs.
    pub fn window(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> Option<Option<KitRequest>> {
        let mut out = None;
        let mut open = true;
        let kits = packs::kits(&self.doc);
        let cats: Vec<String> = packs::categories(&self.doc).into_iter().filter(|c| kits.iter().any(|(_, k)| k == c)).collect();
        if !cats.contains(&self.category) {
            self.category = cats.first().cloned().unwrap_or_default();
        }
        egui::Window::new(lang.tr("Select a PACKS Kit")).id(egui::Id::new("gm_kit_npc")).open(&mut open).default_size([620.0, 480.0]).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("gm_kit_top").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(lang.tr("Category"));
                crate::combo::Combo::from_id_salt("gm_kit_cat").selected_text(self.category.clone()).width(240.0).show_ui(ui, |ui| {
                    for c in &cats {
                        if crate::combo::selectable_label(ui, *c == self.category, c).clicked() {
                            self.category = c.clone();
                            self.kit = None;
                        }
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Metatype"));
                crate::combo::Combo::from_id_salt("gm_kit_metatype").selected_text(self.metatype.clone()).width(240.0).show_ui(ui, |ui| {
                    for h in chummer_core::chargen::karma_metatypes(&engine.store) {
                        crate::combo::selectable_value(ui, &mut self.metatype, h.metatype.clone(), h.metatype);
                    }
                });
                ui.end_row();
                ui.label(lang.tr("Name"));
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(self.kit.as_ref().map(|k| k.0.clone()).unwrap_or_default()).desired_width(240.0));
                ui.end_row();
                ui.label(lang.tr("Count"));
                ui.add(egui::DragValue::new(&mut self.count).range(1..=20));
                ui.end_row();
            });
            ui.separator();
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical().id_salt("gm_kit_list").max_height(280.0).show(&mut cols[0], |ui| {
                    for (n, c) in kits.iter().filter(|(_, c)| *c == self.category) {
                        let sel = self.kit.as_ref().is_some_and(|(sn, sc)| sn == n && sc == c);
                        if crate::combo::selectable_label(ui, sel, n).clicked() {
                            self.kit = Some((n.clone(), c.clone()));
                        }
                    }
                });
                let ui = &mut cols[1];
                let Some(kit) = self.kit.as_ref().and_then(|(n, c)| packs::find_kit(&self.doc, n, c)) else {
                    ui.weak(lang.tr("Choose a kit."));
                    return;
                };
                egui::ScrollArea::vertical().id_salt("gm_kit_contents").max_height(280.0).show(ui, |ui| {
                    for (section, lines) in packs::contents(kit) {
                        ui.label(RichText::new(lang.tr(section)).color(crate::theme::accent(ui)));
                        for l in lines {
                            ui.label(format!("  {l}"));
                        }
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                let kit = self.kit.as_ref().and_then(|(n, c)| packs::find_kit(&self.doc, n, c));
                if ui.add_enabled(kit.is_some(), crate::theme::primary_button(ui, lang.tr("Add"))).clicked() {
                    let kit = kit.expect("enabled");
                    let name = if self.name.trim().is_empty() { self.kit.as_ref().map(|k| k.0.clone()).unwrap_or_default() } else { self.name.trim().to_owned() };
                    out = Some(Some(KitRequest { kit_xml: kit.to_xml_string(), metatype: self.metatype.clone(), name, count: self.count }));
                }
                if ui.button(lang.tr("Cancel")).clicked() {
                    out = Some(None);
                }
            });
        });
        if !open {
            out = Some(None);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// GM awards
// ---------------------------------------------------------------------------

/// "GM gave you 100 karma: note".
pub struct AwardForm {
    pub karma: bool,
    pub amount: f64,
    pub note: String,
}

impl Default for AwardForm {
    fn default() -> Self {
        AwardForm { karma: true, amount: 5.0, note: String::new() }
    }
}

impl AwardForm {
    /// The command that gives (`gain`) or takes the amount: a karma or
    /// nuyen expense entry with the reason.
    pub fn command(&self, gain: bool) -> Command {
        Command::ManualExpense { karma: self.karma, gain, expense: ManualExpense { amount: self.amount, reason: self.note.trim().to_owned(), ..Default::default() } }
    }

    /// Draw the form; returns the command to run (give or take).
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language, career: bool) -> Option<Command> {
        let mut out = None;
        ui.add_enabled_ui(career, |ui| {
            ui.horizontal(|ui| {
                crate::combo::selectable_value(ui, &mut self.karma, true, lang.tr("Karma"));
                crate::combo::selectable_value(ui, &mut self.karma, false, lang.tr("Nuyen"));
                let (step, max) = if self.karma { (1.0, 1000.0) } else { (100.0, 10_000_000.0) };
                ui.add(egui::DragValue::new(&mut self.amount).range(0.0..=max).speed(step));
            });
            ui.add(egui::TextEdit::singleline(&mut self.note).hint_text(lang.tr("Reason")).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                let ok = self.amount > 0.0;
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Give"))).clicked() {
                    out = Some(self.command(true));
                }
                if ui.add_enabled(ok, egui::Button::new(lang.tr("Take"))).clicked() {
                    out = Some(self.command(false));
                }
            });
        });
        if !career {
            ui.weak(lang.tr("Awards are for characters in Career Mode."));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Quick damage
// ---------------------------------------------------------------------------

pub struct DamageForm {
    pub code: String,
    pub soak_roll: bool,
}

impl Default for DamageForm {
    fn default() -> Self {
        DamageForm { code: "6P".into(), soak_roll: true }
    }
}

/// What a damage entry did.
pub struct DamageResult {
    pub physical_filled: i32,
    pub stun_filled: i32,
    /// For the feed: "took 8P AP-2: soaked 3 (14 dice), 5 Stun".
    pub text: String,
}

impl DamageForm {
    /// Draw the entry; `Some(attack)` when Apply was clicked with a valid
    /// code.
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: &Language) -> Option<Attack> {
        let mut out = None;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.code).hint_text("8P AP-2").desired_width(90.0));
            let parsed = Attack::parse(&self.code);
            if parsed.is_none() && !self.code.trim().is_empty() {
                r.on_hover_text(lang.tr("Damage code, e.g. 8P AP-2 or 6S"));
            }
            ui.checkbox(&mut self.soak_roll, lang.tr("Soak roll"));
            if ui.add_enabled(parsed.is_some(), egui::Button::new(lang.tr("Apply"))).clicked() {
                out = parsed;
            }
        });
        out
    }
}

/// Resolve an attack against a defender and its tracks: soak (rolled or
/// not), conversion to Stun, overflow.
pub fn resolve(rng: &mut Rng, a: Attack, d: Defender, t: Tracks, roll_soak: bool) -> DamageResult {
    let inc = damage::incoming(a, d);
    let (hits, soak) = if roll_soak {
        let r = dice::roll(rng, inc.soak_pool.max(0) as u32, false, None);
        (r.hits, format!(", soaked {} of {} dice", r.hits, inc.soak_pool))
    } else {
        (0, String::new())
    };
    let boxes = damage::after_soak(inc, hits);
    let (p, s) = damage::apply(t, boxes, inc.physical);
    let kind = if inc.physical { "Physical" } else { "Stun" };
    let ap = if a.ap != 0 { format!(" AP{:+}", a.ap) } else { String::new() };
    let conv = if inc.converted { ", Stun (DV below armor)" } else { "" };
    let result = if boxes == 0 { "no damage".to_owned() } else { format!("{boxes} {kind}") };
    DamageResult { physical_filled: p, stun_filled: s, text: format!("took {}{}{ap}{conv}{soak}: {result}", a.dv, if a.physical { "P" } else { "S" }) }
}

/// Damage to a character: soaked with its Body (an A.I.'s home node)
/// and armor, onto its condition monitors. Returns the tracks before and
/// what happened.
pub fn damage_character(rng: &mut Rng, a: Attack, ch: &chummer_core::character::Character, sheet: &chummer_core::calc::Sheet, roll_soak: bool) -> (Tracks, DamageResult) {
    let d = Defender { body: chummer_core::calc::soak_body(ch, sheet), armor: sheet.armor, bonus: 0 };
    let t = Tracks {
        physical: sheet.physical_cm,
        stun: sheet.stun_cm,
        overflow: sheet.cm_overflow,
        physical_filled: chummer_core::play::ai::physical_filled(ch),
        stun_filled: chummer_core::play::ai::stun_filled(ch),
    };
    let r = resolve(rng, a, d, t, roll_soak);
    (t, r)
}

/// The commands that put a damage result on the condition monitors.
pub fn damage_commands(before: &Tracks, r: &DamageResult) -> Vec<Command> {
    let mut out = Vec::new();
    if r.physical_filled != before.physical_filled {
        out.push(Command::SetPhysicalDamage { filled: r.physical_filled });
    }
    if r.stun_filled != before.stun_filled {
        out.push(Command::SetStunDamage { filled: r.stun_filled });
    }
    out
}

/// A pool as a chip; click rolls it. Returns a line for the roll log.
pub fn pool_roll(ui: &mut egui::Ui, rng: &mut Rng, lang: &Language, who: &str, label: &str, pool: i32) -> Option<String> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(label);
        if crate::theme::pool_chip(ui, pool.to_string()).on_hover_text(lang.tr("Roll")).clicked() {
            out = Some(roll_pool(rng, lang, who, label, pool));
        }
    });
    out
}

/// Roll a pool for the GM's roll log: "Who: Label 9d6 → 3 hits  [6 5 …]".
pub fn roll_pool(rng: &mut Rng, lang: &Language, who: &str, label: &str, pool: i32) -> String {
    let r = dice::roll(rng, pool.max(0) as u32, false, None);
    roll_line(lang, who, label, pool, &r)
}

/// The roll log line of a roll.
pub fn roll_line(lang: &Language, who: &str, label: &str, pool: i32, r: &dice::Roll) -> String {
    let glitch = match r.glitch {
        dice::Glitch::None => String::new(),
        dice::Glitch::Glitch => format!(" — {}", lang.tr("GLITCH")),
        dice::Glitch::Critical => format!(" — {}", lang.tr("CRITICAL GLITCH")),
    };
    let dice: Vec<String> = r.dice.iter().map(u8::to_string).collect();
    format!("{who}: {label} {pool}d6 → {}{glitch}  [{}]", lang.tr_fmt("{0} hits", &[&r.hits]), dice.join(" "))
}

// ---------------------------------------------------------------------------
// Dice pools at the table
// ---------------------------------------------------------------------------

/// A dice pool to roll at the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    /// Translated.
    pub label: String,
    pub pool: i32,
    /// The pool with a specialization, for skills that have one.
    pub spec: Option<i32>,
}

/// The pools rolled most at the table (the GM screen's card, the
/// Workspace's Play screen): Defense (REA + INT, wound modifier
/// included), Damage Resistance (Body + armor), Composure, Judge
/// Intentions, then the `skills` active skills with the biggest pools.
pub fn quick_pools(ch: &chummer_core::character::Character, sheet: &chummer_core::calc::Sheet, lang: &Language, skills: usize) -> Vec<Pool> {
    let rea_int = sheet.attr("REA") + sheet.attr("INT") + sheet.wound_modifier;
    let soak = chummer_core::calc::soak_body(ch, sheet) + sheet.armor;
    let fixed = [("Defense", rea_int), ("Damage Resistance", soak), ("Composure", sheet.composure), ("Judge Intentions", sheet.judge_intentions)];
    let mut out: Vec<Pool> = fixed.into_iter().map(|(l, p)| Pool { label: lang.tr(l), pool: p, spec: None }).collect();
    let mut best: Vec<_> = sheet.skills.iter().filter(|s| s.rating > 0 && !s.disabled).collect();
    best.sort_by(|a, b| b.pool.cmp(&a.pool).then(a.name.cmp(&b.name)));
    out.extend(best.into_iter().take(skills).map(|s| Pool {
        label: lang.data_name("skills.xml", "", &s.name),
        pool: s.pool,
        spec: (!s.specs.is_empty() && s.spec_bonus > 0).then_some(s.pool + s.spec_bonus),
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_pools_lead_with_defense_and_soak() {
        let Ok(engine) = Engine::load() else { return };
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let ch = chummer_core::character::Character::load(&p).unwrap();
        let sheet = engine.sheet(&ch);
        let lang = Language::default();
        let pools = quick_pools(&ch, &sheet, &lang, 6);
        assert_eq!(pools[0], Pool { label: "Defense".into(), pool: sheet.attr("REA") + sheet.attr("INT") + sheet.wound_modifier, spec: None });
        assert_eq!(pools[1].pool, sheet.attr("BOD") + sheet.armor);
        assert_eq!((pools[2].pool, pools[3].pool), (sheet.composure, sheet.judge_intentions));
        let skills = &pools[4..];
        assert!(skills.len() <= 6 && !skills.is_empty());
        assert!(skills.windows(2).all(|w| w[0].pool >= w[1].pool), "the biggest pools first");
        assert!(skills.iter().all(|s| s.spec.is_none_or(|x| x > s.pool)));
    }

    #[test]
    fn damage_goes_on_the_tracks() {
        let Ok(engine) = Engine::load() else { return };
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let ch = chummer_core::character::Character::load(&p).unwrap();
        let sheet = engine.sheet(&ch);
        let mut rng = Rng::from_time();
        // Not rolled: no soak hits, so a big DV always does damage.
        let (before, r) = damage_character(&mut rng, Attack::parse("30P").unwrap(), &ch, &sheet, false);
        assert_eq!(before.physical, sheet.physical_cm);
        assert!(r.physical_filled > before.physical_filled);
        let cmds = damage_commands(&before, &r);
        assert!(matches!(cmds.as_slice(), [Command::SetPhysicalDamage { filled }] if *filled == r.physical_filled), "{cmds:?}");
        let (before, r) = damage_character(&mut rng, Attack::parse("0S").unwrap(), &ch, &sheet, false);
        assert!(damage_commands(&before, &r).is_empty(), "no damage, no commands");
    }

    #[test]
    fn roll_lines() {
        let lang = Language::default();
        let r = dice::evaluate(vec![6, 5, 1, 2], 5);
        assert_eq!(roll_line(&lang, "Apex", "Pistols", 4, &r), "Apex: Pistols 4d6 → 2 hits  [6 5 1 2]");
        let r = dice::evaluate(vec![1, 1, 3], 5);
        assert_eq!(roll_line(&lang, "Apex", "Soak", 3, &r), "Apex: Soak 3d6 → 0 hits — CRITICAL GLITCH  [1 1 3]");
    }
}
