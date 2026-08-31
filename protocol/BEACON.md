# Theme beacon — connection-less theme sync over BLE advertising

**v3, 2026-08-27.** Between `omarchy-themesync` (desktop) and the OW-Watch firmware
(`~/dev/onewheel/watch`). Rewritten after the independent review in
`docs/review-2026-08-27.md`; no compatibility with v1 (2026-08-26) or v2 (2026-08-27 morning).
What changed and why, in one line each:

- The beacon carries **content and a MAC**, no `seq`, no `host`, no ack record. A receiver
  applies a beacon when its theme bytes differ from what it is showing (reported state, like
  an MQTT retained message), and verifies the MAC before applying. The review showed `seq`
  stranding the watch (a beacon ignored while waiting for a request was recorded as applied),
  a 1-in-256 silent ignore after a desktop restart, and an unauthenticated beacon writing the
  watch's flash on every packet.
- A request is answered when the beacon **shows the theme the request named** (the watch's
  `theme_expect` rule); the nonce/ack machinery that did the same job a second way is gone.
- The request carries a **monotonic counter** instead of a random nonce; the desktop accepts
  only a counter greater than the last one accepted for the key. One rule replaces the
  address-keyed dedup and the BlueZ-cache sweep at daemon start, and gives replay
  protection for free.
- The theme-list MAC covers the **transfer header** as well as the bytes.
- **Reliable requests (added the same evening):** the beacon echoes the last request counter
  the desktop accepted (`0x43`), and the watch retransmits until it sees its counter echoed
  — stop-and-wait ARQ, the same shape as CoAP confirmable messages (RFC 7252), MQTT QoS 1
  and BLE Mesh's segment acks. Before this a request lost on the air was a lost gesture
  (on hardware: 6 of 16 in one series, every one a gesture made a few seconds after the
  previous answer).
- Radio: the desktop beacons at a **fixed 30 ms**, constantly (the watch scans a 45 ms
  window every 1 s idle, every 320 ms with a request out); the watch advertises every
  **1 s** when idle with an empty scan response (the desktop's scan is active, so the watch
  answered a SCAN_RSP with the service UUID every 200 ms, forever) and 100 ms while a
  request is on the air.

Two independent broadcasts, no GATT connection needed for day-to-day use:

```
desktop  ──(state beacon: the current theme + MAC, repeated forever)──▶  any receiver
watch    ──(request: SET <theme> / RESEND / LIST, counter, MAC, ≤10 s)──▶ desktop
                                    ▲                                        │
                                    └── the desktop applies it; its beacon now shows
                                        that theme; the watch sees it and stops asking
```

Both packets are Manufacturer Specific Data (AD type `0xFF`), company id `0xFFFF` (the
"reserved for testing" id; the magic byte disambiguates from other 0xFFFF users). All
multi-byte integers little-endian. `crc16` = CRC-16/CCITT-FALSE. `mac4(k, m)` = the first 4
bytes of HMAC-SHA256(k, m). The key `k` is the 16-byte pairing key (§2b). One desktop per
watch: the key is the desktop's identity, so a watch follows exactly the desktop it paired with.

The watch holds the theme list (§3): it is pushed right after pairing and whenever the watch
asks. Every theme is addressed by `crc16(slug)` — the slug is the name `omarchy-theme-set`
takes, e.g. `"tokyo-night"` → `0xAAE5`. The watch computes it from the `0x40` name of a list
entry; the desktop resolves it against `omarchy-theme-list`. (Known weakness, accepted for
now: ~0.4 % chance of a collision among 22 slugs; the desktop refuses to build a list with
a collision. Planned replacement: `arg = [list index][list crc8]`, see §5.)

## 1. State beacon (desktop → receivers)

Extended advertising (BLE 5, primary 1M, secondary 1M), non-connectable, non-scannable.
Verified 2026-08-26 that BlueZ 5.87 on the Intel BT 5.4 adapter registers such an
advertisement with 77 bytes of manufacturer data. Any receiver that cannot do extended
scanning gets nothing; that is accepted (the watch can: `CONFIG_BT_NIMBLE_50_FEATURE_SUPPORT`).

