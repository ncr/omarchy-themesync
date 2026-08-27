//! `themesync daemon`: the connection-less bridge of `protocol/BEACON.md` (v3).
//!
//! * Advertises the current Omarchy theme as the state beacon, signed with the pairing key,
//!   forever (30 ms "burst" for 10 s after a change, then a fixed 80 ms).
//! * Scans for the watch's requests (SET/RESEND/LIST), checks their MAC against the pairing
//!   key and their counter against the last one accepted, and drives `omarchy-theme-set` —
//!   which fires the hook, which comes back through the socket, which refreshes the beacon.
//!   One loop for both directions.
//! * Pushes the theme list over GATT (`protocol/BEACON.md` §3) when a pairing completes, when
//!   the watch asks (LIST, or a SET naming a theme this desktop does not have), and on
//!   `themesync push-list`.
//! * Serves the socket the hook and `themesync sync` talk to.
//! * Until the watch firmware receives beacons, also pushes each change over a one-shot
//!   GATT connection (`--no-gatt` turns that off).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bluer::Address;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, Semaphore};

use crate::beacon::{self, Op, Verified};
use crate::omarchy::Omarchy;
use crate::palette::map_source;
use crate::protocol;
use crate::themelist;
use crate::transport::adv::Radio;
use crate::transport::ble::{self, BleOptions};
use crate::transport::ipc::{socket_path, Reply, Request};

const BURST: Duration = Duration::from_millis(30);
/// Fixed (min == max). With the controller's 0–10 ms advDelay the worst gap between two
/// events is 90 ms, inside the watch's 120 ms scan window with margin (BEACON.md §1).
const STEADY: Duration = Duration::from_millis(80);
const BURST_FOR: Duration = Duration::from_secs(10);
/// BlueZ silently drops advertisements when the adapter resets (suspend/resume, `bluetoothctl
/// power off`); re-registering on a timer is the cheap insurance for a daemon that runs forever.
const REPUBLISH_EVERY: Duration = Duration::from_secs(60);

type Key = [u8; beacon::KEY_LEN];

fn log(msg: &str) {
    eprintln!("[themesync] {msg}");
}

/// The active Omarchy theme: its slug and the v2 packet (what GATT `…0002` takes and what
/// the beacon carries).
fn current_theme() -> Result<(String, Vec<u8>)> {
    let om = Omarchy::from_env()?;
    let src = om.load_current()?;
    let p = map_source(&src)?;
    Ok((p.name.clone(), protocol::encode_v2(&p, false)))
}

/// The state beacon for this theme and this echo (the last request counter accepted) —
/// empty when there is no key yet, because a beacon without a MAC is one no paired watch
/// would apply.
fn sign(key: Option<&Key>, v2: &[u8], echo: u16) -> Vec<u8> {
    match key {
        Some(k) if !v2.is_empty() => beacon::encode_state(k, v2, echo),
        _ => Vec::new(),
    }
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

/// A SET the scanner accepted: the watch's counter (for the log) and the slug it asked for.
type SetJob = (u16, String);

/// One thread runs `omarchy-theme-set` for the watch (it takes seconds; two at once would
/// race each other). SET is absolute, so nothing queues: while one runs, only the newest
/// SET received is kept — the ones it replaced are never applied (the watch has already
/// moved on). The answer is the beacon itself: once the hook reports the new theme, the
/// beacon shows the slug the watch asked for.
fn spawn_actor(om: Omarchy) -> std::sync::mpsc::Sender<SetJob> {
    let (tx, rx) = std::sync::mpsc::channel::<SetJob>();
    std::thread::spawn(move || {
        while let Ok(mut job) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                log(&format!("SET #{} ({}) superseded by #{}", job.0, job.1, newer.0));
                job = newer;
            }
            let (ctr, name) = job;
            log(&format!("SET #{ctr} -> omarchy-theme-set {name}"));
            if let Err(e) = om.set_theme(&name) {
                log(&format!("SET {name} failed: {e:#}"));
            }
        }
    });
    tx
}

