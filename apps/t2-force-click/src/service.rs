use std::process::Command;

use crate::error::{ForceClickError, Result};

#[derive(Clone, Debug)]
pub struct ActiveSession {
    pub uid: u32,
    pub user: String,
}

/// Finds the active local graphical session for daemon authorization.
pub fn active_session() -> Result<ActiveSession> {
    let sessions = Command::new("loginctl")
        .args(["list-sessions", "--no-legend", "--no-pager"])
        .output()
        .map_err(ForceClickError::ProcessSpawn)?;
    if !sessions.status.success() {
        return Err(command_failed("loginctl list-sessions", &sessions));
    }

    for line in String::from_utf8_lossy(&sessions.stdout).lines() {
        let Some(id) = line.split_whitespace().next() else { continue };
        if loginctl_value(id, "Active")? != "yes" {
            continue;
        }
        let uid = loginctl_value(id, "User").and_then(|value| {
            value
                .parse::<u32>()
                .map_err(|_| ForceClickError::Protocol("invalid active-session uid".to_owned()))
        })?;
        let user = loginctl_value(id, "Name")?;
        if !user.is_empty() {
            return Ok(ActiveSession { uid, user });
        }
    }
    Err(ForceClickError::Protocol("no active local user session".to_owned()))
}

fn loginctl_value(session: &str, property: &str) -> Result<String> {
    let output = Command::new("loginctl")
        .args(["show-session", session, "--property", property, "--value"])
        .output()
        .map_err(ForceClickError::ProcessSpawn)?;
    if !output.status.success() {
        return Err(command_failed("loginctl show-session", &output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn command_failed(command: &str, output: &std::process::Output) -> ForceClickError {
    ForceClickError::CommandFailed {
        command: command.to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}
