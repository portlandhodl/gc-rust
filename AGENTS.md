# gc-rust

Self-hosted Rust toolchain for Nintendo GameCube homebrew. **No devkitPro,
no libogc, no C compiler** — everything links through `rust-lld` and packs
via the in-tree `tools/gc-dol` host tool.

## Layout

- `powerpc-gekko-none-eabi.json` — custom rustc target spec
  (`powerpc-unknown-none` LLVM triple, `cpu = "750"`, `+fpu`,
  `panic-strategy = "abort"`, `linker = "rust-lld"`)
- `memory.x.ld` — linker script (MEM1 24 MiB); reserves TWO worst-case
  XFB slots at `__xfb_base` (0x81684000) for the VI flip chain; the main
  stack sits above them (~76 KiB) below `__stack` (0x817FEFF0).
- `crates/gc-std/` — the platform library:
  - `crt0.rs` — `_start` (asm bring-up, BSS zero, stack). doc: basis is
    libogc's PPCEarlyInit (zlib), ported 1:1.
  - `hw.rs` — MMIO helpers, write-gather pipe, cache ops (`dcbf`, etc.),
    timebase (`mftb`), YAGCD addresses.
  - `video.rs` — VI driver (libogc `video.c` port; NTSC/PAL IntDf + 480p)
    with a two-slot flip chain (`Video::flip`; `gx.end_frame` flips).
  - `console.rs`, `font.rs` — YUY2 text console + embedded 8x16 font.
  - `input.rs` — SI controller polling with origin calibration (cmd 0x41)
    and plug/unplug detect (port of libogc pad protocol).
  - `irq.rs` — PI interrupt controller + asm exception trampolines and a
    per-source dispatcher. VI retrace = one consumer; other sources welcome.
  - `lwp.rs` — preemptive single-core threads: DEC vector (0x0900) handler
    with full PPCState save/restore per TCB; round-robin every 4 ms;
    `spawn`/`sleep_ms`/`yield_now`/`join`/exit.
  - `lwp_sync.rs` — wait-queues, sleeping Mutex, bounded Channel.
  - `bba.rs` — BBA Ethernet MAC on EXI ch0/dev2 (libogc bba.c port;
    TX FIFO + RX page ring, 32 MHz frames).
  - `net.rs` — slimmest possible homebrew stack: ethernet frames, ARP
    reply/resolve, ICMP echo, UDP with checksums.
  - `gx.rs` — GX driver: pipe reg writers, immediate mode, TEV, dirty-state
    flush (port of libogc `gx.c`).
  - `gu.rs` — matrix math (pure Rust; Cephes-style sin/cos/sqrt inside).
  - `exi.rs` — EXI bus: lock/select/immediate/DMA (sync port of libogc
    exi.c), device-id read, insert probe. Used by card.rs/sram.rs/usbgecko.rs.
  - `sram.rs` — system SRAM (settings blob) read/write with checksums.
  - `card.rs` — memory-card driver (CARD_* port; card image workarea in
    MEM1, dual dir/FAT copies, next-fit allocation, erase-before-write).
    Wire layer is a `CardBus` trait; `gc-host-tests` emulates a card and
    runs the real state machine against it.
  - `usbgecko.rs` — USB Gecko debug channel (host-visible over TCP 55020
    under Dolphin's Gecko emulation).
  - `dvd.rs` — DI drive (read disk ID, inquiry, raw sector reads, stop
    motor; sync port of libogc `DVD_Low*` register dances).
  - `observe.rs` — observability mailbox at 0x80001C00 for host-side
    verification via Dolphin MemoryWatcher (compile-in tests) / debuggers.
- `crates/apploader/` — minimal Rust GC ISO apploader (three-callback
  contract; patched section table on pack).
- `tools/gc-iso/` — host tool producing bootable GCM/ISO images from a DOL
  (+ extra files at fixed LBAs).
  - `adpcm.rs` — Nintendo DSP-ADPCM → s16 mono decode (canonical math; 8
    predictor-pair coeff table from the asset header).
  - `sd.rs` — SD/SDHC block IO over SD Gecko (SPI mode, CMD0/8/16/17/24/41/
    55/58 per sdgecko_io.c; `SdSpi` trait; console path drives EXI).
  - `fat.rs` — read-only FAT16/FAT32 (MBR or superfloppy, 8.3 names,
    whole-file reads) over any 512-byte `BlockIo` backend.
  - `audio.rs` — simple AI DMA PCM streaming (48 kHz stereo 16-bit,
    int-refill).
  - `aesnd.rs` — polyphonic DSP-mixer audio (port of libaesnd's host
    protocol: voice parameter blocks, `0xface00xx` mailbox commands,
    ARAM staging per voice).
  - `aram.rs` — sync ARAM DMA + bump allocator (voice-bucket shaped).
  - `dsp.rs` — DSP mailbox/task loader + AESND task interrupt plumbing.
  - `dspcode.rs` — byte paraphrase of libaesnd's DSP mixer microcode.
  - `heap.rs` — best-fit free-list allocator over MEM1 for `alloc`
    (byte-exact splits; host-tested in tools/gc-host-tests).
  - `runtime.rs` — `#[panic_handler]` + `#[alloc_error_handler]`.
  - `system.rs` — `exit()` → reload stub at 0x80001800.
  - `timebase.rs` — `mftb` / microsecond sleeps.
- `examples/NN-name/` — a bin per example; package name = dir basename without `NN-`.
  Workspace globs them in automatically (`members = ["crates/gc-std", "examples/*"]`).
- `tools/gc-dol/` — host tool converting ELF32BE → GameCube DOL.
- `tools/gc-bnr/` — host tool building `opening.bnr` (BNR1: 96x32 RGB5A3
  banner + title/description) from a PPM + 5-line text file.

## Build

- `make` — builds tool + all `dist/*.dol`.
- `make <pkg>` — one example (names: hello-console, pad-input, heap-strings,
  video-info, pixel-plasma, gx-clear, gx-triangle, gx-cube, gx-textured-cube,
  gx-lit-cube, irq-timer, pad-calibrated, audio-beep, dsp-mixer, exi-sram,
  memcard, usb-gecko, sd-file, dvd-read, threads, thread-sync, net-echo,
  yarn-cat).
- `make iso EXAMPLE=dvd-read` — build a bootable GCM (gc-iso + apploader).
- `make sd EXAMPLE=yarn-cat` — Swiss SD folder `dist/sd/<ex>/` with
  `default.dol` + `opening.bnr` (from `examples/NN-<ex>/banner.ppm` +
  `banner.txt`); Swiss shows such a folder as one entry with the banner.
- `tests/dvd-iso-e2e.sh` — boots the ISO in Dolphin + watches the
  observation mailbox via MemoryWatcher (needs a Dolphin build with
  USE_MEMORYWATCHER compiled in).
- `tests/memcard-persist.sh` / `tests/usbgecko-e2e.sh` — manual E2E tests
  (need desktop Dolphin; not wired into `make check`).
- `tests/dolphin-iso-smoke.sh` — ISO boot smoke (nogui source build).
- A working local source build of Dolphin (nogui) lives at
  `~/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui` — build
  it with:
  `cmake -G Ninja -DENABLE_QT=OFF -DENABLE_NOGUI=ON -DENABLE_TESTS=OFF -DENABLE_VULKAN=OFF -DENABLE_LLVM=OFF`.
  Its `Binaries/Sys` must exist (symlink to `../../Data/Sys`) — the build
  is not installed, and without Sys Dolphin can't find its data.
  The flatpak Dolphin is no longer used.
- `make run EXAMPLE=<pkg>` — opens the DOL in that Dolphin (X11 window).
- Seeing frames without a window: `-p headless -C Dolphin.Movie.DumpFrames=True
  -C Dolphin.Movie.DumpFramesSilent=True` writes `<user>/Dump/Frames/*.avi`
  (this build has FFmpeg, so it's an AVI, not PNGs); pull stills with ffmpeg.

## Conventions

- Rust code under `crates/gc-std` and `examples` is `#![no_std]`; examples are
  also `#![no_main]` and enter via `#[no_mangle] extern "C" fn main() -> i32`.
- MMIO/U code lives behind safe wrappers; `hw.rs` is the lowest layer.
- Everything built for the target must run with `panic = abort`. Atomics:
  only 32-bit align load/store (`max-atomic-width = 32`, Gekko lacks
  byte/halfword atomics).
- Register writes go through `hw::vi_write/cp_write/pe_write/si_write` do
  not open-code `0xCC00xxxx` pokes elsewhere.

## Testing

- `make check` — full suite:
  1. `tools/gc-dol`: packer unit tests + validation of every `dist/*.dol`
     header (all addresses inside MEM1, entry = 0x80003100).
  2. `tools/gc-host-tests`: pure-Rust math (perspective/look-at/concat/
     rotations) vs reference constants, heap allocator
     correctness/fragmentation suites, a memory-card emulation suite
     driving the real card.rs state machine, and an SPI-level emulated SD
     card with a FAT32 image driving sd.rs + fat.rs, tested on host.
  3. `tests/dolphin-smoke.sh`: boots each `dist/*.dol` in headless Dolphin
     (nogui source build) for 6s; boot failure, panic, DSI/ISI, illegal
     instruction or invalid memory access = fail.
- Run manually: `sh tests/dolphin-smoke.sh [names...]`,
  `RUN_SECS=<n>` overrides the 6s default.
