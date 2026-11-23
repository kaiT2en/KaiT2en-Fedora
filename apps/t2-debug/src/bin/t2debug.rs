// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! t2debug: headless CLI over the T2's Instruments dtservicehub. Same engine as
//! the GUI (t2-debug), no gtk. Handy over SSH. Read/observe only.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use t2_debug::archive::Obj;
use t2_debug::dtx::{self, Arg, Dtx, Payload};
use t2_debug::logfmt::{self, Filter};
use t2_debug::sysmon;
use t2_debug::tap;
use t2_debug::{ACTIVITYTAP, DEVICEINFO, SYSMONTAP};

#[derive(Parser)]
#[command(about = "Headless T2 dtservicehub client (process top + live os_log)")]
struct Cli {
    /// CDC-NCM interface (auto-detected when omitted)
    #[arg(long)]
    interface: Option<String>,
    /// T2 link-local address (auto-detected when omitted)
    #[arg(long)]
    host: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capabilities the hub advertises after the handshake
    Capabilities,
    /// Running bridgeOS processes (pid, name, path)
    Procs,
    /// deviceinfo scalar selectors (mach time, trace codes, ...)
    Info,
    /// Directory listing of a bridgeOS path
    Ls { path: String },
    /// System + per-process sample (CPU, power, memory, wakeups)
    Sysmon {
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 0)]
        count: usize,
        #[arg(long, default_value_t = 12)]
        top: usize,
    },
    /// Ask the hub to open a service channel and print its reply (RE probe).
    /// A bare name gets the `com.apple.instruments.server.services.` prefix.
    /// With a selector, call it on the opened channel with an optional string arg.
    Channel {
        name: String,
        selector: Option<String>,
        arg: Option<String>,
    },
    /// Call an arbitrary deviceinfo selector (RE probe), with an optional string arg
    Call {
        selector: String,
        arg: Option<String>,
    },
    /// Decode a saved raw capture offline (no T2 connection)
    Decode {
        file: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Stream and decode the live os_log / activity tap
    Logs {
        #[arg(long, default_value_t = 0)]
        count: usize,
        #[arg(long)]
        save: Option<String>,
        #[command(flatten)]
        filter: FilterArgs,
    },
}

#[derive(clap::Args)]
struct FilterArgs {
    /// Regex over the whole formatted line
    #[arg(long)]
    grep: Option<String>,
    /// Substring match on the subsystem (case-insensitive)
    #[arg(long)]
    subsystem: Option<String>,
    /// Only this process id
    #[arg(long)]
    pid: Option<u64>,
    /// Drop lines whose whole text matches this regex (inverse of --grep)
    #[arg(long)]
    exclude: Option<String>,
    /// Drop the HID input-device noise (trackpad, keyboard, relay)
    #[arg(long)]
    no_hid: bool,
}

impl FilterArgs {
    fn build(&self) -> Result<Filter> {
        Filter::new(
            self.grep.as_deref(),
            self.subsystem.as_deref(),
            self.pid,
            self.exclude.as_deref(),
            self.no_hid,
        )
    }
}

