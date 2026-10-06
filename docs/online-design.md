# Online campaigns: design

How chummer-rs syncs characters between a GM and players. Agreed with the
project owner on 2026-10-06; this file is the reference for the work.

## Decisions

- Desktop only (Linux, Windows, macOS). No web or browser client.
- The GM's app is the authority for a campaign. There is no central
  game server. A small public relay (run by the project owner, and
  self-hostable by anyone) only connects peers and holds an encrypted
  mailbox.
- No VPNs and no port forwarding: peers connect with
  [iroh](https://www.iroh.computer/) (QUIC, addressed by public key, NAT
  hole punching, relay fallback).
- Logging, not anti-cheat. Every change is recorded and shown; nothing
  a player does is blocked for being "not allowed". The GM can revert
  any change.
- Players see and edit only their own characters. The GM sees and edits
  every character in the campaign. One GM per campaign (co-GMs may come
  later).
- GM edits to a player's character apply at once and are logged ("GM
  gave you 100 karma: <note>").
- Changes made while the other side is offline are queued, and go
  through the relay's mailbox, so play-by-post games work.
- `.chum5`/`.chum5lz` import stays. Export and the storage format do not
  have to match Chummer's.

## Building blocks

### 1. Commands

Every change to a character is a `Command`: a serialisable value that
describes the intent ("raise Pistols by 1", "add gear X with rating 4",
"set notes", "GM: add 100 karma, note N"). All edits, in the GUI and the
CLI, run through one function:

```text
apply(character, command) -> Result<Applied, Rejected>
```

- Commands are deterministic. Anything random or time-based (new item
  GUIDs, timestamps, dice) is created by the client and stored in the
  command, so every machine gets the same result.
- `Applied` holds what undo needs. Local undo/redo comes from this
  before any networking exists.
- The karma/nuyen ledger stays as it is; commands that spend karma or
  nuyen still write ledger entries.

Implemented in `chummer_core::command` (local part, no networking):

- `Command` is a serde enum (about 100 variants; `Command::examples()`
  has one of each). It names things by stable identifiers: item, skill,
  contact and ledger guids, data records by `<id>` with the name as a
  fallback (`RecordRef`), improvements by index plus `<sourcename>`
  (refused when they no longer match). Kits and imported contact files
  travel as their XML text, so a command does not depend on files on
  the sender's disk.
- A command travels in an `Envelope { cmd, seed: u64, at: i64 (Unix
  ms), author: String }`. JSON (`to_json`) is for debugging and logs;
  postcard (`to_bytes`) is the compact wire form.
- `apply(ch, engine, &Envelope) -> Result<Applied, Rejected>`. While it
  runs, `items::new_guid()` draws from the envelope's seed and
  `chargen::now_iso()` returns the envelope's time (a thread-local scope,
  `dice::deterministic`); outside `apply` both behave as before. After a
  successful command the essence-loss improvements are refreshed as the
  GUI used to do after every edit (creation: always; career: when the
  essence or the essence at special start changed; plain-text edits
  skip it).
- A rejected command leaves the character byte-identical to before
  (`apply` restores its copy). `Rejected.confirm` marks a reason that
  is a question for the user (firing with too few rounds); the UI then
  sends a follow-up command.
- `Applied` holds the character before the command (for undo), a
  description for the log ("Raised Pistols to 5 (10 karma)"), an optional
  status message and a count for bulk commands. `changed: false` means
  there was nothing to do; nothing is recorded.
- `Session` (one per open character) holds the log, the undo stack (100
  steps) and the redo stack. Undo restores the state before the last
  command exactly and takes its entry off the log; redo puts it back. Undo
  steps store only the top-level parts of the document that changed, so
  large mugshots are not copied 100 times. Commands with the same
  `coalesce_key` (one text box, one spinner) less than 1.5 s apart merge
  into one log entry and one undo step; this is safe because those
  commands set a value outright.
- The career ledger's own "Undo" (refund an expense) is a command,
  `UndoExpense`, and can itself be undone.

### 2. Versions, hashes and snapshots

- Each character in a campaign has a version: the number of commands
  the authority has applied to it.
- After each command the authority sends the new version and a hash of
  the character's canonical form.
- A snapshot is the whole character (compressed). Snapshots are taken
  every N commands so logs stay short, and they are what a client loads
  to resync.

### 3. Concurrent edits

A client sends `(command, base_version, base_hash)`.

1. `base_version` is the current version: apply the command.
2. Otherwise the authority rebases: it runs the command against the
   current state. Commands are intents, so most still apply (the GM gave
   karma while the player raised a skill: both happen).
3. If the command no longer passes (not enough karma any more), it is
   rejected with the reason and the client shows it.
4. The client compares its own hash after applying with the authority's.
   A mismatch means its copy drifted: it loads the authority's snapshot.

### 4. Offline and play-by-post

- Each player keeps a full local copy of their characters and can play
  and level up offline. Their commands go into an outbox.
- When the GM's app is reachable, the outbox is sent and rebased as
  above. Rejected commands are listed for the player.
- When it is not reachable, the outbox goes to the relay's mailbox,
  encrypted to the GM's key. The GM's app collects it when it next
  comes online. The same works the other way for GM changes to a player
  who is offline.
- The mailbox only holds encrypted blobs, with a size limit and an
  expiry time. The relay operator cannot read them.

### 5. The log

The authority's command log is the audit trail and the activity feed:
who did what, when, with the karma or nuyen it cost, and whether it was
an override (a change outside the normal rules, which the GM can look
at). The GM can revert any entry.

### 6. Identity and invites

- Each installation has a key pair (the iroh node key). There are no
  accounts or passwords, and the relay stores no personal data.
- The GM creates a campaign and an invite link:
  `chummer-rs://join/<gm-node-id>?campaign=<id>&invite=<token>`.
  Opening it registers the player's key with the campaign.
- Roles: GM (everything) and player (own characters only).

### 7. Processes

- `chummer-gui`: client, and the authority when its user hosts a
  campaign ("Host campaign").
- `chummer-relay`: the public relay plus mailbox. One small binary
  (with Docker and systemd examples) so anyone can host one.
- `chummer-authority` (optional): the authority without a GUI, for
  groups that want a campaign online all the time.

Implemented (local part):

- `Session::version()` is the number of commands in the log. It is not
  saved in the `.chum5`. Undo lowers it and redo raises it again, so a
  version always names one state of this session's history.
- `state_hash(ch)` is BLAKE3 of the canonical form: the saved XML
  without `<chummerrsversion>` and the export-only totals (`<totaless>`,
  attribute `<totalvalue>`), so the hash does not depend on the
  chummer-rs version or on whether the file was saved. Loading a saved
  file and saving it again gives the same XML for every test fixture.
  A saved and reloaded character has the live session's hash.
- `snapshot(ch)` is the canonical XML, LZMA-compressed as `.chum5lz`;
  `restore(bytes)` loads it.
- `replay(ch, engine, &[Envelope])` applies a log; the tests replay a
  live session's log (with undos and merged edits) onto the original
  file and get the same hash.

## Work order

1. Persistent creation warnings and guided creation.
2. macOS builds.
3. The command layer, with undo/redo (done).
4. Local GM screen and campaign file (players, NPCs, critters,
   initiative, damage), using commands.
5. Sync between two local instances: rebasing, hashes, snapshots.
6. iroh connections, the relay, invite links.
7. Outbox and mailbox.
8. The optional headless authority.
