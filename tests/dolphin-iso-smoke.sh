#!/bin/sh
# Boot built dist/*.iso images in a Dolphin build. Used for the ISO-packaged
# examples (e.g. dvd-read). The local source build (nogui) is preferred; the
# flatpak headless build is a weaker fallback (ISOs boot there too).
#
# PASS = the ISO boots and stays up until timeout without a panic.
#
# Usage: tests/dolphin-iso-smoke.sh [ISO-NAMES...] (all dist/*.iso if none)
set -u

FAILURES=0
RUN_SECS=${RUN_SECS:-8}
USER_DIR=$(mktemp -d)

DOLPHIN_SRC=${DOLPHIN_NOGUI:-$HOME/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui}
if [ -x "$DOLPHIN_SRC" ]; then
    DOLPHIN_BIN="$DOLPHIN_SRC"
    DOLPHIN_FLATPAK=""
elif flatpak info org.DolphinEmu.dolphin-emu >/dev/null 2>&1; then
    DOLPHIN_BIN=""
    DOLPHIN_FLATPAK=1
else
    echo "SKIP: no dolphin build found"
    exit 77
fi
# coreutils `timeout`; macOS has it as `gtimeout` (brew install coreutils)
TIMEOUT=$(command -v timeout || command -v gtimeout) || {
    echo "SKIP: no timeout/gtimeout command" >&2
    exit 77
}

run_iso() {
    iso="$1"
    name=$(basename "$iso" .iso)
    log="$USER_DIR/$name.log"
    if [ -n "$DOLPHIN_FLATPAK" ]; then
        "$TIMEOUT" -s KILL -k 2 "$RUN_SECS" \
            flatpak run --env=QT_QPA_PLATFORM=offscreen \
            org.DolphinEmu.dolphin-emu -b -v Null -a HLE \
            -u "$USER_DIR/$name" -e "$iso" >"$log" 2>&1
    else
        "$TIMEOUT" -s KILL -k 2 "$RUN_SECS" \
            "$DOLPHIN_BIN" -v Null -a HLE \
            -u "$USER_DIR/$name" -e "$iso" >"$log" 2>&1
    fi
    rc=$?
    errs=$(grep -icE 'panicalert|assert|segfault|core dumped|DSI|ISI|illegal instruction|aborting|Could not boot' "$log" 2>/dev/null || true)
    if [ "$rc" = "124" ] && [ "$errs" = "0" ]; then
        echo "PASS $name"
    else
        echo "FAIL $name (rc=$rc errs=$errs)"
        tail -3 "$log" 2>/dev/null
        FAILURES=$((FAILURES+1))
    fi
}

cd "$(dirname "$0")/.." || exit 1

if [ "$#" -eq 0 ]; then
    set -- dist/*.iso
fi

[ -e "$1" ] || { echo "SKIP: no ISOs to boot (build with: make iso EXAMPLE=dvd-read)"; exit 77; }

for iso in "$@"; do
    run_iso "$iso"
done

rm -rf "$USER_DIR"
if [ "$FAILURES" -eq 0 ]; then
    echo "All ISO smoke tests passed."
    exit 0
else
    echo "$FAILURES ISO smoke test(s) failed." >&2
    exit 1
fi
