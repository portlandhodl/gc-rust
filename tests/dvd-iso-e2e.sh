#!/bin/sh
# End-to-end DI test: boot dist/dvd-read.iso in headless Dolphin (full ISO
# boot path: BS2 HLE -> our Rust apploader -> gc-std main), watching the
# gc-std observation mailbox via Dolphin's MemoryWatcher socket.
#
# Uses Dolphin's DEFAULT user data directory for the watcher socket (this
# flatpak build ignores -u). Any MemoryWatcher dir content created is
# removed afterwards.
#
# Expects: heartbeat (0x80001c00), tag 1/2 = 0, tag 3 = 0xDA7A600D.
# Usage: tests/dvd-iso-e2e.sh
set -u

if ! flatpak info org.DolphinEmu.dolphin-emu >/dev/null 2>&1; then
    echo "SKIP: flatpak dolphin not installed" >&2
    exit 77
fi

DATA_DIR="$HOME/.var/app/org.DolphinEmu.dolphin-emu/data/dolphin-emu"
MW="$DATA_DIR/MemoryWatcher"
SOCK="$MW/MemoryWatcher.sock"
[ -f "$MW/Locations.txt" ] && { echo "FAIL: $MW/Locations.txt already exists (refusing to clobber)"; exit 1; }

mkdir -p "$MW"
cat > "$MW/Locations.txt" <<'EOF'
80001c00
80001c04
80001c08
80001c0c
EOF

SEEN=$(mktemp)
python3 "$(dirname "$0")/observe_watch.py" "$SOCK" 24 > "$SEEN" 2>&1 &
WATCHER=$!

timeout -s TERM -k 4 22 \
    flatpak run --env=QT_QPA_PLATFORM=offscreen \
    org.DolphinEmu.dolphin-emu -b -v Null -a HLE \
    -e dist/dvd-read.iso > /dev/null 2>&1

kill $WATCHER 2>/dev/null
wait $WATCHER 2>/dev/null

rm -f "$MW/Locations.txt" "$SOCK"
rmdir "$MW" 2>/dev/null

OUT=$(cat "$SEEN"); rm -f "$SEEN"

echo "-- mailbox trace --"
echo "$OUT"
echo "------------------"

case "$OUT" in
    *80001c00:*beef0013*) : ;;
    *) echo "FAIL: example never poked its heartbeat (boot failed)"; exit 1 ;;
esac
echo "$OUT" | grep -qE "80001c04:0*$" || { echo "FAIL: disc-ID read failed"; exit 1; }
case "$OUT" in
    *80001c0c:0*da7a600d*) echo "PASS: ISO boot + DI read verified"; exit 0 ;;
    *) echo "FAIL: readme cookie absent/wrong"; exit 1 ;;
esac
