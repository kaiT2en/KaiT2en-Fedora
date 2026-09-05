// SPDX-License-Identifier: MIT

mod archive;
mod discovery;
mod historical;
mod journal;
mod progress;
mod record;
mod remote;
mod store;
mod unified;
mod xpc;

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use regex::Regex;

use record::Record;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Select Linux boot by relative index, or all retained boots
    #[arg(short = 'b', long, num_args = 0..=1, default_missing_value = "0", allow_negative_numbers = true)]
    boot: Option<journal::Boot>,

    /// List Linux boots
    #[arg(long)]
    list_boots: bool,

    /// Filter record text and metadata with a regular expression
    #[arg(short = 'g', long)]
    grep: Option<Regex>,

    /// Limit output to one source
    #[arg(long, value_enum)]
    source: Option<Source>,

    /// Output representation
    #[arg(short = 'o', long, value_enum, default_value = "text")]
    output: Output,

    /// Override the BridgeOS snapshot path
    #[arg(long, global = true)]
    state_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch and replace the current BridgeOS snapshot
    Refresh(Refresh),
}

#[derive(Args, Default)]
struct Refresh {
    /// Parse an existing sysdiagnose archive instead of downloading one
    #[arg(long)]
    archive: Option<PathBuf>,

    /// T2 CDC-NCM interface; detected automatically when omitted
    #[arg(long)]
    interface: Option<String>,

    /// T2 IPv6 link-local address; detected automatically when omitted
    #[arg(long)]
    host: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Source {
    Linux,
    T2,
}

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Jsonl,
}

fn refresh(args: &Refresh, state_file: PathBuf) -> Result<()> {
    store::prepare(&state_file)?;
    let work = tempfile::Builder::new().prefix("t2-journal-").tempdir()?;
    let archive_path = if let Some(path) = &args.archive {
        path.clone()
    } else {
        let interface = discovery::interface(args.interface.clone())?;
        let host = discovery::host(&interface, args.host.clone())?;
        eprintln!("Scanning T2 RemoteXPC services on [{host}%{interface}]");
        let mut scan = progress::Bar::new("RemoteXPC scan");
        let port = remote::discover_service(&interface, host, |current, total| {
            scan.set(current, total);
        })?;
        scan.finish();
        eprintln!("Fetching com.apple.sysdiagnose.remote from port {port}");
        let path = work.path().join("sysdiagnose.tar.gz");
        let mut download = progress::Bar::new("Sysdiagnose download");
        remote::fetch_sysdiagnose(&interface, host, port, &path, |current, total| {
            download.set(current, total);
        })?;
        download.finish();
        path
    };

    eprintln!("Extracting {}", archive_path.display());
    let extracted = archive::extract(&archive_path, &work.path().join("extracted"))?;
    eprintln!("Parsing {}", extracted.logarchive.display());
    let mut parsing = progress::Bar::new("Unified log parsing");
    let mut records = unified::parse(&extracted.logarchive, |current, total| {
        parsing.set(current, total);
    })?;
    parsing.finish();
    let unified_count = records.len();
    let historical = historical::parse(&extracted.root)?;
    let historical_count = historical.len();
    records.extend(historical);
    records.sort_by_key(|record| record.timestamp_ns);
    eprintln!(
        "Writing {} BridgeOS records ({unified_count} unified, {historical_count} historical)",
        records.len()
    );
    store::write_atomic(&state_file, &records)?;
    eprintln!("Replaced {}", state_file.display());
    Ok(())
}

fn show(cli: &Cli, state_file: PathBuf) -> Result<()> {
    let boot = cli.boot.unwrap_or(journal::Boot::Offset(0));
    let mut linux = journal::linux_boot(boot)?;
    let mut bridge = store::read(&state_file)?;
    if matches!(boot, journal::Boot::Offset(_)) {
        if let (Some(first), Some(last)) = (linux.first(), linux.last()) {
            bridge.retain(|record| {
                record.timestamp_ns >= first.timestamp_ns
                    && record.timestamp_ns <= last.timestamp_ns
            });
        } else {
            bridge.clear();
        }
    }
    linux.append(&mut bridge);
    linux.sort_by_key(|record| record.timestamp_ns);

    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    for record in linux {
        if !matches_source(&record, cli.source) {
            continue;
        }
        if cli.grep.as_ref().is_some_and(|grep| {
            !grep.is_match(&record.message)
                && !grep.is_match(&record.process)
                && !grep.is_match(&record.subsystem)
                && !grep.is_match(&record.category)
                && !grep.is_match(&record.source)
        }) {
            continue;
        }
        let result: io::Result<()> = match cli.output {
            Output::Text => writeln!(output, "{}", record.text()),
            Output::Jsonl => {
                let mut line = serde_json::to_vec(&record).map_err(io::Error::other)?;
                line.push(b'\n');
                output.write_all(&line)
            }
        };
        if let Err(error) = result {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(error.into());
        }
    }
    Ok(())
}

fn matches_source(record: &Record, source: Option<Source>) -> bool {
    match source {
        None => true,
        Some(Source::Linux) => record.source == "LNX",
        Some(Source::T2) => record.source == "T2",
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let state_file = discovery::state_file(cli.state_file.clone())?;
    if cli.list_boots {
        return journal::list_boots();
    }
    if cli.command.is_none() && !state_file.try_exists()? {
        eprintln!("No BridgeOS snapshot found; performing initial refresh");
        refresh(&Refresh::default(), state_file.clone()).context("automatic refresh failed")?;
    }
    match &cli.command {
        Some(Command::Refresh(args)) => refresh(args, state_file).map(|_| 0),
        None => show(&cli, state_file).map(|_| 0),
    }
}

fn main() {
    match run().context("t2journal failed") {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}
