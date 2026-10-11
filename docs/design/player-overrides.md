# Player overrides: design

Status: proposal, for review by the project owner before any code is
written. Date: 2026-10-11.

The request: let players easily do what the rules engine says no to, when
the table says yes. The main cases are:

- **Free purchase.** Loot found in play, a gift from a Johnson, a quality
  the GM hands out. No nuyen or karma is spent.
- **Ignore availability.** Buy above the creation availability limit, or
  buy restricted or forbidden gear.
- **Ignore requirements.** Take a quality, spell or power whose
  required/forbidden lists say no.
- **Force placement.** Put an item where the placement rules refuse it,
  or past a container's capacity.
- **Override values.** Set a custom cost, a rating above the maximum, or
  a custom name.
- **Custom item.** Add something that is not in the data.

This file has six parts: (1) what exists now, (2) where the controls go
in both layouts, (3) the rules for recording and showing overrides,
(4) keyboard and discoverability, (5) an implementation plan, and
(6) open questions.

Paths are relative to `crates/` unless they start with `docs/`. Chummer5a
paths are relative to `~/Work/chummer5a/Chummer/`.

---

## 1. Inventory

### 1.1 What chummer-rs has now

**Engine (chummer-core)**

| Override | Exists? | Where | Notes |
|---|---|---|---|
| `Purchase.free` | yes, in the command | `chummer-core/src/items/mod.rs:52` | Serialised in `Command::AddItem` (`command.rs:129`). It overwrites `<cost>` with `"0"`, so nothing on the item says "this was free" and the data cost is lost. |
| free: gear | yes | `items/gear.rs:494` `apply_cost_choices` | cost = 0 |
| free: weapons, accessories | yes | `items/weapon.rs:388`, `:421` | |
| free: armor, armor mods | yes | `items/armor.rs:360`, `:392` | |
| free: cyberware, bioware | yes | `items/cyberware.rs:655` | |
| free: vehicles, mods, mounts | yes | `items/vehicle.rs:108` | |
| free: drugs | yes | `items/drug.rs:214` | |
| free: lifestyles | **ignored** | `items/lifestyle.rs:79-85` | Lifestyle *qualities* do have a stored `<free>` (`lifestyle.rs:341-347`, `Command::AddLifestyleQuality{free}`). |
| free: qualities | **no** | `career/karma.rs:509` (career), `chargen.rs:604` (creation) | `purchase.free` is not read. |
| free: martial arts, critter powers, metamagic | **no** | `career/magic.rs:194`, `:254`; `run.rs:342` | |
| free: AI programs | yes | `career/karma.rs:746-769` `learn_item` | The only karma path that reads `purchase.free`. |
| free: spells | yes, separate flag | `items/magic/spell.rs:15` `SpellOptions.free_bonus`, saved as `<freebonus>` | `pay_for_spell` skips the karma. |
| free: contacts | yes, stored | `contacts.rs:72` `<free>`, `SetContactField{key:"free"}` | |
| free: specialisations | stored, never set by a command | `skills.rs:10` | |
| free career raises (attribute, skill, group, knowledge, power point, initiation) | **no** | `career/karma.rs:102`, `:292`, `:378`, `:309`; `career/magic.rs:170` | Back door: `SetAttributeKarma`, `SetSkillKarma`, `SetKarma` and `SetNuyen` (`run.rs:98-149`) do not check `ch.created`, so they change values in career with no ledger entry. |
| Black market / cost multiplier | partly | `Purchase.cost_multiplier` (`items/mod.rs:55`) | Gear honours any multiplier. Vehicles turn any multiplier below 1 into 10 %. Weapons, armor and cyberware ignore it. The GUI always sends 1.0. |
| Availability check | validation only | `chargen/issues.rs:469-510` (`AvailabilityTooHigh`, `RestrictedGearUsed`) | Creation only. The issue list is empty in career or with `<ignorerules>` (`issues.rs:247`). `FinishCreation` (`run.rs:170`) does **not** block on errors. Legality (R/F) is parsed (`expr.rs:504`) but never checked. |
| Requirement check | GUI only | `requirements.rs:20` `unmet` | `AddItem` and `career::add_quality` never call it. |
| Placement | `MoveItem` only | `items/place.rs:247` `check`, `:412` `Host::takes`, `:554` `move_item` | Capacity is enforced only when the settings preset's `enforcecapacity` is on (`place.rs:241`). `AddItem` with a `parent` does **not** call `place::check`; only the GUI checks. There is no force flag. |
| Rating override | no | `items/edit.rs:332` `rating_range`, `:378` | Ratings are clamped. `<ignorerules>` lifts only power levels and vehicle maximums. |
| Custom name, notes | yes | `edit::custom_name_field` (`edit.rs:457`), `SetItemText` | |
| Custom cost | **no command** | | `SetItemText{field:"cost"}` would write any child element (`run.rs:256` checks only that it is an XML name). That is a loophole, not a feature. |
| Custom item | data only | `resources/data/gear.xml:19983` "Custom Item", cost `Variable(0-1000000)` | No custom-name/cost flow. Custom drugs (`AddCustomDrug`) and custom spells (`gm/custom_spell.rs`) exist. |
| Ignore Rules | per-character flag | `<ignorerules>`, `ch.flag("ignorerules")`, set for critters (`gm/critter.rs:271`) | No UI toggle. It does not bypass cost, availability, requirements, ratings or placement. |
| History text | yes | `command/describe.rs:167` | "Added {name} {rating} ({answer})", plus "(N karma, ¥)" from the before/after difference. **It does not mention free, cost multiplier or a forced placement**, so a free item reads like a paid one with no cost suffix. |
| Career ledger | yes | `career/ledger.rs`; `ManualExpense` `:296`; `UndoExpense` → `career/undo.rs:17` | A free item writes **no** entry (`run.rs:525` skips cost 0), so it does not appear in the ledger and the ledger's Undo cannot reach it. `edit_entry` (`ledger.rs:422`) has no command. |
| Campaign feed | yes | `chummer-sync/src/feed.rs:16` `text`, `msg.rs:294` `FeedEntry` | Text only. There is no override marker. The authority refuses nothing for being "not allowed" (`authority.rs:17-23`). |
| Campaign settings | `RollSettings` only | `chummer-core/src/campaign.rs:403` | There are no permissions and no GM approval. |

