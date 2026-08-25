//! Colour types, the source-side palette vocabulary, the watch-side semantic palette,
//! and the deterministic mapping between them.
//!
//! Two vocabularies live here on purpose:
//!
//! * [`SourcePalette`] is "whatever the desktop knows", keyed by name. The key names are
//!   Omarchy's canonical `colors.toml` keys (`background`, `lighter_background`, `red`, ...)
//!   because that vocabulary is already a sensible superset of pywal / base16 / wallust;
//!   a future non-Omarchy adapter just has to emit those names.
//! * [`WatchPalette`] is the small role-based palette the watch UI consumes
//!   (`background`, `surface`, `text_primary`, `accent`, `danger`, ...). Roles are numbered;
//!   the number is the slot in the wire format, and the list is append-only.
//!
//! [`map_source`] is the only place that knows how one becomes the other.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// 8-bit-per-channel RGB colour. The wire format carries exactly this.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }

    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    pub const WHITE: Rgb = Rgb::new(255, 255, 255);

    /// Accepts `#rrggbb`, `rrggbb`, `0xrrggbb` and the 8-digit variants (alpha is dropped),
    /// which covers everything an Omarchy `colors.toml` or `--all` output can hold.
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim();
        let hex = s
            .strip_prefix('#')
            .or_else(|| s.strip_prefix("0x"))
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(s);
        if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(&hex[..6], 16).ok()?;
        Some(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Linear blend `self * (1 - t) + other * t`, rounded exactly like Omarchy's
    /// `mix_color` (`int(x + 0.5)`), so derived shades match what its templates produce.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let ch = |a: u8, b: u8| ((a as f32) * (1.0 - t) + (b as f32) * t + 0.5).floor() as u8;
        Rgb::new(ch(self.r, other.r), ch(self.g, other.g), ch(self.b, other.b))
    }

    /// WCAG 2.x relative luminance (0 = black, 1 = white).
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// WCAG contrast ratio, 1.0 ..= 21.0.
    pub fn contrast(self, other: Rgb) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The panel's native format; only used for size comparisons and debugging output.
    pub fn to_rgb565(self) -> u16 {
        ((self.r as u16 & 0xF8) << 8) | ((self.g as u16 & 0xFC) << 3) | (self.b as u16 >> 3)
    }
}

impl fmt::Debug for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl From<Rgb> for String {
    fn from(c: Rgb) -> String {
        c.to_hex()
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;
    fn try_from(s: String) -> Result<Rgb, String> {
        Rgb::parse(&s).ok_or_else(|| format!("not a colour: {s:?}"))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    /// Omarchy's own auto-detect (`omarchy-theme-color`): `r + g + b > 382` means light.
    pub fn from_background(bg: Rgb) -> Mode {
        if bg.r as u32 + bg.g as u32 + bg.b as u32 > 382 {
            Mode::Light
        } else {
            Mode::Dark
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }
}

/// A resolved desktop palette, source-agnostic. Keys use Omarchy's canonical names.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourcePalette {
    /// Theme slug, e.g. `tokyo-night`, if the source knows it.
    pub name: Option<String>,
    /// Explicit mode if the source declares one; otherwise derived from `background`.
    pub mode: Option<Mode>,
    pub colors: BTreeMap<String, Rgb>,
    /// Non-colour values the source exposed (gradients, `theme_type`, ...). Kept for debugging.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: BTreeMap<String, String>,
}

impl SourcePalette {
    pub fn get(&self, key: &str) -> Option<Rgb> {
        self.colors.get(key).copied()
    }

    pub fn mode(&self) -> Option<Mode> {
        self.mode.or_else(|| self.get("background").map(Mode::from_background))
    }
}

/// Semantic roles the watch UI paints with. The discriminant is the wire slot; never
/// reorder, only append (and bump `Role::COUNT`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Role {
    Background = 0,
    Surface = 1,
    SurfaceAlt = 2,
    TextPrimary = 3,
    TextSecondary = 4,
    TextDisabled = 5,
    Accent = 6,
    OnAccent = 7,
    Selection = 8,
    Divider = 9,
    Danger = 10,
    Warning = 11,
    Success = 12,
    Info = 13,
}

impl Role {
    pub const COUNT: usize = 14;
    pub const ALL: [Role; Role::COUNT] = [
        Role::Background,
        Role::Surface,
        Role::SurfaceAlt,
        Role::TextPrimary,
        Role::TextSecondary,
        Role::TextDisabled,
        Role::Accent,
        Role::OnAccent,
        Role::Selection,
        Role::Divider,
        Role::Danger,
        Role::Warning,
        Role::Success,
        Role::Info,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Background => "background",
            Role::Surface => "surface",
            Role::SurfaceAlt => "surface_alt",
            Role::TextPrimary => "text_primary",
            Role::TextSecondary => "text_secondary",
            Role::TextDisabled => "text_disabled",
            Role::Accent => "accent",
            Role::OnAccent => "on_accent",
            Role::Selection => "selection",
            Role::Divider => "divider",
            Role::Danger => "danger",
            Role::Warning => "warning",
            Role::Success => "success",
            Role::Info => "info",
        }
    }

