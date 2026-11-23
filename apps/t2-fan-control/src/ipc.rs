// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
    path::{Path, PathBuf},
};

use crate::{
    config::{normalize_curve, AppConfig, CurvePoint, MIN_SOAK_TEMP_C, MAX_SOAK_TEMP_C},
    error::{FanControlError, Result},
    sysfs::{FanEndpoint, TemperatureRole, TemperatureSource},
};

pub const SOCKET_DIR: &str = "/run/t2-fancontrol";
pub const SOCKET_PATH: &str = "/run/t2-fancontrol/daemon.sock";

#[derive(Clone, Debug, Default)]
pub struct FanStatus {
    pub name: String,
    pub current_speed: Option<u32>,
    pub min_speed: u32,
    pub max_speed: u32,
    pub app_controlled: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct SensorChoice { pub key: String, pub name: String }

#[derive(Clone, Debug, Default)]
pub struct DaemonState {
    pub status: String,
    pub control_active: bool,
    pub autostart_enabled: bool,
    pub soak_temp_c: u8,
    pub system_cooling_time_s: u16,
    pub any_sensor_enabled: bool,
    pub any_sensor_temp_c: u8,
    pub custom_curve: Vec<CurvePoint>,
    pub curve_sensor_key: Option<String>,
    pub sensor_choices: Vec<SensorChoice>,
    pub cpu_temp_c: Option<u8>,
    pub gpu_temp_c: Option<u8>,
    pub effective_temp_c: Option<u8>,
    pub hottest_sensor_name: Option<String>,
    pub optional_curve_temp_c: Option<u8>,
    pub overall_hottest_temp_c: Option<u8>,
    pub overall_hottest_sensor_name: Option<String>,
    pub system_temp_c: Option<u8>,
    pub system_sensor_count: usize,
    pub monitored_sensor_count: usize,
    pub heat_soak_cooling: bool,
    pub any_sensor_cooling: bool,
    pub target_percent: Option<u8>,
    pub fans: Vec<FanStatus>,
}

#[derive(Clone, Debug)]
pub enum Request {
    GetState,
    SetActive(bool),
    SetAutostart(bool),
    SetCurve(u8, Vec<CurvePoint>),
    SetAnySensor(bool, u8),
    SetSystemCoolingTime(u16),
    SetCurveSensor(Option<String>),
}

pub fn bind_listener() -> Result<UnixListener> {
    let dir = Path::new(SOCKET_DIR);
    fs::create_dir_all(dir).map_err(|source| FanControlError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).map_err(|source| {
        FanControlError::Io {
            path: dir.to_path_buf(),
            source,
        }
    })?;

    let socket_path = Path::new(SOCKET_PATH);
    if socket_path.exists() {
        fs::remove_file(socket_path).map_err(|source| FanControlError::Io {
            path: socket_path.to_path_buf(),
            source,
        })?;
    }

    let listener = UnixListener::bind(socket_path).map_err(|source| FanControlError::Io {
        path: socket_path.to_path_buf(),
        source,
    })?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666)).map_err(|source| {
        FanControlError::Io {
            path: socket_path.to_path_buf(),
            source,
        }
    })?;
    Ok(listener)
}

pub fn send_request(request: Request) -> Result<DaemonState> {
    let mut stream = UnixStream::connect(SOCKET_PATH).map_err(|source| FanControlError::Io {
        path: PathBuf::from(SOCKET_PATH),
        source,
    })?;
    let payload = encode_request(&request);
    stream
        .write_all(payload.as_bytes())
        .map_err(FanControlError::ProcessSpawn)?;
    stream.flush().map_err(FanControlError::ProcessSpawn)?;
    decode_response(BufReader::new(stream))
}

