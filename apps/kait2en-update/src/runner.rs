// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! Background work and the privileged installer run.

use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::mpsc::{self, Sender, TryRecvError},
    thread,
    time::Duration,
};

use gtk4::glib;

use crate::{git::Repository, optional::Part};

const HELPER: &str = "/usr/local/libexec/kait2en-update-helper";

/// Runs work on a thread and hands the result to the main loop.
pub fn background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(60), move || match receiver.try_recv() {
        Ok(value) => {
            if let Some(done) = done.take() {
                done(value);
            }
            glib::ControlFlow::Break
        }
        Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

pub enum Event {
    Line(String),
    /// The helper ended, Ok when the installer reported no error.
    Finished(Result<(), String>),
}

/// What is installed or removed.
pub enum Selection {
    All,
    Only { steps: Vec<String>, apps: Vec<String> },
    Optional { part: &'static Part, install: bool },
}

impl Selection {
    fn command(&self) -> Vec<String> {
        let helper = |args: &[&str]| {
            ["pkexec", HELPER].iter().chain(args).map(|arg| arg.to_string()).collect()
        };
        match self {
            Self::All => helper(&["apply", "--all"]),
            Self::Only { steps, apps } => helper(&["apply", &steps.join(","), &apps.join(",")]),
            Self::Optional { part, install } if part.privileged => {
                let name = part.name.trim_start_matches("app:");
                helper(&["optional", name, if *install { "install" } else { "remove" }])
            }
            // A user part runs in the session of the user, without the helper.
            Self::Optional { part, install } => {
                let script = Repository::system().path().join("scripts/fedora").join(part.name);
                let mut command = vec!["bash".to_owned(), script.display().to_string()];
                if !install {
                    command.push("--remove".to_owned());
                }
                command
            }
        }
    }
}

fn forward(stream: impl Read + Send + 'static, sender: Sender<Event>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if sender.send(Event::Line(line)).is_err() {
                break;
            }
        }
    })
}

/// Runs the installer for a selection and streams its output line by line.
pub fn apply(selection: &Selection, mut on_event: impl FnMut(Event) + 'static) {
    let (sender, receiver) = mpsc::channel();
    let command = selection.command();
    thread::spawn(move || {
        let child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                let _ = sender.send(Event::Finished(Err(format!("Could not start {}: {error}", command[0]))));
                return;
            }
        };
        let readers = [
            child.stdout.take().map(|out| forward(out, sender.clone())),
            child.stderr.take().map(|err| forward(err, sender.clone())),
        ];
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        let result = match child.wait() {
            Ok(status) if status.success() => Ok(()),
            // pkexec answers 126 when authorization was dismissed.
            Ok(status) if status.code() == Some(126) => Err("Authorization was cancelled.".into()),
            Ok(status) => Err(format!("The installer reported errors ({status}).")),
            Err(error) => Err(format!("Could not wait for the installer: {error}")),
        };
        let _ = sender.send(Event::Finished(result));
    });
    glib::timeout_add_local(Duration::from_millis(60), move || loop {
        match receiver.try_recv() {
            Ok(Event::Finished(result)) => {
                on_event(Event::Finished(result));
                return glib::ControlFlow::Break;
            }
            Ok(event) => on_event(event),
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        }
    });
}
