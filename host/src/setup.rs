//! `themesync install` / `uninstall` / `doctor`: the parts of a release that a package cannot
//! do for one user — the systemd user unit, the Omarchy hook, and a diagnosis of everything
//! the daemon depends on (Omarchy, BlueZ, the controller, the unit, the key).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::beacon;
use crate::omarchy::Omarchy;
use crate::transport::ipc::{self, Request};

pub const UNIT_NAME: &str = "themesync.service";
/// Where a package installs the unit; `install` then only enables it.
pub const PACKAGED_UNIT: &str = "/usr/lib/systemd/user/themesync.service";
pub const PACKAGED_BIN: &str = "/usr/bin/themesync";

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")
}

pub fn user_unit_path() -> Result<PathBuf> {
    Ok(home()?.join(".config/systemd/user").join(UNIT_NAME))
}

/// The unit, with `exe` as the daemon binary. Same text as `systemd/themesync.service` in
/// the repo, which is the packaged copy (`/usr/bin/themesync`).
pub fn unit_text(exe: &Path) -> String {
    format!(
        "# Omarchy theme beacon + watch request scanner. Written by `themesync install`.\n\
         # Tied to the graphical session like Omarchy's own user units: the daemon needs the\n\
         # session environment (OMARCHY_PATH, WAYLAND_DISPLAY) for omarchy-theme-set.\n\
         [Unit]\n\
         Description=Omarchy theme beacon + watch request scanner (themesync daemon)\n\
         After=bluetooth.target graphical-session.target\n\
         PartOf=graphical-session.target\n\
         Wants=bluetooth.target\n\
         ConditionEnvironment=WAYLAND_DISPLAY\n\
         ConditionPathIsDirectory=/sys/class/bluetooth\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} daemon --no-gatt\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        exe.display()
    )
}

pub fn hook_text(exe: &Path) -> String {
    format!(
        "#!/bin/bash\n\
         # Omarchy theme-set hook: push the new theme to the watch. Written by `themesync install`.\n\
         # Omarchy runs this synchronously (bash <file> <theme-slug>) after the theme directory swap\n\
         # and all app retints, so return fast: the daemon (or a background one-shot) does the BLE work.\n\
         THEMESYNC=\"${{THEMESYNC_BIN:-{}}}\"\n\
         if ! command -v \"$THEMESYNC\" >/dev/null 2>&1; then\n\
         \x20 echo \"themesync hook: $THEMESYNC not found; the watch will not follow this theme change (run themesync install)\" | systemd-cat -t themesync -p warning 2>/dev/null\n\
         \x20 exit 0\n\
         fi\n\
         \"$THEMESYNC\" sync --async >/dev/null 2>&1 || true\n",
        exe.display()
    )
}

fn systemctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("systemctl").arg("--user").args(args).output().context("running systemctl --user")
}

fn systemctl_ok(args: &[&str]) -> bool {
    systemctl(args).map(|o| o.status.success()).unwrap_or(false)
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("resolving this binary's path")
}