pub fn handle_request_line(line: &str) -> Result<Request> {
    let line = line.trim();
    if line == "GET_STATE" {
        return Ok(Request::GetState);
    }
    if let Some(value) = line.strip_prefix("SET_ACTIVE ") {
        return Ok(Request::SetActive(parse_bool_flag(value)?));
    }
    if let Some(value) = line.strip_prefix("SET_AUTOSTART ") {
        return Ok(Request::SetAutostart(parse_bool_flag(value)?));
    }
    if let Some(value) = line.strip_prefix("SET_CURVE ") {
        let (soak, points) = value.split_once(' ')
            .ok_or_else(|| protocol_error(String::from("missing system temperature target")))?;
        let soak = soak.parse::<u8>().map_err(|_| protocol_error(format!("invalid system temperature target: {soak}")))?
            .clamp(MIN_SOAK_TEMP_C, MAX_SOAK_TEMP_C);
        let curve = parse_curve(points)?;
        return Ok(Request::SetCurve(soak, curve));
    }
    if let Some(value) = line.strip_prefix("SET_ANY_SENSOR ") {
        let (enabled, temp) = value.split_once(' ')
            .ok_or_else(|| protocol_error(String::from("missing any-sensor temperature")))?;
        let enabled = parse_bool_flag(enabled)?;
        let temp = temp.parse::<u8>().map_err(|_| protocol_error(format!("invalid any-sensor temperature: {temp}")))?.min(100);
        return Ok(Request::SetAnySensor(enabled, temp));
    }
    if let Some(value) = line.strip_prefix("SET_SYSTEM_COOLING_TIME ") {
        let seconds = value.parse::<u16>()
            .map_err(|_| protocol_error(format!("invalid system cooling time: {value}")))?
            .min(600);
        return Ok(Request::SetSystemCoolingTime(seconds));
    }
    if let Some(value) = line.strip_prefix("SET_CURVE_SENSOR ") {
        let value = value.trim();
        return Ok(Request::SetCurveSensor((value != "NONE").then(|| value.to_owned())));
    }

    Err(protocol_error(format!("unknown request: {line}")))
}

pub fn read_request(stream: &UnixStream) -> Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(FanControlError::ProcessSpawn)?;
    handle_request_line(&line)
}

pub fn write_response(mut stream: &UnixStream, state: &DaemonState) -> Result<()> {
    let mut body = String::from("ok=1\n");
    push_field(&mut body, "status", &state.status);
    push_field(&mut body, "control_active", if state.control_active { "1" } else { "0" });
    push_field(
        &mut body,
        "autostart_enabled",
        if state.autostart_enabled { "1" } else { "0" },
    );
    push_field(&mut body, "system_temp_target_c", &state.soak_temp_c.to_string());
    push_field(&mut body, "system_cooling_time_s", &state.system_cooling_time_s.to_string());
    push_field(&mut body, "any_sensor_enabled", if state.any_sensor_enabled { "1" } else { "0" });
    push_field(&mut body, "any_sensor_temp_c", &state.any_sensor_temp_c.to_string());
    push_field(
        &mut body,
        "custom_curve",
        &format_curve(&state.custom_curve),
    );
    push_field(&mut body, "curve_sensor_key", state.curve_sensor_key.as_deref().unwrap_or(""));
    push_field(&mut body, "sensor_count", &state.sensor_choices.len().to_string());
    for (index, sensor) in state.sensor_choices.iter().enumerate() {
        push_field(&mut body, &format!("sensor.{index}.key"), &sensor.key);
        push_field(&mut body, &format!("sensor.{index}.name"), &sensor.name);
    }
    push_option_u8(&mut body, "cpu_temp_c", state.cpu_temp_c);
    push_option_u8(&mut body, "gpu_temp_c", state.gpu_temp_c);
    push_option_u8(&mut body, "effective_temp_c", state.effective_temp_c);
    push_field(&mut body, "hottest_sensor_name", state.hottest_sensor_name.as_deref().unwrap_or(""));
    push_option_u8(&mut body, "optional_curve_temp_c", state.optional_curve_temp_c);
    push_option_u8(&mut body, "overall_hottest_temp_c", state.overall_hottest_temp_c);
    push_field(&mut body, "overall_hottest_sensor_name", state.overall_hottest_sensor_name.as_deref().unwrap_or(""));
    push_option_u8(&mut body, "system_temp_c", state.system_temp_c);
    push_field(&mut body, "system_sensor_count", &state.system_sensor_count.to_string());
    push_field(&mut body, "monitored_sensor_count", &state.monitored_sensor_count.to_string());
    push_field(&mut body, "heat_soak_cooling", if state.heat_soak_cooling { "1" } else { "0" });
    push_field(&mut body, "any_sensor_cooling", if state.any_sensor_cooling { "1" } else { "0" });
    push_option_u8(&mut body, "target_percent", state.target_percent);
    push_field(&mut body, "fan_count", &state.fans.len().to_string());
    for (index, fan) in state.fans.iter().enumerate() {
        push_field(&mut body, &format!("fan.{index}.name"), &fan.name);
        push_option_u32(&mut body, &format!("fan.{index}.current_speed"), fan.current_speed);
        push_field(
            &mut body,
            &format!("fan.{index}.min_speed"),
            &fan.min_speed.to_string(),
        );
        push_field(
            &mut body,
            &format!("fan.{index}.max_speed"),
            &fan.max_speed.to_string(),
        );
        push_field(
            &mut body,
            &format!("fan.{index}.app_controlled"),
            match fan.app_controlled {
                Some(true) => "1",
                Some(false) => "0",
                None => "",
            },
        );
    }
    body.push('\n');
    stream
        .write_all(body.as_bytes())
        .map_err(FanControlError::ProcessSpawn)?;
    stream.flush().map_err(FanControlError::ProcessSpawn)
}

