#!/usr/bin/env bash
# End-to-end tests of the relay, its mailbox and online campaigns, in Docker.
#
#   tests/e2e/run.sh              # every scenario
#   tests/e2e/run.sh netem pbp    # some of them
#   tests/e2e/run.sh --list
#
# Each scenario starts a relay (chummer-relay, self-signed certificate), a
# GM (chummer-authority serving a campaign file) and players
# (chummer-testpeer player, built on chummer-sync's PlayerSession), each on
# its own private Docker network with only the relay on all of them, so all
# traffic goes through the relay. It then injects faults (tc netem, network
# partitions, kills and restarts, hostile mail, a full disk, a damaged
# database) and checks that every player's copy reaches the authority's
# version and hash, and (where no crash may lose acknowledged changes) that
# every edit was applied exactly once.
#
# Needs: docker, cargo, python3. Everything it creates in Docker carries the
# label chummer-e2e and is removed at the end (set E2E_KEEP=1 to keep the
# image). Output, logs and a summary: tests/e2e/out/<time>/.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
P=chummer-e2e
LABEL=chummer-e2e
IMG=chummer-e2e:local
BASE=debian:bookworm-slim
OUT="${E2E_OUT:-$HERE/out/$(date +%Y%m%d-%H%M%S)}"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
TP="$TARGET/release/chummer-testpeer"
FIXTURE="$ROOT/crates/chummer-core/tests/fixtures/Munin_Career.chum5"
UG="$(id -u):$(id -g)"
KARMA=50
TIMEOUT="${E2E_TIMEOUT:-300}"

ALL_SCENARIOS=(baseline netem partition pbp relay-restart relay-kill gm-crash player-crash many-players long-pbp mail-expiry abuse disk-full db-damage)

