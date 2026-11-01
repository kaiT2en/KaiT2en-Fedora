// SPDX-License-Identifier: GPL-3.0-or-later
//! Root helper of T2 AirShare for Quick Share, org.t2airshare.QuickShareHotspot
//! on the system bus.
//! A phone that sends over Bluetooth moves the transfer to a Wi-Fi hotspot
//! of the receiver. The helper adds an AP interface next to the normal
//! Wi-Fi interface, so the Wi-Fi connection stays up, and runs the hotspot
//! through NetworkManager on it. The hotspot lives until the caller stops it
//! or leaves the bus. D-Bus starts the helper, it exits after a minute
//! without a hotspot.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use log::{info, warn};
use rand::Rng;
use rand::distributions::Alphanumeric;
use tokio::process::Command;
use tokio::sync::Mutex;
use zbus::message::Header;
use zbus::{fdo, interface};

const BUS_NAME: &str = "org.t2airshare.QuickShareHotspot";
const PATH: &str = "/org/t2airshare/QuickShareHotspot";
const IFACE: &str = "t2qs0";
const CONNECTION: &str = "t2airshare-hotspot";
/// Address of the receiver in the hotspot, NetworkManager hands out the
/// rest of the /24 over DHCP.
const ADDRESS: &str = "10.42.97.1";
const IDLE_EXIT: Duration = Duration::from_secs(60);

struct Active {
    owner: String,
    port: u16,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    idle_since: Option<Instant>,
}

#[derive(Clone)]
struct Hotspot {
    state: Arc<Mutex<State>>,
}

#[interface(name = "org.t2airshare.QuickShareHotspot1")]
impl Hotspot {
    /// Starts the hotspot and opens TCP `port` in it. Returns SSID,
    /// password, the receiver's address and the frequency in MHz.
    async fn start(&self, port: u16, #[zbus(header)] header: Header<'_>) -> fdo::Result<(String, String, String, i32)> {
        let caller = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let mut state = self.state.lock().await;
        if let Some(active) = &state.active {
            if active.owner != caller {
                return Err(fdo::Error::Failed("the hotspot is in use".into()));
            }
            teardown(Some(active.port)).await;
            state.active = None;
        }
        match setup(port).await {
            Ok(result) => {
                info!("hotspot {} on {} MHz for {caller}, port {port}", result.0, result.3);
                state.active = Some(Active { owner: caller, port });
                state.idle_since = None;
                Ok(result)
            }
            Err(err) => {
                warn!("hotspot: {err:#}");
                teardown(Some(port)).await;
                state.idle_since = Some(Instant::now());
                Err(fdo::Error::Failed(format!("{err:#}")))
            }
        }
    }

    async fn stop(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let caller = header.sender().map(|s| s.to_string()).unwrap_or_default();
        self.stop_for(&caller).await;
        Ok(())
    }
}

impl Hotspot {
    async fn stop_for(&self, owner: &str) {
        let mut state = self.state.lock().await;
        if state.active.as_ref().is_some_and(|a| a.owner == owner) {
            let active = state.active.take().expect("checked");
            teardown(Some(active.port)).await;
            state.idle_since = Some(Instant::now());
            info!("hotspot stopped");
        }
    }
}

async fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The Wi-Fi interface NetworkManager uses and its phy.
fn wifi_interface() -> Result<(String, String)> {
    for entry in std::fs::read_dir("/sys/class/net")?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == IFACE || name.starts_with("awdl") {
            continue;
        }
        let Ok(phy) = std::fs::read_link(entry.path().join("phy80211")) else { continue };
        let uevent = std::fs::read_to_string(entry.path().join("uevent")).unwrap_or_default();
        if !uevent.lines().any(|l| l == "DEVTYPE=wlan") {
            continue;
        }
        let phy = phy.file_name().map(|p| p.to_string_lossy().into_owned()).context("phy name")?;
        return Ok((name, phy));
    }
    bail!("no Wi-Fi interface")
}

/// Frequency of the current Wi-Fi connection, the hotspot shares its
/// channel.
async fn connected_frequency(wifi: &str) -> Option<u32> {
    let link = run("iw", &["dev", wifi, "link"]).await.ok()?;
    let freq = link.lines().find_map(|l| l.trim().strip_prefix("freq:"))?;
    freq.trim().parse::<f32>().ok().map(|f| f as u32)
}

/// Band and channel for NetworkManager. Without a Wi-Fi connection the
/// regulatory domain may forbid sending on 5 GHz, channel 6 is allowed
/// everywhere.
fn band_channel(freq: Option<u32>) -> (&'static str, u32, u32) {
    match freq {
        Some(f @ 5000..=5900) => ("a", (f - 5000) / 5, f),
        Some(f @ 2412..=2472) => ("bg", (f - 2407) / 5, f),
        _ => ("bg", 6, 2437),
    }
}