    pub fn from_slot(slot: u8) -> Option<Role> {
        Role::ALL.get(slot as usize).copied()
    }
}

/// What the watch receives: a mode, a display name, and one colour per role.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WatchPalette {
    pub mode: Mode,
    /// Display name, at most [`WatchPalette::MAX_NAME_BYTES`] bytes of UTF-8.
    pub name: String,
    pub colors: [Rgb; Role::COUNT],
}

impl WatchPalette {
    pub const MAX_NAME_BYTES: usize = 32;

    pub fn get(&self, role: Role) -> Rgb {
        self.colors[role as usize]
    }

    pub fn set(&mut self, role: Role, c: Rgb) {
        self.colors[role as usize] = c;
    }

    /// Truncate on a char boundary so the wire form never carries a split code point.
    pub fn clamp_name(name: &str) -> String {
        let mut out = String::new();
        for ch in name.chars() {
            if out.len() + ch.len_utf8() > Self::MAX_NAME_BYTES {
                break;
            }
            out.push(ch);
        }
        out
    }

    /// The human-readable form (`omawatch theme --json`): flat, one key per role.
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("version".into(), serde_json::Value::from(crate::protocol::VERSION));
        m.insert("mode".into(), serde_json::Value::from(self.mode.as_str()));
        m.insert("name".into(), serde_json::Value::from(self.name.clone()));
        for role in Role::ALL {
            m.insert(role.name().into(), serde_json::Value::from(self.get(role).to_hex()));
        }
        serde_json::Value::Object(m)
    }

    /// Contrast ratios the UI actually depends on; used by `omawatch theme --contrast`
    /// and by the tests that keep light themes honest.
    pub fn contrast_report(&self) -> Vec<(Role, Role, f32)> {
        let pairs = [
            (Role::TextPrimary, Role::Background),
            (Role::TextPrimary, Role::Surface),
            (Role::TextSecondary, Role::Surface),
            (Role::TextDisabled, Role::Surface),
            (Role::OnAccent, Role::Accent),
            (Role::Accent, Role::Background),
            (Role::Surface, Role::Background),
            (Role::SurfaceAlt, Role::Surface),
            (Role::Danger, Role::Surface),
            (Role::Warning, Role::Surface),
            (Role::Success, Role::Surface),
            (Role::Info, Role::Surface),
        ];
        pairs
            .iter()
            .map(|&(a, b)| (a, b, self.get(a).contrast(self.get(b))))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    MissingKey(&'static str),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::MissingKey(k) => write!(f, "source palette has no `{k}` colour"),
        }
    }
}

impl std::error::Error for MapError {}

/// Step size (toward `text_primary`) used when a surface has to be derived.
const SURFACE_STEP: f32 = 0.08;
const SURFACE_ALT_STEP: f32 = 0.10;
/// Omarchy's own "muted text" derivation (`pi.json.tpl`: `mix foreground background 34%`).
const SECONDARY_TEXT_MIX: f32 = 0.34;
const MIN_SECONDARY_CONTRAST: f32 = 3.0;
const MIN_DIVIDER_CONTRAST: f32 = 1.25;

