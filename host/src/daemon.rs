//! `omawatch daemon`: keep one BLE connection to the watch open, push themes on request
//! (from the Omarchy hook via the socket), and act on the watch's own requests
//! (next/prev/toggle) by driving `omarchy-theme-set` — which fires the hook, which comes
//! back through the socket, which pushes the new palette. One path for both directions.

use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::omarchy::Omarchy;
use crate::palette::map_source;
use crate::protocol::{self, Control};
use crate::transport::ble::{self, BleOptions, Watch, CHR_CONTROL, CHR_STATUS};
use crate::transport::ipc::{socket_path, Reply, Request};

fn log(msg: &str) {
    eprintln!("[omawatch] {msg}");
}

/// Build the packet for the active Omarchy theme.
fn current_packet() -> Result<(String, Vec<u8>)> {
    let src = Omarchy::from_env()?.load_current()?;
    let p = map_source(&src)?;
    Ok((p.name.clone(), protocol::encode_theme(&p)))
}

struct Ipc {
    req: Request,
    reply: tokio::sync::oneshot::Sender<Reply>,
}

async fn serve_socket(tx: mpsc::Sender<Ipc>) -> Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    log(&format!("listening on {}", path.display()));
    loop {
        let (stream, _) = listener.accept().await?;
        let tx = tx.clone();
        tokio::spawn(async move {
            let (rd, mut wr) = stream.into_split();
            let mut line = String::new();
            if BufReader::new(rd).read_line(&mut line).await.is_err() || line.trim().is_empty() {
                return;
            }
            let reply = match serde_json::from_str::<Request>(line.trim()) {
                Err(e) => Reply::err(format!("bad request: {e}")),
                Ok(req) => {
                    let (otx, orx) = tokio::sync::oneshot::channel();
                    if tx.send(Ipc { req, reply: otx }).await.is_err() {
                        Reply::err("daemon shutting down")
                    } else {
                        tokio::time::timeout(Duration::from_secs(20), orx)
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .unwrap_or_else(|| Reply::err("timed out waiting for the BLE link"))
                    }
                }
            };
            let mut out = serde_json::to_string(&reply).unwrap_or_default();
            out.push('\n');
            let _ = wr.write_all(out.as_bytes()).await;
        });
    }
}

/// What the watch asked for, translated into an Omarchy action.
fn handle_control(om: &Omarchy, c: Control) -> Result<Option<String>> {
    let target = match c {
        Control::NextTheme => om.neighbour_theme(1),
        Control::PrevTheme => om.neighbour_theme(-1),
        Control::ToggleMode => om.opposite_mode_theme(),
        Control::Resend => return Ok(None),
    };
    let Some(name) = target else {
        log(&format!("{c:?}: no suitable theme found"));
        return Ok(None);
    };
    log(&format!("{c:?} -> omarchy-theme-set {name}"));
    om.set_theme(&name)?;
    Ok(Some(name))
}

pub async fn run(opts: BleOptions) -> Result<()> {
    let adapter = ble::adapter().await?;
    let (tx, mut rx) = mpsc::channel::<Ipc>(16);
    tokio::spawn(async move {
        if let Err(e) = serve_socket(tx).await {
            log(&format!("socket server died: {e:#}"));
        }
    });
    let om = Omarchy::from_env().ok();

    loop {
        // ---- connect (forever, with backoff) ----
        let watch: Watch = ble::connect_with_retry(&adapter, &opts, 0, log).await?;
        log(&format!("connected to {}", watch.name));
        if let Ok(Some(info)) = watch.info().await {
            log(&format!(
                "watch protocol v{}..v{}, {} colour slots, features {:#04x}",
                info.proto_min, info.proto_max, info.max_colors, info.features
            ));
            if !info.supports(protocol::VERSION) {
                log("watch does not speak protocol v1; nothing to do until one side is updated");
            }
        }
        let mut notifications = match watch.subscribe().await {
            Ok(n) => Some(n),
            Err(e) => {
                log(&format!("no notifications: {e:#}"));
                None
            }
        };

        // Push the current theme right away: the watch may have rebooted with a stale one.
        match current_packet() {
            Ok((name, packet)) => match watch.send_theme(&packet).await {
                Ok(_) => log(&format!("pushed {name} ({} bytes)", packet.len())),
                Err(e) => log(&format!("initial push failed: {e:#}")),
            },
            Err(e) => log(&format!("no active Omarchy theme to push yet: {e:#}")),
        }

        // ---- serve until the link drops ----
        let mut keepalive = tokio::time::interval(Duration::from_secs(5));
        keepalive.tick().await;
        loop {
            tokio::select! {
                Some(ipc) = rx.recv() => {
                    let reply = match ipc.req {
                        Request::Ping => Reply { ok: true, connected: Some(watch.is_connected().await), watch: Some(watch.name.clone()), ..Reply::ok("pong") },
                        Request::Status => match watch.status().await {
                            Ok(Some(s)) => Reply { connected: Some(true), watch: Some(watch.name.clone()), ..Reply::ok(format!("{s:?}")) },
                            Ok(None) => Reply::ok("watch has no Status characteristic"),
                            Err(e) => Reply::err(format!("{e:#}")),
                        },
                        Request::Sync => match current_packet() {
                            Ok((name, packet)) => match watch.send_theme(&packet).await {
                                Ok(_) => { log(&format!("pushed {name}")); Reply { theme: Some(name), connected: Some(true), watch: Some(watch.name.clone()), ..Reply::ok("sent") } }
                                Err(e) => Reply::err(format!("{e:#}")),
                            },
                            Err(e) => Reply::err(format!("{e:#}")),
                        },
                        Request::Push { packet_hex } => match protocol::from_hex(&packet_hex) {
                            Some(packet) => match watch.send_theme(&packet).await {
                                Ok(_) => Reply { connected: Some(true), watch: Some(watch.name.clone()), ..Reply::ok("sent") },
                                Err(e) => Reply::err(format!("{e:#}")),
                            },
                            None => Reply::err("packet_hex is not hex"),
                        },
                    };
                    let _ = ipc.reply.send(reply);
                }
                Some(n) = async { match notifications.as_mut() { Some(s) => s.next().await, None => std::future::pending().await } } => {
                    if n.uuid == CHR_CONTROL {
                        match Control::decode(&n.value) {
                            Ok(Control::Resend) => {
                                if let Ok((name, packet)) = current_packet() {
                                    match watch.send_theme(&packet).await {
                                        Ok(_) => log(&format!("resend -> pushed {name}")),
                                        Err(e) => log(&format!("resend failed: {e:#}")),
                                    }
                                }
                            }
                            Ok(c) => {
                                if let Some(om) = &om {
                                    // omarchy-theme-set blocks for a few seconds (app retints);
                                    // keep the BLE loop responsive.
                                    let om = om.clone();
                                    tokio::task::spawn_blocking(move || {
                                        if let Err(e) = handle_control(&om, c) { log(&format!("{c:?} failed: {e:#}")); }
                                    });
                                }
                            }
                            Err(e) => log(&format!("bad control packet {:02x?}: {e}", n.value)),
                        }
                    } else if n.uuid == CHR_STATUS {
                        if let Ok(s) = protocol::Status::decode(&n.value) {
                            log(&format!("status: {:?} crc {:#06x} mode {}", s.result, s.applied_crc, s.mode.as_str()));
                        }
                    }
                }
                _ = keepalive.tick() => {
                    if !watch.is_connected().await {
                        log("link lost; reconnecting");
                        break;
                    }
                }
            }
        }
        watch.disconnect().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
