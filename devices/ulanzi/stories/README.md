# Ulanzi paid-order scenes

Six scenes run **on the TC002 itself**, at native 52×16 RGB888, a 60-fps output cadence (order-world movement remains deliberately slow):
`city`, `rail`, `orbit`, `vinyl`, `maker`, `garden`. `renderer.js` is the same v5 source as the approved browser preview. QuickJS executes it locally; the host never streams animation frames over Wi-Fi.

The desktop accent recolours structural elements and the counter in every scene.
The secondary colour accents machinery and decorative elements. Saturated, quantized
channels keep the black background unlit and avoid low-channel LED haze; garden
foliage, rain and flower colours retain their meaning. Palette changes preserve the
count, selected scene and brightness, and persist across engine restarts.

Controls:

- **− / +**: previous / next scene. Choice persists on the clock.
- **Ulanzi**: scene / normal ThemeSync clock. A newly paid order temporarily shows the animation, then returns to the clock.
- **Encoder rotation**: brightness, 5–75%; default 35%.
- **Encoder click**: replay an animation, without adding a paid order.
- **Encoder hold**: native runtime menu.

Host CLI: `themesync ulanzi scene rail`, `themesync ulanzi clock`, `themesync ulanzi replay`, `themesync ulanzi sync`.

## Real BaseLinker data

`paid_orders.py` is a read-only bridge. Its only allowed BaseLinker methods are `getOrders`, `getJournalList`, `getOrderPaymentsHistory`. TLS verifies the API connection. Use a read-only BaseLinker token stored in a mode-600 file outside the repository. No customer names, addresses, contacts or product details are stored or sent to the clock.

The day is **Europe/Warsaw**, including DST. Payment history with `show_full_history=true` identifies the first transition from partial/unpaid to fully paid. Partial payments, price reductions and repeated notifications do not add an order. The initial state is applied quietly. Later increases queue complete scene-specific animations. Deduplication survives service/device restarts. At midnight the clock resets independently of host connectivity. The exact number stays correct above 20; worlds retain 20 objects, with an overflow celebration for subsequent orders.

Bootstrap discovers orders from the last 90 days whose creation/current-status change occurred today, plus **all active statuses regardless of order age**. This covers today's incoming orders and older waiting orders that pay today. A historical payment today on a terminal order older than 90 days with no status change today is outside that bootstrap scan. The event journal covers future events once enabled. Enable the event journal in the BaseLinker API settings; activation can take **up to 24 hours**. Until then, payment-history polling remains active. Never interpret an empty journal as zero sales.

Normal polling is every 30 seconds, with six active statuses; the wider 90-day/all-active reconciliation runs every five minutes. API calls are paced at no more than 24/minute for this bridge, and payment histories are cached. Transient API/device errors retry with backoff and preserve the last known count. Only the aggregate `{day,count,revision,asof}` is sent to Ulanzi. New events are not simulated in production.

User service: `themesync-paid-orders.service`, enabled under `default.target`. Configuration: `~/.config/themesync/paid-orders.json`; private state: `~/.local/state/themesync/paid-orders.json`; public compact health: `~/.local/state/themesync/paid-orders-status.json`.

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

- `node test-scene-palettes.cjs`: visible pixel changes in all six scenes for counts 0/12/20, preserved silhouettes and black background, persistence, no low-channel haze or repeated flash writes.
- `node test-device.cjs`: all scenes, controls, baseline without replay, queued burst, dedup, restart, clock return, midnight, counts >20.
- `python3 test-paid-orders.py`: exact decimal totals, full/partial transitions, delayed history retry, cached/restarted dedup, first full-payment date, 23/25-hour DST days, no PII retention.
- 126 static scene states and 4600 object transport states in the shared v5 scene verifier.
- 51 native API parser tests pass, including the new stream opt-in regression; ARM runtime builds.
- All 51 ThemeSync host tests and release Clippy pass.
- All six actual device screen RGB buffers matched the engine buffers exactly, 832/832 pixels each; captures are in the deployment output `live-*-52x16.png`.
- Evdev injection through the real device kernel confirmed −/+, clock toggle, both encoder directions and encoder click replay; the real count remained unchanged and the previous brightness was restored.

Baseline validation of the broader upstream native suite reported 692/694 passing: an unused-byte comparison in the log-message roundtrip fixture fails and the TLS-worker test aborts in Zig's ML-KEM stack. Neither is used by this plaintext local frame path; no claim of a wholly green native suite is made.

Rollback: stop the paid-order service, stop storyd, restore the recorded runtime configuration and previous ThemeSync binary/config. A power cycle without the host restores the stock Ulanzi application. There is no firmware flash to undo.


## Omarchy / Tiny Compo

`demos.js` adds 28 distinct deterministic animations at native 52×16, rendered locally
by QuickJS at a target of **60 fps**. A native C rasterizer samples geometry and
procedural fields on a **4×4 grid per physical pixel**, then reduces coverage to
52×16 RGB888. Subpixel movement, lines, ellipses, metaball contours and the
perspective floor receive actual spatial antialiasing; integer-position pixel
lettering stays sharp. This is not CSS smoothing of an enlarged preview.

