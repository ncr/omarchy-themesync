//! The desktop's side of `protocol/BEACON.md` over BlueZ (D-Bus, via `bluer`): registering
//! the state beacon as an extended advertisement, and scanning for the watch's requests.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use bluer::adv::{Advertisement, AdvertisementHandle, SecondaryChannel, Type};
use bluer::monitor::{self, Monitor, MonitorEvent, Pattern};
use bluer::{Adapter, AdapterEvent, Address, DeviceEvent, DeviceProperty, DiscoveryFilter, DiscoveryTransport, Session};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::beacon::{self, COMPANY_ID};

pub struct Radio {
    _session: Session,
    pub adapter: Adapter,
    handle: Option<AdvertisementHandle>,
}

impl Radio {
    pub async fn open() -> Result<Radio> {
        let session = Session::new().await.context("connecting to BlueZ over D-Bus")?;
        let adapter = session.default_adapter().await.context("no Bluetooth adapter")?;
        adapter.set_powered(true).await.context("powering the adapter")?;
        Ok(Radio { _session: session, adapter, handle: None })
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.name().to_string()
    }

    /// (Re)register the state beacon. BlueZ has no "update data" call, so the old
    /// advertisement is dropped and a new one registered; the gap is a few milliseconds.
    /// Extended advertising (secondary channel 1M) because the payload exceeds 31 bytes.
    /// `interval` is requested as min == max: a range lets the controller pick anything in
    /// it, and with the mandatory 0–10 ms advDelay per event a 100–110 ms range reached the
    /// watch's 120 ms scan window with no margin (BEACON.md §1).
    pub async fn set_beacon(&mut self, data: Vec<u8>, interval: Duration) -> Result<()> {
        self.handle.take();
        let adv = Advertisement {
            advertisement_type: Type::Broadcast,
            manufacturer_data: [(COMPANY_ID, data)].into_iter().collect(),
            secondary_channel: Some(SecondaryChannel::OneM),
            min_interval: Some(interval),
            max_interval: Some(interval),
            ..Default::default()
        };
        let handle = self.adapter.advertise(adv).await.context("registering the state beacon with BlueZ")?;
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
    pub async fn scan_ours(&self, mut on_packet: impl FnMut(Address, &[u8])) -> Result<()> {
        // An Advertisement Monitor on our manufacturer data (company 0xFFFF, magic 'T',
        // kind request). Its events are not what we consume — the point is a side effect in
        // the kernel: while any monitor is registered, LE scanning runs with the controller's
        // duplicate filter *disabled* (hci_sync.c, hci_active_scan_sync). With the filter on,
        // this adapter deduplicates by address, so a request the watch swaps into its
        // advertisement stays invisible until the kernel's periodic scan restart — 0–2 s of
        // latency, and a short-lived request lost outright (hardware, 2026-08-27, #203).
        // If bluetoothd has no AdvertisementMonitorManager1, scan without it and say so.
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
                Err(e) => { eprintln!("[themesync] advertisement monitor not registered ({e}); the controller may filter duplicates by address"); None }
            },
            None => { eprintln!("[themesync] no AdvertisementMonitorManager1 in bluetoothd; the controller may filter duplicates by address"); None }
        };
        let filter = DiscoveryFilter { transport: DiscoveryTransport::Le, duplicate_data: true, ..Default::default() };
        self.adapter.set_discovery_filter(filter).await.context("setting the discovery filter")?;
        let mut events = self.adapter.discover_devices().await.context("starting discovery")?;
        let (tx, mut rx) = mpsc::channel::<(Address, Vec<u8>)>(64);
        // Devices with a live event subscription. A task removes its address when its
        // stream ends, so the next DeviceAdded for that device subscribes again.
        let watched: Arc<Mutex<HashSet<Address>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            tokio::select! {
                ev = events.next() => {
                    match ev {
                        None => anyhow::bail!("discovery stream ended"),
                        Some(AdapterEvent::DeviceRemoved(addr)) => { watched.lock().unwrap().remove(&addr); }
                        Some(AdapterEvent::DeviceAdded(addr)) => {
                            if !watched.lock().unwrap().insert(addr) { continue; }
                            let Ok(dev) = self.adapter.device(addr) else { watched.lock().unwrap().remove(&addr); continue };
                            let tx = tx.clone();
                            let watched = watched.clone();
                            tokio::spawn(async move {
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
                                watched.lock().unwrap().remove(&addr);
                            });
                        }
                        Some(_) => {}
                    }
                }
                Some((addr, data)) = rx.recv() => on_packet(addr, &data),
                Some(ev) = async { match monitor.as_mut() { Some(m) => m.next().await, None => std::future::pending().await } } => {
                    if let MonitorEvent::DeviceFound(id) = ev {
                        eprintln!("[themesync] monitor: {} carries our data", id.device);
                    }
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
