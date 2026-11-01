// SPDX-License-Identifier: GPL-3.0-or-later
//! Bandwidth upgrade of a transfer that started over Bluetooth. As in
//! google/nearby (commit 203b6e1) connections/implementation/bwu_manager.cc
//! the receiver starts it: it opens a Wi-Fi hotspot and sends its
//! credentials over the encrypted Bluetooth channel. The phone joins,
//! connects over TCP and introduces itself in plain text. Both sides then
//! send LAST_WRITE_TO_PRIOR_CHANNEL on Bluetooth, answer it with
//! SAFE_TO_CLOSE_PRIOR_CHANNEL and continue on TCP with the same keys and
//! sequence numbers.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use log::info;
use prost::Message;
use tokio::net::TcpListener;

use crate::hotspot::Hotspot;
use crate::link::{Frames, Writer, offline_frame, write_frame};
use crate::proto::location::nearby::connections as nc;

use nc::bandwidth_upgrade_negotiation_frame::{EventType, UpgradePathInfo, upgrade_path_info};

/// How long the phone gets to join the hotspot and connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A hotspot that is up, with the listener the phone connects to.
pub struct Prepared {
    pub listener: TcpListener,
    pub hotspot: Hotspot,
}

pub async fn prepare() -> Result<Prepared> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).await.context("listen")?;
    let port = listener.local_addr()?.port();
    let hotspot = Hotspot::start(port).await?;
    info!("hotspot {} on {} MHz, waiting on port {port}", hotspot.ssid, hotspot.frequency);
    Ok(Prepared { listener, hotspot })
}

fn negotiation(frame: nc::BandwidthUpgradeNegotiationFrame) -> nc::V1Frame {
    nc::V1Frame {
        r#type: Some(nc::v1_frame::FrameType::BandwidthUpgradeNegotiation.into()),
        bandwidth_upgrade_negotiation: Some(frame),
        ..Default::default()
    }
}

/// UPGRADE_PATH_AVAILABLE with the hotspot credentials. Encryption stays
/// on, the phone confirms its introduction with an ack.
pub fn path_available(prepared: &Prepared) -> Result<nc::V1Frame> {
    let hotspot = &prepared.hotspot;
    let port = prepared.listener.local_addr()?.port();
    Ok(negotiation(nc::BandwidthUpgradeNegotiationFrame {
        event_type: Some(EventType::UpgradePathAvailable.into()),
        upgrade_path_info: Some(UpgradePathInfo {
            medium: Some(upgrade_path_info::Medium::WifiHotspot.into()),
            wifi_hotspot_credentials: Some(upgrade_path_info::WifiHotspotCredentials {
                ssid: Some(hotspot.ssid.clone()),
                password: Some(hotspot.password.clone()),
                port: Some(port.into()),
                gateway: Some(hotspot.gateway.clone()),
                frequency: Some(hotspot.frequency),
            }),
            supports_disabling_encryption: Some(false),
            supports_client_introduction_ack: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

pub fn event(event_type: EventType) -> nc::V1Frame {
    negotiation(nc::BandwidthUpgradeNegotiationFrame {
        event_type: Some(event_type.into()),
        ..Default::default()
    })
}

/// Waits for the phone on the hotspot, reads its CLIENT_INTRODUCTION and
/// acks it, both in plain text.
pub async fn accept(listener: TcpListener) -> Result<(Frames, Writer)> {
    let (stream, peer) = tokio::time::timeout(CONNECT_TIMEOUT, listener.accept())
        .await
        .map_err(|_| anyhow!("the phone did not connect to the hotspot"))??;
    info!("upgrade connection from {peer}");
    let (mut frames, mut writer) = Frames::split(stream);
    let v1 = frames.expect(nc::v1_frame::FrameType::BandwidthUpgradeNegotiation).await?;
    let negotiation = v1.bandwidth_upgrade_negotiation.unwrap_or_default();
    if negotiation.event_type() != EventType::ClientIntroduction {
        bail!("expected CLIENT_INTRODUCTION, got {:?}", negotiation.event_type());
    }
    let ack = offline_frame(event(EventType::ClientIntroductionAck));
    write_frame(&mut writer, &ack.encode_to_vec()).await?;
    Ok((frames, writer))
}

/// The plain text DISCONNECTION that ends the prior channel.
pub fn disconnection() -> Vec<u8> {
    offline_frame(nc::V1Frame {
        r#type: Some(nc::v1_frame::FrameType::Disconnection.into()),
        disconnection: Some(nc::DisconnectionFrame {
            request_safe_to_disconnect: Some(false),
            ack_safe_to_disconnect: Some(false),
        }),
        ..Default::default()
    })
    .encode_to_vec()
}
