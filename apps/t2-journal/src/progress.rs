// SPDX-License-Identifier: MIT

use std::io::{self, IsTerminal, Write};

const WIDTH: u64 = 32;

pub struct Bar {
    label: &'static str,
    terminal: bool,
    last_percent: Option<u64>,
    rendered: bool,
    finished: bool,
}

impl Bar {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            terminal: io::stderr().is_terminal(),
            last_percent: None,
            rendered: false,
            finished: false,
        }
    }

    pub fn set(&mut self, current: u64, total: u64) {
        if total == 0 {
            return;
        }
        let percent = current.min(total).saturating_mul(100) / total;
        if self.last_percent == Some(percent) {
            return;
        }
        if !self.terminal
            && percent != 100
            && self
                .last_percent
                .is_some_and(|previous| percent < previous + 10)
        {
            return;
        }
        self.last_percent = Some(percent);
        self.rendered = true;
        if self.terminal {
            let filled = percent * WIDTH / 100;
            let empty = WIDTH - filled;
            eprint!(
                "\r{:<23} [{}{}] {:>3}%",
                self.label,
                "=".repeat(filled as usize),
                " ".repeat(empty as usize),
                percent
            );
            let _ = io::stderr().flush();
        } else {
            eprintln!("{}: {}%", self.label, percent);
        }
    }

    pub fn finish(&mut self) {
        if self.terminal && self.rendered {
            eprintln!();
        }
        self.finished = true;
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        if self.terminal && self.rendered && !self.finished {
            eprintln!();
        }
    }
}