extern "C" fn on_signal(_: libc::c_int) {
    dtx::INTERRUPTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn main() -> Result<()> {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        // Ignore SIGPIPE so a closed pipe (e.g. `| head`) does not kill us
        // mid-stream: the write then unwinds, Drop sends the FIN, and the hub
        // stays healthy instead of wedging on a half-open connection.
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let cli = Cli::parse();

    if let Command::Decode { file, filter } = &cli.command {
        return decode_file(file, &filter.build()?);
    }

    let mut dtx = t2_debug::connect(cli.interface, cli.host)?;
    match cli.command {
        Command::Capabilities => print_json(&dtx.capabilities),
        Command::Procs => procs(&mut dtx)?,
        Command::Info => info(&mut dtx)?,
        Command::Ls { path } => ls(&mut dtx, &path)?,
        Command::Channel { name, selector, arg } => {
            channel(&mut dtx, &name, selector.as_deref(), arg.as_deref())?
        }
        Command::Call { selector, arg } => call(&mut dtx, &selector, arg.as_deref())?,
        Command::Sysmon { watch, count, top } => sysmon(&mut dtx, watch, count, top)?,
        Command::Logs { count, save, filter } => {
            logs(&mut dtx, count, save.as_deref(), &filter.build()?)?
        }
        Command::Decode { .. } => unreachable!("handled before connecting"),
    }
    Ok(())
}

fn print_json(obj: &Obj) {
    println!("{}", serde_json::to_string_pretty(&obj.to_json()).unwrap());
}

fn procs(dtx: &mut Dtx) -> Result<()> {
    dtx.open_channel(1, DEVICEINFO)?;
    let reply = dtx.call(1, "runningProcesses", &[])?;
    let Payload::Obj(Obj::Array(rows)) = reply.payload else {
        return Ok(print_json(reply.object().unwrap_or(&Obj::Null)));
    };
    for row in rows {
        let pid = row.get("pid").and_then(Obj::as_f64).unwrap_or_default() as i64;
        let name = row.get("name").and_then(Obj::as_str).unwrap_or("?");
        let path = row.get("realAppName").and_then(Obj::as_str).unwrap_or("");
        println!("{pid:>5}  {name:<28} {path}");
    }
    Ok(())
}

fn info(dtx: &mut Dtx) -> Result<()> {
    dtx.open_channel(1, DEVICEINFO)?;
    for selector in ["machTimeInfo", "networkInformation", "traceCodesFile"] {
        match dtx.call(1, selector, &[]) {
            Ok(message) => match message.payload {
                Payload::Obj(Obj::Str(text)) => {
                    println!("== {selector}  ({} lines)", text.lines().count())
                }
                Payload::Obj(obj) => {
                    println!("== {selector}");
                    print_json(&obj);
                }
                _ => println!("== {selector}: (no object)"),
            },
            Err(error) => println!("== {selector}: {error}"),
        }
    }
    Ok(())
}

fn ls(dtx: &mut Dtx, path: &str) -> Result<()> {
    dtx.open_channel(1, DEVICEINFO)?;
    let reply = dtx.call(1, "directoryListingForPath:", &[Arg::Obj(Obj::Str(path.into()))])?;
    match reply.payload {
        Payload::Obj(Obj::Array(entries)) => {
            for entry in entries {
                if let Some(name) = entry.as_str() {
                    println!("{name}");
                }
            }
        }
        Payload::Obj(Obj::Null) | Payload::None => eprintln!("{path}: no listing"),
        Payload::Obj(other) => print_json(&other),
        Payload::Raw(_) => eprintln!("{path}: unexpected raw reply"),
    }
    Ok(())
}

fn channel(dtx: &mut Dtx, name: &str, selector: Option<&str>, arg: Option<&str>) -> Result<()> {
    let name = if name.contains('.') {
        name.to_owned()
    } else {
        format!("com.apple.instruments.server.services.{name}")
    };
    let ident = dtx.send(
        0,
        Some("_requestChannelWithCode:identifier:"),
        &[Arg::I32(1), Arg::Obj(Obj::Str(name.clone()))],
        true,
    )?;
    let mut reply = dtx.wait(ident)?;
    println!("{name}: flags 0x{:x}", reply.flags);
    if let Some(selector) = selector {
        let args: Vec<Arg> = arg.map(|value| Arg::Obj(Obj::Str(value.into()))).into_iter().collect();
        reply = dtx.call(1, selector, &args)?;
        println!("{selector}: flags 0x{:x}", reply.flags);
    }
    match reply.payload {
        Payload::Obj(Obj::Str(text)) => println!("{text}"),
        Payload::Obj(obj) => print_json(&obj),
        Payload::Raw(bytes) => println!("<{} raw bytes> {}", bytes.len(), hexdump(&bytes)),
        Payload::None => println!("(no payload)"),
    }
    Ok(())
}

fn call(dtx: &mut Dtx, selector: &str, arg: Option<&str>) -> Result<()> {
    dtx.open_channel(1, DEVICEINFO)?;
    let args: Vec<Arg> = match arg {
        Some(value) => vec![Arg::Obj(Obj::Str(value.into()))],
        None => vec![],
    };
    let reply = dtx.call(1, selector, &args)?;
    match reply.payload {
        Payload::Obj(Obj::Str(text)) => println!("{text}"),
        Payload::Obj(obj) => print_json(&obj),
        Payload::Raw(bytes) => println!("<{} raw bytes> {}", bytes.len(), hexdump(&bytes)),
        Payload::None => println!("(no payload)"),
    }
    Ok(())
}

fn hexdump(bytes: &[u8]) -> String {
    bytes.iter().take(64).map(|b| format!("{b:02x}")).collect()
}

fn sysmon(dtx: &mut Dtx, watch: bool, count: usize, top: usize) -> Result<()> {
    dtx.open_channel(1, DEVICEINFO)?;
    let proc_attrs = sysmon::string_list(dtx.call(1, "sysmonProcessAttributes", &[])?.payload);
    let sys_attrs = sysmon::string_list(dtx.call(1, "sysmonSystemAttributes", &[])?.payload);

    dtx.open_channel(2, SYSMONTAP)?;
    dtx.call(2, "setConfig:", &[Arg::Obj(sysmon::config(&proc_attrs, &sys_attrs))])?;
    dtx.send(2, Some("start"), &[], true)?;

    let mut proc_samples = 0usize;
    let mut printed = 0usize;
    let mut last_cpu = 0.0f64;
    let stop = loop {
        let message = match dtx.recv() {
            Ok(message) => message,
            Err(_) if dtx::interrupted() => break false,
            Err(error) => return Err(error),
        };
        let Payload::Obj(Obj::Array(rows)) = &message.payload else {
            continue;
        };
        for row in rows {
            if let Some(load) = row.get("SystemCPUUsage").and_then(|u| u.get("CPU_TotalLoad")) {
                last_cpu = load.as_f64().unwrap_or(last_cpu);
            }
            if !watch {
                if let Some(Obj::Array(values)) = row.get("System") {
                    println!("== system");
                    for (name, value) in sysmon::decode_system(values, &sys_attrs) {
                        println!("  {name:<22} {value}");
                    }
                }
            }
            let Some(processes) = row.get("Processes") else {
                continue;
            };
            proc_samples += 1;
            if proc_samples < 2 {
                continue;
            }
            if watch {
                println!("-- sample {} (cpu total {last_cpu:.0}%) --", proc_samples - 1);
            }
            print_processes(processes, &proc_attrs, top);
            printed += 1;
            if !watch || (count != 0 && printed >= count) {
                break;
            }
            println!();
        }
        if !watch || (count != 0 && printed >= count) || dtx::interrupted() {
            break true;
        }
    };
    if stop {
        dtx.send(2, Some("stop"), &[], true)?;
    }
    Ok(())
}

fn print_processes(processes: &Obj, attrs: &[String], top: usize) {
    println!(
        "  {:>5}  {:<22} {:>6}  {:>6}  {:>8}  {:>8}  {:>8}",
        "pid", "name", "cpu%", "power", "wakeups", "footMB", "diskWkB"
    );
    for p in sysmon::decode_processes(processes, attrs, Some(top)) {
        println!(
            "  {:>5}  {:<22} {:>6.2}  {:>6.2}  {:>8}  {:>8.1}  {:>8.0}",
            p.pid, p.name, p.cpu, p.power, p.wakeups, p.foot_mb, p.disk_kb
        );
    }
}

fn logs(dtx: &mut Dtx, count: usize, save: Option<&str>, filter: &Filter) -> Result<()> {
    use std::io::Write as _;
    dtx.open_channel(1, ACTIVITYTAP)?;
    dtx.call(1, "setConfig:", &[Arg::Obj(logs_config())])?;
    dtx.send(1, Some("start"), &[], true)?;

    let mut file = save.map(std::fs::File::create).transpose()?;
    let mut decoder = tap::Decoder::default();
    let mut mapping = None;
    let mut shown = 0usize;
    let stop = loop {
        let message = match dtx.recv() {
            Ok(message) => message,
            Err(_) if dtx::interrupted() => break false,
            Err(error) => return Err(error),
        };
        match message.payload {
            Payload::Obj(ref obj) => {
                if let Some(found) = logfmt::parse_time_mapping(obj) {
                    mapping = Some(found);
                }
            }
            Payload::Raw(bytes) => {
                if let Some(file) = file.as_mut() {
                    file.write_all(&(bytes.len() as u32).to_le_bytes())?;
                    file.write_all(&bytes)?;
                }
                for row in decoder.feed(&bytes) {
                    if let Some(line) = logfmt::format_log_row(&row, mapping.as_ref()) {
                        if filter.passes(&line) {
                            println!("{}", line.text);
                            shown += 1;
                            if count != 0 && shown >= count {
                                break;
                            }
                        }
                    }
                }
                if count != 0 && shown >= count {
                    break true;
                }
            }
            Payload::None => {}
        }
        if dtx::interrupted() {
            break false;
        }
    };
    if stop {
        dtx.send(1, Some("stop"), &[], true)?;
    }
    Ok(())
}

fn decode_file(path: &str, filter: &Filter) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("read {path}"))?;
    let mut decoder = tap::Decoder::default();
    let mut offset = 0usize;
    while offset + 4 <= data.len() {
        let len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let end = (offset + len).min(data.len());
        for row in decoder.feed(&data[offset..end]) {
            if let Some(line) = logfmt::format_log_row(&row, None) {
                if filter.passes(&line) {
                    println!("{}", line.text);
                }
            }
        }
        offset = end;
    }
    Ok(())
}

fn logs_config() -> Obj {
    Obj::Dict(vec![
        ("bm".into(), Obj::Int(0)),
        ("combineDataScope".into(), Obj::Int(0)),
        ("machTimebaseDenom".into(), Obj::Int(3)),
        ("machTimebaseNumer".into(), Obj::Int(125)),
        ("onlySignposts".into(), Obj::Int(0)),
        ("pidToInjectCombineDYLIB".into(), Obj::Str("-1".into())),
        (
            "predicate".into(),
            Obj::Str(
                "(messageType == info OR messageType == debug OR messageType == default \
                 OR messageType == error OR messageType == fault)"
                    .into(),
            ),
        ),
        ("signpostsAndLogs".into(), Obj::Int(1)),
        ("trackPidToExecNameMapping".into(), Obj::Bool(true)),
        ("enableHTTPArchiveLogging".into(), Obj::Bool(false)),
        ("targetPID".into(), Obj::Int(-3)),
        ("trackExpiredPIDs".into(), Obj::Int(1)),
        ("ur".into(), Obj::Int(500)),
    ])
}
