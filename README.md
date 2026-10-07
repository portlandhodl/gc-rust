# gc-rust

**Write Nintendo GameCube homebrew in Rust — with `alloc`, `String` and
GX 3D graphics.**

gc-rust is a custom Rust target (`powerpc-gekko-none-eabi`) plus a small
platform library (`gc-std`) that lets `rustc` compile straight to bootable
GameCube `.dol` binaries. Under the hood it stands on the shoulders of
[devkitPro](https://devkitpro.org/): devkitPPC's GCC drives the final link
(`-mogc`), and the hardware access goes through battle-tested
[libogc](https://github.com/devkitPro/libogc) — so startup code, video
modes and politics with the GPU all behave exactly like regular C
homebrew.

```
┌──────────────┐   ┌─────────────────┐   ┌──────────────────┐   ┌─────────┐
│ your Rust app │─> │ gc-std (this    │─> │ libogc / newlib  │─> │ .dol    │
│  (#![no_std]) │   │  repo; core +   │   │ (devkitPPC)      │   │         │
│               │   │  alloc via      │   │                  │   │         │
│               │   │  -Zbuild-std)   │   │                  │   │         │
└──────────────┘   └─────────────────┘   └──────────────────┘   └─────────┘
```

## What you get

* `core` + `alloc` on the console: `Vec`, `String`, `BTreeMap`, `Box`,
  `format!` — backed by the real GameCube system heap.
* `println!` rendered to the framebuffer text console.
* Controller input: buttons, held-state, analog sticks and analog
  triggers.
* Full GX pipeline: perspective projections (libogc's `gu` math),
  immediate-mode geometry, vertex colors, depth buffering, RGB565
  textures (automatic 4x4 tile swizzle + cache flush).
* Software rendering straight into the YUY2 framebuffer, if you want to
  go completely feral.
* Panic messages and allocation-failure reports printed to the console.

## Requirements

* [devkitPro](https://devkitpro.org/wiki/Getting_Started) with the
  `gamecube-dev` package group (`sudo dkp-pacman -S gamecube-dev`)
* Rust nightly (`rustup toolchain install nightly --component rust-src`)

## Build

```bash
export DEVKITPRO=/opt/devkitpro
export DEVKITPPC=/opt/devkitpro/devkitPPC

make        # builds every example into dist/*.dol
make list   # show example names
make gx-cube
```

## Run

In [Dolphin](https://dolphin-emu.org/):

```bash
make run EXAMPLE=gx-cube            # or:
dolphin-emu --batch --exec=dist/gx-cube.dol
```

On real hardware, drop the `.dol` on an SD card and load it with
[Swiss](https://github.com/emukidid/swiss-gc) (e.g. via SD2SP2, BBA, or a
memory-card exploit boot disc — follow Swiss' docs).

## The examples

| # | Name | Shows |
|---|------------------|--------------------------------------------------------|
| 01 | `hello-console`    | Console text, `println!`, basic pad polling            |
| 02 | `pad-input`        | Buttons down/held, analog sticks, analog triggers      |
| 03 | `heap-strings`     | `Vec`, `String`, `BTreeMap`, `Box`, `format!` on GC    |
| 04 | `video-info`       | Querying the detected video mode                       |
| 05 | `pixel-plasma`     | Software rendering into the YUV (YUY2) framebuffer     |
| 06 | `gx-clear`         | GX pipeline bring-up, clear-color animation            |
| 07 | `gx-triangle`      | Immediate-mode vertices, GX matrices, the rainbow tri  |
| 08 | `gx-cube`          | Depth-tested 3D cube with per-face colors              |
| 09 | `gx-textured-cube` | Procedural RGB565 texture, swizzle, UVs                |
| 10 | `gx-lit-cube`      | Per-vertex lighting (normals × light dir) on the CPU   |

Every example is compiled 100% from Rust; the only C in the final binary
is libogc/newlib itself.

## Writing your own program

Create a new crate in `examples/` (glob member of the workspace), add
`gc-std = { path = "../../crates/gc-std" }`, and start from an example's
`main.rs`. `make <your-crate-name>` will pick it up automatically.

```rust
#![no_std]
#![no_main]

use gc_std::println;

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();          // or: let gx = gc.into_gx();
    println!("Hello, GameCube!");
    loop {}
}
```

Note: `extern "C" fn main` is called by libogc's crt0 after hardware
bring-up; don't use a normal Rust `fn main`.

## How the target works

* `powerpc-gekko-none-eabi.json` describes the Gekko CPU: big-endian
  32-bit PowerPC 750CXe with FPU, `static` relocs, `panic = abort`,
  devkitPPC's gcc as linker driver with `-mogc -mcpu=750 -meabi
  -mhard-float` (which selects the libogc linker script + startfiles).
* `core`/`alloc` are built from source (`build-std`) because no shipped
  std supports this target. `compiler-builtins` provides compiler
  intrinsics.
* The link line mirrors what `powerpc-eabi-gcc -mogc` produces:
  `--start-group -logc -lsysbase -lc -lm -lgcc --end-group`.
* `gc-std` provides the three things `core` can't: a `#[panic_handler]`,
  an `#[alloc_error_handler]`, and a `#[global_allocator]` on top of
  newlib's `malloc`/`free`.

## Limitations / not-yet-done

* No sound (`asnd`/`aesnd`), no storage (`sd`/`fat`/ARAM), no network
  (`bba`/`network.h`) yet — the FFI pattern in `gc-std/src/ffi.rs` shows
  how to add bindings.
* No multithreading (GameCube is single-core; libogc's LWP would map fine
  but isn't bound yet).
* `f32::sqrt` etc. aren't in `core` without std; call libm through FFI or
  avoid them.
* Untested-on-every-TV: PAL/NTSC progressive quirks are inherited
  entirely from libogc's `VIDEO_GetPreferredMode`.

## License

Your code: do what you want. The crates in this repository:

* [MIT](LICENSE-MIT) **or** [Apache-2.0](LICENSE-APACHE), at your option.

Built against [libogc](https://github.com/devkitPro/libogc) and the
devkitPro toolchain (not distributed here).