**GUI (chummer-gui)**

| What | Where | How a user reaches it |
|---|---|---|
| Workspace catalog | `workspace/catalog.rs:641` `ws_catalog_panel` | Item page → "Side by side". Header: `Catalog`, count, `Filters`, close. Then `ws_catalog_search` (search box and kind segments), `ws_catalog_chips` (`Avail ≤ 12 ×`, `Grade: Standard`, "N hidden by filters"), the results table (Name, R, Ess, Avail, Cost; a `+` on rows with no problems), and the footer hints `Enter add · Shift+Enter add, keep focus · Tab to the inventory`. |
| Catalog filters | `catalog.rs:788` `ws_catalog_filters` | `Filters`: Kind, Category, Grade, Rating, "Fits this build" (`Availability ≤ N`, `Fits Essence`, `Affordable`, `Requirements met`), Legality, Books. These are filters, not overrides. |
| Catalog inspector (purchase panel) | `catalog.rs:1677` `ws_catalog_inspector` | Right column "Item": tags (Restricted/Forbidden), Rating stepper, Grade, Quantity, Essence bar, Cost, Availability, "Nuyen left after", "after" rows, Capacity; **RULES** (`Requirements met.`, warnings); `[+ Add X]` and the compare (⚖) button; the note "Remove it later from the list." / "Career mode pays for it from your nuyen."; `▸ Data`. |
| Blocking in the catalog | `catalog.rs:1846-1863`, `:1910` | Add is **disabled** when requirements fail, when availability is over the limit (creation) or when the career purchase is refused (not enough nuyen). The tooltip reads "Fix the problems above first". There is no way past it. |
| Back door in the catalog | `catalog.rs:1533` `ws_catalog_add` | `ws_catalog_add` checks only placement and the required parent, **not** requirements or availability. So these paths add a record that the inspector's Add refuses: Enter / Shift+Enter (`:671-673`), double-click on a dimmed row (`Event::Open`, `:1516`), the folded details strip's Add (`details_strip`, `:2214`, disabled only when refused), and dragging the row onto the inventory (`ws_catalog_drop_buy`, `:1618`). This looks like an accident; see open question 9. |
| Classic select dialog | `select.rs:119` `SelectDialog::show` | Classic "+ Add gear…" (also used for "Add Quality…": `view.rs:805`, Workspace `build.rs:158`). Top row: Search, category combo, `Show unavailable`. Left: the list. Right: details, warnings, `purchase_options` (Rating, Quantity, Grade, Install in, Availability, Cost). Bottom: `Add` / `Cancel`. **Add is disabled for an entry with unmet reasons** (`select.rs:243`), even with "Show unavailable" on. |
| Free spell | `magic_ui.rs:328` | Spell dialog checkbox `Free` ("Costs no karma and does not count against free spells"). |
| Free lifestyle quality | `lifestyle_ui.rs:168` | Lifestyle editor checkbox `Free`. |
| Free contact | `relationships_ui.rs:382` | Contact card flag `Free`. |
| Free improvement | `improvement_ui.rs:524` | Custom improvement dialog `Free!`. |
| Item row menu (Workspace) | `workspace/inventory.rs:448-485` | Creation: `✎ Edit`, `Move…` (submenu), `🗑 Remove (also removes its improvements)`. Career: `✎ Edit`, `Sell…` (100/75/50/25 %), `⋯ More` (moves, `Remove without refund`, `Open the sourcebook at this page`). |
| Item inspector (Workspace) | `workspace/inspector.rs:103` | Header (name, source); grid: Rating (disabled in career), Quantity (disabled in career), State (Equipped, Wireless), Custom name, Location; values: Cost, Availability, Essence, Capacity, Side, Months, Free mounts; Notes; Contents; Sell / Delete. |
| Item editor (Classic) | `item_editor.rs:54` | Click a row in a Classic item tree (`view.rs:1462`). It opens in the right side panel `SidePanel::right("item_editor")` with an "Item ✖" header (`view.rs:1467-1505`). The same fields as the Workspace inspector; Rating is a `DragValue` within `rating_range`, disabled in career. |
| Placement (Workspace drag-drop) | `workspace/inventory.rs:801-856` `ws_inventory_drops`; `table.rs:1082`, `:1177` | A refused target gets a red outline and the reason in the drag chip; releasing there does nothing. Classic has no drag-drop, only the select dialog's "Install in" combo (`select.rs:292`). |
| Row menu access | `table.rs:1369` `row_menu_button` | ⋯ button, right-click, or Shift+F10. F2 renames (`inventory.rs:643`). |
| Manual karma/nuyen entries | Workspace `workspace/career.rs:196` "Add entry"; Classic `view.rs:1782` `log_tab` | Kind (Karma/Nuyen), amount, "Reason (e.g. run payout)", `Gain` / `Spend`. |
| GM awards | `campaign_ui.rs:178` | The GM screen's award gives a `ManualExpense`. |
| History | `history_ui.rs:34` | Undo, Redo, the entries (undone ones struck through), and online the campaign log with Revert. |

