# omawatch — Omarchy theme → smartwatch, over BLE

```
omarchy theme set Gruvbox
  → omarchy-theme-set swaps ~/.local/state/omarchy/current/theme, retints every app
  → ~/.config/omarchy/hooks/theme-set.d/omawatch   (bash, synchronous, slug in $1)
  → omawatch sync --async → daemon socket (open BLE link) or one-shot BLE
  → Theme Protocol v1 packet (62 bytes: 14 semantic colours + name + crc16)
  → watch: decode → ThemeManager → NVS → lv_obj_report_style_change() → one repaint
  ← watch PREV / MODE / NEXT buttons → Control notification → omarchy-theme-set → (loop)
```

Verified against Omarchy **v4.0.1** (quattro, 2026-08-25). Details: `docs/omarchy.md`.

## Layout

```
host/                    Rust crate `omawatch` (Linux BlueZ / macOS CoreBluetooth via btleplug)
  src/omarchy.rs           source adapter: current theme, omarchy-theme-color --all or a port of it, theme-set
  src/palette.rs           Rgb, SourcePalette (Omarchy vocabulary), WatchPalette (14 roles), map_source()
  src/protocol.rs          Theme Protocol v1 encode/decode, Status/Info/Control, crc16
  src/transport/ble.rs     GATT client (+ a 13-byte "mini" adapter for the first prototype firmware)
  src/transport/ipc.rs     hook → daemon Unix socket
  src/transport/sim.rs     simulated watch receiver
  src/daemon.rs            resident link, serves sync requests, handles watch → desktop requests
  tests/fixtures/*.toml    real Omarchy themes (dark + light) for the tests
protocol/THEME_PROTOCOL.md   the wire format, normative
watch/common/theme_proto.[ch] portable C decoder/encoder (firmware and simulator share it)
watch/esp32-lvgl/            ThemeManager (theme.[ch]) + NimBLE service (theme_gatt.[ch]) for ESP-IDF/LVGL 9
watch/esp32-lvgl/onewheel-integration.patch + INTEGRATION.md   how it plugs into ~/dev/onewheel/watch
watch/sim/                   C simulated watch (`make && ./theme_sim`)
hooks/theme-set.d/omawatch   the Omarchy hook
systemd/omawatch.service     user unit for the daemon
docs/                        omarchy.md (verification), prior-art.md, hardware.md, palette-mapping.md
```

## Try it without hardware

```bash
cd host && cargo build && cargo test                     # 26 tests: resolver parity, mapping, protocol, sim
./target/debug/omawatch demo --file tests/fixtures/tokyo-night.toml
./target/debug/omawatch theme --file tests/fixtures/catppuccin-latte.toml --contrast
./target/debug/omawatch encode --file tests/fixtures/gruvbox.toml          # annotated packet
./target/debug/omawatch decode "$(./target/debug/omawatch encode --hex --file tests/fixtures/nord.toml)"
(cd ../watch/sim && make && ./theme_sim --selftest)
./target/debug/omawatch encode --raw --file tests/fixtures/white.toml | ../watch/sim/theme_sim   # the firmware's decoder
```

On an Omarchy box `--file` is optional: the active theme is resolved through
`omarchy-theme-color --all` (or an equivalent built-in resolver when that script is absent).

## With the watch

```bash
omawatch scan                              # '*' = advertises the Theme service
omawatch sync                              # push the active theme (retries; via the daemon if running)
omawatch status                            # Info (protocol range) + Status (result, crc, mode)
omawatch install-hook                      # ~/.config/omarchy/hooks/theme-set.d/omawatch
systemctl --user enable --now omawatch     # optional: persistent link + watch-initiated changes
omawatch next | prev | toggle              # what the watch's buttons trigger on the desktop
omawatch sync --proto mini                 # the 13-byte format of the first prototype firmware
```

Firmware side: see `watch/esp32-lvgl/INTEGRATION.md`.

## Status (2026-08-25)

* Host: complete; 26 unit tests; `demo` runs the whole chain in-process; the C simulator
  decodes host packets with the exact firmware code.
* Hardware: `omawatch sync --proto mini --file tests/fixtures/gruvbox.toml` pushed a theme to
  the physical OW-Watch from a Mac and read it back byte-identical (against the prototype
  firmware flashed at the time).
* Firmware: the Theme Protocol v1 modules + the integration patch compile and link with
  ESP-IDF 5.5.1 against the current onewheel `main.c`/`ble.c` (scratchpad build); not yet
  flashed, so the v1 GATT flow (Status ack, Control buttons, NVS restore) is untested on
  the device. That is the next step.
