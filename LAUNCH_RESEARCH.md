# Foreground application launch research — swagent-1.8-rs

Researched 2026-11-06. Decision: implement a **conditional AM debug launch
request**, replacing direct PM game launching. This is a documented call
sequence, not a claim of successful FW 23 hardware validation. No console or
boot settings were changed by this work.

## Result decoding

Atmosphere revision `6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8`:

- [results_common.hpp:47–75](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/libraries/libvapours/include/vapours/results/results_common.hpp#L47):
  module = `rc & 0x1ff`; description = `(rc >> 9) & 0x1fff`;
  bits 22–31 are reserved. The proposed `0x7ff` description mask is too small.
- `0xAD800A08` and `0xAF400A08` both decode to **module 8, description 5**,
  inner result `0xA08` (display form `2008-0005`). Their reserved bits differ.
  They do NOT decode to module 5, description 8.
- [lr_results.hpp:20–31](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/libraries/libvapours/include/vapours/results/lr_results.hpp#L20)
  identifies module 8 as LR (Location Resolver), but does **not** define
  description 5. [Switchbrew's reverse-engineered error table](https://switchbrew.org/wiki/Error_codes#Error_codes)
  calls `0xA08` `ApplicationNotFound`. That name is not an Atmosphere definition.
- [pm_results.hpp:20–32](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/libraries/libvapours/include/vapours/results/pm_results.hpp#L20)
  defines PM as module **15**, ApplicationRunning as description **5** (`0xA0F`).
- [ncm_results.hpp:20–30](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/libraries/libvapours/include/vapours/results/ncm_results.hpp#L20)
  defines module **5** as NCM, description **8** as AllocationFailed.
  Neither AM nor PM is module 5.

There is no Atmosphere AM implementation or `am_results.hpp` in this revision.
It is therefore impossible to cite an Atmosphere AM file/line proving that a
caller-identity check rejected these particular hardware requests. Their result
module instead points to resolution. The source of the differing reserved bits
has not been established.

## Why changing PM flags does not solve foreground launching

[pm_process_manager.cpp:120–132](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/stratosphere/pm/source/impl/pm_process_manager.cpp#L120)
gets program information through loader, derives application type from program
metadata, and rejects a second application with `pm::ResultApplicationRunning`.
This is NOT a check requiring the caller to be an applet. Subsequent PM work
creates/registers/starts the process; it does not perform AM's foreground and
applet lifecycle management. A successful process launch alone would not prove
a usable foreground game launch.

The previous assertion that storage 0 automatically resolves installed games
was unsupported: [ldr_content_management.cpp:80–85](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/stratosphere/loader/source/ldr_content_management.cpp#L80)
special-cases None by returning `/`, rather than searching installed storage.
PM with storage None remains appropriate for swagent's existing SD program
self-update path; this work does not change that path.

## A route without an applet proxy

[Switchbrew AM interface research](https://switchbrew.org/wiki/AM_services#GetDebugFunctions)
documents root `appletAE` command **1000**, GetDebugFunctions. This is distinct
from command 1000 on an already-open applet proxy. It requires AM's cached
`am.debug!dev_function` to be one byte with value 1. Without it AM can return
success with a null interface. It is not ordinary applet registration.

The returned IDebugFunctions supports command **30**,
[RequestLaunchApplicationWithUserAndArgumentForDebug](https://switchbrew.org/wiki/AM_services#RequestLaunchApplicationWithUserAndArgumentForDebug).
The implementation sends flag=false, the application ID, and two empty input
map-alias buffers (no selected users and no user argument). It requests HOME
Menu launch handling rather than directly creating a process. The command ABI
matches [libnx applet.c](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/source/services/applet.c#L2858).

Sequence implemented: get appletAE → convert session to domain → root command
1000 → reject null object → debug command 30 → close child and root sessions.
No call to appletInitialize, no fabricated applet type, no PM game fallback.
The protocol's `launch: ok TITLEID` now means **request accepted**, not launch
completion; HOME Menu may show UI or reject the subsequent launch.

Required configuration, merged into `atmosphere/config/system_settings.ini`:

```ini
[am.debug]
dev_function = u8!0x1
```

Reboot after changing it, because AM caches the setting. Atmosphere's separate
`[atmosphere] enable_am_debug_mode` setting overrides GetDebugModeFlag; it is
not the same setting and does not replace `dev_function`:
[setsys_mitm_service.cpp:260–269](https://github.com/Atmosphere-NX/Atmosphere/blob/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8/stratosphere/ams_mitm/source/set_mitm/setsys_mitm_service.cpp#L260).

This documented conditional route prevents a defensible universal NO. Its
availability, zero-user handling, and eventual foreground transition still need
validation on the user's FW 23 console. An unconditional, hardware-proven YES
would also exceed the available evidence.

## Other interfaces and precedents

- hbmenu normally chainloads an NRO using `envSetNextLoad`, within the existing
  hbl process, rather than launching an installed title through PM:
  [builtin.c:54–85](https://github.com/switchbrew/nx-hbmenu/blob/089fdf85ff82ad7077527092f8615f2e57ffc985/nx_main/loaders/builtin.c#L54).
- libnx's ordinary `appletRequestLaunchApplication` checks application/library
  applet context. It uses ILibraryAppletSelfAccessor command 90 in library
  applet mode; ILibraryAppletAccessor is a different interface. See
  [applet.c:1999–2028](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/source/services/applet.c#L1999).
  Setting swagent's AppletType variable alone does not register it with AM.
- uLaunch supplies a concrete full lifecycle example: its qlaunch replacement
  runs as SystemApplet, creates an application, pushes launch parameters,
  starts it and requests foreground. It is not an ordinary background
  sysmodule precedent:
  [app_Application.cpp:115–145](https://github.com/XorTroll/uLaunch/blob/6da94f9164f5fa61380010008270457f95ff4b04/projects/uSystem/source/ul/system/app/app_Application.cpp#L115).
- [NS interface research](https://switchbrew.org/wiki/NS_services#IApplicationManagerInterface)
  lists modern ApplicationManager command 94; its input changed to 0x88 bytes
  on FW 19+. A raw title-ID-only call to an old LaunchApplication command is
  incorrect on FW 23. Its existence is not proof of complete AM foreground
  lifecycle management for swagent. No verified such sequence was found.
- Tinfoil/DBI operating in an application/applet environment do not establish
  background sysmodule capability. No independently verified FW 23 background
  game-launching homebrew precedent was found in this research; that absence
  does not prove impossibility.
- The Tesla premise needs correction: Tesla overlays are loaded by the
  nx-ovlloader sysmodule, not necessarily injected into an AM applet. Status
  Monitor's bundled libtesla explicitly sets `AppletType_None`:
  [tesla.hpp:2639](https://github.com/masagrator/Status-Monitor-Overlay/blob/0434c0bac70f4fd2a82db85179f283fb9c504d77/lib/libtesla/include/tesla.hpp#L2639).
  This does not establish a game-launch capability.

## Validation

The requested devkitPro `make -j8` build succeeded. Repository `make check`
passed all 24 host tests and the NSP/NPDM/ELF artifact checks. Host tests cover protocol validation, accepted requests, failed
requests, quit behavior and self-update rollback. They mock the C service
boundary and do not validate AM IPC or actual game startup. On-device testing
is required both with debug functions disabled (nonzero error, no fallback)
and enabled (request accepted and HOME Menu launches a known installed title).

Deliverable: `dist/atmosphere/contents/430000000000000B/exefs.nsp`, 128,066
bytes. SHA-256: `7a9f3d61119e75d43e260a933da15789b0cd95fe08d946b5f7a95399fb4879b8`.
Version remains `swagent-1.8-rs`. No commit created.
