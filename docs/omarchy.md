# Omarchy theme internals, verified 2026-08-25

Primary source: `github.com/basecamp/omarchy`, default branch **`quattro`** (not `master`,
which is stale v3), HEAD `0ae1694` (2026-08-25), identical to tag **v4.0.1** (released
2026-08-25, "Fast-Follow Fixes" after v4.0.0 "The Quattro Release", 2026-08-14) for every
file cited below. Also cross-checked with a local shallow clone.

## What was right and wrong in the initial assumptions

| assumption | verdict |
|---|---|
| `colors.toml` with `mode`, `accent`, `selection`, `muted`, `background`/`dark_`/`darker_`/`lighter_background`, `foreground`/`dark_`/`light_`/`bright_foreground`, `red yellow orange green cyan blue magenta brown`, `bright_*` | **Correct for v4.0.x.** Bright variants exist for red/yellow/green/cyan/blue/magenta only. Some themes omit keys (`white` has no orange/brown); the resolver derives them. v3.8.x themes had only `accent, cursor, foreground, background, selection_*, color0..15`. |
| `omarchy-theme-color --all` is the programmatic interface | **Correct** (hidden from `omarchy --help`, on PATH; created 2026-07-03). Output is `key<TAB>value`, C-sorted, ~56 keys for tokyo-night. No `--json`. `--file F` resolves any colors.toml. `<key> [fallback]` prints one value. |
| Hooks in `~/.config/omarchy/hooks/theme-set.d/` | **Correct** (since v3.8.0; hooks since v3.1.0). A flat `~/.config/omarchy/hooks/theme-set` runs first. |
| Current theme at `~/.config/omarchy/current/theme` | **Wrong for v4.** It is `~/.local/state/omarchy/current/theme/` — a real directory, atomically `mv`'d in — with the slug in `~/.local/state/omarchy/current/theme.name`. v3 used `~/.config/omarchy/current`. |
| Stock themes under `~/.local/share/omarchy/themes` | **Both exist on v4.** `$OMARCHY_PATH/themes` = `/usr/share/omarchy/themes`, and on the user's box (checked 2026-08-26) `~/.local/share/omarchy/themes` lists the same 22 slugs. `OMARCHY_PATH` is *not* exported to non-login shells or the systemd user manager (`systemctl --user show-environment` has only `PATH` with `/usr/share/omarchy/bin` and `DESKTOP_SESSION=omarchy`), so `themesync` follows `$OMARCHY_PATH` when set, else the `~/.local/share` tree when it has `themes/`, else `/usr/share`. `~/.config/omarchy/themes/<slug>` holds user/installed themes and overlays the stock one. |
| `omarchy-theme-next` | Does not exist. Only `omarchy-theme-bg-next`, `omarchy-theme-switcher`, `omarchy-theme-refresh`. `themesync next/prev` enumerate the theme dirs the way `omarchy-theme-list` does. |

## The resolver (`bin/omarchy-theme-color`)

Parses `colors.toml` line by line (first `=` splits; quotes/spaces stripped from keys;
value = text between the first pair of quotes; keys `[A-Za-z0-9_-]+`, values restricted to
`[A-Za-z0-9#(),._+/% -]` since 2026-08-11), then applies, in order: legacy short names
(`bg`→`background`, …); `background`←`color0`, `foreground`←`color7`; `red`←`color1` …
`bright_cyan`←`color14`, `magenta`←`purple`; `light_foreground`←`color7`|fg,
`bright_foreground`←`color15`|fg, `cursor`=`bright_foreground` (always),
`lighter_background`←`color0`|bg, `dark_foreground`←`color8`|fg, `muted`←`color8`|`dark_foreground`,
`selection`←`selection_background`|`color8`|`color0`|bg, `orange`←`yellow`,
`brown`=mix(orange,#000,50%), `dark_background`=mix(bg,#000,25%), `darker_background`=mix(bg,#000,50%),
`bright_*`=mix(x,#fff,20%); then ANSI aliases `color0..15` from the semantic names and the
short names from the canonical ones. Mode: `mode` key → legacy `theme_type` → a `light.mode`
file → `r+g+b > 382` of `background` → dark. `host/src/omarchy.rs::resolve` is a port of this
cascade (used when the script is not on PATH, i.e. tests and non-Omarchy machines) and
`themesync` prefers the real script when present.

Legacy themes without `colors.toml`: `omarchy-theme-set` generates one from
`alacritty.toml` via `omarchy-theme-colors-from-alacritty` (`accent=color4`, `selection`,
`background`, `foreground`, `color0..15`); for git-installed themes only the generated file
is staged. All 22 stock themes ship `colors.toml`; none has `light.mode` anymore.

## `omarchy-theme-set <name>` sequence

1. slug = lowercase, spaces→`-`; must exist in stock or user themes; `flock` serialises runs.
2. stage into `~/.local/state/omarchy/current/next-theme`: stock copy, user overlay (filtered
   if git-installed), colors.toml from alacritty if missing, `omarchy-theme-set-templates`
   renders `default/themed/*.tpl` and `~/.config/omarchy/themed/*.tpl`.
3. `rm -rf current/theme; mv next-theme current/theme; echo slug > current/theme.name`;
   shell IPC + background transition; lock released.
4. `post_theme_commands` in parallel (terminal/hyprctl/btop/helix restarts, foot/tmux/gnome/
   pi/claude/browser/vscode/obsidian/keyboard setters), **wait** for all.
5. `omarchy-hook theme-set "$THEME_NAME" >/dev/null` — runs `bash ~/.config/omarchy/hooks/theme-set <slug>`
   if present, then `bash` each non-`.sample` file in `theme-set.d/` in glob order. **Synchronous**,
   failures swallowed with "Hook failed", stdout discarded, no dedicated env vars, `$1` = slug.
   Not run when `OMARCHY_THEME_HEADLESS=1`/`OMARCHY_THEME_OFFLINE=1`.
6. `omarchy-theme-switcher --preload`, `omarchy-theme-bg-cache &`.

So a hook sees the *fully* applied theme (directory, name, every app retinted) and must
return quickly — hence `themesync sync --async`.

## What Omarchy recommends for third parties

`docs/theming.md` + `manual/43-making-your-own-theme.md`: templates in
`~/.config/omarchy/themed/*.tpl` for apps that read a config file (rendered at theme-set
time into `current/theme/<name>`), hooks for actions, `omarchy-theme-color` as the shared
resolver "so every consumer resolves the exact same palette". Following a new app
first-party means adding it to `post_theme_commands`. The semantic ramp is documented as
"centered on background → bright_foreground; dark themes read darkest→lightest, light
themes lightest→darkest" — the key fact behind `docs/palette-mapping.md`.

Sources: bin/omarchy-theme-set, bin/omarchy-theme-color, bin/omarchy-hook, bin/omarchy-hook-install,
bin/omarchy-theme-set-templates, bin/omarchy-theme-colors-from-alacritty, docs/theming.md,
docs/file-layout.md, default/agents/skills/omarchy/hooks.md,
config/omarchy/hooks/theme-set.d/show-theme-notification.sample (all on `quattro`),
https://github.com/basecamp/omarchy/releases/tag/v4.0.1, https://omarchy.org/manual/making-your-own-theme/.
