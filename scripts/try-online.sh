#!/usr/bin/env bash
# Try online campaigns on one machine: a local test relay, a GM app and a
# player app, each with its own profile (node key and settings).
#
#   scripts/try-online.sh          # set up (first run) and start everything
#   scripts/try-online.sh reset    # delete the test folder and start fresh
#
# Nothing in your own ~/.config/chummer-rs is used or changed. Everything
# lives in $TRY (default ~/.cache/chummer-rs-try). Ctrl+C stops it all.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TRY=${TRY:-$HOME/.cache/chummer-rs-try}
BIN=$ROOT/target/release

if [ "${1:-}" = reset ]; then
    rm -rf "$TRY"
fi

cargo build --release --manifest-path "$ROOT/Cargo.toml" \
    -p chummer-gui -p chummer-cli -p chummer-relay

mkdir -p "$TRY/relay" "$TRY/gm/config" "$TRY/gm/data" "$TRY/player/config" "$TRY/player/data"

# 1. The relay. Its first lines say which relay entry to use.
"$BIN/chummer-relay" --dev --data-dir "$TRY/relay" > "$TRY/relay/relay.log" 2>&1 &
RELAY=$!
PIDS=($RELAY)
trap 'kill "${PIDS[@]}" 2>/dev/null; wait 2>/dev/null' EXIT INT TERM

entry=""
for _ in $(seq 1 50); do
    entry=$(grep -o 'https://127\.0\.0\.1:3443/#[0-9a-f]*' "$TRY/relay/relay.log" | head -1 || true)
    [ -n "$entry" ] && break
    sleep 0.2
done
if [ -z "$entry" ]; then
    echo "The relay did not start; see $TRY/relay/relay.log" >&2
    exit 1
fi
cert="$TRY/relay/self-signed-cert.pem"

# 2. One profile each, pointing at the relay.
for who in gm player; do
    name=$([ $who = gm ] && echo "GM" || echo "Player")
    mkdir -p "$TRY/$who/config/chummer-rs"
    cat > "$TRY/$who/config/chummer-rs/online.json" <<EOF
{"name": "$name", "relays": ["$entry"], "ca_files": ["$cert"], "port": null}
EOF
done

# 3. A demo campaign (first run only).
camp="$TRY/gm/Demo.chummercampaign"
if [ ! -e "$camp" ]; then
    fx="$ROOT/crates/chummer-core/tests/fixtures"
    "$BIN/chummer-cli" campaign new "$camp" "Demo Run"
    "$BIN/chummer-cli" campaign add "$camp" "$fx/Soma (Career).chum5"
    "$BIN/chummer-cli" campaign add "$camp" "$fx/Munin_Career.chum5"
    "$BIN/chummer-cli" campaign add "$camp" "$fx/Gangerbean.chum5" --kind Enemy --copies 3 --group Halloweeners
fi

run() {
    local who=$1; shift
    XDG_CONFIG_HOME="$TRY/$who/config" XDG_DATA_HOME="$TRY/$who/data" \
        "$BIN/chummer-rs" "$@" > "$TRY/$who/app.log" 2>&1 &
    PIDS+=($!)
}
run gm "$camp"
run player

cat <<EOF

Running: test relay ($entry), the GM app and the player app.
The GM app has "Demo Run" open; the player app starts empty.

  GM:     tick "Host online" (right-hand panel), then "Invite player" → Copy.
  Player: File → Join Campaign…, paste the link, enter a name, Join.
  GM:     select Soma in the roster, set "Played by" to the player.
  Player: Character Roster tab → Campaigns → open Soma.

Then edit on either side and watch the other one, the Activity feed and
History. For play-by-post, untick "Host online" (or close the GM app),
edit as the player (the badge shows ⟳ and "via mailbox"), host again
and press "Check mail".

Logs and files: $TRY
Press Ctrl+C here to stop everything.
EOF
wait "$RELAY"