pub fn write_error(mut stream: &UnixStream, message: &str) -> Result<()> {
    let mut body = String::from("ok=0\n");
    push_field(&mut body, "error", message);
    body.push('\n');
    stream
        .write_all(body.as_bytes())
        .map_err(FanControlError::ProcessSpawn)?;
    stream.flush().map_err(FanControlError::ProcessSpawn)
}

pub fn state_from(config: &AppConfig, status: String, temps: (Option<u8>, Option<u8>, Option<u8>, Option<String>, Option<u8>, Option<u8>, Option<String>, Option<u8>, usize, usize, bool, bool), target_percent: Option<u8>, fans: &[FanEndpoint], temperatures: &[TemperatureSource]) -> DaemonState {
    let mut sensor_choices = temperatures.iter()
        .filter(|source| source.role == TemperatureRole::System && source.last_temp_c.is_some_and(|temp| temp > 0))
        .map(|source| SensorChoice { key: source.key.clone(), name: source.name.clone() })
        .collect::<Vec<_>>();
    sensor_choices.sort_by_key(|sensor| sensor.name.to_lowercase());

    DaemonState {
        status,
        control_active: config.automatic_control_enabled,
        autostart_enabled: config.autostart_enabled,
        soak_temp_c: config.soak_temp_c,
        system_cooling_time_s: config.system_cooling_time_s,
        any_sensor_enabled: config.any_sensor_enabled,
        any_sensor_temp_c: config.any_sensor_temp_c,
        custom_curve: config.custom_curve.clone(),
        curve_sensor_key: config.curve_sensor_key.clone(),
        sensor_choices,
        cpu_temp_c: temps.0,
        gpu_temp_c: temps.1,
        effective_temp_c: temps.2,
        hottest_sensor_name: temps.3,
        optional_curve_temp_c: temps.4,
        overall_hottest_temp_c: temps.5,
        overall_hottest_sensor_name: temps.6,
        system_temp_c: temps.7,
        system_sensor_count: temps.8,
        monitored_sensor_count: temps.9,
        heat_soak_cooling: temps.10,
        any_sensor_cooling: temps.11,
        target_percent,
        fans: fans
            .iter()
            .map(|fan| FanStatus {
                name: fan.name.clone(),
                current_speed: fan.current_speed,
                min_speed: fan.min_speed,
                max_speed: fan.max_speed,
                app_controlled: fan.app_controlled,
            })
            .collect(),
    }
}

