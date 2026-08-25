//! `omawatch` — push the active Omarchy theme to a smartwatch (Theme Protocol v1 over BLE).
//!
//! ```text
//! omawatch theme [--file F] [--json] [--source] [--contrast]   resolved watch palette
//! omawatch encode [--file F] [--hex|--raw]                     the v1 packet
//! omawatch decode <hex|-> [--json]                             simulated watch receiver
//! omawatch demo [--file F]                                     the whole chain, printed
//! omawatch sync [--file F] [--direct] [--retries N]            push to the watch
//! omawatch daemon                                              resident link + bidi control
//! omawatch scan / status / next / prev / toggle / install-hook
//! ```

mod daemon;
mod omarchy;
mod palette;
mod protocol;
mod transport;

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use omarchy::Omarchy;
use palette::{map_source, WatchPalette};
use transport::ble::{self, BleOptions};
use transport::ipc::{self, Reply, Request};
use transport::sim::{self, SimWatch};

#[derive(Parser)]
#[command(name = "omawatch", version, about = "Sync the Omarchy desktop theme to a smartwatch over BLE")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone, Default)]
struct ThemeSource {
    /// Resolve this colors.toml instead of the active Omarchy theme.
    #[arg(long, value_name = "colors.toml", global = true)]
    file: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct BleArgs {
    /// Only accept a watch advertising this name (env: OMAWATCH_NAME).
    #[arg(long)]
    name: Option<String>,
    /// Seconds to scan for the watch before giving up (per attempt).
    #[arg(long, default_value_t = 8)]
    timeout: u64,
}

impl BleArgs {
    fn options(&self) -> BleOptions {
        let mut o = BleOptions::default();
        if self.name.is_some() {
            o.name = self.name.clone();
        }
        o.scan_timeout = Duration::from_secs(self.timeout);
        o
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the resolved watch palette for the active theme.
    Theme {
        #[command(flatten)]
        src: ThemeSource,
        /// JSON (the human-readable protocol form).
        #[arg(long)]
        json: bool,
        /// Show the resolved *source* (Omarchy) palette instead of the mapped one.
        #[arg(long)]
        source: bool,
        /// Append a WCAG contrast report for the pairs the UI relies on.
        #[arg(long)]
        contrast: bool,
    },
    /// Serialize the active theme as a Theme Protocol v1 packet.
    Encode {
        #[command(flatten)]
        src: ThemeSource,
        /// Plain hex on one line (default is an annotated dump).
        #[arg(long)]
        hex: bool,
        /// Raw bytes to stdout (pipe into the C simulator: `omawatch encode --raw | watch/sim/theme_sim`).
        #[arg(long)]
        raw: bool,
    },
    /// Decode a packet the way the watch does and print the resulting palette.
    Decode {
        /// Hex string, or `-` to read hex from stdin.
        packet: String,
        #[arg(long)]
        json: bool,
    },
    /// Run the whole chain without hardware: resolve -> map -> encode -> simulated watch.
    Demo {
        #[command(flatten)]
        src: ThemeSource,
    },
    /// Push the active theme to the watch (through the daemon if one is running).
    Sync {
        #[command(flatten)]
        src: ThemeSource,
        #[command(flatten)]
        ble: BleArgs,
        /// Skip the daemon and open a one-shot BLE connection.
        #[arg(long)]
        direct: bool,
        /// Connection attempts before giving up.
        #[arg(long, default_value_t = 4)]
        retries: u32,
        /// Return immediately after handing the job to a background process (for hooks).
        #[arg(long)]
        r#async: bool,
        /// Wire format: `v1` (Theme Protocol v1, default) or `mini` (the 13-byte prototype
        /// format of the onewheel watch's first firmware).
        #[arg(long, default_value = "v1")]
        proto: String,
    },
    /// Keep a connection open, serve `sync` requests, and act on the watch's requests.
    Daemon {
        #[command(flatten)]
        ble: BleArgs,
    },
    /// List nearby BLE devices, flagging the ones advertising the Theme service.
    Scan {
        #[command(flatten)]
        ble: BleArgs,
    },
    /// Read the watch's Info/Status characteristics (and the daemon's state).
    Status {
        #[command(flatten)]
        ble: BleArgs,
    },
    /// Switch Omarchy to the next theme (what the watch's NEXT button does).
    Next,
    /// Switch Omarchy to the previous theme.
    Prev,
    /// Switch Omarchy to the next theme of the opposite light/dark mode.
    Toggle,
    /// Install the theme-set hook into ~/.config/omarchy/hooks/theme-set.d/.
    InstallHook {
        /// Print the hook instead of writing it.
        #[arg(long)]
        print: bool,
    },
}

fn resolve(src: &ThemeSource) -> Result<WatchPalette> {
    let s = omarchy::load(src.file.as_deref())?;
    map_source(&s).map_err(Into::into)
}

fn use_ansi() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn hook_script() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "omawatch".into());
    format!(
        "#!/bin/bash\n\
         # Omarchy theme-set hook: push the new theme to the watch. Installed by `omawatch install-hook`.\n\
         # Omarchy runs this synchronously (bash <file> <theme-slug>) after the theme directory swap\n\
         # and all app retints, so return fast: the daemon (or a background one-shot) does the BLE work.\n\
         OMAWATCH=\"${{OMAWATCH_BIN:-{exe}}}\"\n\
         command -v \"$OMAWATCH\" >/dev/null 2>&1 || OMAWATCH=omawatch\n\
         \"$OMAWATCH\" sync --async >/dev/null 2>&1 || true\n"
    )
}

