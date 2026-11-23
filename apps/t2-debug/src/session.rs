// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! One long-lived dtservicehub connection, driven by commands from the UI.
//!
//! A single background thread owns one connection and switches between the
//! sysmontap "top" and the activitytracetap log on command, so the app never
//! reconnects per tab. Reconnect churn is what wedges the bridgeOS deviceinfo
//! backend, and a single connection also means no blocking teardown on the UI
//! thread. On stop it tells the active tap to stop and drops the socket (FIN).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};

use anyhow::Result;

use crate::archive::Obj;
use crate::dtx::{Arg, Dtx, Payload};
use crate::logfmt::{self, LogLine, TimeMapping};
use crate::sysmon::{self, Proc};
use crate::{ACTIVITYTAP, DEVICEINFO, SYSMONTAP};

const CH_DEVICEINFO: i32 = 1;
const CH_SYSMONTAP: i32 = 2;
const CH_ACTIVITYTAP: i32 = 3;

/// Which stream the worker is currently delivering.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sysmon,
    Logs,
}

/// A one-shot deviceinfo query requested from the UI.
pub enum Selector {
    Procs,
    MachTime,
    NetworkInfo,
    TraceCodes,
    Capabilities,
    Ls(String),
}

enum Command {
    SetMode(Mode),
    Selector(Selector),
}

/// What the worker sends back to the UI.
pub enum Event {
    Connected,
    Sysmon { cpu_total: f64, system: Vec<(String, String)>, procs: Vec<Proc> },
    Log(LogLine),
    Selector { name: String, text: String },
    Error(String),
}

/// A running worker. Dropping or `shutdown`ing it stops the stream; the thread
/// then tears the connection down on its own (no blocking join on the UI).
pub struct Session {
    stop: Arc<AtomicBool>,
    commands: Sender<Command>,
    _join: JoinHandle<()>,
}

impl Session {
    pub fn start(
        interface: Option<String>,
        host: Option<String>,
        events: Sender<Event>,
    ) -> Session {
        let stop = Arc::new(AtomicBool::new(false));
        let (commands_tx, commands_rx) = channel();
        let worker_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            if let Err(error) = run(interface, host, &worker_stop, &commands_rx, &events) {
                if !worker_stop.load(Ordering::Relaxed) {
                    let _ = events.send(Event::Error(format!("{error:#}")));
                }
            }
        });
        Session { stop, commands: commands_tx, _join: join }
    }

    pub fn set_mode(&self, mode: Mode) {
        let _ = self.commands.send(Command::SetMode(mode));
    }

    pub fn request(&self, selector: Selector) {
        let _ = self.commands.send(Command::Selector(selector));
    }

    /// Ask the worker to stop; it tears the connection down on its own.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Per-connection worker state.
struct Worker {
    dtx: Dtx,
    mode: Option<Mode>,
    attrs: Option<(Vec<String>, Vec<String>)>, // sysmon process / system attributes
    sysmon_running: bool,
    logs_running: bool,
    decoder: crate::tap::Decoder,
    mapping: Option<TimeMapping>,
    last_cpu: f64,
    system: Vec<(String, String)>,
    proc_samples: usize,
}