### 1.2 What Chummer5a offers

| Control | Label | Files | Behaviour |
|---|---|---|---|
| `chkFreeItem` | "Free!" | `Forms/Selection Forms/SelectGear`, `SelectWeapon`, `SelectArmor`, `SelectArmorMod`, `SelectVehicle`, `SelectVehicleMod`, `SelectWeaponAccessory`; `Forms/Creation Forms/CreateWeaponMount` | `FreeCost` (SelectGear.cs:771). Creation: cost "0" (CharacterCreate.cs:5490 and others). Career: no nuyen check and **no expense entry** (CharacterCareer.cs:6689-6734). |
| `chkFree` | "Free!" | `SelectCyberware` (Designer:1052), `SelectDrug`, `SelectQuality` (Designer:227), `SelectLifestyleQuality`, `CreateImprovement` | Quality: no karma (CharacterCareer.cs:8714-8760). |
| `chkFreeBonus` | "Free!" | `SelectSpell` (Designer:372) | `FreeBonus`; outside the spell limit. |
| `chkFreeContact` | "Free Contact" | `SelectContactConnection` | |
| No Free box | | `SelectMartialArt`, `SelectPower`, `SelectComplexForm`, `SelectLifestyle` | |
| `chkBlackMarketDiscount` | "Black Market Discount (10%)" | the gear-like select forms | Only when the character has the Black Market improvement; ×0.9. |
| `nudMarkup` | "Markup: … %" | the gear-like select forms | Career only; cost × (1 + markup/100). |
| `chkShowOnlyAffordItems` | "Show Only Items I Can Afford" | the gear-like select forms | A filter; ignored while Free! is checked. |
| `chkHideOverAvailLimit` | "Hide Items Over Avail Limit ({0})" | the gear-like select forms | Creation only. A filter, **not a block**. |
| `chkLimitList` | "Show only Qualities I can take" | `SelectQuality` (Designer:499) and others | Display only. OK re-checks requirements and refuses (SelectQuality.cs:757). |
| `chkIgnoreRules` | "Ignore Character Creation Rules" | `Forms/Character Creation Forms/SelectBuildMethod.Designer.cs:101` | Per character, `<ignorerules>`. It bypasses `RequirementsMet*`, the availability filter, the whole `CheckCharacterValidity`, quality limits and attribute, skill and gear caps. It is set once, when the character is made. |
| Validation | "Message_InvalidAvail", "Message_CapacityReachedValidate" | CharacterCreate.cs:19905 `CheckCharacterValidity` | Availability and capacity are checked when the character is finished (or with Special → Confirm Validity), not when an item is picked. |
| `chkEnforceCapacity` | "Enforce Capacity limits" | `EditCharacterSettings` | A settings preset option; when off, overflow is allowed everywhere. |
| `CreateExpense` | "Karma Amount:", "Description:" ("Mission Reward"), `chkRefund`, `chkKarmaNuyenExchange` "Working for the Man/People", `chkForceCareerVisible` | `Forms/Creation Forms/CreateExpense.cs` | Manual entries; edit and Undo from `lstKarma`/`lstNuyen`. |
| `chkShowFreeKarma` / `chkShowFreeNuyen` | "Show Free Entries" | CharacterCareer.Designer:21083/21236 | Shows ledger entries of 0. |
| Custom things | "Custom Item" gear, `tsGearAllowRenameName` "Name Gear", `tsCreateSpell`, `btnCreateCustomDrug`, cyberware suites | | No generic "edit the stats" and no rating override. |

### 1.3 Summary

**Exists:** free for most nuyen items (engine only), free spells, free
lifestyle qualities, free contacts, a hidden per-character Ignore Rules
flag, manual karma/nuyen entries, custom names and notes, and the Custom
Item data record.

**Missing:**

- A free choice in the GUI for any item, quality, martial art, power,
  complex form, raise or lifestyle.
- Any marker that an item was free or forced.
- An "ignore availability" or "ignore requirements" choice: the GUI
  blocks both, with no way past.
- A force flag for placement.
- A rating override and a structured custom cost.
- A Custom Item flow.
- Override markers in history, the ledger and the GM feed.
- A campaign setting for overrides.

**Hard to find or inconsistent:**

- The only free choices are spread over the spell dialog, the lifestyle
  editor and the contact card.
- "Show unavailable" shows entries that you still cannot add.
- Enter in the Workspace catalog adds rows that the Add button refuses.
- The Workspace catalog's `Avail ≤ 12` filter is on by default
  (`catalog.rs:198`), and `Clear` in Filters does not reset it
  (`:315-323`); only the chip's × does. Players don't see that the
  over-limit items exist at all.
- No UI sets `<ignorerules>`; it is only read (`magic_ui.rs:638`).
- A free item in career leaves no ledger trace.
- Spells, powers and complex forms added through `AddItem` in career pay
  no karma (`items/magic/mod.rs:64-75`; `learn_complex_form` has no
  callers). This is a likely bug, separate from this design (open
  question 10).

