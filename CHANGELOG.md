# Changelog

Historical notes moved from the old README. Newest first where dated.

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
prints `swagent-1.35-rs listening IP:6000 (press HOME to exit)` with the LAN IP.
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
`version` reports `swagent-1.35-rs nro` or `swagent-1.35-rs sys`.
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
second large buffer. `cat` now supports 512 KiB for screenshots; the heap remains 16 MiB.


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

Version 1.32 (`swagent-1.35-rs`) leaves NRO teardown to the system. Press HOME
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


### Listener recovery protocol (1.35)

Port 6000 has one socket owner: the listener thread. The watchdog publishes
metadata requests only; it does not probe, shutdown, close, or create the socket.
The owner probes metadata and polls nonblocking accept, consumes requests,
closes the old descriptor, then increments the generation before attempting
socket/bind/listen. The update worker also leaves listener close to its owner.
No recovery path resets the BSD service or the independent 6001 heartbeat lane.

Requests contain a monotonically increasing sequence, target generation, reason,
and uptime. Consecutive requests for one generation coalesce. A request for an
old generation is rejected; one arriving just before the generation advances is
acknowledged without touching the new descriptor. Requests for the generation
being built survive until the next owner iteration. Failed rebuilds wait 250ms
(5s minimum for NRO), doubling to 30s; successful bind/listen resets the delay.
Transient accept errors wait 20ms; hard accept errors wait 100ms before rebuilding.
Request storms do not bypass failed-build backoff.

Persistent logs record request/process/rebuild result, reason, old/new generation,
fd, request/handling uptime, rc and errno. Native stage records distinguish socket,
bind and listen results, including the attempted fd on failure. These records are
emitted on transitions, not on each accept poll. Existing 1MB rotation and close
after each log write are unchanged.

`rebuild-success health=unverified` means only that bind/listen/nonblocking setup
completed. Only a successful `pong` send on a connection admitted by that same
generation produces `application-reply health=reply-sent`; this is emitted once
per generation. Peer receipt remains unverified: no internal loopback is used.
For device validation, query 6001 and reconnect to 6000 and send `ping\r\n`,
then correlate the received pong with the generation logs. An old accepted client
cannot confirm a newly rebuilt listener. If the owner is permanently stuck inside
an OS call, requests remain pending; no watchdog can forcibly free that call under
this ownership protocol. The missing process/new-generation logs make this boundary
visible. Sleep/resume as an original trigger remains unproven by host tests.

发布前必须运行 `make release-check`（亦可 `make check`）。两者均强制真实 TCP 6001 bind、128 次重复连接及恢复期间 pong 验证；bind 不可用会打印明确 SKIP 原因并使验证 exit 非零，不能视为通过。默认 cargo test 同样禁止跳过后通过。版本保持 1.35。

`shot\r\n` captures the current display through libnx `caps:sc`
`capsscCaptureJpegScreenShot` (default layer stack, 1 second capture timeout).
Fetch it with `cat /switch/swagent.jpg\r\n` (existing `cat: begin LENGTH\r\n`,
exactly LENGTH binary bytes, `cat: end\r\n` framing), or verify it with
`hash /switch/swagent.jpg\r\n`. The `cat` limit is now 512 KiB to cover the
libnx JPEG buffer. `shot` takes no arguments; raw streaming is not implemented.
Capture runs on the command worker, uses a fallible 512 KiB allocation, and
releases `caps:sc` after each call. Errors reply `shot: error REASON rc=DECIMAL`;
initialization and capture errors are distinguished. Capture failure leaves any
previous JPEG untouched; filesystem write failures can leave a partial file.

This sysmodule (430000000000000B) already has wildcard service access. No applet
proxy is used. Local libnx documents JPEG capture on FW 9+, requiring debug mode
only before FW 10; the obsolete raw capture API is not used. FW 23.0.1 capture
and service policy still require hardware validation: host tests and a successful
build cannot establish that `caps:sc` will permit capture on a particular system.
