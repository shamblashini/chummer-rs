# Likely bugs

chummer-rs ports Chummer5a 5.226 closely, quirks included. This file
lists the places where the port follows (or knowingly does not follow)
behaviour that looks like a Chummer bug, so that each one can be decided
on its own.

Each entry has an id. Open entries with a code site have a matching
`// LIKELY-BUG(<id>)` comment there; fixed entries have a
"chummer-rs deviates from Chummer (<id>)" comment instead. Rules references are to the printed page numbers
(book code as in `books.xml`). The rules text is paraphrased.

All the places where chummer-rs knowingly differs from Chummer, including
the fixed entries here, are summarised in [deviations.md](deviations.md).

Classes:

- **(a)** Chummer bug that chummer-rs copies.
- **(b)** Chummer bug that chummer-rs does not copy (already fixed here).
- **(c)** Simplification or missing feature in chummer-rs.
- **(d)** Checked: intended behaviour, not a bug.

Recommendation: **fix**, **keep** (match Chummer), or **ask** (a
judgement call).

## Recommended fixes

None open: every recommended fix is done (see "Already fixed").

## Ask

None open: the user chose to fix every entry (see "Already fixed").

## Already fixed in chummer-rs

| Id | Area | Chummer | chummer-rs | Rules | Class |
|---|---|---|---|---|---|
| LB-20 | Reload: top up the loaded ammo | `Weapon.Reload`, when a full top-up needs more rounds than the stack has: sets the loaded quantity to `Quantity − selected.Quantity` while its comment says the stacks merge. Rounds disappear. | Merges the stacks (`play/ammo.rs`). Also removes a stack that reaches 0 (Chummer may leave a 0-quantity stack). | n/a (book-keeping). | (b) |
| LB-21 | Create Spell: Extended Area (Detection) | The rule "Extended Area needs Area" tests `chkModifier4` (Active) where `chkModifier14` (Extended Area) is meant. | Tests the Extended Area box, as the comment says. | SG p. 108 (detection spells): Extended Area is an area spell. | (b) |
| LB-22 | Linked contacts: `<relative>` | `Contact.Load` reads `<file>` but never `<relative>` (only `Spirit` reads it). The fallback to the relative path only works before the first reload, and a re-save writes it empty. | Reads `<relative>`, and also looks next to the owner's save. | n/a. | (b) |
| LB-23 | PACKS kits: vehicle weapons | A kit weapon goes into the first vehicle mod that is a weapon mount. Vehicles with built-in `<weaponmounts>` (drones) lose the weapon. | Falls back to a free built-in weapon mount; with none, the weapon goes to the character and is reported. | n/a. | (b) |
| LB-24 | Career: buy karma with nuyen | Logs the nuyen at `NuyenPerBPWftP` but deducts it at `NuyenPerBPWftM`. | Uses WftP for both, so the log matches the balance. Same result with the default settings (both 2,000). | Chummer house-rule setting; no RAW career exchange. | (b) |
| LB-25 | Empty-looking bonuses | `IsNullOrInnerTextIsEmpty` treats `<bonus><unarmeddvphysical/></bonus>` as empty, so the bonus is lost on save. | Keeps such bonuses (as the 5.18x–5.20x saves did). | n/a. | (b) |
| LB-26 | Career undo: A.I. programs | Undo of an A.I. program / Advanced Program purchase refunds the karma and keeps the program. | Fixed: undo removes the program, and is refused while another program on the character requires it. | DT p. 145 (A.I.s). | (b) — fixed |
| LB-03 | Skill karma cost windows (Jack of All Trades) | `Skill.RangeCost` adds `Value × (min(upper, Max) − max(lower, Min − 1))`. (1) When `lower ≥ Max` the count is negative: 6 → 7 pays +1 from the −1 window (17, not 16). (2) A window is only used when `Minimum ≤ lower`: 3 → 7 skips the +2 window for levels 6–7 (42, not 46). | Fixed: the level count is clamped at 0 and every window that overlaps the range counts (16 and 46). This applies to all karma cost windows (attributes, active and knowledge skills, skill groups); multipliers keep Chummer's Minimum test. | RF p. 147, Jack of All Trades: −1 karma per level up to rating 5 (minimum 1), +2 karma per level above 5. | (b) — fixed |
| LB-04 | Weapon accessory cost multiplier (Vintage) | `WeaponAccessory.Create` reads `<accessorycostmultiplier>` from the data, but `Save` does not write it, so the multiplier is lost after a reload. | Fixed: `accessory_element` copies the multiplier into the saved accessory (Chummer's `Load` reads it), and an accessory without it (a Chummer save) takes it from its data record. Apex Predator's saved nuyen is 800 higher than chummer-rs computes (tests/nuyen_oracle.rs). | GH3 p. 3, Vintage: physical upgrades cost twice the listed amount. | (b) — fixed |
| LB-05 | Custom drug grade cost | `CreateCustomDrug` reads the grade's `<cost>` into `_dblCostMultiplier` and never uses it. `Drug.Cost` sums the components only. | Fixed: `drug::cost_with` multiplies the sum by the grade's `<cost>` from drugcomponents.xml. | CF p. 190: street-cooked drugs cost half. Data: Street Cooked 0.5, Pharmaceutical 2, Designer 6. | (b) — fixed |
| LB-06 | Create PACKS Kit: qualities | The export tests `blnPositive` twice, so a kit with only negative qualities is written with none (and a kit with only positive ones gets an empty `<negative/>`). | Fixed: each list is written when it has qualities. | n/a (Chummer feature). | (b) — fixed |
| LB-07 | Print XML: gear `<owncost>` | `Gear.OwnCost` is `(pre * Parent?.ChildCostMultiplier ?? 1) / CostFor`. With no gear/armor parent the product is null, so top-level gear prints 1 / CostFor. | Fixed: gear without a gear/armor parent prints its own cost / CostFor. | n/a. | (b) — fixed |
| LB-08 | Vehicle handling/speed/accel totals | `GetTotalHandling` evaluates the on-road handling bonus against the off-road handling. `GetTotalSpeed`/`GetTotalAccel` compare an off-road override with the on-road total (`off = max(on, override)`) and evaluate the off-road bonus against the on-road value. | Fixed: each of on-road and off-road is upgraded from its own value. No visible change with stock data. | R5 p. 123: the drone mods set Handling/Speed/Acceleration to the upgraded rating. | (b) — fixed |
| LB-09 | PACKS kits: attributes, skills, powers | `AddPACKSKit` lists the kit's attributes, skills, knowledge skills and powers in the dialog but does not apply them (Chummer 5.193 still applied attributes; skills and powers were TODO). Stock `packs.xml` has none of these; custom kits from Create PACKS Kit have attributes. | Fixed (kits now differ from Chummer 5.226): attributes are reset and set to kit value + (metatype minimum − 1); skill groups, skills (with spec; a member of a rated group buys only the levels above it), knowledge skills and adept powers are added. Levels use attribute/special/skill/group/knowledge points first, then karma; values are capped at the maximum (attributes: one at the maximum, `maxnumbermaxattributescreate`) and at the creation skill maximum. | Kits are a Chummer feature (RF p. 63); SR5 p. 66 for one attribute at its maximum. | (b) — fixed |
| LB-11 | Weapon dice pool | `Weapon.DicePool` adds the `WeaponSpecificDV`, `WeaponSpecificAP`, `WeaponSpecificAccuracy` and `WeaponSpecificRange` improvements to the pool, not only `WeaponSpecificDice` (AP also goes to the AP, so it counts twice). | Fixed: only `WeaponSpecificDice` goes to the pool; DV to the damage, AP to the AP, Accuracy to the accuracy, Range as a percent range bonus. No stock data creates the four. | A DV/AP/Accuracy bonus is not a dice pool bonus. | (b) — fixed |
| LB-31 | Cyberware Device Rating by grade | `Grade` reads `<devicerating>` from the grade record and falls back to a name table. | Fixed: reads the grade record's `<devicerating>` from cyberware.xml or bioware.xml (stock data), name table as fallback. Bioware grades give 0. | SR5 p. 234: basic cyberware 2, alphaware 3, betaware 4, deltaware 5. Bioware is not an electronic device. | (b) — fixed |
| LB-01 | Career undo: spirit fettering | Undo of the "Fettered a Spirit" expense refunds Force × 3 karma and drops the entry. The spirit stays fettered and the MAG −1 improvement stays (`ExpenseUndo` has no `SpiritFettering` case). | Fixed: undo also releases the spirit, which removes the MAG −1 `SpiritFettering` improvement (or only drops a stale one if the spirit is gone and no other is fettered). | SG p. 192: fettering costs Force × 3 karma and 1 point of Magic. The refund without unfettering gives a free fetter. | (b) — fixed |
| LB-02 | Create Spell: area combat spells | The "Area" descriptor is added only when `cboRange.SelectedValue` contains "(A)". The combo value is "T"/"LOS" (the "(A)" goes on the saved range from `chkArea`), so a combat spell never gets "Area". | Fixed: an area combat spell (`d.area`) gets "Area" after its other descriptors ("Direct, Area", "Indirect, Elemental, Area", as in the data), so Witness My Hate no longer applies to it. | SR5 p. 282: area spells are marked "(A)" after the range. The data gives every official area combat spell the "Area" descriptor (Manaball: "Direct, Area"). Witness My Hate (RF p. 151) is for single-target Direct spells only and keys on `Direct,NOT(Area)`, so a custom Manaball wrongly gets +2 DV and +2 drain. | (b) — fixed |
| LB-10 | Critter attribute limits at a Force | `ExpressionToInt` gives at least `intMinValueFromForce` (1) when Force > 0, also for a literal "0". Spirits get RES and DEP limits and an ESS minimum of 1 where the data says 0. A failed expression also gives 1. | Fixed: only a value that depends on the Force (`F`, `1D6`, `2D6`) is raised to 1; a constant such as "0" stays (spirit RES/DEP/ESS minimum, sprite MAG/EDG/physical attributes). A failed expression still gives 1. | SR5 p. 303: spirit stat blocks have no Resonance or Depth. The floor of 1 for real attributes (F−3 at Force 1) is fine. | (b) — fixed |
| LB-12 | Print: spell drain "Special" | `Spell.CalculatedDv` sends a non-numeric DV through XPath; it fails and the text is appended: "Special(Special)". | Fixed: when the DV does not evaluate, only the modifiers are appended: "Special" prints as "Special", a limited one as "Special-2". | n/a (cosmetic). | (b) — fixed |

## Simplifications in chummer-rs

| Id | Area | Chummer | chummer-rs | Rules | Class | Rec. |
|---|---|---|---|---|---|---|
| LB-30 | Career: which grade a new metamagic goes to | The player selects an initiation grade node in the tree; the metamagic goes there, free if that grade has no metamagic yet. | Fixed: the Metamagics section has a Grade dropdown (career mode) showing each grade's cost; the lowest grade without one, else the top grade, is preselected (`career::default_metamagic_grade`). | SR5 p. 325: each initiation grade gives one metamagic. | (b) — fixed | fixed |
| LB-32 | Fettered spirits | Fettering does not add the Banishing Resistance power. | Fixed: a fettered spirit (not a sprite) has Banishing Resistance, derived from `<fettered>` (nothing extra saved). The GUI shows it next to the Fettered box; the print lists the spirit's powers (critter data plus Banishing Resistance) for a fettered spirit. Spirits that are not fettered print no powers, as in Chummer (which prints them only for a linked spirit file). | SG p. 192: a fettered spirit gains Banishing Resistance. KC p. 91: a sprite pet gains no power. | (b) — fixed | fixed |
| LB-33 | Essence loss in career mode (RAW) | Burns karma levels and power points step by step as essence drops. | Fixed (ported): career-mode RAW essence loss writes `EssenceLoss` MAG/MAGAdept/RES/DEP improvements for the loss since creation and burns karma levels (and a mystic adept's power points) once a minimum cannot drop further, as Chummer does (karma levels stop at 0; Chummer does not clamp). The GUI refreshes it when Essence or the essence at special start changes. | SR5 p. 95: any fraction of Essence lost lowers Magic/Resonance by 1. | (b) — fixed | fixed |

## Checked, not bugs

| Id | Area | Finding | Class |
|---|---|---|---|
| LB-40 | Burn Edge | `Degrade` takes the point from karma levels, then base points, then the metatype minimum, with no expense entry and no refund. SR5 p. 57: a burnt point is gone and can be bought back with karma, so this is fine. Chummer's comment says "Edge cannot go below 1", but the code (and chummer-rs) refuses only at 0; the rules allow 0. | (d) |
| LB-41 | Linked contacts: relative path | Chummer writes `"../" + MakeRelativeUri(startup, file)` (the URI treats the startup folder as a file). The result still resolves from the startup folder, so the odd form is harmless. | (d) |
| LB-42 | Critter powers karma | Creation karma subtracts every critter power's `<karma>`. Metatype powers (Centaur Search, Sasquatch Mimicry) resolve to the base records, which have no karma; only the optional/Infected copies cost 9. Career mode logs a purchase even at 0 karma (harmless). | (d) |
| LB-43 | `selectquality` | The C# reads `contributetobp` from the data record, where it never appears, so a quality picked through a bonus (life modules) is always free. The module's karma pays for it, so this is the intended result. | (d) |

## Code locations

Line numbers are as of this file's commit; search for `LIKELY-BUG(<id>)`.
Tests that pin the current behaviour change with a fix.

| Id | Code | Test that pins it |
|---|---|---|
| LB-01 | `career/undo.rs` `undo_fettering` | `tests/career_actions.rs` `fettering_undo_releases_the_spirit`, `fettering_undo_of_a_deleted_spirit` |
| LB-02 | `gm/custom_spell.rs` `descriptors` | `tests/gm.rs` `custom_spell_drain_and_descriptors`, `witness_my_hate_skips_custom_area_spells` |
| LB-03 | `calc/karma_cost.rs` (`window_extra`) | `calc/karma_cost.rs` `active_skill_cost_windows`, `jack_of_all_trades_windows_per_level` |
| LB-04 | `items/weapon.rs` (`accessory_element`, `accessory_multiplier`) | `items/weapon.rs` `accessory_multiplier_comes_from_the_saved_accessory`, `vintage_keeps_its_multiplier_through_save_and_reload`; `tests/nuyen_oracle.rs` `vintage_doubling` |
| LB-05 | `items/drug.rs` (`cost_with`) | `tests/drug.rs` `custom_drug_from_components`, `custom_drug_grade_multiplies_the_cost` |
| LB-06 | `gm/packs.rs` (`from_character`) | `tests/gm.rs` `kit_export_writes_each_quality_list_it_has` |
| LB-07 | `print/items.rs` (`gear`) | `tests/print_oracle.rs` `top_level_gear_prints_its_own_cost` |
| LB-08 | `items/vehicle/stats.rs` (`total_handling`, `speed_like`) | `tests/vehicles.rs` `offroad_values_upgrade_from_their_own_base` |
| LB-09 | `gm/packs.rs` (`apply`, `attributes`, `skills`, `knowledge_skills`, `powers`) | `tests/gm.rs` `kit_attributes_and_skills_are_applied`, `kit_round_trip_through_the_packs_folder` |
| LB-10 | `gm/mod.rs` `expression_to_int` | `gm/mod.rs` tests ("a constant 0 stays 0"), `tests/gm.rs` `critter_constant_limits_are_not_raised` |
| LB-11 | `items/weapon.rs` (`dice_pool`, `damage`, `ap`, `accuracy`, `range_bonus`) | `tests/weapons_armor.rs` `weapon_specific_bonuses_go_to_their_own_stat` |
| LB-12 | `print/magic.rs` `calculated_dv` | `tests/print_oracle.rs` `special_dv_prints_as_is`, `limited_special_dv_appends_the_modifier` |
| LB-20 | `play/ammo.rs:644` | — |
| LB-21 | `gm/custom_spell.rs:161` | `tests/gm.rs` (detection: extended area implies area) |
| LB-22 | `contacts.rs:278` | — |
| LB-23 | `gm/packs.rs:568` | — |
| LB-24 | `career/ledger.rs:331` | — |
| LB-25 | `items/magic/mod.rs:135` | — |
| LB-26 | `career/undo.rs:94` | `tests/ai.rs:192` |
| LB-30 | `chummer-gui/src/magic_ui.rs` `metamagic_ui`, `career/magic.rs` `default_metamagic_grade` | `tests/career_actions.rs` `metamagic_goes_to_the_chosen_grade` |
| LB-31 | `play/matrix.rs` (`grade_device_rating`) | `tests/play.rs` `ware_device_rating_comes_from_the_grade_data` |
| LB-32 | `items/magic/spirit.rs` `powers`, `print/magic.rs` `spirit` | `tests/career_actions.rs` `fettered_spirits_gain_banishing_resistance` |
| LB-33 | `essence_loss.rs` `raw_career`, `chummer-gui/src/view.rs` `recompute` | `tests/essence_loss.rs` `career_mode_essence_loss_matches_saved`, `career_mode_essence_loss_lowers_mag_and_burns_karma` |
| LB-40 | `career/actions.rs:55` | — |

Code paths are under `crates/chummer-core/src/` and test paths under
`crates/chummer-core/tests/`, unless shown otherwise.
