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
	sd-file

DOLS := $(addprefix dist/,$(addsuffix .dol,$(EXAMPLES)))

HOST_TARGET := $(shell rustc -vV | sed -n 's/host: //p')

.PHONY: all list clean run check test $(EXAMPLES)

all: tools/dol/gc-dol $(DOLS)
	@echo built: $(DOLS)

# Unit tests + DOL header validation + Dolphin smoke test.
check: all
	@cd tools/gc-dol && cargo test --release
	@cd tools/gc-host-tests && cargo test --release
	@sh tests/dolphin-smoke.sh
	@sh tests/dolphin-iso-smoke.sh

test: check

list:
	@echo $(EXAMPLES) | tr ' ' '\n'

$(EXAMPLES): %: dist/%.dol
	@echo built: $<

tools/dol/gc-dol: tools/gc-dol/src/main.rs tools/gc-dol/Cargo.toml
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
	dolphin-emu --batch --exec=$<


# ISO packaging path: builds the apploader payload + packs a bootable GCM.
APPLOADER_ELF := crates/apploader/target/powerpc-gekko-none-eabi/release/gc-apploader
GC_ISO := tools/gc-iso/target/x86_64-unknown-linux-gnu/release/gc-iso

.PHONY: iso run-iso

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
	dolphin-emu --batch --exec=dist/$(EXAMPLE).iso
