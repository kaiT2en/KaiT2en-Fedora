use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::{fd::AsRawFd, unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}}},
    path::{Path, PathBuf},
};

use crate::{
    config::{ActionKind, AppConfig, MAX_FORCE_CLICK_PERCENT, MIN_FORCE_CLICK_PERCENT},
    error::{ForceClickError, Result},
};

pub const SOCKET_DIR: &str = "/run/t2-force-click";
pub const SOCKET_PATH: &str = "/run/t2-force-click/daemon.sock";

#[derive(Clone, Debug, Default)]
pub struct DaemonState {
    pub driver_loaded: bool,
    pub device_found: bool,
    pub click_strength: u8,
    pub force_click_threshold_percent: u32,
    pub force_click_disabled: bool,
    pub action_kind: String,
    pub key_combo: String,
    pub command: String,
}

#[derive(Clone, Debug)]
pub enum Request {
    GetState,
    SetConfig(AppConfig),
}

pub fn bind_listener() -> Result<UnixListener> {
    let dir = Path::new(SOCKET_DIR);
    fs::create_dir_all(dir).map_err(|source| ForceClickError::Io { path: dir.to_path_buf(), source })?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755))
        .map_err(|source| ForceClickError::Io { path: dir.to_path_buf(), source })?;

    let socket_path = Path::new(SOCKET_PATH);
    if socket_path.exists() {
        fs::remove_file(socket_path)
            .map_err(|source| ForceClickError::Io { path: socket_path.to_path_buf(), source })?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|source| ForceClickError::Io { path: socket_path.to_path_buf(), source })?;
    /*
     * The daemon authenticates every connection with SO_PEERCRED.  The
     * socket must remain connectable by the graphical user, so filesystem
     * mode alone cannot be the authorization mechanism here.
     */
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666))
        .map_err(|source| ForceClickError::Io { path: socket_path.to_path_buf(), source })?;
    Ok(listener)
}

pub fn send_request(request: Request) -> Result<DaemonState> {
    let mut stream = UnixStream::connect(SOCKET_PATH)
        .map_err(|source| ForceClickError::Io { path: PathBuf::from(SOCKET_PATH), source })?;
    let payload = encode_request(&request);
    stream.write_all(payload.as_bytes()).map_err(ForceClickError::ProcessSpawn)?;
    stream.flush().map_err(ForceClickError::ProcessSpawn)?;
    decode_response(BufReader::new(stream))
}

fn encode_request(request: &Request) -> String {
    match request {
        Request::GetState => "GET_STATE\n".to_owned(),
        Request::SetConfig(config) => format!(
            "SET_CONFIG {} {} {} {} {} {}\n",
            config.click_strength,
            config.force_click_threshold_percent,
            config.force_click_disabled,
            config.action_kind.as_str(),
            urlencode(&config.key_combo),
            urlencode(&config.command),
        ),
    }
}

pub fn handle_request_line(line: &str) -> Result<Request> {
    let line = line.trim();
    if line == "GET_STATE" {
        return Ok(Request::GetState);
    }
    if let Some(value) = line.strip_prefix("SET_CONFIG ") {
        let mut fields = value.splitn(6, ' ');
        let click_strength = fields
            .next()
            .and_then(|v| v.parse::<u8>().ok())
            .ok_or_else(|| protocol_error("missing click_strength".to_owned()))?
            .min(2);
        let force_click_threshold_percent = fields
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| protocol_error("missing force_click_threshold_percent".to_owned()))?
            .clamp(MIN_FORCE_CLICK_PERCENT, MAX_FORCE_CLICK_PERCENT);
        let force_click_disabled = fields
            .next()
            .ok_or_else(|| protocol_error("missing force_click_disabled".to_owned()))
            .and_then(parse_bool_flag)?;
        let action_kind = ActionKind::from_str(
            fields.next().ok_or_else(|| protocol_error("missing action_kind".to_owned()))?,
        );
        let key_combo = urldecode(fields.next().unwrap_or(""));
        let command = urldecode(fields.next().unwrap_or(""));
        return Ok(Request::SetConfig(AppConfig {
            click_strength,
            force_click_threshold_percent,
            force_click_disabled,
            action_kind,
            key_combo,
            command,
        }));
    }
    Err(protocol_error(format!("unknown request: {line}")))
}