---

## 2. Proposed design

### 2.1 One idea: overrides are explicit, named and logged

Every override is a named flag in the command and on the item:

| Flag | Short name in UI | Meaning |
|---|---|---|
| `free` | **Free** | Pays nothing (nuyen or karma). |
| `found` | **Found in play** | `free` plus `ignore_avail`, plus the reason "found in play". This is the common case and gets its own toggle. |
| `ignore_avail` | **Ignore availability** | Over the creation limit, or restricted/forbidden, is accepted and not reported as an issue. |
| `ignore_reqs` | **Ignore requirements** | Required/forbidden lists, slot limits and metatype/magic requirements are skipped. |
| `force_place` | **Force placement** | Put it there anyway (wrong kind, category, mount, or over capacity). |
| `cost` | **Custom cost** | The price paid, in place of the data cost. |
| `rating_over` | **Rating above maximum** | The rating may exceed the data maximum. |

There is no global "cheat mode". A player chooses each override for a
purchase, sees it named on the button ("Add Ares Predator V · free"), and
sees it again on the item afterwards.

The one exception is the **Found in play** *mode*: a toggle that stays on
while the player enters a batch of loot (2.2). It is shown as a coloured
bar, so it is never on by accident.

### 2.2 Workspace layout: the catalog (purchase panel)

Changes:

