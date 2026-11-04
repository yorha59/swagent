# swagent — swagent-1.0-rs

Minimal resident Nintendo Switch agent. Title ID `430000000000000B`, TCP port
`6000`, no UI, no polling tasks beyond accepting commands. Path B: Rust `no_std`
staticlib and a small C ABI shim, linked with devkitA64/libnx's stock CRT.
All command, queue, HTTP, PNG, installation and handover policy is Rust.
See [RESEARCH.md](RESEARCH.md) for the build-path decision.

## Build and deploy

This machine has two Rust installations. Homebrew's `rustc` is 1.99.1; the
rustup stable compiler actually installed is 1.93.1. The Makefile explicitly
uses rustup's cargo/compiler, whose prebuilt bare-metal core target is available.
No nightly, build-std, bindgen or third-party Rust crates are required.

```sh
rustup target add aarch64-unknown-none
make -j2
make check
```

Requires `$HOME/devkitpro` (override `DEVKITPRO`), libnx, devkitA64, Switch tools
and the installed Switch portlib `libturbojpeg`. TurboJPEG supplies fallible JPEG
decoding; Rust streams an RGB PNG with stored DEFLATE blocks and base64 encoding.

Copy the generated `dist/atmosphere` directory onto the SD root, merging folders:

```sh
cp -R dist/atmosphere /Volumes/YOUR_SWITCH_SD/
```

The installed paths are:

```text
atmosphere/contents/430000000000000B/exefs.nsp
atmosphere/contents/430000000000000B/flags/boot2.flag
```

Reboot Atmosphere to replace the currently loaded C++ module. Subsequent
`update` commands use resident handover. A copy of the exact NSP is also at
`build/swagent.nsp`; ELF, map and unpacked ExeFS are under `build/`.

Connect using the Switch's IP. Every command and response ends in CRLF:

```sh
printf 'ping\r\nversion\r\n' | nc SWITCH_IP 6000
```

## Protocol

One active TCP client. Lines are ASCII, at most 511 bytes excluding CRLF.
Malformed/oversized lines are discarded through the next CRLF; the stream
then recovers. Commands and button names are case-sensitive.

| Command | Response |
| --- | --- |
| `ping` | `pong` |
| `version` | `swagent-1.0-rs` |
| `shot` | Base64 PNG lines (up to 768 characters), then `shot: end` |
| `key BUTTON [hold_ms]` | `key: ok`, or `key: error WHAT rc=N` |
| `touch x y [hold_ms]` | `touch: ok`, or `touch: error WHAT rc=N` |
| `install http://host[:port]/file.nsp [sd\|nand]` | `install: ok TITLEID`, or `install: error WHAT rc=N` |
| `update http://host[:port]/exefs.nsp` | `update: ok` then disconnect, or `update: error WHAT rc=N` |
| `log [N]` | Up to N ring entries, then `log: end` (default 100, max 1000) |

Buttons: `A B X Y L R ZL ZR PLUS MINUS LEFT UP RIGHT DOWN LSTICK RSTICK`
(with `+ - LS RS` aliases). Hold defaults to 100 ms, range 0–60000 ms.
Touch coordinates: x 0–1279, y 0–719. Release is attempted even when press fails.
Capture errors return `shot: error WHAT rc=N` without `shot: end`.

The receive thread executes `ping`, `version` and `update` directly. Every
other command uses the eight-entry worker queue; overflow returns exactly
`error: busy`. Install never occupies the receive thread. Responses are
serialized per line, so fast replies can appear between screenshot/log lines.
Discard `pong`/version responses before concatenating screenshot base64 if
pipelining commands. Jobs from a disconnected session are discarded. A running
install continues through a client disconnect and records completion in the ring.

`update` intentionally runs synchronously on the receive thread; fast commands
resume by reconnecting to the replacement after a successful update. It never
enters the worker queue.

## Storage, installation and handover

Logging is an atomic, in-memory 1000-slot ring with entries capped at 191 bytes.
There are **no SD log files**, flushes or file logging paths. Formatting and
socket output never hold a log mutex. Queue/session locks are independent.

