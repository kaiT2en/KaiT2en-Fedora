// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! T2 Debug core: talk to the Apple T2's Instruments `dtservicehub` over the
//! CDC-NCM link and decode its diagnostic channels — a process "top"
//! (sysmontap) and the live os_log stream (activitytracetap). Read/observe
//! only. Shared by the GUI (`t2-debug`) and the headless CLI (`t2debug`).

pub mod archive;
pub mod dtx;
pub mod logfmt;
pub mod session;
pub mod sysmon;
pub mod tap;

use anyhow::{Context, Result};

pub use t2_bridgexpc::discovery;

pub const SERVICE: &str = "com.apple.instruments.dtservicehub";
pub const DEVICEINFO: &str = "com.apple.instruments.server.services.deviceinfo";
pub const SYSMONTAP: &str = "com.apple.instruments.server.services.sysmontap";
pub const ACTIVITYTAP: &str = "com.apple.instruments.server.services.activitytracetap";

/// Discover the CDC-NCM link and connect to the T2's dtservicehub, with a
/// caller-owned stop flag for clean background teardown.
pub fn connect_with_stop(
    interface: Option<String>,
    host: Option<String>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<dtx::Dtx> {
    let interface = discovery::interface(interface)?;
    let host = discovery::host(&interface, host)?;
    let discovered = t2_bridgexpc::remote::discover_direct_named_service(&interface, host, SERVICE)
        .context("locate dtservicehub via BridgeXPC service directory")?;
    dtx::Dtx::connect_with_stop(&interface, host, discovered.service_port, stop)
}

/// Discover the link and connect (CLI path; own stop flag).
pub fn connect(interface: Option<String>, host: Option<String>) -> Result<dtx::Dtx> {
    connect_with_stop(interface, host, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
}
