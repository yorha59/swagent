# Experimental NIFM WiFi (1.14)

Source: https://switchbrew.org/w/index.php?title=Network_Interface_services&oldid=15050
Conversion cross-check: https://github.com/switchbrew/libnx/blob/master/nx/source/services/nifm.c

Wire layouts (hex offsets/sizes):

| Structure | Fields offset:size |
| --- | --- |
| SfNetworkProfileData (17c) | IP 0:c2; UUID c2:10; name d2:40; type 112:1; interface 113:1; auto-connect 114:1; large-capacity 115:1; wireless 116:65; reserved 17b:1 |
| IpSettingData (c2) | IP 0:d; DNS d:9; proxy 16:aa; MTU c0:2 |
| IpAddressSetting (d) | auto 0:1; address 1:4; subnet 5:4; gateway 9:4 |
| DnsSetting (9) | auto 0:1; primary 1:4; secondary 5:4 |
| SfWirelessSettingData (65) | SSID 0:21; nonbroadcast 21:1; authentication 22:1; encryption 23:1; shared-key 24:41 |
| Ssid (21) | length 0:1; bytes 1:20 |
| SharedKey (41) | length 0:1; bytes 1:40 |
| SfNetworkProfileBasicInfo (75) | UUID 0:10; name 10:40; type 50:1; interface 51:1; SSID 52:21; authentication 73:1; encryption 74:1 |

The wireless metadata mappings follow libnx's native-to-wire conversion;
Switchbrew leaves those three packed metadata descriptions blank. No literal
0x22 prefix, quoted payload, or SfSsidList serialization is involved.

## Behavior

`wifi` enumerates all profile types (mask 7) via admin command 7, filters WiFi,
and reports zero-based enumeration indices. Maximum 256 entries; overflow
returns an error rather than silently truncating. No credentials are read.

`wifi join "my ssid" "my password"` accepts quoted arguments with spaces
(no escape syntax), 1–32 SSID bytes and 8–63 printable ASCII passphrase bytes.
The profile uses automatic IP/DNS, zero addresses/proxy/UUID, MTU 1500,
User type, WiFi, auto-connect, WPA2-PSK (6), AES (4). Unused bytes are zero.
SetNetworkProfile returns the UUID; CreateRequest preset 2, event acquisition,
GetRequestState, SetNetworkProfileId and Submit configure the request.
Successful submission retains IRequest. It does not prove association or DHCP.
A later join replaces that request; successful listing retains it.

Installed libnx lacks enumeration and request UUID selection. The shim also
opens private non-domain admin sessions to avoid global libnx NIFM ownership
and implements the sequence directly. No invented GetNetworkProfileList,
CreateNetworkProfile or SetNetworkRequestId command is used.

All acquisition, IPC and cleanup run on one dedicated lane. The worker waits
at most five seconds for NIFM. Each completed IPC logs its step and result;
credentials are never logged. A timeout marks the job abandoned and drops the
retained connection handle. If IPC remains stuck, its thread and static job
storage stay quarantined; subsequent commands return timeout immediately.
When IPC returns, abandonment prevents further steps and closes owned handles.
This bounds worker waiting without unsafe forced thread termination. It does
not cancel an already-issued profile write: a timeout may still have persisted
that profile. Raw handle closure avoids synchronous CMIF close IPC.

## Validation

`make -j8`: passed. Three WiFi host tests: passed (whole-profile fixture,
maximum lengths/rejections, quoted command failure followed by responsive ping).
Artifact checks: passed. Full `make check`: sandbox denied mock HTTP server
bind (EPERM), followed by shared test-lock poisoning; not a clean full-suite
pass. NIFM execution/timeouts require Switch hardware validation. No commit.
