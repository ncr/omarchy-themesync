# Theme beacon — connection-less theme sync over BLE advertising

Draft 2026-08-26, reviewed by the watch side the same day. Between `omarchy-themesync`
(desktop) and the OW-Watch firmware (`~/dev/onewheel/watch`). §1, §2 and §2b are verified on
the device; §3 (the theme list) is implemented on the desktop and being built on the watch.

Two independent broadcasts, no GATT connection needed for day-to-day use:

```
desktop  ──(state beacon: the current theme, repeated forever)──▶  any receiver
watch    ──(request: NEXT / PREV / SET / RESEND / LIST, repeated ≤10 s)──▶ desktop
                                    ▲                                   │
                                    └── the desktop applies it, its beacon changes,
                                        the watch sees the change and stops asking
```

The state beacon is the acknowledgement of a request. Both packets are Manufacturer
Specific Data (AD type `0xFF`), company id `0xFFFF` (the "reserved for testing" id; the
magic byte disambiguates from other 0xFFFF users). All multi-byte integers little-endian.

## 1. State beacon (desktop → receivers)

Extended advertising (BLE 5, primary 1M, secondary 1M), non-connectable, non-scannable.
Verified 2026-08-26 that BlueZ 5.87 on the Intel BT 5.4 adapter registers such an
advertisement with 77 bytes of manufacturer data. Any receiver that cannot do extended
scanning gets nothing; that is accepted (the watch can: `CONFIG_BT_NIMBLE_50_FEATURE_SUPPORT`).

```
off  size  field
 0    1    magic     0x54 'T'
 1    1    kind      0x01 = theme state
 2    1    seq       increments on every desktop theme change, wraps; receivers
                     re-apply only when it differs from the last one they applied
 3    1    host      crc8 of the desktop's hostname; lets a watch follow one desktop
                     when two are in range (0x00 = "any", receivers may ignore)
 4    n    theme     the OW-Watch `colors` packet, verbatim: v2 TLV
                     ([2][role r g b]*[0x40 len name][0x41 1 flags]) — same bytes the
                     desktop would write to characteristic 7a0e0002; the receiver hands
                     them to theme_set_wire() unchanged — followed by two beacon-only
                     meta records the theme parser skips:
                       0x42 previous theme, 0x43 next theme (omarchy-theme-list order,
                       wrapping): [len][bg r g b][fg r g b][accent r g b][alarm r g b]
                       [name UTF-8 ≤ 20 B]; len = 12 + name length.
                     ~150 bytes with neighbours.
```

The neighbours exist so the watch can show "prev | current | next" the way Omarchy's own
switcher does; after a NEXT/PREV the returning beacon carries the new current theme and
its new neighbours, so the screen re-renders from the beacon alone.

Timing: 100 ms interval steady state (the desktop is mains powered); for 10 s after a
change, 30 ms ("burst") so a scanning watch catches it on the first window. The receiver
scans with a window ≥ 1 steady-state interval so one scan sees at least one beacon:
recommendation for the watch, window 120 ms every 3 s while the screen is on, no scanning
with the screen off (the theme cannot be seen then anyway) → ~4 % radio duty, ≤ 3 s latency.

No authentication: the worst an attacker can do is recolour the watch.

## 2. Request (watch → desktop)

Legacy advertising, added as one more AD structure to the watch's existing connectable
advertisement (flags 3 B + name "OW-Watch" 10 B; the TX-power AD (3 B) is dropped to make
room: 31 − 13 = 18 B = 4 B manufacturer header + 14 B payload). The 128-bit service UUID
stays in the scan response as today.

```
off  size  field
 0    1    magic     0x54 'T'
 1    1    kind      0x02 = theme request
 2    1    nonce     random byte drawn when the button is pressed; the same value is
                     repeated for as long as that one press is advertised. Not a clock,
                     not a counter, nothing persisted: it only lets the desktop tell
                     "still the same press" from "pressed again".
 3    1    op        0x01 NEXT   0x02 PREV   0x03 reserved (was TOGGLE, dropped 2026-08-26:
                     Omarchy themes have no light/dark pairs, so it jumped to an unrelated
                     theme)   0x04 SET   0x05 RESEND (please burst the state beacon)
                     0x06 LIST (please push the theme list over GATT, §3)
 4    2    arg       SET: crc16/CCITT-FALSE of the theme slug (e.g. "tokyo-night"); the
                     desktop resolves it against `omarchy-theme-list`. Else 0.
 6    4    mac       first 4 bytes of HMAC-SHA256(key, bytes 0..6)
```

10 bytes. There is no time and no sequence number anywhere in the protocol.

