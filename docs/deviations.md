# Deviations from Chummer5a

chummer-rs ports Chummer5a 5.226 closely. This file lists every place
where it knowingly behaves differently:

- a Chummer bug that chummer-rs does not copy (it follows the rules or the
  evident intent instead),
- a feature done differently, or one that Chummer does not have,
- a simplification or an omission,
- a difference in what is written to `.chum5` / `.chum5lz` files.

Matching Chummer is the default; anything not listed here is meant to
behave as Chummer does.

[likely-bugs.md](likely-bugs.md) has the full analysis of each suspected
Chummer bug (`LB-xx`), with the C# code, the rules text and the tests.
Entries here that come from it give only the id and a summary. Some
likely-bugs entries are no longer deviations, because chummer-rs now does
what Chummer does: LB-30 (only the GUI differs, see below), LB-31 and
LB-33 (except for the karma clamp below). The "Checked, not bugs" entries
(LB-40..43) match Chummer.

Each table has these columns:

- **Chummer5a**: what Chummer 5.226 does.
- **chummer-rs**: what chummer-rs does instead.
- **Why**: a rules reference (book code and printed page, as in
  `books.xml`) or the reason.
- **File**: "yes" when the difference changes what is saved in the
  character file. All such files still load in Chummer 5.226
  ([interop.md](interop.md)).

## Rules fixes vs Chummer bugs

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Karma cost windows (Jack of All Trades): a window above the maximum counts negative levels, and a window is skipped unless the start level is inside it. 6 → 7 costs 17 and 3 → 7 costs 42. | Each window counts its overlap with the range, never less than 0: 16 and 46. This applies to attributes, active and knowledge skills and skill groups. (LB-03) | RF p. 147: −1 karma per level up to 5, +2 per level above 5. | yes (karma spent) |
| Weapon-specific DV, AP, Accuracy and Range improvements are added to the dice pool (AP is also counted as AP). | Only `WeaponSpecificDice` goes to the pool. DV, AP, Accuracy and Range go to their own stats. No stock data creates these. (LB-11) | A DV/AP/Accuracy bonus is not a dice pool bonus. | no |
| Vehicle off-road handling, speed and accel are upgraded from the on-road values. | Each of on-road and off-road is upgraded from its own value. No visible change with stock data. (LB-08) | R5 p. 123. | no |
| Critter attributes at a Force: every limit is at least 1, also a constant "0" in the data (spirit RES/DEP, ESS minimum). | Only a value that depends on the Force is raised to 1. A constant stays as it is. (LB-10) | SR5 p. 303: spirits have no Resonance or Depth. | yes (attribute limits) |

## Career mode

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Undo of "Fettered a Spirit" refunds the karma, but the spirit stays fettered and keeps its MAG −1. | Undo also releases the spirit and removes the MAG −1 improvement. (LB-01) | SG p. 192. Otherwise the fetter is free. | yes |
| Buying karma with nuyen logs the nuyen at `NuyenPerBPWftP` but deducts it at `NuyenPerBPWftM`. | Uses WftP for both, so the log matches the balance. Same result with the default settings (both 2,000). (LB-24) | Book-keeping. | yes (nuyen) |
| Career essence loss can lower an attribute's karma levels below 0. | Karma levels stop at 0. (LB-33) | Negative purchased levels have no meaning. | yes (karma levels) |
| Reload with a top-up that needs more rounds than the stack has: the loaded quantity becomes `Quantity − selected.Quantity`, so rounds disappear. A stack can stay at quantity 0. | The stacks merge, as Chummer's own comment says. A stack that reaches 0 is removed. (LB-20) | Book-keeping. | yes (ammo quantities) |

## Creation

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| The creation form has no Improvements tab. | The Improvements tab is shown in creation mode when the character has custom improvements, so they can still be edited. | Nothing becomes unreachable. | no |
| Creation problems show only as a popup when you click Finish. | Problems are listed all the time: a ⚠ badge with a count on each tab, a panel on the open tab, marks on the rows concerned, and the full list in Karma Summary. An optional guide walks through creation one step at a time. | Community request: don't dump everything at once, and say what's wrong before Finish. | no |
| A technomancer without a stream is an error. | It is a warning and does not block Finish. | chummer-rs has no stream picker yet, so it must not block finishing. | no |

