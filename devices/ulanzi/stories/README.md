# Ulanzi paid-order scenes

Five scenes of **today's paid orders** run **on the TC002 itself**, at native 52×16 RGB888 and 60 fps:

| id | name | what it shows |
|---|---|---|
| `hours` | Liczydło | 24 hour rods; each order is a bead in the hour it was paid, coloured by value |
| `sky` | Gwiazdozbiór | each order a star: across = time of day, up = value; lines join them in order |
| `jar` | Słoik | marbles fall and settle into a mound (simulated); size = value, colour = time of day |
| `board` | Tablica | split-flap count and the times of the two latest orders |
| `glass` | Witraż | weighted-Voronoi stained glass; a bigger order gets a bigger pane, sunlight follows the time of day |

Each new order plays an arrival (falling bead, comet, dropping marble, flapping tiles, a pane growing in). They are designed for up to about 20 orders a day; above that they stay correct and simply get denser. The scenes see only the local time each order was paid and its value in whole złoty. Colours come from the desktop theme: the accent and secondary colour first, then the next theme colour with a clearly different hue, pushed to full LED brightness. `renderer.js` renders the same frames in Node (tests, preview) as QuickJS on the clock, through the shared 4×4 antialiasing rasterizer.

Browser preview: `devices/ulanzi/orders/index.html` plays all five on a simulated day (order count, time of day, theme, a "new order" button). Rebuild it with `python3 devices/ulanzi/orders/build.py`.

Controls:

- **− / +**: previous / next scene. Choice persists on the clock.
- **Ulanzi**: scene / normal ThemeSync clock. A newly paid order temporarily shows the animation, then returns to the clock.
- **Encoder rotation**: brightness, 5–75%; default 35%.
- **Encoder click**: replay an animation, without adding a paid order.
- **Encoder hold**: native runtime menu.

Host CLI: `themesync ulanzi scene rail`, `themesync ulanzi clock`, `themesync ulanzi replay`, `themesync ulanzi sync`.

## Real BaseLinker data

`paid_orders.py` is a read-only bridge. Its only allowed BaseLinker methods are `getOrders`, `getJournalList`, `getOrderPaymentsHistory`. TLS verifies the API connection. Use a read-only BaseLinker token stored in a mode-600 file outside the repository. No customer names, addresses, contacts or product details are stored or sent to the clock.

The day is **Europe/Warsaw**, including DST. Payment history with `show_full_history=true` identifies the first transition from partial/unpaid to fully paid. Partial payments, price reductions and repeated notifications do not add an order. The initial state is applied quietly. Later increases queue complete scene-specific animations. Deduplication survives service/device restarts. At midnight the clock resets independently of host connectivity. The exact number stays correct at any count.

Bootstrap discovers orders from the last 90 days whose creation/current-status change occurred today, plus **all active statuses regardless of order age**. This covers today's incoming orders and older waiting orders that pay today. A historical payment today on a terminal order older than 90 days with no status change today is outside that bootstrap scan. The event journal covers future events once enabled. Enable the event journal in the BaseLinker API settings; activation can take **up to 24 hours**. Until then, payment-history polling remains active. Never interpret an empty journal as zero sales.

Normal polling is every 30 seconds, with six active statuses; the wider 90-day/all-active reconciliation runs every five minutes. API calls are paced at no more than 24/minute for this bridge, and payment histories are cached. Transient API/device errors retry with backoff and preserve the last known count. Only `{day,count,orders,revision,asof}` is sent to Ulanzi, where `orders` is `[seconds since local midnight, whole złoty]` per order, oldest first, at most the latest 100. A bridge that sends only the count still works: the clock places new orders at the moment they arrived. New events are not simulated in production.

Install or update with `python3 devices/ulanzi/install.py [--device]`: it runs the tests, copies the bridge and clock files to `~/.local/share/themesync/ulanzi/` (also the recovery bundle), points the config and the user unit there and restarts the bridge; `--device` also uploads to the clock. Services never run from a working copy. User service: `themesync-paid-orders.service`, enabled under `default.target`. Configuration: `~/.config/themesync/paid-orders.json`; private state: `~/.local/state/themesync/paid-orders.json`; public compact health: `~/.local/state/themesync/paid-orders-status.json`.

`ensure_runtime.py` reconnects through the existing authorized ADB connection and reloads the tested RAM runtime after a power cycle. Animation/buttons continue without the PC as long as the running clock has power; **fresh BaseLinker updates and RAM recovery after power cycling require this host to be running**. Firmware/boot/init partitions remain untouched. Scripts, choice and daily count live under `/data/tc002/stories`; runtime binaries execute under `/tmp/tc002`.

## Runtime patch

Apply `runtime.patch` to upstream `atomicstack/tc002-customisation` commit
`ed902b8c949c`, then build with its pinned Zig toolchain. The patch also includes the
Berry stack regression fix, uploaded sprite forwarding and the legacy Quiet Garden.
The runtime's original license continues to apply.

## Runtime extension

