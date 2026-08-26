//! Direct Linux/macOS -> watch BLE GATT link (btleplug: BlueZ on Linux, CoreBluetooth on macOS).
//!
//! GATT layout (see `protocol/THEME_PROTOCOL.md`):
//!
//! | characteristic | uuid             | props                | payload            |
//! |----------------|------------------|----------------------|--------------------|
//! | Theme State    | `7e450002-…`     | write, read          | ThemeState packet  |
//! | Status         | `7e450003-…`     | read, notify         | 6-byte Status      |
//! | Control        | `7e450004-…`     | notify               | 4-byte Control     |
//! | Info           | `7e450005-…`     | read                 | 4-byte Info        |
//!
//! The write is "with response" so the stack does an ATT long write automatically when the
//! packet does not fit in `MTU - 3`; no application-level fragmentation exists.

use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use btleplug::api::{Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::Stream;
use uuid::{uuid, Uuid};

use crate::protocol::{self, Info, Status, StatusCode};

pub const SERVICE_UUID: Uuid = uuid!("7e450001-5029-4337-8dde-aaefb009b2df");
pub const CHR_THEME_STATE: Uuid = uuid!("7e450002-5029-4337-8dde-aaefb009b2df");
pub const CHR_STATUS: Uuid = uuid!("7e450003-5029-4337-8dde-aaefb009b2df");
pub const CHR_CONTROL: Uuid = uuid!("7e450004-5029-4337-8dde-aaefb009b2df");
pub const CHR_INFO: Uuid = uuid!("7e450005-5029-4337-8dde-aaefb009b2df");

#[derive(Debug, Clone)]
pub struct BleOptions {
    /// Only accept a watch whose advertised name matches (otherwise: any device advertising
    /// the Theme service).
    pub name: Option<String>,
    pub scan_timeout: Duration,
}

impl Default for BleOptions {
    fn default() -> Self {
        BleOptions { name: std::env::var("THEMESYNC_NAME").ok().filter(|s| !s.is_empty()), scan_timeout: Duration::from_secs(8) }
    }
}

pub async fn adapter() -> Result<Adapter> {
    let manager = Manager::new().await.context("initialising the Bluetooth stack")?;
    manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no Bluetooth adapter found"))
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub name: Option<String>,
    pub id: String,
    pub rssi: Option<i16>,
    pub has_service: bool,
}

/// One scan window; everything seen, with a flag for the ones advertising our service.
pub async fn scan(adapter: &Adapter, timeout: Duration) -> Result<Vec<Seen>> {
    adapter.start_scan(ScanFilter::default()).await.context("starting BLE scan")?;
    tokio::time::sleep(timeout).await;
    let mut out = Vec::new();
    for p in adapter.peripherals().await? {
        if let Ok(Some(props)) = p.properties().await {
            out.push(Seen {
                name: props.local_name,
                id: p.id().to_string(),
                rssi: props.rssi,
                has_service: props.services.contains(&SERVICE_UUID),
            });
        }
    }
    let _ = adapter.stop_scan().await;
    Ok(out)
}

/// Find a watch advertising the Theme service (optionally by name). Polls the adapter's
/// device list rather than the event stream: simpler, and identical on BlueZ/CoreBluetooth.
pub async fn discover(adapter: &Adapter, opts: &BleOptions) -> Result<Peripheral> {
    discover_service(adapter, opts, SERVICE_UUID).await
}

pub async fn discover_service(adapter: &Adapter, opts: &BleOptions, service: Uuid) -> Result<Peripheral> {
    // Unfiltered scan on purpose: the OW-Watch puts the service UUID in the scan response,
    // not the advertisement, and a BlueZ UUID filter did not surface it (found only when
    // BlueZ already had it cached from an earlier `bluetoothctl scan`). Matching happens
    // below on the collected properties instead.
    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("starting BLE scan (is Bluetooth powered on?)")?;
    let deadline = Instant::now() + opts.scan_timeout;
    let result = loop {
        let mut found = None;
        // No `?` inside the loop: every exit must go through `stop_scan()` below, or the
        // next `start_scan` on BlueZ fails with "Operation already in progress".
        // A transient D-Bus error here ("Remote peer disconnected" when a device vanishes
        // mid-listing) is not a reason to give up on the whole scan.
        let peripherals = adapter.peripherals().await.unwrap_or_default();
        for p in peripherals {
            let Ok(Some(props)) = p.properties().await else { continue };
            let by_service = props.services.contains(&service);
            let by_name = match (&opts.name, &props.local_name) {
                (Some(want), Some(have)) => want == have,
                (Some(_), None) => false,
                (None, _) => true,
            };
            // A name match alone is accepted only when the caller asked for a specific name
            // and the stack surfaced no service list at all (scan-response-only names).
            if (by_service && by_name) || (opts.name.is_some() && by_name && props.services.is_empty()) {
                found = Some(p);
                break;
            }
        }
        if let Some(p) = found {
            break Ok(p);
        }
        if Instant::now() >= deadline {
            break Err(anyhow!(
                "no device advertising {} found within {:?}{}",
                service,
                opts.scan_timeout,
                opts.name.as_ref().map(|n| format!(" (name filter: {n:?})")).unwrap_or_default()
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let _ = adapter.stop_scan().await;
    result
}

pub struct Watch {
    peripheral: Peripheral,
    theme: Characteristic,
    status: Option<Characteristic>,
    control: Option<Characteristic>,
    info: Option<Characteristic>,
    pub name: String,
}

impl Watch {
    /// Connect and resolve the Theme service characteristics.
    pub async fn connect(peripheral: Peripheral) -> Result<Watch> {
        if !peripheral.is_connected().await.unwrap_or(false) {
            peripheral.connect().await.context("connecting to the watch")?;
        }
        peripheral.discover_services().await.context("discovering GATT services")?;
        let chars = peripheral.characteristics();
        let find = |uuid: Uuid| chars.iter().find(|c| c.uuid == uuid).cloned();
        let theme = find(CHR_THEME_STATE).ok_or_else(|| {
            anyhow!("connected, but the device has no Theme State characteristic {CHR_THEME_STATE} — wrong device or old firmware")
        })?;
        let name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name)
            .unwrap_or_else(|| peripheral.id().to_string());
        Ok(Watch { theme, status: find(CHR_STATUS), control: find(CHR_CONTROL), info: find(CHR_INFO), peripheral, name })
    }

    pub async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    pub async fn disconnect(&self) {
        let _ = self.peripheral.disconnect().await;
    }

    pub async fn info(&self) -> Result<Option<Info>> {
        match &self.info {
            None => Ok(None),
            Some(c) => Ok(Some(Info::decode(&self.peripheral.read(c).await.context("reading Info")?)?)),
        }
    }

    pub async fn status(&self) -> Result<Option<Status>> {
        match &self.status {
            None => Ok(None),
            Some(c) => Ok(Some(Status::decode(&self.peripheral.read(c).await.context("reading Status")?)?)),
        }
    }

    /// Read back the last ThemeState the watch holds (what it would show after a reboot).
    pub async fn read_theme(&self) -> Result<Vec<u8>> {
        self.peripheral.read(&self.theme).await.context("reading Theme State")
    }

    /// Write a ThemeState packet and confirm it via Status (crc of the applied packet).
    /// If the firmware has no Status characteristic the write itself is the ack.
    pub async fn send_theme(&self, packet: &[u8]) -> Result<Option<Status>> {
        if packet.len() > protocol::MAX_PACKET_LEN {
            bail!("packet is {} bytes, the watch accepts at most {}", packet.len(), protocol::MAX_PACKET_LEN);
        }
        let write_type = if self.theme.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            WriteType::WithoutResponse
        };
        self.peripheral.write(&self.theme, packet, write_type).await.context("writing Theme State")?;
        let Some(status) = self.status().await? else { return Ok(None) };
        let expected = u16::from_le_bytes([packet[packet.len() - 2], packet[packet.len() - 1]]);
        match status.result {
            StatusCode::Ok if status.applied_crc == expected => Ok(Some(status)),
            StatusCode::Ok => bail!(
                "watch reports OK but for crc {:#06x}, we sent {:#06x} (stale status?)",
                status.applied_crc,
                expected
            ),
            other => bail!("watch rejected the theme: {other:?}"),
        }
    }

    #[allow(dead_code)]
    pub fn has_control(&self) -> bool {
        self.control.is_some()
    }

    /// Subscribe to the Control characteristic (watch -> desktop requests) and Status.
    /// Returns the merged notification stream; filter on `uuid`.
    pub async fn subscribe(&self) -> Result<Pin<Box<dyn Stream<Item = ValueNotification> + Send>>> {
        if let Some(c) = &self.control {
            self.peripheral.subscribe(c).await.context("subscribing to Control")?;
        }
        if let Some(c) = &self.status {
            self.peripheral.subscribe(c).await.context("subscribing to Status")?;
        }
        Ok(self.peripheral.notifications().await?)
    }
}

/// Discover + connect with retries. `attempts == 0` means forever.
pub async fn connect_with_retry(adapter: &Adapter, opts: &BleOptions, attempts: u32, log: impl Fn(&str)) -> Result<Watch> {
    let mut delay = Duration::from_millis(500);
    let mut n = 0u32;
    loop {
        n += 1;
        let step = async {
            let p = discover(adapter, opts).await?;
            Watch::connect(p).await
        };
        match step.await {
            Ok(w) => return Ok(w),
            Err(e) if attempts == 0 || n < attempts => {
                log(&format!("attempt {n}: {e:#}; retrying in {delay:?}"));
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(15));
            }
            Err(e) => return Err(e.context(format!("giving up after {n} attempts"))),
        }
    }
}

// ---- "mini" wire adapter --------------------------------------------------------------
// The onewheel watch's first prototype firmware (main/ble.c there) speaks a 13-byte format
// on a different service; this adapter lets the same host pipeline drive it. It is the
// existence proof that the source-side code does not care what the watch speaks.
//
//   service  7a0e0001-0f0e-4d0c-9c0b-0a0908070605
//   colors   7a0e0002-…  write/read: [ver=1][bg rgb][fg rgb][accent rgb][color1 rgb]
//   name     7a0e0003-…  write/read: UTF-8, <= 31 bytes

pub const MINI_SERVICE_UUID: Uuid = uuid!("7a0e0001-0f0e-4d0c-9c0b-0a0908070605");
pub const MINI_CHR_COLORS: Uuid = uuid!("7a0e0002-0f0e-4d0c-9c0b-0a0908070605");
pub const MINI_CHR_NAME: Uuid = uuid!("7a0e0003-0f0e-4d0c-9c0b-0a0908070605");
/// Pairing key for beacon requests (protocol/BEACON.md §2b): write-only `[0x01][code][key]`.
pub const MINI_CHR_KEY: Uuid = uuid!("7a0e0005-0f0e-4d0c-9c0b-0a0908070605");
/// The theme list (protocol/BEACON.md §3): read = 6-byte status, write = BEGIN/DATA/COMMIT frames.
pub const MINI_CHR_LIST: Uuid = uuid!("7a0e0006-0f0e-4d0c-9c0b-0a0908070605");

/// Connect, write a `colors` packet (13-byte legacy or the firmware's v2 TLV), optionally the
/// legacy `name` characteristic, and read `colors` back. Returns the read-back bytes as-is:
/// the current firmware answers with the palette it applied, in its v2 format.
pub async fn send_colors(peripheral: &Peripheral, wire: &[u8], name: Option<&str>) -> Result<Vec<u8>> {
    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await.context("connecting to the watch")?;
    }
    peripheral.discover_services().await.context("discovering GATT services")?;
    let chars = peripheral.characteristics();
    let colors = chars.iter().find(|c| c.uuid == MINI_CHR_COLORS).cloned().ok_or_else(|| anyhow!("no colors characteristic {MINI_CHR_COLORS}"))?;
    peripheral.write(&colors, wire, WriteType::WithResponse).await.context("writing colors")?;
    if let Some(name) = name {
        if let Some(nc) = chars.iter().find(|c| c.uuid == MINI_CHR_NAME).cloned() {
            let n: String = name.chars().take_while({ let mut len = 0; move |c| { len += c.len_utf8(); len <= 31 } }).collect();
            peripheral.write(&nc, n.as_bytes(), WriteType::WithResponse).await.context("writing name")?;
        }
    }
    let back = peripheral.read(&colors).await.context("reading colors back")?;
    Ok(back)
}

/// Write one arbitrary characteristic on the watch's theme service (used for the pairing key).
pub async fn write_characteristic(peripheral: &Peripheral, uuid: Uuid, value: &[u8]) -> Result<()> {
    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await.context("connecting to the watch")?;
    }
    peripheral.discover_services().await.context("discovering GATT services")?;
    let chr = peripheral.characteristics().into_iter().find(|c| c.uuid == uuid).ok_or_else(|| anyhow!("the watch has no characteristic {uuid} (firmware without it?)"))?;
    peripheral.write(&chr, value, WriteType::WithResponse).await.with_context(|| format!("writing {uuid}"))?;
    Ok(())
}

