# omarchy-themesync — the Omarchy desktop theme, on your other devices

`omarchy theme set` retints every app on the desktop. This repo carries the same theme one
step further, to hardware next to the desk: a smartwatch today, any other device that can
take a small packet of colours next. The Omarchy side (hook, theme resolver, semantic
palette) and the wire format (Theme Protocol v1) are device-neutral; each receiver adds
its own transport and firmware.

```
omarchy theme set Gruvbox
  → omarchy-theme-set swaps ~/.local/state/omarchy/current/theme, retints every app
  → ~/.config/omarchy/hooks/theme-set.d/*        (bash, synchronous, slug in $1)
  → omarchy-theme-color --all → SourcePalette     (Omarchy's colors.toml vocabulary)
  → map_source() → 14 semantic roles             (background, surface, text_primary, accent, danger, …)
  → Theme Protocol v1 packet                     (62 bytes: 14 colours + mode + name + crc16)
  → device: decode → repaint
  ← device controls → Control notification → omarchy-theme-set → (loop)
```

Verified against Omarchy **v4.0.1** (quattro, 2026-08-25). Details: `docs/omarchy.md`.

## What is shared and what belongs to one device

| layer | where | depends on |
|---|---|---|
| Omarchy hook + source adapter | `hooks/`, `host/src/omarchy.rs` | Omarchy v4.0.x (`theme-set.d`, `omarchy-theme-color`) |
| semantic palette + mapping | `host/src/palette.rs`, `docs/palette-mapping.md` | nothing device-specific; the role list is append-only |
| Theme Protocol v1 | `protocol/THEME_PROTOCOL.md`, `host/src/protocol.rs`, `watch/common/theme_proto.[ch]` | nothing: it knows neither Omarchy nor the receiver |
| GATT transport, CLI | `host/src/transport/ble.rs`, `host/src/main.rs` | the OW-Watch theme service `7a0e0001` |
| beacon (advertising both ways), daemon | `protocol/BEACON.md`, `host/src/beacon.rs`, `host/src/transport/adv.rs`, `host/src/daemon.rs` | BlueZ ≥ 5.5x with extended advertising (Linux) |
| receiver firmware | `watch/esp32-lvgl/` | ESP32-S3 + LVGL 9 + NimBLE (the OW-Watch) |

To add a receiver: implement the packet from `protocol/THEME_PROTOCOL.md`
(`watch/common/theme_proto.c` is a 140-line C decoder/encoder with no dependencies, meant
to be copied). A device that is not a BLE peripheral also needs a transport next to
`host/src/transport/ble.rs`.

## Receivers

### Smartwatch over BLE

The first receiver. Rust host (Linux BlueZ / macOS CoreBluetooth via btleplug), ESP32/LVGL
firmware modules, and a C simulator that runs the firmware's decoder on the desktop.

```
host/                    Rust crate `omarchy-themesync`, binary `themesync`
  src/omarchy.rs           source adapter: current theme, omarchy-theme-color --all or a port of it, theme-set
  src/palette.rs           Rgb, SourcePalette (Omarchy vocabulary), WatchPalette (14 roles), map_source()
  src/protocol.rs          Theme Protocol v1 encode/decode + the watch's v2 TLV codec, crc16
  src/beacon.rs            state beacon / request packets, HMAC, pairing key
  src/themelist.rs         the theme list pushed over GATT (list bytes, BEGIN/DATA/COMMIT frames, status)
  src/transport/ble.rs     GATT client (v2 / 13-byte packets on 7a0e0001; v1 on 7e450001)
  src/transport/adv.rs     BlueZ advertising + request scan from D-Bus signals (bluer)
  src/transport/ipc.rs     hook → daemon Unix socket
  src/transport/sim.rs     simulated watch receiver
  src/daemon.rs            beacon + request scanner + socket for the hook (+ GATT fallback, list push); Linux only
  tests/fixtures/*.toml    real Omarchy themes (dark + light) for the tests
protocol/BEACON.md           connection-less sync over advertising (agreed with the watch side)
protocol/THEME_PROTOCOL.md   Theme Protocol v1 (earlier design, not deployed)
watch/common/theme_proto.[ch] portable C decoder/encoder (firmware and simulator share it)
watch/esp32-lvgl/            ThemeManager (theme.[ch]) + NimBLE service (theme_gatt.[ch]) for ESP-IDF/LVGL 9
watch/esp32-lvgl/onewheel-integration.patch + INTEGRATION.md   how it plugs into ~/dev/onewheel/watch
watch/sim/                   C simulated watch (`make && ./theme_sim`)
hooks/theme-set.d/themesync   the Omarchy hook
systemd/themesync.service     user unit for the daemon
docs/                        omarchy.md (verification), prior-art.md, hardware.md (watch platform survey), palette-mapping.md
```

