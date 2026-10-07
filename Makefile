# gc-rust — build system for the GameCube Rust examples.
#
# Requires devkitPro (devkitPPC + libogc + tools) with the usual
# environment variables:
#   export DEVKITPRO=/opt/devkitpro
#   export DEVKITPPC=/opt/devkitpro/devkitPPC
#
#   make                  build every example into dist/*.dol
#   make hello-console    build one example (any name from `make list`)
#   make list             list example names
#   make run EXAMPLE=...
#                         build + run one example in Dolphin (if installed)
#   make clean

ifeq ($(strip $(DEVKITPRO)),)
$(error "Please set DEVKITPRO in your environment, e.g. export DEVKITPRO=/opt/devkitpro")
endif
ifeq ($(strip $(DEVKITPPC)),)
$(error "Please set DEVKITPPC in your environment, e.g. export DEVKITPPC=$(DEVKITPRO)/devkitPPC")
endif

# powerpc-eabi-gcc (Rust's linker driver) and elf2dol come from devkitPro.
export PATH := $(DEVKITPPC)/bin:$(DEVKITPRO)/tools/bin:$(PATH)

TARGET_SPEC  := powerpc-gekko-none-eabi.json
TARGET_TRIPLE := powerpc-gekko-none-eabi
PROFILE      := release

# Link wiring for every GameCube binary, applied only to the GC target via
# CARGO_TARGET_*_RUSTFLAGS (host tools such as build scripts are unaffected):
#  - -mogc is already handled by the target spec (linker script, crt0)
#  - libogc/libsysbase/newlib/libgcc close the symbol references
export CARGO_TARGET_POWERPC_GEKKO_NONE_EABI_RUSTFLAGS := \
	-L$(DEVKITPRO)/libogc/lib/cube \
	-Clink-arg=-Wl,--start-group \
	-Clink-arg=-logc \
	-Clink-arg=-lsysbase \
	-Clink-arg=-lc \
	-Clink-arg=-lm \
	-Clink-arg=-lgcc \
	-Clink-arg=-Wl,--end-group

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

.PHONY: all list clean run $(EXAMPLES)

all: $(DOLS)
	@echo built: $(DOLS)

list:
	@echo $(EXAMPLES) | tr ' ' '\n'

$(EXAMPLES): %: dist/%.dol
	@echo built: $<

dist/%.dol: FORCE
	@mkdir -p dist
	cargo build --release --target $(TARGET_SPEC) -p $*
	elf2dol target/$(TARGET_TRIPLE)/$(PROFILE)/$* $@

FORCE:

clean:
	cargo clean
	rm -rf dist

run: dist/$(EXAMPLE).dol
	dolphin-emu --batch --exec=$<
