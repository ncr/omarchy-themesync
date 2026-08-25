# Theme Protocol v1

A tiny, versioned wire format for pushing a complete semantic colour theme from a desktop
to a wearable, and for the wearable to ask the desktop for a theme change. It knows
nothing about Omarchy: any source (pywal, wallust, a Catppuccin flavour, a hand-written
JSON file) that can produce the 14 roles below can drive any receiver that implements it.

Reference implementations: `host/src/protocol.rs` (Rust), `watch/common/theme_proto.c` (C,
no dependencies, used unchanged by the ESP32 firmware and the desktop simulator).

## Semantic palette (the roles)

Slot numbers are the wire order. The list is **append-only**; never renumber.

| slot | role             | meaning on the watch                                   |
|-----:|------------------|--------------------------------------------------------|
| 0    | `background`     | screen background                                      |
| 1    | `surface`        | cards, panels                                          |
| 2    | `surface_alt`    | wells / bar tracks / neutral buttons (one step further)|
| 3    | `text_primary`   | readable text                                          |
| 4    | `text_secondary` | titles, hints, units                                   |
| 5    | `text_disabled`  | inactive / placeholder                                 |
| 6    | `accent`         | the theme's accent (primary buttons, highlights)       |
| 7    | `on_accent`      | text drawn on top of `accent`                          |
| 8    | `selection`      | selected row / selection tint                          |
| 9    | `divider`        | hairlines, borders                                     |
| 10   | `danger`         | errors, pushback, battery critical                     |
| 11   | `warning`        | warnings, battery low                                  |
| 12   | `success`        | ok, charging, connected                                |
| 13   | `info`           | informational (kept distinct from `accent`)            |

Human-readable form (what `omawatch theme --json` prints, and what a future JSON transport
would carry verbatim):

```json
{
  "version": 1, "mode": "dark", "name": "tokyo-night",
  "background": "#1a1b26", "surface": "#24283b", "surface_alt": "#31364b",
  "text_primary": "#a9b1d6", "text_secondary": "#787e9a", "text_disabled": "#414868",
  "accent": "#7aa2f7", "on_accent": "#1a1b26", "selection": "#292e42", "divider": "#414868",
  "danger": "#f7768e", "warning": "#e0af68", "success": "#9ece6a", "info": "#449dab"
}
```

## ThemeState packet (desktop → watch)

All multi-byte integers little-endian.

```
off  size  field
 0    2    magic      0x54 0x48            "TH"
 2    1    version    0x01
 3    1    flags      bit0 = light mode; bits 1..7 reserved, must be 0
 4    1    n_colors   number of RGB888 slots that follow (v1 senders: 14)
 5    3n   colors     slot 0 .. slot n-1, each r g b
 5+3n ...  tlv*       [tag u8][len u8][value]; tag 0x01 = theme name, UTF-8, ≤ 32 bytes
 end-2 2   crc16      CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection,
                      no xor-out) over bytes [0, end-2). crc16("123456789") = 0x29B1
```

Sizes: header 5 + 14×3 = 47, + name TLV (2 + len) + crc 2. `tokyo-night` = **62 bytes**;
no name = 49 bytes. Hard cap: 240 bytes per write (`THEME_PROTO_MAX_PACKET`).

Receiver algorithm (identical in both implementations, in this order):

1. `len < 7` or `len > 240` → TRUNCATED. Magic mismatch → BAD_MAGIC. `version != 1` → BAD_VERSION.
2. CRC over everything but the last two bytes must match → else BAD_CRC.
3. `5 + 3·n_colors` must fit before the CRC → else TRUNCATED.
4. Apply `min(n_colors, slots the receiver knows)` colours; slots the sender did not send
   keep their previous value (built-in palette or previous theme).
5. Walk TLVs; a TLV that overruns → BAD_TLV; unknown tags are skipped.
6. Persist the raw packet (NVS) and expose the packet's CRC as the acknowledgement token.

### Compatibility rules

* **Newer desktop, older watch**: extra colour slots (`n_colors` > known) and unknown TLV
  tags are ignored. No version bump needed for either.
* **Older desktop, newer watch**: missing slots keep the receiver's defaults. The watch's
  Info characteristic tells the desktop what it supports; the desktop encodes
  `min(host_max, watch_max)` and downgrades on BAD_VERSION.
* A **version bump** is reserved for layout changes (e.g. switching colour encoding).
  v1 receivers reject other versions rather than guessing.

## Status (watch → desktop), 6 bytes, read/notify

```
[version=1][result][applied_crc lo][applied_crc hi][n_applied][flags (bit0 light)]
result: 0 OK, 1 BAD_MAGIC, 2 BAD_VERSION, 3 BAD_CRC, 4 TRUNCATED, 5 BAD_TLV, 6 NO_THEME
```

`applied_crc` is the CRC of the last packet that was *applied*; the desktop matches it
against the packet it just wrote. That is the whole acknowledgement: no sequence numbers,
because a theme push is idempotent and the latest write always wins.

## Info (read), 4 bytes

```
[proto_min][proto_max][max_colors][features]   features: bit0 control, bit1 persist
```

## Control (watch → desktop), 4 bytes, notify

```
[0x54 0x43 "TC"][version=1][op]   op: 1 NEXT_THEME, 2 PREV_THEME, 3 TOGGLE_MODE, 4 RESEND
```

No CRC: the link layer protects notifications, they always fit in one ATT PDU, and every
op is idempotent. The desktop maps them to source-specific actions (for Omarchy:
`omarchy-theme-set <next slug>`), and the resulting theme comes back through the normal
push path — the watch never needs to know theme names or the source's rules.

## Why these choices

*RGB888, not RGB565*: the source is 8-bit per channel, LVGL 9's `lv_color_t` is 8:8:8, and
the difference is 14 bytes (49 vs 35 for the colour block). RGB565 would add a lossy
conversion to save less than one ATT PDU.

*Fixed slot order + count, not TLV per colour*: a colour table is the one part of this
format that is naturally an ordered array, and "append a slot" covers every foreseeable
extension. TLV is used only for the genuinely optional, variable-length extras (name),
where it costs 2 bytes.

*Not CBOR/protobuf*: both would need a parser on a microcontroller for a 62-byte payload
with one string in it; the hand format is 60 lines of C and has no allocation.

*CRC-16*: guards against a truncated long write or a stale MTU assumption, not against
radio corruption (the link layer already does that). Five lines in any language.

*Version negotiation via Info, not handshake*: one extra read on connect; the write itself
still works blind against a receiver with no Info characteristic.

## Transport binding: BLE GATT

```
service         7e450001-5029-4337-8dde-aaefb009b2df   (advertised in the ADV packet)
theme state     7e450002-…   WRITE (with response; long write allowed) | READ (last packet)
status          7e450003-…   READ | NOTIFY
control         7e450004-…   NOTIFY | READ (last request)
info            7e450005-…   READ
device name     in the scan response (e.g. "OW-Watch")
```

Packet flow, cold: scan (filter on the service UUID) → connect → read Info → write
ThemeState with response → read Status → compare `applied_crc` → disconnect. Warm (daemon):
stay connected, subscribed to Status and Control; a theme change is one write + one
notification, well under a second.

MTU: with the default 23-byte ATT MTU a 62-byte write becomes an automatic ATT long write
(prepare/execute), which NimBLE reassembles before calling the application; with any
negotiated MTU ≥ 65 it is a single write. Neither side fragments in application code.
