# gc-rust

Self-hosted Rust toolchain for Nintendo GameCube homebrew. **No devkitPro,
no libogc, no C compiler** — everything links through `rust-lld` and packs
via the in-tree `tools/gc-dol` host tool.

## Layout

- `powerpc-gekko-none-eabi.json` — custom rustc target spec
  (`powerpc-unknown-none` LLVM triple, `cpu = "750"`, `+fpu`,
  `panic-strategy = "abort"`, `linker = "rust-lld"`)
- `memory.x.ld` — linker script (MEM1 0 x80003100, 24 MiB)
- `crates/gc-std/` — the platform library:
  - `crt0.rs` — `_start` (asm bring-up, BSS zero, stack). doc: basis is
    libogc's PPCEarlyInit (zlib), ported 1:1.
  - `hw.rs` — MMIO helpers, write-gather pipe, cache ops (`dcbf`, etc.),
    timebase (`mftb`), YAGCD addresses.
  - `video.rs` — VI driver (libogc `video.c` port; NTSC/PAL IntDf + 480p).
  - `console.rs`, `font.rs` — YUY2 text console + embedded 8x16 font.
  - `input.rs` — SI controller polling with origin calibration (cmd 0x41)
    and plug/unplug detect (port of libogc pad protocol).
  - `irq.rs` — PI interrupt controller + asm exception trampolines and a
    per-source dispatcher. VI retrace = one consumer; other sources welcome.
  - `gx.rs` — GX driver: pipe reg writers, immediate mode, TEV, dirty-state
    flush (port of libogc `gx.c`).
  - `gu.rs` — matrix math (pure Rust; Cephes-style sin/cos/sqrt inside).
  - `audio.rs` — AI DMA PCM streaming (48 kHz stereo 16-bit, int-refill).
  - `aram.rs` — sync ARAM DMA + crude block allocator (voice-bucket shaped).
  - `dsp.rs` — DSP mailbox/task loader (and microcode upload for AESND-path).
  - `dspcode.rs` — byte paraphrase of libaesnd's DSP mixer microcode.
  - `heap.rs` — free-list allocator over MEM1 for `alloc`.
  - `runtime.rs` — `#[panic_handler]` + `#[alloc_error_handler]`.
  - `system.rs` — `exit()` → reload stub at 0x80001800.
  - `timebase.rs` — `mftb` / microsecond sleeps.
- `examples/NN-name/` — a bin per example; package name = dir basename without `NN-`.
  Workspace globs them in automatically (`members = ["crates/gc-std", "examples/*"]`).
- `tools/gc-dol/` — host tool converting ELF32BE → GameCube DOL.

## Build

- `make` — builds tool + all `dist/*.dol`.
- `make <pkg>` — one example (names: hello-console, pad-input, heap-strings,
  video-info, pixel-plasma, gx-clear, gx-triangle, gx-cube, gx-textured-cube,
  gx-lit-cube, irq-timer, pad-calibrated, audio-beep).
- `make run EXAMPLE=<pkg>` — Dolphin.

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
     rotations) vs reference constants, tested on host.
  3. `tests/dolphin-smoke.sh`: boots each `dist/*.dol` in headless Dolphin
     (flatpak) for 6s; panic/DSI/ISI/illegal-instruction = fail.
- Run manually: `sh tests/dolphin-smoke.sh [names...]`,
  `RUN_SECS=<n>` overrides the 6s default.