log() { printf '\e[1m[%s]\e[0m %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# ----- setup and teardown -----

BASE_PULLED=0
cleanup() {
    local ids
    ids=$(docker ps -aq --filter "label=$LABEL" || true)
    [ -n "$ids" ] && docker rm -f $ids >/dev/null 2>&1 || true
    ids=$(docker network ls -q --filter "label=$LABEL" || true)
    [ -n "$ids" ] && docker network rm $ids >/dev/null 2>&1 || true
    if [ -z "${E2E_KEEP:-}" ]; then
        docker rmi "$IMG" >/dev/null 2>&1 || true
        [ "$BASE_PULLED" = 1 ] && docker rmi "$BASE" >/dev/null 2>&1 || true
    fi
}

build() {
    log "building release binaries"
    (cd "$ROOT" && cargo build --release -p chummer-relay -p chummer-authority -p chummer-testpeer)
    local ctx="$OUT/.docker"
    mkdir -p "$ctx/bin"
    cp "$TARGET/release/chummer-relay" "$TARGET/release/chummer-authority" "$TP" "$ctx/bin/"
    if ! docker image inspect "$BASE" >/dev/null 2>&1; then BASE_PULLED=1; fi
    log "building image $IMG"
    docker build -q --label "$LABEL=1" -t "$IMG" -f "$HERE/Dockerfile" "$ctx" >/dev/null
    rm -rf "$ctx"
}

# ----- one world: relay, GM, players -----

S=""; D=""; ENTRY=""; LINK=""; NPLAYERS=0; MB=""
declare -a PIDS

# world <scenario> <players> [relay limits toml lines...]
world() {
    S="$1"; NPLAYERS="$2"; shift 2
    D="$OUT/$S"
    mkdir -p "$D/relay" "$D/gm/config" "$D/status" "$D/logs"
    docker network create --label "$LABEL=1" "$P-$S-gm" >/dev/null
    for i in $(seq 1 "$NPLAYERS"); do
        mkdir -p "$D/p$i"
        docker network create --label "$LABEL=1" "$P-$S-p$i" >/dev/null
    done
    docker network create --label "$LABEL=1" "$P-$S-x" >/dev/null
    MB=$("$TP" keygen "$D/relay/mailbox.key")
    {
        echo 'hostname = "relay"'
        echo 'data_dir = "/data"'
        echo 'http_bind = "0.0.0.0:3340"'
        echo 'https_bind = "0.0.0.0:3443"'
        echo 'qad_bind = "0.0.0.0:7842"'
        echo 'mailbox_port = 7843'
        echo '[tls]'
        echo 'cert_mode = "self-signed"'
        echo '[limits]'
        for l in "$@"; do echo "$l"; done
    } > "$D/relay/relay.toml"
    ENTRY="https://relay:3443#$MB"
    "$TP" keygen "$D/gm/gm.key" >/dev/null
    local owners=()
    for i in $(seq 1 "$NPLAYERS"); do
        owners+=(--owner "$("$TP" keygen "$D/p$i/node.key")")
    done
    LINK=$("$TP" make-campaign --out "$D/gm/campaign.chummercampaign" --character "$FIXTURE" --gm-key "$D/gm/gm.key" --karma "$KARMA" "${owners[@]}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["link"])')
}

# relay [extra docker args...]: starts the relay with its data in $D/relay.
relay_up() {
    docker run -d --name "$P-relay" --label "$LABEL=1" --user "$UG" --cap-add NET_ADMIN \
        --network "$P-$S-gm" --network-alias relay "$@" \
        -v "$D/relay:/data" "$IMG" chummer-relay --config /data/relay.toml >/dev/null
    for n in $(seq 1 "$NPLAYERS") x; do
        docker network connect --alias relay "$P-$S-${n/#[0-9]*/p$n}" "$P-relay"
    done
    wait_for "relay certificate" 30 test -s "$D/relay/self-signed-cert.pem"
    wait_for "relay mailbox online" 60 logs_have "$P-relay" "mailbox node connected"
}

gm_up() {
    docker run -d --name "$P-gm" --label "$LABEL=1" --user "$UG" --cap-add NET_ADMIN \
        --network "$P-$S-gm" -e XDG_CONFIG_HOME=/gm/config -e HOME=/gm \
        -v "$D/gm:/gm" -v "$D/relay/self-signed-cert.pem:/ca.pem:ro" -v "$ROOT/resources:/resources:ro" \
        "$IMG" chummer-authority --key /gm/gm.key run /gm/campaign.chummercampaign \
        --relay "$ENTRY" --ca /ca.pem --mail-every 10 --write-back-every 15 >/dev/null
}

# player <i> <edits> [extra testpeer args...]
player_up() {
    local i="$1" edits="$2"; shift 2
    docker run -d --name "$P-p$i" --label "$LABEL=1" --user "$UG" --cap-add NET_ADMIN \
        --network "$P-$S-p$i" -e HOME=/state \
        -v "$D/p$i:/state" -v "$D/status:/status" -v "$D/relay/self-signed-cert.pem:/ca.pem:ro" -v "$ROOT/resources:/resources:ro" \
        "$IMG" chummer-testpeer player --key /state/node.key --link "$LINK" --relay "$ENTRY" --ca /ca.pem \
        --state /state --status "/status/p$i.json" --name "P$i" --edits "$edits" --every-ms "${EVERY_MS:-400}" --sync-secs 3 "$@" >/dev/null
}

players_up() {
    local edits="$1"; shift
    for i in $(seq 1 "$NPLAYERS"); do player_up "$i" "$edits" "$@"; done
}

# logs_have <container> <pattern>
logs_have() { docker logs "$1" 2>&1 | grep -q "$2"; }

wait_for() {
    local what="$1" secs="$2"; shift 2
    local end=$((SECONDS + secs))
    until "$@" 2>/dev/null; do
        if [ $SECONDS -ge $end ]; then log "timed out waiting for $what"; return 1; fi
        sleep 1
    done
}

# tc netem on every interface of a container.
netem() {
    local c="$1"; shift
    docker exec --user 0 "$c" sh -c "for d in \$(ls /sys/class/net | grep -v '^lo\$'); do tc qdisc replace dev \$d root netem $*; done"
}

# The authority's state as JSON (from its sidecar, saved every 2 s).
inspect() { "$TP" inspect "$D/gm/campaign.authority" > "$D/authority.json"; }

# check <strict|converge> [timeout]: waits until the check holds.
converge() {
    local mode="$1" secs="${2:-$TIMEOUT}"
    local end=$((SECONDS + secs)) rc=1
    while :; do
        if inspect 2>/dev/null; then
            set +e
            python3 "$HERE/check.py" "$D/authority.json" "$D/status" "$KARMA" "$mode" > "$D/check.txt"
            rc=$?
            set -e
            [ $rc -ne 1 ] && break
        fi
        [ $SECONDS -ge $end ] && break
        sleep 3
    done
    cat "$D/check.txt" >&2 || true
    return $rc
}

# Common checks after a scenario: no panics, no crash loops, memory.
health() {
    local bad=0 c
    for c in $(docker ps -a --filter "label=$LABEL" --format '{{.Names}}'); do
        docker logs "$c" > "$D/logs/${c#$P-}.log" 2>&1 || true
        if grep -q "panicked" "$D/logs/${c#$P-}.log"; then log "PANIC in $c"; bad=1; fi
    done
    if docker ps --filter "name=^$P-relay\$" --format '{{.Names}}' | grep -q relay; then
        NOTE="${NOTE:-} relay mem $(docker stats --no-stream --format '{{.MemUsage}}' "$P-relay" | cut -d/ -f1 | tr -d ' ')"
    fi
    return $bad
}

teardown() {
    local ids
    ids=$(docker ps -aq --filter "label=$LABEL")
    [ -n "$ids" ] && docker rm -f $ids >/dev/null
    for n in $(docker network ls --filter "label=$LABEL" --format '{{.Name}}' | grep "^$P-$S-"); do
        docker network rm "$n" >/dev/null
    done
}

# ----- scenarios -----
# Each prints nothing on success and returns non-zero on failure; NOTE
# collects remarks for the summary.

# Waits until every player has its character, then stops the GM.
gm_away_after_join() {
    wait_for "players have their characters" 180 python3 -c "import json,glob,sys; s=[json.load(open(f)) for f in glob.glob('$D/status/*.json')]; sys.exit(0 if len(s)==$NPLAYERS and all(x['copies'] for x in s) else 1)"
    log "the GM goes offline: players go on by mail"
    docker stop -t 10 "$P-gm" >/dev/null
}

players_done() {
    wait_for "players done" "$1" python3 -c "import json,glob,sys; s=[json.load(open(f)) for f in glob.glob('$D/status/*.json')]; sys.exit(0 if all(x['done'] for x in s) else 1)"
}

sc_baseline() {
    world baseline 3
    relay_up; gm_up; players_up 15
    converge strict
}

sc_netem() {
    world netem 3
    relay_up; gm_up
    netem "$P-relay" delay 150ms 50ms loss 10% reorder 25% 50%
    netem "$P-gm" delay 100ms 50ms loss 10%
    players_up 15
    for i in $(seq 1 3); do netem "$P-p$i" delay 200ms 80ms loss 15% reorder 25% 50% duplicate 5%; done
    converge strict 420
}

sc_partition() {
    world partition 3
    relay_up; gm_up; players_up 30
    sleep 6
    log "cutting P1 and P2 off for 25 s (they keep editing)"
    docker network disconnect "$P-$S-p1" "$P-p1"
    netem "$P-p2" loss 100%
    sleep 25
    docker network connect "$P-$S-p1" "$P-p1"
    docker exec --user 0 "$P-p2" sh -c 'for d in $(ls /sys/class/net | grep -v "^lo$"); do tc qdisc del dev $d root; done'
    converge strict
}

sc_pbp() {
    world pbp 3
    relay_up; gm_up
    EVERY_MS=1500 players_up 15
    gm_away_after_join
    players_done 180
    sleep 6
    log "the GM comes online"
    docker start "$P-gm" >/dev/null
    converge strict
}

sc_relay-restart() {
    world relay-restart 3
    relay_up; gm_up; players_up 30
    sleep 8
    log "restarting the relay (clean stop)"
    docker restart -t 10 "$P-relay" >/dev/null
    sleep 5
    log "the GM goes offline, the relay restarts again while mail waits"
    docker stop -t 10 "$P-gm" >/dev/null
    sleep 10
    docker restart -t 10 "$P-relay" >/dev/null
    sleep 5
    docker start "$P-gm" >/dev/null
    converge strict
}

sc_relay-kill() {
    world relay-kill 3
    relay_up; gm_up
    EVERY_MS=800 players_up 25 --big 20000
    gm_away_after_join
    sleep 6
    log "killing the relay (SIGKILL) while players mail"
    docker kill -s KILL "$P-relay" >/dev/null
    sleep 3
    docker start "$P-relay" >/dev/null
    sleep 8
    docker kill -s KILL "$P-relay" >/dev/null
    docker start "$P-relay" >/dev/null
    sleep 8
    docker start "$P-gm" >/dev/null
    converge strict
}

sc_gm-crash() {
    world gm-crash 3
    relay_up; gm_up; players_up 30
    sleep 7
    log "killing the GM (SIGKILL); its journal keeps what it acknowledged"
    docker kill -s KILL "$P-gm" >/dev/null
    sleep 5
    docker start "$P-gm" >/dev/null
    sleep 8
    docker kill -s KILL "$P-gm" >/dev/null
    docker start "$P-gm" >/dev/null
    converge strict
}

sc_player-crash() {
    world player-crash 3
    relay_up; gm_up; players_up 30
    sleep 5
    log "killing P1 and P2 (SIGKILL) mid-edits, then restarting them"
    docker kill -s KILL "$P-p1" "$P-p2" >/dev/null
    sleep 4
    docker start "$P-p1" "$P-p2" >/dev/null
    sleep 5
    docker kill -s KILL "$P-p1" >/dev/null
    docker start "$P-p1" >/dev/null
    converge strict
}

sc_many-players() {
    world many-players 12
    relay_up; gm_up
    EVERY_MS=700 players_up 10
    converge strict 480
}

sc_long-pbp() {
    # Small blobs: snapshots and large commands travel in many chunks.
    world long-pbp 3 'max_blob_bytes = 16384'
    relay_up; gm_up
    EVERY_MS=300 players_up 80 --big 6000
    gm_away_after_join
    players_done 240
    sleep 6
    log "the GM comes online"
    docker start "$P-gm" >/dev/null
    converge strict 420
}

sc_mail-expiry() {
    # Mail expires 20 s after it is stored; the GM is away longer.
    world mail-expiry 2 'expiry_secs = 20'
    relay_up; gm_up
    EVERY_MS=1000 players_up 8 --remail-secs 45
    gm_away_after_join
    players_done 60
    sleep 25
    log "the relay restarts (its purge runs at start); the mail has expired"
    docker restart -t 5 "$P-relay" >/dev/null
    sleep 5
    log "the GM comes online; players must mail again"
    docker start "$P-gm" >/dev/null
    converge strict 240
}

sc_abuse() {
    world abuse 2 'max_messages_per_recipient = 300' 'max_messages_per_sender_per_day = 150'
    relay_up; gm_up
    local gm; gm=$("$TP" keygen "$D/gm/gm.key")
    mkdir -p "$D/x"
    "$TP" keygen "$D/x/node.key" >/dev/null
    cp "$D/p1/node.key" "$D/x/member.key"
    for m in oversized garbage forged frames flood member-garbage; do
        local key=/x/node.key
        [ "$m" = member-garbage ] && key=/x/member.key
        docker run --rm --label "$LABEL=1" --user "$UG" --network "$P-$S-x" \
            -v "$D/x:/x" -v "$D/relay/self-signed-cert.pem:/ca.pem:ro" "$IMG" \
            chummer-testpeer abuse --key "$key" --relay "$ENTRY" --ca /ca.pem --target "$gm" --mode "$m" --count 400 \
            > "$D/abuse-$m.json" 2> "$D/logs/abuse-$m.log" || { log "abuse $m failed"; return 1; }
    done
    grep -q 'too large' "$D/abuse-oversized.json" || { log "oversized blob was not refused"; return 1; }
    grep -q 'limit\|full' "$D/abuse-flood.json" || { log "the flood was never limited"; return 1; }
    grep -q 'stored' "$D/abuse-frames.json" || { log "mailbox unusable after bad frames"; return 1; }
    players_up 10
    converge strict
    sleep 12
    grep -q "dropped" "$D/logs/gm.log" 2>/dev/null || docker logs "$P-gm" 2>&1 | grep -q "dropped" || NOTE="(GM did not log dropped mail)"
}

sc_disk-full() {
    # The relay's data on a 3 MiB tmpfs, nearly full.
    world disk-full 2
    cp "$D/relay/relay.toml" "$D/relay.toml"
    docker run -d --name "$P-relay" --label "$LABEL=1" --cap-add NET_ADMIN --network "$P-$S-gm" --network-alias relay \
        --tmpfs /data:size=3m,uid="$(id -u)",gid="$(id -g)" --user "$UG" \
        -v "$D/relay.toml:/etc/relay.toml:ro" -v "$D/relay/mailbox.key:/etc/mailbox.key:ro" "$IMG" \
        sh -c 'cp /etc/mailbox.key /data/mailbox.key && exec chummer-relay --config /etc/relay.toml' >/dev/null
    for n in $(seq 1 "$NPLAYERS") x; do docker network connect --alias relay "$P-$S-${n/#[0-9]*/p$n}" "$P-relay"; done
    wait_for "relay up" 30 docker exec "$P-relay" test -s /data/self-signed-cert.pem
    docker exec "$P-relay" cat /data/self-signed-cert.pem > "$D/relay/self-signed-cert.pem"
    wait_for "relay mailbox online" 60 logs_have "$P-relay" "mailbox node connected"
    log "filling the disk"
    docker exec "$P-relay" sh -c 'dd if=/dev/zero of=/data/fill bs=1k count=100000 2>/dev/null; df -k /data | tail -1'
    gm_up
    EVERY_MS=1000 players_up 10 --big 30000
    gm_away_after_join
    players_done 90
    sleep 10
    docker logs "$P-relay" 2>&1 | grep -iE "error|space|full" | tail -3 >&2 || true
    docker ps --filter "name=^$P-relay\$" --format '{{.Status}}' | grep -q Up || { log "the relay died on a full disk"; return 1; }
    log "freeing the disk: the mailbox must work again without a restart"
    docker exec "$P-relay" rm -f /data/fill
    local gm; gm=$("$TP" keygen "$D/gm/gm.key")
    mkdir -p "$D/x"; "$TP" keygen "$D/x/node.key" >/dev/null
    docker run --rm --label "$LABEL=1" --user "$UG" --network "$P-$S-x" -v "$D/x:/x" -v "$D/relay/self-signed-cert.pem:/ca.pem:ro" "$IMG" \
        chummer-testpeer abuse --key /x/node.key --relay "$ENTRY" --ca /ca.pem --target "$gm" --mode garbage > "$D/after-free.json"
    grep -q stored "$D/after-free.json" || { log "the mailbox still fails after the disk was freed: $(cat "$D/after-free.json")"; return 1; }
    log "the GM comes online"
    docker start "$P-gm" >/dev/null
    converge strict 300
}

sc_db-damage() {
    world db-damage 2
    relay_up; gm_up
    EVERY_MS=1000 players_up 6 --remail-secs 30
    gm_away_after_join
    players_done 60
    sleep 5
    docker stop -t 5 "$P-relay" >/dev/null
    log "damaging the mailbox database"
    python3 -c "
import sys
p='$D/relay/mailbox.redb'
b=bytearray(open(p,'rb').read())
for i in range(0, len(b), 509): b[i] ^= 0xa5
open(p,'wb').write(b)"
    docker start "$P-relay" >/dev/null
    sleep 8
    local st; st=$(docker inspect -f '{{.State.Status}} {{.State.ExitCode}}' "$P-relay")
    NOTE="relay after damage: $st"
    docker logs --tail 5 "$P-relay" 2>&1 | sed 's/^/    /' >&2
    if [ "${st%% *}" != running ]; then
        log "the relay does not start on a damaged database (exit ${st##* })"
        return 1
    fi
    log "unreadable database file"
    docker stop -t 5 "$P-relay" >/dev/null
    chmod 000 "$D/relay/mailbox.redb"
    docker start "$P-relay" >/dev/null; sleep 5
    st=$(docker inspect -f '{{.State.Status}} {{.State.ExitCode}}' "$P-relay")
    NOTE="$NOTE; unreadable db: $st"
    chmod 600 "$D/relay/mailbox.redb"
    docker start "$P-relay" >/dev/null 2>&1 || true
    sleep 5
    docker start "$P-gm" >/dev/null
    converge strict 240
}

# ----- main -----

if [ "${1:-}" = --list ]; then printf '%s\n' "${ALL_SCENARIOS[@]}"; exit 0; fi
SCENARIOS=("$@")
[ ${#SCENARIOS[@]} -eq 0 ] && SCENARIOS=("${ALL_SCENARIOS[@]}")
mkdir -p "$OUT"
trap cleanup EXIT
cleanup
build
FAILED=0
printf '%-15s %-6s %6s  %s\n' scenario result secs notes > "$OUT/summary.txt"
for s in "${SCENARIOS[@]}"; do
    declare -F "sc_$s" >/dev/null || { log "no scenario $s"; exit 2; }
    log "=== $s"
    NOTE=""; start=$SECONDS; result=PASS
    set +e
    ( set -e; "sc_$s"; echo "$NOTE" > "$OUT/$s.note" ); rc=$?
    set -e
    S="$s"; D="$OUT/$s"
    NOTE="$(cat "$OUT/$s.note" 2>/dev/null || true)"
    [ $rc -ne 0 ] && result=FAIL
    health || result=FAIL
    teardown || true
    [ "$result" = FAIL ] && FAILED=1
    printf '%-15s %-6s %6d  %s\n' "$s" "$result" $((SECONDS - start)) "$NOTE" | tee -a "$OUT/summary.txt" >&2
done
log "summary in $OUT/summary.txt"
cat "$OUT/summary.txt"
exit $FAILED
