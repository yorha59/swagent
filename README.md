# swagent — swagent-1.9-rs

Minimal resident Nintendo Switch agent. Title ID `430000000000000B`, TCP port
`6000`, no UI, no polling tasks beyond accepting commands. Path B: Rust `no_std`
staticlib and a small C ABI shim, linked with devkitA64/libnx's stock CRT.
All command, queue, HTTP, installation and handover policy is Rust.
See [RESEARCH.md](RESEARCH.md) for the build-path decision.

> **Disclaimer** — This software is provided for educational and research
> purposes, for use on consoles you own, with content you are legally entitled
> to install. It contains no copyrighted keys, firmware, or proprietary code.
> The authors are not affiliated with Nintendo. Use at your own risk.

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

Requires `$HOME/devkitpro` (override `DEVKITPRO`), libnx, devkitA64 and Switch tools.

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
then recovers. Commands are case-sensitive.

| Command | Response |
| --- | --- |
| `ping` | `pong` |
| `version` | `swagent-1.9-rs` |
| `install http://host[:port]/file.nsp [sd\|nand] [md5=HEX]` | `install: ok TITLEID`, or `install: error WHAT rc=N` |
| `update http://host[:port]/exefs.nsp` | `update: ok` then disconnect, or `update: error WHAT rc=N` |
| `launch TITLEID` | `launch: ok TITLEID`, or `launch: error rc=N` |
| `quit TITLEID` | `quit: ok PID`, `quit: not running`, or `quit: error rc=N` (self: `quit: refusing self`) |
| `log [N]` | Up to N ring entries, then `log: end` (default 100, max 1000) |

`launch` and `quit` require exactly 16 hexadecimal digits without a `0x`
prefix; successful launch replies normalize the ID to lowercase. Invalid
arguments return `COMMAND: error expected 16-hex-digit title ID rc=0`.
Launch requests use `appletAE` → `GetDebugFunctions` (1000) →
`RequestLaunchApplicationWithUserAndArgumentForDebug` (30), with no user IDs
or launch argument. HOME Menu handles the request, including any required UI.
`launch: ok` means the request was accepted, not that the title has started.
This requires the following boot-time setting in
`sd:/atmosphere/config/system_settings.ini` (merge into any existing section):

```ini
[am.debug]
dev_function = u8!0x1
```

Reboot after changing it. Without the debug interface, launch returns a nonzero
error; it never falls back to direct PM application launching. No applet proxy
or launcher registration is used. FW 23 hardware validation is still required;
see [LAUNCH_RESEARCH.md](LAUNCH_RESEARCH.md) for evidence and limits.
The ring log records launch attempts and results. Quit scans the same process
list as `ps`, skipping failed program-ID lookups (processes may have exited),
and uses the same PM termination path and self-protection as `kill`.

The receive thread executes `ping`, `version` and `update` directly. Every
other command uses the eight-entry worker queue; overflow returns exactly
`error: busy`. Install never occupies the receive thread. Responses are
serialized per line, so fast replies can appear between log lines when
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
size. Files are accessed through 64-bit libnx FS offsets.

Installation follows the inspected Goldleaf sequence:

1. Stream HTTP to `sdmc:/switch/.install.tmp.nsp` (up to 64 GiB).
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
the client, wait briefly for the replacement to bind, then terminate itself
cleanly via `pmshellTerminateProcess` (never `_exit`; an abnormal exit would be
flagged by creport, and sleeping forever would accumulate zombie instances until
the kernel refuses further launches). The replacement retries EADDRINUSE five
times at 1 s intervals; startup has an initial 5 s service delay. The old image
backup remains on SD.

Application code never calls a process exit. Linker wrappers also divert libc
`exit`/`_exit`/`abort` fallbacks into a parked thread, including library failure
fallbacks. Invariant panic handlers park the affected thread and leave an
in-memory diagnostic; all expected command/network/service errors return lines.

## Validation and limits

`make check` builds the real AArch64 NSP, runs host tests using libnx ABI doubles,
validates the generated
PFS0, NSO, NPDM title ID, ELF architecture and absence of linked process-exit
functions. Host doubles are never linked into the NSP.

Tests cover protocol framing/recovery, queue overflow/FIFO, URL rejection,
removed-command rejection, truncated/chunked HTTP, parser bounds, ring wrap
and concurrency, multi-chunk installs, ticket import, placeholder cleanup after
injected failures, update title validation and failed-launch rollback. A blocked
install test verifies `ping`, `version` and overflow replies on the live session.

