//! `themesync daemon`: the connection-less bridge of `protocol/BEACON.md` (v3).
//!
//! * Advertises the current Omarchy theme as the state beacon, signed with the pairing key,
//!   forever at a fixed 30 ms (BEACON.md §1: the watch scans a 45 ms window).
//! * Scans for the watch's requests (SET/RESEND/LIST), checks their MAC against the pairing
//!   key and their counter against the last one accepted, and drives `omarchy-theme-set`.
//!   The beacon is refreshed from the applied theme directly, and again by the hook, which
//!   comes back through the socket. One loop for both directions.
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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::beacon::{self, Op, Verified};
use crate::omarchy::Omarchy;
use crate::palette::map_source;
use crate::protocol;
use crate::themelist;
use crate::transport::adv::{OpenError, Radio};
use crate::transport::ble::{self, BleOptions};
use crate::transport::ipc::{socket_path, Reply, Request, StatusInfo};

/// What `--version` and `status` report as the wire protocol.
pub const PROTOCOL: &str = "beacon v3, list status v2";

/// Fixed (min == max). With the controller's 0–10 ms advDelay the worst gap between two
/// events is 40 ms, inside the watch's 45 ms scan window (BEACON.md §1). Constant, no
/// "burst" after a change: the watch's window is sized for this rate at all times.
const INTERVAL: Duration = Duration::from_millis(30);
/// BlueZ silently drops advertisements when the adapter resets (suspend/resume, `bluetoothctl
/// power off`); re-registering on a timer is the cheap insurance for a daemon that runs forever.
const REPUBLISH_EVERY: Duration = Duration::from_secs(60);
/// A GATT session (find + connect + a few writes) that takes longer than this is stuck in
/// BlueZ; give up so the next trigger can run.
const GATT_TIMEOUT: Duration = Duration::from_secs(60);
/// One IPC request is one line; a client that sends more, or nothing, is dropped.
const IPC_LINE_MAX: u64 = 4096;
const IPC_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How the daemon waits for the radio: 3 s, doubling, up to a minute.
const RETRY_MIN: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);

type Key = [u8; beacon::KEY_LEN];

fn log(msg: &str) {
    eprintln!("[themesync] {msg}");
}

/// The active Omarchy theme: its slug and the v2 packet (what GATT `…0002` takes and what
/// the beacon carries). Runs `omarchy-theme-color`, so it is called off the async loop.
fn current_theme_blocking() -> Result<(String, Vec<u8>)> {
    let om = Omarchy::from_env()?;
    let src = om.load_current()?;
    let p = map_source(&src)?;
    Ok((p.name.clone(), protocol::encode_v2(&p, false)))
}