fn random_text(len: usize) -> String {
    rand::thread_rng().sample_iter(&Alphanumeric).take(len).map(char::from).collect()
}

async fn setup(port: u16) -> Result<(String, String, String, i32)> {
    let (wifi, phy) = wifi_interface()?;
    let (band, channel, freq) = band_channel(connected_frequency(&wifi).await);
    run("iw", &["phy", &phy, "interface", "add", IFACE, "type", "__ap"]).await?;
    // a locally administered address of its own
    let mut mac: [u8; 6] = rand::random();
    mac[0] = (mac[0] & 0xfc) | 0x02;
    let mac = mac.map(|b| format!("{b:02x}")).join(":");
    run("ip", &["link", "set", "dev", IFACE, "address", &mac]).await?;

    // NetworkManager picks the new interface up after a moment and leaves
    // it unmanaged, it was created outside of it
    let deadline = Instant::now() + Duration::from_secs(10);
    while run("nmcli", &["-g", "GENERAL.STATE", "device", "show", IFACE]).await.is_err() {
        if Instant::now() > deadline {
            bail!("NetworkManager does not see {IFACE}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    run("nmcli", &["device", "set", IFACE, "managed", "yes"]).await?;
    // 30 is disconnected, ready for a connection
    loop {
        let state = run("nmcli", &["-g", "GENERAL.STATE", "device", "show", IFACE]).await?;
        if state.trim().starts_with("30") {
            break;
        }
        if Instant::now() > deadline {
            bail!("{IFACE} stays in state {}", state.trim());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let ssid = format!("T2AirShare-{}", random_text(4));
    let password = random_text(16);
    let channel = channel.to_string();
    let address = format!("{ADDRESS}/24");
    run(
        "nmcli",
        &[
            "connection", "add", "save", "no", "type", "wifi", "ifname", IFACE,
            "con-name", CONNECTION, "autoconnect", "no", "ssid", &ssid,
            "802-11-wireless.mode", "ap", "802-11-wireless.band", band,
            "802-11-wireless.channel", &channel,
            "ipv4.method", "shared", "ipv4.addresses", &address,
            "ipv6.method", "disabled", "connection.zone", "nm-shared",
            // With WPA2 and CCMP alone the access point of the Apple firmware
            // only offers PSK-SHA256, which an S22 does not join. With WPA
            // and WPA2, TKIP and CCMP and the TKIP group cipher it joins.
            "wifi-sec.key-mgmt", "wpa-psk", "wifi-sec.proto", "wpa,rsn",
            "wifi-sec.pairwise", "tkip,ccmp", "wifi-sec.group", "tkip",
            "wifi-sec.pmf", "disable", "wifi-sec.wps-method", "disabled",
            "wifi-sec.psk", &password,
        ],
    )
    .await?;
    run("nmcli", &["--wait", "30", "connection", "up", CONNECTION]).await?;
    if run("firewall-cmd", &["--state"]).await.is_ok() {
        run("firewall-cmd", &["--zone=nm-shared", &format!("--add-port={port}/tcp")]).await?;
    }
    Ok((ssid, password, ADDRESS.to_string(), freq as i32))
}

/// Removes everything `setup` may have left, errors only mean it was not
/// there.
async fn teardown(port: Option<u16>) {
    if let Some(port) = port {
        let _ = run("firewall-cmd", &["--zone=nm-shared", &format!("--remove-port={port}/tcp")]).await;
    }
    let _ = run("nmcli", &["connection", "delete", CONNECTION]).await;
    let _ = run("iw", &["dev", IFACE, "del"]).await;
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // leftovers of a helper that did not stop cleanly
    teardown(None).await;

    let hotspot = Hotspot {
        state: Arc::new(Mutex::new(State { active: None, idle_since: Some(Instant::now()) })),
    };
    let connection = zbus::connection::Builder::system()?
        .serve_at(PATH, hotspot.clone())?
        .name(BUS_NAME)?
        .build()
        .await
        .context("system bus")?;

    let dbus = fdo::DBusProxy::new(&connection).await?;
    let mut owners = dbus.receive_name_owner_changed().await?;
    let mut check = tokio::time::interval(Duration::from_secs(10));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            Some(signal) = owners.next() => {
                let Ok(args) = signal.args() else { continue };
                if args.new_owner().is_none() {
                    hotspot.stop_for(args.name().as_str()).await;
                }
            }
            _ = check.tick() => {
                let state = hotspot.state.lock().await;
                if state.idle_since.is_some_and(|t| t.elapsed() > IDLE_EXIT) {
                    info!("idle, exiting");
                    break;
                }
            }
            _ = term.recv() => break,
        }
    }
    let port = hotspot.state.lock().await.active.take().map(|a| a.port);
    teardown(port).await;
    drop(connection);
    Ok(())
}
