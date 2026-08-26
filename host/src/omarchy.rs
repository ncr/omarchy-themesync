//! The Omarchy source adapter: where the current theme lives, how to resolve its palette,
//! and how to change it (for the watch -> desktop direction).
//!
//! Verified against basecamp/omarchy `quattro` HEAD (v4.0.1, 2026-08-25):
//!
//! * the active theme is a real directory `~/.local/state/omarchy/current/theme/`, its slug
//!   is in `~/.local/state/omarchy/current/theme.name` (v3.x used `~/.config/omarchy/current`);
//! * `omarchy-theme-color [--file F] --all` prints `key<TAB>value`, C-sorted, after the
//!   alias/derivation cascade; it is the shared resolver every Omarchy consumer uses, so it
//!   is what we call when it is on `PATH`;
//! * when it is not (tests, non-Omarchy machines, v3 installs) [`resolve`] is a line-for-line
//!   port of that cascade, so both paths yield the same palette;
//! * `omarchy-theme-set <name>` applies a theme (name or slug), and fires
//!   `~/.config/omarchy/hooks/theme-set.d/*` synchronously with the slug in `$1` afterwards.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::palette::{Mode, Rgb, SourcePalette};

/// Filesystem layout of an Omarchy install, resolved from the environment.
#[derive(Debug, Clone)]
pub struct Omarchy {
    pub home: PathBuf,
    /// `~/.local/state/omarchy/current` (v4) — falls back to `~/.config/omarchy/current` (v3).
    pub current_dir: PathBuf,
    pub user_themes: PathBuf,
    /// `$OMARCHY_PATH/themes`; without the variable (a systemd user service does not get the
    /// shell's exports) `~/.local/share/omarchy/themes` when that exists, else
    /// `/usr/share/omarchy/themes`.
    pub system_themes: PathBuf,
}

impl Omarchy {
    pub fn from_env() -> Result<Omarchy> {
        let home = std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")?;
        let omarchy_path = std::env::var_os("OMARCHY_PATH").map(PathBuf::from);
        Ok(Omarchy::at(home, omarchy_path))
    }

    /// The layout under `home`, with `omarchy_path` = `$OMARCHY_PATH` if set.
    pub fn at(home: PathBuf, omarchy_path: Option<PathBuf>) -> Omarchy {
        let v4 = home.join(".local/state/omarchy/current");
        let v3 = home.join(".config/omarchy/current");
        let current_dir = if v4.exists() || !v3.exists() { v4 } else { v3 };
        let omarchy_path = omarchy_path.unwrap_or_else(|| {
            let local = home.join(".local/share/omarchy");
            if local.join("themes").is_dir() { local } else { PathBuf::from("/usr/share/omarchy") }
        });
        Omarchy {
            user_themes: home.join(".config/omarchy/themes"),
            system_themes: omarchy_path.join("themes"),
            current_dir,
            home,
        }
    }

    pub fn colors_file(&self) -> PathBuf {
        self.current_dir.join("theme/colors.toml")
    }

    pub fn theme_name_file(&self) -> PathBuf {
        self.current_dir.join("theme.name")
    }

    pub fn hooks_dir(&self) -> PathBuf {
        self.home.join(".config/omarchy/hooks/theme-set.d")
    }

