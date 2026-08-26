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
}

impl Reply {
    pub fn ok(msg: impl Into<String>) -> Reply {
        Reply { ok: true, message: Some(msg.into()), connected: None, watch: None, theme: None }
    }
    pub fn err(msg: impl Into<String>) -> Reply {
        Reply { ok: false, message: Some(msg.into()), connected: None, watch: None, theme: None }
    }
}

/// `$THEMESYNC_SOCKET`, else `$XDG_RUNTIME_DIR/themesync.sock`, else `/tmp/themesync-<uid>.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("THEMESYNC_SOCKET") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("themesync.sock");
    }
    let uid = std::env::var("UID").ok().unwrap_or_else(|| {
        // no libc dep: fall back to the login name, which is unique enough for a socket
        std::env::var("USER").unwrap_or_else(|_| "user".into())
    });
    std::env::temp_dir().join(format!("themesync-{uid}.sock"))
}

/// Send one request to a running daemon. `Ok(None)` when no daemon is listening.
pub async fn request(req: &Request, timeout: Duration) -> Result<Option<Reply>> {
    let path = socket_path();
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
        let r: Reply = serde_json::from_str(r#"{"ok":true,"message":"sent","connected":true}"#).unwrap();
        assert!(r.ok && r.connected == Some(true));
    }
}
