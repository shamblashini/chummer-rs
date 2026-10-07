# Testing

There are two layers:

1. **`cargo test --workspace`**: everything that runs in one process, with
   no Docker and no internet. CI runs it on every push.
2. **`tests/e2e/run.sh`**: the relay, the GM's authority and several
   players as separate programs in Docker, with network faults, crashes
   and hostile traffic. It takes about an hour, so it is run by hand or
   from the manual GitHub workflow "E2E".

## In-process tests

Run everything:

    cargo test --workspace

The suites added for robustness, by crate:

| Where | What it checks |
|---|---|
| `chummer-sync/tests/chaos.rs` | Randomised: 2-3 players and one authority. The network delays, reorders, duplicates and loses messages. Players edit offline and restart. The GM edits and reverts. The authority restarts, cleanly or by crashing back to its last save. At the end all copies must have the authority's version and hash. Every command must have run exactly once (karma equals the start plus the accepted amounts) or have been reported as refused. Each message goes through the wire encoding. |
| `chummer-sync/tests/regressions.rs` | The bugs that chaos.rs found, each reduced to a small exchange. |
| `chummer-sync/tests/net_faults.rs` | Real connections through an in-process relay. Play-by-post across a relay restart. Mail lost at the relay is sent again. A member that stops reading does not stall the pushes to the others. |
| `chummer-relay/tests/robustness.rs` | The mailbox store with damaged, empty or unreadable database files. Concurrent writers. Paging of big mail. Clock jumps against expiry and quotas. Hostile frames on raw mailbox streams. A relay restart while a client is connected. |
| `chummer-core/tests/fuzz_*.rs`, `prop_commands.rs`, `roundtrip_saves.rs` | Mutated `.chum5`, `.chum5lz`, campaign and settings files never panic. All fixtures save and load to the same state. Random command sequences are deterministic and survive postcard and JSON round trips. Undo and redo restore the exact state. |
| `chummer-cli/tests/cli_robustness.rs` | The CLI on missing, empty, binary, deep, non-UTF-8 and odd inputs: an error and no panic. |
| `chummer-gui/src/ui_tests.rs` | Headless egui: opens every character fixture in both layouts and draws every section for a few frames. Also the command palette, catalog add, and undo/redo. |

Tuning knobs (environment variables):

| Variable | Effect |
|---|---|
| `CHUMMER_CHAOS_SEEDS=n` | Runs n seeds per chaos test (the default is 1-2). |
| `CHUMMER_CHAOS_SEED=s` | Runs only seed s, to reproduce a failure. A failure prints its seed. |
| `CHUMMER_CHAOS_LOG=file` | On failure, writes the whole step log of the run to `file`. |
| `CHUMMER_FUZZ_ITERS=n` | Multiplies the fuzz rounds by n. `CHUMMER_FUZZ_SEED` sets the base seed. Failing inputs go to `/tmp/chummer-rs-fuzz-failures/`. |
| `CHUMMER_UI_FULL=1` | Runs the whole GUI matrix (every fixture × layout × section × size). |

Tests marked `#[ignore = "BUG: ..."]` reproduce known bugs that are not
fixed yet. Run them with `cargo test -- --ignored`.

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
  player, owned by that player. The GM does a mailbox round every 10 s.
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
| abuse | Hostile mail: an oversized blob, unsealed bytes, a sealed message from a non-member, malformed frames, a flood until the quota stops it, and a member's garbage. Then normal play. | the relay refuses or limits each one; strict |
| disk-full | The relay's data is on a nearly full 3 MiB tmpfs while players mail. Then the disk is freed and the GM comes online. | the relay stays up; the mailbox stores mail again without a restart; strict |
| db-damage | `mailbox.redb` is damaged, and then made unreadable, between relay restarts. Mail in the damaged file is lost; players mail it again. | the relay starts again on the damaged file (it moves it aside); it refuses the unreadable one with an error; strict |

### Results

See the report of the run that added these tests (in the commit history)
for the results at that time. Run the script for the current state.
