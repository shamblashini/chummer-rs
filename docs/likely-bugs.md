# Likely bugs

chummer-rs ports Chummer5a 5.226 closely, quirks included. This file
lists the places where the port follows (or knowingly does not follow)
behaviour that looks like a Chummer bug, so that each one can be decided
on its own.

Each entry has an id. Entries with a code site have a matching
`// LIKELY-BUG(<id>)` comment there (LB-26 has none, because the A.I.
merge is changing that code). Rules references are to the printed page numbers
(book code as in `books.xml`). The rules text is paraphrased.

Classes:

- **(a)** Chummer bug that chummer-rs copies.
- **(b)** Chummer bug that chummer-rs does not copy (already fixed here).
- **(c)** Simplification or missing feature in chummer-rs.
- **(d)** Checked: intended behaviour, not a bug.

Recommendation: **fix**, **keep** (match Chummer), or **ask** (a
judgement call).

## Recommended fixes

| Id | Area | Chummer | chummer-rs | Rules | Class | Rec. |
|---|---|---|---|---|---|---|
| LB-01 | Career undo: spirit fettering | Undo of the "Fettered a Spirit" expense refunds Force × 3 karma and drops the entry. The spirit stays fettered and the MAG −1 improvement stays (`ExpenseUndo` has no `SpiritFettering` case). | Same. | SG p. 192: fettering costs Force × 3 karma and 1 point of Magic. The refund without unfettering gives a free fetter. | (a) | fix |
| LB-02 | Create Spell: area combat spells | The "Area" descriptor is added only when `cboRange.SelectedValue` contains "(A)". The combo value is "T"/"LOS" (the "(A)" goes on the saved range from `chkArea`), so a combat spell never gets "Area". | Fixed: an area combat spell (`d.area`) gets "Area" after its other descriptors ("Direct, Area", "Indirect, Elemental, Area", as in the data), so Witness My Hate no longer applies to it. | SR5 p. 282: area spells are marked "(A)" after the range. The data gives every official area combat spell the "Area" descriptor (Manaball: "Direct, Area"). Witness My Hate (RF p. 151) is for single-target Direct spells only and keys on `Direct,NOT(Area)`, so a custom Manaball wrongly gets +2 DV and +2 drain. | (b) — fixed | fixed |
| LB-03 | Skill karma cost windows (Jack of All Trades) | `Skill.RangeCost` adds `Value × (min(upper, Max) − max(lower, Min − 1))`. (1) When `lower ≥ Max` the count is negative: 6 → 7 pays +1 from the −1 window (17, not 16). (2) A window is only used when `Minimum ≤ lower`: 3 → 7 skips the +2 window for levels 6–7 (42, not 46). | Same. | RF p. 147, Jack of All Trades: −1 karma per level up to rating 5 (minimum 1), +2 karma per level above 5. Correct costs: 16 and 46. | (a) | fix |
| LB-04 | Weapon accessory cost multiplier (Vintage) | `WeaponAccessory.Create` reads `<accessorycostmultiplier>` from the data, but `Save` does not write it, so the multiplier is lost after a reload. | Worse: `accessory_element` does not copy the field, so Vintage never doubles the other accessories, not even before saving. | GH3 p. 3, Vintage: physical upgrades cost twice the listed amount. | (a) | fix (copy the field from the data record when the item is made) |
| LB-05 | Custom drug grade cost | `CreateCustomDrug` reads the grade's `<cost>` into `_dblCostMultiplier` and never uses it. `Drug.Cost` sums the components only. | Same (`drug::cost`). | CF p. 190: street-cooked drugs cost half. Data: Street Cooked 0.5, Pharmaceutical 2, Designer 6. | (a) | fix |
| LB-06 | Create PACKS Kit: qualities | The export tests `blnPositive` twice, so a kit with only negative qualities is written with none. | Same. | n/a (Chummer feature). | (a) | fix |
| LB-07 | Print XML: gear `<owncost>` | `Gear.OwnCost` is `(pre * Parent?.ChildCostMultiplier ?? 1) / CostFor`. With no gear/armor parent the product is null, so top-level gear prints 1 / CostFor. | Same. No bundled sheet shows `owncost`; only print XML / export output. | n/a. | (a) | fix (low priority) |

## Ask