pub fn install_hook(om: &Omarchy, exe: &Path) -> Result<PathBuf> {
    let dir = om.hooks_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("themesync");
    std::fs::write(&path, hook_text(exe))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Unit + hook + enable, then the doctor. `enable = false` writes the files only.
pub async fn install(enable: bool) -> Result<()> {
    let exe = current_exe()?;
    let om = Omarchy::from_env().context("no Omarchy install found")?;
    let hook = install_hook(&om, &exe)?;
    println!("hook      {}", hook.display());

    let packaged = exe == Path::new(PACKAGED_BIN) && Path::new(PACKAGED_UNIT).is_file();
    let user_unit = user_unit_path()?;
    if packaged {
        if user_unit.is_file() {
            std::fs::remove_file(&user_unit)?;
            println!("unit      removed {} (the packaged unit {} takes over)", user_unit.display(), PACKAGED_UNIT);
        } else {
            println!("unit      {} (packaged)", PACKAGED_UNIT);
        }
    } else {
        std::fs::create_dir_all(user_unit.parent().unwrap())?;
        std::fs::write(&user_unit, unit_text(&exe))?;
        println!("unit      {}", user_unit.display());
    }
    let o = systemctl(&["daemon-reload"])?;
    if !o.status.success() {
        bail!("systemctl --user daemon-reload failed: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    if enable {
        let o = systemctl(&["enable", "--now", UNIT_NAME])?;
        if !o.status.success() {
            bail!("systemctl --user enable --now {UNIT_NAME} failed: {}", String::from_utf8_lossy(&o.stderr).trim());
        }
        println!("service   enabled and started (journalctl --user -u themesync -f)");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    println!();
    let report = doctor().await;
    print!("{}", report.render());
    println!();
    if beacon::load_key().is_none() {
        println!("next: `themesync pair` with the watch nearby, then `themesync status`.");
    } else {
        println!("next: `themesync status`; `themesync pair` if the watch was reflashed.");
    }
    Ok(())
}

/// Disable + stop the service, remove the unit (if it is the user copy) and the hook.
/// `purge` also removes `~/.config/themesync` (the pairing key, the counter, the watch's
/// address).
pub fn uninstall(purge: bool) -> Result<()> {
    if systemctl_ok(&["is-enabled", "--quiet", UNIT_NAME]) || systemctl_ok(&["is-active", "--quiet", UNIT_NAME]) {
        let _ = systemctl(&["disable", "--now", UNIT_NAME]);
        println!("service   disabled and stopped");
    }
    let user_unit = user_unit_path()?;
    if user_unit.is_file() {
        std::fs::remove_file(&user_unit)?;
        let _ = systemctl(&["daemon-reload"]);
        println!("unit      removed {}", user_unit.display());
    }
    if let Ok(om) = Omarchy::from_env() {
        let hook = om.hooks_dir().join("themesync");
        if hook.is_file() {
            std::fs::remove_file(&hook)?;
            println!("hook      removed {}", hook.display());
        }
    }
    if let Some(sock) = ipc::socket_path().ok().filter(|p| p.exists()) {
        let _ = std::fs::remove_file(sock);
    }
    let cfg = beacon::key_path().parent().map(|p| p.to_path_buf());
    match cfg {
        Some(dir) if dir.is_dir() && purge => {
            std::fs::remove_dir_all(&dir)?;
            println!("config    removed {} (pairing key, counter, watch address)", dir.display());
        }
        Some(dir) if dir.is_dir() => println!("config    kept {} (pairing key etc.; `--purge` removes it)", dir.display()),
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub level: Level,
    pub name: &'static str,
    pub detail: String,
    /// What to do about it; empty when `Ok`.
    pub fix: String,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check { level: Level::Ok, name, detail: detail.into(), fix: String::new() }
    }
    pub fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check { level: Level::Warn, name, detail: detail.into(), fix: fix.into() }
    }
    pub fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check { level: Level::Fail, name, detail: detail.into(), fix: fix.into() }
    }
}

pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.level == Level::Fail)
    }
    pub fn render(&self) -> String {
        let mut out = String::new();
        for c in &self.checks {
            let tag = match c.level { Level::Ok => " ok ", Level::Warn => "WARN", Level::Fail => "FAIL" };
            out.push_str(&format!("[{tag}] {:<14} {}\n", c.name, c.detail));
            if !c.fix.is_empty() {
                out.push_str(&format!("       {:<14} → {}\n", "", c.fix));
            }
        }
        out
    }
}

fn bluetooth_conf_experimental() -> Option<bool> {
    let text = std::fs::read_to_string("/etc/bluetooth/main.conf").ok()?;
    let mut value = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Experimental") {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                value = Some(v.trim().eq_ignore_ascii_case("true"));
            }
        }
    }
    Some(value.unwrap_or(false))
}