    /// The active theme slug (`tokyo-night`), if Omarchy has ever set one.
    pub fn current_theme_name(&self) -> Option<String> {
        fs::read_to_string(self.theme_name_file())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Resolve the active theme's palette.
    pub fn load_current(&self) -> Result<SourcePalette> {
        let file = self.colors_file();
        let mut p = load_file(&file)
            .with_context(|| format!("resolving the active Omarchy theme from {}", file.display()))?;
        if p.name.is_none() {
            p.name = self.current_theme_name();
        }
        Ok(p)
    }

    /// All installed theme slugs, user + system, sorted and de-duplicated — the same set and
    /// order `omarchy-theme-list` prints (it Title-Cases them; `omarchy-theme-set` accepts both).
    pub fn list_themes(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for dir in [&self.user_themes, &self.system_themes] {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    if let Some(n) = path.file_name().and_then(|n| n.to_str()) {
                        if !n.starts_with('.') {
                            names.push(n.to_string());
                        }
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// `omarchy-theme-dir` semantics: the user copy wins over the stock one.
    pub fn theme_dir(&self, slug: &str) -> PathBuf {
        let user = self.user_themes.join(slug);
        if user.is_dir() {
            user
        } else {
            self.system_themes.join(slug)
        }
    }

    /// Pick the theme `steps` positions away from the current one (wrapping).
    pub fn neighbour_theme(&self, steps: i32) -> Option<String> {
        self.neighbour_of(self.current_theme_name().as_deref(), steps)
    }

    /// Same, relative to an explicit slug (`None` = the first theme).
    pub fn neighbour_of(&self, slug: Option<&str>, steps: i32) -> Option<String> {
        let themes = self.list_themes();
        if themes.is_empty() {
            return None;
        }
        let idx = slug.and_then(|c| themes.iter().position(|t| t == c)).unwrap_or(0) as i32;
        let n = themes.len() as i32;
        Some(themes[(((idx + steps) % n) + n) as usize % themes.len()].clone())
    }

    /// Resolve an installed theme by slug without activating it.
    pub fn load_theme(&self, slug: &str) -> Result<SourcePalette> {
        let mut p = load_file(&self.theme_dir(slug).join("colors.toml"))?;
        if p.name.is_none() {
            p.name = Some(slug.to_string());
        }
        Ok(p)
    }

    /// Apply a theme through Omarchy itself, so every app retints and the hooks fire.
    pub fn set_theme(&self, name: &str) -> Result<()> {
        let status = Command::new("omarchy-theme-set")
            .arg(name)
            .status()
            .context("running omarchy-theme-set (is Omarchy on PATH?)")?;
        if !status.success() {
            bail!("omarchy-theme-set {name:?} exited with {status}");
        }
        Ok(())
    }
}

/// Resolve a `colors.toml`. Uses `omarchy-theme-color --file F --all` when available so the
/// palette is byte-for-byte what Omarchy's own templates saw; otherwise the Rust port.
pub fn load_file(path: &Path) -> Result<SourcePalette> {
    if !path.is_file() {
        bail!("{} does not exist", path.display());
    }
    if let Some(p) = load_via_omarchy_theme_color(path)? {
        return Ok(p);
    }
    let text = fs::read_to_string(path)?;
    let raw = parse_colors_toml(&text);
    Ok(resolve(raw, path.parent()))
}

fn load_via_omarchy_theme_color(path: &Path) -> Result<Option<SourcePalette>> {
    if std::env::var_os("THEMESYNC_NO_OMARCHY_RESOLVER").is_some() {
        return Ok(None);
    }
    let out = match Command::new("omarchy-theme-color").arg("--file").arg(path).arg("--all").output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("running omarchy-theme-color"),
    };
    if !out.status.success() {
        bail!(
            "omarchy-theme-color --all failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(Some(parse_all_output(&String::from_utf8_lossy(&out.stdout))))
}

/// Parse the `key<TAB>value` lines `omarchy-theme-color --all` prints.
pub fn parse_all_output(text: &str) -> SourcePalette {
    let mut p = SourcePalette::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('\t') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || v.is_empty() {
            continue;
        }
        match k {
            "mode" | "theme_type" => {
                p.mode = match v {
                    "light" => Some(Mode::Light),
                    "dark" => Some(Mode::Dark),
                    _ => p.mode,
                };
                p.extras.insert(k.to_string(), v.to_string());
            }
            _ => match Rgb::parse(v) {
                Some(c) => {
                    p.colors.insert(k.to_string(), c);
                }
                None => {
                    p.extras.insert(k.to_string(), v.to_string());
                }
            },
        }
    }
    p
}

/// The line-based parser from `omarchy-theme-color::parse_colors_file`, verbatim in spirit:
/// split on the first `=`, strip quotes/spaces from the key, take the text between the first
/// pair of quotes as the value (or the trimmed bare word), skip comments, and reject keys and
/// values outside Omarchy's charset.
pub fn parse_colors_toml(text: &str) -> BTreeMap<String, String> {
    let key_ok = |k: &str| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let val_ok = |v: &str| v.chars().all(|c| c.is_ascii_alphanumeric() || "#(),._+/% -".contains(c));

    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let key: String = key.chars().filter(|c| !matches!(c, '"' | '\'' | ' ')).collect();
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        let value = if value.contains('"') || value.contains('\'') {
            let start = value.find(['"', '\'']).unwrap() + 1;
            let rest = &value[start..];
            let end = rest.find(['"', '\'']).unwrap_or(rest.len());
            rest[..end].to_string()
        } else {
            value.trim().to_string()
        };
        if !key_ok(&key) || !val_ok(&value) {
            continue;
        }
        out.insert(key, value);
    }
    out
}

/// Port of `omarchy-theme-color::resolve_theme_colors` + `resolve_theme_mode`.
/// `dir` is where a legacy `light.mode` marker would sit.
pub fn resolve(raw: BTreeMap<String, String>, dir: Option<&Path>) -> SourcePalette {
    // Empty strings count as unset, exactly like `[[ ${THEME_COLORS[k]} ]]` in bash.
    let mut m: BTreeMap<String, String> = raw.into_iter().filter(|(_, v)| !v.is_empty()).collect();

    fn alias(m: &mut BTreeMap<String, String>, key: &str, fallback: &str) {
        if !m.contains_key(key) {
            if let Some(v) = m.get(fallback).cloned() {
                m.insert(key.to_string(), v);
            }
        }
    }
    fn get(m: &BTreeMap<String, String>, k: &str) -> Option<String> {
        m.get(k).cloned()
    }
    fn derive(m: &mut BTreeMap<String, String>, key: &str, f: impl FnOnce(&BTreeMap<String, String>) -> Option<String>) {
        if !m.contains_key(key) {
            if let Some(v) = f(&*m) {
                m.insert(key.to_string(), v);
            }
        }
    }
    fn mix(m: &BTreeMap<String, String>, key: &str, other: Rgb, t: f32) -> Option<String> {
        get(m, key).and_then(|v| Rgb::parse(&v)).map(|c| c.mix(other, t).to_hex())
    }

    const LEGACY_PALETTE: [(&str, &str); 8] = [
        ("background", "bg"),
        ("dark_background", "dark_bg"),
        ("darker_background", "darker_bg"),
        ("lighter_background", "lighter_bg"),
        ("foreground", "fg"),
        ("dark_foreground", "dark_fg"),
        ("light_foreground", "light_fg"),
        ("bright_foreground", "bright_fg"),
    ];
    for (canon, legacy) in LEGACY_PALETTE {
        alias(&mut m, canon, legacy);
    }

    alias(&mut m, "background", "color0");
    alias(&mut m, "foreground", "color7");
    if let Some(bg) = get(&m, "background") {
        m.insert("color0".into(), bg);
    }
    if let Some(fg) = get(&m, "foreground") {
        m.insert("color7".into(), fg);
    }

    const LEGACY_ANSI: [(&str, &str); 12] = [
        ("red", "color1"),
        ("green", "color2"),
        ("yellow", "color3"),
        ("blue", "color4"),
        ("magenta", "color5"),
        ("cyan", "color6"),
        ("bright_red", "color9"),
        ("bright_green", "color10"),
        ("bright_yellow", "color11"),
        ("bright_blue", "color12"),
        ("bright_magenta", "color13"),
        ("bright_cyan", "color14"),
    ];
    for (canon, ansi) in LEGACY_ANSI {
        alias(&mut m, canon, ansi);
    }
    alias(&mut m, "magenta", "purple");
    alias(&mut m, "bright_magenta", "bright_purple");

    derive(&mut m, "light_foreground", |m| get(m, "color7").or_else(|| get(m, "foreground")));
    derive(&mut m, "bright_foreground", |m| get(m, "color15").or_else(|| get(m, "foreground")));
    if let Some(v) = get(&m, "bright_foreground") {
        m.insert("cursor".into(), v);
    }
    derive(&mut m, "lighter_background", |m| get(m, "color0").or_else(|| get(m, "background")));
    derive(&mut m, "dark_foreground", |m| get(m, "color8").or_else(|| get(m, "foreground")));
    derive(&mut m, "muted", |m| get(m, "color8").or_else(|| get(m, "dark_foreground")));
    derive(&mut m, "selection", |m| {
        get(m, "selection_background")
            .or_else(|| get(m, "color8"))
            .or_else(|| get(m, "color0"))
            .or_else(|| get(m, "background"))
    });
    derive(&mut m, "selection_background", |m| get(m, "selection"));
    derive(&mut m, "selection_foreground", |m| get(m, "bright_foreground"));
    derive(&mut m, "orange", |m| get(m, "yellow"));
    derive(&mut m, "brown", |m| mix(m, "orange", Rgb::BLACK, 0.5));

    derive(&mut m, "dark_background", |m| mix(m, "background", Rgb::BLACK, 0.25));
    derive(&mut m, "darker_background", |m| mix(m, "background", Rgb::BLACK, 0.5));
    for base in ["red", "yellow", "green", "cyan", "blue", "magenta"] {
        let bright = format!("bright_{base}");
        derive(&mut m, &bright, |m| mix(m, base, Rgb::WHITE, 0.2));
    }
    alias(&mut m, "purple", "magenta");
    alias(&mut m, "bright_purple", "bright_magenta");

    const ANSI_ALIAS: [(&str, &str); 16] = [
        ("color0", "background"),
        ("color1", "red"),
        ("color2", "green"),
        ("color3", "yellow"),
        ("color4", "blue"),
        ("color5", "magenta"),
        ("color6", "cyan"),
        ("color7", "foreground"),
        ("color8", "muted"),
        ("color9", "bright_red"),
        ("color10", "bright_green"),
        ("color11", "bright_yellow"),
        ("color12", "bright_blue"),
        ("color13", "bright_magenta"),
        ("color14", "bright_cyan"),
        ("color15", "bright_foreground"),
    ];
    for (ansi, canon) in ANSI_ALIAS {
        alias(&mut m, ansi, canon);
    }
    for (canon, legacy) in LEGACY_PALETTE {
        if let Some(v) = get(&m, canon) {
            m.insert(legacy.to_string(), v);
        }
    }

    // mode precedence: `mode`, legacy `theme_type`, a light.mode marker, background luminance, dark
    alias(&mut m, "mode", "theme_type");
    let mode = match get(&m, "mode").as_deref() {
        Some("light") => Mode::Light,
        Some("dark") => Mode::Dark,
        Some(other) => {
            // Omarchy passes an unknown mode string through untouched; we normalise the
            // colour side to the same auto-detect it would have used had the key been absent.
            let _ = other;
            auto_mode(&m, dir)
        }
        None => auto_mode(&m, dir),
    };
    m.insert("mode".into(), mode.as_str().into());
    m.insert("theme_type".into(), mode.as_str().into());

    let mut p = SourcePalette { mode: Some(mode), ..Default::default() };
    for (k, v) in m {
        match Rgb::parse(&v) {
            Some(c) => {
                p.colors.insert(k, c);
            }
            None => {
                p.extras.insert(k, v);
            }
        }
    }
    p
}

fn auto_mode(m: &BTreeMap<String, String>, dir: Option<&Path>) -> Mode {
    if dir.map(|d| d.join("light.mode").is_file()).unwrap_or(false) {
        return Mode::Light;
    }
    match m.get("background").and_then(|v| Rgb::parse(v)) {
        Some(bg) if m["background"].len() == 7 && m["background"].starts_with('#') => Mode::from_background(bg),
        _ => Mode::Dark,
    }
}

/// Convenience for the CLI: `--file` beats the live install.
pub fn load(file: Option<&Path>) -> Result<SourcePalette> {
    match file {
        Some(f) => {
            let mut p = load_file(f)?;
            if p.name.is_none() {
                // tests/fixtures/tokyo-night.toml -> "tokyo-night"; themes/tokyo-night/colors.toml -> "tokyo-night"
                p.name = f
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| *s != "colors")
                    .map(str::to_string)
                    .or_else(|| f.parent().and_then(|d| d.file_name()).and_then(|s| s.to_str()).map(str::to_string));
            }
            Ok(p)
        }
        None => Omarchy::from_env()?.load_current().map_err(|e| {
            anyhow!("{e:#}\n(hint: pass --file <colors.toml> when not running on Omarchy)")
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.toml"))
    }

    fn resolve_fixture(name: &str) -> SourcePalette {
        let text = fs::read_to_string(fixture(name)).unwrap();
        resolve(parse_colors_toml(&text), None)
    }

    #[test]
    fn parser_handles_quotes_comments_and_spacing() {
        let raw = parse_colors_toml(
            "mode = \"dark\"\n# comment\naccent=\"#7aa2f7\" # trailing\n\n  bg = '#111111'\nbad key = \"#000\"\nweird = \"a;b\"\nbare = ff0000\n",
        );
        assert_eq!(raw["mode"], "dark");
        assert_eq!(raw["accent"], "#7aa2f7");
        assert_eq!(raw["bg"], "#111111");
        assert_eq!(raw["bare"], "ff0000");
        assert_eq!(raw["badkey"], "#000"); // bash strips spaces from the key, so "bad key" is accepted as "badkey"
        assert!(!raw.contains_key("weird")); // ';' is outside Omarchy's value charset
    }

    #[test]
    fn modern_theme_resolves_all_derived_keys() {
        let p = resolve_fixture("tokyo-night");
        assert_eq!(p.mode, Some(Mode::Dark));
        assert_eq!(p.get("background"), Rgb::parse("#1a1b26"));
        assert_eq!(p.get("color0"), Rgb::parse("#1a1b26"));
        assert_eq!(p.get("color8"), Rgb::parse("#414868")); // muted
        assert_eq!(p.get("color15"), Rgb::parse("#c0caf5")); // bright_foreground
        assert_eq!(p.get("cursor"), Rgb::parse("#c0caf5"));
        assert_eq!(p.get("bg"), Rgb::parse("#1a1b26"));
        assert_eq!(p.get("lighter_bg"), Rgb::parse("#24283b"));
        assert_eq!(p.get("purple"), Rgb::parse("#ad8ee6"));
        assert_eq!(p.get("selection_background"), Rgb::parse("#292e42"));
        assert_eq!(p.get("selection_foreground"), Rgb::parse("#c0caf5"));
        assert_eq!(p.extras.get("theme_type").map(String::as_str), Some("dark"));
        // omarchy-theme-color --all prints 56 keys for tokyo-night (agent-verified); we keep
        // mode/theme_type in `extras`, everything else as colours.
        assert_eq!(p.colors.len() + p.extras.len(), 56);
    }

    #[test]
    fn legacy_ansi_only_theme_resolves_like_omarchy() {
        // What omarchy-theme-colors-from-alacritty generates for a theme without colors.toml.
        let raw = parse_colors_toml(
            r##"
accent = "#0000ff"
selection = "#333333"
background = "#101010"
foreground = "#e0e0e0"
color0 = "#101010"
color1 = "#ff0000"
color2 = "#00ff00"
color3 = "#ffff00"
color4 = "#0000ff"
color5 = "#ff00ff"
color6 = "#00ffff"
color7 = "#e0e0e0"
color8 = "#808080"
color9 = "#ff8080"
color10 = "#80ff80"
color11 = "#ffff80"
color12 = "#8080ff"
color13 = "#ff80ff"
color14 = "#80ffff"
color15 = "#ffffff"
"##,
        );
        let p = resolve(raw, None);
        assert_eq!(p.get("red"), Rgb::parse("#ff0000"));
        assert_eq!(p.get("bright_cyan"), Rgb::parse("#80ffff"));
        assert_eq!(p.get("muted"), Rgb::parse("#808080")); // color8
        assert_eq!(p.get("dark_foreground"), Rgb::parse("#808080")); // color8
        assert_eq!(p.get("lighter_background"), Rgb::parse("#101010")); // color0 == background (!)
        assert_eq!(p.get("bright_foreground"), Rgb::parse("#ffffff")); // color15
        assert_eq!(p.get("light_foreground"), Rgb::parse("#e0e0e0")); // color7
        assert_eq!(p.get("orange"), Rgb::parse("#ffff00")); // yellow
        assert_eq!(p.get("brown"), Rgb::parse("#808000")); // mix(orange, black, 50%)
        assert_eq!(p.get("dark_background"), Rgb::parse("#0c0c0c")); // mix(bg, black, 25%)
        assert_eq!(p.get("darker_background"), Rgb::parse("#080808"));
        assert_eq!(p.mode, Some(Mode::Dark)); // auto-detected from background
    }

    #[test]
    fn mode_precedence() {
        let mut raw = BTreeMap::new();
        raw.insert("background".to_string(), "#ffffff".to_string());
        raw.insert("foreground".to_string(), "#000000".to_string());
        assert_eq!(resolve(raw.clone(), None).mode, Some(Mode::Light)); // luminance
        raw.insert("theme_type".to_string(), "dark".to_string());
        assert_eq!(resolve(raw.clone(), None).mode, Some(Mode::Dark)); // legacy key
        raw.insert("mode".to_string(), "light".to_string());
        assert_eq!(resolve(raw.clone(), None).mode, Some(Mode::Light)); // canonical key wins

        let dir = std::env::temp_dir().join(format!("themesync-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("light.mode"), "").unwrap();
        let mut raw = BTreeMap::new();
        raw.insert("background".to_string(), "#000000".to_string());
        raw.insert("foreground".to_string(), "#ffffff".to_string());
        assert_eq!(resolve(raw, Some(&dir)).mode, Some(Mode::Light)); // marker file
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn system_themes_follow_omarchy_path_then_the_local_install() {
        let home = std::env::temp_dir().join(format!("themesync-home-{}", std::process::id()));
        fs::create_dir_all(home.join(".local/share/omarchy/themes/nord")).unwrap();
        assert_eq!(Omarchy::at(home.clone(), Some(PathBuf::from("/opt/omarchy"))).system_themes, PathBuf::from("/opt/omarchy/themes"));
        assert_eq!(Omarchy::at(home.clone(), None).system_themes, home.join(".local/share/omarchy/themes"));
        assert_eq!(Omarchy::at(home.clone(), None).list_themes(), vec!["nord".to_string()]);
        fs::remove_dir_all(&home).unwrap();
        assert_eq!(Omarchy::at(home.clone(), None).system_themes, PathBuf::from("/usr/share/omarchy/themes"));
        assert_eq!(Omarchy::at(home.clone(), None).user_themes, home.join(".config/omarchy/themes"));
    }

    #[test]
    fn all_output_parser_separates_colours_from_extras() {
        let p = parse_all_output(
            "accent\t#7aa2f7\nhyprland_active_border\trgba(33ccffee) rgba(00ff99ee) 45deg\nmode\tlight\n\nbroken line\ntheme_type\tlight\n",
        );
        assert_eq!(p.get("accent"), Rgb::parse("#7aa2f7"));
        assert_eq!(p.mode, Some(Mode::Light));
        assert_eq!(p.extras["hyprland_active_border"], "rgba(33ccffee) rgba(00ff99ee) 45deg");
        assert!(!p.colors.contains_key("broken line"));
    }

    #[test]
    fn every_fixture_resolves_with_both_paths_agreeing_on_file_keys() {
        for name in ["tokyo-night", "gruvbox", "catppuccin-latte", "flexoki-light", "white", "rose-pine", "nord"] {
            let text = fs::read_to_string(fixture(name)).unwrap();
            let raw = parse_colors_toml(&text);
            let p = resolve(raw.clone(), None);
            // canonical keys from the file survive untouched
            for (k, v) in &raw {
                if let Some(c) = Rgb::parse(v) {
                    assert_eq!(p.get(k), Some(c), "{name}: key {k}");
                }
            }
            // derived keys always present
            for k in ["dark_background", "darker_background", "lighter_background", "muted", "selection", "orange", "brown", "color8", "color15", "bg", "fg"] {
                assert!(p.colors.contains_key(k), "{name}: missing derived key {k}");
            }
        }
    }
}
