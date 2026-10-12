# Print sheet designs

Three mockups for a new default character sheet, built from the print XML of
`crates/chummer-core/tests/fixtures/Munin_Career.chum5`. Munin is a career-mode
shamanic magician: 7 spells, initiate grade 1, mentor Raven, essence 5.78. These
files are design only. No code changes.

| File | Pages (US Letter) |
| --- | --- |
| `current.html` / `current.pdf` (default sheet, for comparison) | 3 |
| `design-1-clean.html` / `.pdf` | 4 |
| `design-2-dense.html` / `.pdf` | 2 |
| `design-3-flavour.html` / `.pdf` | 3 |

The PNGs of each page are in `png/<name>-N.png` (90 dpi).

## Rebuild

```sh
chummer-cli sheet crates/chummer-core/tests/fixtures/Munin_Career.chum5 \
    --xml /tmp/munin-print.xml -o /tmp/current-raw.html
python3 docs/design/print/generate.py /tmp/munin-print.xml \
    --current-in /tmp/current-raw.html --render
```

`generate.py` reads the print XML and writes the three design files. With
`--current-in`, it also writes `current.html`. With `--render`, it prints every
HTML file to PDF with Playwright's `chrome-headless-shell` (headless, under
`nice -n 19`), and then makes the PNGs with `pdftoppm -r 90`. It needs only the
Python standard library. Pillow is optional: it makes the small portrait.

The print XML is not committed, because it holds the full 1.5 MB portrait.

## Problems with the current sheet

These problems come from the current default sheet,
"Shadowrun 5 (Skills grouped by Rating greater 0)":

- The full-size portrait fills half of page 1 and makes the HTML 1.6 MB.
- Attributes are small "LABEL: value" boxes. You cannot find a value quickly.
- The bottom third of page 1 is empty.
- All text has the same size and weight. There is no hierarchy.
- The green headers and green condition-monitor boxes print as grey blocks.
- Weapons have nested range sub-tables.
- Spells show only "F-3". The player must calculate the drain at the table.
- The resistance block fills a third of page 2.
- There are no page numbers.

All three designs are black and white and print-friendly, with no solid dark
fills. Each block has `break-inside: avoid`, so a page break does not cut a
block. The footer shows "page X / Y" (CSS `@page` margin boxes). Condition
monitors are empty outlined boxes. The wound modifier is written in the box
(−1 at box 4 and −2 at box 7, because High Pain Tolerance moves the marks by
one box). The overflow boxes have dashed borders. Each design has an Edge
tracker. Each design contains attributes, derived values, limits, condition
monitors, skills with pools, qualities, weapons (DV, AP, modes, RC, ammo,
pools), armor, cyberware and bioware with essence, gear, spells with drain,
contacts, lifestyle, karma and nuyen, and notes.

## Design 1: clean modern (4 pages)

**Idea.** A calm, modern sheet. It uses the system sans-serif font, rounded
cards with light rules, and three type levels: big numbers, bold labels and
small grey detail text.

- **Header.** The name is large. Below it are the real name, metatype, sex and
  age, then a profile line. At the right is a block with karma, career karma,
  nuyen, essence, street cred / notoriety / public awareness, and Edge left.
  The portrait is a small thumbnail (about 22 mm).
- **Page 1.** The Attributes card shows big numbers in boxes, with "max N"
  under each one. EDG, MAG, ESS and INIT are in the last row. The Limits card
  shows Physical, Mental, Social and Astral as big numbers, with the limit
  modifiers below. The Derived card shows initiative, astral and matrix
  initiative, defense, damage resistance, armor, surprise, composure, judge
  intentions, memory, lift/carry, movement and natural recovery. The Condition
  card shows Physical 9 + 3 and Stun 10 as box grids, and Edge circles. Below
  these cards are the active skills in two columns, with ATTR, RTG and a bold
  POOL column. The skill groups are small pills.
- **Page 2.** Weapons, one per row, with a range line under each row. The
  underbarrel weapon is indented under its parent. Then armor with the worn
  total, cyberware and bioware with essence, qualities, and knowledge skills
  and languages.
- **Page 3.** Spells by category, with drain at Force 1–6. Then tradition and
  spirits, initiation, mentor spirit, lifestyle and contacts.
- **Page 4.** Gear in two balanced columns, the drone with its condition
  monitor, karma and nuyen, description, background and note lines.
- **Footer.** "Munin — Elena Wagner" at the left. "Chummer-rs · page N / 4" at
  the right.

**Improvements over the current sheet.** The portrait is a thumbnail, so page 1
has space for the stats that the player uses most. Attributes and limits are
big numbers. There is a clear type hierarchy. There are no colour fills. Each
weapon uses one row and one range line, with no sub-tables. Drain is calculated
for each Force. The page numbers are in the footer. The resistance block is
removed. Its most important values (defense, damage resistance, drain) are in
the Derived card.

## Design 2: table-top dense (2 pages)

**Idea.** Use the fewest pages possible at the table. The type is condensed and
about 7.6 pt (Noto Sans Condensed), in two flowing columns. The most-used
numbers are at the top of page 1.

- **Top line.** The name, then metatype, profile and player. At the right:
  KARMA/TOTAL, NUYEN, ESSENCE and SC/NOT/PA.
