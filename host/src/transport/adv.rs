//! BlueZ advertising and passive scanning through D-Bus (`bluer`): the desktop's state
//! beacon goes out here, the watch's requests come in here. Linux only.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use bluer::adv::{Advertisement, AdvertisementHandle, SecondaryChannel, Type};
use bluer::{Adapter, AdapterEvent, Address, DiscoveryFilter, DiscoveryTransport, Session};
use futures::StreamExt;

use crate::beacon::{self, COMPANY_ID};

pub struct Radio {
    _session: Session,
    pub adapter: Adapter,
    handle: Option<AdvertisementHandle>,
}

impl Radio {
    pub async fn open() -> Result<Radio> {
        let session = Session::new().await.context("connecting to bluetoothd over D-Bus")?;
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
    pub async fn set_beacon(&mut self, data: Vec<u8>, interval: Duration) -> Result<()> {
        self.handle.take();
        let adv = Advertisement {
            advertisement_type: Type::Broadcast,
            manufacturer_data: [(COMPANY_ID, data)].into_iter().collect(),
            secondary_channel: Some(SecondaryChannel::OneM),
            min_interval: Some(interval),
            max_interval: Some(interval + Duration::from_millis(10)),
            ..Default::default()
        };
        let handle = self.adapter.advertise(adv).await.context("registering the state beacon with BlueZ")?;
        self.handle = Some(handle);
        Ok(())
    }

    /// Passive LE scan with duplicates reported, forever; `on_packet(addr, data, cached)` gets
    /// every manufacturer-data payload of ours (either kind, filtered by `beacon::is_ours`).
    ///
    /// BlueZ keeps a device's last ManufacturerData in its cache after the device stopped
    /// advertising it, and hands it back on every property change (RSSI included). So the
    /// payloads already cached when the scan starts are delivered first with `cached = true`
    /// — they are history, not new requests — and the caller must dedup repeats itself.
    pub async fn scan_ours(&self, mut on_packet: impl FnMut(Address, &[u8], bool)) -> Result<()> {
        if let Ok(addrs) = self.adapter.device_addresses().await {
            for addr in addrs {
                let Ok(dev) = self.adapter.device(addr) else { continue };
                let md: Option<HashMap<u16, Vec<u8>>> = dev.manufacturer_data().await.unwrap_or(None);
                if let Some(data) = md.and_then(|m| m.get(&COMPANY_ID).cloned()) {
                    if beacon::is_ours(&data) {
                        on_packet(addr, &data, true);
                    }
                }
            }
        }
        let filter = DiscoveryFilter { transport: DiscoveryTransport::Le, duplicate_data: true, ..Default::default() };
        self.adapter.set_discovery_filter(filter).await.context("setting the discovery filter")?;
        let mut events = self.adapter.discover_devices_with_changes().await.context("starting discovery")?;
        while let Some(ev) = events.next().await {
            if let AdapterEvent::DeviceAdded(addr) = ev {
                let Ok(dev) = self.adapter.device(addr) else { continue };
                let md: Option<HashMap<u16, Vec<u8>>> = dev.manufacturer_data().await.unwrap_or(None);
                if let Some(data) = md.and_then(|m| m.get(&COMPANY_ID).cloned()) {
                    if beacon::is_ours(&data) {
                        on_packet(addr, &data, false);
                    }
                }
            }
        }
        anyhow::bail!("discovery stream ended")
    }
}