async fn sync_via_daemon(src: &ThemeSource) -> Result<Option<Reply>> {
    let req = match &src.file {
        None => Request::Sync,
        Some(_) => Request::Push { packet_hex: protocol::to_hex(&protocol::encode_theme(&resolve(src)?)) },
    };
    ipc::request(&req, Duration::from_secs(25)).await
}

async fn sync_direct(src: &ThemeSource, opts: &BleOptions, retries: u32) -> Result<()> {
    let p = resolve(src)?;
    let packet = protocol::encode_theme(&p);
    let adapter = ble::adapter().await?;
    let watch = ble::connect_with_retry(&adapter, opts, retries, |m| eprintln!("[omawatch] {m}")).await?;
    if let Ok(Some(info)) = watch.info().await {
        if !info.supports(protocol::VERSION) {
            watch.disconnect().await;
            bail!("watch speaks protocol v{}..v{}, this build sends v{}", info.proto_min, info.proto_max, protocol::VERSION);
        }
    }
    let status = watch.send_theme(&packet).await;
    watch.disconnect().await;
    let status = status?;
    match status {
        Some(s) => println!("sent {} ({} bytes) to {}: acknowledged, crc {:#06x}, {} colours, {}", p.name, packet.len(), watch.name, s.applied_crc, s.n_applied, s.mode.as_str()),
        None => println!("sent {} ({} bytes) to {} (no status characteristic; write accepted)", p.name, packet.len(), watch.name),
    }
    Ok(())
}

