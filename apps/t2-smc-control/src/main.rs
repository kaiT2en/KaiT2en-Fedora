use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

#[allow(unused_imports)]
use adw::prelude::*;
use glib::timeout_add_local;
use gtk4::{gio, glib};

const APP_ID: &str = "org.t2smccontrol.gtk";
const APP_VERSION: &str = "0.02";
const HWMON_NAMES: &[&str] = &["t2smc", "macsmc"];
const RTC_NAME_PREFIX: &str = "t2smc ";
const HWCLOCK_PATH: &str = "/usr/sbin/hwclock";

fn kait2en_brand() -> gtk4::DrawingArea {
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let brand = gtk4::DrawingArea::new();
    brand.set_content_width(80); brand.set_content_height(21); brand.set_size_request(80, 21);
    brand.set_draw_func(move |_area, context, width, height| {
        let scale = f64::min(width as f64 / pixbuf.width() as f64, height as f64 / pixbuf.height() as f64);
        let _ = context.save();
        context.translate((width as f64 - pixbuf.width() as f64 * scale) / 2.0, (height as f64 - pixbuf.height() as f64 * scale) / 2.0);
        context.scale(scale, scale); context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = context.paint(); let _ = context.restore();
    });
    brand
}

fn palette_css(dark: bool) -> String {
    let (window_bg, window_fg, box_bg) = if dark {
        ("#161616", "#e8e8e8", "#101010")
    } else {
        ("#f2f2f2", "#242424", "#e8e8e8")
    };
    format!(
         "window, popover, .smc-root, .smc-root headerbar {{
             background: {window_bg};
             color: alpha({window_fg}, 0.72);
             font-family: 'JetBrains Mono';
             font-size: 11pt;
             font-weight: 400;
         }}
         button, button label, entry, spinbutton, dropdown {{ color: alpha({window_fg}, 0.72); font-size: 11pt; font-weight: 400; }}
         .title-1, .title-2, .title-3, .title-4, .title, .heading, windowtitle .title {{ color: {window_fg}; font-size: 11pt; font-weight: 400; }}
         .dim-label, windowtitle .subtitle {{ color: alpha({window_fg}, 0.72); font-size: 11pt; font-weight: 400; }}
         headerbar {{ background: @headerbar_bg_color; }}
         .smc-panel, .boxed-list {{
             background: {box_bg};
             color: alpha({window_fg}, 0.72);
             border: none;
             border-radius: 12px;
             box-shadow: none;
         }}
         .smc-panel > border {{ border: none; }}
         .boxed-list row {{ background: {box_bg}; color: alpha({window_fg}, 0.72); }}
         .overview-value {{ font-size: 11pt; font-weight: 400; font-feature-settings: 'tnum'; }}
         .overview-status {{ font-size: 11pt; }}
         scale.overview-meter,
         progressbar.overview-meter {{ min-height: 24px; }}
         scale.overview-meter trough,
         progressbar.overview-meter trough {{ min-height: 10px; border-radius: 5px; }}
         progressbar.overview-meter trough {{ margin-top: 6px; }}
         progressbar.overview-meter progress {{ min-height: 10px; border-radius: 5px; }}
         scale.overview-meter slider {{ min-width: 18px; min-height: 18px; }}
         .donate-link, .donate-link > label {{ color: alpha({window_fg}, 0.72); font-size: 11pt; font-weight: 400; }}"
    )
}

fn install_palette() {
    let provider = gtk4::CssProvider::new();
    let style = adw::StyleManager::default();
    provider.load_from_data(&palette_css(style.is_dark()));
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    style.connect_dark_notify(move |manager| {
        provider.load_from_data(&palette_css(manager.is_dark()));
    });
}

fn cpu_core_label(key: &str) -> Option<String> {
    let bytes = key.as_bytes();
    if bytes.len() != 4 || bytes[0] != b'T' || bytes[1] != b'C' || bytes[3] != b'C' {
        return None;
    }

    let smc_index = (bytes[2] as char).to_digit(10)? as usize;
    if !(1..=8).contains(&smc_index) {
        return None;
    }

    Some(format!("CPU Core {smc_index}"))
}

fn memory_label(key: &str) -> Option<String> {
    match key {
        "TM0P" => Some("Mem Bank A1".into()),
        "TM1P" => Some("Mem Bank A2".into()),
        "TM2P" => Some("Mem Bank A3".into()),
        "TM3P" => Some("Mem Bank A4".into()),
        "TM8P" => Some("Mem Bank B1".into()),
        "TM9P" => Some("Mem Bank B2".into()),
        "TM0S" => Some("Mem Module A1".into()),
        "TM1S" => Some("Mem Module A2".into()),
        "TM2S" => Some("Mem Module A3".into()),
        "TM3S" => Some("Mem Module A4".into()),
        "TM8S" => Some("Mem Module B1".into()),
        "TM9S" => Some("Mem Module B2".into()),
        _ => None,
    }
}

fn heatpipe_label(key: &str) -> Option<String> {
    match key {
        "Th0H" => Some("CPU Heatpipe".into()),
        "Th1H" => Some("Right Fin Stack".into()),
        "Th2H" => Some("Left Fin Stack".into()),
        _ => None,
    }
}

