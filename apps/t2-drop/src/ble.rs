// SPDX-License-Identifier: GPL-3.0-or-later
//! The Bluetooth LE advertisement that makes Apple devices start their
//! AirDrop server: Apple manufacturer data of type 0x05 (AirDrop) with
//! version 1 and empty contact hashes, which receivers in "Everyone" mode
//! accept.

use std::collections::HashMap;

use anyhow::{Context, Result};
use log::debug;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface, proxy};

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

/// A registered AirDrop advertisement, removed by `stop`.
pub struct AirDropBeacon {
    connection: Connection,
    adapter: OwnedObjectPath,
}

impl AirDropBeacon {
    pub async fn start() -> Result<Self> {
        let connection = Connection::system().await.context("system bus")?;
        let objects = BluezObjectsProxy::new(&connection).await?.get_managed_objects().await?;
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