fn run(
    interface: Option<String>,
    host: Option<String>,
    stop: &Arc<AtomicBool>,
    commands: &Receiver<Command>,
    events: &Sender<Event>,
) -> Result<()> {
    let dtx = crate::connect_with_stop(interface, host, Arc::clone(stop))?;
    let _ = events.send(Event::Connected);
    let mut worker = Worker {
        dtx,
        mode: None,
        attrs: None,
        sysmon_running: false,
        logs_running: false,
        decoder: crate::tap::Decoder::default(),
        mapping: None,
        last_cpu: 0.0,
        system: Vec::new(),
        proc_samples: 0,
    };

    while !stop.load(Ordering::Relaxed) {
        // Apply any pending commands first (mode switches, selectors).
        loop {
            match commands.try_recv() {
                Ok(Command::SetMode(mode)) => worker.set_mode(mode, events)?,
                Ok(Command::Selector(selector)) => worker.selector(selector, events),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if worker.mode.is_none() {
            // Nothing started yet; wait for the first SetMode without busy-looping.
            match commands.recv() {
                Ok(Command::SetMode(mode)) => worker.set_mode(mode, events)?,
                Ok(Command::Selector(selector)) => worker.selector(selector, events),
                Err(_) => break,
            }
            continue;
        }

        let message = match worker.dtx.recv() {
            Ok(message) => message,
            Err(_) if stop.load(Ordering::Relaxed) => break,
            Err(error) => return Err(error),
        };
        worker.handle(message, events);
    }

    worker.stop_taps();
    Ok(())
}

impl Worker {
    fn set_mode(&mut self, mode: Mode, events: &Sender<Event>) -> Result<()> {
        if self.mode == Some(mode) {
            return Ok(());
        }
        match mode {
            Mode::Sysmon => {
                if self.logs_running {
                    let _ = self.dtx.send(CH_ACTIVITYTAP, Some("stop"), &[], true);
                    self.logs_running = false;
                }
                self.start_sysmon()?;
            }
            Mode::Logs => {
                if self.sysmon_running {
                    let _ = self.dtx.send(CH_SYSMONTAP, Some("stop"), &[], true);
                    self.sysmon_running = false;
                }
                self.start_logs()?;
            }
        }
        self.mode = Some(mode);
        let _ = events;
        Ok(())
    }

    fn start_sysmon(&mut self) -> Result<()> {
        self.dtx.open_channel(CH_DEVICEINFO, DEVICEINFO)?;
        if self.attrs.is_none() {
            let proc_attrs =
                sysmon::string_list(self.dtx.call(CH_DEVICEINFO, "sysmonProcessAttributes", &[])?.payload);
            let sys_attrs =
                sysmon::string_list(self.dtx.call(CH_DEVICEINFO, "sysmonSystemAttributes", &[])?.payload);
            self.attrs = Some((proc_attrs, sys_attrs));
        }
        let (proc_attrs, sys_attrs) = self.attrs.as_ref().unwrap();
        self.dtx.open_channel(CH_SYSMONTAP, SYSMONTAP)?;
        self.dtx.call(CH_SYSMONTAP, "setConfig:", &[Arg::Obj(sysmon::config(proc_attrs, sys_attrs))])?;
        self.dtx.send(CH_SYSMONTAP, Some("start"), &[], true)?;
        self.sysmon_running = true;
        self.proc_samples = 0;
        Ok(())
    }

    fn start_logs(&mut self) -> Result<()> {
        self.dtx.open_channel(CH_ACTIVITYTAP, ACTIVITYTAP)?;
        self.dtx.call(CH_ACTIVITYTAP, "setConfig:", &[Arg::Obj(logs_config())])?;
        self.dtx.send(CH_ACTIVITYTAP, Some("start"), &[], true)?;
        self.logs_running = true;
        Ok(())
    }

    fn stop_taps(&mut self) {
        if self.sysmon_running {
            let _ = self.dtx.send(CH_SYSMONTAP, Some("stop"), &[], true);
        }
        if self.logs_running {
            let _ = self.dtx.send(CH_ACTIVITYTAP, Some("stop"), &[], true);
        }
    }

    fn handle(&mut self, message: crate::dtx::Message, events: &Sender<Event>) {
        match self.mode {
            Some(Mode::Sysmon) => self.handle_sysmon(&message, events),
            Some(Mode::Logs) => self.handle_logs(message, events),
            None => {}
        }
    }

    fn handle_sysmon(&mut self, message: &crate::dtx::Message, events: &Sender<Event>) {
        let Payload::Obj(Obj::Array(rows)) = &message.payload else {
            return;
        };
        let Some((proc_attrs, sys_attrs)) = self.attrs.as_ref() else {
            return;
        };
        for row in rows {
            if let Some(load) = row.get("SystemCPUUsage").and_then(|u| u.get("CPU_TotalLoad")) {
                self.last_cpu = load.as_f64().unwrap_or(self.last_cpu);
            }
            if let Some(Obj::Array(values)) = row.get("System") {
                self.system = sysmon::decode_system(values, sys_attrs);
            }
            if let Some(processes) = row.get("Processes") {
                self.proc_samples += 1;
                if self.proc_samples < 2 {
                    continue; // first sample has no deltas
                }
                let procs = sysmon::decode_processes(processes, proc_attrs, None);
                let _ = events.send(Event::Sysmon {
                    cpu_total: self.last_cpu,
                    system: self.system.clone(),
                    procs,
                });
            }
        }
    }

    fn handle_logs(&mut self, message: crate::dtx::Message, events: &Sender<Event>) {
        match message.payload {
            Payload::Obj(ref obj) => {
                if let Some(found) = logfmt::parse_time_mapping(obj) {
                    self.mapping = Some(found);
                }
            }
            Payload::Raw(bytes) => {
                for row in self.decoder.feed(&bytes) {
                    if let Some(line) = logfmt::format_log_row(&row, self.mapping.as_ref()) {
                        let _ = events.send(Event::Log(line));
                    }
                }
            }
            Payload::None => {}
        }
    }

    fn selector(&mut self, selector: Selector, events: &Sender<Event>) {
        if self.dtx.open_channel(CH_DEVICEINFO, DEVICEINFO).is_err() {
            return;
        }
        let (name, text) = match selector {
            Selector::Procs => ("runningProcesses".to_owned(), self.selector_procs()),
            Selector::MachTime => ("machTimeInfo".to_owned(), self.selector_json("machTimeInfo")),
            Selector::NetworkInfo => {
                ("networkInformation".to_owned(), self.selector_json("networkInformation"))
            }
            Selector::TraceCodes => ("traceCodesFile".to_owned(), self.selector_trace_codes()),
            Selector::Capabilities => (
                "capabilities".to_owned(),
                serde_json::to_string_pretty(&self.dtx.capabilities.to_json()).unwrap_or_default(),
            ),
            Selector::Ls(path) => {
                let text = self.selector_ls(&path);
                (format!("ls {path}"), text)
            }
        };
        let _ = events.send(Event::Selector { name, text });
    }

    fn selector_json(&mut self, selector: &str) -> String {
        match self.dtx.call(CH_DEVICEINFO, selector, &[]) {
            Ok(message) => match message.payload {
                Payload::Obj(obj) => {
                    serde_json::to_string_pretty(&obj.to_json()).unwrap_or_default()
                }
                _ => "(no object)".into(),
            },
            Err(error) => format!("error: {error:#}"),
        }
    }

    fn selector_trace_codes(&mut self) -> String {
        match self.dtx.call(CH_DEVICEINFO, "traceCodesFile", &[]) {
            Ok(message) => match message.payload {
                Payload::Obj(Obj::Str(text)) => text,
                _ => "(no text)".into(),
            },
            Err(error) => format!("error: {error:#}"),
        }
    }

    fn selector_procs(&mut self) -> String {
        match self.dtx.call(CH_DEVICEINFO, "runningProcesses", &[]) {
            Ok(message) => match message.payload {
                Payload::Obj(Obj::Array(rows)) => rows
                    .iter()
                    .map(|row| {
                        let pid = row.get("pid").and_then(Obj::as_f64).unwrap_or_default() as i64;
                        let name = row.get("name").and_then(Obj::as_str).unwrap_or("?");
                        let path = row.get("realAppName").and_then(Obj::as_str).unwrap_or("");
                        format!("{pid:>5}  {name:<28} {path}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => "(no processes)".into(),
            },
            Err(error) => format!("error: {error:#}"),
        }
    }

    fn selector_ls(&mut self, path: &str) -> String {
        match self.dtx.call(CH_DEVICEINFO, "directoryListingForPath:", &[Arg::Obj(Obj::Str(path.into()))])
        {
            Ok(message) => match message.payload {
                Payload::Obj(Obj::Array(entries)) => entries
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Payload::Obj(Obj::Null) | Payload::None => format!("{path}: no listing"),
                _ => format!("{path}: unexpected reply"),
            },
            Err(error) => format!("error: {error:#}"),
        }
    }
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