fn sensor_label(key: &str) -> String {
    if let Some(label) = cpu_core_label(key) {
        return label;
    }
    if let Some(label) = memory_label(key) {
        return label;
    }
    if let Some(label) = heatpipe_label(key) {
        return label;
    }

    match key {
        "TA0V" => "Ambient".into(),
        "TA0P" => "Airflow 1".into(),
        "TA1P" => "Airflow 2".into(),
        "TA0S" => "PCI Slot 1 Pos 1".into(),
        "TA1S" => "PCI Slot 1 Pos 2".into(),
        "TA2S" => "PCI Slot 2 Pos 1".into(),
        "TA3S" => "PCI Slot 2 Pos 2".into(),
        "TB0T" => "Battery TS_MAX".into(),
        "TB1T" => "Battery 1".into(),
        "TB2T" => "Battery 2".into(),
        "TB3T" => "Battery".into(),
        "Tb0P" => "BLC Proximity".into(),
        "TC0P" => "CPU 1 Proximity".into(),
        "TC0H" => "CPU 1 Heatsink".into(),
        "TC0D" => "CPU 1 Diode".into(),
        "TC0E" => "CPU 1 Diode Virtual".into(),
        "TC0F" => "CPU 1 Diode Filtered".into(),
        "TCAH" => "CPU 1 Heatsink Alt.".into(),
        "TCAD" => "CPU 1 Package".into(),
        "TC1P" => "CPU 2 Proximity".into(),
        "TC1H" => "CPU 2 Heatsink".into(),
        "TC1D" => "CPU 2 Package".into(),
        "TC1E" => "CPU 2 Diode Virtual".into(),
        "TC1F" => "CPU 2 Diode Filtered".into(),
        "TCBH" => "CPU 2 Heatsink Alt".into(),
        "TCBD" => "CPU 2 Package Alt".into(),
        "TCGC" => "GPU Intel Graphics".into(),
        "TCMX" => "CPU Memory".into(),
        "TCSC" | "TCSc" | "TCSA" => "PECI SA".into(),
        "TCXC" | "TCXc" => "PECI CPU".into(),
        "TG0P" => "GPU AMD Proximity".into(),
        "TG0D" | "TG1D" => "GPU AMD Die".into(),
        "TG0H" | "TG1H" => "GPU AMD Heatsink".into(),
        "TGDD" => "GPU AMD Die digital".into(),
        "TGDF" => "GPU Die analog".into(),
        "TGVP" => "GPU VR".into(),
        "TH0F" => "SSD Heatsink".into(),
        "TH0X" => "SSD Controller".into(),
        "TH0a" => "SSD NAND".into(),
        "TH0b" => "SSD NAND 2".into(),
        "TH1a" => "Drive 1 Raw A".into(),
        "TH1b" => "Drive 1 Raw B".into(),
        "Tm0P" => "Mainboard".into(),
        "Tm1P" => "Mainboard Bottom".into(),
        "TN0D" => "Northbridge Diode".into(),
        "TN0P" => "Northbridge 1".into(),
        "TN1P" => "Northbridge 2".into(),
        "TN0C" => "MCH Diode".into(),
        "TN0H" => "MCH Heatsink".into(),
        "TP0D" => "PCH Diode".into(),
        "TPCD" => "PCH Diode".into(),
        "TP0P" => "PCH Proximity".into(),
        "Tp0P" => "Powerboard".into(),
        "Tp0C" => "Power Supply 1 Alt.".into(),
        "Tp1P" => "Power Supply 2".into(),
        "Tp1C" => "Power Supply 2 Alt.".into(),
        "Tp2P" => "Power Supply 3".into(),
        "Tp3P" => "Power Supply 4".into(),
        "Tp4P" => "Power Supply 5".into(),
        "Tp5P" => "Power Supply 6".into(),
        "TL0P" => "LCD".into(),
        "TH0P" => "HDD Bay 1".into(),
        "TH1P" => "HDD Bay 2".into(),
        "TH2P" => "HDD Bay 3".into(),
        "TH3P" => "HDD Bay 4".into(),
        "TO0P" => "Optical Drive".into(),
        "TS0C" => "Expansion Slots".into(),
        "TTLD" => "Thunderbolt L".into(),
        "TTRD" => "Thunderbolt R".into(),
        "TW0P" => "Airport".into(),
        "TaLC" => "Audio L".into(),
        "TaRC" => "Audio R".into(),
        "Ts0P" => "Palmrest L".into(),
        "Ts0S" => "Palmrest L skin".into(),
        "Ts1P" => "Palmrest R".into(),
        "Ts1S" => "Palmrest R skin".into(),
        "Ts2S" => "Touchpad".into(),
        _ => format!("unknown ({key})"),
    }
}

fn find_hwmon_in(base: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(base).ok()?.flatten() {
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with("hwmon") {
            candidates.push(entry.path());
        }
    }
    candidates.sort();

    for path in candidates {
        let Ok(name) = fs::read_to_string(path.join("name")) else {
            continue;
        };
        if HWMON_NAMES.contains(&name.trim()) {
            return Some(path);
        }
    }
    None
}

fn find_hwmon() -> Option<PathBuf> {
    find_hwmon_in(Path::new("/sys/class/hwmon"))
}

