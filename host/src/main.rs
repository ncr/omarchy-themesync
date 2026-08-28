//! `themesync` — push the active Omarchy theme to external devices (the OW-Watch first).
//!
//! ```text
//! themesync theme [--file F] [--json] [--source] [--contrast]   resolved watch palette
//! themesync encode [--file F] [--hex|--raw]                     the Theme Protocol v1 packet
//! themesync decode <hex|-> [--json]                             simulated v1 receiver
//! themesync demo [--file F]                                     the v1 chain, printed
//! themesync sync [--file F] [--direct] [--proto v2|mini|v1]     push to the watch (daemon or one-shot GATT)
//! themesync daemon [--no-gatt]                                  state beacon + request scanner (protocol/BEACON.md)
//! themesync pair                                                pairing key for beacon requests
//! themesync push-list [--force] [--dry-run] [--direct]          the theme list over GATT (protocol/BEACON.md §3)
//! themesync install [--no-enable] / uninstall [--purge] / doctor   unit + hook + diagnosis
//! themesync status [--json] / reset-counter / install-hook
//! themesync scan / encode / decode / demo                         Theme Protocol v1 tooling (hidden)
//! ```

// The beacon/request helpers are only called by the daemon, which is Linux-only.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod beacon;
#[cfg(target_os = "linux")]
mod daemon;
mod omarchy;
mod palette;
mod protocol;
#[cfg(target_os = "linux")]
mod setup;
mod themelist;
mod transport;

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use omarchy::Omarchy;
use palette::{map_source, WatchPalette};
use transport::ble::{self, BleOptions};
use transport::ipc::{self, Reply, Request};
use transport::sim::{self, SimWatch};

#[cfg(target_os = "linux")]
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (beacon v3, list status v2)");
#[cfg(not(target_os = "linux"))]
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (GATT only on this OS)");

#[derive(Parser)]
#[command(name = "themesync", version = VERSION, about = "Push the Omarchy desktop theme to a smartwatch over BLE")]
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
    /// Only accept a watch advertising this name (env: THEMESYNC_NAME).
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
    /// Serialize the active theme as a Theme Protocol v1 packet (no device speaks v1).
    #[command(hide = true)]
    Encode {
        #[command(flatten)]
        src: ThemeSource,
        /// Plain hex on one line (default is an annotated dump).
        #[arg(long)]
        hex: bool,
        /// Raw bytes to stdout (pipe into the C simulator: `themesync encode --raw | watch/sim/theme_sim`).
        #[arg(long)]
        raw: bool,
    },
    /// Decode a Theme Protocol v1 packet the way a v1 receiver would.
    #[command(hide = true)]
    Decode {
        /// Hex string, or `-` to read hex from stdin.
        packet: String,
        #[arg(long)]
        json: bool,
    },
    /// Run the v1 chain without hardware: resolve -> map -> encode -> simulated watch.
    #[command(hide = true)]
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
        /// Wire format on the OW-Watch service 7a0e0001: `v2` (default: the firmware's
        /// role-tagged packet, every role) or `mini` (the 13-byte core-four packet);
        /// `v1` is Theme Protocol v1 on service 7e450001 (not on any device yet).
        #[arg(long, default_value = "v2")]
        proto: String,
    },
    /// Advertise the current theme as a beacon, scan for the watch's requests, serve `sync`.
    Daemon {
        #[command(flatten)]
        ble: BleArgs,
        /// Do not also push each change over a one-shot GATT connection (the pre-beacon path).
        #[arg(long)]
        no_gatt: bool,
    },
    /// Pair with the watch: new key over GATT (7a0e0005) + a 2-digit code to confirm on the watch.
    Pair {
        #[command(flatten)]
        ble: BleArgs,
        /// Only write ~/.config/themesync/key; skip the watch.
        #[arg(long)]
        no_watch: bool,
    },
    /// Push the list of installed themes to the watch over GATT (7a0e0006), so it can show a picker.
    PushList {
        #[command(flatten)]
        ble: BleArgs,
        /// Send even if the watch reports it already holds this list (same crc).
        #[arg(long)]
        force: bool,
        /// Print the list and the BEGIN/DATA/COMMIT frames instead of connecting.
        #[arg(long)]
        dry_run: bool,
        /// Skip the daemon and open a one-shot BLE connection (key from ~/.config/themesync/key).
        #[arg(long)]
        direct: bool,
        /// DATA frame size in bytes (default: the negotiated MTU - 3, at most 509; --dry-run: 509).
        #[arg(long)]
        frame: Option<usize>,
        /// Find + connect + push attempts before giving up (--direct).
        #[arg(long, default_value_t = 3)]
        retries: u32,
    },
    /// List nearby BLE devices, flagging the ones advertising the Theme Protocol v1 service.
    #[command(hide = true)]
    Scan {
        #[command(flatten)]
        ble: BleArgs,
    },
    /// The daemon's state: pairing, beacon, scan, counter, last request, last list push.
    Status {
        #[command(flatten)]
        ble: BleArgs,
        /// The daemon's reply as JSON (for scripts and the bar widget).
        #[arg(long)]
        json: bool,
    },
    /// Write the systemd user unit and the Omarchy hook for this binary, enable the service, run `doctor`.
    Install {
        /// Write the files only; do not enable or start the service.
        #[arg(long)]
        no_enable: bool,
    },
    /// Stop and disable the service, remove the unit and the hook (keeps ~/.config/themesync unless --purge).
    Uninstall {
        /// Also delete ~/.config/themesync (the pairing key, the counter, the watch's address).
        #[arg(long)]
        purge: bool,
    },
    /// Check everything the daemon needs: Omarchy, BlueZ, the controller, the unit, the hook, the key.
    Doctor,
    /// Forget the last accepted request counter (BEACON.md §2): after the watch was
    /// reflashed and counts from 1 again. Prefer `themesync pair`, which resets both sides.
    ResetCounter,
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