No Switch hardware is attached: retail NCA installation
and PM relaunch still require on-device validation. The build artifact is a
proper ExeFS NSP; a successful host build does not establish runtime compatibility
with every Horizon/Atmosphere version.

## Source tree

```text
Cargo.toml, Cargo.lock, .cargo/config.toml  Rust staticlib, no dependencies
Makefile, swagent.json                     devkitA64/NSO/NPDM/PFS0 build
native/shim.c                             startup and ABI adapters
src/lib.rs, ffi.rs, util.rs                entry, bindings, bounded utilities
src/protocol.rs                            receive fast path, queue, update
src/http.rs, io.rs                         streaming HTTP and scoped handles
src/log.rs                                atomic ring
src/install.rs                            PFS0, CNMT, tickets, NCM/NS flow
src/tests.rs, tests/host.c                  host tests and libnx doubles
tests/check_artifact.py                    independent artifact checks
RESEARCH.md                               Phase 0 findings
```

System settings (1.9): `sleep off` disables both handheld and docked auto-sleep;
`sleep on` restores finite plans remembered during this process, or defaults to
10 minutes handheld / 1 hour docked after restart. Replies are `sleep: ok on|off`
(or `mixed` if readback differs). Other sleep flags are preserved. These are
persistent console settings. `set sleep` reads state and raw timeout-plan enums.
Read-only keys: `set firmware`, `set hostname` (console nickname, not DNS),
`set serial`, `set battery` (percent), `set temperature` (SoC, whole Celsius;
uses the modern ts session API, 10.0.0+). Responses are `set: KEY VALUE`.
Service failures return command errors with the result code. No generic settings
writer or ini writes are provided.

### Download integrity (swagent-1.13-rs)

`install URL sd md5=HEX` (also `nand`) verifies the entire completed temporary
file, including any resumed prefix, before PFS0 validation or NCM writes. The
optional digest must contain 32 hexadecimal digits (case insensitive). Only MD5
is supported; SHA-256 arguments are rejected. Without a digest, installation
retains its previous behavior and has no end-to-end integrity guarantee.
Hashing streams through the existing 4 MiB buffer. Success logs and replies
`install: hash ok`. A mismatch deletes the temporary NSP, logs and replies
`install: error hash mismatch (got X expected Y) rc=0`, and stops installation.

`hash SD-PATH` replies `hash: md5 HEX`. It uses the same read-path whitelist as
`cat` (any SD path, no traversal), with an 8 GiB size cap and a 4 MiB streaming
buffer. This cap applies only to the query; install verification supports the
full 64 GiB download limit. MD5 detects accidental corruption; it does not
provide authenticity against deliberate modification.

Resumed responses must be 206 with a valid Content-Range whose total equals
the original response's known total. A 200 response discards the prefix and
restarts with a truncated file and a zero byte counter. A 416 response is an
error, never proof that a partial file is complete. When the original total is
unknown (e.g. chunked transfer), a 206 resume is rejected rather than guessed.


### Manual hbmenu edition (1.17)

Run `make nro` and copy `dist/switch/swagent.nro` to `/switch/swagent.nro`
on the SD card. Launch it in hbmenu through Album. The console displays
`swagent-nro swagent-1.18-rs`, the exact `getsockname` bind endpoint
(`0.0.0.0:6000`). Find the console's LAN IP in system settings and connect on TCP
port 6000. Stop the sysmodule first if it already owns that port.

PLUS or a bare TCP `quit` exits the NRO. `quit <title-id>` retains the
existing title termination command. Both editions use the same Rust command
handlers, installation services, 16 MiB heap and reusable 4 MiB install chunk.
`version` reports `swagent-1.18-rs nro` or `swagent-1.18-rs sys`.
The `update` command retains its resident sysmodule update behavior; it does
not replace the running NRO file. Installation uses the existing NS/FS path
and hbloader permissions; actual service availability depends on the console.

`tools/nro_packer.py` uses only Python's standard library. It preserves the
libnx startup stub and MOD0 offsets and writes the standard 0x70-byte NRO0
header at offset 0x10. NRO2 is not a format accepted by Switch hbmenu.
`make check` builds both editions and checks host behavior and both containers.
