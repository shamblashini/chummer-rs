# Testing

There are three layers:

1. **`cargo test --workspace`**: everything that runs in one process, with
   no Docker and no internet. CI runs it on every push.
2. **`tests/e2e/run.sh`**: the relay, the GM's authority and several
   players as separate programs in Docker, with network faults, crashes
   and hostile traffic. It takes about an hour, so it is run by hand or
   from the manual GitHub workflow "E2E".
3. **`scripts/perf.sh`**: the performance suite. Release builds, timings
   of the main operations, budgets and a comparison with the previous
   run. The GitHub workflow "Performance" runs it every night
   ([Performance tests](#performance-tests)).

## In-process tests

Run everything:

    cargo test --workspace

The suites added for robustness, by crate:

| Where | What it checks |
|---|---|
| `chummer-sync/tests/chaos.rs` | Randomised: 2-3 players and one authority. The network delays, reorders, duplicates and loses messages. Players edit offline and restart. The GM edits and reverts. The authority restarts, cleanly or by crashing back to its last save. At the end all copies must have the authority's version and hash. Every command must have run exactly once (karma equals the start plus the accepted amounts) or have been reported as refused. Each message goes through the wire encoding. |
| `chummer-sync/tests/rolls.rs` | Dice rolls: a player's roll reaches the authority once, live and by mail (resent, replayed); dice that do not fit their pool, or a roll for someone else's character, are answered and dropped; the GM's rolls reach players only when open; "players see each other's rolls"; the roll log is bounded and survives saves; journaled rolls come back after a crash. `net.rs` does it over real connections (`player_rolls_reach_the_gm_live_and_by_mail`). |
| `chummer-sync/tests/regressions.rs` | The bugs that chaos.rs found, each reduced to a small exchange. |
| `chummer-sync/tests/net_faults.rs` | Real connections through an in-process relay. Play-by-post across a relay restart. Mail lost at the relay is sent again. A member that stops reading does not stall the pushes to the others. |
| `chummer-sync/tests/invites.rs` | Per-player invites. On the authority: the first device claims an invite and the same link on another device is refused; a re-issued link refuses the old key everywhere and moves the characters to the new device; revoke cuts one member and leaves the others; unclaimed invites expire; mailed claims need a proof for the joining node; members added by node id mail with their node key; the invites file applies once; assign on claim reaches the campaign file; key rotation. Through an in-process relay: revoke closes the live connection, refuses the rejoin and the member's mail, and the other member goes on; a re-issued link supersedes the old one on a new device; a first join by mail with the character assigned, the claim binding the invite's key to that device, and a leaked link refused by the relay from another device; a rotated campaign key reaches an offline member. |
| `chummer-relay/tests/net.rs` | The campaign handshake with a member key: the challenge, a second device with a claimed key, a proof by another key or none. The mailbox: only the recipient reads and deletes its mail; puts from a stranger, unsigned, with a forged signature, replayed into another mailbox or by another uploader, or to a mailbox with no registrations are refused; the per-key cap; a key bound to a node refused from another; a registration without a key revokes it; registrations survive a relay restart. |
| `chummer-relay/tests/robustness.rs` | The mailbox store with damaged, empty or unreadable database files. Concurrent writers. Paging of big mail. Clock jumps against expiry and quotas. Hostile frames on raw mailbox streams. A relay restart while a client is connected. |
| `chummer-core/tests/chumrs.rs` | Every fixture `.chum5` (and the `.chum5lz` from Chummer) → `.chumrs` → `.chum5`/`.chum5lz` keeps the same `state_hash`, mugshots byte for byte; history and guide state survive; damaged files, hash mismatches and newer schema versions fail with a clear message; campaigns use the container. |
| `chummer-core/tests/fuzz_*.rs`, `prop_commands.rs`, `roundtrip_saves.rs` | Mutated `.chum5`, `.chum5lz`, `.chumrs`, campaign and settings files never panic; forged `.chumrs` manifests, ZIP bombs and archives with thousands of entries are refused within their limits. All fixtures save and load to the same state. Random command sequences are deterministic and survive postcard and JSON round trips. Undo and redo restore the exact state. |
| `chummer-cli/tests/cli_robustness.rs` | The CLI on missing, empty, binary, deep, non-UTF-8 and odd inputs: an error and no panic. |
| `chummer-gui/src/ui_tests.rs` | Headless egui: opens every character fixture in both layouts and draws every section for a few frames. Also the command palette, catalog add, undo/redo, and the GM's Players & invites panel (new invite, new link, revoke, new campaign key through its buttons) in Classic and Workspace; the Workspace dialogs (Add spell, Spell Options, Escape closes), Change Priority Selection in both layouts and the priority swap on the Attributes page, and the contact Sort menu. |

Tuning knobs (environment variables):

| Variable | Effect |
|---|---|
| `CHUMMER_CHAOS_SEEDS=n` | Runs n seeds per chaos test (the default is 1-2). |
| `CHUMMER_CHAOS_SEED=s` | Runs only seed s, to reproduce a failure. A failure prints its seed. |
| `CHUMMER_CHAOS_LOG=file` | On failure, writes the whole step log of the run to `file`. |
| `CHUMMER_FUZZ_ITERS=n` | Multiplies the fuzz rounds by n. `CHUMMER_FUZZ_SEED` sets the base seed. Failing inputs go to `/tmp/chummer-rs-fuzz-failures/`. |
| `CHUMMER_UI_FULL=1` | Runs the whole GUI matrix (every fixture × layout × section × size). |

Tests marked `#[ignore = "BUG: ..."]` reproduce known bugs that are not
fixed yet (there are none at the moment; the remaining ignored tests are
timing benchmarks, and `perf_frames`, the performance suite's GUI part).
Run them with `cargo test -- --ignored`.

## Performance tests

    scripts/perf.sh --quick          # a quick check: 3 samples, short backlogs
    scripts/perf.sh                  # the full suite, as the workflow runs it
    scripts/perf.sh --only core,sync # a subset: core, sync, gui, or name prefixes

The script builds in release (the shipped profile), then runs two parts:

- `chummer-bench run` (`crates/chummer-bench`): the engine, character
  files, commands, saves, print and online sync, on Serpent (the most
  items) and Ghile Mear (the biggest file, mostly mugshots). Engine start
  and peak memory are measured in a fresh child process.
- `cargo test --release -p chummer-gui perf_frames -- --ignored`
  (`chummer-gui/src/ui_tests/perf.rs`): headless frame times, as the
  other UI tests. No window opens.

The quick run takes about a minute after the build (the release build of
the GUI tests takes 5-10 minutes the first time). It uses `nice -n 19`
and 4 cargo jobs (`PERF_NICE`, `PERF_JOBS`). Settings and saved files go
to `target/perf/work`, not to your own folders. The results are in
`target/perf/`: `core.json` and `gui.json` from the two parts, and
`results.json`, which merges them and adds the check.

| Group | What is timed |
|---|---|
| `core.engine` | `Engine::load` in a fresh process; the first sheet after it |
| `core.load`, `core.compute` | loading a `.chum5` and a `.chumrs`; a full compute (`Engine::sheet`) |
| `core.command` | `SetField`, `AddItem` (a Medkit), `RaiseAttribute`, `RaiseSkill` and undo, on a fresh session each sample |
| `core.save` | the phases of `save_timing.rs` (clone, stamp, `to_document`, XML text, write, `.chum5lz`, `.chumrs` bytes) and whole saves to `.chum5` and `.chumrs`, written to a tmpfs (`/dev/shm`) so the disk's sync latency does not count; `disk_chum5` is one save to the disk, for the trend |
| `core.print` | the print XML; the default sheet through `xsltproc` (skipped with a note if there is none) |
| `core.memory` | peak RSS (VmHWM, Linux only) of a process with the engine and the 10 biggest fixtures |
| `sync.state_hash`, `sync.snapshot`, `sync.restore` | what the authority does per change and per resync |
| `sync.replica`, `sync.authority` | a player's edit, its submit and ack; a GM edit at the authority |
| `sync.catchup` | a replica that comes back after 200 changes (replayed, per change) and after 500 (beyond the log window, so a snapshot) |
| `sync.mail` | sealing and opening 20 offline edits, and a snapshot push |
| `gui.frame` | the mean, median and p95 frame of every page in Workspace and Classic, and the GM screen with six members |
| `gui.switch` | the first frame after going to a page; each page's first visit |
| `gui.catalog` | the gear catalog with a search shown, and the frame that takes a typed search |

Each benchmark has one warm-up run and then several samples (3 in quick
mode). The report gives the median, mean, p95, minimum, maximum and the
median absolute deviation, and keeps the samples.

### Budgets

`perf/budgets.toml` gives each benchmark an absolute budget: the largest
median allowed. A key is a benchmark name or a prefix that ends in `*`;
the longest key that matches wins. A median over its budget fails the
run. A benchmark without a budget gives a warning only. The first
budgets are about 4 times the medians of a quick local run (the file
says which), because GitHub's runners are slower and noisier. Change a
budget only for a reason, and give the reason in the commit message.

### Regression check

`chummer-bench check` compares each median with the previous successful
run on master (the workflow downloads its `perf-results` artifact):

- A change smaller than the benchmark's `min_delta` (in its unit, from
  the budgets file) is noise and is ignored.
- +25 % or more (`warn_pct`): a warning.
- +50 % or more (`fail_pct`): marked "slow". When the next run is also
  +50 % or more, it fails.
- While a benchmark is flagged, the next run is compared with the
  baseline from before the slowdown, not with the slow run. So a lasting
  slowdown cannot become the new normal by itself, and a single slow run
  (a noisy runner) only warns.

A failed run is not used as a baseline. To accept a slowdown on purpose,
run the workflow by hand with `baseline: reset`; that run is then the
baseline. Locally, `--previous target/perf/results.json` (copied
somewhere first) compares with an earlier run.

### The workflow

`.github/workflows/perf.yml` runs every night at 03:23 UTC and by hand
(Actions → Performance → Run workflow, with a subset and the baseline
choice). It never runs twice at the same time. It does not run on
pushes, so the normal CI stays as fast as before.

The job summary has a table: each benchmark's median, p95, budget, the
baseline (the previous median, or the one from before a slowdown that
is still flagged), the change in percent, the trend over the last five
successful runs (oldest first, this run in bold), and the status
(passed, warning, slow, failed, new or skipped). Failures are listed
above the table, notes on what a benchmark measures below it. The JSON
files are kept as the artifact `perf-results` for 90 days; they are the
history.

On the first run there is no previous artifact. The summary says so,
only the budgets are checked, and that run becomes the baseline.

## End-to-end tests in Docker

    tests/e2e/run.sh                 # all scenarios
    tests/e2e/run.sh netem pbp       # some of them
    tests/e2e/run.sh --list

You need Docker, cargo and python3. The script builds the release
binaries `chummer-relay`, `chummer-authority` and `chummer-testpeer` on
the host. It copies them into a small `debian:bookworm-slim` image
(`chummer-e2e:local`), so the host needs glibc 2.34 or newer.

When the script ends, it removes every container, network and image that
it made. It finds them by the label `chummer-e2e`. To keep the image for
the next run, set `E2E_KEEP=1`. The script removes the base image only
if it pulled it. It does not touch anything else in Docker.

The output, the container logs and `summary.txt` are in
`tests/e2e/out/<time>/`.

### Set-up for each scenario

- **Relay.** `chummer-relay` with a self-signed certificate for the name
  `relay`. Its data directory is on the host, so it survives restarts. Its
  mailbox key is made in advance, so the relay entry
  `https://relay:3443#<mailbox-id>` is known before it starts.
- **GM.** `chummer-authority run` serves a campaign file. `chummer-testpeer
  make-campaign` makes that file, with one Munin_Career character for each
  player, owned by that player (added by node id; their link carries the
  GM's campaign key). Players listed in `INVITED` get a character without
  an owner and their own invite instead (`chummer-authority invite create
  --assign`, run on the host), and join with that link. The GM does a
  mailbox round every 10 s.
- **Players.** Each player is `chummer-testpeer player`, which is built on
  chummer-sync's `PlayerSession`. It makes a scripted series of karma
  gains of known amounts, with large notes when asked, and syncs live or
  by mail. Every second it writes its state as JSON: version, hash,
  karma, outbox and mode.
- **Networks.** Each program is on its own Docker network. Only the relay
  is on all of them, so all traffic goes through the relay and no direct
  path is found.
- **Faults.** `tc netem` (the containers have `NET_ADMIN`), `docker
  network disconnect`, `docker kill -s KILL`, `docker restart`, a tmpfs
  data directory for a full disk, and byte-level damage to
  `mailbox.redb`.

### Checks

The check reads the authority's sidecar (`chummer-testpeer inspect`) and
the players' status files (`check.py`).

- **converge:** every player's copy has the authority's version and
  hash, an empty outbox and no pending resync.
- **strict:** converge, and the authority's karma for each character is
  the start value plus everything its player gained. This means no edit
  was lost or applied twice.

After each scenario the script also checks:

- that no container log contains `panicked`;
- that no container is in a crash loop;
- the relay's memory use.

### Scenarios

| Scenario | What happens | Check |
|---|---|---|
| baseline | 3 players, 15 edits each, all online | strict |
| netem | 100-200 ms delay with jitter, 10-15 % loss, 25 % reordering and 5 % duplication on the relay, GM and players | strict |
| partition | One player is disconnected from its network for 25 s and another drops 100 % of its packets. Both keep editing offline. | strict |
| pbp | The GM is offline. Players edit by mail, then the GM comes online. | strict |
| relay-restart | The relay restarts cleanly mid-game. Then it restarts again while the GM is away and mail is waiting. | strict |
| relay-kill | The relay is killed with SIGKILL twice while players mail large edits, so redb has to recover. | strict |
| gm-crash | The GM is killed with SIGKILL twice mid-game. Its journal must give back every change it acknowledged after its last save. | strict |
| player-crash | Players are killed with SIGKILL mid-edits and restarted. | strict |
| many-players | 12 players at once. | strict |
| long-pbp | 3 players make 80 edits each by mail, with large notes. The blob limit is 16 KiB, so mail travels in many chunks. | strict |
| mail-expiry | Mail expires 20 s after it is stored, and the GM comes online after that. Players must mail their commands again. | strict |
| abuse | Hostile mail: an oversized blob, unsealed bytes, a sealed message from a stranger, an unsigned put, malformed frames, a stranger's flood, a member's flood until the quota stops it, and a member's garbage. Then normal play. | the relay refuses a stranger's puts (none stored) and limits the member's; strict |
| disk-full | The GM registers its mailbox, then the relay's data is on a nearly full 3 MiB tmpfs while players mail. Then the disk is freed and the GM comes online. | the relay stays up; the mailbox stores mail again without a restart; strict |
| db-damage | `mailbox.redb` is damaged, and then made unreadable, between relay restarts. Mail in the damaged file is lost; players mail it again. | the relay starts again on the damaged file (it moves it aside); it refuses the unreadable one with an error; strict |
| stranger-flood | A node no player gave a key floods the GM's mailbox (1500 puts) and a player's (500), and sends an unsigned put, while the GM is away and players play by mail. | no stranger put is stored; the GM's mailbox counts the refusals; strict |
| revoked-player | P3 joins with its own invite (its character assigned on claim). Mid-game the GM runs `chummer-authority invite revoke`. P3 keeps editing. | P3 is told "revoked" and goes offline; its character does not change after the revoke; P3 is no longer a member; P1 and P2 strict |
| leaked-link | P2 joins with its own invite (the claim binds its key to P2's device at the relay); its link leaks. Another device tries it live; while the GM is away a third one tries it by mail, and a fourth floods with the leaked key (200 puts). | both intruders are refused ("claimed") and get no character; the third is refused by the relay, so the GM never sees its join; no leaked-key put is stored; the members stay GM, P1, P2; strict |

### Results

See the report of the run that added these tests (in the commit history)
for the results at that time. Run the script for the current state.

The run that added per-player invites (2026-10-08) passed baseline, pbp,
gm-crash, relay-kill, abuse, disk-full, db-damage, mail-expiry,
stranger-flood (1501 refused puts counted, none stored), revoked-player
and leaked-link (the intruder refused live and by mail; 38 leaked-key
puts stored before the cap of 40, the player's own mail counting
towards it).

The run that bound claimed invite keys to their device (2026-10-08)
passed leaked-link (X2 refused by the relay; 0 of 200 leaked-key puts
stored), baseline and pbp.