```
off  size  field
 0    1    magic     0x54 'T'
 1    1    kind      0x01 = theme state
 2    n    theme     the OW-Watch `colors` packet, verbatim: v2 TLV
                     ([2][role r g b]*[0x40 len name][0x41 1 flags]) — same bytes the
                     desktop would write to characteristic 7a0e0002; the receiver hands
                     them to theme_set_wire() unchanged. The 0x40 name is the theme SLUG.
 2+n  4    echo      [0x43][2][ctr u16 le] — the last request counter the desktop accepted
                     under this key, 0 = none yet. The sender's ack number (§2).
 6+n  9    time      [0x44][7][year-2000][month 1-12][day][hour][min][sec][wday 0-6]
                     — the desktop's local civil wall clock at the moment the payload was
                     built, weekday 0 = Sunday. Optional: a receiver must accept a beacon
                     without it (and one with it: firmware older than the record skips
                     unknown tags ≥ 0x40 and only requires the mac to be last).
15+n  6    mac       [0x42][4][mac4(k, everything before it)] — always the last record;
                     covers everything before it, the echo and the time included.
                     All three are meta records the theme parser skips. ~89 bytes in total.
```

**Time.** The daemon re-signs and re-registers the advertisement at every wall-clock second
change, so the stamp in the air is at most ~1 s stale (plus the ≤ 40 ms advertising gap);
a receiver may treat it as "now" with a one-second error bar and set its RTC from it (the
watch does, when the drift exceeds a few seconds — which is why the refresh cannot be
slower: a stamp refreshed every 10 s would look like 10 s of drift on a healthy clock).
Local civil time on purpose — the watch face shows what the desktop's clock shows, no time
zone database on either side; DST jumps arrive as a one-hour "drift" and are corrected like
any other. Out-of-range years saturate at 2000/2255; a leap second is sent as :59.

**Apply rule (receiver).** Let `crc = crc16(bytes 2 .. 2+n)` (the theme bytes only — the
echo changes with every request, the theme does not).

1. `crc == applied_crc` and no request outstanding → nothing (the common case: the same
   beacon hundreds of times). While a request is outstanding this shortcut is skipped: the
   watch has already painted the list entry locally, and the beacon that answers carries
   the same bytes (same encoder on the desktop) — it must still reach rule 4.
2. Verify the mac. Bad → drop, count it (a stranger, or a desktop with another key).
3. If a request is outstanding (`expect` = the slug it asked for) and the beacon's slug is
   not `expect` → ignore, and **do not record anything** (the desktop changed on its own
   while the request was in flight; the request's own answer is still coming).
4. Apply, `applied_crc = crc`. If the slug equals `expect`: the request is answered — clear
   `expect`, take the request off the air.

Persisting: the watch writes the theme to NVS only when it has been stable for 10 s,
differs from what NVS holds, and **at least 60 s after the previous write** (a change inside
that minute waits for it to end; the first write after boot is not delayed). Verified
2026-08-27: `saved to NVS: 'osaka-jade' (72 B, stable 10 s)`.

There is no sequence number. A restarted desktop, a late joiner, a beacon delivered twice
and two desktops in range (only one has the key) all converge on the content.

**Replay is accepted.** The beacon has no freshness (the echo is the desktop's ack number,
not a nonce), so a beacon recorded under the current key stays valid: a stranger who
recorded two beacons can alternate them and the watch follows — the worst outcome is a
theme the desktop once had, until the real beacon (≤ 1 s away with the screen on) wins
again. The same goes for the time record: a replayed beacon carries the clock of its
recording, so the watch bounds the damage on its side — it corrects the RTC only past a
drift threshold, rate-limits corrections, and the next real beacon (seconds away) sets the
clock right again. The MAC stops forgery, not replay; the 10 s stability window and the 60 s NVS floor
bound the flash cost of a replay to one write a minute. A counter under the MAC would close
this; it is not worth a persisted state on the watch for a nuisance.

**Timing.** Fixed 30 ms interval, constantly (min = max; the controller adds 0–10 ms of
advDelay per event, so the worst gap on a primary channel is 40 ms, inside the watch's 45 ms
window). No "burst" after a change: the desktop's rate is the same at all times, and the
watch's window is sized for it. The watch runs one continuous scan with a 45 ms window
every 1 s while the screen is on (4.5 % of radio time, the same budget as the earlier
120 ms / 2.56 s, with 2.5× lower latency), one window immediately when the screen turns
on, and 45 ms every 320 ms while it has a request on the air (≈14 %, ≤10 s); no scanning
with the screen off (the theme cannot be seen then anyway). Worst-case latency for a
desktop-initiated change, screen on: 1 s.

Measured 2026-08-28 (`btmon`: "Min/Max advertising interval: 30.000 msec" — BlueZ programs
what the daemon asks for). Desktop → watch, from the end of `omarchy-theme-set` to the
watch's `beacon: applied` line, 21 samples at uneven spacing: 0.11 … 1.15 s, mean ≈ 0.6 s,
uniform over the 1 s scan period plus a ~0.1 s floor (the daemon publishes ~1 ms after the
command ends; the rest is UART/polling on the watch); one 2.03 s — one missed window in
~21 at −68…−71 dBm, where a window holds only 1–2 advertising events and one corrupted
packet costs a full period. For comparison, the 80 ms beacon against the same 45 ms window
missed every second window (0.09 0.09 0.12 1.08 1.08 1.26 s). Measurement trap: samples at
a constant ~3.0 s spacing gave 8 × 0.27 ± 0.02 s — the test loop phase-locked to the 1 s
scan period; space the samples unevenly.

### 1a. The theme packet (v2 TLV)

The theme bytes are the OW-Watch's own `colors` format (`main/theme.h` in the firmware;
`host/src/protocol.rs` mirrors it). This is the only place it is written down outside the
two implementations. `protocol/THEME_PROTOCOL.md` describes Theme Protocol v1, an earlier
design that no device speaks.