Key: 16 random bytes, delivered by the pairing flow in §2b. Until paired the desktop ignores
requests.

Desktop acceptance: mac valid, and (address, nonce, op, arg) not seen in the last 60 s.
A captured packet could be replayed later by someone who also holds the radio — the
effect is one theme change, which is accepted as harmless.

The watch advertises the request for at most 10 s, or until the state beacon shows a `seq`
newer than the one it saw when the user pressed the button; then it restores its normal
advertising data. The desktop, on accepting: NEXT/PREV → `omarchy-theme-set` on the
neighbour in `omarchy-theme-list` order; SET → the slug whose crc16 matches; RESEND → a
beacon burst; LIST → a GATT connection to the requesting address and the §3 transfer.
`omarchy-theme-set` takes 2–5 s (app retints); the hook then bumps `seq` and starts a burst.

Desktop side runs one passive, duplicate-reporting scan continuously (BlueZ
`SetDiscoveryFilter {DuplicateData: true, Transport: le}`); the daemon both advertises and
scans, which the adapter supports concurrently.

## 2b. Pairing (the key, confirmed with a code on the watch)

```
desktop: themesync pair
  key  = 16 random bytes,  code = 1 random byte, shown as two hex digits, e.g. "7C"
  GATT write to 7a0e0005-…  [0x01][code][key 16 B]        18 bytes, write-only characteristic
  prints the code, waits (≤ 120 s) for the watch's confirmation
watch:  on that write → "Pairing" screen: two rollers 0-F (iOS-style wheel), OK
  entered == code → key → NVS namespace "pair", then advertise a RESEND request signed
                    with the NEW key (normal §2 request), show "paired"
  entered != code → discard, show "wrong code", back to the previous screen
  no OK within 120 s → discard
desktop: a §2 request whose MAC verifies with the pending key = confirmation:
  the pending key becomes the active key (~/.config/themesync/key); the old key is kept
  active until then, so a wrong code or a timeout changes nothing on either side.
```

The code is a confirmation, not key material (8 bits): it proves the person at the watch
is the person who ran `pair`, and which watch is being paired. The key itself travels over
GATT. Characteristic `…0005` accepts only this 18-byte form (no raw 16-byte key write);
write-only, not readable. A watch with no key in NVS may ship a build-time default key so
the request path can be exercised before the first pairing; the first successful pairing
replaces it.

## 2a. Priority on the watch

The Onewheel (ESC) link is what the watch is for. The beacon scan is strictly lower
priority: while the watch is connected to the ESC as a central, the scan backs off (longer
period) or pauses entirely; theme latency is never bought with a dropped telemetry packet.
The 120 ms / 3 s scan is a starting point to be **measured** (`SCROLL_STRESS=1` + periodic
ext-scan, watching "largest DMA block" in the heartbeat) before it is called final — the
earlier screen freeze was internal-RAM starvation while the radio was up.

## 3. What stays on GATT: the pairing key, and the theme list

Rare, large, or secret goes over a normal connection: the pairing key (§2b) and the theme
list below. `…0004` is reserved by the watch project. Neither is needed for the loop above.

### 3a. The theme list (agreed 2026-08-26; both sides implemented — watch firmware
github.com/ncr/onewheel `ce199f8` + `8b724c1`, verified from a Mac the same day with a
Python mock of the daemon (`tools/omarchy-watch-daemon.py` there): pair → code → RESEND with
the pending key → list pushed (22 themes, 1231 B) → swipe → pick → LIST refresh; the list is
saved to `/sdcard/onewheel/themes.bin` and reloaded at boot)

Every installed theme, so the watch can show a tappable picker (tap = a SET request) and
paint the picked theme before the desktop confirms it. The watch keeps it on its SD card
(NVS fallback).

