//! Hook -> daemon hand-off over a Unix socket, so a theme change reuses the daemon's open
//! BLE connection instead of paying for a scan + connect every time.
//!
//! Wire format: one JSON object per line, request then reply. Kept trivially simple so the
//! hook could even be `socat` if the binary were unavailable.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Re-read the active theme and push it to the watch.
    Sync,
    /// Push this pre-built packet (hex) as-is — used by `sync --file` through the daemon.
    Push { packet_hex: String },
    Status,
    Ping,
    /// `themesync pair`: hold this key as pending until a watch request verifies with it.
    PairPending { key_hex: String },
    /// `themesync push-list`: send the theme list over GATT (protocol/BEACON.md §3);
    /// `force` sends it even when the watch reports the same crc.
    PushList {
        #[serde(default)]
        force: bool,
    },
    /// `themesync reset-counter`: forget the last accepted request counter (BEACON.md §2),
    /// for a watch that was reflashed and starts counting from 1 again.
    ResetCounter,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// `status` only: the daemon's state as data (`themesync status --json`, the bar widget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<StatusInfo>,
}

/// The daemon's state, for `status --json` and anything that wants to show it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct StatusInfo {
    pub protocol: String,
    /// "paired" / "pairing pending" / "no key"
    pub pairing: String,
    pub paired: bool,
    /// "on" (registered with BlueZ), "idle" (nothing to send: no key or no theme), "off_air" (registration failing)
    pub beacon: String,
    /// "on" / "starting" / "off" (no key) / "degraded" (no advertisement monitor)
    pub scan: String,
    /// Whether the advertisement monitor is registered (None while the scan is off/starting).
    pub monitor: Option<bool>,
    pub theme: String,
    pub ctr_last: u16,
    pub ctr_locked: bool,
    pub stale_rejected: u32,
    pub watch: Option<String>,
    pub last_request: Option<String>,
    pub list_push: String,
    pub hook_installed: bool,
}

impl Reply {
    pub fn ok(msg: impl Into<String>) -> Reply {
        Reply { ok: true, message: Some(msg.into()), connected: None, watch: None, theme: None, info: None }
    }
    pub fn err(msg: impl Into<String>) -> Reply {
        Reply { ok: false, message: Some(msg.into()), connected: None, watch: None, theme: None, info: None }
    }
}

/// `$THEMESYNC_SOCKET`, else `$XDG_RUNTIME_DIR/themesync.sock`. No fallback into `/tmp`: a
/// predictable name in a world-writable directory can be squatted by another local user,
/// and whoever owns this socket can reset the counter, install a pairing key, or push a
/// beacon — it must live in a directory only this user can write.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("THEMESYNC_SOCKET") {
        return Ok(PathBuf::from(p));
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir).join("themesync.sock"));
    }
    anyhow::bail!("XDG_RUNTIME_DIR is not set (not a login session?); set THEMESYNC_SOCKET to a path in a directory only you can write")
}

/// Send one request to a running daemon. `Ok(None)` when no daemon is listening.
pub async fn request(req: &Request, timeout: Duration) -> Result<Option<Reply>> {
    let path = socket_path()?;
    let stream = match UnixStream::connect(&path).await {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            return Ok(None)
        }
        Err(e) => return Err(e).with_context(|| format!("connecting to daemon socket {}", path.display())),
    };
    let (rd, mut wr) = stream.into_split();
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    wr.write_all(line.as_bytes()).await?;
    let mut reader = BufReader::new(rd);
    let mut reply = String::new();
    tokio::time::timeout(timeout, reader.read_line(&mut reply)).await.context("daemon did not answer in time")??;
    if reply.trim().is_empty() {
        return Ok(Some(Reply::err("daemon closed the connection without a reply")));
    }
    Ok(Some(serde_json::from_str(reply.trim()).context("parsing daemon reply")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_format_is_stable() {
        assert_eq!(serde_json::to_string(&Request::Sync).unwrap(), r#"{"cmd":"sync"}"#);
        assert_eq!(
            serde_json::to_string(&Request::Push { packet_hex: "5448".into() }).unwrap(),
            r#"{"cmd":"push","packet_hex":"5448"}"#
        );
        assert_eq!(serde_json::to_string(&Request::PushList { force: true }).unwrap(), r#"{"cmd":"push_list","force":true}"#);
        assert_eq!(serde_json::from_str::<Request>(r#"{"cmd":"push_list"}"#).unwrap(), Request::PushList { force: false });
        let r: Reply = serde_json::from_str(r#"{"ok":true,"message":"sent","connected":true}"#).unwrap();
        assert!(r.ok && r.connected == Some(true));
    }
}