```
[0x02]  then records, in any order:
  colour  [role id 1..0x3F][R][G][B]
  meta    [tag >= 0x40][len][payload]      0x40 name (UTF-8, <= 31 B, the theme slug)
                                           0x41 flags (1 B): bit0 light, bit1 force_black
                                           0x43 echo (2 B) and 0x42 mac (4 B): beacon only, §1
```

Role ids (append-only; a receiver skips ids it does not know and derives every role it
was not given from background + foreground, which are required):

```
   1  Background
   2  TextPrimary
   3  Accent
   4  Danger
   5  Warning
   6  Success
   7  Info
   8  Surface
   9  SurfaceAlt
  10  TextSecondary
  11  TextDisabled
  12  OnAccent
  13  Selection
  14  Divider
```

A record that would run past the end of the packet ends parsing; a duplicate `0x40`/`0x41`
last-wins. The list entries of §3a carry exactly this packet with the slug as the name and
no meta records beyond `0x41`.

## 2. Request (watch → desktop)

**Its own advertising instance, its own address.** The request goes out on a second,
non-connectable, non-scannable legacy advertisement (ADV_NONCONN_IND: flags 3 B + 2 + 2 +
11 = 18 ≤ 31 B), **from a fresh non-resolvable private address** (top two bits 00, the rest
random) drawn for every transmission and every retransmission (start, 1.5 s, 3 s, 6 s).
Reason: LE controllers filter duplicate advertising reports **by advertiser address**; a
request swapped in place under the watch's static address stayed invisible to the desktop
until the kernel's periodic scan restart (0–2.2 s, measured on an Intel controller
2026-08-27), and the only cure on the host side — an Advertisement Monitor, which makes the
kernel scan with the filter off — needs `Experimental = true` in bluetoothd, a root-level
change on every user's machine. A new address per attempt makes every request "a new
device" to any controller, so it is reported at once, with a stock BlueZ. Side effects: the
desktop identifies the watch by the MAC only and **never records a request's source
address**; the GATT address for list pushes is learned at pairing (§2b, `pair` connects to
the watch and knows it); nobody can track the watch by its request traffic.

