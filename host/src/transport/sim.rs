//! The simulated watch: a receiver that runs the same decode + "apply" logic the firmware
//! runs, in-process, and prints what the watch UI would paint with. Lets the whole host
//! chain (resolver -> mapping -> packet -> receiver -> semantic palette) be exercised
//! without hardware. `watch/sim/` is the C twin of this, compiled from the firmware's decoder.

use crate::palette::{Mode, Rgb, Role, WatchPalette};
use crate::protocol::{decode_theme, DecodeError, DecodedTheme};

/// What a freshly flashed watch shows before it ever received a theme: the firmware's
/// built-in dark palette (mirrors `THEME_BUILTIN` in `watch/esp32-lvgl/theme.c`).
pub fn builtin_watch_theme() -> WatchPalette {
    let mut p = WatchPalette { mode: Mode::Dark, name: "builtin".into(), colors: [Rgb::default(); Role::COUNT] };
    let set = |p: &mut WatchPalette, r: Role, hex: &str| p.set(r, Rgb::parse(hex).unwrap());
    set(&mut p, Role::Background, "#0a0b10");
    set(&mut p, Role::Surface, "#161922");
    set(&mut p, Role::SurfaceAlt, "#2a2f3a");
    set(&mut p, Role::TextPrimary, "#e6e8ee");
    set(&mut p, Role::TextSecondary, "#8b90a0");
    set(&mut p, Role::TextDisabled, "#5a5f6e");
    set(&mut p, Role::Accent, "#00e676");
    set(&mut p, Role::OnAccent, "#00110a");
    set(&mut p, Role::Selection, "#1f3a2c");
    set(&mut p, Role::Divider, "#2a2f3a");
    set(&mut p, Role::Danger, "#ff5252");
    set(&mut p, Role::Warning, "#ffab00");
    set(&mut p, Role::Success, "#00e676");
    set(&mut p, Role::Info, "#40c4ff");
    p
}

/// A stateful fake watch: holds "NVS" (the last applied palette) like the real one.
#[derive(Debug, Clone)]
pub struct SimWatch {
    pub current: WatchPalette,
    pub last_crc: Option<u16>,
}

impl Default for SimWatch {
    fn default() -> Self {
        SimWatch { current: builtin_watch_theme(), last_crc: None }
    }
}

impl SimWatch {
    /// Exactly what the firmware does on a Theme State write.
    pub fn receive(&mut self, packet: &[u8]) -> Result<&WatchPalette, DecodeError> {
        let decoded: DecodedTheme = decode_theme(packet)?;
        self.last_crc = Some(decoded.crc);
        self.current = decoded.into_palette(&self.current);
        Ok(&self.current)
    }
}

/// Terminal rendering of a palette with true-colour swatches (when `ansi` is on).
pub fn render_palette(p: &WatchPalette, ansi: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!("theme: {}   mode: {}\n", if p.name.is_empty() { "(unnamed)" } else { &p.name }, p.mode.as_str()));
    for role in Role::ALL {
        let c = p.get(role);
        if ansi {
            let bg = p.get(Role::Background);
            let surface = p.get(Role::Surface);
            // swatch, then the colour used as text on the surface, so the reader sees the
            // actual UI pairing rather than a bare chip
            s.push_str(&format!(
                "  {:<15} {}  \x1b[48;2;{};{};{}m      \x1b[0m  \x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m Aa \x1b[0m\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m Aa \x1b[0m\n",
                role.name(),
                c.to_hex(),
                c.r, c.g, c.b,
                bg.r, bg.g, bg.b, c.r, c.g, c.b,
                surface.r, surface.g, surface.b, c.r, c.g, c.b,
            ));
        } else {
            s.push_str(&format!("  {:<15} {}\n", role.name(), c.to_hex()));
        }
    }
    s
}

pub fn render_contrast(p: &WatchPalette) -> String {
    let mut s = String::from("contrast (WCAG ratio; 4.5 = AA text, 3.0 = large text/UI):\n");
    for (a, b, ratio) in p.contrast_report() {
        let mark = if ratio >= 4.5 { "ok " } else if ratio >= 3.0 { "ok-" } else { "LOW" };
        s.push_str(&format!("  {mark} {:>5.2}  {} on {}\n", ratio, a.name(), b.name()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_theme;

    #[test]
    fn sim_applies_and_keeps_state() {
        let mut w = SimWatch::default();
        assert_eq!(w.current.name, "builtin");
        let mut p = builtin_watch_theme();
        p.name = "x".into();
        p.mode = Mode::Light;
        p.set(Role::Accent, Rgb::new(1, 2, 3));
        let bytes = encode_theme(&p);
        let applied = w.receive(&bytes).unwrap().clone();
        assert_eq!(applied, p);
        assert!(w.last_crc.is_some());
        // a bad packet leaves the previous theme untouched
        let err = w.receive(&bytes[..10]).unwrap_err();
        assert!(matches!(err, DecodeError::BadCrc { .. } | DecodeError::Truncated));
        assert_eq!(w.current, p);
    }
}