A fixed 16 MiB newlib heap serves fallible Rust buffer allocations and libraries.
Rust uses no allocating collections or global allocator. Install has one 4 MiB
buffer reused for download, CNMT extraction and NCM placeholder writes. HTTP
accumulates full buffers before writing, with a final partial buffer. PFS0 headers
are capped at 256 KiB / 2048 files; packaged/rebuilt metadata at 512 KiB each.
The static ring is about 216 KiB outside the heap. Concurrent update download
uses only 64 KiB, rather than another 4 MiB buffer. No allocation scales with NSP
size. PNG encoding streams scanlines; only bounded JPEG and decoded RGB reside
in memory. Files are accessed through 64-bit libnx FS offsets.

Installation follows the inspected Goldleaf sequence:

1. Stream HTTP to `sdmc:/switch/.install.tmp.nsp` (up to 32 GiB).
2. Validate PFS0 names, table/file bounds, duplicate/overlapping entries.
3. Import matching ticket/certificate pairs through ES command 1, clearing the
   temporary ticket bit. Ticketless content is accepted; a missing certificate
   for a supplied ticket returns an error.
4. Extract the CNMT NCA, mount it through FSP ContentMeta, parse the packaged
   CNMT and validate every required NCA filename/declared size.
5. Create/write/register NCM placeholders, skipping content IDs already present.
   Delete each placeholder on success or failure through a scope guard.
6. Rebuild/commit the installed CNMT, merge existing application content records,
   then use NS application-manager commands 27/16 to delete/push the record.
   Restore old records if push fails. NS command 17 reads existing records.
7. Remove both installation temporary files on return and keep serving.

This minimal installer accepts one CNMT per NSP and Application/Patch/AddOnContent
metadata. It rejects system content, multi-CNMT bundles and metadata exceeding
its explicit bounds. FSP/ES/NCM validate supported formats and firmware keys.
Downloads require plain HTTP, status 200; Content-Length, connection-close and
chunked bodies are supported. HTTPS, redirects and compressed bodies return
errors. FAT32's individual-file limit still applies to the SD temporary NSP.
Failure after content registration/database commit can leave staged content;
this is not a transactional installer. It does not bypass service permission,
key-generation or content validation failures.

Update downloads to `exefs.nsp.tmp`, validates PFS0/NSO/NPDM and the exact title
ID, renames the old image to `exefs.nsp.bak`, swaps the new image, and launches
it using PM shell. A failed launch attempts to restore the previous image.
After success: mark retired, release the listening FD first, acknowledge/close
the client, then sleep forever. The worker observes retirement between chunks
and sleeps too. The replacement retries EADDRINUSE five times at 1 s intervals;
startup has an initial 5 s service delay. The old image backup remains on SD.

Application code never calls a process exit. Linker wrappers also divert libc
`exit`/`_exit`/`abort` fallbacks into a parked thread, including library failure
fallbacks. Invariant panic handlers park the affected thread and leave an
in-memory diagnostic; all expected command/network/service errors return lines.

## Validation and limits

`make check` builds the real AArch64 NSP, runs host tests using libnx ABI doubles,
independently checks PNG CRC/zlib/pixels with Python, and validates the generated
PFS0, NSO, NPDM title ID, ELF architecture and absence of linked process-exit
functions. Host doubles are never linked into the NSP.

Tests cover protocol framing/recovery, queue overflow/FIFO, URL rejection,
base64 boundaries, PNG output, truncated/chunked HTTP, parser bounds, ring wrap
and concurrency, multi-chunk installs, ticket import, placeholder cleanup after
injected failures, update title validation and failed-launch rollback. A blocked
install test verifies `ping`, `version` and overflow replies on the live session.

No Switch hardware is attached: actual caps/HID behavior, retail NCA installation
and PM relaunch still require on-device validation. The build artifact is a
proper ExeFS NSP; a successful host build does not establish runtime compatibility
with every Horizon/Atmosphere version.

## Source tree

```text
Cargo.toml, Cargo.lock, .cargo/config.toml  Rust staticlib, no dependencies
Makefile, swagent.json                     devkitA64/NSO/NPDM/PFS0 build
native/shim.c                             startup and ABI adapters
src/lib.rs, ffi.rs, util.rs                entry, bindings, bounded utilities
src/protocol.rs                            receive fast path, queue, HID, update
src/http.rs, io.rs                         streaming HTTP and scoped handles
src/image.rs, log.rs                       PNG/base64 and atomic ring
src/install.rs                            PFS0, CNMT, tickets, NCM/NS flow
src/tests.rs, tests/host.c                  host tests and libnx doubles
tests/check_png.py, check_artifact.py       independent format checks
RESEARCH.md                               Phase 0 findings
```