Measured 2026-08-28, host with the Advertisement Monitor deliberately off
(`THEMESYNC_NO_MONITOR=1`, i.e. a stock BlueZ): 43 swipes and picks, every one a new
address (NRPA, top bits 00), every one seen and accepted, none rejected or repeated —
including a request the watch replaced after 0.7 s. Watch-side loop request → echo: 0.2–0.6 s
(that is the watch's own 320 ms scan window while a request is out), zero retransmissions
across the series. Before this change, the same host without the monitor missed every
second request and saw the rest 0–2.2 s late.

The watch's connectable advertisement (ADV_IND, static address, name "OW-Watch", 1 s idle)
stays as it was, without the request, for GATT (pairing, list push). It is scannable by
definition, so the watch keeps a scan response but an **empty** one: the 128-bit service
UUID is not advertised — the desktop connects by address and finds an unpaired watch by its
name.

```
off  size  field
 0    1    magic     0x54 'T'
 1    1    kind      0x03 = theme request (v3; v1/v2 requests were 0x02 and 10 bytes —
                     a mixed pair fails on the kind, not on the length)
 2    2    ctr       monotonic counter, per pairing key (see below); never 0
 4    1    op        0x01 SET     switch to the theme in `arg`
                     0x02 RESEND  ping: echo this counter, change nothing
                     0x03 LIST    please push the theme list over GATT (§3)
 5    2    arg       SET: crc16(slug). Else 0 (enforced: a signed RESEND/LIST with arg ≠ 0 is dropped).
 7    4    mac       mac4(k, bytes 0 .. 7)   — checked before anything else in the packet is
                                              looked at: a stranger gets one answer, no parse oracle
```

11 bytes. No time anywhere in the protocol.

**Counter.** Strictly increasing per key, u16, in the MAC. The desktop keeps the last
accepted value next to the key (`~/.config/themesync/ctr`, written atomically: temp file +
rename) and accepts a request iff `ctr > last`; on acceptance `last = ctr`. Gaps are fine
(lost requests, crashed watch). Pairing (§2b) resets both sides: the desktop's `last` for the
new key is 0, the watch starts at 1. The watch persists the counter in blocks: at boot it
reads `next_block` from NVS, uses counters from there, and writes `next_block + 100` back —
one flash write per 100 presses, a crash costs at most 100 values (BLE Mesh does the same
with its sequence numbers). At 65 535 the watch stops sending and shows "re-pair".

**Desync is loud, never silent.** A request with `ctr < last` (an erased watch after a
reflash, a replay, a stale copy from BlueZ's cache) is dropped with a log line naming both
numbers and the remedy (`themesync pair` again, or `themesync reset-counter` on a watch you
trust), and `themesync status` shows the count of such drops. `ctr == last` is different:
it is the watch's own retransmission of the request just accepted (BlueZ re-delivers every
advertising event while it is on the air) — expected, dropped silently, not counted. The
desktop never accepts a backward jump on its own. A counter file the desktop cannot parse
locks it (nothing accepted, `status` says so) rather than reopening every replay as 0.

Acceptance on the desktop: kind and length right, mac valid under the active key (or the
pending key: §2b), `ctr > last`. Nothing is keyed by the source address any more; the address
of an accepted request is remembered only as where to connect for a list push.

**On the air — stop-and-wait with retransmission.** One request outstanding at a time; the
watch's state machine:

```
IDLE ──gesture (debounced ~300 ms)──▶ SENT      request on the air at 100 ms, ctr = next
SENT ──beacon with echo ≥ ctr──▶ RECEIVED       the desktop has it; wait for the theme
SENT ──no echo for 1.5 s──▶ retransmit          stop/start the advertisement with the same
                                                bytes; wait 3 s, then 6 s; after the third
                                                miss (~10.5 s) → IDLE, "no answer",
                                                applied_crc forgotten
RECEIVED ──beacon slug == expect, or list COMMIT──▶ IDLE   answered
RECEIVED ──10 s without either──▶ IDLE          the desktop had it but did not apply it
                                                (unknown slug and the list push failed, or
                                                overridden); applied_crc forgotten
any ──new gesture──▶ SENT with a new ctr        newest wins, nothing queues
```

Retransmitting the same counter is safe: the desktop either has it (drops it as already
seen) or has not (accepts it) — at-least-once delivery, exactly-once effect. Leaving SENT or
RECEIVED without an answer forgets `applied_crc`, so the very next beacon repaints the
watch to the desktop's actual theme: the desktop is the source of truth and a lost request
must never leave the watch on a theme the desktop does not have. Back in IDLE the
advertisement returns to its 1 s interval. The watch shows the state (sent / received /
applied) so "no answer" only ever means the desktop was unreachable for ~10 s.

**The desktop, on accepting** — first, before anything else, it puts `ctr` into the beacon's
echo, so the watch sees "received" within one scan window (≤ 0.32 s) and
stops retransmitting while `omarchy-theme-set` is still running. Then:

- SET, slug known → `omarchy-theme-set <slug>` (2–5 s, apps retint; the hook then triggers
  a fresh beacon). Requests do not queue: while one `omarchy-theme-set` runs,
  only the newest SET received is kept and runs next.
- SET, slug unknown → the watch's list is stale (a theme removed or renamed since the push):
  the desktop pushes the current list over GATT to the requesting address (§3) and changes
  no theme. The COMMIT takes the SET off the air; the user picks again from the fresh list.
- RESEND → nothing more (the echo above is the answer).
- LIST → a GATT connection to the requesting address and the §3 transfer.

**Desktop scan.** BlueZ discovery is an *active* scan (`SetDiscoveryFilter {DuplicateData:
true, Transport: le}` has no passive option) — the desktop sends SCAN_REQs; that is why the
request advertisement is non-scannable. Every request arrives as a **new BlueZ device**
(new address): the daemon reads that device's manufacturer data once when BlueZ adds it,
then follows the `ManufacturerData` property-changed signal. Devices that already existed
when the scan started are followed but never read: BlueZ keeps a device's last manufacturer
data cached for a while after it stopped advertising, and a minutes-old request would pass
the counter check. A request on the air during the ~1 s of a daemon restart is lost; the
watch times out after 10 s and repaints from the beacon. When bluetoothd offers an
Advertisement Monitor (`Experimental = true`) the daemon registers one as a bonus — the
kernel then scans without the controller's duplicate filter — but nothing depends on it.
The daemon both advertises and scans; the adapter supports that concurrently.

## 2b. Pairing (the key, confirmed with a code on the watch)

Started on the watch (its Pair screen) or on a desktop (`themesync pair`); either way every
desktop that takes part *offers* a key, and the person at the watch picks the desktop and
proves it with that desktop's code. The watch keeps up to 4 pairings and one of them is
*active*.

```
watch:   Pair screen → for 60 s its connectable advertisement (static address) also carries
         manufacturer data 0xFFFF  'T' 0x04 [token u32 le]     6 B, unsigned (no key yet);
         the token is random per opening of the screen
desktop: every daemon in range that sees a token it has not answered:
  key  = 16 random bytes,  code = 1 random byte, shown as two hex digits, e.g. "7C"
  GATT write to 7a0e0005-…  [0x01][code][key 16 B][name 0–12 B]    18–30 bytes, write-only;
                            name = the desktop's hostname (UTF-8, cut on a character boundary)
  shows the code in a desktop notification ("On the watch pick \"<name>\" and enter 7C");
  holds the key as *pending* for 120 s (`themesync pair` from the CLI does the same offer,
  finding the watch by its saved address or its name, without waiting for a 0x04)
watch:  collects the offers (≤ 4) while the screen is open and lists their names;
  the user picks one and enters its code on two rollers 0-F
  entered == code → {name, key} → NVS (up to 4 pairings; a 5th replaces the oldest), it
                    becomes the active pairing, then a RESEND request signed with the NEW
                    key, "paired"
  entered != code → "wrong code", the offer stays until the screen closes
  screen closed / 60 s → every offer is dropped
desktop: a §2 request whose MAC verifies with the pending key = confirmation:
  the pending key becomes the active key (~/.config/themesync/key), last counter = that
  request's ctr, the address the offer was written to is saved (~/.config/themesync/watch)
  for list pushes — the request itself comes from a throwaway random address (§2);
  the old key stays active until then, so a wrong code, a timeout, or the user picking
  another desktop changes nothing here. The pending key is forgotten 120 s after the offer
  (it survives a daemon restart inside the window).
```

Verified 2026-08-28 21:07 on the hardware, started from the watch's Pair screen: `0x04`
seen → offer written as "spawner" (10 s: finding the watch by address in a fresh btleplug
scan against a 1 s advertisement) → code entered → confirmation from a random address
(30:2D:…) → key saved, list push aimed at the pairing address (skipped: the watch held the
same list), 12 SETs afterwards with the watch's counter continuing unreset.

**Several desktops.** The watch verifies every beacon against every stored key (after the
crc rule of §1, so only a changed beacon costs a MAC per key) and calls a desktop "in
range" while its beacon was seen ≤ 3 s ago. It **paints from the active desktop's beacon
only** and signs its requests with the active key only; a desktop verifies with its own key,
so the others drop the request on the MAC and nothing else changes. One request counter on
the watch, shared by all pairings (monotonic; each desktop needs only `ctr > its last`).
The Computers screen on the watch lists the pairings with an in-range dot and the active
one; tap = make active (and repaint from its last beacon, if in range); hold = forget.

The code is a confirmation, not key material (8 bits): it proves the person at the watch is
the person looking at that desktop's screen, and which watch is being paired. The key itself
travels over GATT.

A stranger's watch advertising `0x04` makes every desktop in range write an offer to it and
show a code — a nuisance, not a break: the active key stays until *this* desktop's code is
entered on that watch, which needs its screen. One offer per token; a desktop remembers
answered tokens for 5 minutes.

Known weaknesses, accepted for now and listed in §5: the key travels in clear over an
unencrypted link (a sniffer within range during the pairing window gets it); any central
can open the Pairing screen on the watch; a build-time default key may exist in the firmware
for bench work and must not count as "paired".

## 2a. Priority and energy on the watch

The Onewheel (ESC) link is what the watch is for. The beacon scan is strictly lower
priority: while the watch is connected to the ESC as a central, the scan backs off or pauses
entirely; theme latency is never bought with a dropped telemetry packet.

Where the radio time goes, and what this version does about it:

| activity | before v3 | v3 |
|---|---|---|
| beacon scan, screen on | 120 ms / 2.56 s ≈ 4.7 % | **45 ms / 1 s = 4.5 %** (same budget, ≤ 1 s latency; needs the desktop at 30 ms) |
| beacon scan, request out | 120 ms / 640 ms ≈ 19 %, ≤ 10 s | **45 ms / 320 ms ≈ 14 %**; typically over in 1–2 s (echo ≤ 0.32 s, theme +0.4 s) |
| retransmissions | — | ≤ 3 stop/start per gesture, milliseconds of radio |
| beacon scan, screen off | none | none |
| own advertisement, idle | 200 ms, connectable + scannable | **1 s**, connectable (ADV_IND) |
| scan responses to the desktop's active scan | one per ~200 ms, forever, with the UUID | one per ~1 s, empty |
| own advertisement, request out | 200 ms | **100 ms, ≤ 10 s**, on a second non-connectable instance from a fresh random address per attempt (2026-08-28) |
| NVS writes | every applied beacon | after 10 s stable, only if different |
| HMAC | — | one SHA-256 per *changed* theme (rule 1 of §1 runs first) |

The 45 ms / 1 s figure's effect on the screen is still to be **measured** (`SCROLL_STRESS=1` + periodic
ext-scan, watching "largest DMA block" in the heartbeat) — the earlier screen freeze was
internal-RAM starvation while the radio was up.