/// Turn a resolved source palette into the watch palette.
///
/// The rules, and why (see `docs/palette-mapping.md` for the long version):
///
/// * Omarchy's neutral ramp is ordered *by distance from `background` toward the
///   foreground*, not by luminance. `lighter_background` is the first step of that ramp in
///   **both** modes (in `catppuccin-latte` it is `#dce0e8`, darker than the `#eff1f5`
///   background), so it is the card/surface colour regardless of mode. `dark_background`
///   is *not* ramp-consistent (it is literally darker in light themes too, i.e. it moves
///   toward the text there and away from it in dark themes), so no UI role is built on it.
/// * `dark_foreground` and `muted` *are* ramp-consistent (both move from the foreground
///   toward the background in every first-party theme), so they become secondary and
///   disabled text, with a contrast guard and a derived fallback.
/// * `on_accent` is derived, not copied: it is whichever of `background` / `text_primary`
///   reads best on the accent, falling back to pure black/white.
/// * Semantic statuses copy the named ANSI-ish colours the theme author already tuned for
///   the mode (light themes ship darkened reds/yellows), with `info` avoiding the accent.
pub fn map_source(src: &SourcePalette) -> Result<WatchPalette, MapError> {
    let background = src.get("background").ok_or(MapError::MissingKey("background"))?;
    let foreground = src.get("foreground").ok_or(MapError::MissingKey("foreground"))?;
    let mode = src.mode().unwrap_or(Mode::Dark);

    let text_primary = foreground;

    // --- surfaces ---------------------------------------------------------------------
    let surface = match src.get("lighter_background") {
        Some(c) if c != background => c,
        _ => background.mix(text_primary, SURFACE_STEP),
    };
    // One more step up the ramp from the surface. Derived rather than copied from
    // `selection`, because `selection` collides with `lighter_background` in some themes
    // (`white`) and is used for a different role below.
    let surface_alt = surface.mix(text_primary, SURFACE_ALT_STEP);

    // --- text -------------------------------------------------------------------------
    let derived_secondary = foreground.mix(background, SECONDARY_TEXT_MIX);
    let text_secondary = match src.get("dark_foreground") {
        Some(c) if c != foreground && c.contrast(surface) >= MIN_SECONDARY_CONTRAST => c,
        _ => derived_secondary,
    };
    // Disabled text must be *less* prominent than secondary text; `muted` normally is,
    // but not in every theme (`white` inverts them), so check and derive if needed.
    let text_disabled = match src.get("muted") {
        Some(c) if c.contrast(surface) < text_secondary.contrast(surface) && c.contrast(surface) >= 1.2 => c,
        _ => text_secondary.mix(background, 0.40),
    };

    // --- accent -----------------------------------------------------------------------
    let accent = src
        .get("accent")
        .or_else(|| src.get("blue"))
        .unwrap_or(text_primary);
    let on_accent = {
        let candidates = [background, text_primary, Rgb::BLACK, Rgb::WHITE];
        // Prefer the themed candidates when they are readable; otherwise pure black/white.
        let themed = candidates[..2]
            .iter()
            .copied()
            .filter(|c| c.contrast(accent) >= 4.5)
            .max_by(|a, b| a.contrast(accent).total_cmp(&b.contrast(accent)));
        themed.unwrap_or_else(|| {
            candidates[2..]
                .iter()
                .copied()
                .max_by(|a, b| a.contrast(accent).total_cmp(&b.contrast(accent)))
                .unwrap()
        })
    };

    let selection = match src.get("selection") {
        Some(c) if c != background => c,
        _ => background.mix(accent, 0.30),
    };

    let divider = match src.get("muted") {
        Some(c) if c.contrast(surface) >= MIN_DIVIDER_CONTRAST => c,
        _ => surface.mix(text_primary, 0.20),
    };

    // --- statuses ---------------------------------------------------------------------
    let danger = src.get("red").unwrap_or(accent);
    let warning = src.get("yellow").or_else(|| src.get("orange")).unwrap_or(accent);
    let success = src.get("green").unwrap_or(accent);
    let info = match (src.get("blue"), src.get("cyan")) {
        (Some(b), _) if b != accent => b,
        (_, Some(c)) => c,
        (Some(b), None) => b,
        (None, None) => accent,
    };

    let mut colors = [Rgb::default(); Role::COUNT];
    colors[Role::Background as usize] = background;
    colors[Role::Surface as usize] = surface;
    colors[Role::SurfaceAlt as usize] = surface_alt;
    colors[Role::TextPrimary as usize] = text_primary;
    colors[Role::TextSecondary as usize] = text_secondary;
    colors[Role::TextDisabled as usize] = text_disabled;
    colors[Role::Accent as usize] = accent;
    colors[Role::OnAccent as usize] = on_accent;
    colors[Role::Selection as usize] = selection;
    colors[Role::Divider as usize] = divider;
    colors[Role::Danger as usize] = danger;
    colors[Role::Warning as usize] = warning;
    colors[Role::Success as usize] = success;
    colors[Role::Info as usize] = info;

    Ok(WatchPalette {
        mode,
        name: WatchPalette::clamp_name(src.name.as_deref().unwrap_or("")),
        colors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Rgb::parse("#1a1b26"), Some(Rgb::new(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("1A1B26"), Some(Rgb::new(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("0x1a1b26"), Some(Rgb::new(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("#1a1b26ff"), Some(Rgb::new(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("#1a1b2"), None);
        assert_eq!(Rgb::parse("rgba(1,2,3,0.5)"), None);
        assert_eq!(Rgb::parse("#zz1b26"), None);
    }

    #[test]
    fn mix_matches_omarchy_awk_rounding() {
        // omarchy-theme-color: brown = mix(orange, #000000, 50%), tokyo-night orange #eb927b
        // -> int(0xeb*0.5+0.5)=118=0x76, int(0x92*0.5+0.5)=73=0x49, int(0x7b*0.5+0.5)=62=0x3e
        assert_eq!(Rgb::parse("#eb927b").unwrap().mix(Rgb::BLACK, 0.5), Rgb::parse("#76493e").unwrap());
        // dark_background = mix(background, #000, 25%) for a theme without one
        assert_eq!(Rgb::parse("#282828").unwrap().mix(Rgb::BLACK, 0.25), Rgb::parse("#1e1e1e").unwrap());
        assert_eq!(Rgb::WHITE.mix(Rgb::BLACK, 0.0), Rgb::WHITE);
        assert_eq!(Rgb::WHITE.mix(Rgb::BLACK, 1.0), Rgb::BLACK);
    }

    #[test]
    fn contrast_is_symmetric_and_bounded() {
        assert!((Rgb::BLACK.contrast(Rgb::WHITE) - 21.0).abs() < 0.01);
        assert!((Rgb::WHITE.contrast(Rgb::BLACK) - 21.0).abs() < 0.01);
        assert!((Rgb::WHITE.contrast(Rgb::WHITE) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mode_autodetect_matches_omarchy_threshold() {
        assert_eq!(Mode::from_background(Rgb::new(127, 127, 128)), Mode::Dark); // 382
        assert_eq!(Mode::from_background(Rgb::new(127, 128, 128)), Mode::Light); // 383
    }

    #[test]
    fn rgb565_packing() {
        assert_eq!(Rgb::WHITE.to_rgb565(), 0xFFFF);
        assert_eq!(Rgb::new(0xF8, 0, 0).to_rgb565(), 0xF800);
        assert_eq!(Rgb::new(0, 0xFC, 0).to_rgb565(), 0x07E0);
        assert_eq!(Rgb::new(0, 0, 0xF8).to_rgb565(), 0x001F);
    }

    #[test]
    fn name_clamps_on_char_boundary() {
        let long = "ż".repeat(40); // 2 bytes each
        let clamped = WatchPalette::clamp_name(&long);
        assert_eq!(clamped.len(), 32);
        assert_eq!(clamped.chars().count(), 16);
    }

    fn fixture_palette(name: &str) -> WatchPalette {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path).unwrap();
        let mut src = crate::omarchy::resolve(crate::omarchy::parse_colors_toml(&text), None);
        src.name = Some(name.to_string());
        map_source(&src).unwrap()
    }

    #[test]
    fn first_party_themes_map_legibly_in_both_modes() {
        for name in ["tokyo-night", "gruvbox", "nord", "catppuccin-latte", "flexoki-light", "white", "rose-pine"] {
            let p = fixture_palette(name);
            let c = |a: Role, b: Role| p.get(a).contrast(p.get(b));
            assert!(c(Role::TextPrimary, Role::Background) >= 6.0, "{name}: text on background {}", c(Role::TextPrimary, Role::Background));
            assert!(c(Role::TextPrimary, Role::Surface) >= 6.0, "{name}: text on surface {}", c(Role::TextPrimary, Role::Surface));
            assert!(c(Role::TextSecondary, Role::Surface) >= 2.5, "{name}: secondary on surface {}", c(Role::TextSecondary, Role::Surface));
            assert!(c(Role::TextDisabled, Role::Surface) < c(Role::TextSecondary, Role::Surface), "{name}: disabled must be dimmer than secondary");
            assert!(c(Role::OnAccent, Role::Accent) >= 4.5, "{name}: on_accent on accent {}", c(Role::OnAccent, Role::Accent));
            assert_ne!(p.get(Role::Surface), p.get(Role::Background), "{name}: surface must differ from background");
            assert_ne!(p.get(Role::SurfaceAlt), p.get(Role::Surface), "{name}: surface_alt must differ from surface");
            assert_ne!(p.get(Role::Info), p.get(Role::Accent), "{name}: info must differ from accent");
        }
    }

    #[test]
    fn light_themes_step_toward_the_text_not_toward_white() {
        // The ramp is "away from background toward text" in both modes: in a light theme the
        // card is darker than the screen, and secondary text is lighter than primary.
        for name in ["catppuccin-latte", "flexoki-light", "white"] {
            let p = fixture_palette(name);
            assert_eq!(p.mode, Mode::Light, "{name}");
            assert!(p.get(Role::Surface).luminance() < p.get(Role::Background).luminance(), "{name}: surface must be darker than background");
            assert!(p.get(Role::SurfaceAlt).luminance() < p.get(Role::Surface).luminance(), "{name}: surface_alt darker still");
            assert!(p.get(Role::TextSecondary).luminance() > p.get(Role::TextPrimary).luminance(), "{name}: secondary text lighter than primary");
        }
        for name in ["tokyo-night", "gruvbox", "nord"] {
            let p = fixture_palette(name);
            assert_eq!(p.mode, Mode::Dark, "{name}");
            assert!(p.get(Role::Surface).luminance() > p.get(Role::Background).luminance(), "{name}: surface lighter than background");
            assert!(p.get(Role::TextSecondary).luminance() < p.get(Role::TextPrimary).luminance(), "{name}: secondary text dimmer than primary");
        }
        // catppuccin-latte: lighter_background (#dce0e8) is darker than background (#eff1f5)
        // and is still the surface — a luminance-based pick would have inverted the card.
        let latte = fixture_palette("catppuccin-latte");
        assert_eq!(latte.get(Role::Surface), Rgb::parse("#dce0e8").unwrap());
        // background #eff1f5 reads only 4.3:1 on the #1e66f5 accent (below the 4.5 gate), so
        // on_accent falls through to pure white (4.9:1) rather than the themed candidate.
        assert_eq!(latte.get(Role::OnAccent), Rgb::WHITE);
    }

    #[test]
    fn map_requires_background_and_foreground() {
        let mut src = SourcePalette::default();
        assert_eq!(map_source(&src).unwrap_err(), MapError::MissingKey("background"));
        src.colors.insert("background".into(), Rgb::BLACK);
        assert_eq!(map_source(&src).unwrap_err(), MapError::MissingKey("foreground"));
        src.colors.insert("foreground".into(), Rgb::WHITE);
        let p = map_source(&src).unwrap();
        // Everything else is derived, nothing is left at the default (black) except where
        // black is the legitimate answer.
        assert_eq!(p.get(Role::Background), Rgb::BLACK);
        assert_eq!(p.get(Role::TextPrimary), Rgb::WHITE);
        assert_ne!(p.get(Role::Surface), Rgb::BLACK);
        assert_ne!(p.get(Role::SurfaceAlt), p.get(Role::Surface));
        assert_eq!(p.get(Role::Accent), Rgb::WHITE); // no accent, no blue -> text colour
        assert_eq!(p.get(Role::OnAccent), Rgb::BLACK);
        assert_eq!(p.mode, Mode::Dark);
    }
}
