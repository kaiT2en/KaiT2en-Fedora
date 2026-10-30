// SPDX-License-Identifier: GPL-3.0-or-later
//! Finding phones to send to. Android only announces its Quick Share mDNS
//! service after it saw a matching BLE advertisement, so discovery sends
//! one through BlueZ while it browses.
//!
//! The advertisement contents follow NearDrop PROTOCOL.md and rquickshare
//! `core_lib/src/hdl/blea.rs` (commit 378d8ae, GPL-3.0).

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use log::{debug, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface, proxy};

use crate::endpoint::{DeviceType, parse_endpoint_info};
use crate::link::random_bytes;
use crate::mdns::SERVICE_TYPE;

/// A phone that announced its Quick Share service.
#[derive(Clone, Debug)]
pub struct Peer {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub device_type: DeviceType,
    pub addr: SocketAddr,
}

impl Peer {
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("Hidden device {}", self.endpoint_id))
    }
}

/// Browses for phones while a BLE advertisement wakes them up. Both stop
/// when this is dropped.
pub struct Discovery {
    daemon: ServiceDaemon,
    events: mdns_sd::Receiver<ServiceEvent>,
    local: HashSet<IpAddr>,
    seen: HashSet<String>,
    _ble: Option<BleAdvertisement>,
}

impl Discovery {
    pub async fn start() -> Result<Self> {
        let ble = match BleAdvertisement::start().await {
            Ok(ble) => Some(ble),
            Err(err) => {
                warn!("no BLE advertisement, phones may not show up: {err:#}");
                None
            }
        };
        let daemon = ServiceDaemon::new().context("mDNS daemon")?;
        let events = daemon.browse(SERVICE_TYPE).context("mDNS browse")?;
        let local = if_addrs::get_if_addrs()
            .map(|addrs| addrs.into_iter().map(|a| a.ip()).collect())
            .unwrap_or_default();
        Ok(Self { daemon, events, local, seen: HashSet::new(), _ble: ble })
    }

    /// The next phone not seen before. Services of this machine, such as
    /// the receiving service, are skipped.
    pub async fn next(&mut self) -> Result<Peer> {
        loop {
            let event = self.events.recv_async().await.map_err(|_| anyhow!("mDNS stopped"))?;
            let ServiceEvent::ServiceResolved(service) = event else { continue };
            let Some(addr) = service
                .get_addresses_v4()
                .into_iter()
                .map(IpAddr::V4)
                .find(|ip| !self.local.contains(ip))
            else {
                continue;
            };
            let Some(peer) = parse_service(&service.fullname, service.get_property_val_str("n"))
            else {
                debug!("ignoring {}", service.fullname);
                continue;
            };
            if !self.seen.insert(peer.0.clone()) {
                continue;
            }
            let (endpoint_id, device_type, name) = peer;
            let peer = Peer {
                endpoint_id,
                name,
                device_type,
                addr: SocketAddr::new(addr, service.get_port()),
            };
            info!("found {} at {}", peer.label(), peer.addr);
            return Ok(peer);
        }
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

/// Endpoint id, device type and name from the instance name and the `n`
/// TXT record.
fn parse_service(
    fullname: &str,
    endpoint_info: Option<&str>,
) -> Option<(String, DeviceType, Option<String>)> {
    let instance = fullname.split('.').next()?;
    let raw = URL_SAFE_NO_PAD.decode(instance).ok()?;
    if raw.len() < 8 || raw[0] != 0x23 || raw[5..8] != [0xfc, 0x9f, 0x5e] {
        return None;
    }
    let endpoint_id = String::from_utf8_lossy(&raw[1..5]).into_owned();
    let info = URL_SAFE_NO_PAD.decode(endpoint_info?).ok()?;
    let (device_type, name) = parse_endpoint_info(&info)?;
    Some((endpoint_id, device_type, name))
}

#[proxy(interface = "org.bluez.LEAdvertisingManager1", default_service = "org.bluez")]
trait LEAdvertisingManager1 {
    fn register_advertisement(
        &self,
        advertisement: &ObjectPath<'_>,
        options: HashMap<&str, &Value<'_>>,
    ) -> zbus::Result<()>;

    fn unregister_advertisement(&self, advertisement: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.DBus.ObjectManager",
    default_service = "org.bluez",
    default_path = "/"
)]
trait BluezObjects {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

const ADVERTISEMENT_PATH: &str = "/org/t2airshare/advertisement";
/// The 16 bit service UUID 0xFE2C in its 128 bit form.
const QUICK_SHARE_UUID: &str = "0000fe2c-0000-1000-8000-00805f9b34fb";

/// The advertisement object BlueZ reads its contents from.
struct Advertisement {
    service_data: Vec<u8>,
}

#[interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    fn release(&self) {}

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "broadcast".to_string()
    }

    #[zbus(property)]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        let value = OwnedValue::try_from(Value::from(self.service_data.clone()))
            .expect("byte array converts");
        HashMap::from([(QUICK_SHARE_UUID.to_string(), value)])
    }
}