## 3. What stays on GATT: the pairing key, and the theme list

Rare, large, or secret goes over a normal connection: the pairing key (§2b) and the theme
list below. `…0004` is reserved by the watch project. Neither is needed for the loop above.
The desktop connects **by address** (the one `pair` connected to, saved then; requests
arrive from random addresses and teach it nothing); the watch's scan response is empty, so
the service UUID is discovered after connecting, not from a scan.

### 3a. The theme list

Every installed theme, so the watch can show a picker and a prev | current | next switcher,
paint the chosen theme before the desktop confirms it, and address it by slug crc in a SET.
The watch keeps the list on its SD card (NVS fallback).

```
CHARACTERISTIC  7a0e0006-0f0e-4d0c-9c0b-0a0908070605  "list"   read + write (with response,
                                                                one frame per write)

READ → status   [0x02 ver][count u8][crc16 le][flags u8][nonce u32 le]    9 bytes
                crc16 = crc16 of the stored list bytes
                flags bit0 = stored on SD, bit1 = a list is loaded
                nonce: random, never 0, drawn at boot and again after EVERY COMMIT frame
                (accepted or rejected); BEGIN/DATA leave it. The desktop reads it right
                before BEGIN and signs the COMMIT against it. (The list bytes' own version
                byte stays 0x01: the list format did not change, the status did.)

WRITE frames    each ≤ MTU − 3 (≤ 509 B at the watch's preferred MTU 512)
  BEGIN   [0x01][count u8][total u16 le][crc16 u16 le]          crc over the `total` list bytes
  DATA    [0x02][offset u16 le][bytes…]                          strictly sequential: offset ==
                                                                 bytes received so far, else
                                                                 ATT error 0x80 (out of order),
                                                                 0x81 (no BEGIN), 0x82 (too big).
                                                                 After any rejected frame, DATA
                                                                 keeps failing with 0x81 until a
                                                                 new BEGIN.
  COMMIT  [0x03][mac 4 B]   mac = mac4(k, 0x03 ‖ nonce u32 le ‖ total u16 le ‖ crc16 u16 le ‖ list bytes):
                            bound to this transfer (the nonce), so a recorded push cannot be
                            played back later — the characteristic is writable by any central
                            and this MAC is the only thing that gates it. The watch
                            checks crc + mac, saves, swaps the list in, and takes its own
                            request (LIST or SET, if any) off the air — synchronously, before
                            the ATT ack, so a READ right after the ack shows the new count/crc.
                            Bad mac → ATT error 0x83, bad crc → 0x84, bytes that do not
                            parse → 0x85; the list is discarded.

LIST BYTES      [0x01 ver][count u8] then count × [len u8][theme packet v2, `len` bytes]
                A slug longer than 31 bytes (the v2 name limit) is left out of the list: the
                packet would truncate the name the watch hashes into its SET, which could then
                never match the full slug the desktop resolves — an endless "unknown theme →
                push the list" loop.
                The v2 packet is exactly what the state beacon / characteristic …0002 carry:
                [0x02] + [role R G B] colour records (every role the desktop maps: 1..14;
                cursor (15) is derived by the watch, as for the beacon) + 0x40 name = the
                theme SLUG (ASCII, what omarchy-theme-set takes) + 0x41 flags (bit0 light).
                No 0x42 mac record. Entries in `omarchy-theme-list` order (= the switcher
                order on the watch). ~75 B per entry; the stock 19 themes ≈ 1.4 KB.
                Limits (THEMELIST_MAX / THEMELIST_MAX_BYTES on the watch): 64 entries,
                8192 bytes; the desktop drops what does not fit, in list order, and logs it.
                The desktop refuses to build a list in which two slugs share a crc16.

REQUEST OP      0x03 LIST, arg 0 (the signed 11-byte request of §2).
```

