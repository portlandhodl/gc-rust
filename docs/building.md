# Building and running gc-rust

Exact commands for **Linux**, **macOS** and **Windows**. Every OS ends up with
the same thing: `dist/*.dol` files you can run in Dolphin or on a real GameCube.
CI builds the repo on all three
([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)).

What you need everywhere:

- **Rust** via rustup. The pinned nightly and `rust-src` are installed
  automatically from `rust-toolchain.toml` the first time you build.
- **GNU make** and a POSIX shell.
- **Your OS's normal host linker** (gcc/clang on Linux, the Xcode Command Line
  Tools on macOS, Visual Studio Build Tools on Windows). Only the small host
  tools (`gc-dol`, `gc-bnr`, `gc-iso`) need it. The GameCube code itself links
  with `rust-lld`, so no C compiler is involved.
- **Dolphin** (optional), to run the examples on your computer.

---

## 🐧 Linux

### 1. One-time setup

Debian / Ubuntu:

```bash
sudo apt update
sudo apt install -y build-essential make git curl
```

Fedora:

```bash
sudo dnf install -y gcc make git curl
```

Arch:

```bash
sudo pacman -S --needed base-devel git curl
```

Then install Rust (all distros):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

### 2. Build

```bash
git clone https://github.com/portlandhodl/gc-rust
cd gc-rust
make                        # every example -> dist/*.dol
make yarn-cat               # or just one example -> dist/yarn-cat.dol
```

### 3. Run in Dolphin

Install Dolphin from your distro (`sudo apt install dolphin-emu`,
`sudo dnf install dolphin-emu`, `sudo pacman -S dolphin-emu`) or from
[dolphin-emu.org](https://dolphin-emu.org/download/), then:

```bash
make run EXAMPLE=yarn-cat DOLPHIN_NOGUI=dolphin-emu
```

To open the file yourself instead, use `dolphin-emu -e dist/yarn-cat.dol` or
*File → Open* in Dolphin.

---

## 🍎 macOS (Intel and Apple silicon)

### 1. One-time setup

```bash
xcode-select --install      # make, git and the host linker
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

### 2. Build

```bash
git clone https://github.com/portlandhodl/gc-rust
cd gc-rust
make                        # every example -> dist/*.dol
make yarn-cat               # or just one example
```

The `make` that ships with macOS (GNU make 3.81) is fine.

### 3. Run in Dolphin

Download Dolphin from [dolphin-emu.org](https://dolphin-emu.org/download/)
and drag `Dolphin.app` into `/Applications`, then:

```bash
make run EXAMPLE=yarn-cat DOLPHIN_NOGUI=/Applications/Dolphin.app/Contents/MacOS/Dolphin
```

Or double-click your way there: *File → Open* → `dist/yarn-cat.dol`.

`make check` also runs headless Dolphin boot tests. They're skipped when
there's no Dolphin, and on macOS they need `timeout` from coreutils
(`brew install coreutils`).

---

## 🪟 Windows

There are two options. **WSL2** is the simplest: it *is* Linux. **MSYS2**
builds natively and is what CI uses.

### Option A: WSL2

In PowerShell (as Administrator), once:

```powershell
wsl --install -d Ubuntu
```

Reboot, open **Ubuntu** from the Start menu, and follow the
[Linux](#-linux) steps exactly. Your built files are in
`\\wsl$\Ubuntu\home\<you>\gc-rust\dist\` in Explorer. Open them in the Windows
version of Dolphin, or copy them to an SD card.

### Option B: native, with MSYS2

#### 1. One-time setup

1. Install Rust: download and run
   [rustup-init.exe](https://win.rustup.rs/x86_64). When it offers to install
   the **Visual Studio Build Tools**, accept (that's the host linker).
2. Install [MSYS2](https://www.msys2.org) (default location `C:\msys64`).
3. Open the **MSYS2 MSYS** shell from the Start menu and run:

```bash
pacman -S --needed make git
# make the Windows Rust install visible inside MSYS2 (add this line to
# ~/.bashrc so it sticks):
export PATH="$PATH:$(cygpath "$USERPROFILE")/.cargo/bin"
cargo --version              # should print a version
```

#### 2. Build (in the MSYS2 MSYS shell)

```bash
git clone https://github.com/portlandhodl/gc-rust
cd gc-rust
make                        # every example -> dist/*.dol
make yarn-cat               # or just one example
```

The repo's `.gitattributes` keeps LF line endings, so a Windows checkout
builds byte-for-byte the same way.

#### 3. Run in Dolphin

Download Dolphin from [dolphin-emu.org](https://dolphin-emu.org/download/)
and extract it, e.g. to `C:\Dolphin-x64`. Then, in the MSYS2 shell:

```bash
make run EXAMPLE=yarn-cat DOLPHIN_NOGUI=/c/Dolphin-x64/Dolphin.exe
```

Or open `dist\yarn-cat.dol` with *File → Open* in Dolphin.

---

## 🎮 Run on a real GameCube (any OS)

You need a way to boot homebrew (Swiss) and an SD adapter (SD2SP2, SD Gecko, …).

```bash
make sd EXAMPLE=yarn-cat
```

Copy the whole `dist/sd/yarn-cat/` folder to the SD card and boot Swiss. The
folder shows up as one entry with its banner. Any other `dist/<name>.dol` can
be copied to the card and loaded from Swiss directly.

Disc image instead (for ODEs or Dolphin):

```bash
make iso EXAMPLE=dvd-read   # -> dist/dvd-read.iso
```

---

## ✅ Check it worked

```bash
make list                   # all example names
make check                  # unit + host tests, DOL validation; Dolphin
                            # boot tests run only if Dolphin is found
ls dist/                    # your .dol files
```

`make check` looks for Dolphin at
`~/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui` (a nogui
source build; see the README's *Testing* section). Pass
`DOLPHIN_NOGUI=/path/to/dolphin-emu-nogui` to use another build.

## 🔧 Troubleshooting

| symptom | fix |
|---------|-----|
| `cargo: command not found` | Linux/macOS: `. "$HOME/.cargo/env"`. MSYS2: the `export PATH=...` line above. |
| `linker 'cc' not found` (Linux) | install `build-essential` / `gcc` |
| `xcrun: error: invalid active developer path` (macOS) | `xcode-select --install` |
| `link.exe not found` (Windows) | re-run rustup-init and install the VS Build Tools ("Desktop development with C++") |
| `make: command not found` (Windows) | you're in PowerShell/cmd. Use the **MSYS2 MSYS** shell, or WSL |
| first build is slow | normal: `core`/`alloc` are compiled from source once, and later builds are incremental |
