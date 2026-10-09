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
- `.chum5`/`.chum5lz` import and export stay. The storage format does not
  have to match Chummer's (it is `.chumrs`, docs/file-format.md).

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
- Access is by capability: holding a key is the permission. Decided with
  the owner on 2026-10-08 (before 1.0 the protocol, links, relay and
  sidecar formats change without migration).
- The GM makes **one invite per player**, labelled ("Anna"). Each has
  its own member key (ed25519); the secret is in the link:
  `chummer-rs://join/<gm-node-id>?campaign=<id>&member=<secret>&gm=<campaign-key>[&relay=<url>]`.
  Links of the first protocol (`&invite=<token>`) are refused with "ask
  your GM for a new one".
- **Claim.** The first node that proves the member key joins and claims
  the invite: it becomes a member bound to that node id. The same link
  on another device is refused ("already used on another device").
- **Re-issue.** A new link for the same member (a new device): a new
  key; the invite is unclaimed again; the old node is out at once and its
  key refused ("replaced by a newer link"). Whoever claims the new link
  takes over the old node's characters.
- **Revoke.** The invite stays listed as revoked; its node is out (the
  live connection is closed) and its key no longer joins or mails.
  **Remove** deletes the invite (and takes its member out). Other members
  are not affected.
- **Expiry** (optional): an unclaimed invite stops working at a time the
  GM chose; claimed ones do not expire.
- **Assign on claim** (optional): the invite names a character that the
  claiming player gets.
- Members the GM adds by node id (a campaign file's owner, or
  `chummer-authority assign`) have no invite; they prove themselves by
  their node key. Revoked, removed and replaced nodes are remembered and
  not made members again from the campaign file.
- **Proof.** Live: the campaign protocol's hello carries the member
  key's public half; the host answers a random challenge, which the key
  signs together with the campaign, both node ids and the nonce
  (`chummer_net::campaign::hello_message`). By mail (the GM offline): the
  first mailed `Join` carries a `ClaimProof`, the key's signature over
  the campaign and both node ids; the mail is sealed to the GM and
  signed by the joining node, so only the GM sees it and it is no good
  for another node. The authority (`Authority::admit`) checks both the
  same way and maps member key -> node id -> member.
- **The GM's campaign key** signs what the GM's app puts into players'
  mailboxes. It is derived from the GM's node key, the campaign id and
  a generation (`derive_campaign_key`, BLAKE3 `derive_key`), so it needs
  no storage and `chummer-authority` can make links without the running
  host. Its public half is in the link (a play-by-post player registers
  it before any contact) and in every membership (current key first, the
  previous one during a rotation). `rotate-key` moves to the next
  generation; mail to a member stays signed with the previous key until
  that member was sent a membership naming the new one.