/// A registered BLE advertisement, removed on drop.
struct BleAdvertisement {
    connection: Connection,
    adapter: OwnedObjectPath,
}

impl BleAdvertisement {
    async fn start() -> Result<Self> {
        let connection = Connection::system().await.context("system bus")?;
        let objects = BluezObjectsProxy::new(&connection).await?.get_managed_objects().await?;
        let (adapter, interfaces) = objects
            .into_iter()
            .find(|(_, interfaces)| interfaces.contains_key("org.bluez.LEAdvertisingManager1"))
            .context("no Bluetooth adapter with LE advertising")?;
        let powered = interfaces
            .get("org.bluez.Adapter1")
            .and_then(|adapter| adapter.get("Powered"))
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        if !powered {
            anyhow::bail!("Bluetooth is off, phones need it to show up");
        }

        // Fast Initiation beacon, layout from google/nearby
        // sharing/internal/api/fast_init_ble_beacon.h: model id, metadata,
        // adjusted tx power, UWB metadata and address, salt, secret id hash
        // and a flags byte. The flags stay 0, a set self_only_advertising
        // bit would only wake devices of the same account.
        let mut service_data = vec![0xfc, 0x12, 0x8e, 0x01, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        service_data.extend(random_bytes(9));
        service_data.push(0);
        connection
            .object_server()
            .at(ADVERTISEMENT_PATH, Advertisement { service_data })
            .await?;
        let path = ObjectPath::try_from(ADVERTISEMENT_PATH)?;
        LEAdvertisingManager1Proxy::builder(&connection)
            .path(adapter.clone())?
            .build()
            .await?
            .register_advertisement(&path, HashMap::new())
            .await
            .context("register BLE advertisement")?;
        debug!("BLE advertisement registered on {}", adapter.as_str());
        Ok(Self { connection, adapter })
    }
}

impl Drop for BleAdvertisement {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let adapter = self.adapter.clone();
        // Unregistering needs the bus, which only async code can use.
        tokio::spawn(async move {
            let path = ObjectPath::try_from(ADVERTISEMENT_PATH).expect("valid path");
            if let Ok(manager) = LEAdvertisingManager1Proxy::builder(&connection)
                .path(adapter)
                .expect("valid path")
                .build()
                .await
            {
                let _ = manager.unregister_advertisement(&path).await;
            }
            let _ = connection.object_server().remove::<Advertisement, _>(ADVERTISEMENT_PATH).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::{endpoint_info, mdns_instance_name};

    #[test]
    fn parses_an_android_service() {
        let fullname = format!("{}.{SERVICE_TYPE}", mdns_instance_name(*b"Ab12"));
        let info = URL_SAFE_NO_PAD.encode(endpoint_info(DeviceType::Phone, "S22 Ultra"));
        let (id, device_type, name) = parse_service(&fullname, Some(&info)).unwrap();
        assert_eq!(id, "Ab12");
        assert_eq!(device_type, DeviceType::Phone);
        assert_eq!(name.as_deref(), Some("S22 Ultra"));
        assert!(parse_service("printer._ipp._tcp.local.", Some(&info)).is_none());
    }
}
