// SPDX-License-Identifier: GPL-3.0-or-later
//! Quick Share over Bluetooth, for phones that are not in the same WLAN.
//! Android finds the receiver through a BLE advertisement header, reads the
//! full advertisement with the name, the Bluetooth address and the L2CAP
//! PSM from a GATT server, then connects over a BLE L2CAP channel (see
//! l2cap.rs) or Bluetooth Classic RFCOMM. Both carry the same frames as the
//! TCP connection in the WLAN.
//!
//! The formats follow google/nearby (commit 203b6e1), connections/
//! implementation/mediums/ble.cc and ble_v2 advertisement code.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use log::{debug, info, warn};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface, proxy};

use crate::endpoint::{DeviceType, endpoint_info};
use crate::l2cap;

/// A connection over RFCOMM or BLE L2CAP.
pub trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}
pub type Incoming = (String, Box<dyn Stream>);

const SERVICE_ID: &[u8] = b"NearbySharing";
/// UUID v3 (MD5) of the service id, the RFCOMM service of Nearby Sharing.
const RFCOMM_UUID: &str = "a82efa21-ae5c-3dde-9bbc-f16da7b16c5a";
/// BlueZ only listens on RFCOMM for a UUID it does not know when the
/// profile names a channel.
const RFCOMM_CHANNEL: u16 = 9;
const FEF3: &str = "0000fef3-0000-1000-8000-00805f9b34fb";
/// Characteristic of advertisement slot 0: msb 0x3000, lsb 0x8000... | slot.
const SLOT0_UUID: &str = "00000000-0000-3000-8000-000000000000";
const PROFILE_PATH: &str = "/org/t2airshare/bluetooth/profile";
const GATT_ROOT: &str = "/org/t2airshare/bluetooth/gatt";
const SERVICE_PATH: &str = "/org/t2airshare/bluetooth/gatt/service0";
const CHAR_PATH: &str = "/org/t2airshare/bluetooth/gatt/service0/char0";
const ADV_PATHS: [&str; 2] = [
    "/org/t2airshare/bluetooth/advertisement0",
    "/org/t2airshare/bluetooth/advertisement1",
];
/// Service data of the advertisement for legacy devices,
/// CreateAdvertisingDataForLegacyDevice in mediums/ble.cc with
/// DiscoveredPeripheralTracker::kDummyAdvertisementValue.
const DUMMY: &[u8] = b"QCAAABACAAADAAAAAAAAAAA";