- **Relay mailbox access** (see [relay.md](relay.md#who-may-put-mail)):
  each mailbox owner registers which keys may put mail into its mailbox,
  per campaign (a scope). The GM's app registers every active invite key
  (claimed, or unclaimed and not expired, so a first join by mail gets
  in) and the node keys of members added by node id, at every mailbox
  round and right after an invite changed. Each player's app registers
  the GM's campaign keys. Leaving a campaign removes the player's scope.
- The authority keeps the invites with their secrets (the GM's own
  file), so the GM can copy an unclaimed link again. Members record when
  they were last seen (hello or mail).
- Roles: GM (everything) and player (own characters only). The GM keeps
  full power over characters; keys only govern who may talk to the
  campaign.

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

### 8. Sync (implemented: `chummer-sync`)

Work-order steps 5 and 7 and the messages for step 6.

- Messages (`chummer_sync::msg`): one version byte (`SYNC_VERSION`, 3 since the item commands changed (`MoveItem`, `Purchase::location`); 2
  since member keys: `Join` carries an optional `ClaimProof`, the
  membership names the invite's label and the GM's campaign keys, and
  `ServerMessage::Denied` tells a mailed join why it was refused), then
  postcard. `ClientMessage` (Join with the versions the client has,
  Submit, Resync) travels as chummer-net's `Request::Submit` payload and
  `ServerMessage` (Joined, Ack, Push, Membership, Error) as its answer
  or as a server push. In the mailbox they are wrapped in `MailMessage`.
- Every command travels as an `Op { id: OpId, env: Envelope }`. `OpId`
  is a random per-replica origin plus a counter; the authority remembers
  the outcome of the last 20000 per character, so a resubmitted outbox
  or a replayed mail runs once and gets the same answer. The envelope's
  `author` is replaced by the proven sender's node id.
- `Authority`: per character the live state, version, hash, the last
  256 log entries with the hash after each (the compaction: a client
  further behind gets a snapshot) and the op-id outcomes. A submit
  applies each command to the current state (this is the rebase);
  engine refusals are rejected with the reason; a player may only
  submit for characters they own. The `Ack` carries the authority's
  log from the batch's base (or a snapshot when the base is too old or
  its hash does not match), so the client never orders an ack against
  pushes. GM edits (`apply_local`) are logged with the GM as author and
  pushed to the owner. The activity feed holds every applied and refused
  command ("<author>: <text>").
- `Replica`: per character the confirmed state, version and hash, and
  the outbox. What the user sees is the confirmed state with the outbox
  applied. Every Ack or Push moves the confirmed state forward by the
  authority's entries, drops answered commands from the outbox, applies
  the rest again and compares hashes; a mismatch asks for a snapshot.
  Refused commands are kept for the UI until dismissed.
- Mailbox: outboxes are split into batches that fit one blob, every
  message is cut into chunks under the relay's blob limit (lowered when
  the relay answers `TooLarge`) and sealed to the recipient. The
  receiver checks the signer (a campaign member for the authority, the
  GM for a player) and reassembles chunks in a saved `Inbox`. Mail from
  a node that is not a member yet is read only if it is a complete,
  one-blob `Join` with a valid claim proof; nothing else of theirs is
  kept. Each put is signed by a key the recipient registered with the
  relay: the player's member key (or node key), the GM's campaign key.
  Registered keys are bound to the uploading node where it is known (a
  claimed invite to the claiming device, the GM's keys to the GM's
  node), so a copied key cannot put mail from another device. The
  authority mails offline members everything they were not sent (the
  membership, answers to mailed submits, pushes since the version last
  sent).
- Files: `Authority::save` and `Replica::save` write a 4-byte magic
  (`CRSA`, `CRSR`), a u16 format version and postcard; characters are
  stored as snapshots. Written through a temporary file and a rename.
- Transport: `AuthorityHost` (a chummer-net `CampaignHandler`: invite
  check, joins, submits, live pushes, mailbox rounds, periodic saves)
  and `PlayerSession` (connects, falls back to the mailbox, saves the
  replica after every change). `PlayerSession::edit_now` applies an
  edit at once and leaves sending it to a background task (for a UI
  thread); `keep_synced` syncs on start, then every minute and when a
  live connection drops.
- One endpoint per app (`chummer_sync::Node`): two endpoints with one
  key would push each other off the relay. It always accepts the
  campaign ALPN; a `HostSlot` hands connections to the hosted campaign
  or hangs up when nothing is hosted (players then use the mailbox).
  `stop_serving` closes the players' connections.
- Feed lines merge a burst of edits of one value (same author, same
  `coalesce_key`, less than 1.5 s apart, consecutive versions) into one
  line, in the authority and in replicas, as the local `Session` does
  for undo steps. The commands themselves stay separate.

### Revert (implemented)

The GM can take back any change still in a character's log window (the
last 256): `Authority::revert(engine, id, version)`.

- The authority keeps the character at the window's base (`base`, saved
  as a snapshot; the file format is 2). When the window drops an entry,
  the base takes it (commands are deterministic).
- Reverting version *v* takes back *v* and the earlier entries of its
  burst (`revert_range`). The state is rebuilt from the base: every
  entry in the window is applied again in order, except the reverted
  ones and those of earlier reverts still in force (a revert of a
  revert brings its entries back). An entry that no longer applies is
  dropped and named ("…, and 1 later change that needed it").
- The result is logged as a new version with `Command::Revert {
  snapshot, what, from, to }` and the GM as author. Versions only go
  forward, so replicas apply it like any other entry (the command sets
  the whole character from the snapshot) and the hashes agree. A revert
  that changes nothing is refused. `from..=to` is what later reverts read
  to rebuild around it; a revert whose range fell out of the window is
  replayed as its snapshot.
- Dedup is unaffected: a replayed copy of a reverted op is answered from
  the seen set and does not run again.

### The hosted campaign (implemented: `chummer_sync::hosted`)

