# Headless WiFi configuration research — swagent

Researched 2026-11-06 for a Switch 1 running Atmosphere / Horizon FW 23.
Decision: **research only**. The available sources establish a NIFM IPC route
for profile management and connection requests, but do not establish the
claimed WiFi savedata ID or its inner file format. No console was accessed.
No savedata was mounted, read, modified, deleted, or committed.

## Conclusion

A sysmodule has a documented programmatic route that does not inherently
require the Settings UI: obtain `nifm:a`, set a network profile, create a
network request, select its UUID, and submit it. This is a credible interface
sequence, **not a hardware-validated FW 23 success claim**. Privilege checks,
profile construction, arbitration with existing requests, and failure handling
still need validation. Direct savedata injection is unnecessary for that route.

The requested savedata parser cannot be responsibly implemented from the
evidence found. In particular, an IPC profile struct is not evidence of the
on-disk record layout. Per the task's condition, no `wifi` command or version
bump was made; the existing version remains `swagent-1.13-rs`.

## Storage location and mount paths

The premise `0x8000000000000056 = WiFi settings savedata` is **unverified**.
[Switchbrew's SystemSaveData table](https://switchbrew.org/w/index.php?title=SystemSaveData&oldid=15400)
lists these settings-owned saves, but has no entry for `…0056`:

| System savedata ID | Owner's documented mount |
| --- | --- |
| `0x8000000000000050` | `SystemSettings:/` |
| `0x8000000000000051` | `FwdbgSettingsS:/` (development mode only on relevant versions) |
| `0x8000000000000052` | `PrivateSettings:/` |
| `0x8000000000000053` | `DeviceSettings:/` |
| `0x8000000000000054` | `ApplnSettings:/` |

These mount names belong to the owning process; they are not globally
accessible POSIX paths in swagent. The table alone does not identify an inner
file containing network profiles.

The precise **container path convention** is `SYSTEM:/save/<lowercase hex ID>`
in a decrypted SYSTEM partition. Thus the established SystemSettings container
is `SYSTEM:/save/8000000000000050`. If the proposed `…0056` save existed, its
conventional container path would be `SYSTEM:/save/8000000000000056`; this is
not a verified WiFi location. SYSTEM is BIS-encrypted; a NAND container path
is distinct from a mounted savedata file path.
[Flash Filesystem](https://switchbrew.org/wiki/Flash_Filesystem#SYSTEM)
documents the partition layout and filename convention.

**Exact WiFi inner file path: unknown from verified sources.** No defensible
`wifi:/<filename>` or `SystemSettings:/<filename> + profile offset` was found.
The settings service exposes `set:sys` commands 1/2, SetNetworkSettings and
GetNetworkSettings, suggesting investigation of settings-managed persistence,
but not proving its physical layout or that `…0050` alone holds every field.
[Settings services](https://switchbrew.org/wiki/Settings_services#set:sys).

For future read-only exploration, libnx supplies
`fsOpenReadOnlySaveDataFileSystem(out, FsSaveDataSpaceId_System, attr)` on
FW 2+. The attribute must identify a confirmed system save, with the correct
type/ID and user fields. The resulting `FsFileSystem` can be read directly or
registered under a caller-chosen devoptab name; that name is not an intrinsic
NAND mount path. `fsOpen_SystemSaveData` is a different helper and should not
be mistaken for a read-only open.
[libnx fs.h](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/include/switch/services/fs.h).
Open/mount permissions and availability while the owner holds the save must
also be tested; no successful concurrent mount is claimed here.

## What format is known

The outer Horizon save container has public documentation: duplicate headers,
integrity structures, journaling and remapping. That describes extraction and
filesystem integrity, not the network-profile payload.
[Savegames](https://switchbrew.org/wiki/Savegames).

The NIFM **wire format** is much better established. The pinned
[libnx header](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/include/switch/services/nifm.h)
and [conversion/IPC implementation](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/source/services/nifm.c)
show:

| Wire structure / field | Offset | Size |
| --- | --- | --- |
| `NifmSfNetworkProfileData` | — | `0x17c` |
| IP settings | `0x000` | `0xc2` |
| UUID | `0x0c2` | `0x10` |
| Profile display name | `0x0d2` | `0x40` |
| Four profile flags/type bytes | `0x112` | `4` |
| Wireless settings | `0x116` | `0x65` |
| Reserved | `0x17b` | `1` |
| Wireless SSID length (relative to wireless settings) | `0x00` | `1` |
| Wireless SSID bytes | `0x01` | `0x20` |
| Three wireless metadata bytes | `0x21` | `3` |
| Passphrase storage | `0x24` | `0x41` |

The SDK-facing/native struct has a different field order and padding. libnx
converts between them and clamps received SSID length to 32. A profile's
display name is not necessarily its SSID. User profile type is a bitmask value
`1`, not enum value zero. Some fields remain named `unk_*` in libnx, so a
zero-filled hand-built write is not established as correct for every network.

Missing for a savedata parser: confirmed save and filename, payload version,
header, record framing/count/stride, active/deleted entry rules, checksums,
and FW 23 compatibility. Searching for printable SSIDs or treating the save
as repeated `0x17c` IPC records would be guessing. No such parser was added.

## NIFM and lower-level services

[Switchbrew's NIFM interface](https://switchbrew.org/wiki/Network_Interface_services)
documents `IGeneralService` commands 7 EnumerateNetworkProfiles, 8
GetNetworkProfile, 9 SetNetworkProfile (admin only), and 14
CreateTemporaryNetworkProfile. `IRequest` command 9 selects a profile UUID;
command 4 submits the connection request. Temporary profiles expose Update
and Persist operations. These interfaces directly contradict a blanket claim
that only Settings UI can configure networking.

libnx implements the following prospective flow without applet initialization:

1. `nifmInitialize(NifmServiceType_Admin)` opens `nifm:a` and creates the general
   service (modern static command 5, reserved u64 zero, caller PID sent).
2. Obtain/construct a valid native profile and call `nifmSetNetworkProfile`.
   libnx converts it and dispatches general command 9 with a fixed-size input
   pointer buffer; it receives a UUID.
3. `nifmCreateRequest` uses requirement preset 2 and obtains event handles.
4. `nifmRequestSetNetworkProfileId` selects the UUID. Submit and observe request
   state/result/events; retain the request for the desired connection lifetime.
5. Handle connection rejection/error without invoking an applet, then close
   request and service objects on teardown.

This flow is inferred from the public interfaces and libnx implementation,
not demonstrated on this console. Association, DHCP, internet verification,
and request acceptance are separate outcomes. A connection error must not
automatically be treated as a demand to open Settings.

swagent's existing `swagent.json` grants wildcard service access and all FS
permission bits, so its manifest does not obviously exclude `nifm:a`; that is
not proof every runtime check will pass. libnx also has process-global NIFM
initialization state, so a future implementation must audit session ownership
and initialization order instead of assuming a second initialize upgrades an
existing user session.

For “lan”: the documented modern wired service is `eth:nd`, not a WiFi profile
manager. The old `wlan:inf` Connect interface was removed in FW 15; modern
`wlan:nd` is a driver-facing interface. Neither provides evidence for a savedata
profile format or a preferable FW 23 profile-management route.
[Ethernet services](https://switchbrew.org/wiki/Ethernet_services),
[WLAN services](https://switchbrew.org/wiki/WLAN_services).

## Community tools and migration leads

Two actual source-verified homebrews read network information through IPC:

- [GetNetworkPass main.c](https://github.com/masagrator/GetNetworkPass/blob/8c82e59bbf063e49c06619e2679b8ab15c97e5ad/source/main.c)
  opens admin NIFM and calls GetCurrentNetworkProfile. It displays the connected
  network's password, using its own adjusted native struct. It neither parses
  the save nor demonstrates joining a new network.
- [ShoWiFi main.cpp](https://github.com/vgmoose/ShoWiFi/blob/126c2890c87822ae2f7725f3be1192b697b8e45e/main.cpp)
  uses system NIFM and GetCurrentNetworkProfile. Its failure text asks the user
  to connect in Settings. It is not a headless join precedent.

Web searches and GitHub repository searches for `nx_wifi`, `wifixtract`,
reimu + wifixtract, “Switch WiFi extractor”, “Switch-Network-Manager”,
“switch-wifi-tool”, and `wificontents` did not establish a Switch profile
extractor/parser or headless-join implementation under those names. Most
similarly named results concern other operating systems. One scoped GitHub
search hit API rate limiting; this is a bounded negative finding, not proof
these projects never existed.

The source archives of
[Atmosphere at 6b0c077](https://github.com/Atmosphere-NX/Atmosphere/tree/6b0c077b4c608a3a2ee712b8dc35fee87ceb62e8)
and [Hekate at 45cd81e](https://github.com/CTCaer/hekate/tree/45cd81ed08e30a26ba380958c7e20b55c342834b)
were inspected for WiFi/NIFM/migration paths and relevant save-ID/network-settings
strings in C/C++ headers, sources and Markdown. No matching profile serializer
or `wificontents` implementation was found. No verified SciresM WiFi migration
research or Atmosphere systransfer profile format was located. General
[migration service research](https://switchbrew.org/wiki/Migration_services)
does not by itself establish an on-disk WiFi format.

## Corruption and recovery risk

| Action / failure | Assessment |
| --- | --- |
| Read through NIFM enumeration | Avoids direct savedata mutation; a future SSID-only command can use basic-info records without requesting passwords. |
| Read-only savedata open | No intended persistent mutation; permissions, busy-save behavior and parsing correctness remain unvalidated. |
| Write malformed inner payload through FS | FS integrity can remain valid while settings/network consumers receive invalid application data. Lost profiles, service errors or crashes are plausible; precise FW 23 behavior is unknown. |
| Patch raw save container | Also risks invalid integrity/journal metadata, causing FS to reject the save before its payload is parsed. |
| Delete save expecting repair | No verified guarantee of automatic recreation or benign defaults for the claimed WiFi save on FW 23. Do not rely on it. |

Does garbage crash the network stack **at boot**? Not established. Does it
automatically rebuild? Not established. Neither a guaranteed brick nor
guaranteed self-repair is supported. libnx explicitly warns that corrupt
SystemSaveData writes can brick the system in its
[system-save helper documentation](https://github.com/switchbrew/libnx/blob/b156b4774fb2df7b93402169509266da7547fc4c/nx/include/switch/services/fs.h).
That is a general warning, not a WiFi-specific FW 23 crash trace. A future
write experiment would require restorable storage and must account for the
owning service's cached state; external edits could be overwritten or leave
inconsistent state (an engineering risk, not a measured result here).

## Delivery and next evidence needed

Only this research document was added. No code, manifest, version, or build
artifact changed; the conditional build was therefore not run. No commit.

To unlock the requested savedata prototype, first obtain a source-backed FW 23
save/file identification and record schema, ideally corroborated by controlled
before/after dumps from a restorable emuMMC. A separate, explicitly scoped
prototype could instead list SSIDs through `nifmEnumerateNetworkProfiles(User)`
today using documented IPC; that would not satisfy this task's requirement to
mount and parse savedata, so it was not substituted.
