// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv6Addr;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Manage Apple T2 RemoteXPC services")]
pub struct Cli {
    #[arg(long, default_value = "/run/t2remote.sock", global = true)]
    pub socket: PathBuf,

    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand)]
pub enum Action {
    Daemon {
        #[arg(long)]
        interface: Option<String>,
        #[arg(long)]
        host: Option<Ipv6Addr>,
    },
    Acquire {
        service: String,
    },
    Release {
        service: String,
    },
    PreSuspend,
    PostResume,
    Status,
}

pub fn request(socket: &Path, command: &str) -> Result<()> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connect {}", socket.display()))?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply = reply.trim();
    if let Some(error) = reply.strip_prefix("ERR ") {
        bail!("{error}");
    }
    println!("{reply}");
    Ok(())
}
