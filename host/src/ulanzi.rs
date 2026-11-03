//! TC002 Wi-Fi receiver. Its retry loop is independent of the BLE radio and never blocks it.
use crate::{
    omarchy,
    palette::{Rgb, SourcePalette},
};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCRIPT: &str = include_str!("../../devices/ulanzi/garden.be");
const LANDSCAPES: &[u8] = include_bytes!("../../devices/ulanzi/assets/garden.rgb");
const PLANTS: &[u8] = include_bytes!("../../devices/ulanzi/assets/plants.rgb");

#[derive(Subcommand)]
pub enum Command {
    /// Register the TC002 custom-runtime address and a local credentials file.
    Add {
        host: String,
        #[arg(long)]
        token_file: PathBuf,
    },
    /// Apply the current theme immediately; --file previews a palette without changing the desktop.
    Sync {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Read the display and last background-sync result (no token is printed).
    Status,
    /// Show the growing garden. --bloom is an idempotent event, e.g. paid-order:123.
    Garden {
        #[arg(long, value_name = "EVENT_ID")]
        bloom: Option<String>,
    },
    /// Return to the clock, leaving the garden age and flowers intact.
    Clock,
    /// Select a locally rendered paid-order scene.
    Scene {
        #[arg(value_parser = ["city", "rail", "orbit", "vinyl", "maker", "garden"])]
        name: String,
    },
    /// Replay the current scene animation without changing the real paid count.
    Replay,
    /// Run just the Wi-Fi receiver (normally also started by themesync daemon).
    Watch,
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    host: String,
    token_file: PathBuf,
    garden: Garden,
    /// Native six-scene engine owns the panel; never reinstall the legacy garden.
    #[serde(default)]
    stories: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
struct Garden {
    planted_at: u64,
    paid_flowers: u64,
    initial_season: usize,
    #[serde(default)]
    events: Vec<String>,
}

impl Garden {
    fn new() -> Self {
        Self {
            planted_at: now(),
            paid_flowers: 0,
            initial_season: ((local_month() + 10) % 12) / 3,
            events: vec![],
        }
    }
    fn season(&self, timestamp: u64) -> usize {
        (self.initial_season + timestamp.saturating_sub(self.planted_at) as usize / (7 * 86400)) % 4
    }
    fn bloom(&mut self, event: &str) -> Result<bool> {
        if event.is_empty() || event.len() > 128 || event.chars().any(char::is_control) {
            bail!("event id must be 1..128 printable characters");
        }
        if self.events.iter().any(|e| e == event) {
            return Ok(false);
        }
        self.paid_flowers = self.paid_flowers.saturating_add(1);
        self.events.push(event.into());
        Ok(true)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn local_parts() -> (usize, usize) {
    #[cfg(unix)]
    unsafe {
        let timestamp = now() as libc::time_t;
        let mut parts: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&timestamp, &mut parts).is_null() {
            return (parts.tm_mon as usize, parts.tm_hour as usize);
        }
    }
    (10, 12)
}
fn local_month() -> usize {
    local_parts().0
}
fn variant(g: &Garden) -> usize {
    g.season(now()) * 2 + usize::from(!(7..19).contains(&local_parts().1))
}
fn config_path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config")
                .into()
        }))
        .join("themesync/ulanzi.json"),
    )
}
fn state_path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("XDG_STATE_HOME").unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/state")
                .into()
        }))
        .join("themesync/ulanzi-status.json"),
    )
}
fn load() -> Result<Config> {
    let path = config_path()?;
    serde_json::from_slice(
        &fs::read(&path)
            .with_context(|| format!("read {}; run themesync ulanzi add", path.display()))?,
    )
    .context("invalid Ulanzi config")
}
pub fn cached_status() -> Option<Value> {
    fs::read(state_path().ok()?)
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

/// Serializes device operations across hook, CLI and background worker, including garden updates.
async fn device_lock() -> Result<fs::File> {
    let path = config_path()?.with_extension("lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        for _ in 0..200 {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("another Ulanzi sync is still running");
    }
    #[cfg(not(unix))]
    {
        Ok(file)
    }
}

struct Device {
    client: Client,
    url: String,
    control: String,
    admin: String,
}
impl Device {
    fn open(config: &Config) -> Result<Self> {
        let host = config.host.trim_end_matches('/');
        let url = if host.starts_with("http://") || host.starts_with("https://") {
            host.to_string()
        } else {
            format!("http://{host}")
        };
        let parsed = reqwest::Url::parse(&url).context("invalid device address")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            bail!("use only the display host or host:port, without credentials or a path");
        }
        let contents =
            fs::read_to_string(&config.token_file).context("read local Ulanzi token file")?;
        let get = |key: &str| -> Result<String> {
            let value = contents
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{key}=")))
                .context("token file needs control= and admin= lines")?;
            if value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
                bail!("invalid {key} token in local credentials file");
            }
            Ok(value.to_string())
        };
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()?,
            url,
            control: get("control")?,
            admin: get("admin")?,
        })
    }
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        admin: bool,
    ) -> Result<Value> {
        let mut req = self
            .client
            .request(method, format!("{}/api/v1{path}", self.url))
            .bearer_auth(if admin { &self.admin } else { &self.control });
        if let Some(value) = body {
            req = req.json(&value);
        }
        let response = req
            .send()
            .await
            .with_context(|| format!("display {path} unreachable"))?;
        let status = response.status();
        let raw = response.bytes().await?;
        if !status.is_success() {
            bail!("display {path}: HTTP {status}");
        }
        serde_json::from_slice(&raw).with_context(|| format!("display {path}: invalid JSON"))
    }
    async fn stories(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut url = reqwest::Url::parse(&self.url)?;
        url.set_port(Some(8878))
            .map_err(|_| anyhow::anyhow!("invalid stories address"))?;
        url.set_path(path);
        let mut request = self.client.request(method, url).bearer_auth(&self.control);
        if let Some(body) = body {
            request = request.json(&body);
        }
        Ok(request.send().await?.error_for_status()?.json().await?)
    }
    async fn script(&self, name: &str, source: &str) -> Result<()> {
        self.client
            .put(format!("{}/api/v1/berry/scripts/{name}", self.url))
            .bearer_auth(&self.admin)
            .header("content-type", "text/plain")
            .body(source.to_string())
            .send()
            .await?
            .error_for_status()
            .context("store garden script")?;
        Ok(())
    }
    async fn run_script(&self, name: &str) -> Result<()> {
        self.send(
            Method::POST,
            &format!("/berry/scripts/{name}/run"),
            None,
            true,
        )
        .await?;
        Ok(())
    }
    async fn sprite(&self, name: &str, data: Vec<u8>) -> Result<()> {
        self.client
            .put(format!("{}/api/v1/sprites/{name}", self.url))
            .bearer_auth(&self.admin)
            .header("content-type", "application/octet-stream")
            .body(data)
            .send()
            .await?
            .error_for_status()
            .context("store garden sprite")?;
        Ok(())
    }
}

