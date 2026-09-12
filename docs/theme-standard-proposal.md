# Proposal: on-demand binary themes for small devices

Discussion draft · 2026-09-12 · Not implemented or an adopted standard.

A wearable, small display or lighting device requests a compact palette when it
needs one, stores it locally and decides how to render it. The provider may read
Omarchy, another theme source or a manually configured palette. Devices need no
knowledge of Omarchy, desktop applications or theme names.

The scope is a small semantic palette and conditional request/response exchange
for constrained links, including BLE and LoRa-based transports. No background
broadcast, subscription, desktop control, theme catalog or periodic theme polling
is required. A desktop theme change only updates the provider's cached response.

## Device-driven exchange

```text
device                                      provider
  user requests refresh
  GET(profile, encoding, cached tag) -------> compare with current palette
                                     <----- DATA(tag, colors) or UNCHANGED
  validate, store if changed, render locally
  radio returns to its normal sleep policy
```

Provisioning, an explicit refresh action or a product-defined need can initiate a
pull. Waking the display does not automatically require a radio exchange. A device
can append a theme request to an already planned uplink if its transport supports
multiplexing. It can stay offline indefinitely with its last accepted palette.

Timeout means “no answer”; it must never mean “unchanged”. Keep the old palette,
end the receive window and use a bounded retry policy. An unchanged response is
useful feedback, but on a constrained network it still costs a downlink.

## Shared meaning, local rendering

Proposed fixed profiles:

| Slot | Basic: 3 colors | Status: 6 colors |
|---|---|---|
| 0 | Background | Background |
| 1 | Foreground | Foreground |
| 2 | Accent | Accent |
| 3 | — | Error |
| 4 | — | Warning |
| 5 | — | Success |

Basic is the minimum profile. Status extends it for devices that want theme-based
status colors. Each profile has a fixed order and length; no per-color names, tags
or lengths travel over the radio. Slot meanings stay fixed across implementations.
New role sets need new profile identifiers, not a change to an existing profile.

The receiver owns layout, fonts, shapes, animation, secondary text, surfaces and
contrast adjustments. A watch can derive cards and buttons; a monochrome display
can quantize to its available tones; a single LED can use only the accent.
Receivers may ignore roles they do not use. They must not redefine a shared role:
error still means error, even if the device renders it as an icon or a blink.

Locally meaningful or safety-relevant indications keep their device-defined
behavior. A theme must not turn a required warning into an indistinguishable
normal state. Device-side contrast adjustment is allowed and expected; identical
pixels across devices are not a conformance goal.

## Palette encoding

Start with **RGB565** for minimum bytes, with explicit RGB888 support for devices
that need the original color precision. The requester selects the encoding and
profile, so no separate capability handshake is needed.

| Profile | RGB565 palette | RGB888 palette |
|---|---:|---:|
| Basic, 3 colors | 6 B | 9 B |
| Status, 6 colors | 12 B | 18 B |

RGB565 is lossy: red and blue have 5 bits, green has 6. Define its byte order and
conversion exactly. Proposed conversion from 8-bit sRGB channels:

```text
r5 = (r * 31 + 127) / 255    # integer division
g6 = (g * 63 + 127) / 255
b5 = (b * 31 + 127) / 255
word = (r5 << 11) | (g6 << 5) | b5
wire = [word >> 8, word & 255]   # most significant byte first
```

A decoder expands with nearest-integer scaling back to 0..255. This describes
sRGB code values, not linear-light color mixing. RGB888 sends R, G, B in order.
Light/dark mode is explicit metadata and does not command a particular rendering.

No strings, wallpaper, fonts, timestamps or executable content belong in the core
payload. No compression dictionary or delta state is needed for 6–18 color bytes.
A full palette on change is cheap and recovers cleanly after missed responses.

## Candidate compact framing

These are proposed application-message sizes, excluding radio, network, routing
and security overhead. Field assignments are a sketch to validate, not a frozen
wire specification.

```text
GET        [control:1][request:1][format:1][known_tag:4]              7 B
DATA       [control:1][request:1][format:1][tag:4][colors:6..18]     13–25 B
UNCHANGED  [control:1][request:1]                                   2 B
ERROR      [control:1][request:1][reason:1]                          3 B
```

`control` packs version and message kind. `format` packs profile, encoding and,
in DATA, light/dark mode. GET leaves the mode bit clear. `request` correlates the
response with the one outstanding request. A DATA response must match the requested
profile and encoding; unsupported requests get ERROR rather than a guessed format.
Validate the exact length before applying a complete palette atomically.

A 3-color RGB565 DATA response is **13 B**; 6 colors are **19 B**. RGB888 adds
3 B or 6 B respectively. An unchanged exchange is 7 B up and 2 B down at this
application layer. An unconditional refresh uses `known_tag = 0` and always gets
DATA when the provider can answer.