/// One-shot GATT push of the current theme (the pre-beacon path), detached.
fn gatt_push(opts: BleOptions, v2: Vec<u8>) {
    tokio::spawn(async move {
        let r: Result<()> = async {
            let adapter = ble::adapter().await?;
            let mut delay = Duration::from_millis(500);
            let mut peripheral = None;
            for _ in 0..3 {
                match ble::find_watch(&adapter, &opts, beacon::load_watch_addr().as_deref()).await {
                    Ok(p) => { peripheral = Some(p); break; }
                    Err(e) => { log(&format!("gatt: {e:#}; retrying")); tokio::time::sleep(delay).await; delay *= 2; }
                }
            }
            let Some(p) = peripheral else { anyhow::bail!("watch not found for the GATT push") };
            let back = ble::send_colors(&p, &v2, None).await;
            let _ = btleplug::api::Peripheral::disconnect(&p).await;
            let back = back?;
            match protocol::decode_v2(&back) {
                Ok(d) => log(&format!("gatt: watch applied {}", d.summary())),
                Err(e) => log(&format!("gatt: wrote {} bytes, read back {} bytes ({e})", v2.len(), back.len())),
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
    key: Key,
    /// The watch's address; `None` = find it by name.
    addr: Option<String>,
    force: bool,
    reason: &'static str,
    /// Told when the push succeeded, so the loop can burst the beacon: the watch forgets its
    /// applied theme on a list COMMIT and repaints from the first beacon it sees.
    after: mpsc::Sender<&'static str>,
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
            if let Some((a, b)) = &built.collision {
                anyhow::bail!("themes {a:?} and {b:?} share slug crc {:#06x}: a SET could not tell them apart; rename one", beacon::slug_id(a));
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
                let _ = job.after.try_send(job.reason);
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

/// Register the beacon; on failure (BlueZ mid-reset, "maximum advertisements reached" from
/// the drop/register race) try once more after 200 ms rather than staying dark until the
/// next republish tick.
async fn publish(radio: &mut Radio, wire: &[u8], burst: bool) {
    if wire.is_empty() {
        return;
    }
    let interval = if burst { BURST } else { STEADY };
    if let Err(e) = radio.set_beacon(wire.to_vec(), interval).await {
        log(&format!("beacon: {e:#}; retrying in 200 ms"));
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Err(e) = radio.set_beacon(wire.to_vec(), interval).await {
            log(&format!("beacon: {e:#}"));
        }
    }
}

pub async fn run(opts: Options) -> Result<()> {
    let mut radio = Radio::open().await?;
    log(&format!("adapter {}", radio.adapter_name()));
    let mut key: Option<Key> = beacon::load_key();
    match &key {
        Some(_) => log(&format!("pairing key loaded from {}", beacon::key_path().display())),
        None => log("no pairing key (run `themesync pair`): no beacon, watch requests ignored"),
    }
    let om = Omarchy::from_env().ok();
    let actor = om.clone().map(spawn_actor);

    let (tx, mut rx) = mpsc::channel::<Ipc>(16);
    tokio::spawn(async move {
        if let Err(e) = serve_socket(tx).await {
            log(&format!("socket server died: {e:#}"));
        }
    });

    // Requests from the air, via a second bluer session (the scan borrows the adapter for
    // as long as it runs).
    let (rtx, mut rrx) = mpsc::channel::<(Address, Vec<u8>)>(32);
    tokio::spawn(async move {
        loop {
            match Radio::open().await {
                Ok(r) => {
                    let rtx = rtx.clone();
                    if let Err(e) = r.scan_ours(|addr, data| { let _ = rtx.try_send((addr, data.to_vec())); }).await {
                        log(&format!("scan stopped: {e:#}; restarting in 3 s"));
                    }
                }
                Err(e) => log(&format!("scan: {e:#}; retrying in 3 s")),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // ---- state ----
    let (mut theme_name, mut theme_v2) = match current_theme() {
        Ok(t) => t,
        Err(e) => {
            log(&format!("no active Omarchy theme yet ({e:#}); beacon idle until the first sync"));
            (String::new(), Vec::new())
        }
    };
    let mut burst_until: Option<Instant> = None;
    let mut last_request: Option<String> = None;
    // The request counter (BEACON.md §2): accept only ctr > last accepted under the active
    // key. This one rule covers repeats of the same press, BlueZ's cached copy of an old
    // request re-delivered on any property change, a request still on the air across a
    // daemon restart, and replays.
    let mut ctr_last: u16 = beacon::load_ctr();
    let mut theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
    let mut ctr_rejected: u32 = 0;
    let mut last_rejected: Option<u16> = None;
    let mut last_bad: Option<Vec<u8>> = None;
    // A pending key never expires on its own: the watch may have committed it already (the
    // user entered the code) while its confirmation was lost on the air; the next request it
    // signs with that key completes the pairing, however late.
    let mut pending: Option<Key> = beacon::load_pending_key();
    if pending.is_some() {
        log(&format!("pairing: pending key restored from {}", beacon::pending_key_path().display()));
    }
    let mut pair_state = String::from(if pending.is_some() { "pairing pending" } else if key.is_some() { "paired" } else { "no key" });
    // The theme list push (protocol/BEACON.md §3): the watch's address (saved at pairing,
    // refreshed from every accepted request), the one-at-a-time gate, the last outcome.
    let mut watch_addr: Option<String> = beacon::load_watch_addr();
    let list_gate = Arc::new(Semaphore::new(1));
    let list_state = Arc::new(Mutex::new(String::from("never")));
    let (pushed_tx, mut pushed_rx) = mpsc::channel::<&'static str>(4);
    let list_job = |key: Key, addr: Option<String>, force: bool, reason: &'static str| ListPushJob { opts: opts.ble.clone(), key, addr, force, reason, after: pushed_tx.clone() };

    publish(&mut radio, &theme_wire, true).await;
    if !theme_wire.is_empty() {
        burst_until = Some(Instant::now() + BURST_FOR);
        log(&format!("beacon: theme {theme_name} ({} bytes)", theme_wire.len()));
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
                    publish(&mut radio, &theme_wire, false).await;
                }
            }
            Some(ipc) = rx.recv() => {
                if let Request::PushList { force } = ipc.req {
                    match key {
                        Some(k) => list_push(list_job(k, watch_addr.clone(), force, "push-list"), list_gate.clone(), list_state.clone(), Some(ipc.reply)),
                        None => { let _ = ipc.reply.send(Reply::err("no pairing key: run `themesync pair` first (the list's COMMIT is keyed)")); }
                    }
                    continue;
                }
                let reply = match ipc.req {
                    Request::Ping => Reply { connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("pong") },
                    Request::Status => Reply {
                        theme: Some(theme_name.clone()),
                        connected: Some(true),
                        watch: Some(format!(
                            "beacon {}, key {}, counter last {ctr_last} rejected {ctr_rejected}, watch {}, last request {}, list push {}",
                            if theme_wire.is_empty() { "idle" } else { "on" },
                            if key.is_some() { "loaded" } else { "missing" },
                            watch_addr.as_deref().unwrap_or("unknown"),
                            last_request.as_deref().unwrap_or("none"),
                            list_state.lock().unwrap()
                        )),
                        ..Reply::ok(pair_state.clone())
                    },
                    Request::PushList { .. } => unreachable!("handled above"),
                    Request::ResetCounter => {
                        ctr_last = 0;
                        last_rejected = None;
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        publish(&mut radio, &theme_wire, burst_until.is_some()).await;
                        match beacon::save_ctr(0) {
                            Ok(()) => { log("counter reset to 0 by request"); Reply::ok("counter reset; the watch's next request (any counter > 0) will be accepted") }
                            Err(e) => Reply::err(format!("could not write {}: {e}", beacon::ctr_path().display())),
                        }
                    }
                    Request::PairPending { key_hex } => match protocol::from_hex(&key_hex).and_then(|v| Key::try_from(v).ok()) {
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
                        Ok((name, v2)) => {
                            theme_name = name.clone();
                            theme_v2 = v2.clone();
                            theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                            publish(&mut radio, &theme_wire, true).await;
                            burst_until = Some(Instant::now() + BURST_FOR);
                            log(&format!("beacon: theme {name} ({} bytes{})", theme_wire.len(), if key.is_none() { ", not sent: no key" } else { "" }));
                            if opts.gatt { gatt_push(opts.ble.clone(), v2); }
                            Reply { theme: Some(name), connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("sent") }
                        }
                        Err(e) => Reply::err(format!("{e:#}")),
                    },
                    Request::Push { packet_hex } => match protocol::from_hex(&packet_hex) {
                        Some(v2) => {
                            theme_name = protocol::decode_v2(&v2).ok().and_then(|d| d.name).unwrap_or_default();
                            theme_v2 = v2.clone();
                            theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                            publish(&mut radio, &theme_wire, true).await;
                            burst_until = Some(Instant::now() + BURST_FOR);
                            log(&format!("beacon: pushed packet ({} bytes)", theme_wire.len()));
                            if opts.gatt { gatt_push(opts.ble.clone(), v2); }
                            Reply { connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("sent") }
                        }
                        None => Reply::err("packet_hex is not hex"),
                    },
                };
                let _ = ipc.reply.send(reply);
            }
            Some((addr, data)) = rrx.recv() => {
                if data.get(1) != Some(&beacon::KIND_REQUEST) { continue; }
                if key.is_none() && pending.is_none() { continue; }
                match beacon::decode_request_with(key.as_ref(), pending.as_ref(), &data) {
                    Err(e) => {
                        // once per distinct packet: BlueZ re-delivers the same bytes on every RSSI tick
                        if last_bad.as_deref() != Some(&data[..]) {
                            log(&format!("{addr}: request ignored: {e}"));
                            last_bad = Some(data.clone());
                        }
                    }
                    Ok((req, Verified::Pending)) => {
                        let k = pending.take().unwrap();
                        beacon::clear_pending_key();
                        match beacon::save_key(&k) {
                            Ok(p) => log(&format!("pairing: confirmed by {addr}; key saved to {}", p.display())),
                            Err(e) => log(&format!("pairing: confirmed by {addr} but saving the key failed: {e}")),
                        }
                        key = Some(k);
                        ctr_last = req.ctr;
                        last_rejected = None;
                        if let Err(e) = beacon::save_ctr(ctr_last) { log(&format!("could not write {}: {e}", beacon::ctr_path().display())); }
                        watch_addr = Some(addr.to_string());
                        if let Err(e) = beacon::save_watch_addr(&addr.to_string()) { log(&format!("could not write {}: {e}", beacon::watch_path().display())); }
                        pair_state = format!("paired with {addr}");
                        last_request = Some(format!("{:?} #{} from {addr} [pairing confirmation]", req.op, req.ctr));
                        // The beacon is signed with the new key from now on.
                        if let Ok((name, v2)) = current_theme() {
                            theme_name = name; theme_v2 = v2;
                        }
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        publish(&mut radio, &theme_wire, true).await;
                        burst_until = Some(Instant::now() + BURST_FOR);
                        // "The list arrives with pairing": push it now, over a connection to that watch.
                        list_push(list_job(k, watch_addr.clone(), false, "pairing"), list_gate.clone(), list_state.clone(), None);
                    }
                    Ok((req, Verified::Active)) => {
                        if req.ctr <= ctr_last {
                            if last_rejected != Some(req.ctr) {
                                ctr_rejected += 1;
                                last_rejected = Some(req.ctr);
                                if req.ctr < ctr_last.saturating_sub(100) {
                                    log(&format!("{addr}: {:?} #{} rejected: last accepted is #{ctr_last} — a reflashed watch counts from 1 again; run `themesync pair` (resets both sides) or `themesync reset-counter`", req.op, req.ctr));
                                } else {
                                    log(&format!("{addr}: {:?} #{} rejected: already seen (last accepted #{ctr_last})", req.op, req.ctr));
                                }
                            }
                            continue;
                        }
                        ctr_last = req.ctr;
                        if let Err(e) = beacon::save_ctr(ctr_last) { log(&format!("could not write {}: {e}", beacon::ctr_path().display())); }
                        // Echo first (BEACON.md §2): the watch learns its request arrived
                        // within one scan window and stops retransmitting, before
                        // omarchy-theme-set even starts.
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        publish(&mut radio, &theme_wire, true).await;
                        burst_until = Some(Instant::now() + BURST_FOR);
                        let a = addr.to_string();
                        if watch_addr.as_deref() != Some(&a) {
                            watch_addr = Some(a.clone());
                            if let Err(e) = beacon::save_watch_addr(&a) { log(&format!("could not write {}: {e}", beacon::watch_path().display())); }
                        }
                        last_request = Some(format!("{:?} #{} from {a}", req.op, req.ctr));
                        log(&format!("{a}: {:?} #{}", req.op, req.ctr));
                        let k = key.expect("Verified::Active implies an active key");
                        if req.op == Op::Resend {
                            continue; // the echo burst above is the answer
                        }
                        if req.op == Op::List {
                            // The watch asked for a refresh: always send (its request stays on the
                            // air until our COMMIT lands).
                            list_push(list_job(k, watch_addr.clone(), true, "request"), list_gate.clone(), list_state.clone(), None);
                            continue;
                        }
                        // SET: the slug crc against the installed themes. No match = the
                        // watch's list is stale (a theme removed since the push): answer
                        // with the current list, exactly like LIST (BEACON.md §2).
                        let (Some(om), Some(tx)) = (&om, &actor) else {
                            log("no Omarchy install found: request dropped");
                            continue;
                        };
                        match om.list_themes().into_iter().find(|s| beacon::slug_id(s) == req.arg) {
                            Some(slug) => { let _ = tx.send((req.ctr, slug)); }
                            None => {
                                log(&format!("{a}: SET {:#06x} matches no installed theme; pushing the list", req.arg));
                                list_push(list_job(k, watch_addr.clone(), true, "unknown theme"), list_gate.clone(), list_state.clone(), None);
                            }
                        }
                    }
                }
            }
            Some(reason) = pushed_rx.recv() => {
                // The watch just took a new list and forgot its applied theme: burst so its
                // next scan window repaints it from the current beacon.
                log(&format!("beacon: burst after the list push ({reason})"));
                publish(&mut radio, &theme_wire, true).await;
                burst_until = Some(Instant::now() + BURST_FOR);
            }
            _ = relax => {
                burst_until = None;
                publish(&mut radio, &theme_wire, false).await;
            }
        }
    }
}