The fast trio is **Switchback** (two cars on a compact oval), **Ricochet** (pinball
rebounds with a time-based trail), and **Afterburner** (a ship threading gates at
48 pixels per second). Use the gallery's **Szybkie dema** shortcut and the FPS
selector to compare how their motion reads at 8 and 60 FPS.

The complete named Omarchy palette supplies foreground, selection, red, green,
blue, yellow, orange and magenta as well as accent/secondary. Exact RGB values and
derived shades survive; changing a non-accent colour also synchronizes. The dark
unoccupied background remains off. No audio, network assets or full-screen strobe.

Continuous HTTP frames have an independent 90-Hz admission budget (60-Hz producer
plus scheduling margin); discrete frames retain their 10-Hz limit. Continuous
streams use a linear coverage LUT with the same full-intensity brightness peak as
the legacy curve, avoiding its artificial minimum-channel lift. Clock/discrete
rendering retains the legacy curve. The loop uses absolute 60-Hz deadlines and
reports measured `fps`, `target_fps`, `render_ms`, and `antialias_samples` in `/status`.
`render_ms` includes the local HTTP handoff, not just rasterization.

```
themesync ulanzi demo                 # automatic 28-entry show (~7 min 56 s)
themesync ulanzi demo drive --manual  # loop one animated entry
themesync ulanzi demo --list          # names, IDs, descriptions and durations
themesync ulanzi scene garden        # leave the show for the order world
```

Three short loops use scene colours for water, hot steel and paper, with accents
from the active theme. Open them individually on a device running the updated
`demos.js`:

```sh
themesync ulanzi demo lighthouse --manual  # Night Beacon, 8 s
themesync ulanzi demo forge --manual       # Ember Forge, 6 s
themesync ulanzi demo kites --manual       # Kite Festival, 8 s
```

Two six-second pop-art loops pair bright colour panels with black silhouettes
and white wired earbuds:

```sh
themesync ulanzi demo pocket-groove --manual
themesync ulanzi demo cable-closeup --manual
```

In demo mode, **− / +** choose entries and switch to a single animated entry.
**Encoder click** toggles automatic / single-entry playback; rotation adjusts
brightness. **Ulanzi** cycles demo → order world → clock → demo. Explicit CLI scene
or clock commands leave the compo session. Encoder hold retains the native menu.

The real paid counter continues updating. A new paid order temporarily shows the
selected order-world story, then returns to the demo. The show timer pauses during
that story. Gallery simulation never writes fake orders. Selected mode, manual entry
and autoplay preference survive engine restart. Automatic advances do not write flash.

The C host loads sibling `demos.js` between `renderer.js` and `device.js`; older
installations without it retain order scenes and reject unavailable demo commands.
`ensure_runtime.py` installs all four files. `POST /demo` accepts an optional `id`,
`autoplay` boolean, optional `time` in seconds and `paused` boolean for frame inspection, and `enabled:false` to leave the show; `GET /demos` returns the
28-entry catalogue. The existing authenticated API and fixed-buffer limits apply.

Browser gallery: `devices/ulanzi/compo/index.html` is a self-contained preview with
all 28 entries, seek, play/pause, autoplay, six actual Omarchy palette presets, 8/30/60 FPS comparison, an AA toggle and optional LED grid. Static thumbnails are cached; animated thumbnails run at 10 fps independently of the main 60-fps preview.
It contains no clock/API credentials and does not control the hardware.
Rebuild it with `python3 devices/ulanzi/compo/build.py`.

Export full-length 8-fps GIFs and `contact-sheet.png` (Node + Python/Pillow):

```
cd devices/ulanzi/compo
node export.cjs
python3 render_previews.py
```

`test-compo.cjs` samples the loops at 8 fps, checks distinct motion, bounds, subpixel AA, palette roles and idempotent palette updates, then exercises autoplay without flash writes, selection, mode
cycling, persistence and return after a paid-order alert.


After building the host binary with `build.py --host`, run `node test-native-compo.cjs`.
It compares three frames per entry between the C rasterizer and the browser's
reference implementation. The expected difference is zero RGB channel values.
`raster.h` is compiled into `storyd`; only the usual four runtime files are uploaded.
The restored-state JSON buffer is explicitly terminated before QuickJS parsing.

Hardware verification after deployment: the six heavier entries (voxel, floor,
liquid, cube, silk, windows) measured 59.97–60.04 accepted fps with zero frame
failures; their render-plus-handoff averages were 8.1–11.4 ms. Native and browser
renderers matched all 60 reference frames exactly. A frozen Pacific Drive frame
changed 680 RGB bytes between the real p-bloom and Osaka Jade palettes, with all
11 palette roles read back from the device. Settings and the real paid counter
were restored after checking. FPS telemetry measures generated/accepted frames;
`presented` counts driver transfers and its sampled status is unsuitable for very
short per-scene FPS measurements.

On the TC002, Switchback, Ricochet and Afterburner sustained 59.98–60.04
accepted fps with zero frame failures over six seconds per scene. The native
and browser rasterizers matched all 69 reference frames across the catalogue.
