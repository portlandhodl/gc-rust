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
	hello-console \
	pad-input \
	heap-strings \
	video-info \
	pixel-plasma \
	gx-clear \
	gx-triangle \
	gx-cube \
	gx-textured-cube \
	gx-lit-cube

DOLS := $(addprefix dist/,$(addsuffix .dol,$(EXAMPLES)))

HOST_TARGET := $(shell rustc -vV | sed -n 's/host: //p')

.PHONY: all list clean run $(EXAMPLES)

all: tools/dol/gc-dol $(DOLS)
	@echo built: $(DOLS)

list:
	@echo $(EXAMPLES) | tr ' ' '\n'

$(EXAMPLES): %: dist/%.dol
	@echo built: $<

tools/dol/gc-dol: tools/gc-dol/main.rs tools/gc-dol/Cargo.toml
	cd tools/gc-dol && cargo build --release --target x86_64-unknown-linux-gnu
	mkdir -p tools/dol
	cp -f tools/gc-dol/target-host/x86_64-unknown-linux-gnu/release/gc-dol tools/dol/gc-dol
	touch $@

# Link with our memory script. RUSTFLAGS applies to all crates in this
# workspace (gc-only target).
RUSTFLAGS := -C link-arg=-T -C link-arg=$(abspath memory.x.ld)

CARGO_BUILD := RUSTFLAGS='$(RUSTFLAGS)' cargo build --release --target $(TARGET_SPEC)

dist/%.elf: memory.x.ld $(TARGET_SPEC) $(shell find crates/gc-std examples -name '*.rs' -o -name Cargo.toml)
	@mkdir -p dist
	$(CARGO_BUILD) -p $*
	cp -f target/$(TARGET_TRIPLE)/$(PROFILE)/$* $@

dist/%.dol: dist/%.elf tools/dol/gc-dol
	tools/dol/gc-dol $< $@

clean:
	cargo clean
	rm -rf dist tools/dol/gc-dol

run: dist/$(EXAMPLE).dol
	dolphin-emu --batch --exec=$<