When the desktop pushes:

1. **After pairing** — the moment the RESEND signed with the pending key promotes it (§2b),
   the daemon connects to that address and pushes (skipped if the watch already reports
   this list's crc).
2. **On demand** — `themesync push-list` (through the daemon if it runs, else a one-shot
   connection with the key from `~/.config/themesync/key`); `--force` sends even when the
   crc matches; `--dry-run` prints the list and every frame without touching the radio.
3. **When the watch asks** — a LIST request that verifies: pushed unconditionally, so the
   COMMIT that clears the request always lands.
4. **When the watch asks for a theme the desktop does not have** — a SET whose crc matches
   no installed slug (§2): pushed unconditionally, like 3.

After a successful push the watch has just forgotten its applied theme (a list COMMIT
clears `applied_crc` like a timeout does) and repaints from the first beacon it sees, ≤ 1 s
later; the desktop does nothing extra (the beacon is always on the air at 30 ms).

The watch is local-first with the list: on a swipe or a pick it applies the entry's palette
itself at once and *then* sends SET with that entry's slug crc; the beacon that answers
shows the same theme (§1 rule 4) and the watch's runner stops. A theme written to `…0002`
over GATT also completes a pending request whose slug it matches, so a desktop that cannot
beacon (a Mac) closes the loop through the one-shot GATT push. The list can only diverge
from the desktop when the installed set changes after the push; then either the beacon shows
a slug the watch does not know (the watch shows that theme alone, without neighbours, and
may send LIST once) or a SET names a slug the desktop does not know (the desktop answers with
the list). `themesync push-list` or the watch's "refresh list" row do the same by hand.

Desktop details: one transfer at a time (a trigger during a transfer is dropped, the next
one sends the current list anyway); find + connect + push is retried 3× except after an
ATT error (a rejected frame will not pass on retry; a retry is a fresh BEGIN, never a
resume); DATA frames are sized from the MTU BlueZ negotiated (it asks for 517 on connect,
the watch prefers 512 → 509-byte frames, 3–4 writes for 19 themes). After COMMIT the status
is read back and must show this list's crc and count.

## 4. Interop vectors (key `00 01 … 0f`)

```
request  SET "tokyo-night" (crc 0xAAE5), ctr 1   54 03 01 00 01 e5 aa 3f 0e c9 9b
request  RESEND, ctr 2                            54 03 02 00 02 00 00 0c c2 e4 fc
request  LIST, ctr 3                              54 03 03 00 03 00 00 ea 01 ce c7

beacon   theme: bg 10 20 30, fg 40 50 60, name "nord", flags 0
         echo 0:  54 01 02 01 10 20 30 02 40 50 60 40 04 6e 6f 72 64 41 01 00 43 02 00 00 | 42 04 4c dc bd 41
         echo 1:  54 01 02 01 10 20 30 02 40 50 60 40 04 6e 6f 72 64 41 01 00 43 02 01 00 | 42 04 ca a5 31 27
         echo 1, time 2026-08-31 13:05:09 Monday:
                  54 01 02 01 10 20 30 02 40 50 60 40 04 6e 6f 72 64 41 01 00 43 02 01 00 | 44 07 1a 08 1f 0d 05 09 01 | 42 04 e2 8a ef c0
         theme crc16 (the apply rule's key) = 0xD5E2 for all three      30 / 39 bytes

list     01 01 12 02 01 10 20 30 02 40 50 60 40 04 6e 6f 72 64 41 01 00     (21 B)
         crc16 0xDB97
status   02 01 97 db 02 04 03 02 01                                    (READ before BEGIN: nonce 0x01020304)
BEGIN    01 01 15 00 97 db
DATA     02 00 00 + the 21 list bytes
COMMIT   03 00 7f 0f 8a          mac4(k, 03 ‖ 04 03 02 01 ‖ 15 00 ‖ 97 db ‖ list); with nonce 0: 12 05 ec b8
```

## 5. Settled, and still open

Settled by the user:
- 2026-08-26: no time in any packet (the watch RTC loses time on power-off). Two-way over
  advertising, no GATT session in daily use. The watch's v2 TLV is the shared theme format.
  (The "no time" half was reversed on 2026-08-31 — see below; the rest stands.)
- 2026-08-27 (morning): the watch names the theme (SET by slug crc); no NEXT/PREV; the watch
  shows prev | current | next from its own list; greenfield — wire bytes may be renumbered.
- 2026-08-27 (v3): a counter is fine (it is not a clock and needs no sync beyond pairing);
  keep the 2.56 s idle scan (≤ 3 s latency over battery); idle advertisement 1 s and
  non-scannable; MAC the beacon now.
- 2026-08-28: same scan budget, spent as 45 ms every 1 s instead of 120 ms every 2.56 s;
  the desktop therefore beacons at 30 ms constantly and the 10 s burst is gone.
- 2026-08-28 (release): the request goes out on its own non-connectable instance from a
  fresh non-resolvable private address per (re)transmission, so the controller's duplicate
  filter never hides it — the Advertisement Monitor (BlueZ `Experimental = true`, a root
  change on every user's box) becomes optional. The desktop never records a request's
  address; the GATT address comes from pairing.
- 2026-08-28 (code review, `docs/review-code-2026-08-28.md`): the list COMMIT MAC gets a
  per-transfer nonce from the READ status (it was a pure function of the list bytes, so a
  recorded push replayed); beacon replay is accepted and written down, with a 60 s floor on
  NVS writes; the request MAC is checked first and `arg = 0` is enforced; the pending key
  expires after 120 s on the desktop too.
- 2026-08-31: the beacon carries the desktop's wall clock after all (the `0x44` record, §1),
  so the watch RTC no longer depends on the build time — the earlier "no time in any packet"
  predates the RTC being used for a clock face. Optional record between echo and mac, under
  the mac; the daemon re-signs every second so the stamp never goes stale; older firmware
  skips it. The watch corrects its RTC only past a drift threshold and rate-limits the
  corrections (its side of §1's replay note).

Open, in the order they should be taken:
1. **Pairing hardening**: accept the `…0005` write only within ~60 s of the user opening
   "Pair" on the watch; then `BLE_GATT_CHR_F_WRITE_ENC` (LE link encryption) or an X25519
   agreement with the two digits as a short authenticated string over the transcript;
   the build-time default key behind a build flag, off by default.
2. **Theme addressing** `arg = [list index u8][list crc8 u8]` (an ETag / If-Match): no crc
   collisions, and a stale list is detected on every SET, including renames and additions.
3. Measure: the watch's scan duty under the ESC link (the beacon interval is done: 30 ms
   confirmed with `btmon`, §1).