#### Try it without hardware

```bash
cd host && cargo build && cargo test                     # 40 tests: resolver parity, mapping, protocol, beacon, list, sim
./target/debug/themesync demo --file tests/fixtures/tokyo-night.toml
./target/debug/themesync theme --file tests/fixtures/catppuccin-latte.toml --contrast
./target/debug/themesync encode --file tests/fixtures/gruvbox.toml          # annotated packet
./target/debug/themesync decode "$(./target/debug/themesync encode --hex --file tests/fixtures/nord.toml)"
(cd ../watch/sim && make && ./theme_sim --selftest)
./target/debug/themesync encode --raw --file tests/fixtures/white.toml | ../watch/sim/theme_sim   # the firmware's decoder
```

On an Omarchy box `--file` is optional: the active theme is resolved through
`omarchy-theme-color --all` (or an equivalent built-in resolver when that script is absent).

#### With the watch

```bash
cargo install --path host                   # ~/.cargo/bin/themesync
themesync sync                              # push the active theme: full palette over GATT (--proto v2, default)
themesync install-hook                      # ~/.config/omarchy/hooks/theme-set.d/themesync
themesync daemon                            # state beacon + request scanner (protocol/BEACON.md); the hook then goes through it
systemctl --user enable --now themesync     # the daemon as a user service (systemd/themesync.service); after code changes:
                                            #   cargo install --path host && systemctl --user restart themesync
themesync pair                              # new key over GATT + a 2-digit code you confirm on the watch's Pairing screen
themesync status                            # daemon state (key, counter, last request, last list push)
themesync reset-counter                     # after reflashing the watch (or just `pair` again)
themesync push-list [--force] [--dry-run]   # the installed themes to the watch over GATT (7a0e0006), for its picker
themesync sync --proto mini                 # the 13-byte core-four packet instead
themesync scan / sync --proto v1 / encode / decode / demo   # Theme Protocol v1 tooling (no device speaks it)
```

**Wire formats.** The OW-Watch firmware (`~/dev/onewheel/watch`, `main/theme.h`) serves
service `7a0e0001-…` with a `colors` characteristic that accepts its own role-tagged
**v2 TLV** packet (`[role id][R][G][B]` records, `0x40` name, `0x41` flags; roles 1..15,
append-only) and the legacy **13-byte** packet. `themesync sync` sends v2 with all 14 roles
this host maps (the watch derives `cursor`); the read-back is compared role by role.
Verified 2026-08-26 on the device: 70 bytes out, "15 roles, all sent roles match".