/// Find a peripheral by Bluetooth address (`AA:BB:CC:DD:EE:FF`, any case): the daemon knows
/// the watch's address from the request it just scanned. BlueZ only; CoreBluetooth hides
/// addresses, so there this never matches and callers fall back to [`discover_service`].
pub async fn discover_by_address(adapter: &Adapter, addr: &str, timeout: Duration) -> Result<Peripheral> {
    let want = addr.to_ascii_uppercase();
    adapter.start_scan(ScanFilter::default()).await.context("starting BLE scan (is Bluetooth powered on?)")?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        let hit = adapter.peripherals().await.unwrap_or_default().into_iter().find(|p| p.address().to_string().to_ascii_uppercase() == want);
        if let Some(p) = hit {
            break Ok(p);
        }
        if Instant::now() >= deadline {
            break Err(anyhow!("{addr} not seen within {timeout:?}"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let _ = adapter.stop_scan().await;
    result
}

/// The watch at `addr` if it is known, else any watch advertising the theme service.
pub async fn find_watch(adapter: &Adapter, opts: &BleOptions, addr: Option<&str>) -> Result<Peripheral> {
    if let Some(a) = addr {
        if let Ok(p) = discover_by_address(adapter, a, opts.scan_timeout).await {
            return Ok(p);
        }
    }
    discover_service(adapter, opts, MINI_SERVICE_UUID).await
}

/// How a list push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListPush {
    /// The watch already held this list (same crc and count): nothing was written.
    Skipped(crate::themelist::ListStatus),
    Pushed { frames: usize, frame_len: usize, status: Option<crate::themelist::ListStatus> },
}

impl std::fmt::Display for ListPush {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListPush::Skipped(s) => write!(f, "watch already holds it ({s})"),
            ListPush::Pushed { frames, frame_len, status: Some(s) } => write!(f, "pushed in {frames} writes of <= {frame_len} B; watch reports {s}"),
            ListPush::Pushed { frames, frame_len, status: None } => write!(f, "pushed in {frames} writes of <= {frame_len} B (status not readable)"),
        }
    }
}