/// Push through the 13-byte "mini" adapter (see `transport::ble::mini_wire`).
async fn sync_mini(src: &ThemeSource, opts: &BleOptions, retries: u32) -> Result<()> {
    let p = resolve(src)?;
    let wire = ble::mini_wire(&p);
    let adapter = ble::adapter().await?;
    let mut delay = Duration::from_millis(500);
    let mut attempt = 0;
    let peripheral = loop {
        attempt += 1;
        match ble::discover_service(&adapter, opts, ble::MINI_SERVICE_UUID).await {
            Ok(per) => break per,
            Err(e) if attempt < retries => {
                eprintln!("[omawatch] attempt {attempt}: {e:#}; retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(15));
            }
            Err(e) => return Err(e),
        }
    };
    let result = ble::send_mini(&peripheral, &wire, &p.name).await;
    let _ = btleplug::api::Peripheral::disconnect(&peripheral).await;
    let back = result?;
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    println!(
        "mini: sent {} = bg #{} fg #{} accent #{} color1 #{}; read back {}",
        p.name,
        hex(&wire[1..4]),
        hex(&wire[4..7]),
        hex(&wire[7..10]),
        hex(&wire[10..13]),
        if back == wire { "OK (identical)" } else { "MISMATCH" }
    );
    if back != wire {
        bail!("watch returned {}", hex(&back));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Theme { src, json, source, contrast } => {
            if source {
                let s = omarchy::load(src.file.as_deref())?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&s)?);
                } else {
                    println!("theme: {}   mode: {}", s.name.as_deref().unwrap_or("?"), s.mode().map(|m| m.as_str()).unwrap_or("?"));
                    for (k, v) in &s.colors {
                        println!("  {k:<22} {v}");
                    }
                    for (k, v) in &s.extras {
                        println!("  {k:<22} {v}");
                    }
                }
                return Ok(());
            }
            let p = resolve(&src)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&p.to_json())?);
            } else {
                print!("{}", sim::render_palette(&p, use_ansi()));
            }
            if contrast {
                print!("{}", sim::render_contrast(&p));
            }
        }
        Cmd::Encode { src, hex, raw } => {
            let p = resolve(&src)?;
            let bytes = protocol::encode_theme(&p);
            if raw {
                std::io::stdout().write_all(&bytes)?;
            } else if hex {
                println!("{}", protocol::to_hex(&bytes));
            } else {
                println!("{}", protocol::hexdump_annotated(&bytes));
            }
        }
        Cmd::Decode { packet, json } => {
            let text = if packet == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                packet
            };
            let bytes = protocol::from_hex(&text).ok_or_else(|| anyhow!("not a hex string"))?;
            let mut w = SimWatch::default();
            let applied = w.receive(&bytes).with_context(|| "the watch would reject this packet")?.clone();
            if json {
                println!("{}", serde_json::to_string_pretty(&applied.to_json())?);
            } else {
                println!("watch accepted {} bytes, crc {:#06x}", bytes.len(), w.last_crc.unwrap());
                print!("{}", sim::render_palette(&applied, use_ansi()));
            }
        }
        Cmd::Demo { src } => {
            let s = omarchy::load(src.file.as_deref())?;
            println!("== 1. source palette ({} keys from {})", s.colors.len(), src.file.as_ref().map(|f| f.display().to_string()).unwrap_or_else(|| "the active Omarchy theme".into()));
            for k in ["background", "lighter_background", "foreground", "dark_foreground", "muted", "accent", "selection", "red", "yellow", "green", "blue", "cyan"] {
                if let Some(c) = s.get(k) {
                    println!("   {k:<20} {c}");
                }
            }
            let p = map_source(&s)?;
            println!("\n== 2. mapped watch palette");
            print!("{}", sim::render_palette(&p, use_ansi()));
            let bytes = protocol::encode_theme(&p);
            println!("\n== 3. Theme Protocol v1 packet");
            println!("{}", protocol::hexdump_annotated(&bytes));
            println!("\n== 4. simulated watch (decode + apply, same rules as the firmware)");
            let mut w = SimWatch::default();
            println!("   before: {} / {}", w.current.name, w.current.get(palette::Role::Accent));
            let applied = w.receive(&bytes)?.clone();
            println!("   after:  {} / accent {} / background {} / mode {}", applied.name, applied.get(palette::Role::Accent), applied.get(palette::Role::Background), applied.mode.as_str());
            print!("{}", sim::render_contrast(&applied));
        }
        Cmd::Sync { src, ble, direct, retries, r#async, proto } => {
            if !matches!(proto.as_str(), "v1" | "mini") {
                bail!("--proto must be v1 or mini");
            }
            if r#async {
                // Re-exec ourselves detached so the Omarchy hook returns immediately.
                let exe = std::env::current_exe()?;
                let mut cmd = std::process::Command::new(exe);
                cmd.arg("sync");
                if let Some(f) = &src.file {
                    cmd.arg("--file").arg(f);
                }
                if let Some(n) = &ble.name {
                    cmd.arg("--name").arg(n);
                }
                cmd.arg("--timeout").arg(ble.timeout.to_string()).arg("--retries").arg(retries.to_string());
                cmd.arg("--proto").arg(&proto);
                if direct {
                    cmd.arg("--direct");
                }
                cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                cmd.spawn().context("spawning background sync")?;
                return Ok(());
            }
            if proto == "mini" {
                return sync_mini(&src, &ble.options(), retries.max(1)).await;
            }
            if !direct {
                match sync_via_daemon(&src).await? {
                    Some(r) if r.ok => {
                        println!("sent via daemon to {}{}", r.watch.unwrap_or_default(), r.theme.map(|t| format!(" ({t})")).unwrap_or_default());
                        return Ok(());
                    }
                    Some(r) => bail!("daemon: {}", r.message.unwrap_or_default()),
                    None => {}
                }
            }
            sync_direct(&src, &ble.options(), retries).await?;
        }
        Cmd::Daemon { ble } => daemon::run(ble.options()).await?,
        Cmd::Scan { ble } => {
            let adapter = ble::adapter().await?;
            eprintln!("scanning for {}s...", ble.timeout);
            let mut seen = ble::scan(&adapter, Duration::from_secs(ble.timeout)).await?;
            seen.sort_by_key(|s| (!s.has_service, -s.rssi.unwrap_or(-200)));
            for s in seen {
                println!("{} {:<24} {:>5} dBm  {}", if s.has_service { "*" } else { " " }, s.name.as_deref().unwrap_or("(no name)"), s.rssi.map(|r| r.to_string()).unwrap_or_else(|| "?".into()), s.id);
            }
            println!("(* = advertises the Theme service {})", ble::SERVICE_UUID);
        }
        Cmd::Status { ble } => {
            if let Some(r) = ipc::request(&Request::Status, Duration::from_secs(10)).await? {
                println!("daemon: {} {}", if r.ok { "ok" } else { "error" }, r.message.unwrap_or_default());
                if let Some(w) = r.watch {
                    println!("watch:  {w}");
                }
                return Ok(());
            }
            let adapter = ble::adapter().await?;
            let watch = ble::connect_with_retry(&adapter, &ble.options(), 2, |m| eprintln!("[omawatch] {m}")).await?;
            println!("watch:  {}", watch.name);
            match watch.info().await? {
                Some(i) => println!("info:   protocol v{}..v{}, {} colour slots, features {:#04x}", i.proto_min, i.proto_max, i.max_colors, i.features),
                None => println!("info:   (no Info characteristic)"),
            }
            match watch.status().await? {
                Some(s) => println!("status: {:?}, applied crc {:#06x}, {} colours, {}", s.result, s.applied_crc, s.n_applied, s.mode.as_str()),
                None => println!("status: (no Status characteristic)"),
            }
            if let Ok(bytes) = watch.read_theme().await {
                match protocol::decode_theme(&bytes) {
                    Ok(d) => println!("theme:  {} ({} colours, {})", d.name.unwrap_or_else(|| "(unnamed)".into()), d.colors.len(), d.mode.as_str()),
                    Err(e) => println!("theme:  {} bytes, {e}", bytes.len()),
                }
            }
            watch.disconnect().await;
        }
        Cmd::Next => {
            let om = Omarchy::from_env()?;
            let t = om.neighbour_theme(1).ok_or_else(|| anyhow!("no themes installed"))?;
            println!("omarchy-theme-set {t}");
            om.set_theme(&t)?;
        }
        Cmd::Prev => {
            let om = Omarchy::from_env()?;
            let t = om.neighbour_theme(-1).ok_or_else(|| anyhow!("no themes installed"))?;
            println!("omarchy-theme-set {t}");
            om.set_theme(&t)?;
        }
        Cmd::Toggle => {
            let om = Omarchy::from_env()?;
            let t = om.opposite_mode_theme().ok_or_else(|| anyhow!("no theme of the opposite mode installed"))?;
            println!("omarchy-theme-set {t}");
            om.set_theme(&t)?;
        }
        Cmd::InstallHook { print } => {
            let script = hook_script();
            if print {
                print!("{script}");
                return Ok(());
            }
            let om = Omarchy::from_env()?;
            let dir = om.hooks_dir();
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("omawatch");
            std::fs::write(&path, script)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
            println!("installed {}", path.display());
            println!("Omarchy will run it as `bash {} <theme-slug>` after every theme change.", path.display());
        }
    }
    Ok(())
}
