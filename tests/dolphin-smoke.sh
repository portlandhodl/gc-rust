#!/bin/sh
# Dolphin headless smoke test: boots every dist/*.dol for a few seconds.
#
# Usage:  tests/dolphin-smoke.sh [NAME ...]   (all examples if no names)
#
# A run PASSES if the process stays up until we stop it (the demos run
# forever) and Dolphin reports no boot failure, PanicAlert, DSI/ISI,
# illegal instruction or invalid memory access.
#
# Requires: a source build of Dolphin (nogui) at $DOLPHIN_NOGUI, default
# ~/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui.

set -u

FAILURES=0
RUN_SECS=${RUN_SECS:-6}
USER_DIR=$(mktemp -d)

DOLPHIN_NOGUI=${DOLPHIN_NOGUI:-$HOME/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui}
if [ ! -x "$DOLPHIN_NOGUI" ]; then
    echo "SKIP: no dolphin-emu-nogui at $DOLPHIN_NOGUI" >&2
    exit 77
fi
# coreutils `timeout`; macOS has it as `gtimeout` (brew install coreutils)
TIMEOUT=$(command -v timeout || command -v gtimeout) || {
    echo "SKIP: no timeout/gtimeout command" >&2
    exit 77
}

run_one() {
    name="$1"
    dol="dist/${name}.dol"
    [ -f "$dol" ] || { echo "FAIL $name: $dol not found"; FAILURES=$((FAILURES+1)); return 0; }
    log="$USER_DIR/out.log"
    rm -f "$log"
    # SIGTERM first (nogui shuts down cleanly), KILL if it lingers
    "$TIMEOUT" -k 2 "$RUN_SECS" \
        "$DOLPHIN_NOGUI" -p headless -v Null -a HLE \
        -u "$USER_DIR/user" -e "$PWD/$dol" >"$log" 2>&1
    rc=$?
    errs=$(grep -icE 'panicalert|assert|segfault|core dumped|DSI|ISI|illegal instruction|aborting|could not boot|invalid (read|write)' "$log" 2>/dev/null || true)
    if [ "$rc" = "124" ] && [ "${errs}" = "0" ]; then
        echo "PASS $name"
    else
        echo "FAIL $name (rc=$rc errs=$errs)"
        grep -iE 'panicalert|assert|DSI|ISI|illegal instruction|could not boot|invalid (read|write)' "$log" 2>/dev/null | head -5
        FAILURES=$((FAILURES+1))
    fi
}

cd "$(dirname "$0")/.." || exit 1

if [ "$#" -eq 0 ]; then
    set -- $(for f in dist/*.dol; do basename "$f" .dol; done)
fi

for n in "$@"; do
    run_one "$n"
done

rm -rf "$USER_DIR"
if [ "$FAILURES" -eq 0 ]; then
    echo "All Dolphin smoke tests passed."
    exit 0
else
    echo "$FAILURES Dolphin smoke test(s) failed." >&2
    exit 1
fi
