//! `themesync daemon`: the connection-less bridge of `protocol/BEACON.md`.
//!
//! * Advertises the current Omarchy theme as the state beacon, forever (30 ms "burst" for
//!   10 s after a change, then 100 ms).
//! * Scans passively for the watch's requests (NEXT/PREV/SET/RESEND/LIST), checks their
//!   MAC against the pairing key, and drives `omarchy-theme-set` — which fires the hook, which
//!   comes back through the socket, which bumps the beacon. One loop for both directions.
//! * Pushes the theme list over GATT (`protocol/BEACON.md` §3) when a pairing completes, when
//!   the watch asks (LIST), and on `themesync push-list`.
//! * Serves the socket the hook and `themesync sync` talk to.
//! * Until the watch firmware receives beacons, also pushes each change over a one-shot
//!   GATT connection (`--no-gatt` turns that off).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bluer::Address;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, Semaphore};

use crate::beacon::{self, Op, Request as BeaconRequest, Verified};
use crate::omarchy::Omarchy;
use crate::palette::map_source;
use crate::protocol;
use crate::themelist;
use crate::transport::adv::Radio;
use crate::transport::ble::{self, BleOptions};
use crate::transport::ipc::{socket_path, Reply, Request};

const BURST: Duration = Duration::from_millis(30);
const STEADY: Duration = Duration::from_millis(100);
const BURST_FOR: Duration = Duration::from_secs(10);
/// BlueZ silently drops advertisements when the adapter resets (suspend/resume, `bluetoothctl
/// power off`); re-registering on a timer is the cheap insurance for a daemon that runs forever.
const REPUBLISH_EVERY: Duration = Duration::from_secs(300);

fn log(msg: &str) {
    eprintln!("[themesync] {msg}");
}

/// The active Omarchy theme as the watch's v2 packet, plus the previous/next themes in
/// `omarchy-theme-list` order (tags 0x42/0x43) so the watch can show where PREV/NEXT lead.
fn describe(wire: &[u8]) -> String {
    match protocol::decode_v2(wire) {
        Ok(d) => format!("{} bytes; prev {}, next {}", wire.len(), d.prev.map(|n| n.name).unwrap_or_else(|| "-".into()), d.next.map(|n| n.name).unwrap_or_else(|| "-".into())),
        Err(_) => format!("{} bytes", wire.len()),
    }
}

fn current_theme() -> Result<(String, Vec<u8>)> {
    let om = Omarchy::from_env()?;
    let src = om.load_current()?;
    let p = map_source(&src)?;
    let mut wire = protocol::encode_v2(&p, false);
    let cur = om.current_theme_name();
    for (tag, step) in [(protocol::V2_TAG_PREV, -1), (protocol::V2_TAG_NEXT, 1)] {
        let Some(slug) = om.neighbour_of(cur.as_deref(), step) else { continue };
        match om.load_theme(&slug).and_then(|s| map_source(&s).map_err(Into::into)) {
            Ok(np) => protocol::v2_append_neighbour(&mut wire, tag, &protocol::Neighbour::from_palette(&np)),
            Err(e) => log(&format!("neighbour {slug}: {e:#}")),
        }
    }
    Ok((p.name.clone(), wire))
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
                        // A list push (scan + connect + a few writes, with retries) can take a while.
                        tokio::time::timeout(Duration::from_secs(90), orx)
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .unwrap_or_else(|| Reply::err("timed out inside the daemon"))
                    }
                }
            };
            let mut out = serde_json::to_string(&reply).unwrap_or_default();
            out.push('\n');
            let _ = wr.write_all(out.as_bytes()).await;
        });
    }
}


