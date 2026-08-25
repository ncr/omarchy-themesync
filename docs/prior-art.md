# Prior art (searched 2026-08-25)

**Verdict: no public Omarchy → smartwatch theme synchronisation exists**, and nothing
pairs a Linux desktop palette (Omarchy, Hyprland, pywal, wallust, matugen) with any watch
platform (PineTime/InfiniTime, Bangle.js, Wear OS, Garmin, Amazfit/Zepp, AsteroidOS,
Pebble, ESP32/T-Watch) in a live-sync role.

Coverage: GitHub repo/code/issue/topic search (authenticated, so code search worked) for
every combination requested — `omarchy` × smartwatch/watch/watchface/BLE/Wear OS/PineTime/
InfiniTime/Bangle/AsteroidOS/Garmin/Amazfit/Gadgetbridge/ESP32/T-Watch/theme sync;
`pywal`/`wallust`/`matugen`/`catppuccin`/`gruvbox`/`tokyo night` × watch/watchface; the five
`awesome-omarchy` lists and the HANCORE plugin registry (~150 plugins); ~60 web searches
incl. Reddit-scoped ones; HN Algolia; Lemmy. The `omarchy watch` repo hits are all
"watcher"/camera/"Watchmen theme"; `omarchy bluetooth` hits are headphones/desks/lamps.

## Closest things that exist

| project | class | what it is |
|---|---|---|
| [iainfreestone/omarchy-cyd-panel](https://github.com/iainfreestone/omarchy-cyd-panel) | Omarchy → external hardware (not a watch) | ESP32 "Cheap Yellow Display" that follows the Omarchy theme *and font*: host Python reads `colors.toml`, watches the theme files, rasterises text with the real font, streams draw commands over **USB serial**; the ESP32 only blits. Nearest analogue of the whole idea. |
| [ericdahl-dev/omarchy-wled](https://github.com/ericdahl-dev/omarchy-wled) | Omarchy → LED strip | Go daemon sends accent/foreground/wallpaper-average colour to WLED over HTTP. |
| [vonsensey/omargb](https://github.com/vonsensey/omargb), [perfektnacht/openrgb-theme-plugin](https://github.com/perfektnacht/openrgb-theme-plugin), [didlix/omarchy-openrgb](https://github.com/didlix/omarchy-openrgb) | Omarchy → RGB peripherals | theme-set hook → `omarchy-theme-color --all` → semantic roles → OpenRGB SDK. Same hook pattern this project uses. |
| [n1byn1kt/omarchy-garmin](https://github.com/n1byn1kt/omarchy-garmin) | watch → desktop, read-only | Garmin Body Battery/steps in the Omarchy bar via the Garmin Connect web API. Pushes nothing. |
| [akselmo Aksdark watchface](https://codeberg.org/akselmo/Aksdark-Watchface), [MorsMortium/GTKWatchFace](https://codeberg.org/MorsMortium/GTKWatchFace) | static themed watchface | InfiniTime faces coloured after a desktop theme *at compile time* (GTKWatchFace generates C++ from the current GTK theme). No runtime sync. |
| Facer "simple gruvbox", fitbit-watchface-gruvbox, mocha64-tactical (Garmin, Catppuccin) | static themed watchface | hard-coded palettes. |
| InfiniTime PineTimeStyle colour picker; Bangle.js Theme app; Wear OS WFF `ColorConfiguration`; Garmin CIQ settings | manual selection on the watch/phone | no external API. |
| Nextface (iOS) | wallpaper sync phone ↔ Apple Watch | not Linux, not palette. |
| imbypass/omarchy-theme-hook, OldJobobo/thpm, Keyrxng/Omarchy-IDE-Theme-Sync, beaterblank/omarchy-theme-sync, omarchy-firefox-theme, omarchy-vesktop-theme-sync | desktop-only bridges | the `theme-set.d` plugin convention. |
| jrodal98/watch-scripts (Wear OS → Linux via KDE Connect + Tasker), ziehmon/banglecli (archived) | generic PC ↔ watch plumbing | no theming. |
| Wear OS 4+ Material You "match watch face" | dynamic colour, phone-side | seeded from the watch face; Google declined desktop/wallpaper seeding. |

## Building blocks worth knowing

*InfiniTime* has no BLE colour characteristic, but watchface colours are plain fields in
`SettingsData` (`/settings.dat`, versioned, loaded at boot), and the Adafruit-style BLE FS
service (`adaf0100-…`) can write arbitrary files; `itd`/`itctl` and WatchMate already speak
it from Linux. A desktop hook could rewrite `settings.dat` (nearest of 18 colours) + reboot,
or a custom face could read a pushed palette file. *Bangle.js* keeps `g.theme =
{fg,bg,fg2,bg2,fgH,bgH,dark}` in `setting.json`; the watch is a JS REPL over Nordic UART,
so `espruino -d Bangle.js -e '...'` from Linux sets a theme in one line (apps reload to pick
it up). *Wear OS*: an app can host a `BluetoothGattServer`, but the UI/OS are not yours.
*Garmin* Connect IQ BLE is central-only; *Zepp OS* has no GATT server; both are out.
*AsteroidOS*: BlueZ D-Bus GATT via `asteroid-btsyncd`, Qt/QML — full control, but only on
discontinued Wear OS hardware.
