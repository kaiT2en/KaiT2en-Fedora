// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

use std::net::Ipv6Addr;
use anyhow::Result;
use t2_bridgexpc::{remote, xpc::Value};

pub struct Service(remote::RemoteService);

impl Service {
    pub fn connect(interface: &str, host: Ipv6Addr, name: &str) -> Result<Self> {
        let ave = name == "com.apple.aveservice";
        let mut connection = remote::connect_remote_service(
            interface, host, name, ave.then_some(1024 * 1024),
        )?;
        if ave {
            connection.send(&Value::Dict(vec![("startKey".into(), Value::Bool(true))]), 0x101, 1)?;
        }
        Ok(Self(connection))
    }

    pub fn close(mut self, name: &str) -> Result<()> {
        if name == "com.apple.aveservice" {
            self.0.send(&Value::Dict(vec![("stopKey".into(), Value::Bool(true))]), 0x100000, 3)?;
        }
        self.0.close()
    }
}