fn colours(source: &SourcePalette) -> Result<(Rgb, Rgb, Rgb)> {
    let accent = source
        .get("accent")
        .or_else(|| source.get("blue"))
        .context("theme has no accent or blue")?;
    let other = source
        .get("magenta")
        .or_else(|| source.get("purple"))
        .or_else(|| source.get("green"))
        .unwrap_or(accent);
    let leaf = source.get("green").unwrap_or(Rgb::new(77, 153, 100));
    Ok((accent, other, leaf))
}
fn bare(c: Rgb) -> String {
    c.to_hex().trim_start_matches('#').to_string()
}
fn clock_patch(source: &SourcePalette) -> Result<Value> {
    let (a, b, _) = colours(source)?;
    Ok(
        json!({"clock_colour_mode":"gradient","clock_colour":bare(a),"clock_colour2":bare(b),"clock_gradient":"horizontal","clock_fade":true}),
    )
}
fn tint(bytes: &[u8], accent: Rgb, secondary: Rgb) -> Vec<u8> {
    bytes
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| {
            let original = Rgb::new(p[0], p[1], p[2]);
            let mapped = if p[2] > p[0].saturating_add(15) && p[1] > p[0].saturating_add(15) {
                original.mix(accent, 0.25)
            } else if p[2] > p[1].saturating_add(20) && p[0] > p[1].saturating_add(10) {
                original.mix(secondary, 0.20)
            } else {
                original
            };
            // The LED driver lifts any nonzero channel to >=50/255. Keep
            // theme mixing from turning trace RGB values into a white haze.
            [mapped.r, mapped.g, mapped.b].map(|channel| if channel < 32 { 0 } else { channel })
        })
        .collect()
}
fn landscape_tile(index: usize, tile: usize) -> Vec<u8> {
    let frame = &LANDSCAPES[index * 48 * 16 * 3..(index + 1) * 48 * 16 * 3];
    (0..16)
        .flat_map(|row| {
            frame[(row * 48 + tile * 16) * 3..(row * 48 + tile * 16 + 16) * 3]
                .iter()
                .copied()
        })
        .collect()
}
fn theme_script(config: &Config, source: &SourcePalette) -> Result<String> {
    let (a, b, l) = colours(source)?;
    Ok(format!(
        "g_accent=0x{}\ng_secondary=0x{}\ng_leaf=0x{}\ng_epoch={}\ng_paid={}\ng_season={}\n",
        bare(a),
        bare(b),
        bare(l),
        config.garden.planted_at,
        config.garden.paid_flowers,
        config.garden.season(now())
    ))
}

