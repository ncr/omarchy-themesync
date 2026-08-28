# Changelog

Newest first. Protocol history with the reasons is in `protocol/BEACON.md` §5; the two
reviews that shaped v0.1 are `review-2026-08-27.md` (protocol) and
`review-code-2026-08-28.md` (code, release readiness).

## Unreleased (v0.1.0 candidate)

- Pairing starts on the watch: its Pair screen advertises `'T' 0x04 <token>`, every desktop
  in range offers a key + code + its hostname over GATT and shows the code in a
  notification; the person at the watch picks the desktop and enters its code. The watch
  keeps up to four pairings and paints from the active one. `themesync pair` still works
  and now sends the hostname too. The daemon scans even without a key (to see the Pair
  advertisement).

- The watch sends every request (and retransmission) from a fresh non-resolvable private
  address on its own non-connectable advertising instance, so the controller's duplicate
  filter never hides it: the BlueZ `Experimental = true` step is gone, nothing outside the
  home directory needs changing. The daemon reads a new device's data once when BlueZ adds
  it and never records a request's address; the GATT address comes from pairing.

- `themesync install` / `uninstall` / `doctor`; `status --json`; the v1 tooling is hidden
  from `--help`.
- Daemon hardening from the 2026-08-28 review: single instance, socket 0600 and no `/tmp`
  fallback, bounded IPC reads, bounded GATT sessions, register-before-drop for the beacon
  with adapter reopen, wait-with-backoff for bluetoothd / a powered adapter, a clear exit on
  a controller without extended advertising, no scan without a key, the beacon follows a
  watch SET without the hook, key files 0600 from the first byte, an unreadable counter
  locks instead of resetting, the watch's retransmission is silent.
- Protocol: the list COMMIT MAC is signed against a per-transfer nonce from the watch's
  READ status (status v2, 9 bytes); the request MAC is checked before anything else and
  `arg = 0` is enforced for RESEND/LIST; the pending pairing key expires after 120 s on both
  sides; beacon replay is documented as accepted, with a 60 s floor on the watch's NVS
  writes.
- Unit: `WantedBy=graphical-session.target`, `PartOf`, `ConditionEnvironment=WAYLAND_DISPLAY`,
  `Restart=on-failure`; Omarchy's scripts are found through `$OMARCHY_PATH/bin`, not PATH.
- Packaging: `LICENSE`, `packaging/PKGBUILD` + `.install`, CI (Linux + macOS build/test/clippy,
  MSRV build, the C simulator's self-test).

## 2026-08-28 — beacon at 30 ms, no burst

The watch spends the same 4.5 % scan budget as 45 ms every 1 s instead of 120 ms every
2.56 s; the desktop beacons at a constant 30 ms. Desktop → watch measured: mean 0.6 s,
≤ 1.15 s, one missed window in ~21 at −70 dBm (2.03 s). `btmon` confirms BlueZ programs
exactly 30 ms.

## 2026-08-27 — protocol v3

Signed content beacon (HMAC-SHA256/4 under the pairing key) with an echo of the last
accepted request counter; 11-byte requests with a per-key monotonic counter (no nonce, no
seq, no ack record); stop-and-wait retransmission on the watch (1.5 / 3 / 6 s); the
Advertisement Monitor registration so the kernel scans without the controller's duplicate
filter (needs BlueZ `Experimental = true`). Independent review: `review-2026-08-27.md`.

## 2026-08-27 (morning) — the watch names the theme

SET by slug crc16 instead of NEXT/PREV; the watch keeps the whole theme list (pushed over
GATT) and shows prev | current | next itself.

## 2026-08-26 — theme list over GATT, first hardware loop

`push-list`: the installed themes as v2 packets to characteristic `…0006` in BEGIN/DATA/COMMIT
frames sized to the MTU. Pair → code → post-pairing push (22 themes) → swipe → pick → LIST
refresh verified on the OW-Watch. Beacon → watch and watch → desktop verified the same day
with the v1 request format (seq/nonce/ack, since removed). A2DP audio on the same adapter
stayed up throughout.

## 2026-08-25 — themesync

Renamed from omawatch. Omarchy v4.0.1 internals verified (`omarchy.md`): `omarchy-theme-color
--all` as the resolver, `theme-set.d` hooks, `~/.local/state/omarchy/current`.

## 2026-08-24 — omawatch

Theme Protocol v1 (62-byte packet, GATT service `7e450001`), the Rust host, the semantic
14-role palette, the ESP32/LVGL firmware modules and the C simulator. v1 is not on any
device; the OW-Watch speaks its own v2 TLV, which the beacon now carries.