`POST /api/v1/frame?duration_s=3&transition=cut&stream=1` uses the existing replacement-frame IPC, with the configured deadman timeout. It requires the existing display scope, shares the ten-fps limit, bypasses the 128-discrete-command dedup cache, and returns `{"status":"queued"}`. Normal `/frame` acknowledgement/idempotency is unchanged. The supervisor supplies current epoch/timeout and forwards to the existing renderer. Without this option, an 8-fps sender fills the old 60-second dedup window in 16 seconds.

The local story API binds port **8878**, requires the clock's control Bearer token and allows no browser CORS. Only bounded JSON (8 KiB) with fixed routes is accepted. It exposes GET `/status`, `/frame`, PUT `/state`, `/palette`, POST `/scene`, `/clock`, `/input`, `/replay`. It has no code-execution/upload endpoint. Selected scene and aggregate count are saved atomically; frame generation never writes flash. A singleton lock prevents duplicate engines.

## Build and verification

QuickJS official source: https://bellard.org/quickjs/quickjs-2026-06-04.tar.xz
SHA256: `b376e839b322978313d929fd20663b11ba58b75df5a46c126dd19ea2fa70ad2a` (MIT; see upstream LICENSE).

Build: `python3 build.py --quickjs /path/to/quickjs-2026-06-04 --zig /path/to/zig`; ARM Cortex-A7 musl static binary, about 1 MiB. `--host` builds a desktop test engine. QuickJS heap limit 3 MiB, stack 512 KiB. Explicit 16-byte vendor evdev records preserve the device's 32-bit kernel ABI even with musl time64.

Verified:

- `node test-orders.cjs`: all five scenes for 0/1/5/20/100 orders: deterministic frames, theme changes the picture, every arrival animates, idle never freezes.
- `node test-device.cjs`: scenes, controls, queued burst, dedup, restart, clock return, midnight, order validation, count-only bridge, the 100-order state within storyd's 8 KiB.
- `python3 test-paid-orders.py`: exact decimal totals, full/partial transitions, delayed history retry, cached/restarted dedup, first full-payment date, 23/25-hour DST days, no PII retention.
- 126 static scene states and 4600 object transport states in the shared v5 scene verifier.
- 51 native API parser tests pass, including the new stream opt-in regression; ARM runtime builds.
- All 51 ThemeSync host tests and release Clippy pass.
- On the TC002 (2026-11-05) all five scenes ran at 60 fps with zero frame failures; arrivals were checked through the desk camera, including a real order that arrived over the count-only bridge.
- Evdev injection through the real device kernel confirmed −/+, clock toggle, both encoder directions and encoder click replay; the real count remained unchanged and the previous brightness was restored.

Baseline validation of the broader upstream native suite reported 692/694 passing: an unused-byte comparison in the log-message roundtrip fixture fails and the TLS-worker test aborts in Zig's ML-KEM stack. Neither is used by this plaintext local frame path; no claim of a wholly green native suite is made.

Rollback: stop the paid-order service, stop storyd, restore the recorded runtime configuration and previous ThemeSync binary/config. A power cycle without the host restores the stock Ulanzi application. There is no firmware flash to undo.


## Compo

`demos.js` holds eight seamless loops at native 52×16 and 60 fps, drawn with the order scenes' rasterizer and theme palette (`renderer.js` exports both as `UlanziPixels`, and storyd loads it first):

| id | loop | length |
|---|---|---|
| `pendulums` | thirteen pendulums with 18…30 swings per loop: waves, chaos, one line again | 30 s |
| `fireflies` | fireflies over grass slowly synchronise their flashes, hold, and drift apart | 24 s |
| `harmonograph` | a decaying 3:2 harmonograph draws, turns and fades | 24 s |
| `ripples` | raindrops and thin rings spreading over dark water | 20 s |
| `aurora` | aurora curtains with rays above a mountain ridge | 24 s |
| `bloom` | three flowers open and close in one breath | 20 s |
| `lace` | Truchet lace: tiles turn one by one and the pattern flows on | 24 s |
| `tide` | the moon, a silver path on the waves and a passing cloud | 24 s |

Each loop is a pure function of time and ends exactly where it began. Colours: the theme's accent and secondary first, the warmest theme hue for light sources (fireflies, flower hearts), the foreground for highlights. Faint details stay very dim on purpose: on the LEDs low values look much brighter than on a screen.

```
themesync ulanzi demo                  # automatic show
themesync ulanzi demo lace --manual    # hold one loop
themesync ulanzi demo --list
```

In demo mode **− / +** choose a loop, **encoder click** toggles the automatic show, rotation sets brightness, **Ulanzi** cycles demo → order scenes → clock → demo. A new paid order shows its arrival in the selected order scene, then the show resumes. `POST /demo` accepts `id`, `autoplay`, `time` and `paused`; `GET /demos` returns the catalogue.

On the TC002 (2026-11-05) all eight ran at 59.4–60 fps with zero frame failures; render plus handoff 8–15 ms. `node test-compo.cjs` checks seamless loops, motion, theme response, autoplay without flash writes, controls, persistence and the return after a paid order. Gallery: `devices/ulanzi/compo/index.html`, rebuilt with `python3 devices/ulanzi/compo/build.py`.
