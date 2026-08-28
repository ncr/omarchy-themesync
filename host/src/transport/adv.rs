//! The desktop's side of `protocol/BEACON.md` over BlueZ (D-Bus, via `bluer`): registering
//! the state beacon as an extended advertisement, and scanning for the watch's requests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use bluer::adv::{Advertisement, AdvertisementHandle, SecondaryChannel, Type};
use bluer::monitor::{self, Monitor, Pattern};
use bluer::{Adapter, AdapterEvent, Address, DeviceEvent, DeviceProperty, DiscoveryFilter, DiscoveryTransport, Session};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::beacon::{self, COMPANY_ID};

pub struct Radio {
    _session: Session,
    pub adapter: Adapter,
    handle: Option<AdvertisementHandle>,
}

/// Why the radio cannot be used right now. `Retry` is a condition that goes away on its own
/// (bluetoothd not up yet, the adapter powered off by the user); `Fatal` never will.
#[derive(Debug)]
pub enum OpenError {
    Retry(anyhow::Error),
    Fatal(anyhow::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Retry(e) | OpenError::Fatal(e) => write!(f, "{e:#}"),
        }
    }
}

impl Radio {
    /// Open the default adapter. The adapter is not powered on here: a user who switched
    /// Bluetooth off has decided, and the daemon waits (the caller retries) until it is on.
    /// A controller without extended advertising is refused for good: the beacon is ~80
    /// bytes, legacy advertising carries 31.
    pub async fn open() -> std::result::Result<Radio, OpenError> {
        let session = Session::new().await.context("connecting to BlueZ over D-Bus (is bluetooth.service running?)").map_err(OpenError::Retry)?;
        let adapter = session.default_adapter().await.context("no Bluetooth adapter").map_err(OpenError::Retry)?;
        if !adapter.is_powered().await.context("reading the adapter's power state").map_err(OpenError::Retry)? {
            return Err(OpenError::Retry(anyhow::anyhow!("adapter {} is powered off (bluetoothctl power on, or the Bluetooth switch in the bar)", adapter.name())));
        }
        let channels = adapter.supported_advertising_secondary_channels().await.ok().flatten().unwrap_or_default();
        if !channels.contains(&SecondaryChannel::OneM) {
            return Err(OpenError::Fatal(anyhow::anyhow!(
                "adapter {} has no extended advertising (BLE 5): the theme beacon needs it. BlueZ reports secondary channels {:?}",
                adapter.name(),
                channels
            )));
        }
        Ok(Radio { _session: session, adapter, handle: None })
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.name().to_string()
    }

    /// (Re)register the state beacon. BlueZ has no "update data" call, so a new advertisement
    /// is registered and only then the old one dropped: a registration that fails leaves the
    /// previous beacon on the air. If the controller has no free instance for the second
    /// one, the old one is dropped first and the registration tried once more — the gap is a
    /// few milliseconds. Extended advertising (secondary channel 1M) because the payload
    /// exceeds 31 bytes. `interval` is requested as min == max: a range lets the controller
    /// pick anything in it (BEACON.md §1).
    pub async fn set_beacon(&mut self, data: Vec<u8>, interval: Duration) -> Result<()> {
        let adv = Advertisement {
            advertisement_type: Type::Broadcast,
            manufacturer_data: [(COMPANY_ID, data)].into_iter().collect(),
            secondary_channel: Some(SecondaryChannel::OneM),
            min_interval: Some(interval),
            max_interval: Some(interval),
            ..Default::default()
        };
        let handle = match self.adapter.advertise(adv.clone()).await {
            Ok(h) => h,
            Err(first) => {
                if self.handle.is_none() {
                    return Err(first).context("registering the state beacon with BlueZ");
                }
                self.handle.take();
                self.adapter.advertise(adv).await.with_context(|| format!("re-registering the state beacon with BlueZ (first attempt: {first})"))?
            }
        };
        self.handle = Some(handle);
        Ok(())
    }