/// Add the watch's meaning of an ATT application error (0x80..0x84) to a write failure.
fn with_att_hint(e: anyhow::Error) -> anyhow::Error {
    let text = format!("{e:#}");
    let lower = text.to_ascii_lowercase();
    for code in crate::themelist::ATT_ERROR_FIRST..=crate::themelist::ATT_ERROR_LAST {
        if lower.contains(&format!("0x{code:02x}")) || lower.contains(&format!("error {code}")) {
            if let Some(m) = crate::themelist::att_error_meaning(code) {
                return e.context(format!("watch answered ATT error {code:#04x}: {m}"));
            }
        }
    }
    e
}

/// Push the theme list (protocol/BEACON.md §3): read the status, skip when the watch already
/// holds the same bytes (unless `force`), else BEGIN / DATA… / COMMIT, one write-with-response
/// each, DATA frames sized to the negotiated MTU (`frame` overrides), then read the status
/// back to confirm the commit. The peripheral is left connected; the caller disconnects.
pub async fn push_list(peripheral: &Peripheral, list: &[u8], key: &[u8], force: bool, frame: Option<usize>, log: impl Fn(&str)) -> Result<ListPush> {
    use crate::themelist::{self, ListStatus};
    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await.context("connecting to the watch")?;
    }
    peripheral.discover_services().await.context("discovering GATT services")?;
    let chr = peripheral
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == MINI_CHR_LIST)
        .ok_or_else(|| anyhow!("the watch has no list characteristic {MINI_CHR_LIST} (firmware without the theme list?)"))?;
    let status = match peripheral.read(&chr).await {
        Ok(b) => match ListStatus::decode(&b) {
            Ok(s) => Some(s),
            Err(e) => { log(&format!("list status unreadable ({e}); pushing anyway")); None }
        },
        Err(e) => { log(&format!("list status read failed ({e}); pushing anyway")); None }
    };
    if let Some(s) = status {
        log(&format!("watch list status: {s}"));
        if !force && s.holds(list) {
            return Ok(ListPush::Skipped(s));
        }
    }
    let mtu = peripheral.mtu();
    let frame_len = frame.map(|f| f.clamp(themelist::MIN_FRAME, themelist::MAX_FRAME)).unwrap_or_else(|| themelist::frame_len_for_mtu(mtu));
    let frames = themelist::frames(list, key, frame_len);
    log(&format!("mtu {mtu}: {} bytes in {} writes of <= {frame_len} B", list.len(), frames.len()));
    for f in &frames {
        peripheral
            .write(&chr, f, WriteType::WithResponse)
            .await
            .map_err(anyhow::Error::from)
            .map_err(with_att_hint)
            .with_context(|| format!("writing {}", themelist::describe_frame(f)))?;
    }
    let status = match peripheral.read(&chr).await {
        Ok(b) => ListStatus::decode(&b).ok(),
        Err(_) => None,
    };
    if let Some(s) = status {
        if !s.holds(list) {
            bail!("COMMIT accepted but the watch now reports {s}, expected {} themes with crc {:#06x}", list.get(1).copied().unwrap_or(0), crate::protocol::crc16(list));
        }
    }
    Ok(ListPush::Pushed { frames: frames.len(), frame_len, status })
}