async fn sync(config: &Config, source: &SourcePalette, assets: bool) -> Result<Value> {
    let device = Device::open(config)?;
    let status = device.send(Method::GET, "/status", None, false).await?;
    if status["renderer"] != "running" {
        bail!("display renderer is not running; start the custom runtime");
    }
    let current = device.send(Method::GET, "/config", None, false).await?;
    let patch = clock_patch(source)?;
    let (a, b, _) = colours(source)?;
    let palette_drift = current["clock"]["colour"] != bare(a)
        || current["clock"]["colour2"] != bare(b)
        || current["clock"]["colour_mode"] != "gradient";
    let restarted = cached_status()
        .map(
            |last| match (last["frames"].as_u64(), status["presented"].as_u64()) {
                (Some(previous), Some(current)) => {
                    current < previous || last["boot_id"] != status["boot_id"]
                }
                _ => false,
            },
        )
        .unwrap_or(false);
    // The device can change while desktop files stay the same (preview or RAM restart).
    let assets = assets || palette_drift || restarted;
    if palette_drift {
        device
            .send(Method::PATCH, "/config", Some(patch), true)
            .await?;
    }
    if config.stories {
        let scene = device
            .stories(
                Method::PUT,
                "/palette",
                Some(json!({
                    "name": source.name, "accent": bare(a), "secondary": bare(b)
                })),
            )
            .await?;
        return Ok(
            json!({"connected":true,"host":config.host,"theme":source.name,
            "synced_at":now(),"mode":"stories","scene":scene["scene"],"paid_today":scene["count"],
            "day":scene["day"],"build":status["build"],"frames":status["presented"],"boot_id":status["boot_id"]}),
        );
    }
    if assets {
        let installed = device
            .client
            .get(format!("{}/api/v1/berry/scripts/autoexec", device.url))
            .bearer_auth(&device.admin)
            .send()
            .await?;
        let same = installed.status().is_success() && installed.text().await? == SCRIPT;
        for tile in 0..3 {
            device
                .sprite(
                    &format!("garden{tile}"),
                    tint(&landscape_tile(variant(&config.garden), tile), a, b),
                )
                .await?;
        }
        for (i, name) in ["flower0", "flower1", "flower2", "shoot", "moth"]
            .iter()
            .enumerate()
        {
            device
                .sprite(name, tint(&PLANTS[i * 192..(i + 1) * 192], a, b))
                .await?;
        }
        if !same {
            device
                .send(
                    Method::PATCH,
                    "/config",
                    Some(json!({"berry_enabled":false})),
                    true,
                )
                .await?;
            device.script("autoexec", SCRIPT).await?;
            device
                .send(
                    Method::PATCH,
                    "/config",
                    Some(json!({"berry_enabled":true})),
                    true,
                )
                .await?;
        } else if current["berry"]["enabled"] != true {
            device
                .send(
                    Method::PATCH,
                    "/config",
                    Some(json!({"berry_enabled":true})),
                    true,
                )
                .await?;
        }
        // Saving an existing script does not execute it; use a small assignment-only script
        // for palette/state updates, never re-registering autoexec's input handlers or timers.
        for _ in 0..20 {
            if device.send(Method::GET, "/berry", None, false).await?["state"] == "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        device
            .script("theme", &theme_script(config, source)?)
            .await?;
        device.run_script("theme").await?;
    }
    let verified = device.send(Method::GET, "/config", None, false).await?;
    if verified["clock"]["colour"] != bare(a) || verified["clock"]["colour2"] != bare(b) {
        bail!("display palette read-back differs");
    }
    Ok(
        json!({"connected":true,"host":config.host,"theme":source.name,"synced_at":now(),"season":(["spring","summer","autumn","winter"][config.garden.season(now())]),"paid_flowers":config.garden.paid_flowers,"garden_planted_at":config.garden.planted_at,"build":status["build"],"frames":status["presented"],"boot_id":status["boot_id"]}),
    )
}

fn source_fingerprint(config: &Config) -> Result<String> {
    let om = omarchy::Omarchy::from_env()?;
    Ok(format!(
        "{}|{}|{}|{}",
        fs::read_to_string(om.colors_file())?,
        fs::read_to_string(om.theme_name_file()).unwrap_or_default(),
        serde_json::to_string(config)?,
        variant(&config.garden)
    ))
}

pub async fn watch() {
    let mut applied = String::new();
    let mut retry = 1u64;
    let mut quiet_error = String::new();
    let mut last_check = std::time::Instant::now() - Duration::from_secs(60);
    loop {
        if let Ok(config) = load() {
            let wanted = source_fingerprint(&config);
            let changed = wanted.as_ref().map(|key| key != &applied).unwrap_or(true);
            if changed || last_check.elapsed() >= Duration::from_secs(30) {
                let result = async {
                    let _lock = device_lock().await?;
                    // Reload after the lock: a bloom may have arrived while we waited.
                    let config = load()?;
                    let source = tokio::task::spawn_blocking(|| omarchy::load(None)).await??;
                    let result = sync(&config, &source, changed).await?;
                    atomic_json(&state_path()?, &result)?;
                    source_fingerprint(&config)
                }
                .await;
                last_check = std::time::Instant::now();
                match result {
                    Ok(key) => {
                        if changed {
                            eprintln!("[themesync] Ulanzi: palette and garden synchronized");
                        }
                        applied = key;
                        retry = 1;
                        quiet_error.clear();
                    }
                    Err(e) => {
                        let message = format!("{e:#}");
                        if message != quiet_error {
                            eprintln!("[themesync] Ulanzi: {message}; retrying without blocking Bluetooth");
                            quiet_error = message.clone();
                        }
                        let _ = state_path().and_then(|p| {
                            atomic_json(
                                &p,
                                &json!({"connected":false,"checked_at":now(),"error":message}),
                            )
                        });
                        tokio::time::sleep(Duration::from_secs(retry)).await;
                        retry = (retry * 2).min(60);
                        applied.clear();
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub async fn run(command: Command) -> Result<()> {
    if matches!(&command, Command::Watch) {
        watch().await;
        return Ok(());
    }
    let _lock = device_lock().await?;
    match command {
        Command::Add { host, token_file } => {
            let token_file =
                fs::canonicalize(token_file).context("credentials file does not exist")?;
            let config = Config {
                host,
                token_file,
                garden: Garden::new(),
                stories: false,
            };
            let device = Device::open(&config)?;
            device.send(Method::GET, "/status", None, false).await?;
            if config_path()?.exists() {
                bail!("Ulanzi is already configured; edit {} to change the address without losing the garden",config_path()?.display());
            }
            atomic_json(&config_path()?, &config)?;
            println!("Ulanzi registered; garden age saved. Run themesync ulanzi sync.");
        }
        Command::Sync { file } => {
            let config = load()?;
            let source = omarchy::load(file.as_deref())?;
            let report = sync(&config, &source, true).await?;
            atomic_json(&state_path()?, &report)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Status => {
            let config = load()?;
            let device = Device::open(&config)?;
            let status = device.send(Method::GET, "/status", None, false).await?;
            let last = fs::read(state_path()?)
                .ok()
                .and_then(|s| serde_json::from_slice::<Value>(&s).ok());
            let report = if config.stories {
                let stories = device.stories(Method::GET, "/status", None).await?;
                json!({"host":config.host,"mode":"stories","device":status,"stories":stories,"last_sync":last})
            } else {
                json!({"host":config.host,"mode":"garden","device":status,"garden":config.garden,"last_sync":last})
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Garden { bloom } => {
            let mut config = load()?;
            if config.stories {
                if bloom.is_some() {
                    bail!("real daily payments are owned by the BaseLinker bridge; use replay for a demo");
                }
                Device::open(&config)?
                    .stories(Method::POST, "/scene", Some(json!({"scene":"garden"})))
                    .await?;
                println!("Garden scene is showing; real daily count preserved.");
                return Ok(());
            }
            if let Some(event) = bloom {
                if !config.garden.bloom(&event)? {
                    println!("Event already planted; no duplicate flower.");
                    return Ok(());
                }
                atomic_json(&config_path()?, &config)?;
                sync(&config, &omarchy::load(None)?, true).await?;
                let device = Device::open(&config)?;
                device.script("bloom", "garden_bloom()\n").await?;
                device.run_script("bloom").await?;
                println!(
                    "Flower planted. Paid flowers: {}",
                    config.garden.paid_flowers
                );
            } else {
                sync(&config, &omarchy::load(None)?, true).await?;
                let device = Device::open(&config)?;
                device.script("show", "garden_show()\n").await?;
                device.run_script("show").await?;
                println!("Garden is showing.");
            }
        }
        Command::Clock => {
            let config = load()?;
            let device = Device::open(&config)?;
            if config.stories {
                device
                    .stories(Method::POST, "/clock", Some(json!({})))
                    .await?;
                println!("Clock is showing; paid-order alerts remain active.");
                return Ok(());
            }
            device.script("clock", "garden_clock()\n").await?;
            device.run_script("clock").await?;
            println!("Clock is showing; garden preserved.");
        }
        Command::Scene { name } => {
            let config = load()?;
            if !config.stories {
                bail!("native stories engine is not configured");
            }
            let report = Device::open(&config)?
                .stories(Method::POST, "/scene", Some(json!({"scene":name})))
                .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Replay => {
            let config = load()?;
            if !config.stories {
                bail!("native stories engine is not configured");
            }
            Device::open(&config)?
                .stories(Method::POST, "/replay", Some(json!({})))
                .await?;
            println!("Animation replayed; real daily count unchanged.");
        }
        Command::Watch => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_is_idempotent_across_serialization() {
        let mut garden = Garden::new();
        assert!(garden.bloom("paid:42").unwrap());
        let mut restored: Garden =
            serde_json::from_str(&serde_json::to_string(&garden).unwrap()).unwrap();
        assert!(!restored.bloom("paid:42").unwrap());
        assert_eq!(restored.paid_flowers, 1);
        assert!(restored.bloom("paid:43").unwrap());
        assert_eq!(restored.paid_flowers, 2);
    }
    #[test]
    fn seasons_follow_garden_age_without_clock_underflow() {
        let g = Garden {
            planted_at: 100,
            paid_flowers: 0,
            initial_season: 2,
            events: vec![],
        };
        assert_eq!(g.season(0), 2);
        assert_eq!(g.season(100 + 7 * 86400), 3);
        assert_eq!(g.season(100 + 28 * 86400), 2);
    }
    #[test]
    fn palette_patch_preserves_user_brightness_and_font() {
        let mut s = SourcePalette::default();
        s.colors.insert("accent".into(), Rgb::new(1, 2, 3));
        s.colors.insert("magenta".into(), Rgb::new(4, 5, 6));
        let patch = clock_patch(&s).unwrap();
        assert_eq!(patch["clock_colour"], "010203");
        assert_eq!(patch["clock_colour2"], "040506");
        assert!(patch.get("brightness").is_none());
        assert!(patch.get("clock_font").is_none());
    }
    #[test]
    fn atlas_is_exactly_eight_scenes_and_five_tiny_sprites() {
        assert_eq!(LANDSCAPES.len(), 8 * 48 * 16 * 3);
        assert_eq!(PLANTS.len(), 5 * 8 * 8 * 3);
        assert_eq!(landscape_tile(7, 2).len(), 16 * 16 * 3);
    }
    #[test]
    fn black_transparency_and_warm_colours_survive_tinting() {
        let data = [0, 0, 0, 255, 180, 40];
        assert_eq!(tint(&data, Rgb::WHITE, Rgb::WHITE), data);
    }
    #[test]
    fn led_driver_noise_stays_off_after_theme_mixing() {
        let result = tint(
            &[251, 3, 2, 0, 250, 255],
            Rgb::new(76, 201, 255),
            Rgb::new(167, 139, 250),
        );
        assert_eq!(&result[..3], &[251, 0, 0]);
        assert_eq!(result[3], 0);
        assert!(result.iter().all(|&channel| channel == 0 || channel >= 32));
    }
}