#[proxy(interface = "org.bluez.ProfileManager1", default_service = "org.bluez", default_path = "/org/bluez")]
trait ProfileManager1 {
    fn register_profile(
        &self,
        profile: &ObjectPath<'_>,
        uuid: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.GattManager1", default_service = "org.bluez")]
trait GattManager1 {
    fn register_application(
        &self,
        application: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.LEAdvertisingManager1", default_service = "org.bluez")]
trait LeAdvertisingManager1 {
    fn register_advertisement(
        &self,
        advertisement: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
    fn unregister_advertisement(&self, advertisement: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[proxy(interface = "org.freedesktop.DBus.ObjectManager", default_service = "org.bluez", default_path = "/")]
trait BluezObjects {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

/// RFCOMM connections BlueZ hands over.
struct Profile {
    tx: mpsc::Sender<Incoming>,
}

#[interface(name = "org.bluez.Profile1")]
impl Profile {
    async fn new_connection(
        &self,
        device: OwnedObjectPath,
        fd: OwnedFd,
        _properties: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let fd = std::os::fd::OwnedFd::from(fd);
        let stream = std::os::unix::net::UnixStream::from(fd);
        let stream = stream
            .set_nonblocking(true)
            .and_then(|_| UnixStream::from_std(stream))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let _ = self.tx.send((device.to_string(), Box::new(stream))).await;
        Ok(())
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}

    fn release(&self) {}
}

struct GattService;

#[interface(name = "org.bluez.GattService1")]
impl GattService {
    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        FEF3.to_string()
    }

    #[zbus(property)]
    fn primary(&self) -> bool {
        true
    }
}

/// Serves the full advertisement of slot 0.
struct GattCharacteristic {
    value: Vec<u8>,
}

#[interface(name = "org.bluez.GattCharacteristic1")]
impl GattCharacteristic {
    fn read_value(&self, options: HashMap<String, OwnedValue>) -> Vec<u8> {
        let offset = options.get("offset").and_then(|v| u16::try_from(v).ok()).unwrap_or(0);
        self.value.get(offset as usize..).unwrap_or_default().to_vec()
    }

    #[zbus(property, name = "UUID")]
    fn uuid(&self) -> String {
        SLOT0_UUID.to_string()
    }

    #[zbus(property)]
    fn service(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str_unchecked(SERVICE_PATH).into()
    }

    #[zbus(property)]
    fn flags(&self) -> Vec<String> {
        vec!["read".to_string()]
    }
}

/// One BLE advertisement with service data FEF3.
struct Advertisement {
    data: Vec<u8>,
}

#[interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    fn release(&self) {}

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "peripheral".to_string()
    }

    #[zbus(property)]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        let value = OwnedValue::try_from(Value::from(self.data.clone())).expect("no fds");
        HashMap::from([(FEF3.to_string(), value)])
    }

    #[zbus(property)]
    fn discoverable(&self) -> bool {
        true
    }
}

/// Visibility over Bluetooth, kept up while the service runs. BlueZ drops
/// everything registered here when the process ends.
pub struct Visibility {
    connection: Connection,
    endpoint_id: [u8; 4],
    info: Vec<u8>,
    adapter: Option<OwnedObjectPath>,
    advertising: bool,
    /// the send window looks for AirDrop receivers, see `set_paused`
    paused: bool,
    psm: u16,
    /// the GATT advertisement, also answered over L2CAP
    advertisement: Arc<RwLock<Arc<Vec<u8>>>>,
    /// Discoverable and DiscoverableTimeout of the adapter before
    /// `refresh` changed them, put back by `stop`
    previous: Option<(OwnedObjectPath, bool, u32)>,
}

impl Visibility {
    /// Opens the L2CAP server and registers the RFCOMM profile. Accepted
    /// connections arrive on the returned channel with the BlueZ device
    /// path or "L2CAP". Advertising starts with `refresh` once an adapter
    /// is powered.
    pub async fn start(endpoint_id: [u8; 4], name: &str) -> Result<(Self, mpsc::Receiver<Incoming>)> {
        let connection = Connection::system().await.context("system bus")?;
        let (tx, rx) = mpsc::channel(4);
        let advertisement: Arc<RwLock<Arc<Vec<u8>>>> = Arc::default();
        let server = l2cap::Server::open().context("open the L2CAP server")?;
        let psm = server.psm;
        info!("L2CAP PSM {psm:#x}");
        tokio::spawn(accept_l2cap(server, tx.clone(), advertisement.clone()));
        connection.object_server().at(PROFILE_PATH, Profile { tx }).await?;
        let options = HashMap::from([
            ("Name", Value::from("NearbySharing")),
            ("Role", Value::from("server")),
            ("Channel", Value::from(RFCOMM_CHANNEL)),
            ("RequireAuthentication", Value::from(false)),
            ("RequireAuthorization", Value::from(false)),
        ]);
        ProfileManager1Proxy::new(&connection)
            .await?
            .register_profile(&ObjectPath::try_from(PROFILE_PATH)?, RFCOMM_UUID, options)
            .await
            .context("register the RFCOMM profile")?;
        let visibility = Self {
            connection,
            endpoint_id,
            info: endpoint_info(DeviceType::Laptop, name),
            adapter: None,
            advertising: false,
            paused: false,
            psm,
            advertisement,
            previous: None,
        };
        Ok((visibility, rx))
    }

    /// Starts advertising once the adapter is powered, again after it was
    /// switched off and on.
    pub async fn refresh(&mut self) -> Result<()> {
        let objects = BluezObjectsProxy::new(&self.connection).await?.get_managed_objects().await?;
        let adapter = objects.into_iter().find_map(|(path, interfaces)| {
            let adapter = interfaces.get("org.bluez.Adapter1")?;
            let powered = adapter.get("Powered").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
            let address = adapter.get("Address").and_then(|v| String::try_from(v.clone()).ok())?;
            Some((path, powered, address))
        });
        let Some((path, _, address)) = adapter.filter(|(_, powered, _)| *powered) else {
            self.advertising = false;
            return Ok(());
        };
        if self.advertising || self.paused {
            return Ok(());
        }
        if self.adapter.as_ref() != Some(&path) {
            self.serve_gatt(&path, &address).await?;
        }
        // the phone pages this machine over Bluetooth Classic for RFCOMM
        let props = zbus::fdo::PropertiesProxy::builder(&self.connection)
            .destination("org.bluez")?
            .path(path.clone())?
            .build()
            .await?;
        let iface = zbus::names::InterfaceName::try_from("org.bluez.Adapter1")?;
        // BlueZ refuses to set them again while the adapter is discoverable
        if !bool::try_from(props.get(iface.clone(), "Connectable").await?)? {
            props.set(iface.clone(), "Connectable", Value::from(true)).await.context("make the adapter connectable")?;
        }
        // BlueZ marks the advertisements "BR/EDR not supported" while the
        // adapter is not discoverable. Android then takes this machine for
        // an LE only device and does not page it for RFCOMM.
        if self.previous.is_none() {
            let discoverable = bool::try_from(props.get(iface.clone(), "Discoverable").await?)?;
            let timeout = u32::try_from(props.get(iface.clone(), "DiscoverableTimeout").await?)?;
            self.previous = Some((path.clone(), discoverable, timeout));
        }
        if !bool::try_from(props.get(iface.clone(), "Discoverable").await?)? {
            props.set(iface.clone(), "DiscoverableTimeout", Value::from(0u32)).await?;
            props.set(iface, "Discoverable", Value::from(true)).await.context("make the adapter discoverable")?;
        }
        let manager = LeAdvertisingManager1Proxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;
        for adv_path in ADV_PATHS {
            let adv_path = ObjectPath::try_from(adv_path)?;
            // after a power cycle BlueZ may still hold the old registration
            let _ = manager.unregister_advertisement(&adv_path).await;
            if let Err(err) = manager.register_advertisement(&adv_path, HashMap::new()).await {
                warn!("BLE advertisement {adv_path}: {err}");
            }
        }
        self.advertising = true;
        info!("visible over Bluetooth on {path} ({address})");
        Ok(())
    }

    /// Serves the full advertisement on `adapter` and sets up the BLE
    /// advertisements: the legacy dummy and the header that leads to the
    /// GATT server. No fast advertisement, its endpoint info is cut to 17
    /// bytes without the name, and Android replaces the named entry with it.
    async fn serve_gatt(&mut self, adapter: &OwnedObjectPath, address: &str) -> Result<()> {
        let mac = parse_mac(address).context("adapter address")?;
        let value = gatt_advertisement(self.endpoint_id, &self.info, mac, self.psm);
        *self.advertisement.write().expect("not poisoned") = Arc::new(value.clone());
        let advertisements = [
            Advertisement { data: DUMMY.to_vec() },
            Advertisement { data: advertisement_header(&value, self.psm) },
        ];
        let server = self.connection.object_server();
        for (adv_path, advertisement) in ADV_PATHS.into_iter().zip(advertisements) {
            let _ = server.remove::<Advertisement, _>(adv_path).await;
            server.at(adv_path, advertisement).await?;
        }
        let _ = server.remove::<GattCharacteristic, _>(CHAR_PATH).await;
        let _ = server.remove::<GattService, _>(SERVICE_PATH).await;
        server.at(GATT_ROOT, zbus::fdo::ObjectManager).await?;
        server.at(SERVICE_PATH, GattService).await?;
        server.at(CHAR_PATH, GattCharacteristic { value }).await?;
        GattManager1Proxy::builder(&self.connection)
            .path(adapter.clone())?
            .build()
            .await?
            .register_application(&ObjectPath::try_from(GATT_ROOT)?, HashMap::new())
            .await
            .context("register the GATT server")?;
        self.adapter = Some(adapter.clone());
        Ok(())
    }
}

impl Visibility {
    /// Puts the discoverability of the adapter back as it was.
    pub async fn stop(&mut self) {
        let Some((path, discoverable, timeout)) = self.previous.take() else { return };
        let result: Result<()> = async {
            let props = zbus::fdo::PropertiesProxy::builder(&self.connection)
                .destination("org.bluez")?
                .path(path)?
                .build()
                .await?;
            let iface = zbus::names::InterfaceName::try_from("org.bluez.Adapter1")?;
            props.set(iface.clone(), "Discoverable", Value::from(discoverable)).await?;
            props.set(iface, "DiscoverableTimeout", Value::from(timeout)).await?;
            Ok(())
        }
        .await;
        if let Err(err) = result {
            warn!("Bluetooth: restore discoverable: {err:#}");
        }
    }
}

impl Visibility {
    /// Takes the BLE advertisements off the air while the send window looks
    /// for AirDrop receivers. The controller sends its advertisements in
    /// turns, next to these two the AirDrop advertisement of t2-drop rarely
    /// reaches the Macs nearby. GATT, RFCOMM and L2CAP stay up.
    pub async fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if !paused {
            info!("Bluetooth advertising resumed");
            refresh(self).await;
            return;
        }
        info!("Bluetooth advertising paused for an AirDrop search");
        let Some(adapter) = self.adapter.clone() else { return };
        self.advertising = false;
        let result: Result<()> = async {
            let manager = LeAdvertisingManager1Proxy::builder(&self.connection)
                .path(adapter)?
                .build()
                .await?;
            for adv_path in ADV_PATHS {
                let _ = manager.unregister_advertisement(&ObjectPath::try_from(adv_path)?).await;
            }
            Ok(())
        }
        .await;
        if let Err(err) = result {
            warn!("Bluetooth: pause advertising: {err:#}");
        }
    }
}

/// Session bus name of the receiver service, the send window asks it to
/// pause its BLE advertisements.
pub const RECEIVER_BUS_NAME: &str = "org.t2airshare.Receiver";
const RECEIVER_PATH: &str = "/org/t2airshare/Receiver";

/// Holders of a pause, by unique bus name. A holder that leaves the bus
/// releases its pause.
struct Receiver {
    holders: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    tx: mpsc::Sender<bool>,
}

#[interface(name = "org.t2airshare.Receiver1")]
impl Receiver {
    /// Pauses the BLE advertisements until the caller calls
    /// `ResumeAdvertising` or leaves the bus.
    async fn pause_advertising(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
        let Some(sender) = header.sender().map(|s| s.to_string()) else { return };
        let first = {
            let mut holders = self.holders.lock().expect("not poisoned");
            holders.insert(sender) && holders.len() == 1
        };
        if first {
            let _ = self.tx.send(true).await;
        }
    }

    async fn resume_advertising(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
        let Some(sender) = header.sender().map(|s| s.to_string()) else { return };
        release(&self.holders, &sender, &self.tx).await;
    }
}

async fn release(
    holders: &std::sync::Mutex<std::collections::HashSet<String>>,
    name: &str,
    tx: &mpsc::Sender<bool>,
) {
    let last = {
        let mut holders = holders.lock().expect("not poisoned");
        holders.remove(name) && holders.is_empty()
    };
    if last {
        let _ = tx.send(false).await;
    }
}

/// Serves org.t2airshare.Receiver on the session bus. The returned channel
/// says when to pause (true) and resume (false) the advertisements.
pub async fn serve_receiver() -> Result<(Connection, mpsc::Receiver<bool>)> {
    let (tx, rx) = mpsc::channel(8);
    let holders: Arc<std::sync::Mutex<std::collections::HashSet<String>>> = Arc::default();
    let receiver = Receiver { holders: holders.clone(), tx: tx.clone() };
    let connection = zbus::connection::Builder::session()?
        .serve_at(RECEIVER_PATH, receiver)?
        .name(RECEIVER_BUS_NAME)?
        .build()
        .await
        .context("session bus")?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    let mut owners = dbus.receive_name_owner_changed().await?;
    tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(signal) = owners.next().await {
            let Ok(args) = signal.args() else { continue };
            if args.new_owner().is_none() {
                release(&holders, args.name().as_str(), &tx).await;
            }
        }
    });
    Ok((connection, rx))
}

#[proxy(
    interface = "org.t2airshare.Receiver1",
    default_service = "org.t2airshare.Receiver",
    default_path = "/org/t2airshare/Receiver"
)]
trait Receiver1 {
    fn pause_advertising(&self) -> zbus::Result<()>;
}