pub fn read_request(stream: &UnixStream) -> Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(ForceClickError::ProcessSpawn)?;
    handle_request_line(&line)
}

/// Return the uid the kernel attached to this local Unix-socket connection.
pub fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` and `length` point to initialized writable
    // storage of exactly the type and length requested by SO_PEERCRED.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 || length != std::mem::size_of::<libc::ucred>() as libc::socklen_t {
        return Err(protocol_error("could not obtain peer credentials".to_owned()));
    }
    Ok(credentials.uid)
}

pub fn write_response(mut stream: &UnixStream, state: &DaemonState) -> Result<()> {
    let mut body = String::from("ok=1\n");
    push_field(&mut body, "driver_loaded", if state.driver_loaded { "1" } else { "0" });
    push_field(&mut body, "device_found", if state.device_found { "1" } else { "0" });
    push_field(&mut body, "click_strength", &state.click_strength.to_string());
    push_field(&mut body, "force_click_threshold_percent", &state.force_click_threshold_percent.to_string());
    push_field(&mut body, "force_click_disabled", if state.force_click_disabled { "1" } else { "0" });
    push_field(&mut body, "action_kind", &state.action_kind);
    push_field(&mut body, "key_combo", &state.key_combo);
    push_field(&mut body, "command", &state.command);
    body.push('\n');
    stream.write_all(body.as_bytes()).map_err(ForceClickError::ProcessSpawn)?;
    stream.flush().map_err(ForceClickError::ProcessSpawn)
}

pub fn write_error(mut stream: &UnixStream, message: &str) -> Result<()> {
    let mut body = String::from("ok=0\n");
    push_field(&mut body, "error", message);
    body.push('\n');
    stream.write_all(body.as_bytes()).map_err(ForceClickError::ProcessSpawn)?;
    stream.flush().map_err(ForceClickError::ProcessSpawn)
}

fn decode_response(reader: BufReader<UnixStream>) -> Result<DaemonState> {
    let mut ok = false;
    let mut state = DaemonState::default();
    let mut error_message = String::new();
    for line in reader.lines() {
        let line = line.map_err(ForceClickError::ProcessSpawn)?;
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "ok" => ok = value == "1",
            "error" => error_message = value.to_owned(),
            "driver_loaded" => state.driver_loaded = value == "1",
            "device_found" => state.device_found = value == "1",
            "click_strength" => state.click_strength = value.parse().unwrap_or_default(),
            "force_click_threshold_percent" => {
                state.force_click_threshold_percent = value.parse().unwrap_or_default()
            }
            "force_click_disabled" => state.force_click_disabled = value == "1",
            "action_kind" => state.action_kind = value.to_owned(),
            "key_combo" => state.key_combo = value.to_owned(),
            "command" => state.command = value.to_owned(),
            _ => {}
        }
    }
    if !ok {
        return Err(protocol_error(error_message));
    }
    Ok(state)
}

fn push_field(body: &mut String, key: &str, value: &str) {
    body.push_str(key);
    body.push('=');
    body.push_str(value);
    body.push('\n');
}

fn parse_bool_flag(value: &str) -> Result<bool> {
    value
        .trim()
        .parse::<bool>()
        .map_err(|_| protocol_error(format!("invalid boolean: {value}")))
}

fn protocol_error(message: String) -> ForceClickError {
    ForceClickError::Protocol(message)
}

/// The config wire format is space-separated fields on one line.
fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    if out.is_empty() {
        out.push_str("%00EMPTY");
    }
    out
}

fn urldecode(value: &str) -> String {
    if value == "%00EMPTY" {
        return String::new();
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_round_trips() {
        let value = "echo hi && notify-send \"Force Click\"";
        assert_eq!(urldecode(&urlencode(value)), value);
    }

    #[test]
    fn empty_value_round_trips() {
        assert_eq!(urldecode(&urlencode("")), "");
    }
}