/// What the watch asked for, translated into an Omarchy action. Blocking (theme-set takes
/// seconds), so it runs on the blocking pool.
fn act(om: &Omarchy, req: BeaconRequest) -> Result<Option<String>> {
    let target = match req.op {
        Op::Next => om.neighbour_theme(1),
        Op::Prev => om.neighbour_theme(-1),
        Op::Set => om.list_themes().into_iter().find(|s| beacon::slug_id(s) == req.arg),
        Op::Resend | Op::List => return Ok(None), // handled in the loop, no theme change
    };
    let Some(name) = target else {
        log(&format!("{:?} (arg {:#06x}): no matching theme", req.op, req.arg));
        return Ok(None);
    };
    log(&format!("{:?} -> omarchy-theme-set {name}", req.op));
    om.set_theme(&name)?;
    Ok(Some(name))
}

/// One-shot GATT push of the current theme (the pre-beacon path), detached.
fn gatt_push(opts: BleOptions, wire: Vec<u8>) {
    tokio::spawn(async move {
        let r: Result<()> = async {
            let adapter = ble::adapter().await?;
            let mut delay = Duration::from_millis(500);
            let mut peripheral = None;
            for _ in 0..3 {
                match ble::discover_service(&adapter, &opts, ble::MINI_SERVICE_UUID).await {
                    Ok(p) => { peripheral = Some(p); break; }
                    Err(e) => { log(&format!("gatt: {e:#}; retrying")); tokio::time::sleep(delay).await; delay *= 2; }
                }
            }
            let Some(p) = peripheral else { anyhow::bail!("watch not found for the GATT push") };
            let back = ble::send_colors(&p, &wire, None).await;
            let _ = btleplug::api::Peripheral::disconnect(&p).await;
            let back = back?;
            match protocol::decode_v2(&back) {
                Ok(d) => log(&format!("gatt: watch applied {}", d.summary())),
                Err(e) => log(&format!("gatt: wrote {} bytes, read back {} bytes ({e})", wire.len(), back.len())),
            }
            Ok(())
        }
        .await;
        if let Err(e) = r {
            log(&format!("gatt push failed: {e:#}"));
        }
    });
}

/// Everything a theme-list push needs, so the detached task owns its data.
struct ListPushJob {
    opts: BleOptions,
    key: [u8; beacon::KEY_LEN],
    /// The watch's address from the request just scanned; `None` = any watch with the service.
    addr: Option<String>,
    force: bool,
    reason: &'static str,
}

/// Build the theme list and push it over GATT (protocol/BEACON.md §3), detached. One push at
/// a time (`gate`): a trigger while one is in flight is dropped, not queued — the transfer is
/// idempotent and the next trigger sends the current list anyway. `state` is what `status`
/// shows; `done` gets the outcome when `push-list` is waiting on the socket.
fn list_push(job: ListPushJob, gate: Arc<Semaphore>, state: Arc<Mutex<String>>, done: Option<oneshot::Sender<Reply>>) {
    let Ok(permit) = gate.try_acquire_owned() else {
        log(&format!("list push ({}): another push is in progress; skipped", job.reason));
        if let Some(d) = done {
            let _ = d.send(Reply::err("a list push is already in progress"));
        }
        return;
    };
    tokio::spawn(async move {
        let _permit = permit;
        let r: Result<String> = async {
            let built = tokio::task::spawn_blocking(|| Omarchy::from_env().map(|om| themelist::build(&om))).await??;
            for (slug, why) in &built.skipped {
                log(&format!("list: skipping {slug}: {why}"));
            }
            if built.slugs.is_empty() {
                anyhow::bail!("no Omarchy themes found");
            }
            let summary = format!("{} themes, {} B, crc {:#06x}", built.slugs.len(), built.bytes.len(), built.crc());
            log(&format!("list ({}): {summary}{}", job.reason, job.addr.as_ref().map(|a| format!("; watch {a}")).unwrap_or_default()));
            let adapter = ble::adapter().await?;
            let mut delay = Duration::from_millis(500);
            let mut last = None;
            for attempt in 1..=3 {
                let step = async {
                    let p = ble::find_watch(&adapter, &job.opts, job.addr.as_deref()).await?;
                    let r = ble::push_list(&p, &built.bytes, &job.key, job.force, None, |m| log(&format!("list: {m}"))).await;
                    let _ = btleplug::api::Peripheral::disconnect(&p).await;
                    r
                };
                match step.await {
                    Ok(outcome) => return Ok(format!("{summary}: {outcome}")),
                    Err(e) => {
                        let text = format!("{e:#}");
                        log(&format!("list push attempt {attempt}: {text}"));
                        let watch_said_no = text.contains("ATT error"); // a rejected frame will not pass on retry
                        last = Some(e);
                        if watch_said_no {
                            break;
                        }
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                    }
                }
            }
            Err(last.expect("at least one attempt ran"))
        }
        .await;
        let (reply, text) = match r {
            Ok(s) => {
                log(&format!("list push ({}): {s}", job.reason));
                (Reply { watch: job.addr.clone(), ..Reply::ok(s.clone()) }, s)
            }
            Err(e) => {
                let s = format!("{e:#}");
                log(&format!("list push ({}) failed: {s}", job.reason));
                (Reply::err(s.clone()), format!("failed: {s}"))
            }
        };
        *state.lock().unwrap() = format!("{text} [{}]", job.reason);
        if let Some(d) = done {
            let _ = d.send(reply);
        }
    });
}