    /// Scan for the watch's requests, forever; `on_packet(addr, data)` gets every
    /// manufacturer-data payload of ours (filtered by `beacon::is_ours`).
    ///
    /// BlueZ discovery is an *active* scan with duplicates reported. The payload is taken
    /// from the `ManufacturerData` property-changed signal of each device, never by
    /// re-reading the device after a wake-up: a re-read samples BlueZ's cache after a D-Bus
    /// round trip and misses a request the watch replaced in the meantime. Every device the
    /// adapter reports gets one subscription; the cached value is never read (see below).
    ///
    /// `monitor_ok` is set to whether the Advertisement Monitor could be registered.
    pub async fn scan_ours(&self, monitor_ok: &Mutex<Option<bool>>, mut on_packet: impl FnMut(Address, &[u8])) -> Result<()> {
        // An Advertisement Monitor on our manufacturer data (company 0xFFFF, magic 'T',
        // kind request). Its events are not what we consume — the point is a side effect in
        // the kernel: while any monitor is registered, LE scanning runs with the controller's
        // duplicate filter *disabled* (hci_sync.c, hci_active_scan_sync). With the filter on,
        // this adapter deduplicates by address, so a request the watch swaps into its
        // advertisement stays invisible until the kernel's periodic scan restart — 0–2 s of
        // latency, and a short-lived request lost outright (hardware, 2026-08-27, #203).
        // bluetoothd offers AdvertisementMonitorManager1 only with `Experimental = true` in
        // /etc/bluetooth/main.conf; without it, scan anyway and say so loudly.
        let manager = self.adapter.monitor().await.ok();
        let mut monitor = match &manager {
            Some(m) => match m
                .register(Monitor {
                    monitor_type: monitor::Type::OrPatterns,
                    patterns: Some(vec![Pattern::new(0xFF, 0, &[0xFF, 0xFF, beacon::MAGIC, beacon::KIND_REQUEST])]),
                    ..Default::default()
                })
                .await
            {
                Ok(h) => Some(h),
                Err(e) => { eprintln!("[themesync] WARNING: advertisement monitor not registered ({e}): the controller may filter duplicates by address; requests from the watch will be slow (0–2 s) or lost"); None }
            },
            None => { eprintln!("[themesync] WARNING: bluetoothd has no AdvertisementMonitorManager1: set `Experimental = true` in /etc/bluetooth/main.conf and `sudo systemctl restart bluetooth`; until then requests from the watch will be slow (0–2 s) or lost"); None }
        };
        *monitor_ok.lock().unwrap() = Some(monitor.is_some());
        let filter = DiscoveryFilter { transport: DiscoveryTransport::Le, duplicate_data: true, ..Default::default() };
        self.adapter.set_discovery_filter(filter).await.context("setting the discovery filter")?;
        let mut events = self.adapter.discover_devices().await.context("starting discovery")?;
        let (tx, mut rx) = mpsc::channel::<(Address, Vec<u8>)>(64);
        // Devices with a live event subscription: the task's abort handle and a generation
        // number, so a task that ends late (after DeviceRemoved + DeviceAdded replaced it)
        // does not remove its successor's entry.
        let watched: Arc<Mutex<HashMap<Address, (u64, AbortHandle)>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut generation: u64 = 0;
        loop {
            tokio::select! {
                ev = events.next() => {
                    match ev {
                        None => anyhow::bail!("discovery stream ended"),
                        Some(AdapterEvent::DeviceRemoved(addr)) => {
                            if let Some((_, h)) = watched.lock().unwrap().remove(&addr) { h.abort(); }
                        }
                        Some(AdapterEvent::DeviceAdded(addr)) => {
                            if watched.lock().unwrap().contains_key(&addr) { continue; }
                            let Ok(dev) = self.adapter.device(addr) else { continue };
                            generation += 1;
                            let gen = generation;
                            let tx = tx.clone();
                            let watched2 = watched.clone();
                            let task = tokio::spawn(async move {
                                if let Ok(stream) = dev.events().await {
                                    // No initial read of the cached property: BlueZ keeps a
                                    // device's last ManufacturerData long after it stopped
                                    // advertising it, and a minutes-old request would pass the
                                    // counter check (never accepted = still "new"). Only live
                                    // changes count; a request on the air during the ~1 s of a
                                    // daemon restart is lost (the watch times out and repaints).
                                    let mut stream = std::pin::pin!(stream);
                                    // Every event, not just ours: BlueZ sends an RSSI change
                                    // every second or two, and a `while let` on the
                                    // ManufacturerData pattern alone would end the loop there
                                    // (found on hardware 2026-08-27: requests #102–#104 lost).
                                    while let Some(ev) = stream.next().await {
                                        if let DeviceEvent::PropertyChanged(DeviceProperty::ManufacturerData(md)) = ev {
                                            forward(&tx, addr, md).await;
                                        }
                                    }
                                }
                                let mut w = watched2.lock().unwrap();
                                if w.get(&addr).map(|(g, _)| *g) == Some(gen) {
                                    w.remove(&addr);
                                }
                            });
                            watched.lock().unwrap().insert(addr, (gen, task.abort_handle()));
                        }
                        Some(_) => {}
                    }
                }
                Some((addr, data)) = rx.recv() => on_packet(addr, &data),
                Some(_) = async { match monitor.as_mut() { Some(m) => m.next().await, None => std::future::pending().await } } => {
                    // DeviceFound/DeviceLost from the monitor: drained, not used (the
                    // property-changed path above carries the payload).
                }
            }
        }
    }
}

async fn forward(tx: &mpsc::Sender<(Address, Vec<u8>)>, addr: Address, md: HashMap<u16, Vec<u8>>) {
    if let Some(data) = md.get(&COMPANY_ID) {
        if beacon::is_ours(data) {
            let _ = tx.send((addr, data.clone())).await;
        }
    }
}
