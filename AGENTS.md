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
  - `video.rs` — VI driver (libogc `video.c` port; NTSC/PAL IntDf).
  - `console.rs`, `font.rs` — YUY2 text console + embedded 8x16 font.
  - `input.rs` — SI controller polling (port of libogc pad protocol).
  - `gx.rs` — GX driver: pipe reg writers, immediate mode, TEV, dirty-state
    flush (port of libogc `gx.c`).
  - `gu.rs` — matrix math (pure Rust; Cephes-style sin/cos/sqrt inside).
  - `heap.rs` — free-list allocator over MEM1 for `alloc`.
  - `runtime.rs` — `#[panic_handler]` + `#[alloc_error_handler]`.
  - `system.rs` — `exit()` → reload stub at 0x80001800.
- `examples/NN-name/` — a bin per example; package name = dir basename without `NN-`.
  Workspace globs them in automatically (`members = ["crates/gc-std", "examples/*"]`).
- `tools/gc-dol/` — host tool converting ELF32BE → GameCube DOL.

## Build

- `make` — builds tool + all `dist/*.dol`.
- `make <pkg>` — one example (names: hello-console, pad-input, heap-strings,
  video-info, pixel-plasma, gx-clear, gx-triangle, gx-cube, gx-textured-cube,
  gx-lit-cube).
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

- `make all` — compile everything; then mutually-validate the DOL header
  fields (all addresses inside MEM1).
- No automated tests run on-target (no headless emulator harness yet);
  check output in Dolphin when changing drivers.