| Id | Area | Chummer | chummer-rs | Rules | Class | Rec. |
|---|---|---|---|---|---|---|
| LB-08 | Vehicle handling/speed/accel totals | `GetTotalHandling` evaluates the on-road handling bonus against the off-road handling. `GetTotalSpeed`/`GetTotalAccel` compare an off-road override with the on-road total (`off = max(on, override)`). | Same. | R5 p. 123: the drone mods set Handling/Speed/Acceleration to the upgraded rating. No stock vehicle has a different off-road speed or accel, and no handling bonus refers to `Handling`, so there is no visible effect with stock data. | (a) | ask (latent; fix changes nothing today) |
| LB-09 | PACKS kits: attributes and skills | `AddPACKSKit` lists the kit's attributes, skills, knowledge skills and powers in the dialog but does not apply them. | Same; reported as skipped. Listed in README "Not done yet". | Kits are a Chummer feature; no rules. Data has kits that set attributes and skills, which now do nothing. | (a) | ask (a fix makes kits differ from Chummer 5.226) |
| LB-10 | Critter attribute limits at a Force | `ExpressionToInt` gives at least `intMinValueFromForce` (1) when Force > 0, also for a literal "0". Spirits get RES and DEP limits and an ESS minimum of 1 where the data says 0. A failed expression also gives 1. | Fixed: only a value that depends on the Force (`F`, `1D6`, `2D6`) is raised to 1; a constant such as "0" stays (spirit RES/DEP/ESS minimum, sprite MAG/EDG/physical attributes). A failed expression still gives 1. | SR5 p. 303: spirit stat blocks have no Resonance or Depth. The floor of 1 for real attributes (F−3 at Force 1) is fine. | (b) — fixed | fixed |
| LB-11 | Weapon dice pool | `Weapon.DicePool` adds the `WeaponSpecificDV`, `WeaponSpecificAP`, `WeaponSpecificAccuracy` and `WeaponSpecificRange` improvements to the pool, not only `WeaponSpecificDice`. | Same. | A DV/AP/Accuracy bonus is not a dice pool bonus. No stock data or custom improvement type creates these four, so there is no effect today. | (a) | ask (latent) |
| LB-12 | Print: spell drain "Special" | `Spell.CalculatedDv` sends a non-numeric DV through XPath; it fails and the text is appended: "Special(Special)". | Fixed: when the DV does not evaluate, only the modifiers are appended: "Special" prints as "Special", a limited one as "Special-2". | n/a (cosmetic). | (b) — fixed | fixed |

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

## Simplifications in chummer-rs

| Id | Area | Chummer | chummer-rs | Rules | Class | Rec. |
|---|---|---|---|---|---|---|
| LB-30 | Career: which grade a new metamagic goes to | The player selects an initiation grade node in the tree; the metamagic goes there, free if that grade has no metamagic yet. | The GUI picks the lowest grade without one, else the top grade. Costs are the same in total. | SR5 p. 325: each initiation grade gives one metamagic. | (c) | keep |
| LB-31 | Cyberware Device Rating by grade | `Grade` reads `<devicerating>` from the grade record and falls back to a name table. | Uses only the name table. Bioware grades (data: 0) get 2–6; custom grades with their own value are ignored. | SR5 p. 234: basic cyberware 2, alphaware 3, betaware 4, deltaware 5. Bioware is not an electronic device. | (c) | fix (read the data field) |
| LB-32 | Fettered spirits | Fettering does not add the Banishing Resistance power. | Same. | SG p. 192: a fettered spirit gains Banishing Resistance. KC p. 91 for sprite pets. | (c) | ask |
| LB-33 | Essence loss in career mode (RAW) | Burns karma levels and power points step by step as essence drops. | Not ported: a career character's essence-loss improvements stay as they are (`essence_loss.rs`). | SR5 p. 95: any fraction of Essence lost lowers Magic/Resonance by 1. | (c) | fix (missing feature) |

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
| LB-01 | `career/undo.rs:94` (the no-op arm), `career/magic.rs:268` | `tests/career_actions.rs` `fettering_undo_refunds_like_chummer` |
| LB-02 | `gm/custom_spell.rs` `descriptors` | `tests/gm.rs` `custom_spell_drain_and_descriptors`, `witness_my_hate_skips_custom_area_spells` |
| LB-03 | `calc/karma_cost.rs:24` (`window_extra`), `modifiers` below it | `calc/karma_cost.rs` `active_skill_cost_windows` (42 and 17) |
| LB-04 | `items/weapon.rs:703`, `accessory_element` at `items/weapon.rs:242` | `items/weapon.rs` `accessory_multiplier_comes_from_the_saved_accessory` |
| LB-05 | `items/drug.rs:294` | — |
| LB-06 | `gm/packs.rs:696` | — |
| LB-07 | `print/items.rs:233` | — |
| LB-08 | `items/vehicle/stats.rs:527`, `:551` | — |
| LB-09 | `gm/packs.rs:203` | — |
| LB-10 | `gm/mod.rs` `expression_to_int` | `gm/mod.rs` tests ("a constant 0 stays 0"), `tests/gm.rs` `critter_constant_limits_are_not_raised` |
| LB-11 | `items/weapon.rs:1413` | — |
| LB-12 | `print/magic.rs` `calculated_dv` | `tests/print_oracle.rs` `special_dv_prints_as_is`, `limited_special_dv_appends_the_modifier` |
| LB-20 | `play/ammo.rs:644` | — |
| LB-21 | `gm/custom_spell.rs:161` | `tests/gm.rs` (detection: extended area implies area) |
| LB-22 | `contacts.rs:278` | — |
| LB-23 | `gm/packs.rs:568` | — |
| LB-24 | `career/ledger.rs:331` | — |
| LB-25 | `items/magic/mod.rs:135` | — |
| LB-26 | `career/undo.rs:94` | `tests/ai.rs:192` |
| LB-30 | `chummer-gui/src/magic_ui.rs:464` | — |
| LB-31 | `play/matrix.rs:69` | — |
| LB-32 | `items/magic/spirit.rs:122` | — |
| LB-33 | `essence_loss.rs` (module docs) | — |
| LB-40 | `career/actions.rs:55` | — |

Code paths are under `crates/chummer-core/src/` and test paths under
`crates/chummer-core/tests/`, unless shown otherwise.