```
CHARACTERISTIC  7a0e0006-0f0e-4d0c-9c0b-0a0908070605  "list"   read + write (with response,
                                                                one frame per write)

READ → status   [0x01 ver][count u8][crc16 le][flags u8]      5 bytes
                crc16 = CRC-16/CCITT-FALSE of the stored list bytes (the SET slug crc)
                flags bit0 = stored on SD, bit1 = a list is loaded

WRITE frames    each ≤ MTU − 3 (≤ 509 B at the watch's preferred MTU 512)
  BEGIN   [0x01][count u8][total u16 le][crc16 u16 le]          crc over the `total` list bytes
  DATA    [0x02][offset u16 le][bytes…]                          strictly sequential: offset ==
                                                                 bytes received so far, else
                                                                 ATT error 0x80 (out of order),
                                                                 0x81 (no BEGIN), 0x82 (too big).
                                                                 After any rejected frame, DATA
                                                                 keeps failing with 0x81 until a
                                                                 new BEGIN.
  COMMIT  [0x03][mac 4 B]   mac = HMAC-SHA256(pairing key, list bytes)[0..4]. The watch checks
                            crc + mac, saves, swaps the list in, and takes its own LIST request
                            (if any) off the air — synchronously, before the ATT ack, so a
                            READ right after the ack shows the new count/crc. Bad mac → ATT
                            error 0x83, bad crc → 0x84, bytes that do not parse → 0x85; the
                            list is discarded.

LIST BYTES      [0x01 ver][count u8] then count × [len u8][theme packet v2, `len` bytes]
                The v2 packet is exactly what the state beacon / characteristic …0002 carry:
                [0x02] + [role R G B] colour records (every role the desktop maps: 1..14;
                cursor (15) is derived by the watch, as for the beacon) + 0x40 name = the
                theme SLUG (ASCII, what omarchy-theme-set takes) + 0x41 flags (bit0 light).
                No 0x42/0x43 neighbour records. Entries in `omarchy-theme-list` order (= the
                NEXT/PREV order). ~75 B per entry; the stock 19 themes ≈ 1.4 KB.
                Limits (THEMELIST_MAX / THEMELIST_MAX_BYTES on the watch): 64 entries,
                8192 bytes; the desktop drops what does not fit, in list order, and logs it.

REQUEST OP      0x06 LIST, arg 0 (the signed 10-byte request of §2).
```

When the desktop pushes:

1. **After pairing** — the moment the RESEND signed with the pending key promotes it (§2b),
   the daemon connects to that address and pushes (skipped if the watch already reports
   this list's crc).
2. **On demand** — `themesync push-list` (through the daemon if it runs, else a one-shot
   connection with the key from `~/.config/themesync/key`); `--force` sends even when the
   crc matches; `--dry-run` prints the list and every frame without touching the radio.
3. **When the watch asks** — a LIST request whose MAC verifies: the daemon connects to the
   requesting address and pushes unconditionally, so the COMMIT that clears the request
   always lands.

The watch is local-first with the list: on a swipe or a pick it applies the entry's palette
itself at once and *then* sends NEXT/PREV/SET; the state beacon that answers just confirms
(same theme, new `seq` → the watch's runner stops). If the desktop's NEXT/PREV order ever
differs from the pushed list (a theme installed or removed after the push), the beacon wins
and the watch repaints to what the desktop actually set. Since `8b724c1` a theme written
to `…0002` over GATT also completes the watch's pending request, so a desktop that cannot
beacon (a Mac) closes the loop through the one-shot GATT push. On the desktop both orders come
from the same function (`Omarchy::list_themes`, user + system dirs, sorted, de-duplicated),
so they can only diverge when the installed set changes — `themesync push-list` or the
watch's "refresh list" row (a LIST request) brings the watch back in step.

Desktop details: one transfer at a time (a trigger during a transfer is dropped, the next
one sends the current list anyway); find + connect + push is retried 3× except after an
ATT error (a rejected frame will not pass on retry; a retry is a fresh BEGIN, never a
resume); DATA frames are sized from the MTU BlueZ negotiated (it asks for 517 on connect,
the watch prefers 512 since 2026-08-26 → 509-byte frames, 3–4 writes for 19 themes). After
COMMIT the status is read back and must show this list's crc and count.

Interop vector (key `00 01 … 0f`, one entry: bg 10 20 30, fg 40 50 60, name "nord", flags 0):

```
list    01 01 12 02 01 10 20 30 02 40 50 60 40 04 6e 6f 72 64 41 01 00     (21 B)
crc16   0xDB97
BEGIN   01 01 15 00 97 db
DATA    02 00 00 + the 21 list bytes
COMMIT  03 35 e8 31 b9
```

## 4. Open questions for the watch side

1. Extended scanning (`ble_gap_ext_disc`) alongside advertising and, later, the Onewheel
   central link — any objection to the 120 ms / 3 s window while the screen is on?
2. (settled: the TX-power AD is dropped.)
3. (settled: key characteristic `…0005`, write-only, NVS namespace `pair`.)
4. (settled 2026-08-26, user's call: no time and no counter in any packet; a per-press
   random nonce only.)
5. UI (settled by the user 2026-08-26): a "Themes" screen showing prev | current | next
   from the beacon's 0x42/0x43 records; swipe = PREV/NEXT. No TOGGLE. Tap = the §3 list
   (pick = SET), with a "refresh list" row that sends LIST.