- **Catalog header:** a new toggle chip `Found in play` between the count
  and `Filters`. When it is on, an amber strip appears under the header
  ("Found in play: everything you add is free and ignores availability.
  [Turn off]"). The mode is kept per open character, is not saved, and
  turns off when the tab closes.
- **Results rows:** rows that are dimmed for a reason (requirements,
  availability) still have no `+`. They do get a `⋯` with "Add anyway…",
  which selects the row and opens the overrides section in the inspector.
- **Footer hints:** in Found in play mode, `Enter` reads "add free".

```text
┌ Catalog  Cyberware & Bioware ─────── 101 of 110 [Found in play] [Filters 1] « ┐
│ ▌Found in play: added items are free and ignore availability.   [Turn off]  │  ← new strip (amber), only in the mode
│ [🔍 Search 110 cyberware / bioware        ] [All|Cyber|Bio]                   │  ws_catalog_search (unchanged)
│ (Avail ≤ 12 ×)  Grade: Standard   9 hidden by filters                         │  ws_catalog_chips (unchanged)
│ NAME                              R    ESS   AVAIL     COST                   │
│ Taste Booster                     1   0.20      3   3,000¥  [+]               │
│ Cranial Bomb (dimmed)             –   0.00    12F  10,000¥  [⋯] → Add anyway… │  ← new ⋯ on dimmed rows
├───────────────────────────────────────────────────────────────────────────────┤
│ Enter add free · Shift+Enter add, keep focus · Tab to the inventory           │  footer hints (text changes in the mode)
└───────────────────────────────────────────────────────────────────────────────┘
```

**Catalog inspector (`ws_catalog_inspector`):** a new collapsible
section, **Overrides**, between **RULES** and the Add button. It is
closed by default and opens by itself when a rule blocks the purchase.

- The section header shows how many overrides are on: "Overrides 2".
- "Ignore availability" is shown only in creation and only when the item
  is over the limit or R/F. "Ignore requirements" is shown only when a
  requirement fails. "Force placement" is shown only when the target
  refuses the item. The checkboxes in the section can be greyed out and
  explained by the campaign setting (3.3).
- With an override on, the matching RULES line keeps its warning but
  gains "(overridden)", and Add is enabled.
- The Add button's label names the overrides: `+ Add Cranial Bomb ·
  free, ignores avail`. The note under it says what will happen: "Career:
  no nuyen is spent; the ledger notes it as found in play."
- **"Custom cost"** is a number box. It replaces "Free" when it is
  filled; Free is the same as a custom cost of 0, but is kept as its own
  word because it is what players look for.

```text
┌ Item ──────────────────────────────────────── × ⧉ ┐
│ Cranial Bomb - Kink Bomb              📖 SR5 p.461 │
│ (Cyberware) (Headware) (Forbidden)                 │  tags (unchanged)
│ Rating     [− 1 +]                                 │
│ Grade      [Standard           ▾]                  │
│ Essence    5.42 → 5.42  ▓▓▓▓▓▓▓▓▓▓▓░               │
│ Cost                                     10,000¥   │  ← shows "0¥ (was 10,000¥)" when free
│ Availability                         12F  over 12  │
│ Nuyen left after                          −4,495¥  │
│ RULES                                              │
│ ⚠ Availability 12F is above the limit of 12.       │  ← "(overridden)" appended when ticked
│ ⚠ Not enough nuyen.                                │
│ ▾ Overrides 2                                      │  ← NEW section (opens by itself when blocked)
│   [x] Free (found in play)                         │
│   [ ] Custom cost   [        ]¥                    │
│   [x] Ignore availability                          │
│   [ ] Ignore requirements      (only when failing) │
│   [ ] Force placement          (only when refused) │
│   [ ] Rating above maximum     (only rated items)  │
│ [+ Add Cranial Bomb · free, ignores avail]  [⚖]    │  Add button (label names overrides)
│ Logged as an override. The GM sees it in the feed. │  ← note (replaces "Remove it later…")
│ ▸ Data                                             │
└────────────────────────────────────────────────────┘
```

In Found in play mode, the Free and Ignore availability boxes are ticked
for every record, with the reason "found in play".

**Placement refusal (drop or "add into").** Today a refused catalog
add writes the `misfit_text` to the status bar (`catalog.rs:1544`), and a
drop on a refused target does nothing (`table.rs:1082`). New:

- Releasing a drag on a refused target (red outline) opens a small popup
  at the drop point: "Can't go there: {misfit_text}" with `Put it there
  anyway` and `Cancel`. `Put it there anyway` sends the same command
  (`AddItem` with `force_place`, or `MoveItem { force: true }`).
- The catalog target bar (`ws_catalog_target_bar`), which already shows
  the misfit in amber, gets a `Force` link after the misfit text. It
  ticks "Force placement" in the Overrides section.
- Classic: the select dialog's "Install in" combo also lists refused
  parents, greyed with their reason. Picking one ticks "Force placement".

### 2.3 Workspace layout: the item row menu (⋯)

Today the ⋯ button (`row_menu_button`, `table.rs:1369`; also right-click
and Shift+F10) lists the row's actions. A "More" submenu exists only in
career. New: both modes get a "More" group, with the new entries first:

```text
hover:     … [✎] [🗑 or ¤] [⋯]          (unchanged; drawn right-to-left)

⋯ menu (new entries above the line, existing ones below)
┌──────────────────────────────────────────┐
│ Mark as free…           (career: refund?)│  → small confirm (see 3.4)
│ Set cost…                                │  → number dialog
│ Override rating…                         │  → stepper without maximum
│ Clear overrides          (only if any)   │
├──────────────────────────────────────────┤
│ Move to No location     (existing moves) │
│ Remove without refund   (career)         │
│ Open the sourcebook at this page         │
└──────────────────────────────────────────┘
```

A row with any override gets a small amber tag after the name, like the
existing `Added` tag in the Installed table: `Free`, `Found`, `Forced`,
`Custom cost`. The tag's tooltip lists the overrides and who made them.

### 2.4 Workspace layout: the item inspector (`inspector.rs`)

```text
┌ Item ──────────────────────────────── × ⧉ ┐
│ Ares Predator V            📖 SR5 p. 424  │
│ (Found in play)                           │  ← NEW tag row when the item has overrides
│ Rating        [− 2 +]  Override…          │  ← "Override…" link only with rating_over allowed
│ Quantity      [− 1 +]                     │
│ State         [x] Equipped  [x] Wireless  │
│ Custom name   [                     ]     │
│ Location      [None              ▾]       │
│ Cost          0¥ (data: 725¥)    ✎        │  ← ✎ opens "Set cost…"
│ Availability  5R  ignored                 │  ← "ignored" when ignore_avail
│ OVERRIDES                                 │  ← NEW overline, only when any
│  Free (found in play), by Anna, 14 Oct    │
│  Ignore availability                      │
│  [Clear overrides]                        │
│ NOTES                                     │
│ CONTENTS                                  │
│ [Sell…] [Delete]                          │
└───────────────────────────────────────────┘
```

### 2.5 Workspace layout: other pages

- **Add Quality…** (Attributes & Qualities, `build.rs:158`) and the other
  select-dialog users: see the Classic dialog (2.6). The Workspace uses
  the same `SelectDialog`.
- **Spells & Spirits:** the spell dialog's existing `Free` checkbox
  (`magic_ui.rs:328`) stays. The other overrides go in the same row
  ("Ignore requirements").
- **Career advances** (Attributes, Skills: the `+1 · 15 k` buttons): in
  Found in play mode, these read `+1 · free`, and the bar reads "Found in
  play: raises are free". There is no other free-raise control; the mode
  is the one way to raise for free. (Chummer has none either; players use
  manual karma entries.)
- **Karma & Nuyen** (`workspace/career.rs:196`, "Add entry"): unchanged.
  It stays the way to record rewards.

### 2.6 Classic layout

**Select dialog (`select.rs`).** The overrides go in one row above the
`Add` / `Cancel` buttons, in Chummer's style (Chummer puts `Free!` and
`Black Market Discount` at the bottom left).

```text
┌ Add Gear ─────────────────────────────────────────────────────────────┐
│ [Search          ] [All categories ▾] [x] Show unavailable            │  top row (unchanged)
│ 412 shown                                                             │
│ ┌ list ───────────────────────┐  Cranial Bomb                         │
│ │ ...                         │  📖 SR5 p. 461                        │
│ │ Cranial Bomb (greyed)       │  ⚠ availability 12F is above 12       │  dialog::warning (unchanged)
│ │ ...                         │  Rating [1]  Grade [Standard ▾]       │  purchase_options (unchanged)
│ └─────────────────────────────┘  Availability 12F   Cost 10,000¥      │
│ ───────────────────────────────────────────────────────────────────── │
│ [ ] Free!  [ ] Found in play  [ ] Ignore availability                 │  ← NEW override row
│ [ ] Ignore requirements  Cost: [      ]¥                              │     (wraps; entries only when they apply)
│                                       [Add · free] [Cancel]           │  dialog::buttons (label names overrides)
└───────────────────────────────────────────────────────────────────────┘
```

- `Free!` uses Chummer's label (`Checkbox_Free`), so translations exist.
- With "Show unavailable" on, a greyed entry can be selected. Add stays
  disabled until the matching override is ticked; the disabled tooltip
  says which one ("Tick Ignore availability to add it anyway").

**Item editor side panel (`item_editor.rs`, `SidePanel::right("item_editor")`).**
The same additions as the Workspace inspector, in the editor's grid: `Cost` gets a `Set…` button,
`Rating` gets `Override…`, and an `Overrides` group (with `Clear`) sits
above `Notes`.

**Item trees (Classic tables).** Rows with overrides get a small amber
`Free` / `Forced` text in the Name cell. Clicking the row opens the
item editor panel, as today.

**Special menu.** A new menu entry, `Special → Found in Play Mode`, with
a check mark. The Classic status bar shows an amber `Found in play` label
while it is on.

**Karma & Nuyen tab (`view.rs` `log_tab`).** It gains a `Show free
entries` checkbox (Chummer's `chkShowFreeKarma`) for the 0-amount entries
of 3.4.

### 2.7 Custom item

- **Workspace:** the catalog's empty card ("Nothing matches.") gets a
  second button, `Add a custom item…`. The same entry is also at the
  bottom of the Gear catalog's kind segments.
- **Classic:** the select dialog has the same button in its bottom row
  (Gear, Weapons, Armor only).
- Both open a small form: Name, Category (free text with suggestions),
  Rating, Availability, Cost, Notes. It adds Chummer's "Custom Item" gear
  record (`gear.xml:19983`) with these values (custom name =
  `gearname`), so a `.chum5` export still opens in Chummer.
- Weapons and armor with custom stats are out of scope for the first
  version (open question 6).

---

## 3. Rules

### 3.1 How overrides are recorded

**In the command.** `Purchase` gains one field:

```rust
#[serde(default)]
pub overrides: Overrides,

pub struct Overrides {
    pub found: bool,          // found in play (implies free + ignore_avail)
    pub ignore_avail: bool,
    pub ignore_reqs: bool,
    pub force_place: bool,
    pub cost: Option<f64>,    // custom cost; Some(0.0) == free
    pub rating_over: bool,
    pub note: String,         // optional reason, e.g. "from the Tamanous van"
}
```

- `Purchase.free` stays, for compatibility and for internal free adds
  (built-in mods, `addgear` bonuses). A user override sets
  `overrides.cost = Some(0.0)` (or `found`), so internal frees are never
  shown as player overrides.
- New and changed commands:
  - `MoveItem { item, to, force: bool }`
  - `SetItemRating { guid, rating, force: bool }`
  - `SetItemCost { guid, cost: Option<f64> }`. `None` restores the data
    cost. This replaces the `SetItemText{field:"cost"}` loophole, which
    should then refuse cost, avail and rating.
  - `MarkFree { guid, refund: bool }`
  - `ClearOverrides { guid }`
  - `AddCustomItem { name, category, rating, avail, cost, notes }`
  - Quality, martial art, power and complex form adds read `overrides`
    too.
  - `RaiseAttribute`, `RaiseSkill`, `RaiseSkillGroup` and
    `LearnKnowledgeSkill` gain `free: bool`.

**On the item.** One child element, saved with the character:

```xml
<chummerrs_override found="True" ignoreavail="True" by="Anna" at="2026-10-14T19:03:00Z" datacost="725">from the Tamanous van</chummerrs_override>
```

- `datacost` keeps the original cost, so "Clear overrides" can restore
  it and the inspector can show "0¥ (data: 725¥)".
- `<cost>` stays `"0"` (or the custom value), as Chummer writes it, so
  every total and a `.chum5` export are right with no special code.
- Chummer5a ignores and drops the unknown element. That is acceptable:
  the item is still free in Chummer. Record it in `docs/deviations.md`.
- Creation issues (`issues.rs:494`) skip `AvailabilityTooHigh` for
  items with `ignoreavail`, and `OverCapacity` for containers holding a
  `forceplace` item. Skipped items instead get an Info issue, "Cranial
  Bomb: availability ignored (override)", so the build summary still
  shows them.

**In history.** `describe.rs` appends the overrides in a fixed form, so
the local History, the campaign feed and the ledger use the same words:

- "Added Ares Predator V **(found in play: free)**"
- "Added Cranial Bomb **(override: free, ignores availability)**"
- "Moved Smartgun System into Ares Predator V **(override: forced
  placement)**"
- "Set cost of Ares Predator V to 400¥ **(override)**"
- "Raised Pistols to 5 **(found in play: free)**"

### 3.2 How the GM sees them in online campaigns

- `FeedEntry` gains `overrides: Vec<String>`, filled by `feed::from_entry`
  from the envelope's command (no extra network message; replicas build
  the same thing).
- **GM screen feed:** an amber chip on the line (`Found in play`,
  `Override`). The chip's tooltip lists the flags and the player's note.
  A filter toggle, `Overrides only`, sits at the top of the feed. Revert
  works as for any line.
- **Player's History:** the same chip, so the player sees what the GM
  sees.
- This follows `docs/online-design.md` §5 ("whether it was an override
  … which the GM can look at").

### 3.3 Campaign setting: who may use overrides

A new `OverrideSettings` next to `RollSettings` in `Campaign`
(`campaign.rs:403`). The authority sends it to players the same way.
The GM sets it in the GM screen, in a collapsing header `Overrides`
under `Dice rolls` (`gm_screen/online.rs:888`):

| Value | Player experience |
|---|---|
| **Allowed, logged** (proposed default) | Players use every override. Each one is marked in the feed. This matches the project rule "Logging, not anti-cheat … the GM can revert any change". |
| **Found in play only** | Players may use Free and Found in play. Ignore requirements, Force placement, custom cost and rating above maximum are greyed out, with "Your GM keeps these for themself". |
| **GM only** | Every override control is greyed out for players. The authority refuses a player command that carries overrides, with the reason "Overrides are set to GM only in this campaign". |

- "Ask the GM" (a pending queue that the GM approves) is a bigger change:
  commands would wait before they apply. It is listed as open question 2,
  not proposed for the first version.
- Local characters (no campaign) always allow overrides. The
  per-character `<ignorerules>` keeps its Chummer meaning and is not
  reused.

### 3.4 Creation vs career, and undo

**Creation:**

- Free: the item's cost is 0, so it does not use starting nuyen or BP.
  Free qualities do not count toward the positive/negative quality karma
  limits. Chummer counts a free quality as BP 0 (`SetBPAsync(0)`,
  CharacterCreate.cs:6306), so this matches.
- Ignore availability: no `AvailabilityTooHigh` error (only the Info
  line).
- Removing the item is free, as now.

**Career:**

- Free and found: no nuyen or karma is spent. The engine writes a
  **0-amount ledger entry**, "Found in play: Ares Predator V", with the
  item as the undo object. The purchase is then in the ledger, the
  ledger's Undo removes it, and Chummer's `chkShowFreeKarma` /
  `chkShowFreeNuyen` idea ("Show free entries") hides these entries by
  default. Chummer itself writes no entry; this is a deliberate
  difference, to be listed in `docs/deviations.md`.
- Custom cost: the ledger entry uses the custom amount, and its reason
  ends with "(custom cost)".
- `Mark as free…` on an item bought earlier asks one question: `Refund
  725¥` (writes a +725¥ entry, "Marked free: …") or `Don't refund` (only
  the marker changes). This is for "I forgot to tick Free".
