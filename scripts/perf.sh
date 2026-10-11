#!/usr/bin/env bash
# The performance suite (docs/testing.md, "Performance tests"): release
# builds, the core and sync benchmarks (chummer-bench), the GUI frame
# times (an ignored test in chummer-gui), then the check against
# perf/budgets.toml and the previous run.
#
#   scripts/perf.sh --quick                 # a quick local check (~1 min after the build)
#   scripts/perf.sh                         # the full suite, as CI runs it
#   scripts/perf.sh --only core,sync        # a subset: core, sync, gui or name prefixes
#   scripts/perf.sh --previous old.json     # compare with an earlier results.json
#
# Results: target/perf/results.json (and core.json, gui.json). Settings
# and saved files go to target/perf/work, never to your own folders.
# PERF_JOBS (default 4) limits cargo's jobs; PERF_NICE (default 19) is
# the nice level. Exits 1 when a benchmark fails its check.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
OUT=${PERF_OUT:-$TARGET/perf}
JOBS=${PERF_JOBS:-4}
NICE="nice -n ${PERF_NICE:-19}"

QUICK=""
ONLY=""
PREVIOUS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --quick) QUICK=1 ;;
        --only) ONLY=$2; shift ;;
        --previous) PREVIOUS+=(--previous "$2"); shift ;;
        -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done
[ "$ONLY" = all ] && ONLY=""

# Which halves run: the GUI test for "gui…" prefixes, chummer-bench for
# the others.
RUN_GUI=""; RUN_CORE=""
if [ -z "$ONLY" ]; then
    RUN_GUI=1; RUN_CORE=1
else
    IFS=',' read -ra PARTS <<< "$ONLY"
    for p in "${PARTS[@]}"; do
        case "$p" in gui*) RUN_GUI=1 ;; "") ;; *) RUN_CORE=1 ;; esac
    done
fi

WORK=$OUT/work
rm -rf "$WORK"
mkdir -p "$OUT" "$WORK/config" "$WORK/data" "$WORK/state"
rm -f "$OUT/core.json" "$OUT/gui.json"
export XDG_CONFIG_HOME=$WORK/config XDG_DATA_HOME=$WORK/data XDG_STATE_HOME=$WORK/state
export CHUMMER_BENCH_DIR=$WORK
export CHUMMER_PERF_QUICK=${QUICK:-0}
export CHUMMER_PERF_ONLY=$ONLY

$NICE cargo build --release -j "$JOBS" -p chummer-bench
BENCH=$TARGET/release/chummer-bench
REPORTS=()

if [ -n "$RUN_CORE" ]; then
    $NICE "$BENCH" run ${QUICK:+--quick} --only "${ONLY:-all}" --out "$OUT/core.json"
    REPORTS+=("$OUT/core.json")
fi

if [ -n "$RUN_GUI" ]; then
    # Headless (no window): ui_tests.rs clears DISPLAY and WAYLAND_DISPLAY.
    CHUMMER_PERF_OUT=$OUT/gui.json RUST_TEST_THREADS=1 \
        $NICE cargo test --release -j "$JOBS" -p chummer-gui perf_frames -- --ignored --nocapture
    REPORTS+=("$OUT/gui.json")
fi

SUMMARY=()
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && SUMMARY=(--summary "$GITHUB_STEP_SUMMARY")
"$BENCH" check --budgets "$ROOT/perf/budgets.toml" "${PREVIOUS[@]}" --out "$OUT/results.json" "${SUMMARY[@]}" "${REPORTS[@]}"
