# chummer-rs

A Rust rewrite of [Chummer5a](https://github.com/chummer5a/chummer5a), the
Shadowrun 5th Edition character manager. It runs natively on Linux,
Windows and macOS: no Wine, no .NET, no Internet Explorer.

> **Derived from Chummer5a.** chummer-rs is an independent port of
> [chummer5a/chummer5a](https://github.com/chummer5a/chummer5a)
> (GPL-3.0). The rules logic was ported from Chummer5a's C# source. The
> game data, translations, custom data, character sheets and export
> templates in `resources/` are copied from Chummer5a unchanged. Its test
> characters are used as test fixtures. All credit for the original
> program and its data goes to the Chummer5a authors. chummer-rs is not
> affiliated with or endorsed by the Chummer5a project.

chummer-rs reads and writes the same `.chum5` (and compressed `.chum5lz`)
files and uses Chummer5a's own game data, custom data and character
sheets. Characters move between the two programs: files created by chummer-rs load in Chummer5a 5.226 without
warnings (see [docs/interop.md](docs/interop.md)).

## Download

Prebuilt packages for Linux, Windows and macOS are on the
[Releases](../../releases) page. Unpack and run `chummer-rs` (`chummer-rs.exe`
on Windows). Keep the `resources` folder next to the program.

On macOS, unzip and move `chummer-rs.app` to Applications. The app is
not notarised by Apple, so the first time right-click it and choose
Open (or run `xattr -dr com.apple.quarantine /Applications/chummer-rs.app`).
`chummer-cli` is inside the bundle, in `chummer-rs.app/Contents/MacOS/`.

Character sheets need `xsltproc`:
- Linux: it is in the `libxslt` package.
- macOS: it comes with the system.
- Windows: put `xsltproc.exe` on your PATH.

## Build from source

You need a Rust toolchain (1.85 or newer) and, for character sheets,
`xsltproc` (package `libxslt`).

```bash
./install.sh
```

This installs to `~/.local`:

- `chummer-rs`: the desktop application.
- `chummer-cli`: the command-line tool.
- The game data, in `~/.local/share/chummer-rs`.
- A desktop entry, so `.chum5` files open with chummer-rs.

The GUI is called `chummer-rs` so that it never replaces a `chummer`
launcher you may have for Chummer5a under Wine.

To run from the source tree:

```bash
cargo run --release -p chummer-gui -- path/to/character.chum5
```

## Features

### Characters

**Creation.** File → New character (Ctrl+N) supports these build methods:

| Build method | How it works |
|---|---|
| Priority | Five priorities, each letter used once. |
| Sum-to-Ten | Priority values must add up to the preset's total. |
| Point Buy | Everything is bought with karma. |
| Life Modules | Karma build plus life modules added by stage. |

The wizard covers:
- Metatype and metavariant.
- Magic or resonance talent, with its free skills.

A Creation panel shows what is left of each budget:
- Karma.
- Attribute and special attribute points.
- Skill and skill group points.
- Knowledge and contact points.
- Spells and power points.
- Nuyen, including karma converted to nuyen.
- Quality limits.

**Creation issues.** While a character is in creation, chummer-rs checks
it all the time with Chummer's finish-creation rules, so the problems do
not wait for a pop-up at the end:
- Errors block Finish creation: overspent attribute, special, skill and
  skill group points, karma or nuyen; too many attributes at their
  maximum; more than one specialization per skill; quality limits; adept
  power points; no tradition for a magician; Essence at 0; too many native
  languages, martial arts or techniques; a contact worth more than 7
  points; items above the allowed Availability (Restricted Gear is
  counted); banned ware grades; items over their capacity.
- Warnings are reminders: unspent points of any pool, free spells and
  power points left, more karma or nuyen than carries over, a mentor
  spirit not chosen yet, no technomancer stream.
- Each tab with issues has a warning badge with the count (Classic: a
  yellow warning sign; Graphite: a coloured dot). The focused tab shows
  its first issue in one line at the top, with "+n" for the rest; click
  one to go to the row or item. ✖ hides the line until something
  changes. Rows with a problem have a warning mark.
- The Karma Summary lists every issue, and Finish creation shows the
  full list before it switches to career mode, carrying over at most 7
  karma and 5,000¥.

**Guided creation.** For new players, the character's own tabs (Classic)
or sidebar (Workspace) become the build checklist; there is no extra
bar. Turn it on in the New Character wizard (checked by default) or with
View → Guided creation, and off with ✕ (saved in `gui.ini`). A new
character opens on its first real step, e.g. "Attributes · 24 Attribute
points left to spend · Next: Special Attributes →".

- **Checklist**: each tab or sidebar entry with steps shows a tick when
  they are done (visited, no errors or warnings left), an empty ring when
  not visited yet, or the usual issue count. The Workspace's Build
  heading counts the steps done ("Build · 6/11") and ends with **Review
  & Finish**.
- **One hint line** at the top of the page, for the step the page is
  about: its name, the first thing left (click to go there) and "+n" for
  the rest, and **Next** to the next unfinished step. The step follows
  wherever you go (sidebar, tabs, issue links), so nothing is locked.
- **Rules on demand**: ⓘ opens the step's rule in plain words with a 📖
  link to the rulebook page (SR5 or Run Faster); hovering the name shows
  the progress.
- **Review & Finish** lists everything Finish creation would, grouped by
  tab, with Finish creation (Classic: on the Common tab, reached with
  Next). Steps without checks (cyberware, vehicles) count as done once
  visited.

The steps follow the build method:

| Build method | Steps |
|---|---|
| Priority, Sum-to-Ten | Concept & metatype → attributes → special attributes → qualities → active skills → knowledge skills → spells / adept powers / complex forms (if the character has them) → cyberware → street gear → vehicles → contacts → character info → review & finish |
| Point Buy | Concept & metatype → qualities (magic and resonance are qualities here) → attributes → special attributes → skills → … as above |
| Life Modules | Concept & metatype → life modules → qualities → attributes → … as above |

The current step and the steps visited are remembered per file in
`~/.config/chummer-rs/guide.ini`, not in the .chum5.

| | Start | Mid-way | Review |
|---|---|---|---|
| Workspace, dark | ![Guided creation start, Workspace dark](docs/screenshots/guided-creation-dark-start.png) | ![Guided creation mid-way, Workspace dark](docs/screenshots/guided-creation-dark-mid.png) | ![Review & Finish, Workspace dark](docs/screenshots/guided-creation-dark-review.png) |
| Workspace, light | ![Guided creation start, Workspace light](docs/screenshots/guided-creation-light-start.png) | ![Guided creation mid-way, Workspace light](docs/screenshots/guided-creation-light-mid.png) | ![Review & Finish, Workspace light](docs/screenshots/guided-creation-light-review.png) |
| Classic | ![Guided creation start, Classic](docs/screenshots/guided-creation-classic-start.png) | ![Guided creation mid-way, Classic](docs/screenshots/guided-creation-classic-mid.png) | ![Review & Finish, Classic](docs/screenshots/guided-creation-classic-review.png) |

**Career mode.**
- Raise attributes, skills, skill groups and knowledge skills for karma at Chummer's costs.
- Buy specializations, qualities and spells.
- Buy off negative qualities.
- Initiate or submerge (group, ordeal and schooling discounts). A mystic adept with the second-MAG house rule is also limited by MAGAdept.
- Learn martial arts and their techniques.
- Learn metamagics and echoes at a chosen grade (the lowest grade without one is preselected). The first one at a grade is free; each further one costs karma.
- Buy critter powers.
- Buy A.I. programs and Advanced Programs. Undo refunds the karma and removes the program (Chummer keeps it); it is refused while another program the character has requires it.
- Bind foci, up to MAG foci and MAG × 5 total force.
- Fetter a spirit (Force × 3 karma) or a sprite (Force karma). A fettered spirit lowers MAG by 1 and gains Banishing Resistance (Street Grimoire p. 192; Chummer does not add it). Undoing the fettering expense releases the spirit again.
- Join or leave a magical group, and quicken spells.
- Spend and regain Edge, burn a point of Edge and burn street cred.

Every purchase is written to the karma/nuyen ledger. The Karma & Nuyen tab:
- Lists the ledger with Undo.
- Takes manual income and expenses.
- Shows career karma, street cred, notoriety and public awareness.

The calendar tracks in-game weeks.

**At the table** (career mode), as in Chummer5a's career form:
- Edge boxes in the sidebar: click a box to mark Edge spent up to it, or reset it all for a new session (`<edgeused>`).
- Weapon ammunition: clips per slot (accessories add slots), reload from
  the ammunition the character or vehicle carries (split off the stack,
  spare clips and speed loaders, external sources), unload back onto the
  stack, and fire single shots, bursts, full bursts and suppressive fire.
  Weapons with charges reload to their capacity.
- Devices (commlinks, decks, cyberware, vehicles) show their matrix
  attributes and a matrix condition monitor, and one can be the active
  commlink, which sets matrix initiative. An A.I. can make a device its
  home node (Program Limit 2 when Depth is above the device rating).
- Vehicles and drones have a damage track.

**Rules.** The following are computed:
- Attributes, with improvement stacking and cyberlimbs.
- Essence and essence loss, in creation and career mode (in career mode, loss beyond the minimum burns MAG/RES/DEP karma levels, as in Chummer).
- Initiative, condition monitors and limits.
- Derived pools and armor.
- Skill dice pools.
- Weapon stats: DV with STR, AP, accuracy, recoil, ranges and dice pool.
- Vehicle and drone stats after mods.
- Drain and fading.
- Adept power points.

### Equipment and magic

You can add items from the game data with:
- Rating, quantity and grade choices.
- Availability checked against the creation limit.
- A cost preview.

Supported kinds:
- Gear, with nested gear.
- Cyberware and bioware, with grades, cyberlimbs and subsystems.
- Armor and armor mods.
- Weapons, with accessories and underbarrels.
- Vehicles and drones, with mods and weapon mounts.
- Lifestyles, with lifestyle qualities.
- Custom drugs, priced by their grade.
- Spells (limited, extended, alchemical), adept powers, complex forms, spirits and sprites.
- Metamagics and echoes, mentor spirits with their choices, martial arts and techniques, and critter powers.
- A.I.s: programs and Advanced Programs (creation karma with the free program slots, requirements), Edge maximum equal to Depth, a Core or vehicle track and the home node's Matrix track, limits, matrix initiative and spell defense from the home node.

Click an item to edit its rating, quantity, equipped and wireless state,
custom name, location and notes, add things inside it, or sell it.

The "bonus" of every quality, piece of ware, power and item applies, just
as in Chummer5a. Every bonus type in the game data is handled.

**Custom improvements.** On the Improvements tab (career mode; in
creation, as in Chummer, the tab only shows once a character has custom
improvements), a GM can add one-off modifiers of any type in `improvements.xml`
(Add Improvement), for example +1 Agility or an extra condition monitor
box. They can be edited, deleted, turned on and off, sorted into groups
(with Enable All / Disable All), and given notes. They are saved in
Chummer5a's format (`<custom>`, `<customname>`, `<customgroup>`,
`<improvementgroups>`). The tab also lists every automatic improvement.

Item requirements (`<required>`/`<forbidden>`) are checked.

**Relationships.** The Relationships tab has Chummer's Contacts, Enemies
and Pets & Cohorts sub-tabs. Contacts and enemies have name, location,
archetype, connection, loyalty, the Free/Group/Blackmail/Family flags and
an expandable stat block (type, metatype, gender, age, personal life,
preferred payment, hobbies/vice) with `contacts.xml`'s lists; pets have a
name and a `critters.xml` metatype. Those fields take free text, with the
list behind a chevron inside the field; typing filters the list. Contacts can also be added from a
Chummer contacts XML file (Add from File). Any entry can be linked to
another `.chum5` or `.chum5lz` (Attach Character): its name, metatype,
gender, age and mugshot then come from that file, on screen and on
printed sheets, and Open Character opens it in a new tab. The link is
saved as Chummer saves it (`<file>` as picked, `<relative>` from the
program directory); a link made on Windows also works when the linked
file sits next to the character's own save. A missing linked file shows
a warning and nothing else.

To reorder entries, drag a row by its ☰ handle or use the ⏶/⏷ buttons.
The order is saved as the order of the `<contact>` elements, which is
the order Chummer loads them in. The notes button opens a notes dialog
with Chummer's notes colour (Select Colour). The colour is saved as
Chummer saves it (`ColorTranslator.ToHtml`: a colour name such as
`Chocolate` or `#RRGGBB`), the notes text is shown in it, and in dark
themes it is shown the way Chummer's dark mode shows it. A contact's
`<colour>` tints its row, as Chummer paints the contact control with it.

**Compressed saves.** `.chum5lz` files (Chummer's LZMA-compressed saves)
open, save, show in the roster and recent lists, and work as linked
contacts and in every `chummer-cli` command. Save As keeps the format of
the open file and offers both. The file is the `.chum5` XML in the
`.lzma` format with Chummer's default "Balanced" settings (16 MiB
dictionary, lc 3, lp 0, pb 2, end marker). Files saved with any of
Chummer's compression levels open. Files written by Chummer
5.225 open in chummer-rs, and files written by chummer-rs open in
Chummer 5.225.

### More

- **Undo and redo.** Edit → Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z or
  Ctrl+Y) work on every change to a character, in creation and career
  mode. The menu names the change ("Undo: Raised Pistols to 5 (10
  karma)"). Each open character keeps 100 steps. Typing in one text box
  or dragging one spinner is one step. Undo puts the character back
  exactly as it was, and takes back the karma and nuyen too. This is
  not the Undo button on Karma & Nuyen entries: that one refunds an
  expense by Chummer's rules, and is itself a step you can undo.
  While a text box has the keyboard, Ctrl+Z undoes typing in the box.