/// Pauses the BLE advertisements of the receiver service as long as the
/// returned connection lives. None when the service does not run.
pub async fn pause_receiver_advertising() -> Option<Connection> {
    let connection = Connection::session().await.ok()?;
    Receiver1Proxy::new(&connection).await.ok()?.pause_advertising().await.ok()?;
    Some(connection)
}

/// Hands every L2CAP data connection to the receiver.
async fn accept_l2cap(
    server: l2cap::Server,
    tx: mpsc::Sender<Incoming>,
    advertisement: Arc<RwLock<Arc<Vec<u8>>>>,
) {
    loop {
        let channel = match server.accept().await {
            Ok(channel) => channel,
            Err(err) => {
                warn!("L2CAP accept: {err}");
                return;
            }
        };
        let tx = tx.clone();
        let advertisement = advertisement.read().expect("not poisoned").clone();
        tokio::spawn(async move {
            match l2cap::serve(channel, advertisement).await {
                Ok(Some(stream)) => {
                    let _ = tx.send(("L2CAP".to_string(), Box::new(stream))).await;
                }
                Ok(None) => debug!("L2CAP advertisement request served"),
                Err(err) => warn!("L2CAP: {err:#}"),
            }
        });
    }
}

/// Keeps `visibility` up, logs instead of failing.
pub async fn refresh(visibility: &mut Visibility) {
    if let Err(err) = visibility.refresh().await {
        warn!("Bluetooth: {err:#}");
    }
}

