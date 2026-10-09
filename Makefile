# gc-rust — pure-Rust GameCube toolchain. No devkitPro, no libogc, no C.
#
#   make            build every example into dist/*.dol
#   make list       list the example names
#   make <name>     build one example (e.g. make gx-cube)
#   make run EXAMPLE=hello-console
#                   build + run in Dolphin
#   make clean

TARGET_SPEC  := powerpc-gekko-none-eabi.json
TARGET_TRIPLE := powerpc-gekko-none-eabi
PROFILE      := release

EXAMPLES := \
	dvd-read \
	threads \
	thread-sync \
	net-echo \
	hello-console \
	pad-input \
	heap-strings \
	video-info \
	pixel-plasma \
	gx-clear \
	gx-triangle \
	gx-cube \
	gx-textured-cube \
	gx-lit-cube \
	irq-timer \
	pad-calibrated \
	audio-beep \
	dsp-mixer \
	exi-sram \
	memcard \
	usb-gecko \
	sd-file \
	yarn-cat \
	gx-diag \
	gx-selftest

DOLS := $(addprefix dist/,$(addsuffix .dol,$(EXAMPLES)))

HOST_TARGET := $(shell rustc -vV | sed -n 's/host: //p')

# Source-built Dolphin (nogui); `make run` opens it in an X11 window.
DOLPHIN_NOGUI ?= $(HOME)/git/dolphin/build-x86_64-release/Binaries/dolphin-emu-nogui

.PHONY: all list clean run check test $(EXAMPLES)

all: tools/dol/gc-dol $(DOLS)
	@echo built: $(DOLS)

# Unit tests + DOL header validation + Dolphin smoke test.
check: all
	@cd tools/gc-dol && cargo test --release
	@cd tools/gc-bnr && cargo test --release
	@cd tools/gc-host-tests && cargo test --release
	@sh tests/dolphin-smoke.sh
	@sh tests/dolphin-iso-smoke.sh

test: check

list:
	@echo $(EXAMPLES) | tr ' ' '\n'

$(EXAMPLES): %: dist/%.dol
	@echo built: $<

tools/dol/gc-dol: tools/gc-dol/src/main.rs tools/gc-dol/src/lib.rs tools/gc-dol/Cargo.toml
	cd tools/gc-dol && cargo build --release --target x86_64-unknown-linux-gnu
	mkdir -p tools/dol
	cp -f tools/gc-dol/target-host/x86_64-unknown-linux-gnu/release/gc-dol tools/dol/gc-dol
	touch $@

# Link with our memory script; build core/alloc from source (the GC target
# ships no prebuilt std).
RUSTFLAGS := -C link-arg=-T -C link-arg=$(abspath memory.x.ld)

CARGO_BUILD := RUSTFLAGS='$(RUSTFLAGS)' cargo build --release --target $(TARGET_SPEC) \
	-Z build-std=core,alloc -Zbuild-std-features=compiler-builtins-mem

dist/%.elf: memory.x.ld $(TARGET_SPEC) $(shell find crates/gc-std examples -name '*.rs' -o -name Cargo.toml)
	@mkdir -p dist
	$(CARGO_BUILD) -p $*
	cp -f target/$(TARGET_TRIPLE)/$(PROFILE)/$* $@

dist/%.dol: dist/%.elf tools/dol/gc-dol
	tools/dol/gc-dol $< $@
	tools/dol/gc-dol --validate $@ >/dev/null

clean:
	cargo clean
	rm -rf dist tools/dol/gc-dol

run: dist/$(EXAMPLE).dol
	$(DOLPHIN_NOGUI) -p x11 -e $(abspath $<)


# ISO packaging path: builds the apploader payload + packs a bootable GCM.
APPLOADER_ELF := crates/apploader/target/powerpc-gekko-none-eabi/release/gc-apploader
GC_ISO := tools/gc-iso/target/x86_64-unknown-linux-gnu/release/gc-iso

.PHONY: iso run-iso sd

# `make sd EXAMPLE=yarn-cat` — Swiss-ready SD card folder:
#   dist/sd/<example>/default.dol + opening.bnr (banner image + title text)
# Copy the <example> folder to the SD card; Swiss lists it as one entry
# with the banner. Needs examples/NN-<example>/banner.ppm + banner.txt.
GC_BNR := tools/gc-bnr/target-host/x86_64-unknown-linux-gnu/release/gc-bnr
EXAMPLE_DIR = $(firstword $(wildcard examples/*-$(EXAMPLE)))

sd: dist/$(EXAMPLE).dol $(GC_BNR)
	@test -f $(EXAMPLE_DIR)/banner.ppm || { echo "no banner.ppm in $(EXAMPLE_DIR)"; exit 1; }
	mkdir -p dist/sd/$(EXAMPLE)
	cp -f dist/$(EXAMPLE).dol dist/sd/$(EXAMPLE)/default.dol
	$(GC_BNR) --image $(EXAMPLE_DIR)/banner.ppm --text $(EXAMPLE_DIR)/banner.txt \
	    -o dist/sd/$(EXAMPLE)/opening.bnr
	@echo "SD folder ready: dist/sd/$(EXAMPLE)/ (copy the whole folder to the card)"

$(GC_BNR): tools/gc-bnr/src/main.rs tools/gc-bnr/src/lib.rs tools/gc-bnr/Cargo.toml
	cd tools/gc-bnr && cargo build --release

# `make iso EXAMPLE=dvd-read` — emit dist/<example>.iso (bootable GCM)
DVD_README := examples/19-dvd-read/readme.txt
iso: $(APPLOADER_ELF) $(GC_ISO) dist/$(EXAMPLE).dol
	$(GC_ISO) \
	    --apploader $(APPLOADER_ELF) \
	    --dol dist/$(EXAMPLE).dol \
	    --file 0x100000:$(DVD_README) \
	    --name "$(EXAMPLE) (gc-rust ISO demo)" \
	    -o dist/$(EXAMPLE).iso

$(APPLOADER_ELF): crates/apploader/src/main.rs
	cd crates/apploader && RUSTFLAGS='-C link-arg=-T -C link-arg=$(abspath crates/apploader/link-apploader.ld)' cargo build --release --target ../../powerpc-gekko-none-eabi.json -Z build-std=core

$(GC_ISO): tools/gc-iso/src/main.rs
	cd tools/gc-iso && cargo build --release

run-iso: dist/$(EXAMPLE).iso
	$(DOLPHIN_NOGUI) -p x11 -e $(abspath dist/$(EXAMPLE).iso)
