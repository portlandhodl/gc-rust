#!/bin/sh
# USB Gecko end-to-end test: boot dist/usb-gecko.dol in headless Dolphin
# with SlotB=Gecko, connect to Dolphin's gecko TCP server (port 55020),
# and require the example's banner bytes to arrive.
#
# Usage: tests/usbgecko-e2e.sh
#
# NOTE: needs a Dolphin build exposing the Gecko EXI device + TCP server
# headless; the flatpak build doesn't create it here. Run on a desktop
# Dolphin (Config: SlotB = Gecko) instead. (Not part of `make check`.)

set -u

if ! flatpak info org.DolphinEmu.dolphin-emu >/dev/null 2>&1; then
    echo "SKIP: flatpak dolphin not installed" >&2
    exit 77
fi

USER_DIR=$(mktemp -d)
RC=1

# 7 = EXIDeviceType::Gecko (string on the wire is the gecko debug channel)
mkdir -p "$USER_DIR/user/Config"
printf '[Core]\nSlotB = 7\n' > "$USER_DIR/user/Config/Dolphin.ini"

timeout -s KILL -k 2 20 \
    flatpak run --env=QT_QPA_PLATFORM=offscreen \
    org.DolphinEmu.dolphin-emu -b -v Null -a HLE \
    -u "$USER_DIR/user" -e dist/usb-gecko.dol >"$USER_DIR/dol.log" 2>&1 &
DOL_PID=$!

# give the emulator a moment to start the gecko server, then listen
OUT=$(python3 - <<'PYEOF'
import socket, time, sys
deadline = time.time() + 17
buf = b""
while time.time() < deadline:
    try:
        s = socket.create_connection(("127.0.0.1", 55020), timeout=2)
        s.settimeout(2)
        end = time.time() + 10
        while time.time() < end:
            try:
                data = s.recv(4096)
                if data:
                    buf += data
                    if b"GC-RUST GECKO OK" in buf:
                        print("BANNER-FOUND")
                        sys.stdout.flush()
                        raise SystemExit(0)
            except socket.timeout:
                pass
        s.close()
    except (ConnectionRefusedError, OSError):
        time.sleep(0.5)
print(buf[-200:])
PYEOF
)

kill -9 $DOL_PID 2>/dev/null
wait $DOL_PID 2>/dev/null

case "$OUT" in
    *BANNER-FOUND*) echo "PASS usb-gecko: got banner over gecko TCP"; RC=0 ;;
    *) echo "FAIL usb-gecko: no banner from emulated gecko device"; echo "$OUT" ;;
esac

rm -rf "$USER_DIR"
exit $RC
