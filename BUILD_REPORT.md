# Build report — 2026-11-04

- Version: `swagent-1.1-rs`; title ID: `430000000000000B`.
- Headless commands: ping, version, install URL [sd|nand], update URL, log [N].
- Removed screenshot/PNG/base64, HID injection, capture/JPEG FFI and portlib dependencies.
- `make` succeeds with devkitA64/libnx and the stable bare-metal Rust target.
- Artifact: `build/swagent.nsp`, also deployed under `dist/atmosphere`.
- `make check` was attempted: host TCP binding is denied by the execution sandbox
  (Operation not permitted), causing shared-lock poisoning in subsequent tests.
- Non-TCP host tests and independent artifact validation pass when run separately.
- Artifact validation checks PFS0, NSO, NPDM title ID, AArch64 ELF, version,
  absence of capture/HID/JPEG symbols and process-exit implementations, and
  equality of the distribution artifact and convenience copy.
- Update validation accepts smaller headless images while retaining complete
  PFS0/NSO/NPDM identity validation and failed-launch rollback.
- No Switch hardware was available; real installation and PM handover remain
  unverified on device.