fn decode_response<R: BufRead>(mut reader: R) -> Result<DaemonState> {
    let mut line = String::new();
    let mut state = DaemonState::default();
    let mut fan_entries: Vec<FanStatus> = Vec::new();
    let mut sensor_entries: Vec<SensorChoice> = Vec::new();
    let mut ok = true;
    let mut error_message = None;

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).map_err(FanControlError::ProcessSpawn)?;
        if bytes == 0 || line.trim().is_empty() {
            break;
        }
        let (key, value) = line
            .trim_end()
            .split_once('=')
            .ok_or_else(|| protocol_error(format!("invalid response line: {}", line.trim_end())))?;
        match key {
            "ok" => {
                ok = value == "1";
            }
            "error" => error_message = Some(unescape(value)),
            "status" => state.status = unescape(value),
            "control_active" => state.control_active = parse_bool_flag(value)?,
            "autostart_enabled" => state.autostart_enabled = parse_bool_flag(value)?,
            "system_temp_target_c" | "system_temp_limit_c" | "soak_temp_c" => state.soak_temp_c = value.parse::<u8>()
                .map_err(|_| protocol_error(format!("invalid system temperature target: {value}")))?,
            "system_cooling_time_s" => state.system_cooling_time_s = value.parse::<u16>()
                .map_err(|_| protocol_error(format!("invalid system cooling time: {value}")))?,
            "any_sensor_enabled" => state.any_sensor_enabled = parse_bool_flag(value)?,
            "any_sensor_temp_c" => state.any_sensor_temp_c = value.parse::<u8>().map_err(|_| protocol_error(format!("invalid any-sensor temperature: {value}")))?,
            "custom_curve" => state.custom_curve = parse_curve(value)?,
            "curve_sensor_key" => state.curve_sensor_key = (!value.is_empty()).then(|| unescape(value)),
            "sensor_count" => sensor_entries.resize(value.parse::<usize>().map_err(|_| protocol_error(format!("invalid sensor_count: {value}")))?, SensorChoice::default()),
            "cpu_temp_c" => state.cpu_temp_c = parse_option_u8(value)?,
            "gpu_temp_c" => state.gpu_temp_c = parse_option_u8(value)?,
            "effective_temp_c" => state.effective_temp_c = parse_option_u8(value)?,
            "hottest_sensor_name" => state.hottest_sensor_name = (!value.is_empty()).then(|| unescape(value)),
            "optional_curve_temp_c" => state.optional_curve_temp_c = parse_option_u8(value)?,
            "overall_hottest_temp_c" => state.overall_hottest_temp_c = parse_option_u8(value)?,
            "overall_hottest_sensor_name" => state.overall_hottest_sensor_name = (!value.is_empty()).then(|| unescape(value)),
            "system_temp_c" => state.system_temp_c = parse_option_u8(value)?,
            "system_sensor_count" => state.system_sensor_count = value.parse::<usize>().map_err(|_| protocol_error(format!("invalid sensor count: {value}")))?,
            "monitored_sensor_count" => state.monitored_sensor_count = value.parse::<usize>().map_err(|_| protocol_error(format!("invalid monitored sensor count: {value}")))?,
            "heat_soak_cooling" => state.heat_soak_cooling = parse_bool_flag(value)?,
            "any_sensor_cooling" => state.any_sensor_cooling = parse_bool_flag(value)?,
            "target_percent" => state.target_percent = parse_option_u8(value)?,
            "fan_count" => {
                let count = value.parse::<usize>().map_err(|_| {
                    protocol_error(format!("invalid fan_count in response: {value}"))
                })?;
                fan_entries.resize(count, FanStatus::default());
            }
            _ if key.starts_with("fan.") => apply_fan_field(key, value, &mut fan_entries)?,
            _ if key.starts_with("sensor.") => apply_sensor_field(key, value, &mut sensor_entries)?,
            _ => {}
        }
    }

    if !ok {
        return Err(protocol_error(
            error_message.unwrap_or_else(|| String::from("daemon returned an error")),
        ));
    }
    state.fans = fan_entries;
    state.sensor_choices = sensor_entries;
    Ok(state)
}

fn apply_sensor_field(key: &str, value: &str, sensors: &mut [SensorChoice]) -> Result<()> {
    let mut parts = key.split('.');
    let _ = parts.next();
    let index = parts.next().ok_or_else(|| protocol_error(format!("invalid sensor field: {key}")))?
        .parse::<usize>().map_err(|_| protocol_error(format!("invalid sensor index: {key}")))?;
    let field = parts.next().ok_or_else(|| protocol_error(format!("invalid sensor field: {key}")))?;
    let sensor = sensors.get_mut(index).ok_or_else(|| protocol_error(format!("sensor index out of range: {index}")))?;
    match field {
        "key" => sensor.key = unescape(value),
        "name" => sensor.name = unescape(value),
        _ => {}
    }
    Ok(())
}

