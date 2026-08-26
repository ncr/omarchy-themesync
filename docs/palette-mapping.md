# Mapping Omarchy's palette onto the watch roles

Implementation: `host/src/palette.rs::map_source`. Everything below is deterministic; the
same `colors.toml` always yields the same packet.

## The one fact that decides the mapping

Omarchy's own docs (`docs/theming.md`): *"The neutral ramp is centered on
background → bright_foreground. Dark themes should read from darkest to lightest; light
themes should read from lightest to darkest."* So the ramp is ordered by **distance from
the background toward the text**, not by luminance, and the key names lie in light themes:

| theme (mode) | background | lighter_background | dark_background | foreground | dark_foreground | muted |
|---|---|---|---|---|---|---|
| tokyo-night (dark) | #1a1b26 | #24283b (lighter) | #13141c (darker) | #a9b1d6 | #565f89 (dimmer) | #414868 |
| catppuccin-latte (light) | #eff1f5 | #dce0e8 (**darker**) | #e3e4e8 (darker) | #4c4f69 | #9ca0b0 (**lighter**) | #acb0be |
| white (light) | #ffffff | #c0c0c0 (**darker**) | #f5f5f5 (darker) | #000000 | #c0c0c0 (**lighter**) | #808080 |

`lighter_background` and `dark_foreground` are ramp-consistent (always "one step from the
background" / "one step from the text toward the background"), so they can be used as
roles directly. `dark_background` is *not*: it is literally darker in both modes, i.e. it
moves *away* from the text in dark themes and *toward* it in light themes, so no watch role
is built on it. Likewise `light_foreground` is literally lighter in both modes. Any mapping
that picks "the lighter of the two backgrounds" by RGB luminance would make light-theme
cards brighter than the screen, which is the inverse of the theme author's intent.

## Rules

| watch role | rule | why |
|---|---|---|
| `background` | `background` | — |
| `surface` | `lighter_background`; if it equals `background` (legacy/ANSI-only themes alias it to `color0`), derive `mix(background, text_primary, 8%)` | first ramp step in both modes |
| `surface_alt` | derived: `mix(surface, text_primary, 10%)` | "one more step up the ramp"; `selection` would be the natural key but it collides with `lighter_background` in some themes (`white`) and is used for its own role |
| `text_primary` | `foreground` | Omarchy: "primary readable text color" (`bright_foreground` is the cursor colour) |
| `text_secondary` | `dark_foreground` if it differs from `foreground` **and** contrasts ≥ 3.0 with `surface`; else `mix(foreground, background, 34%)` | ramp-consistent key, guarded for wrist-distance legibility (tokyo-night's #565f89 on its card is 2.3:1 → derived #787e9a at 3.6:1). 34 % is Omarchy's own `mutedText` derivation in `pi.json.tpl` |
| `text_disabled` | `muted` if it is *less* prominent than `text_secondary` and ≥ 1.2:1; else `mix(text_secondary, background, 40%)` | `white` inverts `muted`/`dark_foreground`; disabled must never out-contrast secondary |
| `accent` | `accent`, else `blue`, else `text_primary` | Omarchy's own fallback chain |
| `on_accent` | derived: the readable one of `background` / `text_primary` on `accent` (≥ 4.5:1, higher wins), else pure black/white by contrast | themed "on" colour (tokyo: #1a1b26 on #7aa2f7 at 6.8:1; latte: white on #1e66f5 at 4.9:1) instead of a hard-coded dark text |
| `selection` | `selection`; if equal to `background`, `mix(background, accent, 30%)` | — |
| `divider` | `muted` if ≥ 1.25:1 against `surface`, else `mix(surface, text_primary, 20%)` | Omarchy uses `muted` for dividers |
| `danger` / `warning` / `success` | `red` / `yellow` (falls back to `orange`) / `green` | the theme author already tuned these per mode (light themes ship darkened reds/yellows) |
| `info` | `blue` unless it equals `accent`, then `cyan` | most Omarchy themes set `accent = blue`; info must stay distinguishable from the accent |
| `mode` | `mode` key → luminance auto-detect (`r+g+b > 382`) | same precedence as `omarchy-theme-color` |

What is deliberately **not** done: no adjustment of the status colours for contrast on
light surfaces (latte's `warning` #df8e1d reads 2.0:1 on its card). Those colours are used
for bars and large indicators, and darkening them would change the theme; `themesync theme
--contrast` prints the numbers so a future `--boost-contrast` derivation can be an
informed choice rather than a silent one.

## Derived on the watch, not sent

`on_warning` (text on a warning button) is picked on the device as the more distant of
`background` / `text_primary` in luma — one label in the current UI did not justify a wire
slot. Adding it later is an append (slot 14) with no compatibility cost.

## Checking a theme

```
themesync theme --file host/tests/fixtures/catppuccin-latte.toml --contrast
```

prints the 14 roles with swatches and the WCAG ratios for the pairs the UI depends on
(text on background/surface, on_accent on accent, accent on background, statuses on
surface). Every first-party Omarchy theme in `host/tests/fixtures` resolves with
`text_primary` ≥ 6:1 on both background and surface and `on_accent` ≥ 4.5:1 on `accent`.
