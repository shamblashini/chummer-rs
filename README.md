# chummer-rs

A Rust rewrite of [Chummer5a](https://github.com/chummer5a/chummer5a), the
Shadowrun 5th Edition character manager. It runs natively on Linux: no
Wine, no .NET, no Internet Explorer.

It reads and writes the same `.chum5` files and uses Chummer5a's own game
data, so characters move between the two programs.

> **Status: foundation.** chummer-rs opens, shows and edits existing
> characters, and computes their numbers the way Chummer5a does. It
> cannot yet build a new character or add items. See [Status](#status).

## Install

You need a Rust toolchain (1.85 or newer).

```bash
./install.sh
```

This builds release binaries and installs to `~/.local`:

- `chummer`: the desktop application.
- `chummer-cli`: the command-line tool.
- The game data in `~/.local/share/chummer-rs`.
- A desktop entry. `.chum5` files then open with chummer-rs.

To run from the source tree without installing:

```bash
cargo run --release -p chummer-gui -- path/to/character.chum5
```

## Use

### Desktop application

`chummer [--tab <name>] [files...]` opens one tab per character.

Character tabs:
- **Info**
- **Attributes**
- **Skills**
- **Qualities & Contacts**
- **Magic & Resonance**
- **Equipment**: gear, ware, armor, weapons, vehicles and lifestyles. Nested items are shown.
- **Improvements**
- **Karma & Nuyen** log
- **Notes**

The right-hand panel shows:
- Karma, nuyen and essence.
- Initiative: physical, astral, cold-sim and hot-sim.
- Limits, armor, and composure, judge intentions, memory and lift/carry.
- Clickable condition monitors.

Edits recompute every value at once.

Other features:
- Click a skill's dice pool to roll it.
- **Tools → Data browser** searches every item, quality, spell and so on in the game data.
- The **Language** menu switches between the six Chummer5a translations.
- Keyboard: Ctrl+O, Ctrl+S, Ctrl+W, Ctrl+Q.
- Drag and drop `.chum5` files onto the window.

### Command line

```bash
chummer-cli info character.chum5      # sheet summary
chummer-cli skills character.chum5    # skills with dice pools
chummer-cli items character.chum5     # everything the character owns
chummer-cli check ~/characters/       # load and verify many files
chummer-cli search "ares" gear        # search the game data
```

## Status

### What works

- **Loading and saving `.chum5`:**
  - Saving is lossless. All 34 test characters round-trip with every element kept, including the many elements this port does not model yet.
  - `<appversion>` is left as loaded, because Chummer5a uses it to decide how to read a file.
  - Writes go to a temp file first and are then renamed, so a crash cannot corrupt a character.
- **Game data:** all 42 data files, the custom data packs, the settings presets and the six translations load.
- **Rules math, checked against Chummer5a.** Chummer5a writes each attribute's total and the character's essence into the save file. The test suite recomputes these values and compares them: **475 of 475 match across 34 characters**. Three of those characters were saved with house rules (limb count 5, or essence rounded to 3 decimals), and the test applies the same rules to them. Computed values:
  - Attribute minimums, maximums, natural and augmented values. This includes improvement stacking with unique names and precedence, essence loss, and cyberlimb averaging.
  - Essence, including grade multipliers and essence-cost improvements.
  - Initiative: physical, astral and Matrix.
  - Condition monitors and wound modifiers.
  - Physical, mental, social and astral limits.
  - Composure, judge intentions, memory and lift/carry.
  - Armor, including stacking accessories capped at STR.
  - Skill ratings and dice pools, including groups, defaulting and specializations.
  - Karma costs for attributes and skills.
  - The expression language in data files: the XPath subset, `FixedValues`, availability strings. Rounding follows Chummer5a's away-from-zero rounding.
- **Editing:**
  - Text fields.
  - Attribute base and karma levels.
  - Skill, skill group and knowledge skill levels.
  - Karma, nuyen, street cred, notoriety and public awareness.
  - Condition monitor damage.
  - Removing items. This also removes the improvements the item granted.

Performance (release build on the author's machine): `chummer-cli info` loads the data and computes a full sheet in about 30 ms. `chummer-cli check` verifies all 34 test characters in about 0.15 s.

### Not done yet

Roughly in order of priority:

1. **Bonus processor.** In Chummer5a, adding a quality, piece of ware or power turns its `<bonus>` XML into improvements. The data uses 246 bonus types, and the 60 most common cover 92% of uses. Until this exists, chummer-rs cannot add items. Items already in a saved file work, because the file stores their improvements.
2. **Adding items** through selection dialogs: gear, ware, weapons, armor, spells, powers, qualities and contacts. New skills too. The writer can only update skills that already exist in the file.
3. **Character creation:** priority, sum-to-ten, karma and life module builds, metatype selection, and point budgets.
4. **Career mode:** the karma and nuyen ledger with undo, initiation and submersion, and the incremental burn of essence loss in career mode.
5. **Custom data:** the `amend_*.xml` merge from enabled custom data directories. The directories ship but are not applied.
6. **Character sheets and printing.** The XSLT sheets are bundled but not rendered.
7. Smaller items:
   - Vehicle and drone stats, matrix attributes and weapon ranges are shown as stored, not recomputed.
   - Movement, encumbrance, and some A.I. and critter special cases.
8. Out of scope for now: ChummerHub, plugins, Hero Lab import, the auto-updater.

## Layout

| Path | Contents |
|---|---|
| `crates/chummer-core` | Engine. No UI dependencies. |
| `xml.rs` | Owned XML tree that round-trips Chummer files. |
| `data.rs`, `lang.rs`, `settings.rs` | Game data, translations, house-rule presets. |
| `expr.rs` | Data-file expression evaluator (`EvaluateInvariantXPath`). |
| `improvement.rs` | Improvements and `ValueOf` aggregation. |
| `character.rs`, `attributes.rs`, `skills.rs` | The `.chum5` model. |
| `calc.rs` | Rules math. Comments name the C# member each function ports. |
| `engine.rs` | Data + catalog + settings, for front ends. |
| `crates/chummer-gui` | egui desktop application. |
| `crates/chummer-cli` | Command-line tool. |
| `resources/` | Data, translations, custom data and sheets from Chummer5a. |

## Tests

```bash
cargo test --workspace
```

The fixtures in `crates/chummer-core/tests/fixtures` are Chummer5a's own test characters.

## License

GPL-3.0-or-later, the same as Chummer5a. The game data and translations in `resources/` come from Chummer5a. See `resources/xml_license.txt`. Shadowrun is a trademark of The Topps Company, Inc. This project is not affiliated with it or with Catalyst Game Labs.
