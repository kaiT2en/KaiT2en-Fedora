// SPDX-License-Identifier: GPL-3.0-or-later
//! mDNS advertisement that makes this machine show up in Android's share
//! sheet while receiving is enabled.

use std::collections::HashMap;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

use crate::endpoint::{DeviceType, endpoint_info, mdns_instance_name};

pub const SERVICE_TYPE: &str = "_FC9F5ED42C8A._tcp.local.";

/// MAC of the USB network link to the T2 chip, the same on every T2 Mac.
const T2_LINK_MAC: &str = "ac:de:48:00:11:22";

/// A registered Quick Share service, unregistered on drop.
pub struct Advertisement {
    daemon: ServiceDaemon,
    info: ServiceInfo,
}

impl Advertisement {
    pub fn start(endpoint_id: [u8; 4], name: &str, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("mDNS daemon")?;
        // Phones only reach this machine through the WLAN or Ethernet, the
        // T2 link and loopback addresses would only slow them down.
        let mut excluded = vec![IfKind::LoopbackV4, IfKind::LoopbackV6];
        excluded.extend(t2_link_interfaces().into_iter().map(IfKind::Name));
        daemon.disable_interface(excluded).context("mDNS interfaces")?;
        let host = format!("{}.local.", hostname());
        let properties = HashMap::from([(
            "n".to_string(),
            URL_SAFE_NO_PAD.encode(endpoint_info(DeviceType::Laptop, name)),
        )]);
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &mdns_instance_name(endpoint_id),
            &host,
            "",
            port,
            properties,
        )
        .context("mDNS service info")?
        .enable_addr_auto();
        daemon.register(info.clone()).context("mDNS register")?;
        Ok(Self { daemon, info })
    }

    /// Announces the service again. Android does not always pick up a
    /// service that was announced before it started looking.
    pub fn announce(&self) -> Result<()> {
        self.daemon.register(self.info.clone()).context("mDNS register")?;
        Ok(())
    }
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        if let Ok(done) = self.daemon.unregister(self.info.get_fullname()) {
            let _ = done.recv_timeout(std::time::Duration::from_secs(1));
        }
        let _ = self.daemon.shutdown();
    }
}

fn t2_link_interfaces() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            std::fs::read_to_string(entry.path().join("address"))
                .is_ok_and(|mac| mac.trim().eq_ignore_ascii_case(T2_LINK_MAC))
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "t2airshare".to_string())
}
