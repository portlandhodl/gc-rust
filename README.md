# gc-rust

**Write Nintendo GameCube homebrew in 100% pure Rust — no devkitPro, no libogc, no C toolchain needed.**

`gc-rust` is a self-hosted Rust target (`powerpc-gekko-none-eabi`) for the GameCube's Gekko CPU (big-endian PowerPC 750CXe). rustc compiles your code, `rust-lld` links it against a memory map we ship (`memory.x.ld`), and a tiny pure-Rust `gc-dol` tool packs the ELF into a bootable `.dol`.

The platform library, `gc-std`, is written in Rust (with one startup assembly block and MMIO register constants ported from the public-domain-ish libogc register documentation/Dolphin emulator):

```
┌──────────────┐    ┌──────────────────┐    ┌──────────┐    ┌────────┐
│ your Rust app │──▶ │ gc-std (pure Rust)│──▶ │ rust-lld │──▶ │ gc-dol │──▶ .dol
│  (#![no_std]) │    │  crt0 + drivers  │    └──────────┘    └────────┘
└──────────────┘    └──────────────────┘
```

Only requirement: **a nightly Rust toolchain** (`rustup toolchain install nightly --component rust-src`).

Nothing else. No devkitPro, no gcc, no libogc, no elf2dol.

## What's in the box

* `core` + `alloc` via `-Zbuild-std` — `Vec`, `String`, `Box`, `format!`, `BTreeMap`… all work on the console.
* HV bring-up in Rust (`crt0.rs`): BATs, caches, FPU + paired-singles, stack, bss zeroing.
* **Interrupts & exceptions**: PI interrupt controller, exception vector trampolines, per-source handlers, `irq::IrqLock` critical sections.
* Video (VI) driver: NTSC/PAL/MPAL/EURGB60, 480i IntDf and 480p progressive, YUY2 4:2:2 XFB, retrace polling.
* **GX** driver: viewport/scissor, immediate-mode vertex streams through the write-gather pipe, projection/model matrix loads, TEV setup, depth buffer, EFB→XFB copy.
* **Polyphonic audio** (`aesnd`): up to 32 voices mixed on the console's DSP by the libaesnd-compatible mixer microcode (`dspcode.rs`), staged through ARAM, with per-voice volume/pitch/loop and stream-refill callbacks.
* **Audio** (simple path): stereo 16-bit PCM @ 48 kHz straight through the AI DMA engine (interrupt-driven buffer refill, `audio::on_refill` callback) — for when you just need a beep.
* Controller (SI) driver: buttons, held state, analog sticks, analog triggers, origin calibration (cmd `0x41`), hot-plug detect.
* `gu` matrix math (perspective, look-at, concat, rotation…) in pure Rust.
* Font-based text console on the framebuffer (`print!`/`println!`).
* **Framebuffer double-buffering** — a two-slot VI flip chain (`video.flip()`; the GX `end_frame()` flips automatically).
* A best-fit, byte-exact free-list heap allocator on MEM1.
* ARAM streaming-block driver (sync DMA) if you want to study DMA into the DSP.

## Build

```bash
rustup toolchain install nightly --component rust-src
make                       # all examples -> dist/*.dol (header-validated)
make list                  # print example names
make gx-cube               # just one
```

## Test

```bash
make check    # packer unit tests (incl. synthetic ELF + every dist/*.dol),
              # host-side math tests (mtx/perspective/rotations), and a
              # headless Dolphin smoke boot of every dist/*.dol.
```

The Dolphin smoke test needs `flatpak install flathub org.DolphinEmu.dolphin-emu`. It is skipped with code 77 when absent.

Then in Dolphin (GUI for actual pixels):

```bash
make run EXAMPLE=gx-cube   # or: dolphin-emu --batch --exec=dist/gx-cube.dol
```

On hardware: copy the `.dol` onto an SD card and load it with Swiss (or any other GC homebrew loader), e.g. via SD2SP2, BBA, or a memory-card exploit.

## Examples

| # | name | shows |
|---|------------------|----------------------------------------------------------|
| 01 | `hello-console`     | Text console, `println!`, button polling                 |
| 02 | `pad-input`         | Buttons, held state, analog sticks, analog triggers      |
| 03 | `heap-strings`      | `Vec`, `String`, `BTreeMap`, `Box`, `format!`, iterative fib |
| 04 | `video-info`        | Querying the detected video mode                         |
| 05 | `pixel-plasma`      | Software-rendered plasma straight into the YUY2 XFB     |
| 06 | `gx-clear`          | GX bring-up, clear-color animation                       |
| 07 | `gx-triangle`       | Immediate-mode colored triangle                          |
| 08 | `gx-cube`           | Depth-tested colored cube                                |
| 09 | `gx-textured-cube`  | Procedural RGB565 texture + 4×4 swizzle + clamping       |
| 10 | `gx-lit-cube`       | Per-vertex lit cube (normals × light dir)                |
| 11 | `irq-timer`         | VI retrace via PI interrupt handler (no polling)        |
| 12 | `pad-calibrated`    | Pad origin calibration + hot-plug detect                |
| 13 | `audio-beep`        | Stereo PCM out via AI DMA at 48 kHz, interrupt-refilled |
| 14 | `dsp-mixer`         | AESND polyphony: chord loop + accents mixed on the DSP |

Every resulting `.dol` contains Rust + hardware glue only. No C, no assembly libraries, zero non-Rust code.

## Writing your own program

Make a new workspace member under `examples/`, copy the minimal skeleton from `01-hello-console`:

```rust
#![no_std]
#![no_main]
use gc_std::{println, input::button};

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();
    println!("Hello GameCube");
    loop {}
}
```

Your crate's name must match its directory name base (`examples/NN-mycoolgame` → package `mycoolgame`), then `make mycoolgame` works.

## How it works

| piece | notes |
|-------|-------|
| `powerpc-gekko-none-eabi.json` | custom rustc target: big-endian, +FPU (`750`), static reloc model, panic=abort, `rust-lld` linker driver |
| `crates/gc-std/src/crt0.rs` | `_start`: real-mode BAT/HID0/L2/FPSCR setup then MMU back on; clear `.bss`; call `main()` |
| `crates/gc-std/src/hw.rs` | MMIO read/write + write-gather pipe helpers, YAGCD register addresses |
| `crates/gc-std/src/video.rs` | VI driver: timing tables (NTSC/PAL/MPAL/EURGB60 + 480p), framebuffer setup, vsync, flip chain |
| `crates/gc-std/src/aesnd.rs` | DSP-mixer voices: PB structs, `0xface*` mail protocol, AI DMA pacing (libaesnd port) |
| `crates/gc-std/src/irq.rs` | PI interrupt controller, exception trampolines, per-source handlers |
| `crates/gc-std/src/input.rs` | SI `0x4003` polling with origin calibration + hot-plug detect |
| `crates/gc-std/src/gx.rs` | GX register shadowing + BP/CP/XF command writers + pipeline helpers |
| `crates/gc-std/src/gu.rs` | All matrix math in pure Rust (Cephes-style scalar libm included) |
| `tools/gc-dol` | host-side ELF→DOL packer in pure Rust |
| `memory.x.ld` | minimal MEM1 layout (0x80003100 entry, 24 MiB) |

## License

MIT OR Apache-2.0 (see `LICENSE-MIT` / `LICENSE-APACHE`).

Hardware register semantics and bit encodings follow libogc (zlib-style
public-domain documentation of the GameCube) and Dolphin's source. No libogc
code or binaries are linked.
