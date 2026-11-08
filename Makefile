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
.PHONY: all nro rust clean check
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
build/shim-nro.o: native/shim.c
	mkdir -p build
	$(CC) $(CFLAGS) -DSA_NRO -c $< -o $@
build/swagent-nro.elf: rust build/shim-nro.o
	$(CC) $(ARCH) -specs=$(NX)/switch.specs -Wl,-Map,build/swagent-nro.map build/shim-nro.o $(RUSTLIB) -L$(NX)/lib -lnx -lm -o $@
nro: dist/switch/swagent.nro
dist/switch/swagent.nro: build/swagent-nro.elf tools/nro_packer.py
	mkdir -p $(@D)
	$(DEVKITPRO)/tools/bin/elf2nro $< $@
check: all nro
	mkdir -p build
	cc -Wall -Wextra -c tests/host.c -o build/host.o
	RUSTFLAGS="-C link-arg=$(CURDIR)/build/host.o -A dead_code" $(CARGO) test --target aarch64-apple-darwin
	python3 tests/check_artifact.py
	python3 tests/test_nro_packer.py
clean:
	rm -rf build dist
	$(CARGO) clean