The 32-bit cache tag identifies the exact representation: profile, encoding,
mode and encoded colors. It is scoped to a configured provider and is not a theme
name, authentication code or freshness proof. Rebinding clears the cache. Select
and document a nonzero tag allocation/hash scheme before freezing the format;
a finite hash can collide, so an explicit forced refresh must remain available.

The one-byte request field only correlates short exchanges. It does not stop
replays or arbitrarily delayed responses. Each transport binding must define a
bounded response lifetime, identifier reuse and its session/freshness protection.
Bindings with existing request correlation, such as CoAP, can carry the palette
body and reuse that protocol's envelope instead of duplicating all these fields.

## Transport bindings and energy

**BLE:** a short request and response on demand, for example over a GATT service.
No continuous theme advertisements or theme scanning are part of this proposal.
BLE connection/discovery overhead must be measured separately from payload bytes.

**Raw LoRa:** the application supplies addressing, request/receive timing,
framing, integrity, authentication and replay handling as needed by the deployment.
A device transmits, waits for a bounded response window, then sleeps. LoRa radio
modulation alone does not define those application behaviors.

**LoRaWAN Class A:** the request is an uplink and the answer is a downlink in an
available receive window. Class A offers receive opportunities following uplinks;
application-server and queue latency can mean the answer misses that opportunity.
The binding must account for this without assuming an always-listening receiver
or immediate replies. See the [Class A documentation](https://www.thethingsnetwork.org/docs/lorawan/classes/).

For example, EU863-870 DR0–DR2 allow 51 B of application payload in the cited
regional table without FOpts. Limits vary with region, data rate and options;
51 B is not a universal promise. A binding must check its actual payload budget
including its own overhead and use a smaller supported profile or fail explicitly
if it will not fit. Core v1 should require no fragmentation.
[LoRa Alliance RP002-1.0.5, EU863-870 tables](https://read.uberflip.com/i/1540208/36)

Do not translate the byte savings directly into battery-life claims. Measure
number of exchanges, receive time, startup overhead and airtime at the chosen
radio settings. Piggybacking on an existing exchange can save more energy than
removing another byte. Retries must respect transport and deployment limits.

The core does not duplicate transport checksums or authentication fields by
default. A binding must state what protects its messages; neither a cache tag nor
RGB data provides that protection. Security and routing bytes count in the real
budget even when they are outside the table above.

## Existing work relevant to this narrower scope

- Our [deployed BLE beacon](../protocol/BEACON.md) already transports binary
  semantic colors, but uses repeated push advertisements. Reuse palette mapping
  experience and codecs where suitable; it is not this pull-only exchange.
- Our [historical Theme Protocol v1](../protocol/THEME_PROTOCOL.md) used fixed
  RGB888 slots. Its 14-role palette and framing are not the proposed compact
  profiles. Do not silently reinterpret either existing protocol's bytes.
- [CoAP RFC 7252](https://www.rfc-editor.org/info/rfc7252/) already provides GET,
  representation validation using ETags, and a short Valid response. Reuse those
  mechanisms on devices that already have CoAP. The proposed custom exchange is
  only a candidate for transports where a dedicated small envelope is useful.
- [SenML RFC 8428](https://www.rfc-editor.org/info/rfc8428/) provides sensor/device
  parameter representations including CBOR; [Cayenne LPP](https://www.thethingsindustries.com/docs/integrations/payload-formatters/cayenne/)
  is another binary IoT payload precedent. These establish that compact device
  data is existing practice; neither reviewed description specifies these theme
  roles and this pull exchange.
- [Tinted Theming](https://github.com/tinted-theming/home) and the Omarchy SDKs
  found in the earlier review are useful palette sources, not required device
  dependencies. A provider converts their output to the small profiles.

This targeted review did not establish an already adopted standard for this exact
combination. It also does not establish novelty: conditional pulls, binary colors
and local rendering are all existing techniques.

## First implementation to validate

1. Freeze the Basic RGB565 body, request/response behavior and cache-tag rules.
2. Write matching small C and Rust codecs with common byte vectors, length checks,
   invalid-format handling and duplicate/stale-response cases.
3. Implement an Omarchy provider and a tiny receiver that renders locally. Include
   a provider backed by fixed colors to prove Omarchy is optional.
4. Exercise refresh, unchanged, timeout, provider restart, cache reset and offline
   rendering. Measure the complete exchange on one constrained radio binding.
5. Add Status and RGB888 only after confirming the actual device need and budget.

The current daemon and watch firmware remain unchanged by this discussion draft.
A deployed protocol change needs its own version and coordinated implementation.