## Items & costs

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| The Vintage accessory cost multiplier is read from the data, but `Save` does not write it, so it is lost on reload. | The multiplier is written into the saved accessory (Chummer's `Load` reads it). Accessories from a Chummer save take it from the data. (LB-04) | GH3 p. 3: Vintage doubles the cost of physical upgrades. | yes (`<accessorycostmultiplier>`) |
| A custom drug's grade cost multiplier is read but not used. | The component total is multiplied by the grade's `<cost>`. (LB-05) | CF p. 190: Street Cooked 0.5, Pharmaceutical 2, Designer 6. | yes (nuyen) |
| A bonus that looks empty, such as `<bonus><unarmeddvphysical/></bonus>`, is dropped on save. | Such bonuses are kept, as 5.18x–5.20x saves did. (LB-25) | The bonus has an effect. | yes (Chummer drops it again on its next save) |
| Print XML: top-level gear prints `<owncost>` as 1 / CostFor. | Prints its own cost / CostFor. (LB-07) | Null parent multiplier. | no (print only) |

## Magic

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Create Spell: an area combat spell never gets the "Area" descriptor, so Witness My Hate applies to a custom Manaball. | Area combat spells get "Area" after their other descriptors, as in the data. (LB-02) | SR5 p. 282; RF p. 151. | yes (spell descriptors) |
| Create Spell: "Extended Area needs Area" tests the Active box instead of Extended Area. | Tests the Extended Area box. (LB-21) | SG p. 108. | only through the spells it allows |
| A fettered spirit does not gain Banishing Resistance. | A fettered spirit (not a sprite) has Banishing Resistance. It is derived from `<fettered>`, shown next to the Fettered box and printed. (LB-32) | SG p. 192; KC p. 91 (sprites gain nothing). | no (print shows it) |
| Print: a "Special" drain value prints as "Special(Special)". | Prints "Special" (or "Special-2" with a modifier). (LB-12) | Cosmetic. | no (print only) |

## A.I.

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Undo of an A.I. program or Advanced Program purchase refunds the karma and keeps the program. | Undo removes the program. It is refused while another program on the character requires it. (LB-26) | DT p. 145. | yes |

## Contacts and relationships

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Dragging a contact only moves it on screen; the saved order does not change. | Drag (☰ handle) or ⏶/⏷ reorders the contacts, and the order is saved as the order of the `<contact>` elements, which Chummer loads in that order. | Keep the order the user chose. | yes (element order) |
| `Contact.Load` never reads `<relative>`, so a linked file is found by `<file>` only, and a re-save writes `<relative>` empty. | Reads `<relative>`, and also looks for the linked file next to the owner's save (a link made on Windows works on Linux). (LB-22) | Links survive a move between machines. | yes (`<relative>` kept) |

## GM tools

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Add PACKS Kit lists the kit's attributes, skills, knowledge skills and adept powers but does not apply them. | Applies them: attributes set to kit value + (metatype minimum − 1); skill groups, skills (with specs), knowledge skills and powers added. Points are used first, then karma, within the creation maximums. (LB-09) | RF p. 63; SR5 p. 66 (one attribute at its maximum). Custom kits from Create PACKS Kit have attributes. | yes |
| Create PACKS Kit writes a kit with only negative qualities with none, and an empty `<negative/>` for a kit with only positive ones. | Writes each quality list that has qualities. (LB-06) | Evident intent. | no (kit file only) |
| A kit weapon goes into the first vehicle mod that is a weapon mount. Drones with built-in weapon mounts lose it. | Falls back to a free built-in weapon mount; with none, the weapon goes to the character and the report says so. (LB-23) | Nothing is lost. | yes |
| Custom kits are read from and written to Chummer's `packs` folder. | Uses `~/.local/share/chummer-rs/packs` (`$XDG_DATA_HOME`). Copy the file to use a kit in both programs. | Separate user data directory. | no |
| Editing a custom improvement opens the form with Augmented empty, so saving the edit loses it. | Restores Augmented from the improvement's AugmentedMaximum for Attribute and ReplaceAttribute types. | The value is not lost on edit. | yes (Augmented kept) |

## Settings and rulesets

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| When a character's settings file is missing, Chummer looks for a preset with the same `<settingshashcode>`. Failing that, it asks, takes the best-matching preset and opens the build method selection. | Shows a warning on the character with Import and Change Settings File. Budgets use Standard, and `<settings>` stays as it was until the user picks another file. | The file keeps its preset name, and the user can import the right preset. | no (until changed) |
| Writes `<settingshashcode>`. | New characters have no `<settingshashcode>`; chummer-rs does not compute the hash. Existing values are kept as loaded. | Not implemented; Chummer loads files without it. | yes |

## Files and interop

These differences are in files that chummer-rs writes. All of them load in
Chummer 5.226; the entries marked "yes" in the sections above are also
file differences.

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Writes its own version as `<appversion>`. | Keeps `<appversion>` as loaded (new characters: 5.226.0) and writes `<chummerrsversion>`. Chummer drops `<chummerrsversion>` when it re-saves. | Chummer picks its load fix-ups by `<appversion>`. | yes |
| Re-saving rewrites the whole file in the current layout. | Writes back only what changed; other elements stay as loaded (lossless round trip). Old files keep old forms, for example `<sex>` instead of `<gender>`. | Nothing in the file is lost. | yes |
| Writes its calculated values (totals and similar) on save. | Does not write all of them. Chummer computes them again on load. | They are derived. | yes |
| Saves only the clips that hold rounds; on load they are put into the slots in order. | Writes every slot up to the last loaded one, empty slots with the empty guid (Chummer loads them as empty clips). A loaded second magazine stays the second one. | Keep the slot the user loaded. | yes |

## GUI and layout

| Chummer5a | chummer-rs | Why | File |
|---|---|---|---|
| Drop-downs are plain lists. | Lists with more than 8 entries have a search field: words in any order, Enter picks the first match. | Long data lists. | no |
| Magic, resonance, A.I. and critter tabs follow the character's flags. | The same, but a tab also shows while the character has items that it lists. | Nothing becomes unreachable. | no |
| Item details are fields on each tab. | Clicking an item opens a detail pane on the right (rating, quantity, equipped, wireless, names, location, notes, children, sell). | Layout. | no |
| Career: a new metamagic goes to the initiation grade node selected in the tree. | A Grade dropdown in the Metamagics section, with each grade's cost; the lowest grade without a metamagic is preselected. (LB-30) | Same result; no tree selection needed. | no |
| Tree views: multi-level qualities are one node; `startcollapsed` gear starts closed; vehicle mod grouping can be turned off; "Initiate Grade" nodes. | Multi-level qualities are one row per level; that gear starts open; vehicle mods are always grouped by category (Chummer's default); plain "Grade N" groups. | Simplification. | no |
| Character sheets are shown in the built-in sheet viewer. | Chummer's XSLT sheets run through `xsltproc` (on a copy with two libxslt fixes) and open in the web browser. Sheets that `sheets.xml` lists but that are not shipped are left out (Chummer shows "File not found"). | No .NET or embedded browser. | no |
| Light and dark mode. | Two themes: Classic (Chummer's Windows look) and Graphite (dark). | Native look on Linux. | no |
| Sourcebook PDF scan. | The scan matches by title (`&` = "and"), then by each book's known text with `pdftotext`, and lists the PDFs it did not link with the reason. Links can be imported from Chummer in a Wine or Proton prefix. PDFs open in a native viewer (evince, zathura, okular, ...). | Linux; files with odd names. | no |
| — | `chummer-cli`: info, skills, items, check, new, sheet, export, roster, search, settings, sources, hash, apply, commands. | Extra. | no |
| No undo; a failed purchase can leave partial changes behind (Chummer rolls some back by hand). | Edit → Undo/Redo (100 steps per character) and a History tab. Every change is a command; a refused command leaves the character exactly as it was. Saving writes the export totals (`<totalvalue>`, `<totaless>`) into the file only, not into the open character. | Extra; groundwork for GM/player sync (docs/online-design.md). | no |

## Not implemented

Not in chummer-rs yet, or left out on purpose (see also README "Not done
yet"):

- Hero Lab import, ChummerHub, plugins and the auto-updater.
- Career mode: enchantments, rituals and enhancements learned at a grade;
  binding stacked foci (undo of a stacked binding works); the Living
  Persona's matrix bonuses.
- PDF export: use the browser's Print to PDF.
- "Change Settings File" in creation mode only offers presets with the
  same build method. Chummer re-runs metatype and priority selection to
  switch build methods.
- PACKS kits: "Select Martial Art" entries without a fixed art are
  skipped (Chummer asks for one); Create PACKS Kit does not write skills
  (Chummer 5.226 does not either).
- Relationships: no "Swap Ordering" (in Chummer it only changes the panel
  flow, no data); no notes colour editor for locations and items.
  Contact `<colour>` is shown but cannot be edited (Chummer has no editor
  either).
- About 290 UI labels have no Chummer translation string and stay English.
- Custom improvements: no drag and drop between groups (use the 📁 menu).
  Disabling one switches only its modifiers and the special attribute and
  tab flags; objects it created stay until it is deleted.
