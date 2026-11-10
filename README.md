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


### Manual hbmenu edition (1.32)

Run `make nro` and copy `dist/switch/swagent.nro` to `/switch/swagent.nro`
on the SD card. Launch it in hbmenu through Album. After binding, the console
prints `swagent-1.34-rs listening IP:6000 (press HOME to exit)` with the LAN IP.
The listener binds all interfaces. If the sysmodule owns port 6000, the NRO
shows a busy message and retries every five seconds, taking over automatically
when the port becomes available. PLUS displays the HOME instruction while waiting.

Bare TCP `quit` replies `quit: ok` and closes that client in both editions;
in the NRO it leaves the agent running until HOME system teardown. `quit <title-id>` retains
the existing title termination command. Both editions share Rust command
handlers and the reusable 4 MiB install chunk. The resident edition uses its
16 MiB static heap; the NRO uses libnx's hbloader-provided heap and standard
applet/HID/FS startup. It initializes the console before starting network workers,
skips resident sleep-setting recovery, firmware probing and the SD log writer,
and prints the endpoint only from the console thread.
`version` reports `swagent-1.34-rs nro` or `swagent-1.34-rs sys`.
The `update` command retains its resident sysmodule update behavior.

`make nro` uses official devkitPro `elf2nro` and libnx crt0/relocation handling.
`tools/nro_packer.py` remains an independently tested packaging utility.
`make check` builds both editions and checks host behavior, containers, and NRO
entry symbols/init gating. Launch and IPC behavior still require Switch hardware.


Version 1.19 diagnostics: `log [count]` runs alongside a blocked command and dumps
both the ring and a cached 16 KiB tail of `/switch/swagent.log`, including prior
boots. A dedicated writer appends and flushes every line, truncating files over
256 KiB. Producers never wait for SD: a full queue, contended snapshot or SD error
may drop diagnostics. Boot and install traces include uptime, init results,
connection attempts/endpoints and 4 MiB receive progress.

`installed` lists NS application records (NAND and SD) as raw title ID plus an
empty name. In 1.22 it opens the install path's application-manager interface
(`ns:am2` command 7996 on modern firmware) directly and dispatches
ListApplicationRecord command 0. It avoids `nsInitialize` service fallback and
optional control-data queries. `ns:su` is SystemUpdate, not application management. `installed del 430000000000000B
del-430000000000000B-confirm` deletes that application completely through NS.
The confirmation must match the exact 16-digit title ID argument, including case.
NS service failures are returned as errors; hardware validation is required for
service availability on each firmware/sysmodule environment.


Version 1.20 worker handoff: the producer publishes a job and then signals a
counting libnx semaphore; the persistent consumer waits on that semaphore rather
than polling. Startup initializes the semaphore before spawning any lanes.
`worker: ready`, `enqueue: id=... uptime_ms=... cmd=...`, and
`pickup: id=... queue_ms=... cmd=...` distinguish startup, arrival, and pickup.
Queue wait time includes time behind another command; a busy worker cannot
promise a one-second response. The regression test admits a socketpair through
the listener, runs client framing and the persistent worker on separate threads,
and requires eight spaced `ps` replies within one second each.

The 1.16 queue inspection found no missing readiness flag or signal: it used a
5 ms poll, not a notification primitive. The 1.19 source logged install arguments,
not every worker command. Host verification does not establish the cause of the
reported hardware stall; the new timestamps are intended to locate it if it
persists. Version 1.22 builds both the sysmodule NSP and NRO.

Version 1.21 makes the worker fallback explicit after fast-path returns. Unknown
commands also enter the queue and reply `error: unknown command: X`. All 1.19
and 1.20 diagnostics remain. The host regression now uses a real Unix listening
socket and native `accept()`, followed by admission, client framing, dispatch
and the persistent worker. Hosts prohibiting socket bind use a real socketpair
with only accept substituted, and print that limitation. It verifies replies and fresh enqueue
logs for eight `ps` commands and an unknown `zzz` command. The checked-in 1.20
source already had a catch-all enqueue arm; this change does not establish why
the hardware image omitted those traces. Only the sysmodule NSP is rebuilt.

### 1.23 progress watchdog

`version` reports `swagent-1.23-rs`. Install, update and HTTP downloads use a
30-second inactivity deadline, renewed by successful receives (up to the existing
4 MiB chunk size) and disk writes. There is no total runtime ceiling. Retries and
native call entry do not renew it; blocked calls still quarantine the worker.
Budget extensions emit a `debug: budget extended` log at most once per minute.

`put PATH SIZE` accepts up to 4 MiB, including binary NRO payloads. The wire
receiver retains its existing 512-byte streaming buffers and does not allocate a
second large buffer. `cat` retains its 256 KiB limit and the heap remains 16 MiB.


swagent-1.25-rs keeps the 1.24 download retry, reconnect and Range behavior.
Completed install writes, NCA registration and PFS/MD5 validation renew the
same 30-second progress deadline; install IO blocks are capped at 64 MiB.
Content writing logs `install: writing contents pct=N` at every 5% threshold
(including confirmed contents already present). Host regression coverage runs
the disk installer with simulated 200 ms writes for over ten grace windows,
and verifies a stalled write still expires.