async fn current_theme() -> Result<(String, Vec<u8>)> {
    tokio::task::spawn_blocking(current_theme_blocking).await.context("theme resolver task")?
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

/// Is a daemon already answering on `path`? (A stale socket file from a crash does not.)
async fn daemon_answers(path: &std::path::Path) -> bool {
    let Ok(mut s) = tokio::net::UnixStream::connect(path).await else { return false };
    if s.write_all(b"{\"cmd\":\"ping\"}\n").await.is_err() {
        return false;
    }
    let mut line = String::new();
    matches!(tokio::time::timeout(Duration::from_secs(2), BufReader::new(&mut s).read_line(&mut line)).await, Ok(Ok(n)) if n > 0)
}

async fn serve_socket(tx: mpsc::Sender<Ipc>) -> Result<()> {
    let path = socket_path()?;
    if daemon_answers(&path).await {
        anyhow::bail!("another themesync daemon is already listening on {} (systemctl --user status themesync)", path.display());
    }
    let _ = std::fs::remove_file(&path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    log(&format!("listening on {}", path.display()));
    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                // ECONNABORTED, EMFILE and friends are transient; the listener is still fine.
                log(&format!("socket accept: {e}; continuing"));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            let (rd, mut wr) = stream.into_split();
            let mut line = String::new();
            let read = tokio::time::timeout(IPC_READ_TIMEOUT, BufReader::new(rd.take(IPC_LINE_MAX)).read_line(&mut line)).await;
            if !matches!(read, Ok(Ok(n)) if n > 0) || line.trim().is_empty() {
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
/// moved on). Every applied slug is reported on `applied`, so the beacon follows even when
/// the Omarchy hook is not installed.
fn spawn_actor(om: Omarchy, applied: mpsc::Sender<String>) -> std::sync::mpsc::Sender<SetJob> {
    let (tx, rx) = std::sync::mpsc::channel::<SetJob>();
    std::thread::spawn(move || {
        while let Ok(mut job) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                log(&format!("SET #{} ({}) superseded by #{}", job.0, job.1, newer.0));
                job = newer;
            }
            let (ctr, name) = job;
            log(&format!("SET #{ctr} -> omarchy-theme-set {name}"));
            match om.set_theme(&name) {
                Ok(()) => { let _ = applied.blocking_send(name); }
                Err(e) => log(&format!("SET {name} failed: {e:#}")),
            }
        }
    });
    tx
}

/// One-shot GATT push of the current theme (the pre-beacon path), detached and bounded.
fn gatt_push(opts: BleOptions, v2: Vec<u8>) {
    tokio::spawn(async move {
        let work = async {
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
            Ok::<(), anyhow::Error>(())
        };
        match tokio::time::timeout(GATT_TIMEOUT, work).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log(&format!("gatt push failed: {e:#}")),
            Err(_) => log(&format!("gatt push abandoned after {GATT_TIMEOUT:?}")),
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
    /// Told when the push succeeded (logged; the watch forgets its applied theme on a list
    /// COMMIT and repaints from the first beacon it sees — the beacon is already on the air).
    after: mpsc::Sender<&'static str>,
}

/// Build the theme list and push it over GATT (protocol/BEACON.md §3), detached. One push at
/// a time (`gate`): a trigger while one is in flight is dropped, not queued — the transfer is
/// idempotent and the next trigger sends the current list anyway. Every attempt is bounded by
/// [`GATT_TIMEOUT`], so a BlueZ call that never returns cannot hold the gate forever. `state`
/// is what `status` shows; `done` gets the outcome when `push-list` is waiting on the socket.
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
                let step = async { tokio::time::timeout(GATT_TIMEOUT, step).await.unwrap_or_else(|_| Err(anyhow::anyhow!("no answer from BlueZ within {GATT_TIMEOUT:?}"))) };
                match step.await {
                    Ok(outcome) => return Ok(format!("{summary}: {outcome}")),
                    Err(e) => {
                        let text = format!("{e:#}");
                        log(&format!("list push attempt {attempt}: {text}"));
                        // a rejected frame will not pass on retry; neither will a status without a nonce
                        let watch_said_no = text.contains("ATT error") || text.contains("does not decode");
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

/// Open the radio, waiting (with backoff) for the conditions that clear on their own:
/// bluetoothd not up, no adapter yet, adapter powered off. A controller that cannot do the
/// job at all is an error. Logs once per distinct reason, not once per attempt.
async fn open_radio_waiting(what: &str) -> Result<Radio> {
    let mut delay = RETRY_MIN;
    let mut last_msg = String::new();
    loop {
        match Radio::open().await {
            Ok(r) => {
                if !last_msg.is_empty() {
                    log(&format!("{what}: adapter {} available", r.adapter_name()));
                }
                return Ok(r);
            }
            Err(OpenError::Fatal(e)) => return Err(e),
            Err(OpenError::Retry(e)) => {
                let msg = format!("{e:#}");
                if msg != last_msg {
                    log(&format!("{what}: {msg}; waiting (retrying every {}–{} s)", RETRY_MIN.as_secs(), RETRY_MAX.as_secs()));
                    last_msg = msg;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Register the beacon and report whether it is on the air. On failure (BlueZ mid-reset,
/// "maximum advertisements reached" from the drop/register race) try once more after 200
/// ms; if that fails too, reopen the adapter (it may have been reset under us) and try a
/// last time. The previous beacon stays registered until a new one succeeds (`set_beacon`).
async fn publish(radio: &mut Radio, wire: &[u8]) -> bool {
    if wire.is_empty() {
        return false;
    }
    if radio.set_beacon(wire.to_vec(), INTERVAL).await.is_ok() {
        return true;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = radio.set_beacon(wire.to_vec(), INTERVAL).await;
    let Err(e) = second else { return true };
    log(&format!("beacon: {e:#}; reopening the adapter"));
    match Radio::open().await {
        Ok(fresh) => {
            *radio = fresh;
            match radio.set_beacon(wire.to_vec(), INTERVAL).await {
                Ok(()) => { log("beacon: back on the air"); true }
                Err(e) => { log(&format!("beacon: OFF THE AIR: {e:#}; next attempt in {} s", REPUBLISH_EVERY.as_secs())); false }
            }
        }
        Err(e) => { log(&format!("beacon: OFF THE AIR: {e}; next attempt in {} s", REPUBLISH_EVERY.as_secs())); false }
    }
}

/// Unauthenticated or malformed packets from the air get one log line per minute plus a
/// count, not one per packet: anyone can advertise under company 0xFFFF.
struct BadPacketLog {
    window: Instant,
    shown: bool,
    suppressed: u32,
}

impl BadPacketLog {
    fn new() -> Self {
        BadPacketLog { window: Instant::now(), shown: false, suppressed: 0 }
    }
    fn note(&mut self, line: impl FnOnce() -> String) {
        if self.window.elapsed() >= Duration::from_secs(60) {
            if self.suppressed > 0 {
                log(&format!("{} more packets ignored in the last minute", self.suppressed));
            }
            self.window = Instant::now();
            self.shown = false;
            self.suppressed = 0;
        }
        if !self.shown {
            log(&line());
            self.shown = true;
        } else {
            self.suppressed += 1;
        }
    }
}

pub async fn run(opts: Options) -> Result<()> {
    let mut radio = open_radio_waiting("beacon").await?;
    log(&format!("adapter {}", radio.adapter_name()));
    let mut key: Option<Key> = beacon::load_key();
    match &key {
        Some(_) => log(&format!("pairing key loaded from {}", beacon::key_path().display())),
        None => log("no pairing key: no beacon and no request scan until `themesync pair`"),
    }
    let om = Omarchy::from_env().ok();
    if let Some(om) = &om {
        let hook = om.hooks_dir().join("themesync");
        if !hook.is_file() {
            log(&format!("WARNING: no Omarchy hook at {} — desktop-side theme changes will not reach the watch; run `themesync install-hook`", hook.display()));
        }
    } else {
        log("WARNING: no Omarchy install found (HOME/OMARCHY_PATH): watch requests will be dropped");
    }
    let (applied_tx, mut applied_rx) = mpsc::channel::<String>(4);
    let actor = om.clone().map(|o| spawn_actor(o, applied_tx));

    let (tx, mut rx) = mpsc::channel::<Ipc>(16);
    let mut socket_task = tokio::spawn(serve_socket(tx));

    // Requests from the air, via a second bluer session (the scan borrows the adapter for
    // as long as it runs). Only while there is a key to verify them with: without one the
    // scan would keep the adapter in active discovery for nothing.
    let (rtx, mut rrx) = mpsc::channel::<(Address, Vec<u8>)>(32);
    let (scan_on_tx, scan_on_rx) = watch::channel(false);
    let monitor_ok: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
    {
        let monitor_ok = monitor_ok.clone();
        let mut scan_on = scan_on_rx;
        tokio::spawn(async move {
            let mut delay = RETRY_MIN;
            loop {
                if !*scan_on.borrow() {
                    if scan_on.changed().await.is_err() { return; }
                    continue;
                }
                let r = match Radio::open().await {
                    Ok(r) => { delay = RETRY_MIN; r }
                    Err(OpenError::Fatal(e)) => { log(&format!("scan: {e:#}")); return; }
                    Err(OpenError::Retry(e)) => {
                        log(&format!("scan: {e:#}; retrying in {} s", delay.as_secs()));
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(RETRY_MAX);
                        continue;
                    }
                };
                let rtx = rtx.clone();
                tokio::select! {
                    res = r.scan_ours(&monitor_ok, |addr, data| { let _ = rtx.try_send((addr, data.to_vec())); }) => {
                        if let Err(e) = res { log(&format!("scan stopped: {e:#}; restarting in {} s", delay.as_secs())); }
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(RETRY_MAX);
                    }
                    _ = scan_on.changed() => {
                        if !*scan_on.borrow() { log("scan: stopped (no key)"); }
                    }
                }
                *monitor_ok.lock().unwrap() = None;
            }
        });
    }

    // ---- state ----
    let (mut theme_name, mut theme_v2) = match current_theme().await {
        Ok(t) => t,
        Err(e) => {
            log(&format!("no active Omarchy theme yet ({e:#}); retrying every {} s", REPUBLISH_EVERY.as_secs()));
            (String::new(), Vec::new())
        }
    };
    let mut last_request: Option<String> = None;
    // The request counter (BEACON.md §2): accept only ctr > last accepted under the active
    // key. This one rule covers repeats of the same press, BlueZ's cached copy of an old
    // request re-delivered on any property change, a request still on the air across a
    // daemon restart, and replays. A counter file that cannot be read locks the daemon
    // (nothing accepted) rather than reopening those replays.
    let mut ctr_locked = false;
    let mut ctr_last: u16 = match beacon::load_ctr() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("WARNING: {e}: no request will be accepted until `themesync reset-counter` or `themesync pair`"));
            ctr_locked = true;
            u16::MAX
        }
    };
    let mut theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
    let mut ctr_rejected: u32 = 0;
    let mut last_rejected: Option<u16> = None;
    let mut bad_log = BadPacketLog::new();
    // A pending key is honoured for PENDING_KEY_TTL from the moment it was handed over
    // (BEACON.md §2b); the watch's own window is the same, so a later confirmation cannot
    // come. Persisted so a daemon restart inside that window does not lose it. The address
    // `pair` connected to comes with it and becomes the list-push address on confirmation.
    let mut pending: Option<(Key, Instant)> = beacon::load_pending_key().map(|(k, age)| (k, Instant::now() - age));
    let mut pending_addr: Option<String> = None;
    if pending.is_some() {
        log(&format!("pairing: pending key restored from {}", beacon::pending_key_path().display()));
    }
    let mut pair_state = String::from(if pending.is_some() { "pairing pending" } else if key.is_some() { "paired" } else { "no key" });
    // The theme list push (protocol/BEACON.md §3): the watch's GATT address (learned at
    // pairing only — requests arrive from rotating random addresses, BEACON.md §2), the
    // one-at-a-time gate, the last outcome.
    let mut watch_addr: Option<String> = beacon::load_watch_addr();
    let list_gate = Arc::new(Semaphore::new(1));
    let list_state = Arc::new(Mutex::new(String::from("never")));
    let (pushed_tx, mut pushed_rx) = mpsc::channel::<&'static str>(4);
    let list_job = |key: Key, addr: Option<String>, force: bool, reason: &'static str| ListPushJob { opts: opts.ble.clone(), key, addr, force, reason, after: pushed_tx.clone() };

    let _ = scan_on_tx.send(key.is_some() || pending.is_some());
    let mut beacon_up = publish(&mut radio, &theme_wire).await;
    if beacon_up {
        log(&format!("beacon: theme {theme_name} ({} bytes)", theme_wire.len()));
    }

    let mut republish = tokio::time::interval(REPUBLISH_EVERY);
    republish.tick().await; // the first tick fires immediately; the beacon is already up
    loop {
        let pending_expiry = async {
            match pending {
                Some((_, since)) => tokio::time::sleep_until(tokio::time::Instant::from_std(since + beacon::PENDING_KEY_TTL)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            r = &mut socket_task => {
                // The socket is how the hook, `sync`, `pair` and `status` reach the daemon; a
                // daemon without it looks alive and does nothing useful. Exit; systemd restarts.
                let why = match r { Ok(Ok(())) => "socket server returned".to_string(), Ok(Err(e)) => format!("{e:#}"), Err(e) => format!("socket task panicked: {e}") };
                anyhow::bail!("{why}");
            }
            _ = republish.tick() => {
                if theme_v2.is_empty() {
                    if let Ok((name, v2)) = current_theme().await {
                        theme_name = name;
                        theme_v2 = v2;
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        log(&format!("beacon: theme {theme_name} ({} bytes)", theme_wire.len()));
                    }
                }
                beacon_up = publish(&mut radio, &theme_wire).await;
            }
            _ = pending_expiry => {
                pending = None;
                beacon::clear_pending_key();
                pair_state = if key.is_some() { "paired".into() } else { "no key".into() };
                log(&format!("pairing: the pending key expired after {} s without a confirmation from the watch; run `themesync pair` again", beacon::PENDING_KEY_TTL.as_secs()));
                let _ = scan_on_tx.send(key.is_some());
            }
            Some(name) = applied_rx.recv() => {
                // The watch's SET went through omarchy-theme-set: refresh the beacon from the
                // applied theme now, whether or not the hook is installed (it would do the
                // same through the socket a moment later).
                match current_theme().await {
                    Ok((n, v2)) => {
                        theme_name = n;
                        theme_v2 = v2;
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        beacon_up = publish(&mut radio, &theme_wire).await;
                        log(&format!("beacon: theme {theme_name} ({} bytes) after SET {name}", theme_wire.len()));
                    }
                    Err(e) => log(&format!("after SET {name}: cannot read the applied theme: {e:#}")),
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
                    Request::Status => {
                        let monitor = if *scan_on_tx.borrow() { *monitor_ok.lock().unwrap() } else { None };
                        let scan = match (*scan_on_tx.borrow(), monitor) {
                            (false, _) => "off",
                            (true, None) => "starting",
                            (true, Some(_)) => "on",
                        };
                        let beacon_state = if beacon_up { "on" } else if theme_wire.is_empty() { "idle" } else { "off_air" };
                        let info = StatusInfo {
                            protocol: PROTOCOL.into(),
                            pairing: pair_state.clone(),
                            paired: key.is_some(),
                            beacon: beacon_state.into(),
                            scan: scan.into(),
                            monitor,
                            theme: theme_name.clone(),
                            ctr_last,
                            ctr_locked,
                            stale_rejected: ctr_rejected,
                            watch: watch_addr.clone(),
                            last_request: last_request.clone(),
                            list_push: list_state.lock().unwrap().clone(),
                            hook_installed: om.as_ref().map(|o| o.hooks_dir().join("themesync").is_file()).unwrap_or(false),
                        };
                        Reply {
                            theme: Some(theme_name.clone()),
                            connected: Some(beacon_up),
                            watch: Some(format!(
                                "beacon {}, key {}, scan {}, counter last {ctr_last}{} stale-rejected {ctr_rejected}, watch {}, last request {}, list push {}",
                                if beacon_up { "on" } else if theme_wire.is_empty() { "idle" } else { "OFF THE AIR" },
                                if key.is_some() { "loaded" } else { "missing" },
                                match monitor { Some(true) => "on (+ advertisement monitor)", _ => scan },
                                if ctr_locked { " (LOCKED: counter file unreadable)" } else { "" },
                                watch_addr.as_deref().unwrap_or("unknown"),
                                last_request.as_deref().unwrap_or("none"),
                                list_state.lock().unwrap()
                            )),
                            info: Some(info),
                            ..Reply::ok(pair_state.clone())
                        }
                    }
                    Request::PushList { .. } => unreachable!("handled above"),
                    Request::ResetCounter => {
                        ctr_last = 0;
                        ctr_locked = false;
                        last_rejected = None;
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        beacon_up = publish(&mut radio, &theme_wire).await;
                        match beacon::save_ctr(0) {
                            Ok(()) => { log("counter reset to 0 by request"); Reply::ok("counter reset; the watch's next request (any counter > 0) will be accepted") }
                            Err(e) => Reply::err(format!("could not write {}: {e}", beacon::ctr_path().display())),
                        }
                    }
                    Request::PairPending { key_hex, addr } => match protocol::from_hex(&key_hex).and_then(|v| Key::try_from(v).ok()) {
                        Some(k) => {
                            pending = Some((k, Instant::now()));
                            pending_addr = addr;
                            if let Err(e) = beacon::save_pending_key(&k) {
                                log(&format!("pairing: could not persist the pending key: {e}"));
                            }
                            pair_state = "pairing pending".into();
                            let _ = scan_on_tx.send(true);
                            log(&format!("pairing: pending key held for {} s; waiting for a request signed with it", beacon::PENDING_KEY_TTL.as_secs()));
                            Reply::ok("pending")
                        }
                        None => Reply::err("key_hex must be 32 hex digits"),
                    },
                    Request::Sync => match current_theme().await {
                        Ok((name, v2)) => {
                            theme_name = name.clone();
                            theme_v2 = v2.clone();
                            theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                            if key.is_none() {
                                log(&format!("theme {name}: not sent, no pairing key"));
                                Reply { theme: Some(name), ..Reply::err("no pairing key: run `themesync pair`") }
                            } else {
                                beacon_up = publish(&mut radio, &theme_wire).await;
                                log(&format!("beacon: theme {name} ({} bytes){}", theme_wire.len(), if beacon_up { "" } else { " — NOT on the air" }));
                                if opts.gatt { gatt_push(opts.ble.clone(), v2); }
                                if beacon_up {
                                    Reply { theme: Some(name), connected: Some(true), watch: Some("beacon".into()), ..Reply::ok("sent") }
                                } else {
                                    Reply { theme: Some(name), connected: Some(false), ..Reply::err("the beacon is off the air (see the daemon log)") }
                                }
                            }
                        }
                        Err(e) => Reply::err(format!("{e:#}")),
                    },
                    Request::Push { packet_hex } => match protocol::from_hex(&packet_hex) {
                        Some(v2) => match beacon::check_theme(&v2) {
                            Err(e) => Reply::err(e),
                            Ok(()) if key.is_none() => Reply::err("no pairing key: run `themesync pair`"),
                            Ok(()) => {
                                theme_name = protocol::decode_v2(&v2).ok().and_then(|d| d.name).unwrap_or_default();
                                theme_v2 = v2.clone();
                                theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                                beacon_up = publish(&mut radio, &theme_wire).await;
                                log(&format!("beacon: pushed packet ({} bytes)", theme_wire.len()));
                                if opts.gatt { gatt_push(opts.ble.clone(), v2); }
                                Reply { connected: Some(beacon_up), watch: Some("beacon".into()), ..Reply::ok("sent") }
                            }
                        },
                        None => Reply::err("packet_hex is not hex"),
                    },
                };
                let _ = ipc.reply.send(reply);
            }
            Some((addr, data)) = rrx.recv() => {
                if data.get(1) != Some(&beacon::KIND_REQUEST) { continue; }
                if key.is_none() && pending.is_none() { continue; }
                match beacon::decode_request_with(key.as_ref(), pending.as_ref().map(|(k, _)| k), &data) {
                    Err(e) => bad_log.note(|| format!("{addr}: request ignored: {e}")),
                    Ok((req, Verified::Pending)) => {
                        let (k, _) = pending.take().unwrap();
                        beacon::clear_pending_key();
                        match beacon::save_key(&k) {
                            Ok(p) => log(&format!("pairing: confirmed by {addr}; key saved to {}", p.display())),
                            Err(e) => log(&format!("pairing: confirmed by {addr} but saving the key failed: {e}")),
                        }
                        key = Some(k);
                        ctr_last = req.ctr;
                        ctr_locked = false;
                        last_rejected = None;
                        if let Err(e) = beacon::save_ctr(ctr_last) { log(&format!("could not write {}: {e}", beacon::ctr_path().display())); }
                        // The GATT address is the one `pair` connected to; the request's
                        // source address is a random one the watch will never use again.
                        if let Some(a) = pending_addr.take() {
                            watch_addr = Some(a.clone());
                            if let Err(e) = beacon::save_watch_addr(&a) { log(&format!("could not write {}: {e}", beacon::watch_path().display())); }
                        }
                        pair_state = format!("paired with {}", watch_addr.as_deref().unwrap_or("the watch (address unknown: pair again with the daemon running)"));
                        last_request = Some(format!("{:?} #{} [pairing confirmation]", req.op, req.ctr));
                        // The beacon is signed with the new key from now on.
                        if let Ok((name, v2)) = current_theme().await {
                            theme_name = name; theme_v2 = v2;
                        }
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        beacon_up = publish(&mut radio, &theme_wire).await;
                        // "The list arrives with pairing": push it now, over a connection to that watch.
                        list_push(list_job(k, watch_addr.clone(), false, "pairing"), list_gate.clone(), list_state.clone(), None);
                    }
                    Ok((req, Verified::Active)) => {
                        match beacon::judge_ctr(req.ctr, ctr_last, ctr_locked) {
                            beacon::CtrVerdict::Accept => {}
                            // The watch's retransmission of the request just accepted (its
                            // stop-and-wait, BEACON.md §2), re-delivered by BlueZ on every
                            // advertising event: expected, silent.
                            beacon::CtrVerdict::Duplicate => continue,
                            verdict => {
                                if last_rejected != Some(req.ctr) {
                                    ctr_rejected += 1;
                                    last_rejected = Some(req.ctr);
                                    let why = match verdict {
                                        beacon::CtrVerdict::Locked => "the counter file is unreadable; run `themesync reset-counter` or `themesync pair`".to_string(),
                                        beacon::CtrVerdict::Exhausted => "the counter is exhausted (65535); run `themesync pair` to start a new one".to_string(),
                                        _ if req.ctr < ctr_last.saturating_sub(100) => format!("last accepted is #{ctr_last} — a reflashed watch counts from 1 again; run `themesync pair` (resets both sides) or `themesync reset-counter`"),
                                        _ => format!("stale (last accepted #{ctr_last})"),
                                    };
                                    log(&format!("{addr}: {:?} #{} rejected: {why}", req.op, req.ctr));
                                }
                                continue;
                            }
                        }
                        ctr_last = req.ctr;
                        if let Err(e) = beacon::save_ctr(ctr_last) { log(&format!("could not write {}: {e}", beacon::ctr_path().display())); }
                        // Echo first (BEACON.md §2): the watch learns its request arrived
                        // within one scan window and stops retransmitting, before
                        // omarchy-theme-set even starts.
                        theme_wire = sign(key.as_ref(), &theme_v2, ctr_last);
                        beacon_up = publish(&mut radio, &theme_wire).await;
                        // `addr` is a throwaway random address (BEACON.md §2): logged, never saved.
                        last_request = Some(format!("{:?} #{}", req.op, req.ctr));
                        log(&format!("{:?} #{} (from {addr})", req.op, req.ctr));
                        let k = key.expect("Verified::Active implies an active key");
                        if req.op == Op::Resend {
                            continue; // the echo above is the answer
                        }
                        if req.op == Op::List {
                            // The watch asked for a refresh: always send (its request stays on the
                            // air until our COMMIT lands).
                            list_push(list_job(k, watch_addr.clone(), true, "request"), list_gate.clone(), list_state.clone(), None);
                            continue;
                        }
                        // SET: the slug crc against the installed themes. No match = the
                        // watch's list is stale (a theme removed since the push): answer
                        // with the current list, exactly like LIST (BEACON.md §2). Only
                        // slugs the list could carry (themelist::build skips the rest).
                        let (Some(om), Some(tx)) = (&om, &actor) else {
                            log("no Omarchy install found: request dropped");
                            continue;
                        };
                        match om.list_themes().into_iter().filter(|s| s.len() <= protocol::V2_MAX_NAME).find(|s| beacon::slug_id(s) == req.arg) {
                            Some(slug) => { let _ = tx.send((req.ctr, slug)); }
                            None => {
                                log(&format!("SET {:#06x} matches no installed theme; pushing the list", req.arg));
                                list_push(list_job(k, watch_addr.clone(), true, "unknown theme"), list_gate.clone(), list_state.clone(), None);
                            }
                        }
                    }
                }
            }
            Some(reason) = pushed_rx.recv() => {
                // The watch just took a new list and forgot its applied theme; it repaints
                // from the next beacon it hears (≤ 1 s), nothing to do here.
                log(&format!("list push done ({reason}); the watch repaints from the beacon"));
            }
        }
    }
}