fn find_t2smc_rtc() -> Option<PathBuf> {
    for entry in glob::glob("/sys/class/rtc/rtc*/name").ok()? {
        let path = entry.ok()?;
        let name = fs::read_to_string(&path).ok()?;
        if name.trim().starts_with(RTC_NAME_PREFIX) {
            return path.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn sync_rtc_from_system(rtc: &Path) -> Result<(), String> {
    let device = rtc
        .file_name()
        .map(|name| format!("/dev/{}", name.to_string_lossy()))
        .ok_or_else(|| "Cannot derive the RTC device node".to_string())?;

    let output = Command::new("pkexec")
        .args([HWCLOCK_PATH, "--rtc", &device, "--systohc"])
        .output()
        .map_err(|err| format!("Cannot start pkexec: {err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        Err("Writing the hardware clock was not authorized".into())
    } else {
        Err(stderr)
    }
}

fn read_rtc_datetime(rtc: &Path) -> Option<String> {
    let date = fs::read_to_string(rtc.join("date")).ok()?;
    let time = fs::read_to_string(rtc.join("time")).ok()?;

    Some(format!("{} {}", date.trim(), time.trim()))
}

#[derive(Clone, Debug, Default)]
struct BatteryOverview {
    capacity_percent: Option<u8>,
    current_ua: Option<i64>,
    charge_now_uah: Option<i64>,
    charge_full_uah: Option<i64>,
    adapter_power_uw: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatteryFlow {
    Charging,
    Discharging,
    Holding,
}

#[derive(Default)]
struct BatteryCurrentAverage {
    flow: Option<BatteryFlow>,
    adapter_online: Option<bool>,
    samples: VecDeque<i64>,
}

impl BatteryCurrentAverage {
    fn update(&mut self, battery: &BatteryOverview) -> Option<i64> {
        const MIN_MEANINGFUL_CURRENT_UA: i64 = 50_000;
        const MIN_SAMPLES: usize = 5;
        const MAX_SAMPLES: usize = 30;
        let current = battery.current_ua.unwrap_or(0);
        let adapter_online = battery.adapter_power_uw.is_some_and(|power| power > 0);
        let flow = if current >= MIN_MEANINGFUL_CURRENT_UA {
            BatteryFlow::Charging
        } else if current <= -MIN_MEANINGFUL_CURRENT_UA {
            BatteryFlow::Discharging
        } else {
            BatteryFlow::Holding
        };
        if self.flow != Some(flow) || self.adapter_online != Some(adapter_online) {
            self.flow = Some(flow);
            self.adapter_online = Some(adapter_online);
            self.samples.clear();
        }
        if flow == BatteryFlow::Holding {
            return Some(0);
        }
        self.samples.push_back(current);
        if self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        (self.samples.len() >= MIN_SAMPLES).then(|| {
            self.samples.iter().sum::<i64>() / self.samples.len() as i64
        })
    }
}

fn read_battery_overview(hwmon: &Path) -> BatteryOverview {
    BatteryOverview {
        capacity_percent: read_i64(&hwmon.join("smc_battery_capacity_percent"))
            .and_then(|value| u8::try_from(value.clamp(0, 100)).ok()),
        current_ua: read_i64(&hwmon.join("smc_battery_current_ua")),
        charge_now_uah: read_i64(&hwmon.join("smc_battery_charge_now_uah")),
        charge_full_uah: read_i64(&hwmon.join("smc_battery_charge_full_uah")),
        adapter_power_uw: read_i64(&hwmon.join("smc_adapter_power_uw")),
    }
}

fn format_battery_minutes(minutes: u64, suffix: &str) -> String {
    let hours = minutes / 60;
    let minutes = minutes % 60;
    if hours > 0 {
        format!("{hours} h {minutes:02} min {suffix}")
    } else {
        format!("{minutes} min {suffix}")
    }
}

fn battery_time_text(battery: &BatteryOverview, averaged_current_ua: Option<i64>) -> String {
    const MIN_MEANINGFUL_CURRENT_UA: i64 = 50_000;
    let Some(capacity) = battery.capacity_percent else {
        return "Battery data unavailable".into();
    };
    let raw_current = battery.current_ua.unwrap_or(0);
    if raw_current.unsigned_abs() >= MIN_MEANINGFUL_CURRENT_UA as u64
        && averaged_current_ua.is_none()
    {
        return "Estimating…".into();
    }
    let current = averaged_current_ua.unwrap_or(0);
    let adapter_online = battery.adapter_power_uw.is_some_and(|power| power > 0);

    if current <= -MIN_MEANINGFUL_CURRENT_UA {
        if let Some(charge) = battery.charge_now_uah.filter(|charge| *charge > 0) {
            let minutes = ((charge as f64 / current.unsigned_abs() as f64) * 60.0).round() as u64;
            return format_battery_minutes(minutes, "remaining");
        }
    } else if current >= MIN_MEANINGFUL_CURRENT_UA {
        if let (Some(now), Some(full)) = (battery.charge_now_uah, battery.charge_full_uah) {
            let remaining = full.saturating_sub(now);
            let minutes = ((remaining as f64 / current as f64) * 60.0).round() as u64;
            return format_battery_minutes(minutes, "until full");
        }
    }

    if adapter_online {
        if capacity >= 99 {
            "Fully charged".into()
        } else {
            format!("Holding at {capacity}%")
        }
    } else {
        "Battery idle".into()
    }
}

fn update_battery_overview(
    progress: &gtk4::ProgressBar,
    value_label: &gtk4::Label,
    time_label: &gtk4::Label,
    battery: &BatteryOverview,
    current_average: &mut BatteryCurrentAverage,
) {
    match battery.capacity_percent {
        Some(capacity) => {
            progress.set_fraction(capacity as f64 / 100.0);
            value_label.set_text(&format!("{capacity}%"));
        }
        None => {
            progress.set_fraction(0.0);
            value_label.set_text("--%");
        }
    }
    let averaged_current = current_average.update(battery);
    time_label.set_text(&battery_time_text(battery, averaged_current));
}

fn read_sensors(hwmon: &Path) -> Vec<(String, String, Option<i64>)> {
    let pattern = format!("{}/temp*_label", hwmon.display());
    let Ok(entries) = glob::glob(&pattern) else {
        return vec![];
    };
    let mut sensors = Vec::new();
    for entry in entries.flatten() {
        let key = fs::read_to_string(&entry)
            .ok()
            .map(|s| s.trim().to_string());
        let Some(key) = key else { continue };
        if key.is_empty() || key.len() < 2 {
            continue;
        }
        let input = entry.with_file_name(
            entry
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace("_label", "_input"),
        );
        sensors.push((sensor_label(&key), key, read_i64(&input)));
    }
    sensors.sort_by(|a, b| a.1.cmp(&b.1));
    sensors.dedup_by(|a, b| a.1 == b.1);
    sensors
}

fn read_charge_limit(hwmon: &Path) -> Option<u8> {
    fs::read_to_string(hwmon.join("battery_charge_limit"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn read_i64(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn scaled_value(value: i64, unit: &str) -> String {
    if value == 0 {
        format!("0 {unit}")
    } else {
        format!("{:.2} {unit}", value as f64 / 1_000_000.0)
    }
}

fn power_label(key: &str) -> String {
    match key {
        "PAPC" => "WiFi".into(),
        "PCPT" => "CPU package total (PECI)".into(),
        "PCTR" | "PCPL" => "CPU Total".into(),
        "PC0C" => "CPU Core 1".into(),
        "PC1C" => "CPU Core 2".into(),
        "PC2C" => "CPU Core 3".into(),
        "PC3C" => "CPU Core 4".into(),
        "PC4C" => "CPU Core 5".into(),
        "PC5C" => "CPU Core 6".into(),
        "PC6C" => "CPU Core 7".into(),
        "PC7C" => "CPU Core 8".into(),
        "PC0c" => "CPU raw package".into(),
        "PC0G" => "CPU integrated GPU".into(),
        "PC0I" => "CPU I/O high-side".into(),
        "PC0M" => "CPU I/O high-side 2".into(),
        "PC0R" => "CPU high-side average".into(),
        "PC0S" => "CPU system agent".into(),
        "PCAC" => "CPU core".into(),
        "PCAM" => "CPU core (IMON)".into(),
        "PCEC" => "CPU VccEDRAM".into(),
        "PCGC" => "Intel GPU (IMON)".into(),
        "PCGM" => "Intel GPU (IMON) 2".into(),
        "PCPC" => "CPU Cores".into(),
        "PCPG" => "CPU GFX".into(),
        "PCPD" => "CPU DRAM".into(),
        "PC1R" => "CPU Rail".into(),
        "PC5R" => "CPU S0 Rail".into(),
        "PCSC" => "CPU VCCSA (PCSC)".into(),
        "PD0R" => "DC-In MLB S0 rail".into(),
        "PD5R" => "DC-In MLB S5 rail".into(),
        "PDMR" => "DC-In MLB total".into(),
        "PDTR" => "DC-In total".into(),
        "PGTR" => "GPU Total".into(),
        "PG0R" => "GPU 0 rail".into(),
        "PG0C" => "GPU".into(),
        "PG1C" => "External GPU 1.8 V".into(),
        "PG2C" => "External GPU 1.05 V".into(),
        "PG3C" => "External GPU 1.35 V".into(),
        "PH0R" => "Drive 0".into(),
        "PH1R" => "Drive 1".into(),
        "PHPC" => "Heatpipe".into(),
        "PLDC" => "LCD panel".into(),
        "PM0C" => "Memory average".into(),
        "PM0R" => "Memory Rail".into(),
        "PN0C" => "MCH".into(),
        "PN1R" => "PCH Rail".into(),
        "PH02" => "Main 3.3V Rail".into(),
        "PH05" => "Main 5V Rail".into(),
        "Pp0R" => "12V Rail".into(),
        "PD2R" => "Main 12V Rail".into(),
        "PO0R" => "Misc. Rail".into(),
        "PBLC" | "PB0R" => "Battery Rail".into(),
        "PM1C" => "DDR".into(),
        "PO5R" => "Other 5 V high-side".into(),
        "PP0R" => "PBus".into(),
        "PPBR" => "PBus battery discharge".into(),
        "PSTR" => "System total (1 s delayed)".into(),
        "PZ0E" => "Zone 0 average target".into(),
        "PZ0F" => "Zone 0 filtered".into(),
        "PZ0G" => "Zone 0 average".into(),
        "PZ0T" => "Zone 0 throttle".into(),
        "PZ1E" => "Zone 1 target".into(),
        "PZ1F" => "Zone 1 filtered".into(),
        "PZ1G" => "Zone 1 average".into(),
        "PZ1T" => "Zone 1 throttle".into(),
        "PZ2E" => "Zone 2 target".into(),
        "PZ2F" => "Zone 2 filtered".into(),
        "PZ2G" => "Zone 2 average".into(),
        "PZ2T" => "Zone 2 throttle".into(),
        "PZ3E" => "Zone 3 target".into(),
        "PZ3F" => "Zone 3 filtered".into(),
        "PZ3G" => "Zone 3 average".into(),
        "PZ3T" => "Zone 3 throttle".into(),
        "PZ4E" => "Zone 4 target".into(),
        "PZ4F" => "Zone 4 filtered".into(),
        "PZ4G" => "Zone 4 average".into(),
        "PZ4T" => "Zone 4 throttle".into(),
        "PZAP" => "Power zone AP".into(),
        "PZBL" => "Power zone backlight".into(),
        "PZHD" => "Power zone storage".into(),
        _ => format!("unknown ({key})"),
    }
}

fn power_value_text(key: &str, value: i64) -> String {
    if matches!(key, "PZ0T" | "PZ1T" | "PZ2T" | "PZ3T" | "PZ4T") {
        if value == 0 {
            "0".into()
        } else {
            format!("{:.2}", value as f64 / 1_000_000.0)
        }
    } else {
        scaled_value(value, "W")
    }
}

fn read_smc_power_stats(hwmon: &Path) -> Vec<(String, String, String)> {
    let pattern = format!("{}/power*_label", hwmon.display());
    let Ok(entries) = glob::glob(&pattern) else {
        return vec![];
    };
    let mut stats = Vec::new();

    for label_path in entries.flatten() {
        let Some(key) = fs::read_to_string(&label_path)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|key| key.starts_with('P'))
        else {
            continue;
        };
        let input_path = label_path.with_file_name(
            label_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace("_label", "_input"),
        );
        if let Some(value) = read_i64(&input_path) {
            stats.push((
                power_label(&key),
                key.clone(),
                power_value_text(&key, value),
            ));
        }
    }

    stats.sort_by(|a, b| a.1.cmp(&b.1));
    stats
}

fn has_smc_power_key(hwmon: &Path, wanted: &str) -> bool {
    let pattern = format!("{}/power*_label", hwmon.display());
    glob::glob(&pattern).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|path| fs::read_to_string(path).is_ok_and(|key| key.trim() == wanted))
    })
}

fn read_power_telemetry(hwmon: &Path) -> Vec<(String, String, String)> {
    let mut values = Vec::new();
    let adapter_power_key = if has_smc_power_key(hwmon, "PD0R") {
        "PD0R"
    } else {
        "PDTR"
    };
    let mut add = |name: &str, key: &str, file: &str, format: fn(i64) -> String| {
        if let Some(value) = read_i64(&hwmon.join(file)) {
            values.push((name.to_string(), key.to_string(), format(value)));
        }
    };

    add("Power events", "", "power_event_count", |v| v.to_string());
    add(
        "Battery capacity",
        "BRSC",
        "smc_battery_capacity_percent",
        |v| format!("{v}%"),
    );
    add("Battery voltage", "B0AV", "smc_battery_voltage_uv", |v| {
        scaled_value(v, "V")
    });
    add("Battery current", "B0AC", "smc_battery_current_ua", |v| {
        if v == 0 {
            "0 A".into()
        } else {
            format!("{:.3} A", v as f64 / 1_000_000.0)
        }
    });
    add("Battery power", "B0AP", "smc_battery_power_uw", |v| {
        scaled_value(v, "W")
    });
    add(
        "Battery charge",
        "B0RM",
        "smc_battery_charge_now_uah",
        |v| scaled_value(v, "Ah"),
    );
    add(
        "Battery full charge",
        "B0FC",
        "smc_battery_charge_full_uah",
        |v| scaled_value(v, "Ah"),
    );
    add("Battery cycles", "B0CT", "smc_battery_cycle_count", |v| {
        v.to_string()
    });
    add("Adapter voltage", "VD0R", "smc_adapter_voltage_uv", |v| {
        scaled_value(v, "V")
    });
    add("Adapter current", "ID0R", "smc_adapter_current_ua", |v| {
        scaled_value(v, "A")
    });
    add(
        "Adapter power",
        adapter_power_key,
        "smc_adapter_power_uw",
        |v| scaled_value(v, "W"),
    );

    values.extend(read_smc_power_stats(hwmon));
    values.sort_by(|a, b| a.1.cmp(&b.1));

    values
}

fn print_usage() {
    println!("Usage:\n  t2-smc-control");
}

fn handle_cli_args() -> Option<Result<(), String>> {
    match env::args().nth(1)?.as_str() {
        "-h" | "--help" => {
            print_usage();
            Some(Ok(()))
        }
        _ => None,
    }
}

fn set_status(label: &gtk4::Label, text: &str, error: bool) {
    label.set_text(text);
    if error {
        label.add_css_class("error");
        label.remove_css_class("dim-label");
    } else {
        label.remove_css_class("error");
        label.add_css_class("dim-label");
    }
}

fn show_charge_limit(label: &gtk4::Label, meter: &gtk4::ProgressBar, hwmon: Option<&Path>) {
    match hwmon.and_then(read_charge_limit) {
        Some(limit) => {
            label.set_text(&format!("{limit}%"));
            meter.set_fraction(limit as f64 / 100.0);
        }
        None => {
            label.set_text("--%");
            meter.set_fraction(0.0);
        }
    }
}

fn clear_listbox(list: &gtk4::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn sensor_value_text(val: Option<i64>) -> String {
    match val {
        Some(0) => "0 C".into(),
        Some(value) => format!("{:.1} C", value as f64 / 1000.0),
        None => "n/a".into(),
    }
}

fn append_placeholder_row(list: &gtk4::ListBox, text: &str) {
    let row = gtk4::ListBoxRow::new();
    let label = gtk4::Label::new(Some(text));
    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("dim-label");
    row.set_child(Some(&label));
    list.append(&row);
}

fn table_cell(text: &str, width: i32, expand: bool, align: f32) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_width_chars(width);
    label.set_hexpand(expand);
    label.set_xalign(align);
    label.set_halign(if align == 1.0 {
        gtk4::Align::End
    } else {
        gtk4::Align::Fill
    });
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label
}

fn append_table_header(list: &gtk4::ListBox, first: &str, third: &str) {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);
    let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    line.set_margin_top(6);
    line.set_margin_bottom(6);
    line.set_margin_start(10);
    line.set_margin_end(10);
    let first = table_cell(first, 24, true, 0.0);
    let key = table_cell("SMC key", 8, false, 0.0);
    let value = table_cell(third, 10, false, 1.0);
    first.add_css_class("heading");
    key.add_css_class("heading");
    value.add_css_class("heading");
    line.append(&first);
    line.append(&key);
    line.append(&value);
    row.set_child(Some(&line));
    list.append(&row);
}

fn append_table_row(list: &gtk4::ListBox, name: &str, key: &str, value: &str) -> gtk4::Label {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    line.set_margin_top(5);
    line.set_margin_bottom(5);
    line.set_margin_start(10);
    line.set_margin_end(10);

    let name_label = table_cell(name, 24, true, 0.0);
    let key_label = table_cell(if key.is_empty() { "—" } else { key }, 8, false, 0.0);
    key_label.add_css_class("dim-label");
    key_label.add_css_class("monospace");
    let value_label = table_cell(value, 10, false, 1.0);
    value_label.add_css_class("numeric");

    line.append(&name_label);
    line.append(&key_label);
    line.append(&value_label);
    row.set_child(Some(&line));
    list.append(&row);

    value_label
}

fn refresh_value_rows(
    list: &gtk4::ListBox,
    rows: &Rc<RefCell<BTreeMap<String, gtk4::Label>>>,
    values: &[(String, String, String)],
) {
    let rebuild = {
        let rows = rows.borrow();
        rows.len() != values.len()
            || values
                .iter()
                .any(|(name, key, _)| !rows.contains_key(&(name.clone() + key)))
    };

    if rebuild {
        let mut rows = rows.borrow_mut();
        clear_listbox(list);
        rows.clear();
        append_table_header(list, "Metric", "Value");
        for (name, key, value) in values {
            rows.insert(name.clone() + key, append_table_row(list, name, key, value));
        }
        return;
    }

    let rows = rows.borrow();
    for (name, key, value) in values {
        if let Some(label) = rows.get(&(name.clone() + key)) {
            label.set_text(value);
        }
    }
}

fn refresh_sensor_rows(
    list: &gtk4::ListBox,
    rows: &Rc<RefCell<BTreeMap<String, gtk4::Label>>>,
    sensors: &[(String, String, Option<i64>)],
) {
    let has_placeholder = rows.borrow().is_empty() && list.first_child().is_some();

    if sensors.is_empty() {
        if !has_placeholder {
            rows.borrow_mut().clear();
            clear_listbox(list);
            append_placeholder_row(list, "No temperature sensors found");
        }
        return;
    }

    let rebuild = {
        let rows = rows.borrow();
        has_placeholder
            || rows.len() != sensors.len()
            || sensors.iter().any(|(_, key, _)| !rows.contains_key(key))
    };

    if rebuild {
        let mut rows = rows.borrow_mut();
        clear_listbox(list);
        rows.clear();
        append_table_header(list, "Sensor", "Temperature");
        for (name, key, val) in sensors {
            let value_label = append_table_row(list, name, key, &sensor_value_text(*val));
            rows.insert(key.clone(), value_label);
        }
        return;
    }

    let rows = rows.borrow();
    for (_, key, val) in sensors {
        if let Some(label) = rows.get(key) {
            label.set_text(&sensor_value_text(*val));
        }
    }
}

fn rebuild_sensor_rows(
    list: &gtk4::ListBox,
    rows: &Rc<RefCell<BTreeMap<String, gtk4::Label>>>,
    sensors: &[(String, String, Option<i64>)],
) {
    rows.borrow_mut().clear();
    clear_listbox(list);

    if sensors.is_empty() {
        append_placeholder_row(list, "No temperature sensors found");
        return;
    }

    let mut rows = rows.borrow_mut();
    append_table_header(list, "Sensor", "Temperature");
    for (name, key, val) in sensors {
        let value_label = append_table_row(list, name, key, &sensor_value_text(*val));
        rows.insert(key.clone(), value_label);
    }
}

fn main() {
    if let Some(result) = handle_cli_args() {
        if let Err(err) = result {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return;
    }

    register_embedded_resources();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/org/t2smccontrol/gtk")
        .build();

    app.connect_activate(|app| {
        install_palette();
        let hwmon = Rc::new(RefCell::new(find_hwmon()));
        let rtc = Rc::new(RefCell::new(find_t2smc_rtc()));

        // Header bar
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk4::Label::new(Some("SMC Control"))));
        let brand = kait2en_brand();
        brand.set_margin_start(10);
        brand.set_margin_end(10);
        header.pack_start(&brand);
        let rtc_value = gtk4::Label::new(Some("RTC (UTC): --"));
        rtc_value.set_halign(gtk4::Align::End);
        rtc_value.add_css_class("numeric");
        let rtc_sync = gtk4::Button::with_label("Sync RTC");
        rtc_sync.set_tooltip_text(Some("Write the current system time to the SMC hardware clock"));
        rtc_sync.set_sensitive(rtc.borrow().is_some());
        header.pack_end(&rtc_sync);
        header.pack_end(&rtc_value);

        let status = gtk4::Label::new(None);
        status.set_halign(gtk4::Align::Start);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");

        let charge_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let charge_title = gtk4::Label::new(Some("Battery charge limit"));
        charge_title.set_halign(gtk4::Align::Start);
        charge_title.set_xalign(0.0);
        charge_title.set_hexpand(true);
        charge_title.add_css_class("heading");
        let charge_value = gtk4::Label::new(Some("--%"));
        charge_value.add_css_class("numeric");
        charge_value.add_css_class("overview-value");
        charge_header.append(&charge_title);
        charge_header.append(&charge_value);
        let charge_meter = gtk4::ProgressBar::new();
        charge_meter.set_show_text(false);
        charge_meter.set_hexpand(true);
        charge_meter.set_valign(gtk4::Align::Center);
        charge_meter.add_css_class("overview-meter");
        charge_meter.set_tooltip_text(Some("Set in the desktop environment's power settings"));

        // Battery state and remaining-time estimate
        let battery_title = gtk4::Label::new(Some("Battery"));
        battery_title.set_halign(gtk4::Align::Start);
        battery_title.set_xalign(0.0);
        battery_title.set_hexpand(true);
        battery_title.add_css_class("heading");
        let battery_value = gtk4::Label::new(Some("--%"));
        battery_value.add_css_class("numeric");
        battery_value.add_css_class("overview-value");
        let battery_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        battery_header.append(&battery_title);
        battery_header.append(&battery_value);

        let battery_progress = gtk4::ProgressBar::new();
        battery_progress.set_show_text(false);
        battery_progress.set_hexpand(true);
        battery_progress.set_valign(gtk4::Align::Center);
        battery_progress.add_css_class("overview-meter");
        let battery_time = gtk4::Label::new(Some("Battery data unavailable"));
        battery_time.set_halign(gtk4::Align::Start);
        battery_time.set_xalign(0.0);
        battery_time.add_css_class("dim-label");
        battery_time.add_css_class("overview-status");
        status.add_css_class("overview-status");

        // Battery and adapter telemetry
        let power_title = gtk4::Label::new(Some("Power telemetry"));
        power_title.set_halign(gtk4::Align::Start);
        power_title.set_xalign(0.0);
        power_title.add_css_class("heading");

        let power_list = gtk4::ListBox::new();
        power_list.add_css_class("boxed-list");
        let power_rows = Rc::new(RefCell::new(BTreeMap::new()));
        if let Some(ref h) = *hwmon.borrow() {
            refresh_value_rows(&power_list, &power_rows, &read_power_telemetry(h));
        }
        let power_scroll = gtk4::ScrolledWindow::new();
        power_scroll.set_hexpand(true);
        power_scroll.set_vexpand(true);
        power_scroll.set_child(Some(&power_list));

        let power_panel = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        power_panel.set_hexpand(true);
        power_panel.set_vexpand(true);
        power_panel.append(&power_title);
        power_panel.append(&power_scroll);

        // Sensor list
        let sensors_title = gtk4::Label::new(Some("Temperatures"));
        sensors_title.set_halign(gtk4::Align::Start);
        sensors_title.set_xalign(0.0);
        sensors_title.add_css_class("heading");

        let sensor_list = gtk4::ListBox::new();
        sensor_list.add_css_class("boxed-list");
        let sensor_rows = Rc::new(RefCell::new(BTreeMap::new()));
        rebuild_sensor_rows(&sensor_list, &sensor_rows, &[]);

        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&sensor_list));

        let sensor_panel = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        sensor_panel.set_hexpand(true);
        sensor_panel.set_vexpand(true);
        sensor_panel.append(&sensors_title);
        sensor_panel.append(&scroll);

        // Layout
        let overview = gtk4::Grid::new();
        overview.set_column_spacing(18);
        overview.set_row_spacing(8);
        overview.set_column_homogeneous(true);
        overview.set_margin_top(14);
        overview.set_margin_bottom(14);
        overview.set_margin_start(14);
        overview.set_margin_end(14);
        overview.attach(&battery_header, 0, 0, 1, 1);
        overview.attach(&charge_header, 1, 0, 1, 1);
        overview.attach(&battery_progress, 0, 1, 1, 1);
        overview.attach(&charge_meter, 1, 1, 1, 1);
        overview.attach(&battery_time, 0, 2, 1, 1);
        overview.attach(&status, 1, 2, 1, 1);
        let overview_frame = gtk4::Frame::new(None);
        overview_frame.add_css_class("smc-panel");
        overview_frame.set_child(Some(&overview));

        let telemetry = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        telemetry.set_homogeneous(true);
        telemetry.set_hexpand(true);
        telemetry.set_vexpand(true);
        telemetry.append(&power_panel);
        telemetry.append(&sensor_panel);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        vbox.set_vexpand(true);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);
        vbox.append(&overview_frame);
        vbox.append(&telemetry);

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.add_css_class("smc-root");
        root.set_vexpand(true);
        root.append(&header);
        root.append(&vbox);
        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        footer.set_margin_start(8); footer.set_margin_end(8); footer.set_margin_bottom(8);
        let donate = gtk4::LinkButton::builder().uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400").label("Fund our bugs").build();
        donate.add_css_class("donate-link"); footer.append(&donate);
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0); spacer.set_hexpand(true); footer.append(&spacer);
        footer.append(&gtk4::Label::new(Some(&format!("v{APP_VERSION}"))));
        root.append(&footer);

        let window = adw::ApplicationWindow::new(app);
        window.set_title(Some("SMC Control"));
        window.set_default_size(1100, 760);
        window.set_content(Some(&root));

        let battery_current_average = Rc::new(RefCell::new(BatteryCurrentAverage::default()));
        show_charge_limit(&charge_value, &charge_meter, hwmon.borrow().as_deref());
        if let Some(ref h) = *hwmon.borrow() {
            set_status(&status, "Ready", false);
            update_battery_overview(
                &battery_progress,
                &battery_value,
                &battery_time,
                &read_battery_overview(h),
                &mut battery_current_average.borrow_mut(),
            );
        } else {
            set_status(&status, "Searching for t2smc...", false);
        }

        if let Some(ref r) = *rtc.borrow() {
            if let Some(time) = read_rtc_datetime(r) {
                rtc_value.set_text(&format!("RTC (UTC): {time}"));
            } else {
                rtc_value.set_text("RTC (UTC): unavailable");
            }
        } else {
            rtc_value.set_text("RTC (UTC): searching…");
        }

        let rtc_for_sync = rtc.clone();
        let status_for_sync = status.clone();
        let rtc_value_for_sync = rtc_value.clone();
        rtc_sync.connect_clicked(move |button| {
            let Some(r) = rtc_for_sync.borrow().clone() else {
                set_status(&status_for_sync, "t2smc RTC not found", true);
                return;
            };

            button.set_sensitive(false);
            match sync_rtc_from_system(&r) {
                Ok(()) => {
                    if let Some(time) = read_rtc_datetime(&r) {
                        rtc_value_for_sync.set_text(&format!("RTC (UTC): {time}"));
                    }
                    set_status(&status_for_sync, "Hardware clock set from system time", false);
                }
                Err(err) => set_status(&status_for_sync, &err, true),
            }
            button.set_sensitive(true);
        });

        // Poll
        let hw2 = hwmon.clone();
        let status_poll = status.clone();
        let sensor_rows_poll = sensor_rows.clone();
        let power_rows_poll = power_rows.clone();
        let power_list_poll = power_list.clone();
        let rtc_poll = rtc.clone();
        let rtc_value_poll = rtc_value.clone();
        let rtc_sync_poll = rtc_sync.clone();
        let charge_value_poll = charge_value.clone();
        let charge_meter_poll = charge_meter.clone();
        let battery_progress_poll = battery_progress.clone();
        let battery_value_poll = battery_value.clone();
        let battery_time_poll = battery_time.clone();
        let battery_current_average_poll = battery_current_average.clone();
        timeout_add_local(std::time::Duration::from_secs(1), move || {
            let current_hwmon = hw2.borrow().clone();
            show_charge_limit(&charge_value_poll, &charge_meter_poll, current_hwmon.as_deref());
            if let Some(h) = current_hwmon {
                let power = read_power_telemetry(&h);
                refresh_value_rows(&power_list_poll, &power_rows_poll, &power);
                let sensors = read_sensors(&h);
                refresh_sensor_rows(&sensor_list, &sensor_rows_poll, &sensors);
                update_battery_overview(
                    &battery_progress_poll,
                    &battery_value_poll,
                    &battery_time_poll,
                    &read_battery_overview(&h),
                    &mut battery_current_average_poll.borrow_mut(),
                );
            } else if let Some(h) = find_hwmon() {
                set_status(&status_poll, "Ready", false);
                *hw2.borrow_mut() = Some(h);
            }

            let current_rtc = rtc_poll.borrow().clone();
            if let Some(r) = current_rtc {
                match read_rtc_datetime(&r) {
                    Some(time) => {
                        rtc_value_poll.set_text(&format!("RTC (UTC): {time}"));
                    }
                    None => rtc_value_poll.set_text("RTC (UTC): unavailable"),
                }
            } else if let Some(r) = find_t2smc_rtc() {
                if let Some(time) = read_rtc_datetime(&r) {
                    rtc_value_poll.set_text(&format!("RTC (UTC): {time}"));
                }
                *rtc_poll.borrow_mut() = Some(r);
                rtc_sync_poll.set_sensitive(true);
            }
            glib::ControlFlow::Continue
        });

        window.present();
    });

    app.run();
}

