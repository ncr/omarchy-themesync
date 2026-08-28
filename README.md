# omarchy-themesync

The [Omarchy](https://omarchy.org) desktop theme on your smartwatch, both ways. Pick a theme
on the desktop and the watch repaints within a second; swipe to a theme on the watch and
the desktop switches to it. No connection is kept: the desktop broadcasts its theme as a
signed Bluetooth advertisement and listens for the watch's 11-byte requests the same way.

```
desktop                                   watch
  omarchy theme set nord ──hook──▶ daemon ──beacon 30 ms──▶ applies nord (≤ 1 s)
  omarchy-theme-set gruvbox ◀── daemon ◀──request (SET gruvbox, signed, counted)── swipe
```

Status: works on the author's desk (Omarchy 4.0.x, an Intel BLE 5 controller, the OW-Watch
firmware). Not on the AUR yet. If you have the watch, `themesync install` and
`themesync pair` is all there is; if you have another BLE device, `protocol/BEACON.md` is
what to implement.

## Requirements

| what | why |
|---|---|
| Omarchy ≥ 4.0 | `omarchy-theme-set`, `omarchy-theme-color`, the `theme-set.d` hooks |
| BlueZ ≥ 5.6x, stock configuration | advertising and scanning over D-Bus. Nothing in `/etc/bluetooth` needs changing. |
| a **BLE 5** controller (extended advertising) | the beacon is ~80 bytes; legacy advertising carries 31. Most laptops since 2019 and any USB BLE 5 dongle qualify; `themesync doctor` tells you. |
| the watch: OW-Watch firmware from `github.com/ncr/onewheel` (`watch/`), with list status v2 | the other end of `protocol/BEACON.md` v3 |
| a desktop session (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`) | the daemon's socket lives in the runtime dir and the unit is tied to the graphical session |

The daemon runs as your user, needs no privileges, and changes nothing outside your home
directory.

## Install

From source (Rust stable ≥ 1.85):

```bash
git clone https://github.com/ncr/omarchy-themesync && cd omarchy-themesync
cargo install --path host --locked          # → ~/.cargo/bin/themesync
themesync install                           # hook + user service, then a health check
```

As a package: `packaging/PKGBUILD` (`cd packaging && makepkg -si`); the package puts the
binary in `/usr/bin` and the unit in `/usr/lib/systemd/user`, and its post-install text
lists the same per-user steps: `themesync install`, then `themesync pair`.

`themesync install` writes `~/.config/omarchy/hooks/theme-set.d/themesync` (the Omarchy
hook that tells the daemon about a theme change) and `~/.config/systemd/user/themesync.service`
with this binary's path, runs `systemctl --user enable --now themesync`, and prints the
`doctor` report.

## Pair

Open **Pair** on the watch. Every desktop in range running the daemon answers with an
offer: a notification shows a two-digit code and the desktop's name. On the watch pick the
name, enter that code, confirm. The watch answers with a request signed with the new key,
the daemon makes it the active key and pushes the theme list. A wrong code, or picking
another desktop, changes nothing here.

The same from the desktop side, without touching the watch's menu:

```bash
themesync pair
```

The watch keeps up to four desktops and paints from the *active* one (its Computers
screen: tap to switch, hold to forget). Re-pair after reflashing the watch (it resets the
request counter on both ends).

## Verify

```bash
themesync status          # daemon, beacon, scan, watch, counter, last list push
themesync doctor          # Omarchy, BlueZ, controller, unit, hook, key — with the fix for each problem
journalctl --user -u themesync -f
```

A healthy `status`:

```
daemon:   running, paired (beacon v3, list status v2)
beacon:   on the air, theme nord
scan:     on
watch:    28:84:85:B4:FB:9A, last request Set #1110 from 28:84:85:B4:FB:9A
counter:  last accepted #1110
list:     22 themes, 1583 B, crc 0xab60: pushed in 9 writes ... [pairing]
```

Then `omarchy theme set <x>` (or the picker) should repaint the watch within a second, and
a swipe on the watch's Themes screen should switch the desktop (2–5 s: that is
`omarchy-theme-set` retinting every app).

## Troubleshoot

Every one of these is in `themesync doctor`; the journal line is what you will see.

| symptom | journal / doctor line | fix |
|---|---|---|
| nothing happens at all | `no pairing key` / doctor `key` | `themesync pair` |
| desktop changes never reach the watch | doctor `hook: no hook` | `themesync install` |
| the daemon exits at start | `has no extended advertising (BLE 5)` | a BLE 5 controller; the codecs and GATT commands still work |
| `beacon: OFF THE AIR` | BlueZ refused the advertisement twice and a reopen did not help | `journalctl --user -u themesync -n 30`; `bluetoothctl power off; power on` |
| `rejected: last accepted is #N — a reflashed watch counts from 1 again` | the watch was erased | `themesync pair` (or `themesync reset-counter` if you trust the watch) |
| `counter file unreadable` / status `LOCKED` | `~/.config/themesync/ctr` was edited or truncated | `themesync reset-counter` |
| `another themesync daemon is already listening` | you started `themesync daemon` next to the service | use one: `systemctl --user stop themesync` for hand-runs |
| the service is inactive after login | `ConditionEnvironment=WAYLAND_DISPLAY` not met, or Bluetooth off | it starts with the graphical session; the daemon waits for a powered adapter (`bluetoothctl power on`) |

Privacy and radio: the daemon broadcasts the theme name from the adapter's static address
every 30 ms whenever it runs, and (while paired) keeps the adapter in active LE scanning.
Anyone in range can see which theme you use; nobody without the key can change it or make
the watch accept a theme. The watch's requests come from a fresh random address each time,
so its swipes cannot be tracked. Stop the service when that is not what you want.

## Uninstall

```bash
themesync uninstall          # stops and disables the service, removes the unit and the hook
themesync uninstall --purge  # also deletes ~/.config/themesync (the pairing key)
cargo uninstall omarchy-themesync   # or pacman -R omarchy-themesync
```

## How it works

`protocol/BEACON.md` is the whole design; the short version:

- **Beacon** (desktop → watch): manufacturer data `0xFFFF`, `'T' 0x01` + the theme as the
  watch's own v2 TLV packet + an echo of the last accepted request counter + a 4-byte
  HMAC-SHA256 under the pairing key. Extended advertising, 30 ms, constantly. The watch
  scans a 45 ms window every second (4.5 % of its radio time) and applies any beacon whose
  theme bytes differ from what it shows.
- **Request** (watch → desktop): `'T' 0x03` + counter + op (SET slug-crc / RESEND / LIST) +
  arg + HMAC, 11 bytes, in the watch's own advertisement. The desktop accepts only a
  counter greater than the last one it accepted — that one rule covers repeats, replays,
  stale BlueZ cache entries and daemon restarts. The watch retransmits (1.5 / 3 / 6 s)
  until the beacon's echo reaches its counter: stop-and-wait, like CoAP CON or MQTT QoS 1.
- **Theme list** (desktop → watch, GATT): the installed themes as v2 packets so the watch
  can show a picker; pushed after pairing, on the watch's LIST request, and by
  `themesync push-list`. The COMMIT is signed against a nonce from the watch, so a recorded
  transfer cannot be replayed.
- **Pairing** (GATT, once): key + two-digit code; the code is a confirmation, not key
  material. `docs/review-2026-08-27.md` and `docs/review-code-2026-08-28.md` are the two
  independent reviews behind the current shape.

## Repository

```
host/                        Rust crate `omarchy-themesync`, binary `themesync`
  src/daemon.rs                the daemon (beacon + request scan + socket + list push); Linux only
  src/beacon.rs                beacon/request packets, HMAC, the files in ~/.config/themesync
  src/themelist.rs             the theme list: bytes, BEGIN/DATA/COMMIT frames, status with nonce
  src/protocol.rs              the watch's v2 TLV codec (+ Theme Protocol v1, historical)
  src/palette.rs               Rgb, SourcePalette (Omarchy vocabulary), WatchPalette (14 roles), map_source()
  src/omarchy.rs               Omarchy adapter: current theme, omarchy-theme-color / theme-set, hooks
  src/setup.rs                 install / uninstall / doctor
  src/transport/adv.rs         BlueZ advertising + request scan (bluer, D-Bus)
  src/transport/ble.rs         GATT client (btleplug): pair, list push, one-shot theme push
  src/transport/ipc.rs         the daemon's Unix socket (JSON lines)
  tests/fixtures/*.toml        real Omarchy themes for the tests
protocol/BEACON.md           the protocol (v3) — normative
protocol/THEME_PROTOCOL.md   Theme Protocol v1 (historical: not on any device)
watch/                       v1-era C reference (decoder, ESP32/LVGL modules, simulator) — historical;
                             the real firmware lives in github.com/ncr/onewheel
hooks/theme-set.d/themesync  the Omarchy hook (what `themesync install` writes, with an explicit binary search)
systemd/themesync.service    the packaged user unit
packaging/                   PKGBUILD + .install
docs/                        changelog, the two reviews, omarchy.md (Omarchy internals verified), palette-mapping.md, prior-art.md, hardware.md
```

## Development

```bash
cd host && cargo build && cargo test           # 45 tests: codecs, MAC, vectors, list, mapping, resolver parity, sim
cargo run -- theme --file tests/fixtures/tokyo-night.toml --contrast
cargo run -- push-list --dry-run               # the list and its frames, without a watch
cargo install --path . --locked && systemctl --user restart themesync
```

The daemon (BlueZ over D-Bus) compiles on Linux only; the GATT commands (`pair`,
`push-list --direct`, `sync --direct`) and the codecs also build and run on macOS. Interop
vectors for a second implementation are in `protocol/BEACON.md` §4. The watch firmware is
coordinated in `github.com/ncr/onewheel`.

MIT — see `LICENSE`.