pub struct Options {
    pub ble: BleOptions,
    pub gatt: bool,
}

pub async fn run(opts: Options) -> Result<()> {
    let mut radio = Radio::open().await?;
    log(&format!("adapter {}", radio.adapter_name()));
    let mut key = beacon::load_key();
    match &key {
        Some(_) => log(&format!("pairing key loaded from {}", beacon::key_path().display())),
        None => log("no pairing key (run `themesync pair`): watch requests will be ignored"),
    }
    let host = beacon::host_id();
    let om = Omarchy::from_env().ok();

    let (tx, mut rx) = mpsc::channel::<Ipc>(16);
    tokio::spawn(async move {
        if let Err(e) = serve_socket(tx).await {
            log(&format!("socket server died: {e:#}"));
        }
    });

    // Requests from the air, via a second bluer session (the scan borrows the adapter for
    // as long as it runs).
    let (rtx, mut rrx) = mpsc::channel::<(Address, Vec<u8>, bool)>(32);
    tokio::spawn(async move {
        loop {
            match Radio::open().await {
                Ok(r) => {
                    let rtx = rtx.clone();
                    if let Err(e) = r.scan_ours(|addr, data, cached| { let _ = rtx.try_send((addr, data.to_vec(), cached)); }).await {
                        log(&format!("scan stopped: {e:#}; restarting in 3 s"));
                    }
                }
                Err(e) => log(&format!("scan: {e:#}; retrying in 3 s")),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // ---- state ----
    let mut seq: u8 = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) & 0xff) as u8;
    let (mut theme_name, mut theme_wire) = match current_theme() {
        Ok(t) => t,
        Err(e) => {
            log(&format!("no active Omarchy theme yet ({e:#}); beacon idle until the first sync"));
            (String::new(), Vec::new())
        }
    };
    // The last request accepted (or found cached) per address: (nonce, op, arg). Never
    // expires — BlueZ re-delivers a device's last ManufacturerData on every property change
    // for as long as it stays cached, which is far longer than the press was on the air.
    let mut seen: HashMap<String, (u8, u8, u16)> = HashMap::new();
    let mut burst_until: Option<Instant> = None;
    let mut last_request: Option<String> = None;
    // A pending key never expires on its own: the watch may have committed it already (the
    // user entered the code) while its confirmation was lost on the air; the next request it
    // signs with that key completes the pairing, however late.
    let mut pending: Option<[u8; beacon::KEY_LEN]> = beacon::load_pending_key();
    if pending.is_some() {
        log(&format!("pairing: pending key restored from {}", beacon::pending_key_path().display()));
    }
    let mut pair_state = String::from(if pending.is_some() { "pairing pending" } else if key.is_some() { "paired" } else { "no key" });
    // The theme list push (protocol/BEACON.md §3): the watch's address from its last verified
    // request, the one-at-a-time gate, and the last outcome for `status`.
    let mut watch_addr: Option<String> = None;
    let list_gate = Arc::new(Semaphore::new(1));
    let list_state = Arc::new(Mutex::new(String::from("never")));

    async fn publish(radio: &mut Radio, seq: u8, host: u8, wire: &[u8], burst: bool) {
        if wire.is_empty() {
            return;
        }
        let data = beacon::encode_state(seq, host, wire);
        match radio.set_beacon(data, if burst { BURST } else { STEADY }).await {
            Ok(()) => {}
            Err(e) => log(&format!("beacon: {e:#}")),
        }
    }

    publish(&mut radio, seq, host, &theme_wire, true).await;
    if !theme_wire.is_empty() {
        burst_until = Some(Instant::now() + BURST_FOR);
        log(&format!("beacon: seq {seq}, theme {theme_name} ({})", describe(&theme_wire)));
    }

    let mut republish = tokio::time::interval(REPUBLISH_EVERY);
    republish.tick().await; // the first tick fires immediately; the beacon is already up
    loop {
        let relax = async {
            match burst_until {
                Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = republish.tick() => {
                if burst_until.is_none() {
                    publish(&mut radio, seq, host, &theme_wire, false).await;
                }
            }
            Some(ipc) = rx.recv() => {
                if let Request::PushList { force } = ipc.req {
                    match key {
                        Some(k) => list_push(ListPushJob { opts: opts.ble.clone(), key: k, addr: watch_addr.clone(), force, reason: "push-list" }, list_gate.clone(), list_state.clone(), Some(ipc.reply)),
                        None => { let _ = ipc.reply.send(Reply::err("no pairing key: run `themesync pair` first (the list's COMMIT is keyed)")); }
                    }
                    continue;
                }
                let reply = match ipc.req {
                    Request::Ping => Reply { connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("pong") },
                    Request::Status => Reply {
                        theme: Some(theme_name.clone()),
                        connected: Some(true),
                        watch: Some(format!("beacon seq {seq}, host {host:#04x}, key {}, last request {}, list push {}", if key.is_some() { "loaded" } else { "missing" }, last_request.as_deref().unwrap_or("none"), list_state.lock().unwrap())),
                        ..Reply::ok(pair_state.clone())
                    },
                    Request::PushList { .. } => unreachable!("handled above"),
                    Request::PairPending { key_hex } => match protocol::from_hex(&key_hex).and_then(|v| <[u8; beacon::KEY_LEN]>::try_from(v).ok()) {
                        Some(k) => {
                            pending = Some(k);
                            if let Err(e) = beacon::save_pending_key(&k) {
                                log(&format!("pairing: could not persist the pending key: {e}"));
                            }
                            pair_state = "pairing pending".into();
                            log("pairing: pending key held; waiting for a request signed with it");
                            Reply::ok("pending")
                        }
                        None => Reply::err("key_hex must be 32 hex digits"),
                    },
                    Request::Sync => match current_theme() {
                        Ok((name, wire)) => {
                            theme_name = name.clone();
                            theme_wire = wire.clone();
                            seq = seq.wrapping_add(1);
                            publish(&mut radio, seq, host, &theme_wire, true).await;
                            burst_until = Some(Instant::now() + BURST_FOR);
                            log(&format!("beacon: seq {seq}, theme {name} ({})", describe(&theme_wire)));
                            if opts.gatt { gatt_push(opts.ble.clone(), wire); }
                            Reply { theme: Some(name), connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("sent") }
                        }
                        Err(e) => Reply::err(format!("{e:#}")),
                    },
                    Request::Push { packet_hex } => match protocol::from_hex(&packet_hex) {
                        Some(wire) => {
                            theme_name = protocol::decode_v2(&wire).ok().and_then(|d| d.name).unwrap_or_default();
                            theme_wire = wire.clone();
                            seq = seq.wrapping_add(1);
                            publish(&mut radio, seq, host, &theme_wire, true).await;
                            burst_until = Some(Instant::now() + BURST_FOR);
                            log(&format!("beacon: seq {seq}, pushed packet ({} bytes)", wire.len()));
                            if opts.gatt { gatt_push(opts.ble.clone(), wire); }
                            Reply { connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("sent") }
                        }
                        None => Reply::err("packet_hex is not hex"),
                    },
                };
                let _ = ipc.reply.send(reply);
            }
            Some((addr, data, cached)) = rrx.recv() => {
                if data.get(1) != Some(&beacon::KIND_REQUEST) { continue; }
                if cached {
                    // history from BlueZ's cache: remember it so it is never acted on
                    if data.len() == beacon::REQUEST_LEN {
                        seen.insert(addr.to_string(), (data[2], data[3], u16::from_le_bytes([data[4], data[5]])));
                        log(&format!("{addr}: cached request (nonce {:#04x}) marked as already seen", data[2]));
                    }
                    continue;
                }
                if key.is_none() && pending.is_none() { continue; }
                match beacon::decode_request_with(key.as_ref(), pending.as_ref(), &data) {
                    Err(beacon::RequestError::BadMac) => log(&format!("{addr}: request with a bad MAC ignored")),
                    Err(e) => log(&format!("{addr}: {e}")),
                    Ok((req, Verified::Pending)) => {
                        let k = pending.take().unwrap();
                        beacon::clear_pending_key();
                        match beacon::save_key(&k) {
                            Ok(p) => log(&format!("pairing: confirmed by {addr}; key saved to {}", p.display())),
                            Err(e) => log(&format!("pairing: confirmed by {addr} but saving the key failed: {e}")),
                        }
                        key = Some(k);
                        seen.insert(addr.to_string(), (req.nonce, req.op.code(), req.arg));
                        pair_state = format!("paired with {addr}");
                        last_request = Some(format!("{:?} (nonce {:#04x}) from {addr} [pairing confirmation]", req.op, req.nonce));
                        publish(&mut radio, seq, host, &theme_wire, true).await;
                        burst_until = Some(Instant::now() + BURST_FOR);
                        // "The list arrives with pairing": push it now, over a connection to that watch.
                        watch_addr = Some(addr.to_string());
                        list_push(ListPushJob { opts: opts.ble.clone(), key: k, addr: watch_addr.clone(), force: false, reason: "pairing" }, list_gate.clone(), list_state.clone(), None);
                    }
                    Ok((req, Verified::Active)) => {
                        let a = addr.to_string();
                        let k = (req.nonce, req.op.code(), req.arg);
                        if seen.get(&a) == Some(&k) {
                            continue; // the same press: still on the air, or BlueZ's cache of it
                        }
                        seen.insert(a.clone(), k);
                        watch_addr = Some(a.clone());
                        last_request = Some(format!("{:?} (nonce {:#04x}) from {a}", req.op, req.nonce));
                        log(&format!("{a}: {:?} nonce {:#04x}", req.op, req.nonce));
                        if req.op == Op::Resend {
                            publish(&mut radio, seq, host, &theme_wire, true).await;
                            burst_until = Some(Instant::now() + BURST_FOR);
                            continue;
                        }
                        if req.op == Op::List {
                            // The watch asked for a refresh: always send (its request stays on the
                            // air until our COMMIT lands), signed with the key that verified it.
                            let key = key.expect("Verified::Active implies an active key");
                            list_push(ListPushJob { opts: opts.ble.clone(), key, addr: watch_addr.clone(), force: true, reason: "request" }, list_gate.clone(), list_state.clone(), None);
                            continue;
                        }
                        if let Some(om) = &om {
                            let om = om.clone();
                            tokio::task::spawn_blocking(move || {
                                if let Err(e) = act(&om, req) { log(&format!("{:?} failed: {e:#}", req.op)); }
                            });
                        }
                    }
                }
            }
            _ = relax => {
                burst_until = None;
                publish(&mut radio, seq, host, &theme_wire, false).await;
            }
        }
    }
}
