#!/bin/sh
# Dolphin headless smoke test: boots every dist/*.dol for a few seconds.
#
# Usage:  tests/dolphin-smoke.sh [NAME ...]   (all examples if no names)
#
# A run PASSES if the process stays up until we kill it (the demos run
# forever) and no PanicAlert/DSI/"Illegal instruction" lines appear.
#
# Requires: flatpak org.DolphinEmu.dolphin-emu

set -u

FAILURES=0
RUN_SECS=${RUN_SECS:-6}
USER_DIR=$(mktemp -d)

if ! flatpak info org.DolphinEmu.dolphin-emu >/dev/null 2>&1; then
    echo "SKIP: flatpak dolphin not installed" >&2
    exit 77
fi

run_one() {
    name="$1"
    dol="dist/${name}.dol"
    [ -f "$dol" ] || { echo "FAIL $name: $dol not found"; FAILURES=$((FAILURES+1)); return 0; }
    log="$USER_DIR/out.log"
    rm -f "$log"
    timeout -s KILL -k 2 "$RUN_SECS" \
        flatpak run --env=QT_QPA_PLATFORM=offscreen \
        org.DolphinEmu.dolphin-emu -b -v Null -a HLE \
        -u "$USER_DIR/user" -e "$dol" >"$log" 2>&1
    rc=$?
    errs=$(grep -icE 'panicalert|assert|segfault|core dumped|DSI|ISI|illegal instruction|aborting' "$log" 2>/dev/null || true)
    if [ "$rc" = "124" ] && [ "${errs}" = "0" ]; then
        echo "PASS $name"
    else
        echo "FAIL $name (rc=$rc errs=$errs)"
        grep -iE 'panicalert|assert|DSI|ISI|illegal instruction' "$log" 2>/dev/null | head -5
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
