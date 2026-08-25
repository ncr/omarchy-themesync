# Adopting Theme Protocol v1 in the onewheel watch firmware

Target: `~/dev/onewheel/watch` (Waveshare ESP32-S3-Touch-AMOLED-2.06, ESP-IDF 5.5.1,
LVGL 9.5, NimBLE, esp_lvgl_port). Verified to compile and link on 2026-08-25 against
that tree's `main.c` / `ble.c` in a scratchpad copy (app 0x1281c0 bytes); **not flashed**,
because the device was in use by the parallel prototype at the time.

## What goes where

```
watch/common/theme_proto.[ch]   → main/      portable decoder/encoder + CRC (no deps)
watch/esp32-lvgl/theme.[ch]     → main/      ThemeManager: palette, shared lv_style_t roles, NVS
watch/esp32-lvgl/theme_gatt.[ch]→ main/      NimBLE service 7e450001-…, applies packets under lvgl_port_lock
```

`main/CMakeLists.txt`: add `"theme_proto.c" "theme_gatt.c"` to `SRCS` (theme.c is there already
in the current tree, under the same name — this module replaces the prototype's `theme.[ch]`).

## Wiring (what `onewheel-integration.patch` does)

*ble.c*: drop the prototype's 7a0e0001 service/characteristics; in `ble_init()` call
`theme_gatt_register("OW-Watch")` after `nimble_port_init()` and before
`nimble_port_freertos_init()` (NimBLE cannot add services once the host runs);
`start_advertising()` becomes `theme_gatt_advertise(s_own_addr_type)` and
`ble_link_connected()` becomes `theme_gatt_connected()`. theme_gatt re-advertises on
disconnect by itself.

*main.c*: the prototype's `st_*` styles become aliases onto `theme_style(THEME_STYLE_*)`
(so `lv_obj_add_style(x, &st_card, 0)` keeps working); `styles_init()` /
`styles_apply_theme()` / `theme_take_dirty()` go away — `theme_apply()` rewrites the shared
styles and calls `lv_obj_report_style_change(NULL)` itself; `theme_init()` moves to just
before `build_ui()` (inside the `bsp_display_lock`), because it touches LVGL styles; a
THEME card gains PREV / MODE / NEXT buttons that call `theme_gatt_send_control()`.

Threading: the GATT write callback runs on the NimBLE host task (core 1). It takes
`lvgl_port_lock(200)`, applies, persists to NVS, unlocks, then `ble_gatts_chr_updated()`
notifies Status. The LVGL task (core 0) sees the new style values on its next refresh; one
full repaint (~21 ms measured for this UI) is the entire cost of a theme change.

RAM: theme.c holds 24 `lv_style_t` (~50 B each) + a 240-byte packet copy; theme_gatt.c
holds nothing beyond handles. Advertising at 100–150 ms with the controller pinned to
core 1 is far lighter than the boot scan that used to stall the display; if a stutter ever
shows up in the render meter, `ble_shutdown()` remains the escape hatch.

## Rules for UI code from now on

Never `lv_obj_set_style_*_color(obj, lv_color_hex(...), ...)`. Add a role style:

```c
lv_obj_add_style(card,  theme_style(THEME_STYLE_CARD), 0);
lv_obj_add_style(title, theme_style(THEME_STYLE_TEXT_SECONDARY), 0);
lv_obj_add_style(bar,   theme_style(THEME_STYLE_WELL), 0);
lv_obj_add_style(bar,   theme_style(THEME_STYLE_INDICATOR_DANGER), LV_PART_INDICATOR);
```

For the rare custom draw, read `THEME_COLOR(THEME_ACCENT)` at draw time, never cache it.
Need a new role (say `on_warning` on the wire instead of derived)? Append it to
`enum theme_role` in theme_proto.h, to `Role` in host/src/palette.rs, and to the mapping —
the wire format and every existing receiver stay valid.

## Flash and try

```bash
source ~/dev/esp/v5.5.1/esp-idf/export.sh
cd ~/dev/onewheel/watch && idf.py build && idf.py -p /dev/cu.usbmodem101 -b 460800 flash
python tools/monitor.py 20            # expect: theme: no stored theme / restored '...'; theme_gatt: advertising as 'OW-Watch'

cd ~/dev/omarchy-themesync/host
cargo run -- scan                      # '*' next to OW-Watch = advertising 7e450001
cargo run -- sync --file tests/fixtures/catppuccin-latte.toml   # from any machine with BLE
cargo run -- status                    # info v1..v1, 14 slots; status Ok, crc, mode
```

On the Omarchy box: `omawatch install-hook`, optionally `systemctl --user enable --now omawatch`
(daemon: persistent link + PREV/MODE/NEXT from the watch), then `omarchy theme set Gruvbox`.
