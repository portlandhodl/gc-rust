#!/bin/sh
# End-to-end memory-card persistence test:
#   1. boot dist/memcard.dol in headless Dolphin on a fresh user dir
#      (the example creates "gcrust-save.sav", writes a sentinel, bumps a
#      counter, re-reads to verify)
#   2. scan the persisted MemoryCardA raw image for the sentinel
#
# Usage: tests/memcard-persist.sh
#
# NOTE: this needs a Dolphin build that instantiates EXI devices and
# flushes memcard images to disk in headless batch mode. The flatpak
# build in CI containers commonly does not create the card in headless
# mode — run it on a desktop Dolphin install instead. (Not part of
# `make check`; the card driver itself is host-tested in
# tools/gc-host-tests against a faithful card-image emulator.)

set -u

if ! flatpak info org.DolphinEmu.dolphin-emu >/dev/null 2>&1; then
    echo "SKIP: flatpak dolphin not installed" >&2
    exit 77
fi

USER_DIR=$(mktemp -d)
LOG="$USER_DIR/out.log"
RC=1

# Headless profiles default to "no device in slot A" — request a raw
# memory card (EXIDeviceType::MemoryCard = 1).
mkdir -p "$USER_DIR/user/Config"
printf '[Core]\nSlotA = 1\n' > "$USER_DIR/user/Config/Dolphin.ini"

timeout -s KILL -k 2 15 \
    flatpak run --env=QT_QPA_PLATFORM=offscreen \
    org.DolphinEmu.dolphin-emu -b -v Null -a HLE \
    -u "$USER_DIR/user" -e dist/memcard.dol >"$LOG" 2>&1

rc=$?
if [ "$rc" != "124" ] && [ "$rc" != "137" ]; then
    echo "FAIL: dolphin exited early (rc=$rc)"
    grep -icE 'panicalert|DSI|ISI|illegal instruction' "$LOG" 2>/dev/null
elif grep -iaqE 'panicalert|DSI|ISI|illegal instruction' "$LOG"; then
    echo "FAIL: crash signature in log"
else
    # card images land in <user>/GC/
    RAW=$(ls "$USER_DIR"/user/GC/MemoryCardA*.raw 2>/dev/null | head -1)
    if [ -n "$RAW" ] && grep -aq "GC-RUST-SAVE v01" "$RAW"; then
        echo "PASS: sentinel found in $RAW (write persisted through EXI)"
        RC=0
    else
        echo "FAIL: no sentinel in card image (raw: ${RAW:-none})"
        grep -i "memcard\|card" "$LOG" | head -5
    fi
fi

rm -rf "$USER_DIR"
exit $RC