The glue the GUI and `chummer-authority` share between a GM's
`.chummercampaign` file and the authority.

- Files: the GM opens `<name>.chummercampaign` only. The authority
  lives next to it in `<name>.authority` (format 3: per-player invites),
  made the first time the campaign is hosted. `<name>.invites` takes
  invite changes made by `chummer-authority invite ...` (`InviteOp`s,
  one JSON object per line: create, revoke, reissue, remove, rotate-key;
  mode 0600, it holds member keys) for a running host, or the next one,
  to apply. The host moves the file aside, applies the lines (they are
  idempotent), saves, then deletes it.
- A claim that gives a player a character (assign on claim, or a
  re-issued link) is written into the campaign file's owners by the host
  (`adopt_owner_changes`); until then `reconcile` keeps the authority's
  owner.
- Ids: `CharacterId` = the member's `MemberId` (32 hex digits); the
  campaign id is the same 128 bits in both crates.
- Once a sidecar exists every change to the campaign's characters goes
  through the authority, hosted or not, so the authority's characters
  are the newest. `reconcile` adds members that are new in the file
  (with the GM's open copy when there is one), drops removed ones, copies
  the campaign name, and takes an owner the file names (`owner` = a
  player's node id, or `"gm"` to take a character back); a member
  without one keeps the authority's owner. `write_back` (on save) puts
  the authority's characters into embedded members and saves linked
  ones whose file differs, and writes the owners into the file.
- Members with an owner are that player's characters; the rest (NPCs,
  critters, spirits) belong to the GM and are never sent to players.
- `HostedCampaign::open` loads or makes the authority, reconciles and
  starts an `AuthorityHost` saving to the sidecar. A sidecar made with
  another node key is refused (the key is the campaign's address).

### The GUI (implemented)

- `Doc` (an open character) has three backends: a local `Session`
  (files, campaigns not online), a GM character of the authority
  (`AuthorityHost::gm_edit`: applied at once, logged, pushed or mailed),
  and a player's replica copy (`PlayerSession::edit_now`). Online
  documents keep a copy of the backend's state (the backends are behind
  locks) and take the new state each frame when the version, hash or
  outbox changed.
- Undo and redo are for local documents only, and say why in a tooltip
  on online ones: a change is in the campaign log as soon as it is made
  and may have been sent; an inverse command does not exist for every
  command, and rolling a replica back would fight the authority. The GM
  reverts instead (History panel and the GM screen's feed).
- GM screen: Host online (makes the campaign online the first time,
  swaps every member's `Doc` to the authority, including tabs it lent),
  status and relay, Players & invites (Workspace: an inspector panel
  that pops out; Classic: a section of the right-hand panel; one drawing
  function for both): New invite (label, character on claim, expiry) ->
  link with Copy; per invite its state (not used yet / expires, joined
  from device and when, revoked, expired), online or last seen, the
  characters it plays, mail waiting from it, and Copy link, New link,
  Revoke and Remove (the last three confirmed); members added by node id
  with Remove; the mailbox's waiting and refused counts. Played by
  (owner), the authority's feed with author, character and Revert, Check
  mail with the last report, and a mailbox round on host start and every
  three minutes. Opening a
  campaign that has a sidecar backs it by the authority (not served
  until Host online).
- Player: File → Join Campaign (also prefilled from a `chummer-rs://`
  argument), the Campaigns list on the Character Roster tab and the
  Workspace home (state, the invite's label, why the GM's app refused
  the link, pending and refused counts, sync now, leave), characters as normal
  tabs with a badge (✔, ⟳N, ⚠N, ⏸), and the character's campaign log
  in History ("GM gave you 100 karma: note", `feed::for_owner`), with
  refused changes and Dismiss. Joined campaigns are kept in
  `campaigns/joined.json` and `campaigns/<id>.replica` in the config
  folder.
- Tools → Online Settings: name, node id, relay entries, trusted
  certificates (`chummer_net::config::OnlineSettings`, `online.json`).

### Headless authority (implemented: `chummer-authority`)

`run` serves a campaign file with the same glue; it applies invite
changes (and registers the new keys with the mailbox at once) and
re-reads the campaign file when it changes (every 5 s), does a mailbox
round every 3 minutes, writes owners that claims changed into the
campaign file, and writes the characters back every 5 minutes when they
changed and when it stops. `invite create --label --assign --expires`
prints a new player's link, `invite list` shows every invite's state,
`invite revoke`, `invite reissue` (prints the new link) and `invite
remove` name an invite by label or id; they write to the invites file.
`rotate-key` moves the GM's campaign key on. `assign` sets a member's
owner in the campaign file, `status` prints the authority's state. A systemd unit is in `packaging/authority/`.

## Campaigns

Implemented in `chummer_core::campaign` (work-order step 4, local only)
and shown by the GUI's GM screen (`gm_screen.rs`, `campaign_ui.rs`).

### Model

- `Campaign { id, name, created, gm_notes, members, encounters, log }`.
  `id` is a `CampaignId`: 128 bits as 32 lower-case hex digits, the
  same text form as `chummer_net::invite::CampaignId`, so the two convert
  with `to_string()` / `parse()` (core does not depend on chummer-net).
- `Member { id: MemberId, kind, name, character, player, owner, group,
  notes, visible_to_players }`. `kind` is Player, NPC, Enemy, Critter,
  Spirit, Drone, or any other name (kept as `Other`). `character` is
  `Embedded { xml }` (the canonical form, `command::canonical`) or
  `Linked { path }` (a `.chumrs`/`.chum5`/`.chum5lz`; a relative path is resolved
  against the campaign file's folder). `owner` is the owning player's
  node id (hex) for online campaigns; `visible_to_players` is stored for
  later and not used locally.
- `Encounter { id, name, combatants, round, pass, notes }`. A `Combatant`
  is a member (`member: Some(MemberId)`) or a quick one with its own
  `AdHocTrack` (physical and stun boxes). It keeps the stats of its last
  roll (base, dice, Edge, Reaction, Intuition), the score, the rolled
  dice and the acted, delayed, seized and blitzed flags. Initiative and
  damage math (`campaign::initiative`, `campaign::damage`) run outside
  commands: dice are rolled by the GM's app and the results reach a
  character as commands with fixed values (`SetPhysicalDamage`,
  `SetStunDamage`, `SpendEdge`).
- `log: Vec<LogItem { at, author, member, description }>` is the GM's
  activity feed. Locally it is filled from each member's `Session` log
  by `Campaign::absorb` with a `FeedCursor` per member: new entries are
  added, an edit merged into the last entry (a burst of typing) rewrites
  that line, and undone entries add "Undone: …". GM awards read "GM
  gave Ghost 100 karma: note" (`feed_text`). The GM's commands carry
  `author: "GM"`.

### File

A `.chummercampaign` file is a ZIP container shared with `.chumrs`
characters ([file-format.md](file-format.md)): `manifest.json` (format
`"chummer-rs campaign"`, schema version, BLAKE3 checksums),
`campaign.json` with the Campaign's fields, and each embedded member's
canonical XML as `members/<id>.xml` (mugshots as images). Every field
has a default and unknown fields are ignored; a different `format` or a
newer schema version is refused. Older files (one LZMA stream of the
JSON, `"version": 1`) and plain JSON still load. Saving writes a
temporary file, syncs it and renames it. Loading an embedded member and
hashing it gives the `state_hash` it had when saved.

### What the sync (steps 5–8) builds on

- Member ids are stable for a member's life and are the key for
  per-member versions, command logs, snapshots and hashes. Those live in
  chummer-sync, not in the campaign file: the file holds the members'
  current characters only.
- The GM's app is the authority for every member. In a local campaign
  each open member is a `Session` (in the GUI a `Doc` with author "GM");
  once the campaign is online it is a character of the `Authority` (see
  "The hosted campaign" above).
- `owner` (node id) is where "players see and edit only their own
  characters" is checked; `visible_to_players` is stored for showing
  NPCs to players later.
- Encounters stay GM-side state. Players get the feed of their own
  characters through the pushes' log entries.

## Work order

1. Persistent creation warnings and guided creation.
2. macOS builds.
3. The command layer, with undo/redo (done).
4. Local GM screen and campaign file (players, NPCs, critters,
   initiative, damage), using commands (done; see Campaigns).
5. Sync between two local instances: rebasing, hashes, snapshots (done:
   `chummer-sync`).
6. iroh connections, the relay, invite links (done: `chummer-net`,
   `chummer-relay`, and in the app: hosting, joining, settings).
7. Outbox and mailbox (done: `chummer-sync`, wired into the app).
8. The optional headless authority (done: `chummer-authority`).
