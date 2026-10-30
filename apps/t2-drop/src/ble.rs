// SPDX-License-Identifier: GPL-3.0-or-later
//! The Bluetooth LE advertisement that makes Apple devices start their
//! AirDrop server: Apple manufacturer data of type 0x05 (AirDrop) with
//! version 1 and empty contact hashes, which receivers in "Everyone" mode
//! accept. The scanner looks for the same advertisement of senders, as
//! sharingd does before it starts its own server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use log::{debug, warn};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream, interface, proxy};

use crate::mgmt::{self, ScanTiming};

const ADVERTISEMENT_PATH: &str = "/org/t2drop/advertisement";
const APPLE: u16 = 0x004c;
/// Type 0x05, length 0x12, 8 zero bytes, version 1, four 16 bit contact
/// hashes, one zero byte
const AIRDROP: [u8; 20] = [0x05, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[proxy(interface = "org.bluez.LEAdvertisingManager1", default_service = "org.bluez")]
trait LEAdvertisingManager1 {
    fn register_advertisement(
        &self,
        advertisement: &ObjectPath<'_>,
        options: HashMap<&str, &Value<'_>>,
    ) -> zbus::Result<()>;

    fn unregister_advertisement(&self, advertisement: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
trait Adapter1 {
    fn set_discovery_filter(&self, filter: HashMap<&str, &Value<'_>>) -> zbus::Result<()>;
    fn start_discovery(&self) -> zbus::Result<()>;
    fn stop_discovery(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
}

#[proxy(interface = "org.freedesktop.DBus.ObjectManager", default_service = "org.bluez", default_path = "/")]
trait BluezObjects {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

struct Advertisement;

#[interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    fn release(&self) {}

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "broadcast".to_string()
    }

    #[zbus(property)]
    fn manufacturer_data(&self) -> HashMap<u16, OwnedValue> {
        let value = OwnedValue::try_from(Value::from(AIRDROP.to_vec())).expect("byte array converts");
        HashMap::from([(APPLE, value)])
    }
}

/// Bluetooth is powered off, so no advertisement can go out.
#[derive(Debug)]
pub struct BluetoothOff;

impl std::fmt::Display for BluetoothOff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bluetooth is off, Apple devices only start AirDrop after a Bluetooth LE request")
    }
}

impl std::error::Error for BluetoothOff {}

/// The adapter with LE advertising and whether it is powered.
async fn le_adapter(connection: &Connection) -> Result<OwnedObjectPath> {
    let objects = BluezObjectsProxy::new(connection).await?.get_managed_objects().await?;
    let (adapter, interfaces) = objects
        .into_iter()
        .find(|(_, i)| i.contains_key("org.bluez.LEAdvertisingManager1"))
        .context("no Bluetooth adapter with LE advertising")?;
    let powered = interfaces
        .get("org.bluez.Adapter1")
        .and_then(|a| a.get("Powered"))
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    if !powered {
        return Err(BluetoothOff.into());
    }
    Ok(adapter)
}

/// Whether Apple manufacturer data holds an AirDrop (0x05) element.
fn is_airdrop(data: &[u8]) -> bool {
    let mut rest = data;
    while rest.len() >= 2 {
        let len = rest[1] as usize;
        if rest[0] == AIRDROP[0] {
            return true;
        }
        rest = rest.get(2 + len..).unwrap_or_default();
    }
    false
}

/// The Apple data in a ManufacturerData property value.
fn apple_data(value: &Value<'_>) -> Option<Vec<u8>> {
    let Value::Dict(dict) = value else { return None };
    let data: HashMap<u16, OwnedValue> = dict.try_clone().ok()?.try_into().ok()?;
    Vec::<u8>::try_from(data.get(&APPLE)?.try_clone().ok()?).ok()
}

/// How long a paired sender counts as near after its last signal. BlueZ
/// removes devices that are not paired once they stop advertising
/// (TemporaryTimeout, 30 s by default), paired ones stay.
const PAIRED_LOST: Duration = Duration::from_secs(30);

/// LE scan timing while looking for senders: 30 ms out of every 300 ms.
/// A Mac advertises every 30 ms while its AirDrop window is open.
const SENDER_SCAN: ScanTiming = ScanTiming { interval: 480, window: 48 };
/// The kernel's discovery timing, 11.25 ms out of 11.25 ms
const KERNEL_SCAN: ScanTiming = ScanTiming { interval: 0x12, window: 0x12 };

/// The adapter index of a BlueZ adapter path like /org/bluez/hci0.
fn adapter_index(path: &OwnedObjectPath) -> Option<u16> {
    path.as_str().rsplit_once("/hci")?.1.parse().ok()
}

/// Lowers the discovery scan timing of the adapter, returns the timing to
/// set back. A timing equal to the own one is left over from a run that
/// did not end cleanly, the kernel's then.
fn lower_scan(index: u16) -> Result<ScanTiming> {
    let before = mgmt::discovery_scan(index)?;
    mgmt::set_discovery_scan(index, SENDER_SCAN)?;
    Ok(if before == SENDER_SCAN { KERNEL_SCAN } else { before })
}

/// A BlueZ device that carries the AirDrop element.
struct Sender {
    paired: bool,
    last: Instant,
}

type Senders = Arc<Mutex<HashMap<OwnedObjectPath, Sender>>>;

fn bool_prop(props: &HashMap<String, OwnedValue>, name: &str) -> Option<bool> {
    props.get(name).and_then(|v| bool::try_from(v).ok())
}

/// Updates the senders from the Device1 properties `props` of `path`,
/// `added` for a new device or a complete property set.
fn update_sender(senders: &Senders, path: OwnedObjectPath, props: &HashMap<String, OwnedValue>, added: bool) {
    let mut senders = senders.lock().unwrap();
    let airdrop = props.get("ManufacturerData").map(|v| apple_data(v).is_some_and(|d| is_airdrop(&d)));
    let paired = bool_prop(props, "Paired").unwrap_or(false) || bool_prop(props, "Bonded").unwrap_or(false);
    match (airdrop, senders.get_mut(&path)) {
        (Some(false), _) => {
            senders.remove(&path);
        }
        (Some(true), Some(sender)) => sender.last = Instant::now(),
        (Some(true), None) => {
            senders.insert(path, Sender { paired: added && paired, last: Instant::now() });
        }
        (None, Some(sender)) => {
            sender.last = Instant::now();
            if props.contains_key("Paired") || props.contains_key("Bonded") {
                sender.paired = paired;
            }
        }
        (None, None) => {}
    }
}

/// Bluetooth LE discovery that tracks the AirDrop senders nearby. BlueZ
/// reports a new device with its advertisement data once and changes only
/// when the data changes, so a device that carries the AirDrop element
/// counts as near until BlueZ removes it. BlueZ ends the discovery when the
/// connection closes.
pub struct BeaconScanner {
    connection: Connection,
    adapter: OwnedObjectPath,
    /// Adapter index and the scan timing to set back
    restore: Option<(u16, ScanTiming)>,
    senders: Senders,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl BeaconScanner {
    pub async fn start() -> Result<Self> {
        let connection = Connection::system().await.context("system bus")?;
        let adapter_path = le_adapter(&connection).await?;
        let signals = |interface: &'static str, member: &'static str| -> Result<MatchRule<'static>> {
            Ok(MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.bluez")?
                .interface(interface)?
                .member(member)?
                .build())
        };
        let mut changes = MessageStream::for_match_rule(
            signals("org.freedesktop.DBus.Properties", "PropertiesChanged")?,
            &connection,
            None,
        )
        .await?;
        let mut added =
            MessageStream::for_match_rule(signals("org.freedesktop.DBus.ObjectManager", "InterfacesAdded")?, &connection, None)
                .await?;
        let mut removed = MessageStream::for_match_rule(
            signals("org.freedesktop.DBus.ObjectManager", "InterfacesRemoved")?,
            &connection,
            None,
        )
        .await?;

        // Devices BlueZ knows already, the ones not paired were seen within
        // its TemporaryTimeout
        let senders: Senders = Arc::default();
        for (path, interfaces) in BluezObjectsProxy::new(&connection).await?.get_managed_objects().await? {
            if let Some(props) = interfaces.get("org.bluez.Device1") {
                update_sender(&senders, path, props, true);
            }
        }

        let restore = match adapter_index(&adapter_path) {
            Some(index) => match tokio::task::spawn_blocking(move || lower_scan(index)).await? {
                Ok(before) => Some((index, before)),
                Err(err) => {
                    warn!("lower the Bluetooth scan timing: {err:#}");
                    None
                }
            },
            None => None,
        };
        let adapter = Adapter1Proxy::builder(&connection).path(adapter_path.clone())?.build().await?;
        let le = Value::from("le");
        let duplicates = Value::from(true);
        adapter
            .set_discovery_filter(HashMap::from([("Transport", &le), ("DuplicateData", &duplicates)]))
            .await
            .context("Bluetooth discovery filter")?;
        adapter.start_discovery().await.context("start Bluetooth discovery")?;

        let s = senders.clone();
        let changes = tokio::spawn(async move {
            while let Some(Ok(msg)) = changes.next().await {
                let Some(path) = msg.header().path().map(|p| OwnedObjectPath::from(p.to_owned())) else { continue };
                if let Ok((interface, props, _)) =
                    msg.body().deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                {
                    if interface == "org.bluez.Device1" {
                        update_sender(&s, path, &props, false);
                    }
                }
            }
        });
        let s = senders.clone();
        let added = tokio::spawn(async move {
            while let Some(Ok(msg)) = added.next().await {
                if let Ok((path, interfaces)) = msg
                    .body()
                    .deserialize::<(OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>)>()
                {
                    if let Some(props) = interfaces.get("org.bluez.Device1") {
                        update_sender(&s, path, props, true);
                    }
                }
            }
        });
        let s = senders.clone();
        let removed = tokio::spawn(async move {
            while let Some(Ok(msg)) = removed.next().await {
                if let Ok((path, interfaces)) = msg.body().deserialize::<(OwnedObjectPath, Vec<String>)>() {
                    if interfaces.iter().any(|i| i == "org.bluez.Device1") {
                        s.lock().unwrap().remove(&path);
                    }
                }
            }
        });
        debug!("scanning for AirDrop senders on {}", adapter_path.as_str());
        Ok(Self { connection, adapter: adapter_path, restore, senders, tasks: vec![changes, added, removed] })
    }

    /// Whether an AirDrop sender is nearby.
    pub fn sender_near(&self) -> bool {
        let senders = self.senders.lock().unwrap();
        senders.values().any(|s| !s.paired || s.last.elapsed() < PAIRED_LOST)
    }

    /// Whether the adapter still discovers, it stops when Bluetooth goes
    /// off or the adapter goes away.
    pub async fn alive(&self) -> bool {
        let Ok(builder) = Adapter1Proxy::builder(&self.connection).path(self.adapter.clone()) else { return false };
        let Ok(adapter) = builder.cache_properties(zbus::proxy::CacheProperties::No).build().await else {
            return false;
        };
        adapter.discovering().await.unwrap_or(false) && !self.tasks.iter().any(|t| t.is_finished())
    }

    pub async fn stop(self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Ok(builder) = Adapter1Proxy::builder(&self.connection).path(self.adapter) {
            if let Ok(adapter) = builder.build().await {
                let _ = adapter.stop_discovery().await;
            }
        }
        if let Some((index, timing)) = self.restore {
            let r = tokio::task::spawn_blocking(move || mgmt::set_discovery_scan(index, timing)).await;
            if let Ok(Err(err)) = r {
                warn!("set the Bluetooth scan timing back: {err:#}");
            }
        }
    }
}

/// A registered AirDrop advertisement, removed by `stop`.
pub struct AirDropBeacon {
    connection: Connection,
    adapter: OwnedObjectPath,
}

impl AirDropBeacon {
    pub async fn start() -> Result<Self> {
        let connection = Connection::system().await.context("system bus")?;
        let adapter = le_adapter(&connection).await?;
        connection.object_server().at(ADVERTISEMENT_PATH, Advertisement).await?;
        LEAdvertisingManager1Proxy::builder(&connection)
            .path(adapter.clone())?
            .build()
            .await?
            .register_advertisement(&ObjectPath::try_from(ADVERTISEMENT_PATH)?, HashMap::new())
            .await
            .context("register AirDrop advertisement")?;
        debug!("AirDrop advertisement on {}", adapter.as_str());
        Ok(Self { connection, adapter })
    }

    pub async fn stop(self) {
        let path = ObjectPath::try_from(ADVERTISEMENT_PATH).expect("valid path");
        if let Ok(manager) = LEAdvertisingManager1Proxy::builder(&self.connection)
            .path(self.adapter)
            .expect("valid path")
            .build()
            .await
        {
            let _ = manager.unregister_advertisement(&path).await;
        }
        let _ = self.connection.object_server().remove::<Advertisement, _>(ADVERTISEMENT_PATH).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_airdrop_among_apple_elements() {
        // Nearby info (0x10) first, then the AirDrop element macOS sends
        let mut data = vec![0x10, 0x02, 0x0b, 0x1c];
        data.extend([0x05, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 1, 2, 3, 4, 5, 6, 7, 8, 0]);
        assert!(is_airdrop(&data));
        assert!(!is_airdrop(&[0x10, 0x02, 0x0b, 0x1c]));
        assert!(!is_airdrop(&[0x0c, 0x0e]));
    }

    #[test]
    fn reads_apple_data_as_bluez_sends_it() {
        // ManufacturerData is a{qv} with the data as ay in the variant
        let inner: HashMap<u16, Value<'_>> = HashMap::from([(APPLE, Value::from(vec![0x05u8, 0x12]))]);
        let value = OwnedValue::try_from(Value::from(inner)).unwrap();
        let wire = zbus::zvariant::to_bytes(zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::LE, 0), &value).unwrap();
        let back: OwnedValue = wire.deserialize().unwrap().0;
        assert_eq!(apple_data(&back), Some(vec![0x05, 0x12]));
    }
}