- **Top band of three boxes.**
  - ATTRIBUTES, with "/natural max" after each value and the limits below.
  - CONDITION MONITOR: Physical "9 + 3 overflow" and Stun "10 boxes" as
    checkbox tracks with the −1, −2, Down and KO marks. It also has an Edge
    circle, write-in lines for Init and Wound, Sustain ("−2 each") with three
    slots, note lines, and a line with the initiative values.
  - QUICK POOLS: Defense (REA+INT), Soak (BOD+Armor), Drain (WIL+CHA),
    Spellcasting, Counterspelling, Perception, Assensing, Composure, Judge
    intentions, Memory, Surprise and Lift/carry.
- **Two columns.**
  - WEAPONS: POOL is the first column and bold. Then ACC, DV, AP, MODE, RC and
    AMMO. Ranged weapons have ammo checkboxes and a range line.
  - ARMOR on one line, with the worn total.
  - ACTIVE SKILLS: POOL first, then skill (group), ATTR and RTG.
  - KNOWLEDGE & LANGUAGES.
  - SPELLS by category. Each spell has a "DRAIN AT FORCE 1–6" grid. The
    minimum drain is 2. An underlined value means that Force is above Magic,
    so the drain is physical.
  - TRADITION, with the mentor and initiation.
  - QUALITIES, AUGMENTATIONS, GEAR, DRONE, CONTACTS, LIFESTYLE and NOTES.
- **Footer.** "Munin · Elf (Dryad) · Magician" at the left. "page 1 / 2" at the
  right.

**Improvements over the current sheet.** It uses 2 pages instead of 3. Page 1
has everything for a combat turn, so the player does not turn pages during
combat. The pool is the first and boldest value in each row. Damage, Edge,
initiative and sustained spells have places to write. Drain is calculated for
each Force. There is no portrait and there are no colour fills. Page numbers
are in the footer.

## Design 3: Shadowrun flavour, "runner dossier" (3 pages)

**Idea.** The sheet looks like a corporate file on the runner, but it stays
easy to use at the table. It uses monospace labels, numbered sections and
terminal-style marks.

- **Header rule.** "SHADOWRUN 5E // RUNNER DOSSIER", "FILE MUNIN-ED1E45" and
  "CAREER // SUMTOTEN".
- **Portrait.** Greyscale, with corner brackets and "IMG 01 // MUNIN" below it.
- **Identity.** "> MUNIN" in big monospace, with an outline-only, rotated
  "AWAKENED" stamp. Monospace label/value lines: LEGAL NAME, METATYPE,
  PROFILE, MENTOR, SIN STATUS (from the SINner quality), PHYSICAL, HANDLER (the
  fixer contact) and PLAYER.
- **Boxed right column.** KARMA, CAREER KARMA, NUYEN, ESSENCE, STREET CRED,
  NOTORIETY and PUBLIC AWARENESS.
- **Numbered sections with dashed rules.**
  - Page 1: 01 ATTRIBUTES, 02 DERIVED, 03 CONDITION, 03b RESIST and
    03c FIELD NOTES.
  - Page 2: 04 SKILLS, 05 KNOWLEDGE // LANGUAGES, 06 QUALITIES,
    07 COMBAT // WEAPONS and 07b COMBAT // ARMOR.
  - Page 3: 08 MAGIC // SPELLS, 08b MAGIC // TRADITION, 09 AUGMENTATIONS,
    09b LIFESTYLE // ASSETS, 10 GEAR, 10b DRONES and 11 CONTACTS.
- **Marks.** Attributes have hatched pips up to the natural maximum
  ("■ natural · □ to max"). Pools are shown as `[14]`. `[x]` marks worn armor.
  Gear trees use `└─` branches. Spells have one drain column for each Force
  from 1 to 6.
- **Footer.** "// RUNNER DOSSIER MUNIN-ED1E45" at the left and "// PAGE N / 3"
  at the right.

**Improvements over the current sheet.** It has the same improvements in
function as design 1: a small portrait, a hierarchy, no colour fills, one-row
weapons, drain for each Force, and page numbers. The resistance block is one
compact table of 7 rows, not a third of a page. The pips show how much each
attribute can still grow. The flavour comes only from type and line marks, so
it prints cleanly in black and white.

## Recommendation for implementation

Add each design that we accept as a new XSLT in `resources/sheets/`, with a
`<sheet>` entry in `resources/data/sheets.xml`. Use the existing xsltproc
pipeline, which transforms the same print XML that `generate.py` reads. A new
sheet then needs only the XSLT file and the `sheets.xml` entry. The original Chummer sheets stay
selectable. Make the new sheet the default with a saved "Default character
sheet" preference. A "Use original" action then sets that preference back to
the Chummer sheet with one click.

When the XSLTs are written, keep these points from the mockups:

- Print the values that the XML gives (pools, `armor`, `drainvalue`). Do not
  calculate them again. The worn armor is 14 in the XML, not the sum of the
  pieces (12 + 2 + 1).
- Calculate the drain for each Force as max(2, F + modifier). Underline the
  value when F is more than MAG. Show "—" for "Special".
- Put the wound marks at box `offset + k × cmthreshold`, with the offset from
  `physicalcmthresholdoffset` and `stuncmthresholdoffset`.
- Scale the portrait down (or use a CSS thumbnail, and keep the data URI
  small).

## Differences from the original mockups

- The data does not give "ED1E45" (the dossier file number). It is not in the
  print XML or in `chummer-cli hash`, so `generate.py` keeps it as a constant
  (`DOSSIER_FILE_ID`).
- The print XML marks no language as native. A language at rating 0 (German)
  is shown as "N".
- In `current.html`, the full-size PNG portrait is replaced by a JPEG of about
  300 px, with the original display width kept. The file is now 156 KB, not
  1.6 MB. The layout and page count (3) do not change.