fn apply_fan_field(key: &str, value: &str, fan_entries: &mut [FanStatus]) -> Result<()> {
    let mut parts = key.split('.');
    let _ = parts.next();
    let index = parts
        .next()
        .ok_or_else(|| protocol_error(format!("invalid fan field: {key}")))?
        .parse::<usize>()
        .map_err(|_| protocol_error(format!("invalid fan index in field: {key}")))?;
    let field = parts
        .next()
        .ok_or_else(|| protocol_error(format!("invalid fan field: {key}")))?;
    let fan = fan_entries
        .get_mut(index)
        .ok_or_else(|| protocol_error(format!("fan index out of range: {index}")))?;

    match field {
        "name" => fan.name = unescape(value),
        "current_speed" => fan.current_speed = parse_option_u32(value)?,
        "min_speed" => {
            fan.min_speed = value.parse::<u32>().map_err(|_| {
                protocol_error(format!("invalid min_speed in response: {value}"))
            })?
        }
        "max_speed" => {
            fan.max_speed = value.parse::<u32>().map_err(|_| {
                protocol_error(format!("invalid max_speed in response: {value}"))
            })?
        }
        "app_controlled" => {
            fan.app_controlled = if value.is_empty() {
                None
            } else {
                Some(parse_bool_flag(value)?)
            }
        }
        _ => {}
    }

    Ok(())
}

fn encode_request(request: &Request) -> String {
    match request {
        Request::GetState => String::from("GET_STATE\n"),
        Request::SetActive(enabled) => format!("SET_ACTIVE {}\n", bool_flag(*enabled)),
        Request::SetAutostart(enabled) => format!("SET_AUTOSTART {}\n", bool_flag(*enabled)),
        Request::SetCurve(soak, curve) => format!("SET_CURVE {soak} {}\n", format_curve(curve)),
        Request::SetAnySensor(enabled, temp) => format!("SET_ANY_SENSOR {} {temp}\n", bool_flag(*enabled)),
        Request::SetSystemCoolingTime(seconds) => format!("SET_SYSTEM_COOLING_TIME {seconds}\n"),
        Request::SetCurveSensor(key) => format!("SET_CURVE_SENSOR {}\n", key.as_deref().unwrap_or("NONE")),
    }
}

fn parse_curve(value: &str) -> Result<Vec<CurvePoint>> {
    let mut curve = Vec::new();
    for entry in value.split(',').filter(|entry| !entry.trim().is_empty()) {
        let (temp, speed) = entry
            .split_once(':')
            .ok_or_else(|| protocol_error(format!("invalid curve entry: {entry}")))?;
        curve.push(CurvePoint {
            temp_c: temp
                .trim()
                .parse()
                .map_err(|_| protocol_error(format!("invalid temperature in curve: {entry}")))?,
            speed_percent: speed
                .trim()
                .parse()
                .map_err(|_| protocol_error(format!("invalid speed in curve: {entry}")))?,
        });
    }
    normalize_curve(&mut curve);
    Ok(curve)
}

fn format_curve(curve: &[CurvePoint]) -> String {
    curve
        .iter()
        .map(|point| format!("{}:{}", point.temp_c, point.speed_percent))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_bool_flag(value: &str) -> Result<bool> {
    match value.trim() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(protocol_error(format!("invalid boolean flag: {other}"))),
    }
}

fn parse_option_u8(value: &str) -> Result<Option<u8>> {
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse::<u8>()
            .map(Some)
            .map_err(|_| protocol_error(format!("invalid u8 value: {value}")))
    }
}

fn parse_option_u32(value: &str) -> Result<Option<u32>> {
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse::<u32>()
            .map(Some)
            .map_err(|_| protocol_error(format!("invalid u32 value: {value}")))
    }
}

fn push_field(buffer: &mut String, key: &str, value: &str) {
    buffer.push_str(key);
    buffer.push('=');
    buffer.push_str(&escape(value));
    buffer.push('\n');
}

fn push_option_u8(buffer: &mut String, key: &str, value: Option<u8>) {
    push_field(buffer, key, &value.map(|value| value.to_string()).unwrap_or_default());
}

fn push_option_u32(buffer: &mut String, key: &str, value: Option<u32>) {
    push_field(buffer, key, &value.map(|value| value.to_string()).unwrap_or_default());
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('=', "\\=")
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    'n' => out.push('\n'),
                    '=' => out.push('='),
                    '\\' => out.push('\\'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn bool_flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn protocol_error(message: String) -> FanControlError {
    FanControlError::Protocol(message)
}