- **History.** The History tab in the right-hand panel (or View →
  History) lists this session's changes, newest first, with the time
  and the karma or nuyen they cost. Undone changes stay greyed out until
  a new change replaces them.
- **Commands.** Every change, in the GUI and in `chummer-cli apply`, is a
  command that `chummer_core::command::apply` runs. The same command on
  the same character gives the same file on every machine (new GUIDs and
  dates come from the command). This is the base for GM/player sync
  ([docs/online-design.md](docs/online-design.md)).
- **Sourcebook PDFs.**
  - 📖 links on items, skills and data entries open your PDF at the rule's page, in evince, zathura, okular or another viewer.
  - Tools → Sourcebooks can import your Chummer5a links from a Wine or Proton prefix, scan a folder, and detect page offsets with `pdftotext`.
  - A folder scan matches PDFs by title (`&` and "and" are the same), then, with `pdftotext`, by each book's known text. That finds errata, books printed inside another book's PDF (Data Trails' Dissonant Echoes) and files with odd names. Files it can't link are listed with the reason: another edition, errata with no book of its own, a duplicate, or a book Chummer has no data for.
- **Character sheets** (File → Print, Ctrl+P): Chummer's own XSLT sheets, in all six languages, opened in your browser to view or print.
- **Export** to XML, JSON (Chummer's format) and Squad Manager.
- **Custom data:**
  - All of Chummer5a's optional rule packs can be applied through house-rule presets.
  - Tools → Character settings duplicates and edits presets: build method, budgets, books, karma costs, options and custom data.
  - Share house rules: Export saves a preset as a Chummer settings file (Chummer5a reads it too); Import installs one, and asks before it replaces a different file of the same name.
  - A character whose settings file is not installed shows a warning with an Import button; its budgets use Standard, and its `<settings>` stays as it was until you pick another file with "Change Settings File" (Common tab).
- **Tools:**
  - A data browser (the Master Index tab) that searches every item, quality and spell.
  - Searchable drop-downs: lists with more than 8 entries filter as you type (words in any order); Enter picks the first match.
  - A dice roller (click any skill pool) and an initiative tracker.
  - A character roster (the Character Roster tab).
- **GM screen** (File → New Campaign / Open Campaign…): a campaign opens
  as its own tab next to the character tabs.
  ![GM screen](docs/screenshots/gm-screen-graphite.png)
  - **Roster** (left): players, NPCs, enemies, critters, spirits and drones, grouped by kind, with player, group or faction, notes and each member's damage. Add characters from files: copied into the campaign, or linked so they stay in their own `.chum5`/`.chum5lz`. Add critters with the critter builder, NPCs from a PACKS kit (a karma-build character of the chosen metatype with the kit applied, one or several), or an open character tab. Duplicate makes "Halloweener Ganger 1…4": copies with new GUIDs and numbered names.
  - **Open** a member as a full character tab. The tab edits the same character, with the same undo history; closing it hands it back to the GM screen. Saving the campaign (Ctrl+S on the GM screen or on a member's tab) stores copied characters in the campaign file and saves linked ones to their own files.
  - **Encounters** (middle): initiative order from each sheet's initiative (or a score you type), Roll initiative starts the next combat round, Next pass takes 10 from everyone, Next marks the current combatant as acted. Acted, delay, Seize the Initiative and Blitz (5d6) per combatant (Seize and Blitz spend the character's Edge), −5/−10 for interrupts, and quick combatants without a sheet (name, initiative, dice, their own damage tracks). Ties go to Edge, then Reaction, then Intuition.
  - **Combatant card:** physical (with overflow), stun, matrix and vehicle condition monitors, wound modifier, armor, Edge boxes, and dice pools (defense, damage resistance, composure, judge intentions, the best skills, weapons): click a pool to roll it. Quick damage: "8P AP-2" with an optional soak roll; Physical below the modified armor becomes Stun, extra Stun carries over into Physical, and the boxes are set through commands.
  - **GM awards and overrides:** give or take karma or nuyen with a note (a career ledger entry, shown as "GM gave Ghost 100 karma: great run"), and Add Improvement for a custom improvement the GM allows.
  - **Activity** (right): the campaign's feed of every change to its characters (from the command logs, with undos), awards, damage and dice rolls, and the GM's notes.
  - The format is chummer-rs's own: one `.chummercampaign` file, an LZMA-compressed JSON document (see [docs/online-design.md](docs/online-design.md#campaigns)). `chummer-cli campaign new|add|list` makes and reads them.
- **GM tools:**
  - File → New Critter… builds a critter or NPC from `critters.xml`, as Chummer does. Spirits, sprites and other Force creatures are built at a chosen Force: attributes, skills and powers follow from it. Spirits get their optional powers and Materialization (or Possession or Inhabitation). Critters open in career mode with rules ignored.
  - Special → Add PACKS Kit… applies a kit from `packs.xml` to a character in creation: qualities, attributes, skills, skill groups, knowledge skills, adept powers, martial arts, complex forms, A.I. programs, spells, spirits, lifestyles, armor, weapons, cyberware, bioware, gear, vehicles and karma for nuyen. Attribute and skill levels use creation points first, then karma, within the creation maximums. Chummer 5.226 applies no attributes, skills or powers from a kit.
  - Special → Create PACKS Kit… saves the character's things as a Custom kit in `~/.local/share/chummer-rs/packs/custom_*_packs.xml`. Custom kits can also be deleted there.
  - Spells & Spirits tab → Create Spell… designs a custom spell (Street Grimoire). Chummer's rules compute its drain value and descriptors. In career mode it costs spell karma.
- **Languages:** English, German, French, Japanese, Portuguese and Chinese data names and sheets.
- **Online campaigns:** the GM hosts a campaign from the GM screen (or
  with `chummer-authority`); each player joins with their own invite
  link and edits their own characters, live or by play-by-post. Every
  change is logged with its author, and the GM can revert any of them.
  A link works on the first device that uses it; the GM can revoke it
  or give the player a new one, and the relay mailbox only takes mail
  signed by a current player's key. See [Online campaigns](#online-campaigns).

### Layout and themes

The main window uses Chummer5a's layout: the File, Edit, Tools, Special, View,
Window and Help menus, a toolbar, and one tab per open character next to
the Master Index and Character Roster tabs. A character has Chummer's
tabs in Chummer's order (Common, Skills, Limits, Martial Arts, Spells &
Spirits, Adept Powers, Complex Forms & Sprites, Advanced Programs, Critter Powers,
Initiation, Cyberware & Bioware, Street Gear, Vehicles & Drones,
Character Info, Karma & Nuyen, Calendar, Game Notes, Improvements,
Relationships). The magic, resonance, Advanced Programs and critter tabs show only when the
character has them. The right-hand panel has Karma Summary (creation),
Condition Monitor, Other Info, Spell Defense and History; the status bar shows karma, essence and
nuyen.

Item lists are tree tables, grouped and nested the way Chummer5a's tree
views are: gear, armor, weapons and vehicles under "Selected Gear" (etc.)
and their locations; ware in cyberlimbs; mods and gear in armor;
underbarrel weapons and accessories on weapons; locations, mods by
category, weapon mounts, weapons and gear in vehicles; qualities under
Positive / Negative Qualities; spells by category; metamagics by grade;
critter powers and weaknesses; martial arts and their techniques. The
columns stay aligned at every depth. Click ▸ / ▾ (or press Left / Right
on the selected row) to close or open a node; the window remembers it.
Skills and attributes stay flat tables.

![Classic theme, gear tree](docs/screenshots/chummer-rs-classic-gear-tree.png)
![Graphite theme, cyberware tree](docs/screenshots/chummer-rs-graphite-cyberware-tree.png)

View → Appearance selects the layout (Classic, described above, or
[Workspace](#workspace-layout)) and its theme. The choice is saved in
`~/.config/chummer-rs/gui.ini`; `--layout classic|workspace` and
`--theme classic|graphite|dark|light` override it.

| Theme | Look |
|---|---|
| Graphite (default) | Classic layout. Dark greys with one teal accent, IBM Plex Sans and Plex Mono. |
| Classic | Classic layout. Chummer5a's Windows look: light grey panels, white fields, square corners, Windows-blue selection, the Selawik font. |
| Dark, Light | Workspace layout. Colours from the chummer-rs logo, IBM Plex, Phosphor icons. |

![Classic theme](docs/screenshots/chummer-rs-classic-common.png)
![Graphite theme](docs/screenshots/chummer-rs-graphite-common.png)
![Graphite theme, Skills tab](docs/screenshots/chummer-rs-graphite-skills.png)

For comparison, Chummer5a 5.226 under Wine:
[creation](docs/screenshots/chummer5a-create-common.png),
[career](docs/screenshots/chummer5a-career-common.png).

The fonts are in `crates/chummer-gui/assets/fonts/` with their SIL Open
Font License files (Selawik: Microsoft; IBM Plex: IBM). The Workspace
icons are [Phosphor](https://phosphoricons.com) (MIT licence), through the
`egui-phosphor` crate (MIT or Apache-2.0). The app icon is the chummer-rs
logo (`crates/chummer-gui/assets/logo/`, also `packaging/chummer-rs.svg`).

### Workspace layout

View → Appearance → Workspace (or `--layout workspace`) is a second
layout for the same characters, campaigns and data. Classic stays as it
was.

![Workspace, dark, a character in creation](docs/screenshots/workspace-dark-creation.png)
![Workspace, dark, a career character at the table](docs/screenshots/workspace-play-dark.png)

- **Top bar**: the logo and the menu (☰: File, Edit, Tools, Special,
  View, Window and Help, with Print), one tab per open document (Home,
  the campaign, each character with its mode), + to open or create, the
  search field and the sync state (local file saved or not, or an online
  character's sync).
- **Sidebar**: the open document's sections. A character's are grouped
  as Session (career: At the table), Build or Character, Story and
  Records, with the creation-issue counts as badges; Street Gear's
  sub-tabs are entries of their own. Home has the roster and the Master
  Index; the campaign has its roster (see the GM screen below). At the bottom:
  Undo and Redo (the tooltip names the change; online characters keep
  Undo off with the usual explanation), Settings (character settings,
  sourcebooks, online settings, guided creation, back to Classic) and the
  dark/light switch.
- **Budget strip**: creation budgets (attributes, special, skills, skill
  groups, knowledge, contacts, karma, nuyen, essence) with bars, or in
  career karma, nuyen, essence, limits, initiative and armor.
- **Page**: the section, with one hint line above it in creation (the
  guide's step and what is left, or the tab's issues). The build,
  story and record pages are the Workspace's own: tables with steppers in
  creation (points and karma), rating pips, pools you can click to roll,
  issue marks on the rows, and in career a "+1 · cost" button on every
  attribute, skill, skill group and knowledge skill, greyed when the
  karma is not there. Attributes & Qualities adds the priorities (read
  only), the derived values as cards and the qualities; Skills ends with
  "Other advances" in career (the cheapest attribute raises, initiation,
  new qualities and martial arts). Magic, resonance, critter, A.I. and
  martial arts pages show their summary (tradition and drain, power
  points, stream, initiation with its options and cost) above each
  list, and their editors (the spell picker and quickening, the mentor
  spirit and its choices, foci, adept powers, spirits and sprites,
  metamagics and echoes, martial art techniques) use Workspace buttons,
  check boxes, steppers and tables. Character Info, Game Notes,
  Calendar, Improvements (custom improvements as group headings over
  tables), Relationships (a segmented switch for Contacts, Enemies and
  Pets, icon buttons for link, notes and delete) and Karma & Nuyen (with
  the whole ledger) are restyled the same way, and so is the warning for
  a missing settings file.
- **Number steppers**: a small − and + inside the border and a value
  field as wide as the range needs (up to four digits; a longer value
  widens it), so nothing spills out at any value. Drag the value, or
  click it and type (arrow keys step while typing). Used for points,
  karma, ratings, quantities, connection and loyalty, spirit force and
  services, power levels and lifestyle months.
- **Text with presets**: a free-text field with a chevron inside its
  right edge that opens the presets (contact archetype, type, metatype,
  gender, age and the other contact fields, a custom improvement's
  selected value, the PDF viewer command, a new knowledge skill, whose
  type is set from the preset picked). Typing filters the presets, Up,
  Down and Enter pick one, Esc closes the list; any other text is kept.
  Classic uses the same field.
- **At the table** (career): the condition monitor as box grids (rows of
  the wound threshold, the wound modifier in the last box of a row,
  Physical and Stun side by side, overflow under Physical), damage taken
  by code (6P, 8P AP-2, soaked or not, as on the GM screen) and Edge as
  boxes (filled = available); initiative rolled with its passes; quick
  rolls (Defense, Damage Resistance, Composure, Judge Intentions, the
  best skills with their specialization pool, the weapons); every weapon
  with its stats, rounds as pips, fire modes, Fire and Reload (the item
  pane's ammunition choices); the armor worn, ammunition carried, gear at
  hand (drugs, slap patches and other consumables; Show opens the item),
  the Matrix device (attributes, cold/hot-sim initiative, wireless,
  active commlink, its condition monitor), vehicles with their damage
  track, and the session notes (Game Notes). The inspector has the
  character's dice roller (the last roll's dice, hits and glitch; pool,
  limit, Rule of Six) and its recent rolls. Every block pops out.
- **GM screen** (a campaign): the roster in the sidebar, grouped by kind
  with Physical and Stun bars, a filter, Add (files, critters, PACKS
  NPCs, open characters) and Invite; the encounter board (round and pass,
  Roll initiative, each combatant's score and roll, a menu for acted,
  delay, seize, blitz, interrupt, the score and remove; Next and Next
  pass; add from the roster or by name); the combatant's card (condition
  monitors, Edge, damage, Matrix and vehicle tracks, dice pools and
  weapons to roll, skills, gear, qualities, notes, the campaign entry,
  Open, Add Improvement); and in the inspector the players (host online,
  relay, invite link, mailbox), the activity feed (with Revert for an
  online campaign), the GM award and the GM's notes. The encounter, the
  card, the feed and the inspector sections pop out.
- **Item pages** (Cyberware & Bioware, Gear, Clothing & Armor, Weapons,
  Drugs, Lifestyles, Vehicles & Drones): "Add …" buttons, a summary card
  (essence, combat stats, vehicle stats, the lifestyle editor) and the
  item tree in a card, with the same columns, locations, nesting, issue
  marks, source links and remove buttons as Classic; the tree has faint
  guides and Phosphor carets.
- **Inline catalog**: "Add …" opens the game data inside the page instead
  of the selection dialog (Classic keeps the dialog). Filters on the
  left: kind (cyberware and bioware together), category with counts,
  grade, the rating the table shows, what fits the build (availability
  within the creation limit, essence, affordable, requirements met),
  legality and books. In the middle: the search (word matching as in the
  drop-down lists), sorting (best match, name, cost, availability,
  essence), and the results with rating, the kind's own columns,
  essence, availability, cost and the source page; a reason shows under
  a record that cannot be added. Arrow keys move, Enter adds and goes
  back to the list, Shift+Enter adds and stays, Esc goes back. The
  inspector shows the selected record: rating, grade, quantity, where to
  install it, and a preview made by applying the purchase to a copy of
  the character: essence before and after (with a bar), cost,
  availability against the limit, nuyen (left) after, and initiative,
  attributes, armor, limits and condition monitor where they change;
  the dice pools it changes ("Pistols 5 → 7": Defense, Damage
  Resistance, Composure, Judge Intentions, Memory, Lift/Carry, astral
  and Matrix initiative, the active skills with their specialization
  pool, and each weapon's dice pool, DV, AP and accuracy; a new weapon
  shows its values); the
  checks (requirements, availability, money); "Add", which runs the same
  command as the dialog (career mode pays for it), with the same
  question for bonus selections; and a comparison of up to three
  records (essence, initiative, cost).
- **Inspector**: creation issues with Finish creation, the selected
  item in the Workspace style (rating, quantity, equipped and wireless,
  custom name, location, cost, availability, essence, capacity,
  ammunition, matrix and vehicle panels, notes, contents with their
  "Add …" commands, which open the catalog with the parent set, Sell or
  Delete), the selected attribute (range, what modifies it, the
  skills and limits that use it, the raise button), skill (pool,
  specializations, rule) or skill group, in career Karma & Nuyen (karma,
  career karma, nuyen and street cred, Add entry, and the ledger
  filtered All / Karma / Nuyen with Undo on the entries that can be
  undone), the Karma Summary (creation) or Other Info and Spell Defense
  (career), and this session's history.
- **Home**: Continue cards for the recent characters (mode, karma or
  build method, when the file changed), every character of the roster
  folders with a filter and a status filter, the tools (Master Index,
  sourcebooks, dice roller, initiative tracker), the rulesets (character
  settings), and on the right the campaigns: the open one (GM screen),
  the joined ones with their sync state (online, via mailbox, offline,
  pending and refused changes), their characters, Sync now, the invite
  link and Leave; a field for an invite link (opens Join Campaign with
  it); and the campaigns' recent activity.
- **Command palette** (Ctrl+K or the search field): menu actions, the
  document's sections, the character's items, the open documents and
  every game-data record (opens in the Master Index), and in career the
  advances: "Raise Pistols 6 → 7", new specializations, with the karma
  cost (greyed when not affordable). Arrow keys select, Enter runs; the
  line under the list says what it will do ("Pistols 7 · karma 14 →
  0").
- **Pop-out windows**: the button in a panel's header (page, inspector
  section, every block of At the table and the GM screen, dice roller,
  initiative tracker) moves it into its own window; "Dock back" or
  closing the window puts it back. Dialogs opened from a popped-out
  panel (confirmations, the selection dialog, the editors, pickers,
  menus) show in its window; the inline catalog shows wherever its page
  is. Each kind of window opens where it was last and as big (kept
  between sessions), and the panels that were out when the app closed
  come back out when their character or campaign opens again (the dice
  roller and initiative tracker at once). On Wayland the system places
  windows (only the size is kept); on X11, Windows and macOS a new
  window opens next to the main window.

![Command palette](docs/screenshots/workspace-palette.png)
![At the table, light](docs/screenshots/workspace-play-light.png)
![The recent rolls in their own window](docs/screenshots/workspace-play-popout.png)
![A confirmation inside a popped-out Weapons page](docs/screenshots/workspace-popout-dialog.png)
![The catalog's preview with the dice pools a Muscle Toner changes](docs/screenshots/workspace-catalog-pools.png)
![The GM screen with an encounter, dark](docs/screenshots/workspace-gm-dark.png)
![The GM screen, light](docs/screenshots/workspace-gm-light.png)
![Workspace, light, Skills during creation](docs/screenshots/workspace-light-creation.png)
![Workspace, dark, Attributes & Qualities during creation](docs/screenshots/workspace-build-dark.png)
![Workspace, light, Skills during creation, a skill in the inspector](docs/screenshots/workspace-build-light.png)
![Workspace, career, Skills with the +1 buttons and the ledger](docs/screenshots/workspace-career-skills.png)
![Workspace, career, knowledge skills and other advances](docs/screenshots/workspace-career-advances.png)
![Workspace, light, career attributes, Agility in the inspector](docs/screenshots/workspace-career-light.png)
![Workspace, the palette offering advances](docs/screenshots/workspace-career-palette.png)
![Inline catalog, dark: adding cyberware in career mode, with the preview and a comparison](docs/screenshots/workspace-gear-catalog-dark.png)
![Inline catalog, light: adding cyberware during creation](docs/screenshots/workspace-gear-catalog-light.png)
![Weapons with a weapon in the item inspector, dark](docs/screenshots/workspace-gear-inspector-dark.png)
![Cyberware with an item in the inspector, light](docs/screenshots/workspace-gear-inspector-light.png)
![Home, dark](docs/screenshots/workspace-home-dark.png)
![Steppers at 1, 12 and 128, dark](docs/screenshots/workspace-stepper-dark.png)
![Steppers at 1, 12 and 128, light](docs/screenshots/workspace-stepper-light.png)
![A contact's archetype: the presets opened with the chevron, and filtered by typing, dark](docs/screenshots/workspace-preset-input-dark.png)
![The same, light](docs/screenshots/workspace-preset-input-light.png)
![Relationships, dark](docs/screenshots/workspace-relationships-dark.png)
![Relationships, light](docs/screenshots/workspace-relationships-light.png)
![Spells & Spirits with the spirits editor, dark](docs/screenshots/workspace-magic-dark.png)
![Spells & Spirits with the spirits editor, light](docs/screenshots/workspace-magic-light.png)
![Improvements with a custom improvement, dark](docs/screenshots/workspace-improvements-dark.png)
![Improvements with a custom improvement, light](docs/screenshots/workspace-improvements-light.png)
![Home, light](docs/screenshots/workspace-home-light.png)

### Command line

```bash
chummer-cli info character.chum5          # sheet summary
chummer-cli skills character.chum5        # skills with dice pools
chummer-cli items character.chum5         # everything the character owns
chummer-cli check ~/characters/           # load and verify many files
chummer-cli new out.chum5 --metatype Elf --priorities BACDE --talent Magician --skills Spellcasting,Summoning
chummer-cli sheet character.chum5 -o sheet.html [--sheet NAME] [--lang de-de]
chummer-cli export character.chum5 JSON -o character.json
chummer-cli roster ~/characters/
chummer-cli search "ares" gear
chummer-cli settings list | export "House rules" -o house.xml | import house.xml
chummer-cli sources import-wine | scan <dir> | detect | open SR5 143
chummer-cli hash character.chum5          # state hash (BLAKE3 of the saved XML)
chummer-cli commands                      # every command as JSON, for scripts
chummer-cli apply character.chum5 log.json -o out.chum5   # run commands
chummer-cli campaign new seattle.chummercampaign "Seattle Nights"
chummer-cli campaign add seattle.chummercampaign ghost.chum5 --player Anna [--link]
chummer-cli campaign add seattle.chummercampaign ganger.chum5 --kind Enemy --copies 4 --group Halloweeners
chummer-cli campaign list seattle.chummercampaign
```

`apply` reads a JSON array of commands (as `chummer-cli commands`
prints them) or of envelopes (command, seed, time, author). It prints
what each command did and the final version and hash.

Settings are stored in `~/.config/chummer-rs/`.

## Online campaigns

The GM's app is the campaign's host. There is no game server and no
account: peers connect directly (QUIC through [iroh](https://www.iroh.computer/),
with NAT hole punching) and use a relay only to find each other, or when
a direct path is not possible. The relay also keeps a sealed mailbox
for play-by-post. Each installation's identity is its node key
(`node.key` in the config folder).

> The project's public relay is not running yet. Until it is, run your
> own (`chummer-relay`, [docs/relay.md](docs/relay.md)) and enter it in
> Tools → Online Settings.

**The GM hosts a campaign**

1. Make a campaign (File → New Campaign), add the characters, and save it.
2. On the GM screen, tick **Host online** (top of the right-hand panel).
   The first time, the campaign becomes an online campaign: its
   characters move into the campaign's log, kept in
   `<name>.authority` next to `<name>.chummercampaign`. From then on
   every change to them is logged, hosted or not. You still open only
   the `.chummercampaign` file.
3. **Players & invites** (the inspector panel in Workspace, a section of
   the right-hand panel in Classic): **New invite…** asks for the
   player's name ("Anna"), optionally a character to give them when
   they join, and how long an unused link works. **Create link** makes
   a link (`chummer-rs://join/…`) for that one player: copy it and send
   it to them only. The first device that joins with it claims it; the
   same link then works for nobody else.
4. The list shows every invite: not used yet (and when it expires),
   joined from which device and when, last seen, online, the characters
   the player has, and mail from them waiting in your mailbox. Per row:
   **Copy link**, **New link** (for a new device: the old link and the
   device that used it stop working, the new device gets the
   characters), **Revoke** (cut the player off at once: their
   connection, their joins and their mail) and **Remove**. Revoke, New
   link and Remove ask first. Select a character in the roster and set
   **Played by** to give it to a player. NPCs, critters and other
   characters you keep are never sent to players.

   ![Players & invites, Workspace dark: a new link for Carla, Anna online, Bert offline, a revoked and an expired invite](docs/screenshots/workspace-gm-invites-dark.png)
   ![Players & invites, Workspace light: revoking Bert asks first](docs/screenshots/workspace-gm-invites-light.png)
   ![Players & invites in the Classic layout](docs/screenshots/classic-gm-invites.png)
5. Edit characters as usual. Your changes reach the player at once ("GM
   gave you 100 karma: Good run" in their History). Their changes show in
   the Activity feed with their name.
6. **Revert** on a line of the Activity feed (or in a character's
   History) takes that change back. Later changes are applied again on
   top; the ones that needed the reverted change (a skill bought with
   the reverted karma) are dropped and named in the status bar.
7. **Check mail** collects changes players made while you were offline
   and mails them yours. The app also does this when hosting starts and
   every three minutes.

Undo and Redo are off for online characters: a change is in the
campaign log as soon as it is made. Use Revert instead.

**A player joins**

1. File → **Join Campaign…** and paste the link (or open the link: the
   Linux desktop file registers `chummer-rs://`; on Windows, Tools →
   Online Settings has a button for it; any system can pass the link as
   the first argument, `chummer-rs 'chummer-rs://join/…'`). Enter the
   name the GM sees. The link is yours: it works only on the device that
   joins with it first.
2. The campaign shows under **Campaigns** on the Character Roster tab
   with its state: online, via mailbox, or offline, the name the GM gave
   your invite ("as Anna"), and how many changes wait to be confirmed.
   Your characters appear there when the GM gives them to you; click one
   to open it. If the GM's app refuses the link, the reason shows there
   (used on another device, revoked, expired, or replaced by a newer
   link): ask the GM for a new one.
3. The tab shows a badge: ✔ synced, ⟳N changes waiting, ⚠N changes the
   GM's app refused (listed in the History tab, with Dismiss), ⏸
   offline. History shows the character's campaign log.

**Play-by-post.** Players can always edit. When the GM's app is not
reachable, changes go to the relay's mailbox, sealed to the GM; the
GM's app applies them when it next checks mail and mails back the
results and its own changes. Mail is end-to-end encrypted; the relay
cannot read it. A player can even join by mail while the GM is
offline: the join goes into the GM's mailbox and is applied at the GM's
next mail check. The mailbox only takes mail signed by keys its owner
registered: the GM's takes the current invite keys, each player's takes
the GM's campaign key, so strangers cannot fill it.

**Always online: `chummer-authority`.** A campaign can run on a server
instead of the GM's app (not both at the same time):

```bash
chummer-authority --key node.key run Seattle.chummercampaign      # serve it (Ctrl+C / SIGTERM stops)
chummer-authority --key node.key invite create Seattle.chummercampaign --label Anna --assign Ghost --expires 7d
chummer-authority --key node.key invite list Seattle.chummercampaign      # state of every invite
chummer-authority --key node.key invite reissue Seattle.chummercampaign Anna   # a new link (new device)
chummer-authority --key node.key invite revoke Seattle.chummercampaign Anna    # cut Anna off
chummer-authority --key node.key invite remove Seattle.chummercampaign Anna
chummer-authority --key node.key assign Seattle.chummercampaign Ghost Anna   # or a node id, or gm
chummer-authority --key node.key status Seattle.chummercampaign   # members, owners, activity
```

Copy the campaign file, its `.authority` file and the GM's `node.key`
to the server (the key is the campaign's address). `invite ...` and
`assign` work while it runs (it takes the changes in within seconds; the
app, when it hosts the campaign, at its next mail check; an invite names
its player by label or id). `rotate-key` makes a new GM
campaign key, which players get with their next sync. It writes the characters back into the
campaign file every few minutes and when it stops, so the GM can later
open the file in the app again. Every change it accepts is journaled
(`<name>.authority.journal`) before players are told, so a crash or
power cut loses none. A systemd unit is in
`packaging/authority/`; relays come from `online.json` or `--relay`.

**Settings.** Tools → Online Settings: your name, your node id (a GM
can assign you a character with it), the relays
(`https://relay.example.org#<mailbox node id>`, one per line, as
`chummer-relay` prints them) and trusted certificates for relays with a
self-signed certificate. They are stored in `online.json` in the config
folder.

**Testing on one machine.** `chummer-relay --dev` runs a relay on
127.0.0.1 with a self-signed certificate (see
[docs/relay.md](docs/relay.md#local-test-relay)). Give each app
instance its own `XDG_CONFIG_HOME` (another node key), and in Online
Settings enter `https://127.0.0.1:3443#<mailbox id>` and its
`self-signed-cert.pem`.

## Troubleshooting

### "chummer-rs is not responding"

The desktop shows this when the window does not answer for a few
seconds. Saving, opening, printing, file dialogs, going online, mailbox
rounds and campaign saves run on their own threads (the status bar shows
a spinner and what is running), and the online sync no longer
compresses characters while it holds the campaign's state. If the window
still stalls, start chummer-rs from a terminal with frame timing on:

```bash
CHUMMER_TRACE_FRAMES=1 chummer-rs 2>trace.log
```

Every frame slower than 50 ms is then written to stderr with the phases
that ran in it (sheet recompute, issues, catalog preview, palette index,
page drawing, online refresh and so on), time spent painting outside the
frame, waits for the online locks, long holds of those locks by network
tasks, and slow background jobs:

```
[trace    93.208] slow frame #240: update 93.5 ms (worst so far 93.5 ms)
    workspace layout: 93.5 ms
      character page: 93.4 ms
        At the table: 93.2 ms
          Weapons: 79.7 ms
[trace] background job save:3 took 1333.6 ms
[trace   167.842] held the authority lock 34.4 ms (thread chummer-net)
```

Attach the log to a bug report. Without the variable nothing is timed.

## How it is checked

The 34 test characters from Chummer5a's own test suite are oracles. Chummer
wrote values into them that the tests recompute and compare. Most of the
remaining differences come from game data that changed after those files
were saved (they date from Chummer 5.18x-5.202).

| Oracle | Result |
|---|---|
| Attribute totals and essence | 475 / 475 |
| Bonuses replayed into improvements | 653 / 656 |
| Items rebuilt from game data | ~2,300 of ~3,100 saved items |
| Nuyen left after creation | 8 / 27; all 27 with the old 5.202 lifestyle price formula |
| Karma left after creation | 17 / 27; 26 / 27 with the house rules the fixtures were made with |
| Character sheets rendered | every fixture × every sheet × 6 languages |

The items row breaks down by kind:

| Kind | Rebuilt / saved |
|---|---|
| Gear | 1236 / 1513 |
| Cyberware | 139 / 296 |
| Weapons | 67 / 163 |
| Accessories | 140 / 147 |
| Spells | 100 / 101 |
| Qualities | 291 / 379 |

Files written by chummer-rs were also opened in a real Chummer5a 5.226,
built from source and run under Wine. They load without warnings and with
the same budgets ([docs/interop.md](docs/interop.md)).

```bash
cargo test --workspace
```

Beyond the oracles: fuzzed and mutated character, `.chum5lz`, campaign
and settings files; random command sequences (determinism, round trips,
undo/redo); a headless run of the GUI over every fixture, layout and
page; randomised sync with a lossy, reordering network and crashes; and
`tests/e2e/run.sh`, which runs the relay, a GM and players in Docker
under packet loss, partitions, kills, a full disk and hostile mail. See
[docs/testing.md](docs/testing.md).

## Not done yet

- Still on the UI thread, though short: opening a campaign file
  (~0.4 s for four big characters), a joined campaign's local copies at
  start (~0.2 s per big character), the first frame of the Play page
  with many weapons (~0.1 s). Compressing a character for the online
  sync (LZMA, ~2 s for one with big mugshots) runs on network and
  background threads, but costs that CPU on every change that is saved.
- Places where chummer-rs copies (or fixes) what looks like a Chummer5a
  bug are listed, with rules references, in
  [docs/likely-bugs.md](docs/likely-bugs.md). Every place where
  chummer-rs knowingly differs from Chummer5a (bug fixes, extra
  features, omissions, file differences) is in
  [docs/deviations.md](docs/deviations.md).
- Hero Lab import, ChummerHub, plugins and the auto-updater.
- Online campaigns:
  - The project's public relay is not running yet; its URL and mailbox
    id in `chummer_net::config` are placeholders.
  - Only the last 256 changes of a character can be reverted, and only
    by the GM. Undo is off for online characters.
  - One GM per campaign; the GM's node key is the campaign's address, so
    hosting from another machine needs a copy of `node.key`. The app
    and `chummer-authority` must not host the same campaign at once
    (nothing locks the files).
  - A `chummer-rs://` link opened while the app runs starts a second
    app; paste the link into File → Join Campaign instead. On macOS the
    link is not registered (paste it, or pass it as an argument).
  - Typing in an online character's text box sends each keystroke as a
    change (the feed shows a burst as one line, and Revert takes the
    burst back together).
  - NPCs marked "Visible to players" are not shown to players yet, and
    players see only their own characters' activity.
  - The authority file keeps a compressed copy of each character plus
    the state 256 changes back, so characters with large mugshots make
    it large.
  - A player whose invite key leaks (with the link, before or after it
    was claimed) can still put up to 200 messages into the GM's mailbox
    (the per-key cap) until the GM revokes or re-issues the invite; the
    GM's app drops them. Invites are for players only (the role GM is
    accepted but there is no co-GM support).
- Some career-mode details:
  - Enchantments, rituals and enhancements learned at a grade.
  - Binding stacked foci (undo of a stacked focus binding works).
  - The Living Persona's matrix bonuses.
- PDF export needs a browser's Print to PDF; no converter is bundled.
- Settings files:
  - "Change Settings File" in creation mode only offers presets with the same build method. Chummer re-runs metatype and priority selection to switch build methods.
  - Chummer also finds a missing settings file by its `<settingshashcode>`; chummer-rs does not compute that hash.
- PACKS kits:
  - Create PACKS Kit does not write skills (Chummer 5.226 does not either).
  - "Select Martial Art" entries in kits are skipped.
  - Chummer reads custom kits from its own `packs` folder. To use a kit in both programs, copy the file.
- Tree differences from Chummer5a: multi-level qualities are one row per
  level, not one merged node; the one gear item whose data says
  `startcollapsed` starts open; vehicle mods are always grouped by
  category (Chummer's default; the option to turn it off is not read);
  Chummer's "Initiate Grade" nodes are plain "Grade N" groups.
- Relationships:
  - No "Swap Ordering". In Chummer it only switches the contact panel
    between left-to-right and top-to-bottom flow; it changes no data.
  - Chummer has no editor for a contact's `<colour>`; chummer-rs shows
    it but cannot change it either.
  - Locations and items have no notes colour editor.
- About 290 UI labels have no Chummer translation string and stay English.
- Undo/redo and the history are for the open session only. They are not
  saved; closing the character clears them. History descriptions are in
  English.
- GM screen: damage to vehicles and drones is set by clicking their
  boxes (no vehicle soak); quick combatants soak nothing; the feed's
  texts are in English; matrix initiative and astral initiative are not
  offered in the encounter (the sheet's physical initiative is used).
- Creation issues not checked yet: metagenic quality balance, the
  Prototype Transhuman bioware limit, vehicle and drone mod slots, cyberware
  grades whose requirements are not met, and Friends in High Places
  contact limits. A missing technomancer stream is only a warning,
  because there is no stream picker yet.
- Workspace layout: in the build pages the priorities cannot be swapped
  (pick them in the New Character wizard), there is no "Change
  metatype", and the dialogs these pages open (the spell, mentor and
  metamagic pickers, Create Improvement, contact notes) are Classic
  windows with Workspace buttons. The design's
  training time and "Saving for" goals are not there. The catalog's
  table shows essence, cost and availability from the data at the chosen
  rating and grade (the inspector's preview is exact). Home's recent
  activity lists joined campaigns only (the GM's own feed is on the GM
  screen). Pop-outs: on Wayland the system places the windows (only
  their size is kept); with an item page popped out, the catalog's
  record and preview stay in the main window's inspector (pop out its
  Item panel too); the app's own windows (Character Settings, Export,
  Character Sheet, Sourcebooks, the New Character wizard) open in the
  main window. At the table: rolls and the rolled initiative are
  kept for the session only; gear at hand is listed, not used up (career
  mode has no quantity change); the design's situational modifiers, Edge
  actions (Push the Limit, Second Chance) and "Share to GM" are not there
  because the app has no such rules or messages. GM screen: the design's
  scene notes and activity filters are left out.
- Custom improvements: no drag and drop between groups (use the 📁 menu,
  a folder icon in the Workspace),
  and disabling one only switches its modifiers and the special attribute
  and tab flags; objects it created (a free spell, say) stay until it is
  deleted.

## Layout

| Path | Contents |
|---|---|
| `crates/chummer-core` | Engine, with no UI dependencies |
| `xml.rs` | Owned XML tree; files round-trip losslessly |
| `data.rs`, `custom_data/` | Game data and custom data merge (`XmlManager`) |
| `lang.rs`, `settings.rs`, `sources.rs` | Translations, house rules, sourcebook PDFs |
| `expr.rs` | Data-file expressions (`EvaluateInvariantXPath`) |
| `improvement.rs`, `bonus/` | Improvements and the bonus processor (`ImprovementManager`) |
| `character.rs`, `attributes.rs`, `skills.rs`, `calc.rs` | The character and its rules math |
| `items/` | One module per item kind: build, add, cost, edit |
| `chargen.rs`, `career/`, `essence_loss.rs` | Creation, career ledger, essence loss |
| `command.rs`, `command/` | Commands: the one way a character changes; sessions with undo/redo and the log |
| `gm/` | Critters, PACKS kits, custom spells |
| `campaign.rs`, `campaign/` | Local campaigns: members, encounters (initiative, damage), the activity feed, the `.chummercampaign` file |
| `print.rs`, `export.rs`, `roster.rs`, `calendar.rs` | Sheets, export, roster, calendar |
| `tree.rs` | Item lists as Chummer's trees (root nodes, locations, nesting) |
| `crates/chummer-gui` | egui desktop application |
| `chummer-gui/src/workspace/` | The Workspace layout: shell, widgets, command palette, pop-out windows, At the table (`play.rs`), GM screen (`gm.rs`), build/story/career pages, item pages, inline catalog, item inspector, Home |
| `crates/chummer-cli` | Command-line tool |
| `crates/chummer-net` | Online campaigns: iroh endpoints, campaign protocol, invites, mailbox client, sealing |
| `crates/chummer-sync` | Online campaigns: the GM's authority (with revert), player replicas with an outbox, mailbox play-by-post, the hosted campaign file (`hosted.rs`) and the app's node |
| `crates/chummer-authority` | Headless campaign host (`packaging/authority/`) |
| `crates/chummer-relay` | Relay server and mailbox (`packaging/relay/`, [docs/relay.md](docs/relay.md)) |
| `crates/chummer-testpeer`, `tests/e2e/` | Headless test player and the Docker end-to-end tests ([docs/testing.md](docs/testing.md)) |
| `tools/gen_bonus_table.py` | Generates simple bonus handlers from Chummer5a's C# |
| `resources/` | Data, translations, custom data, sheets and export templates from Chummer5a |

## Releases

Releases are built by GitHub Actions (`.github/workflows/release.yml`) when a version tag is pushed:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

## License

GPL-3.0-or-later, the same as Chummer5a. As a derivative work of Chummer5a, chummer-rs is distributed under the same license; see `LICENSE`. The game data and translations in
`resources/` come from Chummer5a; see `resources/xml_license.txt`.
Shadowrun is a trademark of The Topps Company, Inc. This project is not
affiliated with it or with Catalyst Game Labs.
