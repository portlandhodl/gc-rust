# gc-rust

**Write Nintendo GameCube homebrew in 100% pure Rust — no devkitPro, no libogc, no C toolchain needed.**

![yarn-cat: a low-poly orange kitten batting a ball of yarn inside a retro TV](docs/yarn-cat.png)

*`yarn-cat` (example 23): a low-poly kitten that lives inside your TV and
plays with a ball of yarn — GX-rendered, flat-shaded, running on a real
GameCube. `make sd EXAMPLE=yarn-cat` builds a Swiss-ready SD folder with a
banner.*

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
* **Preemptive threads** (`lwp`): background agents with `sleep_ms` / `yield_now` / `join` — one Gekko core, sliced by the decrementer.
* **Memory cards** (`card`): the `CARD_*` save-game filesystem (mount/verify/create/read/write/delete/dir-walk), ported 1:1 from libogc, exercised host-side against a card-image emulator.
* **EXI bus** (`exi`) with libogc-shaped sync API, plus USB Gecko debug output and system SRAM settings access (`sram`).
* **SD Gecko** (`sd`): SD/SDHC block reads & writes over SPI; **`fat`** gives a read-only FAT16/FAT32 layer (list dir, read files) for media-grade storage.
* **ADPCM decode** (`adpcm`): GC DSP-ADPCM → PCM s16 for stock audio assets, usable with `aesnd` voices.
* **BBA networking** (`bba` + `net`): the Ethernet MAC driver plus a purpose-built mini IP stack (ARP, ICMP ping, UDP) for background "agent" threads.
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

The Dolphin smoke tests use a source build of Dolphin (nogui) at
`~/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui` (override
with `DOLPHIN_NOGUI=...`); they are skipped with code 77 when it's absent:

```bash
cmake -G Ninja -B build-x86_64-release \
  -DENABLE_QT=OFF -DENABLE_NOGUI=ON -DENABLE_TESTS=OFF \
  -DENABLE_VULKAN=OFF -DENABLE_LLVM=OFF
ninja -C build-x86_64-release dolphin-emu-nogui
ln -s ../../Data/Sys build-x86_64-release/Binaries/Sys   # uninstalled build needs its data
```

Then, in a window:

```bash
make run EXAMPLE=yarn-cat
```

Dolphin is forgiving where real hardware isn't (cache coherency, GX
fixed-point rasterizer range, the zcomploc copy-clear quirk, alignment
exceptions…): verify on a console before trusting a GX change.

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
| 15 | `exi-sram`           | EXI bus driver + system SRAM/settings readout          |
| 16 | `memcard`            | Save files on a real GC memory card (CARD driver, host-tested) |
| 17 | `usb-gecko`          | USB Gecko debug-channel output (host over TCP under Dolphin) |
| 18 | `sd-file`            | SD Gecko: FAT32 mount, dir listing, file read                |
| 19 | `dvd-read`           | DI drive reads; boots as a full bootable ISO (`make iso`)    |
| 20 | `threads`            | Preemptive LWP: background agent with sleeps + foreground loop |
| 21 | `thread-sync`        | LWP Channel/Mutex/WaitQueue: game pushes jobs onto a parked agent |
| 22 | `net-echo`           | BBA ethernet: probe, bring-up, ARP gateway, ICMP ping          |
| 23 | `yarn-cat`           | Low-poly kitten chasing yarn in a TV room: flat-shaded GX scene, springy animation (A tosses, stick nudges) |
| 24 | `gx-diag`            | GX test card: 2D, depth test, culling per quadrant — photograph it on hardware |
| 25 | `gx-selftest`        | GX self-test: draws, reads the EFB back, prints PASS/FAIL on the console |

### Running on a GameCube with Swiss

`make sd EXAMPLE=yarn-cat` writes `dist/sd/yarn-cat/` containing
`default.dol` + `opening.bnr`. Copy that folder to the SD card; Swiss
lists it as one entry with the banner image, title and description
(`tools/gc-bnr` builds the BNR1 banner from `examples/NN-<name>/banner.ppm`
+ `banner.txt`).

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
| `crates/gc-std/src/dvd.rs` | DI drive: disc ID, inquiry, raw reads (libogc `DVD_Low*` port) |
| `crates/gc-std/src/lwp.rs` | Threading: DEC-vector preemption, full PPCContext per TCB (spawn/sleep/yield/join) |
| `tools/gc-iso` | Bootable GCM packer (+ a tiny Rust apploader payload in `crates/apploader`) |

## Bootable ISOs

`make iso EXAMPLE=dvd-read` produces `dist/dvd-read.iso`: a valid GCM
image with boot.bin + bi2 + FST + our own Rust apploader. It boots in
Dolphin and on real hardware (Swiss / datel loaders). Any extra files
packed (see `Makefile` `--file` entries) sit at fixed LBAs; `dvd-read`
shows reading them back with the pure-Rust DI (`dv`), then proves the
roundtrip to testers via the observation mailbox (`observe.rs` +
MemoryWatcher).
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
