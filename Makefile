DEVKITPRO ?= $(HOME)/devkitpro
export DEVKITPRO
CARGO := $(HOME)/.cargo/bin/cargo
export RUSTC := $(shell rustup which rustc)
CC := $(DEVKITPRO)/devkitA64/bin/aarch64-none-elf-gcc
TOOLS := $(DEVKITPRO)/tools/bin
NX := $(DEVKITPRO)/libnx
ARCH := -march=armv8-a -mtune=cortex-a57 -mtp=soft -fPIE
CFLAGS := $(ARCH) -O2 -g -Wall -Wextra -ffunction-sections -fdata-sections -D__SWITCH__ -I$(NX)/include
RUSTLIB := target/aarch64-unknown-none/release/libswagent.a
.PHONY: all rust clean check
all: dist/atmosphere/contents/430000000000000B/exefs.nsp
rust:
	$(CARGO) build --release
build/shim.o: native/shim.c
	mkdir -p build
	$(CC) $(CFLAGS) -c $< -o $@
build/swagent.elf: rust build/shim.o swagent.json
	$(CC) $(ARCH) -specs=$(NX)/switch.specs -Wl,-Map,build/swagent.map,--wrap=exit,--wrap=_exit,--wrap=abort build/shim.o $(RUSTLIB) -L$(NX)/lib -lnx -lm -o $@
build/exefs/main: build/swagent.elf
	mkdir -p build/exefs
	$(TOOLS)/elf2nso $< $@
build/exefs/main.npdm: swagent.json
	mkdir -p build/exefs
	$(TOOLS)/npdmtool $< $@
dist/atmosphere/contents/430000000000000B/exefs.nsp: build/exefs/main build/exefs/main.npdm
	mkdir -p $(@D)/flags
	$(TOOLS)/build_pfs0 build/exefs $@
	touch $(@D)/flags/boot2.flag
	cp $@ build/swagent.nsp
check: all
	mkdir -p build
	cc -Wall -Wextra -c tests/host.c -o build/host.o
	RUSTFLAGS="-C link-arg=$(CURDIR)/build/host.o -A dead_code" $(CARGO) test --target aarch64-apple-darwin
	python3 tests/check_artifact.py
clean:
	rm -rf build dist
	$(CARGO) clean