/// Everything the daemon depends on, checked from the outside. Read-only.
pub async fn doctor() -> Report {
    let mut c = Vec::new();

    // ---- Omarchy ----
    match Omarchy::from_env() {
        Ok(om) => {
            let version = std::fs::read_to_string(om.bin_dir.parent().map(|p| p.join("version")).unwrap_or_default()).map(|v| v.trim().to_string()).unwrap_or_else(|_| "?".into());
            c.push(Check::ok("omarchy", format!("{} (version {version})", om.bin_dir.parent().map(|p| p.display().to_string()).unwrap_or_default())));
            for bin in ["omarchy-theme-set", "omarchy-theme-color"] {
                let p = om.bin_dir.join(bin);
                if p.is_file() {
                    c.push(Check::ok(if bin == "omarchy-theme-set" { "theme-set" } else { "theme-color" }, p.display().to_string()));
                } else {
                    c.push(Check::fail(if bin == "omarchy-theme-set" { "theme-set" } else { "theme-color" }, format!("{bin} not found in {}", om.bin_dir.display()), "is this Omarchy ≥ 4.0? set OMARCHY_PATH if it lives elsewhere"));
                }
            }
            match om.current_theme_name() {
                Some(t) => c.push(Check::ok("theme", format!("current theme {t}"))),
                None => c.push(Check::warn("theme", "no current theme (nothing under ~/.local/state/omarchy/current)", "pick a theme once: omarchy theme set <name>")),
            }
            let hook = om.hooks_dir().join("themesync");
            match std::fs::read_to_string(&hook) {
                Ok(text) => {
                    let exe = text.lines().find_map(|l| l.trim().strip_prefix("THEMESYNC=\"${THEMESYNC_BIN:-")).and_then(|r| r.strip_suffix("}\"")).map(str::to_string);
                    match exe {
                        Some(e) if Path::new(&e).is_file() => c.push(Check::ok("hook", format!("{} → {e}", hook.display()))),
                        Some(e) => c.push(Check::fail("hook", format!("{} points at {e}, which does not exist", hook.display()), "themesync install")),
                        None => c.push(Check::warn("hook", format!("{} is not the generated hook", hook.display()), "themesync install (or keep yours if it runs `themesync sync --async`)")),
                    }
                }
                Err(_) => c.push(Check::fail("hook", format!("no hook at {}", hook.display()), "themesync install — without it desktop-side theme changes never reach the watch")),
            }
        }
        Err(e) => c.push(Check::fail("omarchy", format!("{e:#}"), "themesync is for Omarchy (https://omarchy.org); set OMARCHY_PATH if it is installed elsewhere")),
    }

    // ---- session ----
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => c.push(Check::ok("session", format!("XDG_RUNTIME_DIR {}", Path::new(&d).display()))),
        None => c.push(Check::fail("session", "XDG_RUNTIME_DIR is not set", "run this from your desktop session (the daemon socket lives there)")),
    }

    // ---- BlueZ / controller ----
    #[cfg(target_os = "linux")]
    {
        c.extend(crate::transport::adv::probe().await);
        match bluetooth_conf_experimental() {
            Some(true) => c.push(Check::ok("bluez-conf", "/etc/bluetooth/main.conf: Experimental = true")),
            Some(false) => c.push(Check::warn("bluez-conf", "/etc/bluetooth/main.conf: Experimental is not true", "sudo sed -i 's/^#\\?Experimental *=.*/Experimental = true/' /etc/bluetooth/main.conf && sudo systemctl restart bluetooth — without it requests from the watch are slow (0–2 s) or lost")),
            None => c.push(Check::warn("bluez-conf", "/etc/bluetooth/main.conf not readable", "check that bluez is installed")),
        }
    }

    // ---- unit ----
    #[cfg(target_os = "linux")]
    {
        let user_unit = user_unit_path().ok();
        let unit_file = match (&user_unit, Path::new(PACKAGED_UNIT).is_file()) {
            (Some(u), _) if u.is_file() => Some(u.display().to_string()),
            (_, true) => Some(PACKAGED_UNIT.to_string()),
            _ => None,
        };
        match unit_file {
            None => c.push(Check::fail("unit", "no themesync.service for this user", "themesync install")),
            Some(f) => {
                let enabled = systemctl_ok(&["is-enabled", "--quiet", UNIT_NAME]);
                let active = systemctl_ok(&["is-active", "--quiet", UNIT_NAME]);
                match (enabled, active) {
                    (true, true) => c.push(Check::ok("unit", format!("{f}: enabled, running"))),
                    (true, false) => c.push(Check::warn("unit", format!("{f}: enabled, not running"), "systemctl --user start themesync; journalctl --user -u themesync -n 30")),
                    (false, true) => c.push(Check::warn("unit", format!("{f}: running but not enabled"), "systemctl --user enable themesync")),
                    (false, false) => c.push(Check::warn("unit", format!("{f}: not enabled"), "themesync install, or systemctl --user enable --now themesync")),
                }
            }
        }
    }

    // ---- key / counter / watch ----
    match beacon::load_key() {
        Some(_) => c.push(Check::ok("key", format!("pairing key in {}", beacon::key_path().display()))),
        None => c.push(Check::warn("key", "no pairing key", "themesync pair (with the watch nearby)")),
    }
    match beacon::load_ctr() {
        Ok(n) => c.push(Check::ok("counter", format!("last accepted request #{n}"))),
        Err(e) => c.push(Check::fail("counter", e, "themesync reset-counter (or themesync pair)")),
    }
    match beacon::load_watch_addr() {
        Some(a) => c.push(Check::ok("watch", format!("address {a}"))),
        None => c.push(Check::warn("watch", "no watch address yet (learned at pairing / from the first request)", "themesync pair")),
    }

    // ---- daemon ----
    match ipc::request(&Request::Status, std::time::Duration::from_secs(5)).await {
        Ok(Some(r)) => {
            let info = r.info.clone();
            let summary = info.as_ref().map(|i| format!("{}, beacon {}, scan {}", r.message.clone().unwrap_or_default(), i.beacon, i.scan)).unwrap_or_else(|| r.message.clone().unwrap_or_default());
            match info {
                Some(i) if i.beacon == "off_air" => c.push(Check::fail("daemon", summary, "journalctl --user -u themesync -n 30")),
                Some(i) if i.monitor == Some(false) => c.push(Check::warn("daemon", summary, "see bluez-conf above")),
                _ => c.push(Check::ok("daemon", summary)),
            }
        }
        Ok(None) => c.push(Check::fail("daemon", "not running (nothing answers on the socket)", "systemctl --user start themesync")),
        Err(e) => c.push(Check::fail("daemon", format!("{e:#}"), "")),
    }

    Report { checks: c }
}