Version 1.26 (`swagent-1.26-rs`) preserves `consoleInit(NULL)` as the very
first NRO entry call, before output or worker startup. The checked-in entry
already had this order; regression checks now enforce it in the preprocessed
NRO source and linked entry point. Printing before console initialization can
crash in `ConsoleSwRenderer_drawChar` with a NULL framebuffer. The hbmenu
console continues to display the LAN IP and port 6000.

Version 1.27 (`swagent-1.27-rs`) displays a startup banner and refreshes the
console after each status print and every 100ms while polling PLUS. Bind
failures retry every five seconds; a busy port shows the sysmodule warning,
and successful takeover displays the LAN IP and port.

Version 1.32 (`swagent-1.34-rs`) leaves NRO teardown to the system. Press HOME
to exit. PLUS only prints `press HOME to exit` and refreshes the console; main
never returns, including when the applet loop reports exit or startup fails.
Workers keep running until system teardown. Bare `quit` disconnects the client.
The custom socket-close/join/consoleExit sequence has been removed following
three reported bsdsocket crashes on hardware in 1.27–1.29; the reported HOME
system teardown did not crash. Host checks cannot establish hardware safety.
The banner, live console logs, busy-port message and five-second listener retry
remain unchanged.

`rm <SD-path>` deletes one exact file and replies `rm: ok`, or `rm: error ...`.
It uses the put whitelist (`switch/`, `atmosphere/config/`, `bootloader/`),
rejects traversal and extra arguments, and uses file-only deletion so directories
are refused. No wildcard expansion or recursive deletion is performed.

`tail <SD-path> [n]` (default 40) scans backwards in 4 KiB blocks and streams
selected bytes between `tail: begin` and `tail: end`. Outbound connections retain
the 15-second per-attempt poll timeout and close failed sockets before retrying.

### Installed-title NSP export (1.32)

`dump <16-hex-digit-titleid> [SD-path]` runs on the install worker lane and
exports exactly that base or update title. The default is
`sdmc:/switch/dump/<titleid>.nsp`; explicit paths must be under `/switch/`,
without `..`. `status` reports the shared phase/rx/total/pct counters.
NSP output uses a 4 MiB stream buffer (dump allocations remain below 8 MiB).
Errors remove partial output, enter the ring/persistent log, clear pending
worker jobs and back off before accepting more work. Missing tickets produce a
warning and a final `notik` suffix.

Common tickets and their certificate chains use ES export commands 22/23.
Personalized plaintext tickets use read-only ES savedata. **Encrypted/volatile
personalized tickets are not yet supported**: they return an error rather than
silently creating a ticketless archive. Read-only savedata mounting may also
fail when ES owns the save; this propagates as an error. This is a remaining
limitation relative to nxdumptool's raw-save and ES-memory decryption path.
No console deployment has been performed.

Host verification (fixtures are required, not silently skipped):

```sh
cc -Wall -Wextra -c tests/host.c -o build/host.o
RUSTFLAGS="-C link-arg=$PWD/build/host.o -A dead_code" cargo test --target aarch64-apple-darwin
DEVKITPRO=~/devkitpro DEVKITA64=~/devkitpro/devkitA64 make
```

### Listener and persistent log recovery (1.32)

A separate low priority listener watchdog checks socket metadata every 30 seconds.
A bad listener or an accept loop without a heartbeat for 60 seconds triggers
shutdown/close and a fresh socket/bind/listen cycle. Failed binds retry forever
with exponential backoff capped at 30 seconds (initially 250 ms for sysmodule,
5 seconds for NRO). Recovery stages are queued to the persistent log.

Clients must finish a command line within 60 seconds of connecting or completing
the previous line; partial input does not renew this deadline. Upload bodies keep
the existing 30-second receive timeout. The log writer reopens its handle once
after 10 consecutive failures and truncates before exceeding 1 MiB; the in-memory
ring continues recording when SD writes fail. These mechanisms handle returned
errors and stalled accept calls that shutdown can release; they cannot guarantee
recovery from a permanently blocked BSD/FS service IPC. See PR_DESCRIPTION.md for
the socket ownership audit and host regression coverage.


### Stage timing and independent heartbeat (1.33)

`6001/TCP` is a dedicated heartbeat thread: send any byte, receive `pong\n`,
then EOF. It bypasses the command worker, SD, logging and connection registry
locks. Silent clients are closed after two seconds. Bind retries use the same
250 ms (NRO: 5 s) to 30 s exponential backoff as port 6000. The listener
watchdog requests a socket health probe; the heartbeat thread owns all closes.

Install and dump emit stage-level `span op=... title=... phase=... t=...ms bytes=...`
lines through the existing log queue and 1 MiB rotation. Active stages emit
`progress=38.1%` at most once per 30 seconds, including stalled IPC stages via
the watchdog. Before install metadata identifies a title, `title=0000000000000000`;
unknown stage totals report 0.0%. There is no per-block log emission. Span work
is skipped when the log writer heartbeat is absent or stale (60 seconds).

### Whole-system reboot (1.34)

`reboot` synchronously appends and flushes `reboot: requested` to
`sdmc:/switch/swagent.log`, sends `reboot: ok`, closes the client connection,
then waits 500ms before calling libnx bpc to reboot the console. No arguments
are required. A log write failure prevents reboot and returns an error.
If bpc returns, its result is logged and the service remains available.
With hekate autoboot configured, the console returns to emuMMC CFW and the
resident swagent starts again. The receipt confirms the request, not boot success.
