# Watch platforms for "receive a ~60-byte theme packet over a custom GATT service from Linux and recolour the UI"

Surveyed 2026-08-25. "FULL" = you build and flash the entire firmware (UI, RTOS, BLE host
config); "OS" = you own the OS but the radio sits behind a daemon; "APP" = your code runs
in a vendor OS; "FACE" = declarative faces only. The distinction that matters here is
whether *you* can define a GATT peripheral service and repaint the whole UI.

| platform | MCU / RAM / flash | display | control | own GATT service | sensors | dev loop | activity | price / availability | battery |
|---|---|---|---|---|---|---|---|---|---|
| **Waveshare ESP32-S3-Touch-AMOLED-2.06** (owned) | ESP32-S3, 512 KB SRAM + 8 MB PSRAM, 32 MB | 2.06" AMOLED 410×502, CO5300/SH8601 QSPI, FT3168 touch | FULL | yes, NimBLE/Bluedroid, C | QMI8658 IMU, PCF85063 RTC, mics, speaker, AXP2101, µSD; no HR | USB-C, built-in USB-Serial/JTAG → OpenOCD/GDB, ~15 s flash | vendor demos, ESPHome/HA community | ~$30–40, in stock; the only Waveshare AMOLED sold as a wearable with strap | unpublished cell (~400 mAh?); hours screen-on, 1–2 d with light-sleep + BLE |
| LilyGO T-Watch Ultra (2025) | ESP32-S3, 8 MB PSRAM, 16 MB | 2.06" AMOLED 410×502 (same panel class) | FULL | yes | BHI260AP, GNSS, NFC, LoRa, haptics, mic | same | LilyGO examples | $78, **sold out** (LilyGO, Tindie); €110 pre-sale | 1100 mAh, IP65, big |
| LilyGO T-Watch S3 / S3 Plus | ESP32-S3 | 1.54" IPS 240×240 | FULL | yes | BMA423, LoRa, haptics, (GPS) | same | mature | $43–59, sold out at LilyGO; ~$75 resellers | ~1–2 d |
| PineTime / InfiniTime | nRF52832, **64 KB RAM**, 512 KB + 4 MB SPI | 1.3" IPS 240×240 | FULL | yes, NimBLE via `ble_gatt_svc_def` (C++), Wasp-OS (MicroPython), Watchful (Rust) | BMA421, HRS3300 HR | OTA DFU 30–60 s; dev kit has SWD; InfiniSim | 1.16 (2026-01), slow but alive; LVGL **7** fork | **$26.99 sealed / dev kit, in stock** | ~1 week |
| PineTime Pro (announced 2026-03) | dual M33, 800 KB + 8 MB PSRAM, BT 5.2 | 2.13" AMOLED 410×502 | FULL (intended) | intended | HR/SpO2, IMU, GPS, mic/speaker | SWD | rev-3 samples with devs | **not purchasable** | — |
| ZSWatch WatchDK | nRF5340 (dedicated BLE core), 512 KB | 1.28" IPS 240×240 | FULL | yes, Zephyr | BMI270, baro, mag, light, RTC, mic | USB-C SWD+UART | active | $99–119 dev kit only, no wearable | nRF-class |
| Bangle.js 2 | nRF52840, 256 KB | 1.3" 176×176 **3-bit** transflective | FULL possible (SWD pads) / JS by default | yes, `NRF.setServices` (JS), MTU 23 default | accel, mag, baro, HR, GPS | instant (Web IDE) | very active | £100 / $150, in stock | weeks |
| Pebble Core 2 Duo / Time 2 / Round 2 | nRF52840 / SiFli SF32LB52J | B/W or 64-colour memory LCD / colour e-paper | FULL (PebbleOS, NimBLE) | in firmware | IMU, baro, (HR) | waf + OTA | very active | $149–225, shipping | ~30 d |
| AsteroidOS 2.0 | Snapdragon Wear 2100–4100 | 390–454 px AMOLED | OS (Linux, Qt/QML) | BlueZ D-Bus GATT (`asteroid-btsyncd`) | full | ssh/scp, fastboot | active | **secondhand only** (all supported watches discontinued) | ~1–2 d |
| Wear OS | Snapdragon W5 / Exynos | AMOLED | APP / FACE (WFF = XML, no code) | an app can host `BluetoothGattServer`; UI/OS not yours | full | adb | huge, Google-gated | $300–450 | 1–2 d |
| Garmin | proprietary | MIP/AMOLED | APP (Monkey C sandbox) | **no** — CIQ BLE is central-only | full | simulator | active | $200+ | days–weeks |
| Amazfit / Zepp OS | proprietary | AMOLED | APP (JS mini-apps) | **no** GATT server; phone side-service only | full | Zeus CLI | active | $100–300 | 1–2 wk |

Excluded on the display requirement: Watchy, Sensor Watch, Open-SmartWatch (ESP32-PICO,
dormant). Gadgetbridge is a phone bridge and irrelevant for direct PC → watch.

## Verdict for this project

Keep the **Waveshare 2.06 you already own**: it has the best display in the hackable class
(only the unreleased PineTime Pro and the sold-out T-Watch Ultra match it), ESP-IDF 5.5 +
LVGL 9.5 + NimBLE is exactly the right stack, and the native USB-Serial/JTAG gives a real
GDB loop with no probe. Its weaknesses — battery (no published cell, no light-sleep yet),
no HR, no upstream firmware ecosystem — do not matter for a desk-synced theme watch. The
BLE/display coexistence concern (the one-shot scan used to stall the LCD DMA) is a
core-pinning and buffer-placement matter already solved in that firmware: controller +
host on core 1, LVGL on core 0, draw buffers in internal DMA RAM; advertising at 100–150 ms
and a connected link at typical 30–50 ms intervals are far lighter than a scan.

Second choice if a second, always-worn target is wanted: **PineTime dev kit** ($27, in
stock) — NimBLE services are a few dozen lines, but 64 KB RAM, LVGL 7 and a 240×240 LCD.
**Bangle.js 2** is the fastest way to prototype the *desktop* side (JS REPL over BLE), but
its 8-colour screen makes "recolour the UI from a palette" meaningless. AsteroidOS is the
only big-AMOLED-plus-full-Linux option and is stuck on secondhand hardware.