fn register_embedded_resources() {
    gio::resources_register_include!("t2-smc-control.gresource")
        .expect("failed to register embedded GTK resources");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "t2-smc-control-test-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn finds_supported_hwmon_device() {
        let base = temp_path("hwmon");
        let unsupported = base.join("hwmon0");
        let supported = base.join("hwmon4");
        fs::create_dir_all(&unsupported).unwrap();
        fs::create_dir_all(&supported).unwrap();
        fs::write(unsupported.join("name"), "other\n").unwrap();
        fs::write(supported.join("name"), "t2smc\n").unwrap();

        assert_eq!(find_hwmon_in(&base), Some(supported));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn formats_available_power_telemetry() {
        let hwmon = temp_path("power");
        fs::create_dir_all(&hwmon).unwrap();
        fs::write(hwmon.join("power_event_count"), "2\n").unwrap();
        fs::write(hwmon.join("smc_battery_voltage_uv"), "12100000\n").unwrap();

        assert_eq!(
            read_power_telemetry(&hwmon),
            vec![
                ("Power events".into(), "".into(), "2".into()),
                ("Battery voltage".into(), "B0AV".into(), "12.10 V".into()),
            ]
        );

        let _ = fs::remove_dir_all(hwmon);
    }

    #[test]
    fn reads_all_p_prefixed_hwmon_power_channels() {
        let hwmon = temp_path("power-keys");
        fs::create_dir_all(&hwmon).unwrap();
        fs::write(hwmon.join("power1_label"), "PC0C\n").unwrap();
        fs::write(hwmon.join("power1_input"), "12345000\n").unwrap();
        fs::write(hwmon.join("power2_label"), "PG0R\n").unwrap();
        fs::write(hwmon.join("power2_input"), "2500000\n").unwrap();
        fs::write(hwmon.join("power3_label"), "not-power\n").unwrap();
        fs::write(hwmon.join("power3_input"), "1\n").unwrap();

        assert_eq!(
            read_smc_power_stats(&hwmon),
            vec![
                ("CPU Core 1".into(), "PC0C".into(), "12.35 W".into()),
                ("GPU 0 rail".into(), "PG0R".into(), "2.50 W".into()),
            ]
        );

        let _ = fs::remove_dir_all(hwmon);
    }

    #[test]
    fn labels_known_power_keys() {
        assert_eq!(power_label("PCPT"), "CPU package total (PECI)");
        assert_eq!(power_label("PC0C"), "CPU Core 1");
        assert_eq!(power_label("PC7C"), "CPU Core 8");
        assert_eq!(power_label("PCPD"), "CPU DRAM");
        assert_eq!(power_label("PGTR"), "GPU Total");
        assert_eq!(power_label("PB0R"), "Battery Rail");
        assert_eq!(power_label("PAPC"), "WiFi");
        assert_eq!(power_label("PG0C"), "GPU");
        assert_eq!(power_label("PLDC"), "LCD panel");
        assert_eq!(power_label("PZ4G"), "Zone 4 average");
        assert_eq!(power_label("PD0R"), "DC-In MLB S0 rail");
        assert_eq!(power_label("PG0R"), "GPU 0 rail");
        assert_eq!(power_label("PZ0G"), "Zone 0 average");
        assert_eq!(power_label("PDTR"), "DC-In total");
        assert_eq!(power_label("PSTR"), "System total (1 s delayed)");
        assert_eq!(power_label("PXYZ"), "unknown (PXYZ)");
    }

    #[test]
    fn labels_documented_mainboard_bottom_sensor() {
        assert_eq!(sensor_label("TC0E"), "CPU 1 Diode Virtual");
        assert_eq!(sensor_label("TC0F"), "CPU 1 Diode Filtered");
        assert_eq!(sensor_label("TC1C"), "CPU Core 1");
        assert_eq!(sensor_label("TC8C"), "CPU Core 8");
        assert_eq!(sensor_label("TCBC"), "unknown (TCBC)");
        assert_eq!(sensor_label("Tm1P"), "Mainboard Bottom");
        assert_eq!(sensor_label("TH1a"), "Drive 1 Raw A");
        assert_eq!(sensor_label("TH1b"), "Drive 1 Raw B");
        assert_eq!(sensor_label("Th1H"), "Right Fin Stack");
        assert_eq!(sensor_label("Th2H"), "Left Fin Stack");
        assert_eq!(sensor_label("TF0S"), "unknown (TF0S)");
    }

    #[test]
    fn calculates_battery_discharge_and_charge_time() {
        let discharging = BatteryOverview {
            capacity_percent: Some(80),
            current_ua: Some(-1_000_000),
            charge_now_uah: Some(3_000_000),
            charge_full_uah: Some(4_000_000),
            adapter_power_uw: Some(0),
        };
        assert_eq!(battery_time_text(&discharging, Some(-1_000_000)), "3 h 00 min remaining");

        let charging = BatteryOverview {
            current_ua: Some(1_000_000),
            adapter_power_uw: Some(20_000_000),
            ..discharging
        };
        assert_eq!(battery_time_text(&charging, Some(1_000_000)), "1 h 00 min until full");
    }

    #[test]
    fn describes_battery_holding_at_charge_limit() {
        let battery = BatteryOverview {
            capacity_percent: Some(80),
            current_ua: Some(0),
            charge_now_uah: Some(3_000_000),
            charge_full_uah: Some(4_000_000),
            adapter_power_uw: Some(7_000_000),
        };
        assert_eq!(battery_time_text(&battery, Some(0)), "Holding at 80%");
    }

    #[test]
    fn averages_current_after_five_samples_and_resets_on_power_change() {
        let mut average = BatteryCurrentAverage::default();
        let mut battery = BatteryOverview {
            capacity_percent: Some(80),
            current_ua: Some(-1_000_000),
            charge_now_uah: Some(3_000_000),
            charge_full_uah: Some(4_000_000),
            adapter_power_uw: Some(0),
        };
        for _ in 0..4 {
            assert_eq!(average.update(&battery), None);
        }
        assert_eq!(average.update(&battery), Some(-1_000_000));
        battery.adapter_power_uw = Some(10_000_000);
        assert_eq!(average.update(&battery), None);
    }

    #[test]
    fn formats_signed_and_zero_temperature_values_as_delivered() {
        assert_eq!(sensor_value_text(Some(-127_000)), "-127.0 C");
        assert_eq!(sensor_value_text(Some(0)), "0 C");
        assert_eq!(sensor_value_text(None), "n/a");
    }

    #[test]
    fn formats_throttle_values_without_watt_unit() {
        assert_eq!(power_value_text("PZ0T", 12_500_000), "12.50");
        assert_eq!(power_value_text("PZ4T", 0), "0");
        assert_eq!(power_value_text("PSTR", 12_500_000), "12.50 W");
        assert_eq!(power_value_text("PSTR", 0), "0 W");
    }

    #[test]
    fn sorts_power_values_by_key() {
        let hwmon = temp_path("power-order");
        fs::create_dir_all(&hwmon).unwrap();
        for (index, key, value) in [
            (1, "PZZZ", 0),
            (2, "PYYY", 1_000_000),
            (3, "PC0C", 0),
            (4, "PDTR", 2_000_000),
        ] {
            fs::write(hwmon.join(format!("power{index}_label")), key).unwrap();
            fs::write(hwmon.join(format!("power{index}_input")), value.to_string()).unwrap();
        }

        let stats = read_smc_power_stats(&hwmon);
        let keys: Vec<_> = stats.iter().map(|(_, key, _)| key.as_str()).collect();
        assert_eq!(keys, vec!["PC0C", "PDTR", "PYYY", "PZZZ"]);

        let _ = fs::remove_dir_all(hwmon);
    }
}
