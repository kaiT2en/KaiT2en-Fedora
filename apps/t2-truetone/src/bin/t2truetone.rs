// SPDX-License-Identifier: GPL-3.0-or-later
//! t2truetone: session daemon and command line.
//!
//!   t2truetone daemon   run the adaptation loop (started by the user unit)
//!   t2truetone status   print the daemon's live state
//!   t2truetone sensor   print raw ambient light samples

use std::sync::mpsc;

use anyhow::Result;
use t2_truetone::{als, daemon, status};

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("daemon") => daemon::run(),
        Some("status") => {
            match status::read() {
                Some(s) => println!("{}", serde_json::to_string_pretty(&s)?),
                None => println!("daemon not running"),
            }
            Ok(())
        }
        Some("sensor") => {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || als::run(vec![tx]));
            for s in rx {
                println!("lux {:9.3}  x {:.5}  y {:.5}  cct {:5.0}", s.lux, s.xy[0], s.xy[1], s.cct);
            }
            Ok(())
        }
        _ => {
            eprintln!("usage: t2truetone daemon|status|sensor");
            std::process::exit(2);
        }
    }
}