#[cfg(target_os = "linux")]
fn hook_script() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("themesync"));
    setup::hook_text(&exe)
}

#[cfg(not(target_os = "linux"))]
fn hook_script() -> String {
    "#!/bin/bash\n# the Omarchy hook is a Linux thing; see `themesync install` there\n".into()
}

async fn sync_via_daemon(src: &ThemeSource) -> Result<Option<Reply>> {
    let req = match &src.file {
        None => Request::Sync,
        Some(_) => Request::Push { packet_hex: protocol::to_hex(&protocol::encode_v2(&resolve(src)?, false)) },
    };
    ipc::request(&req, Duration::from_secs(25)).await
}

/// Theme Protocol v1 over its own service (7e450001): no device speaks it yet.
async fn sync_v1(src: &ThemeSource, opts: &BleOptions, retries: u32) -> Result<()> {
    let p = resolve(src)?;
    let packet = protocol::encode_theme(&p);
    let adapter = ble::adapter().await?;
    let watch = ble::connect_with_retry(&adapter, opts, retries, |m| eprintln!("[themesync] {m}")).await?;
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

/// Find the watch with retries: by the address saved at pairing, else by its advertised
/// name (its advertisement is non-scannable, so the service UUID is not visible).
async fn find_watch(opts: &BleOptions, retries: u32) -> Result<(btleplug::platform::Adapter, btleplug::platform::Peripheral)> {
    let adapter = ble::adapter().await?;
    let saved = beacon::load_watch_addr();
    let mut delay = Duration::from_millis(500);
    let mut attempt = 0;
    loop {
        attempt += 1;
        match ble::find_watch(&adapter, opts, saved.as_deref()).await {
            Ok(per) => return Ok((adapter, per)),
            Err(e) if attempt < retries => {
                eprintln!("[themesync] attempt {attempt}: {e:#}; retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(15));
            }
            Err(e) => return Err(e),
        }
    }
}

/// One-shot GATT push in the OW-Watch's own formats: `v2` (every role) or `mini` (13 bytes
/// + the legacy name characteristic). Reads the applied palette back and compares.
async fn sync_gatt(src: &ThemeSource, opts: &BleOptions, retries: u32, proto: &str) -> Result<()> {
    let p = resolve(src)?;
    let (wire, name, sent_roles): (Vec<u8>, Option<&str>, Vec<palette::Role>) = if proto == "mini" {
        (protocol::encode_v1_legacy(&p).to_vec(), Some(&p.name), vec![palette::Role::Background, palette::Role::TextPrimary, palette::Role::Accent, palette::Role::Danger])
    } else {
        (protocol::encode_v2(&p, false), None, palette::Role::ALL.to_vec())
    };
    // Retry the whole find + connect + write step: BlueZ occasionally aborts a fresh
    // connection (`le-connection-abort-by-local`) right after another client's scan ended.
    let attempts = retries.max(1);
    let mut back = Vec::new();
    for attempt in 1..=attempts {
        let (_adapter, peripheral) = find_watch(opts, attempts).await?;
        let result = ble::send_colors(&peripheral, &wire, name).await;
        let _ = btleplug::api::Peripheral::disconnect(&peripheral).await;
        match result {
            Ok(b) => { back = b; break; }
            Err(e) if attempt < attempts => {
                eprintln!("[themesync] attempt {attempt}: {e:#}; retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e),
        }
    }
    match protocol::decode_v2(&back) {
        Ok(d) => {
            let wrong: Vec<String> = sent_roles
                .iter()
                .filter(|r| d.get(**r) != Some(p.get(**r)))
                .map(|r| format!("{} sent {} got {}", r.name(), p.get(*r), d.get(*r).map(|c| c.to_hex()).unwrap_or_else(|| "-".into())))
                .collect();
            println!("sent {} ({proto}, {} bytes); watch applied {}{}", p.name, wire.len(), d.summary(), if wrong.is_empty() { ": all sent roles match".to_string() } else { format!("; MISMATCH: {}", wrong.join(", ")) });
            if !wrong.is_empty() {
                bail!("read-back differs from what was sent");
            }
        }
        Err(e) => println!("sent {} ({proto}, {} bytes); read back {} bytes, not decodable: {e}", p.name, wire.len(), back.len()),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // `themesync status | head` must end quietly, not with "failed printing to stdout":
    // Rust ignores SIGPIPE and turns EPIPE into a panic; restore the Unix default.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
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
            if !matches!(proto.as_str(), "v2" | "mini" | "v1") {
                bail!("--proto must be v2, mini or v1");
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
            if !direct {
                match sync_via_daemon(&src).await? {
                    Some(r) if r.ok => {
                        println!("sent via daemon ({}){}", r.watch.unwrap_or_default(), r.theme.map(|t| format!(": {t}")).unwrap_or_default());
                        return Ok(());
                    }
                    Some(r) => bail!("daemon: {}", r.message.unwrap_or_default()),
                    None => {}
                }
            }
            if proto == "v1" {
                sync_v1(&src, &ble.options(), retries).await?;
            } else {
                sync_gatt(&src, &ble.options(), retries, &proto).await?;
            }
        }
        #[cfg(target_os = "linux")]
        Cmd::Daemon { ble, no_gatt } => daemon::run(daemon::Options { ble: ble.options(), gatt: !no_gatt }).await?,
        #[cfg(not(target_os = "linux"))]
        Cmd::Daemon { .. } => bail!("the daemon needs BlueZ (Linux): advertising and scanning go through D-Bus"),
        Cmd::Pair { ble, no_watch } => {
            let key = beacon::new_key().context("reading /dev/urandom")?;
            if no_watch {
                let path = beacon::save_key(&key)?;
                println!("key written to {} and the request counter reset (no watch involved; restart the daemon to use it)", path.display());
                return Ok(());
            }
            let code = beacon::new_key().context("reading /dev/urandom")?[0];
            // Hand the key to the daemon as *pending*: it becomes active only when the watch
            // sends a request signed with it, i.e. after the code was entered correctly.
            match ipc::request(&Request::PairPending { key_hex: protocol::to_hex(&key) }, Duration::from_secs(5)).await? {
                Some(r) if r.ok => {}
                Some(r) => bail!("daemon refused the pending key: {}", r.message.unwrap_or_default()),
                // Without the daemon nobody can see the watch's confirmation, and writing the
                // key unconfirmed would cut off the currently paired watch on a mistyped code
                // (BEACON.md §2b: the old key stays active until the new one is confirmed).
                None => bail!("the daemon is not running: start it first (systemctl --user start themesync), or use --no-watch to write a key without confirmation"),
            }
            let (_adapter, peripheral) = find_watch(&ble.options(), 3).await?;
            let r = ble::write_characteristic(&peripheral, ble::MINI_CHR_KEY, &beacon::encode_pair_write(code, &key)).await;
            let _ = btleplug::api::Peripheral::disconnect(&peripheral).await;
            r.context("sending the pairing request to the watch")?;
            println!();
            println!("    ┌──────────────┐");
            println!("    │   code  {:X} {:X}   │", code >> 4, code & 0x0f);
            println!("    └──────────────┘");
            println!();
            println!("enter it on the watch's Pairing screen and confirm.");
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(r) = ipc::request(&Request::Status, Duration::from_secs(5)).await? else { bail!("daemon went away") };
                let state = r.message.unwrap_or_default();
                if state.starts_with("paired with") {
                    println!("{state}: the watch answered with the new key; it is now the active key.");
                    break;
                }
                if Instant::now() >= deadline {
                    println!("no confirmation from the watch within 120 s.");
                    println!("If the watch showed \"paired\", nothing is lost: the daemon keeps the pending key and the");
                    println!("watch's next request completes the pairing. If it showed \"wrong code\", run `themesync pair` again.");
                    break;
                }
            }
        }
        Cmd::PushList { ble, force, dry_run, direct, frame, retries } => {
            let key = beacon::load_key();
            if dry_run {
                let om = Omarchy::from_env()?;
                let built = themelist::build(&om);
                if let Some((a, b)) = &built.collision {
                    bail!("themes {a:?} and {b:?} share slug crc {:#06x}: a SET could not tell them apart; rename one", beacon::slug_id(a));
                }
                for (slug, why) in &built.skipped {
                    eprintln!("skipping {slug}: {why}");
                }
                if built.slugs.is_empty() {
                    bail!("no Omarchy themes found under {} or {}", om.user_themes.display(), om.system_themes.display());
                }
                println!("list: {} themes, {} bytes, crc {:#06x} (omarchy-theme-list order)", built.slugs.len(), built.bytes.len(), built.crc());
                for (i, (slug, packet)) in built.slugs.iter().zip(themelist::decode_list(&built.bytes)?).enumerate() {
                    let d = protocol::decode_v2(&packet)?;
                    println!("  {i:>2} {slug:<24} {:>3} B  {} roles  {}  bg {} fg {} accent {}", packet.len(), d.colors.len(), if d.is_light() { "light" } else { "dark " }, d.get(palette::Role::Background).unwrap().to_hex(), d.get(palette::Role::TextPrimary).unwrap().to_hex(), d.get(palette::Role::Accent).unwrap().to_hex());
                }
                let (key, keyed) = match key {
                    Some(k) => (k, true),
                    None => (([0u8; beacon::KEY_LEN]), false),
                };
                let frames = themelist::frames(&built.bytes, &key, 0, frame.unwrap_or(themelist::MAX_FRAME));
                println!("frames ({} writes to {}; COMMIT signed against nonce 0 — the real one comes from the watch's status):", frames.len(), ble::MINI_CHR_LIST);
                for f in &frames {
                    println!("  {:<34} {}", themelist::describe_frame(f), protocol::to_hex(f));
                }
                if !keyed {
                    println!("(no pairing key in {}: the COMMIT mac above is computed with an all-zero key)", beacon::key_path().display());
                }
                return Ok(());
            }
            if !direct {
                match ipc::request(&Request::PushList { force }, Duration::from_secs(90)).await? {
                    Some(r) if r.ok => {
                        println!("via daemon{}: {}", r.watch.map(|w| format!(" (watch {w})")).unwrap_or_default(), r.message.unwrap_or_default());
                        return Ok(());
                    }
                    Some(r) => bail!("daemon: {}", r.message.unwrap_or_default()),
                    None => {}
                }
            }
            let Some(key) = key else {
                bail!("no pairing key in {} — run `themesync pair` first (the list's COMMIT is keyed)", beacon::key_path().display());
            };
            let om = Omarchy::from_env()?;
            let built = themelist::build(&om);
            for (slug, why) in &built.skipped {
                eprintln!("[themesync] skipping {slug}: {why}");
            }
            if let Some((a, b)) = &built.collision {
                bail!("themes {a:?} and {b:?} share slug crc {:#06x}: a SET could not tell them apart; rename one", beacon::slug_id(a));
            }
            if built.slugs.is_empty() {
                bail!("no Omarchy themes found");
            }
            eprintln!("[themesync] list: {} themes, {} bytes, crc {:#06x}", built.slugs.len(), built.bytes.len(), built.crc());
            let attempts = retries.max(1);
            for attempt in 1..=attempts {
                let (_adapter, peripheral) = find_watch(&ble.options(), attempts).await?;
                let result = ble::push_list(&peripheral, &built.bytes, &key, force, frame, |m| eprintln!("[themesync] {m}")).await;
                let _ = btleplug::api::Peripheral::disconnect(&peripheral).await;
                match result {
                    Ok(outcome) => {
                        println!("{} themes ({} bytes, crc {:#06x}): {outcome}", built.slugs.len(), built.bytes.len(), built.crc());
                        break;
                    }
                    Err(e) if attempt < attempts && !format!("{e:#}").contains("ATT error") => {
                        eprintln!("[themesync] attempt {attempt}: {e:#}; retrying");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
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
        Cmd::Status { ble, json } => {
            if let Some(r) = ipc::request(&Request::Status, Duration::from_secs(10)).await? {
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                    return Ok(());
                }
                match &r.info {
                    Some(i) => {
                        println!("daemon:   running, {} ({})", i.pairing, i.protocol);
                        println!("beacon:   {}{}", match i.beacon.as_str() { "on" => "on the air", "idle" => "idle (nothing to send)", _ => "OFF THE AIR — journalctl --user -u themesync" }, if i.theme.is_empty() { String::new() } else { format!(", theme {}", i.theme) });
                        println!("scan:     {}", match i.scan.as_str() { "on" => "on".to_string(), "off" => "off (no pairing key)".into(), "starting" => "starting".into(), _ => "on, WITHOUT the advertisement monitor: requests slow or lost (BlueZ Experimental = true fixes it)".into() });
                        println!("watch:    {}{}", i.watch.as_deref().unwrap_or("unknown"), i.last_request.as_ref().map(|r| format!(", last request {r}")).unwrap_or_default());
                        println!("counter:  last accepted #{}{}{}", i.ctr_last, if i.ctr_locked { " — LOCKED (counter file unreadable): themesync reset-counter" } else { "" }, if i.stale_rejected > 0 { format!(", {} stale request(s) rejected", i.stale_rejected) } else { String::new() });
                        println!("list:     {}", i.list_push);
                        if !i.hook_installed {
                            println!("hook:     NOT installed — desktop-side theme changes will not reach the watch (themesync install)");
                        }
                    }
                    None => {
                        println!("daemon: {} {}", if r.ok { "ok" } else { "error" }, r.message.unwrap_or_default());
                        if let Some(w) = r.watch { println!("watch:  {w}"); }
                        if let Some(t) = r.theme { println!("theme:  {t}"); }
                    }
                }
                return Ok(());
            }
            if json {
                println!("{}", serde_json::to_string(&Reply::err("daemon not running"))?);
                return Ok(());
            }
            if ble.name.is_none() {
                println!("daemon: not running (systemctl --user status themesync, or `themesync install`); pass --name to query a Theme Protocol v1 device over GATT instead");
                return Ok(());
            }
            let adapter = ble::adapter().await?;
            let watch = ble::connect_with_retry(&adapter, &ble.options(), 2, |m| eprintln!("[themesync] {m}")).await?;
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
        Cmd::ResetCounter => {
            match ipc::request(&Request::ResetCounter, Duration::from_secs(5)).await? {
                Some(r) if r.ok => println!("daemon: {}", r.message.unwrap_or_default()),
                Some(r) => bail!("daemon: {}", r.message.unwrap_or_default()),
                None => {
                    beacon::save_ctr(0)?;
                    println!("counter reset in {} (no daemon running)", beacon::ctr_path().display());
                }
            }
        }
        Cmd::InstallHook { print } => {
            let script = hook_script();
            if print {
                print!("{script}");
                return Ok(());
            }
            #[cfg(target_os = "linux")]
            {
                let om = Omarchy::from_env()?;
                let path = setup::install_hook(&om, &std::env::current_exe()?)?;
                println!("installed {}", path.display());
                println!("Omarchy will run it as `bash {} <theme-slug>` after every theme change.", path.display());
            }
            #[cfg(not(target_os = "linux"))]
            bail!("the Omarchy hook is a Linux thing");
        }
        #[cfg(target_os = "linux")]
        Cmd::Install { no_enable } => setup::install(!no_enable).await?,
        #[cfg(target_os = "linux")]
        Cmd::Uninstall { purge } => setup::uninstall(purge)?,
        #[cfg(target_os = "linux")]
        Cmd::Doctor => {
            let report = setup::doctor().await;
            print!("{}", report.render());
            if report.failed() {
                std::process::exit(1);
            }
        }
        #[cfg(not(target_os = "linux"))]
        Cmd::Install { .. } | Cmd::Uninstall { .. } | Cmd::Doctor => bail!("install/uninstall/doctor are for the Linux daemon"),
    }
    Ok(())
}