- Selling a free item uses its stored cost, which is 0. Chummer does the
  same. The Sell menu shows "0¥" so nobody is surprised (open question 5).

**Undo:**

- Local documents: an override is an ordinary command, so Ctrl+Z takes
  it back exactly, ledger entry included. Nothing new is needed.
- Online: unchanged. There is no undo; the GM reverts in the feed, and
  the player reverts from History where the existing rules allow it.
- `Clear overrides` is a command of its own (undoable). It does not
  re-check the rules: an item kept by force stays where it is, and the
  RULES lines and issues come back.

---

## 4. Keyboard and discoverability

- **No hidden modifiers.** No Alt-click or Ctrl+Enter override. Every
  override is a visible checkbox, and the Add button names what it will
  do.
- **The block explains the way out.** When Add is disabled, its tooltip
  names the override that would allow it ("Tick Ignore availability to
  add it anyway"), and the Overrides section is already open.
- **Found in play is a visible mode.**
  - It is turned on from the catalog header chip, from `Special → Found
    in Play Mode` (Classic) and from the command palette (Ctrl+K, "Found
    in play mode").
  - While it is on, there is an amber strip in the catalog, an amber
    label in the status bar, and `Enter add free` in the hints.
  - It turns off when the tab closes. Proposed shortcut, shown in the
    palette: **Ctrl+Shift+F**. Nothing else gets a shortcut.
- **Enter follows the button.** Enter, double-click, the row `+`, the
  details strip's Add and a drag to the inventory all apply exactly what
  the inspector's Add button would. A blocked record
  is not added by Enter; Enter opens the Overrides section and shows the
  status message "Blocked: tick an override to add it". This closes the
  back door noted in 1.1.
- **The Guide** (`guide_ui.rs`) gets one page: "Found loot or a gift?
  Turn on Found in play".

---

## 5. Implementation plan

Each step is a commit with its tests. Engine first, then the GUI.

1. **Engine: `Overrides` and the item marker.**
   - Files: `chummer-core/src/items/mod.rs` (`Overrides`, `Purchase.overrides`);
     a new `items/overrides.rs` (read/write `<chummerrs_override>`,
     `datacost`); `items/gear.rs`, `weapon.rs`, `armor.rs`,
     `cyberware.rs`, `vehicle.rs`, `drug.rs`, `lifestyle.rs` (one call
     `overrides::apply(el, &p)` in place of the per-kind `if p.free`);
     `command.rs` `examples()`.
   - Tests: `chummer-core/tests/overrides.rs`. For every nuyen kind:
     free gives cost 0, the marker, and `datacost`. A save/load
     round-trip keeps the marker. `.chum5` export keeps cost 0.
2. **Engine: karma kinds.**
   - Files: `career/karma.rs` (`add_quality`), `career/magic.rs` (martial
     arts, critter powers), `chargen.rs:604` (creation qualities),
     `items/magic/mod.rs` (spells, powers and complex forms through
     `AddItem`); the raise commands gain `free`.
   - Tests: a free quality costs 0 karma in career and creation, and is
     outside the quality limits. A free raise costs nothing and writes
     the 0 entry.
3. **Engine: availability, requirements and placement.**
   - Files: `chargen/issues.rs` (skip and Info line); `items/place.rs`
     and `command/run.rs` (`MoveItem.force`, `AddItem` with `parent`
     calls `place::check` unless `force_place`); `items/edit.rs`
     (`SetItemRating.force`).
   - Tests: an over-limit item with `ignore_avail` has no error. A forced
     move into a full container succeeds and is marked. Without force,
     `AddItem` into a full container is refused (a behaviour change; see
     open question 8).
4. **Engine: the new commands.**
   - Files: `command.rs`, `command/run.rs`, `command/describe.rs`.
     Commands: `SetItemCost`, `MarkFree`, `ClearOverrides`,
     `AddCustomItem`, and `SetItemText` refusing cost/avail/rating.
   - Tests: an undo test per command (`command/session.rs` tests). The
     description strings of 3.1. `Command::examples()` covers the new
     variants (the fuzz and robustness suites pick them up).
5. **Engine: career ledger.**
   - Files: `command/run.rs:521-529` (0-amount entry), `career/nuyen.rs`,
     `career/ledger.rs`.
   - Tests: a free career item writes one 0 entry, and `UndoExpense`
     removes the item. `MarkFree{refund:true}` refunds the right amount.
6. **Sync: feed and campaign setting.**
   - Files: `chummer-sync/src/msg.rs` (`FeedEntry.overrides`), `feed.rs`
     (`from_entry`), `chummer-core/src/campaign.rs` (`OverrideSettings`,
     serde default), `chummer-sync/src/authority.rs` (GM only: refuse
     player commands with overrides; GM commands always pass),
     `hosted.rs` (pass the setting on, as for `set_roll_settings`).
   - Tests: `chummer-sync` tests show an override marked in the feed on
     both the authority and a replica. GM only refuses a player and
     accepts the GM. `docs/file-format.md` documents `override_settings`.
7. **GUI Workspace: catalog.**
   - Files: `workspace/catalog.rs` (header chip, strip, Overrides
     section in `ws_catalog_inspector`, button label, Enter follows the
     button, `⋯ Add anyway…` on dimmed rows), `workspace/shell.rs` or
     `palette.rs` (palette entry, Ctrl+Shift+F).
   - Tests: `ui_tests.rs` (egui_kittest, as the existing ones). Enter on
     a blocked row adds nothing. With Ignore availability ticked it adds,
     and the item has the marker.
8. **GUI Workspace: rows and inspector.**
   - Files: `workspace/inventory.rs:448` (`⋯` in creation, new entries,
     tags), `workspace/inspector.rs` (Cost ✎, Override…, Overrides
     block), `workspace/table.rs` (amber tag tone if a new one is needed).
   - Tests: a ui_test per menu entry.
9. **GUI Classic.**
   - Files: `select.rs` (override row, Add label, the disabled
     tooltip), `item_editor.rs`, `view.rs` (Special menu entry, status
     bar label, `Show free entries` in `log_tab`), and the Classic item
     tree name tag.
   - Tests: ui_tests for the select dialog with Show unavailable plus an
     override.
10. **GUI: GM screen and History.**
    - Files: `gm_screen/online.rs` (`Overrides` setting header),
      `workspace/gm.rs` and `gm_screen.rs` (feed chip, "Overrides only"
      filter), `history_ui.rs` (chip).
    - Tests: a ui_test with a hosted campaign fixture, if the existing
      GM tests allow it; otherwise the sync-level test of step 6.
11. **Custom item form** (both layouts), `docs/deviations.md` entries
    (the 0 ledger entries, the marker element), README feature list,
    and labels through `lang.tr` (reuse Chummer's `Checkbox_Free` "Free!"
    where it fits).

Steps 1–6 can merge without any GUI change. The GUI steps depend on
them, but not on each other.

---

## 6. Open questions for the owner

1. **Default campaign setting.** Is "Allowed, logged" the right default,
   or should new campaigns start with "Found in play only"?
2. **GM approval queue.** Do you want a real "Ask the GM" mode, where an
   override waits until the GM accepts it? It means pending commands in
   the authority, which the current design avoids. The proposal is no,
   for now.
3. **Ledger entries for free career purchases.** The proposal writes a
   0-amount entry ("Found in play: …"), so the ledger shows the purchase
   and can undo it. Chummer writes nothing. Keep the 0 entry?
4. **"Found in play" = free + ignore availability.** Should it also skip
   requirements (a cyberware piece found on a body that needs a parent),
   or stay limited to those two?
5. **Selling found items.** Sell uses the stored cost, so a free item
   sells for 0¥, as in Chummer. Should found items sell at the data cost
   (`datacost`) instead? Fencing loot is common at the table.
6. **Custom item scope.** Is gear only (Chummer's "Custom Item" record)
   enough for now, or do you also want custom weapons and armor with
   their own stats (damage, AP, armor value)?
7. **Ctrl+Shift+F** for Found in play mode: OK, or no shortcut at all?
8. **AddItem into a parent is not checked by the engine today.** Step 3
   makes the engine refuse a misfit unless `force_place` is set. This
   changes what a CLI or an old campaign log can do. OK?
9. **The Enter back door** in the Workspace catalog (Enter adds records
   that Add refuses). Close it as part of this work (proposed), or fix it
   on its own first?
10. **Spells, powers and complex forms added through `AddItem` in career
    pay no karma** (`items/magic/mod.rs:64-75`; `learn_complex_form` has
    no callers). This looks like a bug outside this design. Fix it
    separately, or here in step 2?
11. **Per-character Ignore Rules.** Chummer's "Ignore Character Creation
    Rules" (`<ignorerules>`) has no UI in chummer-rs. Should the
    Workspace offer it at character creation as Chummer does, or leave
    it to critters and use the overrides above instead?
