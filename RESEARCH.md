# Phase 0 decision

Path B: stable Rust `no_std` staticlib targeting `aarch64-unknown-none`, linked
with devkitA64 and libnx's normal CRT, switch.specs, NSO and NPDM tools.
The precompiled Rust core component installs successfully; no nightly/build-std
or bespoke runtime is necessary. ABI wrappers stay in C because libnx IPC
helpers are C macros/inline functions and its HID structures change by firmware.
No C++ sources are linked.

2026-11-04: curl requests to GitHub API and raw Cargo.toml for
https://github.com/aarch64-switch/nx and
https://github.com/aarch64-switch/switch-rs returned 404. Rather than depend on
an unverified historical Rust Switch ecosystem, use the installed libnx runtime.
Research ended well inside 30 minutes.

Local references inspected for interface behavior: /tmp/sbbc (caps JPEG, HDLS,
retire-and-sleep), Goldleaf nsp_Installer.cpp (NCM placeholder/meta/NS record
flow), /tmp/glm_install_wip/installer.cpp (IPC numbers and layouts), installed
libnx headers. Implementation is newly written; no reference C++ is compiled.
"Clean-room" here means an independent rewrite from inspected interfaces and
behavior, not a legally separated clean-room process.
