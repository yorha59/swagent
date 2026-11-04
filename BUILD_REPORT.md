# Build report — 2026-11-04

- Path B: stable Rust bare-metal staticlib + libnx/devkitA64 ABI shim and CRT.
- Version: `swagent-1.0-rs`; title ID: `430000000000000B`.
- Compiler actually used: rustup stable **1.93.1**, target `aarch64-unknown-none`.
  Homebrew's separately installed compiler is 1.99.1.
- Rust source: **2,311 lines total**, including **411 test lines**; production
  Rust: **1,900 lines**. C shim: **178 lines**; host-only test doubles: 69 lines.
- Build: `make -j2` succeeds; `make check` succeeds: 12 host tests, independent
  PNG validation, generated NSP/NPDM/NSO/AArch64 ELF validation. No linked
  `_exit`, `exit` or `abort` implementation remains after linker wrapping.
- Deliverable: `dist/atmosphere/contents/430000000000000B/exefs.nsp`.
- Equivalent convenience copy: `build/swagent.nsp`.
- Artifact size: **306,560 bytes**.
- SHA256: `3f9b05428005594fecabd5d49d2eb917567369ef22a114ac65ed7c4a262cde1d`.
- ELF section accounting: text 442,005; data 16,336; BSS 17,026,736 bytes.
  BSS includes the fixed 16 MiB heap, atomic log ring and libnx static storage.
- Deployment: merge `dist/atmosphere` onto the SD root, retaining the generated
  `flags/boot2.flag`, then reboot Atmosphere. TCP port 6000 accepts CRLF lines.
- Source tree, protocol, memory limits, install flow, update behavior and device
  validation checklist are documented in `README.md`.
- No Switch hardware was available. Caps/HID, real ES/FSP/NCM/NS service behavior
  and PM handover need on-device testing. Host tests use ABI doubles for those
  services and real host sockets/files for streaming.

Milestones are committed in git: build-path decision; first linked resident
agent; validation, error handling and deployment documentation.