fn parse_mac(address: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = address.split(':');
    for byte in &mut mac {
        *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

/// The regular medium advertisement (V2, socket V2, not fast) around the
/// connections advertisement (V1, PCP point to point, endpoint info with
/// the name, Bluetooth address, no UWB, extra field 0), as
/// Ble::StartAdvertising serves it in slot 0, with the PSM in the extra
/// fields (BleAdvertisement::BleExtraFields, mask bit 1).
fn gatt_advertisement(endpoint_id: [u8; 4], info: &[u8], mac: [u8; 6], psm: u16) -> Vec<u8> {
    let service_hash = &Sha256::digest(SERVICE_ID)[..3];
    let mut inner = vec![1 << 5 | 3];
    inner.extend_from_slice(service_hash);
    inner.extend_from_slice(&endpoint_id);
    inner.push(info.len() as u8);
    inner.extend_from_slice(info);
    inner.extend_from_slice(&mac);
    inner.extend_from_slice(&[0, 0]);

    let mut outer = vec![2 << 5 | 2 << 2];
    outer.extend_from_slice(service_hash);
    outer.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    outer.extend_from_slice(&inner);
    outer.extend_from_slice(&rand::random::<[u8; 2]>());
    if psm != 0 {
        outer.push(0x01);
        outer.extend_from_slice(&psm.to_be_bytes());
    }
    outer
}

/// BleAdvertisementHeader as Ble::CreateAdvertisementHeader builds it: V2,
/// no extended advertisement, one slot, bloom filter with a random dummy id
/// and the service id, hash chained over the GATT advertisement, the L2CAP
/// PSM.
fn advertisement_header(gatt_advertisement: &[u8], psm: u16) -> Vec<u8> {
    let dummy: [u8; 16] = rand::random();
    let mut bloom = [0u8; 10];
    bloom_add(&mut bloom, &dummy);
    bloom_add(&mut bloom, SERVICE_ID);
    let first = &Sha256::digest(dummy)[..4];
    let hash = &Sha256::digest([first, gatt_advertisement].concat())[..4];

    let mut header = vec![2 << 5 | 1];
    header.extend_from_slice(&bloom);
    header.extend_from_slice(hash);
    header.extend_from_slice(&psm.to_be_bytes());
    header
}

/// mediums::BloomFilter::Add with 80 bits.
fn bloom_add(bits: &mut [u8; 10], item: &[u8]) {
    let hash = murmur3_x64_128_low(item);
    let hash1 = hash as u32 as i32;
    let hash2 = (hash >> 32) as u32 as i32;
    for i in 1..=5i32 {
        let mut combined = hash1.wrapping_add(i.wrapping_mul(hash2));
        if combined < 0 {
            combined = !combined;
        }
        let pos = (combined % 80) as usize;
        bits[pos / 8] |= 1 << (pos % 8);
    }
}

/// The low 64 bits of MurmurHash3 x64_128 with seed 0.
fn murmur3_x64_128_low(data: &[u8]) -> u64 {
    const C1: u64 = 0x87c3_7b91_1142_53d5;
    const C2: u64 = 0x4cf5_ad43_2745_937f;
    fn fmix(mut k: u64) -> u64 {
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^ (k >> 33)
    }
    let le = |b: &[u8]| {
        let mut word = [0u8; 8];
        word[..b.len()].copy_from_slice(b);
        u64::from_le_bytes(word)
    };

    let (mut h1, mut h2) = (0u64, 0u64);
    let blocks = data.chunks_exact(16);
    let tail = blocks.remainder();
    for block in blocks {
        let k1 = le(&block[..8]).wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        h1 ^= k1;
        h1 = h1.rotate_left(27).wrapping_add(h2).wrapping_mul(5).wrapping_add(0x52dc_e729);
        let k2 = le(&block[8..]).wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        h2 ^= k2;
        h2 = h2.rotate_left(31).wrapping_add(h1).wrapping_mul(5).wrapping_add(0x3849_5ab5);
    }
    if tail.len() > 8 {
        h2 ^= le(&tail[8..]).wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
    }
    if !tail.is_empty() {
        h1 ^= le(&tail[..tail.len().min(8)]).wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
    }
    h1 ^= data.len() as u64;
    h2 ^= data.len() as u64;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix(h1);
    h2 = fmix(h2);
    h1.wrapping_add(h2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur3_matches_reference() {
        assert_eq!(murmur3_x64_128_low(b"hello"), 0xcbd8_a7b3_41bd_9b02);
    }

    #[test]
    fn header_layout() {
        let header = advertisement_header(b"advertisement", 0x81);
        assert_eq!(header.len(), 17);
        assert_eq!(header[0], 0x41);
        assert_eq!(&header[15..], &[0, 0x81]);
    }

    #[test]
    fn gatt_advertisement_layout() {
        let info = endpoint_info(DeviceType::Laptop, "kait2en");
        let adv = gatt_advertisement(*b"AbCd", &info, [1, 2, 3, 4, 5, 6], 0x81);
        assert_eq!(&adv[..4], &[0x48, 0xfc, 0x9f, 0x5e]);
        let len = u32::from_be_bytes(adv[4..8].try_into().unwrap()) as usize;
        assert_eq!(adv.len(), 8 + len + 2 + 3);
        assert_eq!(&adv[adv.len() - 3..], &[0x01, 0x00, 0x81]);
        assert_eq!(&adv[8..16], &[0x23, 0xfc, 0x9f, 0x5e, b'A', b'b', b'C', b'd']);
        assert_eq!(&adv[8 + len - 8..8 + len], &[1, 2, 3, 4, 5, 6, 0, 0]);
    }

    #[test]
    fn mac_from_bluez() {
        assert_eq!(parse_mac("A4:83:E7:CF:B8:07"), Some([0xa4, 0x83, 0xe7, 0xcf, 0xb8, 0x07]));
        assert_eq!(parse_mac("A4:83"), None);
    }
}
