# chummer-rs file formats

chummer-rs keeps Chummer5a's **game data** (`resources/data/*.xml`,
custom data, settings files, sheets) exactly as Chummer5a has it. For
**characters** and **campaigns** it has its own files:

| File | Contents | Since |
|---|---|---|
| `.chumrs` | One character | 0.4 |
| `.chummercampaign` | One local campaign (GM screen) | 0.3 (container since 0.4) |

Both use the same container, described here precisely enough to read and
write them with other tools. Chummer5a's `.chum5` and `.chum5lz` stay
fully supported: they open, and File → Export to Chummer5a (or
`chummer-cli convert`) writes them.

The code is `crates/chummer-core/src/container.rs` (the container),
`chumrs.rs` (characters) and `campaign.rs` (campaigns).

Until chummer-rs 1.0 these formats may change without upgrade steps.
From 1.0 on they are frozen: every incompatible change raises
`schema_version` and adds an upgrade step (see [Versions](#versions)).

## The container

A container is a standard **ZIP archive** (PKWARE APPNOTE; no ZIP64, no
encryption). Any ZIP tool can list and extract it:

```text
$ unzip -l Barrett.chumrs
  Length      Date    Time    Name
---------  ---------- -----   ----
      516  1980-01-01 00:00   manifest.json
   280872  1980-01-01 00:00   character.xml
   489437  1980-01-01 00:00   mugshots/0.png
```

- Entries are compressed with **deflate** (method 8), except images
  (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`), which are **stored**
  (method 0) because they are compressed already. Readers must accept
  both methods for every entry.
- `manifest.json` is the first entry. Readers must not depend on the
  order.
- Entry timestamps are always 1980-01-01 00:00, so two saves of the same
  state differ only in the manifest's `modified`. The real times are in
  the manifest.
- Entry names use `/` and are relative. They are keys, never paths on
  disk.

### manifest.json

UTF-8 JSON:

```json
{
  "format": "chummer-rs character",
  "schema_version": 1,
  "app_version": "0.4.0",
  "created": "2026-10-08T15:48:39Z",
  "modified": "2026-10-08T15:48:39Z",
  "entries": {
    "character.xml": { "size": 280872, "blake3": "4ba03ca3…0543" },
    "mugshots/0.png": { "size": 489437, "blake3": "c94c16f3…ef0e" }
  },
  "summary": { "name": "Barrett", "essence": "0.2" }
}
```

| Field | Meaning |
|---|---|
| `format` | What the file is: `"chummer-rs character"` or `"chummer-rs campaign"`. A reader refuses any other value ("this is a chummer-rs campaign file, not a chummer-rs character file"). |
| `schema_version` | The format version of the file's contents (an integer, from 1). See [Versions](#versions). |
| `app_version` | The chummer-rs version that wrote the file. Informational, and named in the error when the file is too new. |
| `created` | When the file was first written, UTC, `YYYY-MM-DDTHH:MM:SSZ`. Kept across saves. (A campaign's is the campaign's own `created`.) |
| `modified` | When it was last written, UTC. |
| `entries` | Every other entry in the archive: its uncompressed `size` in bytes and the **BLAKE3** hash of its uncompressed bytes (64 lower-case hex digits). |
| other fields | Format-specific (see `summary` below). Readers ignore fields they do not know, and chummer-rs keeps them. |

### Reading rules

A reader (chummer-rs does all of this, `container::decode`):

1. Rejects a file that does not start with `PK\x03\x04`, has no
   `manifest.json`, or whose manifest is not valid JSON.
2. Rejects a different `format`.
3. Rejects a `schema_version` newer than it knows, with a message that
   names the version and the writing app ("this character file was saved
   by a newer chummer-rs (version 0.9.0, file format version 2); this
   version reads up to version 1. Update chummer-rs to open it.").
4. Reads every entry through a size limit, never trusting the sizes the
   archive declares, and rejects the file when a limit is passed:

   | Limit | Character | Campaign |
   |---|---|---|
   | File size | 256 MiB | 256 MiB |
   | Entries (listed or not) | 4096 | 16384 |
   | One entry, uncompressed | 256 MiB | 256 MiB |
   | All entries, uncompressed | 512 MiB | 1 GiB |
   | `manifest.json` | 4 MiB | 8 MiB |

5. Checks every entry the manifest lists: it must exist, have the listed
   size and the listed BLAKE3 hash (and the ZIP CRC-32 must match). Any
   mismatch is reported as damage, naming the entry ("the file is
   damaged: character.xml does not match its checksum").
6. Ignores entries the manifest does not list.
7. Runs the upgrade steps from the file's `schema_version` up to the
   current one.

### Writing rules

Write the whole archive to a temporary file in the same folder, flush it
to disk (`fsync`), then rename it over the target (and on Unix `fsync`
the folder). A crash or full disk leaves the old file whole. chummer-rs
does this for `.chumrs`, `.chummercampaign`, `.chum5` and `.chum5lz`
(`container::atomic_write`).

### Versions

`schema_version` counts incompatible changes to one format. Each format
has a list of upgrade steps (`chumrs::MIGRATIONS`,
`campaign::MIGRATIONS`), each taking the archive in memory from version
*n* to *n + 1* (renaming entries, rewriting JSON or XML). Reading runs
them in order, so a file of any older version loads; saving always
writes the current version. A change that older readers can safely
ignore (a new optional entry, a new manifest field) does not need a new
version.

Current versions: character 1, campaign 1. Neither has upgrade steps
yet.

## `.chumrs`: a character

`format` is `"chummer-rs character"`.

| Entry | Required | Contents |
|---|---|---|
| `character.xml` | yes | The character as Chummer XML (below). |
| `mugshots/<n>.<ext>` | when there are mugshots | One image per mugshot. |
| `history.json` | no | What was changed, across sessions. |
| `guide.json` | no | Where guided creation was. |

The manifest's `summary` holds `name` (the character's display name) and,
when known, `essence` (the total essence, as Chummer writes it in
`<totaless>`), so listings need not parse the XML.

### character.xml

The same XML document a `.chum5` holds (Chummer's `<character>` with
Chummer's element names; see `Character.Save` in Chummer5a), in
chummer-rs's canonical form (`command::canonical`):

- without `<chummerrsversion>` (the manifest's `app_version` replaces
  it),
- without the totals Chummer writes only for exports: `<totaless>` and
  each attribute's `<totalvalue>`. A reader that needs them for a
  `.chum5` recomputes them (chummer-rs does on export); `summary.essence`
  has the essence.

Every **mugshot** of the character (`/character/mugshots/mugshot`) is
moved to its own entry: the element stays, in place and in order, but
empty, with an `entry` attribute naming the image:

```xml
<mainmugshotindex>0</mainmugshotindex>
<mugshots>
  <mugshot entry="mugshots/0.png" />
</mugshots>
```

`mugshots/<n>.<ext>` holds the bytes the `.chum5`'s base64 text decoded
to; `<n>` is the mugshot's index, `<ext>` comes from the image's magic
number (`png`, `jpg`, `gif`, `bmp`, `webp`, else `bin`). To make a
`.chum5` again, replace each `<mugshot entry="…"/>` with
`<mugshot>BASE64</mugshot>`, the entry encoded as standard base64 with
`=` padding and no line breaks, and remove the attribute. A mugshot is
only moved out when that encoding gives back exactly the original text;
any other (odd whitespace, no padding) stays inline as base64. So the
round trip is exact. A `<mugshot>` whose `entry` is missing from the
archive is damage, not a missing picture.

Contacts', spirits' and other nested `<mugshots>` stay inline.

### history.json

An array, oldest first, of at most 2000 items:

```json
[ { "at": 1759931319000, "author": "", "description": "Raised Pistols to 5 (10 karma)" } ]
```

`at` is Unix time in milliseconds, `author` who made the change (empty
for the local user, `"GM"` for a GM's change), `description` the text
the History panel shows. It is a record to read, not a log to replay:
the commands themselves are not stored, so a change to chummer-rs's
commands never breaks old files. chummer-rs shows these under "Earlier
sessions" in the History panel and appends the session's changes on
each save. A damaged `history.json` is dropped; the character still
loads.

### guide.json

```json
{ "step": "attributes", "visited": ["concept", "metatype", "attributes"] }
```

Step ids of `chargen::guide::Step`. Without it, chummer-rs uses
`guide.ini` in its configuration folder (which is where it keeps this for
`.chum5` files). A damaged `guide.json` is dropped.

### Not in the file

Online campaign state stays in its own files next to the campaign
(`.authority`, replicas and keys, see [online-design.md](online-design.md)):
it changes on every sync and is per machine. Saving an online character
as `.chumrs` writes a plain offline copy.

## `.chummercampaign`: a campaign

`format` is `"chummer-rs campaign"`.

| Entry | Contents |
|---|---|
| `campaign.json` | The campaign (below). |
| `members/<id>.xml` | Each embedded member's character, as `character.xml` in a `.chumrs`. |
| `members/<id>/mugshots/<n>.<ext>` | That character's mugshots (`entry="members/<id>/mugshots/0.png"`). |

The manifest's `summary` holds `name` and `members` (their number).

`campaign.json` is the campaign as JSON. An embedded member's character
is `{"storage": "embedded", "entry": "members/<id>.xml"}`, a linked one
`{"storage": "linked", "path": "runners/ghost.chumrs"}` (a relative path
is resolved against the campaign file's folder):

```json
{ "id": "<32 hex>", "name": "Seattle Nights", "created": "2026-10-06T12:00:00",
  "gm_notes": "...",
  "members": [ { "id": "<32 hex>", "kind": "Player", "name": "Ghost",
                 "character": { "storage": "embedded", "entry": "members/<32 hex>.xml" },
                 "player": "Anna", "owner": null, "group": "", "notes": "",
                 "visible_to_players": false } ],
  "encounters": [ ... ],
  "log": [ { "at": 1759750000000, "author": "GM", "member": "<32 hex>", "description": "..." } ],
  "roll_settings": { "show_gm_rolls": false, "players_see_each_other": true },
  "rolls": [ { "id": "<32 hex>:<n>", "who": "Ghost", "member": "<32 hex>", "player": "Anna",
               "open": false, "roll": { "at": 1759750000000, "label": "Pistols · Ares Predator V",
               "pool": 9, "edge": null, "rule_of_six": false, "limit": 5, "initiative": null,
               "dice": [6, 5, 1, 2, 3, 4, 6, 2, 5] } } ] }
```

`roll_settings` says who sees which dice rolls in an online campaign;
`rolls` are the last 200 rolls at the table, newest first (the GM's, and
online the players'; `player` is empty for the GM's). A roll's hits are
worked out from its dice, never stored. Every field has a default and
unknown fields are ignored. The full field
list is the `Campaign` type in `crates/chummer-core/src/campaign.rs`.

Older campaign files (before the container) were one LZMA stream (as
`.chum5lz`) of JSON with `"format": "chummer-rs campaign", "version": 1`
and embedded characters as an `"xml"` string. They still open; the next
save writes the container.

## Chummer5a's formats

| File | Read | Written |
|---|---|---|
| `.chum5` | yes | File → Export to Chummer5a, Save As, Save (when chosen), `chummer-cli convert` |
| `.chum5lz` | yes | the same; LZMA "alone" format with Chummer's "Balanced" settings (see `chum5lz.rs`) |

What a file is gets decided by its content: a `.chum5` that is really a
`.chumrs` (renamed) still opens. Writing uses the extension.

The first Save of a character opened from a `.chum5`/`.chum5lz` asks
whether to save it as `.chumrs` (the default, through Save As; the
Chummer5a file is left as it is) or to keep saving the Chummer5a file.
While closing or quitting, Save goes straight to Save As `.chumrs`.