**Two-way, connection-less (`protocol/BEACON.md`, v3 since 2026-08-27).** `themesync daemon`
broadcasts the current theme as an extended advertisement (manufacturer data `0xFFFF`:
`'T' 0x01` + the v2 packet + `0x42` = a 4-byte HMAC over everything before it; ~80 B, fixed
30 ms, constantly) and scans for the watch's requests (`'T' 0x03
ctr op arg mac`, 11 B, HMAC-keyed, a per-key monotonic counter instead of a nonce — the
desktop accepts only `ctr > last accepted`, which covers repeats, BlueZ's cached copies,
daemon restarts and replays in one rule). Ops: SET <slug crc> → `omarchy-theme-set`,
newest wins; an unknown slug means the watch's list is stale and is answered with the list;
RESEND → a ping (the echo is the answer); LIST → the theme list. There is no sequence number and no ack: the watch
applies a beacon whose theme bytes differ from what it shows (after verifying the MAC), and
a request is answered when the beacon shows the theme it asked for. No time in any packet.
Pairing (§2b): `themesync pair` hands the daemon a *pending* key, writes
`[0x01][code][key]` to the watch over GATT and prints the two-digit code; the watch shows a
roller screen, and a request signed with the new key (the watch's RESEND after a correct
code) makes it the active key and resets the counter — a wrong code changes nothing on
either side. **Theme list
(§3):** the installed themes as v2 packets (`[ver][count]` + `[len][packet]`*, slug as the
name, `omarchy-theme-list` order) go to characteristic `…0006` in BEGIN/DATA/COMMIT frames
sized to the negotiated MTU, the COMMIT keyed with the pairing key; the daemon pushes it
right after a pairing completes, when the watch sends a LIST request (op `0x03`), and on
`themesync push-list` (`--dry-run` prints the frames). The watch stores it on its SD card
and shows it as a tappable picker (tap = SET). The firmware side (beacon receive, request sender, key characteristic
`…0005`) is agreed but not built yet; until then the daemon also pushes every change over a
one-shot GATT connection (`--no-gatt` disables that). Theme Protocol v1 (`7e450001-…`) is
the earlier design and is not on any device.

Firmware side of the v1 design: `watch/esp32-lvgl/INTEGRATION.md`.

#### Status (2026-08-26)

* Host: 40 unit tests (v1 protocol, v2 codec, beacon packets/MAC, theme list + frames,
  mapping, resolver parity). Builds on Linux and macOS; the daemon (BlueZ over D-Bus) is
  compiled in on Linux only, the GATT commands (`sync --direct`, `push-list --direct`,
  `pair`) work on both.
* Theme list push (2026-08-26): desktop side done (`themelist.rs`, `push-list`, the daemon
  triggers); the watch firmware (`github.com/ncr/onewheel` `ce199f8`, `8b724c1`: `…0006`,
  op LIST, the SD-backed list, the picker UI) is flashed and the whole loop — pair, code,
  post-pairing list push (22 themes, 1231 B), swipe, pick, LIST refresh — is verified on the
  hardware from a Mac with a Python mock of the daemon; both sides agree on the interop
  vector in `protocol/BEACON.md` §3a. Not yet run end to end from the Linux box — the watch's NVS
  was erased for that work, so `themesync pair` is needed again first (which also exercises
  the post-pairing push). The watch is local-first: it repaints from its list on a swipe or
  pick and then sends the request; the beacon confirms, and wins if the orders differ.
* On this Omarchy box over BlueZ: `omarchy theme set <x>` → hook → `themesync sync --async`
  → daemon socket (or one-shot GATT without a daemon) → watch log
  `theme: set over BLE … name: '<x>'`, about 3 s after the desktop retint. Full v2 palette
  round-trips byte-exact; the daemon's beacon registers with BlueZ (extended advertising)
  and its socket serves `sync`/`status`.
* (History, v1 of the protocol — `seq`, nonces and acks are gone in v3; the independent
  review that drove v3 is `docs/review-2026-08-27.md`.) Beacon → watch verified 2026-08-26 (`themesync daemon --no-gatt`, watch firmware with the
  ext-scan receive path, 120 ms window / 2.56 s): five pushes 30 s apart, watch log
  `beacon: seq 25 applied (69 B, -71 dBm)` … `seq 29`, four real recolours (gruvbox, nord,
  ethereal, gruvbox, ethereal), each seq applied exactly once although every seq is
  broadcast hundreds of times; ~35 KB internal RAM free throughout, no stall. A2DP music on
  the same desktop adapter (WH-CH720N) stayed connected through the whole run.
* Watch → desktop verified the same day, whole loop without a GATT connection: the watch's
  "Themes" screen (then swipe = NEXT/PREV, tap = RESEND; since 2026-08-27 a swipe sends
  SET with the neighbour's slug crc) puts the signed
  10-byte request on its advertisement; daemon log `28:84:…: Toggle nonce 0x24` →
  `Toggle -> omarchy-theme-set flexoki-light` → `beacon: seq 86, theme flexoki-light` →
  watch log `beacon: seq 86 applied` + `request cleared`. Fast gestures out-run
  `omarchy-theme-set` (2–5 s) and a request replaced on the air within well under a second
  can go unseen by BlueZ — a minimum on-air time per request on the watch is the fix.
  Pairing (§2b) verified the same day: `themesync pair` → code on the desktop → rollers on
  the watch → `pairing: confirmed by 28:84:…; key saved`; the pending key is persisted in
  `~/.config/themesync/key.pending` until the watch confirms, so a slow user or a daemon
  restart does not lose it. Two firmware bugs found on the way: a serial-monitor reset wiped
  the RAM-only pending key, and re-configuring the ext-adv instance after a GATT disconnect
  returned `BLE_HS_EBUSY`, silencing every request until reboot.
* BlueZ lessons in `transport/ble.rs`: scan unfiltered (the watch carries the service UUID
  in the scan response), always `stop_scan()` before returning, tolerate a transient error
  while listing peripherals. `bluer` (D-Bus) does the advertising and the request scan;
  `btleplug` does GATT. And in `transport/adv.rs` + `daemon.rs`: BlueZ keeps a device's
  last ManufacturerData cached long after the device stopped advertising it and hands it
  back on every property change (RSSI ticks), so a request is deduplicated per address
  without expiry and whatever is cached at daemon start is marked as already seen — a 60 s
  dedup window produced one phantom request per minute.

### Other devices

None yet. `docs/prior-art.md` lists what already exists for Omarchy → external hardware
(an ESP32 display panel over USB serial, WLED strips, OpenRGB peripherals); none of them
share a wire format, which is the gap Theme Protocol v1 is meant to fill.
