use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

#[allow(unused_imports)]
use adw::prelude::*;
use glib::timeout_add_local;
use gtk4::{gio, glib};

const APP_ID: &str = "org.t2smccontrol.gtk";
const HWMON_NAMES: &[&str] = &["t2smc", "macsmc"];
const RTC_NAME_PREFIX: &str = "t2smc ";

fn physical_cpu_core_count() -> Option<usize> {
    let entries = fs::read_dir("/sys/devices/system/cpu").ok()?;
    let mut cores = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(cpu_id) = name.strip_prefix("cpu") else {
            continue;
        };
        if cpu_id.parse::<usize>().is_err() {
            continue;
        }

        let package_id = fs::read_to_string(entry.path().join("topology/physical_package_id"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let Some(core_id) = fs::read_to_string(entry.path().join("topology/core_id"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        else {
            continue;
        };

        cores.push((package_id, core_id));
    }

    cores.sort_unstable();
    cores.dedup();
    (!cores.is_empty()).then_some(cores.len())
}

fn cpu_core_label(key: &str, physical_cores: Option<usize>) -> Option<String> {
    let bytes = key.as_bytes();
    if bytes.len() != 4 || bytes[0] != b'T' || bytes[1] != b'C' || bytes[3] != b'C' {
        return None;
    }

    let smc_index = (bytes[2] as char).to_digit(10)? as usize;
    if smc_index == 0 {
        return None;
    }

    let core_index = smc_index - 1;
    if physical_cores.is_some_and(|count| core_index >= count) {
        return None;
    }

    Some(format!("CPU Core {core_index}"))
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
        "Th0H" | "TH1a" => Some("Heatpipe 1".into()),
        "Th1H" | "TH1b" => Some("Heatpipe 2".into()),
        "Th2H" => Some("Heatpipe 3".into()),
        _ => None,
    }
}

fn sensor_label(key: &str, physical_cores: Option<usize>) -> String {
    if let Some(label) = cpu_core_label(key, physical_cores) {
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
        "TB0T" => "Battery 1".into(),
        "TB1T" => "Battery 2".into(),
        "TB2T" => "Battery 3".into(),
        "TC0P" => "CPU Proximity".into(),
        "TC0E" => "CPU Sensor 0E".into(),
        "TC0F" => "CPU Sensor 0F".into(),
        "TCGC" => "CPU Graphics Core".into(),
        "TCMX" => "CPU Memory".into(),
        "TCSA" => "CPU System Agent".into(),
        "TCXC" => "CPU Package".into(),
        "TG0P" => "GPU Proximity".into(),
        "TGDD" => "GPU Die digital".into(),
        "TGDF" => "GPU Die analog".into(),
        "TGVP" => "GPU VR".into(),
        "TH0F" => "SSD Heatsink".into(),
        "TH0X" => "SSD Controller".into(),
        "TH0a" => "SSD NAND".into(),
        "TH0b" => "SSD NAND 2".into(),
        "Tm0P" => "Mainboard Proximity".into(),
        "TPCD" => "PCH Die".into(),
        "TTLD" => "Thunderbolt L".into(),
        "TTRD" => "Thunderbolt R".into(),
        "TW0P" => "WiFi".into(),
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

fn read_u32(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn write_string(path: &Path, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|err| format!("Cannot write {}: {err}", path.display()))
}

fn find_hwmon() -> Option<PathBuf> {
    for entry in glob::glob("/sys/class/hwmon/hwmon*/name").ok()? {
        let path = entry.ok()?;
        let name = fs::read_to_string(&path).ok()?;
        if HWMON_NAMES.contains(&name.trim()) {
            return path.parent().map(Path::to_path_buf);
        }
    }
    None
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

fn read_rtc_datetime(rtc: &Path) -> Option<String> {
    let date = fs::read_to_string(rtc.join("date")).ok()?;
    let time = fs::read_to_string(rtc.join("time")).ok()?;

    Some(format!("{} {}", date.trim(), time.trim()))
}

fn read_sensors(hwmon: &Path) -> Vec<(String, Option<u32>)> {
    let pattern = format!("{}/temp*_label", hwmon.display());
    let Ok(entries) = glob::glob(&pattern) else {
        return vec![];
    };
    let mut sensors = Vec::new();
    let physical_cores = physical_cpu_core_count();
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
        sensors.push((sensor_label(&key, physical_cores), read_u32(&input)));
    }
    sensors.sort_by(|a, b| a.0.cmp(&b.0));
    sensors.dedup_by(|a, b| a.0 == b.0);
    sensors
}

fn read_charge_limit(hwmon: &Path) -> Option<u8> {
    let path = hwmon.join("battery_charge_limit");
    fs::read_to_string(&path).ok()?.trim().parse().ok()
}

fn write_charge_limit(hwmon: &Path, percent: u8) -> Result<(), String> {
    write_string(&hwmon.join("battery_charge_limit"), &percent.to_string())
}

fn prepare_charge_limit_access(hwmon: &Path) -> Result<(), String> {
    let path = hwmon.join("battery_charge_limit");

    if OpenOptions::new().write(true).open(&path).is_ok() {
        return Ok(());
    }

    let status = Command::new("pkexec")
        .arg("chmod")
        .arg("a+w")
        .arg(&path)
        .status()
        .map_err(|err| format!("Cannot start pkexec: {err}"))?;

    if !status.success() {
        return Err("Write access to battery_charge_limit was not granted".into());
    }

    OpenOptions::new()
        .write(true)
        .open(&path)
        .map(|_| ())
        .map_err(|err| format!("battery_charge_limit is still not writable: {err}"))
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

fn initialize_charge_limit(
    hwmon: &Path,
    slider: &gtk4::Scale,
    charge_value: &gtk4::Label,
    status: &gtk4::Label,
    updating_slider: &Rc<RefCell<bool>>,
    last_committed: &Rc<RefCell<Option<u8>>>,
) {
    let Some(limit) = read_charge_limit(hwmon) else {
        slider.set_sensitive(false);
        set_status(status, "battery_charge_limit not found", true);
        return;
    };

    *updating_slider.borrow_mut() = true;
    slider.set_value(limit as f64);
    *updating_slider.borrow_mut() = false;
    charge_value.set_text(&format!("{limit}%"));
    *last_committed.borrow_mut() = Some(limit);

    match prepare_charge_limit_access(hwmon) {
        Ok(()) => {
            slider.set_sensitive(true);
            set_status(status, "Ready", false);
        }
        Err(err) => {
            slider.set_sensitive(false);
            set_status(status, &err, true);
        }
    }
}

fn clear_listbox(list: &gtk4::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn sensor_value_text(val: Option<u32>) -> String {
    val.map(|v| format!("{:.1} C", v as f64 / 1000.0))
        .unwrap_or_else(|| "n/a".into())
}

fn append_placeholder_row(list: &gtk4::ListBox) {
    let row = gtk4::ListBoxRow::new();
    let label = gtk4::Label::new(Some("No temperature sensors found"));
    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("dim-label");
    row.set_child(Some(&label));
    list.append(&row);
}

fn append_sensor_row(list: &gtk4::ListBox, name: &str, val: Option<u32>) -> gtk4::Label {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);

    let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    line.set_margin_top(10);
    line.set_margin_bottom(10);
    line.set_margin_start(12);
    line.set_margin_end(12);

    let name_label = gtk4::Label::new(Some(name));
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_hexpand(true);
    name_label.set_xalign(0.0);

    let value_label = gtk4::Label::new(Some(&sensor_value_text(val)));
    value_label.set_halign(gtk4::Align::End);
    value_label.add_css_class("numeric");

    line.append(&name_label);
    line.append(&value_label);
    row.set_child(Some(&line));
    list.append(&row);

    value_label
}

fn refresh_sensor_rows(
    list: &gtk4::ListBox,
    rows: &Rc<RefCell<BTreeMap<String, gtk4::Label>>>,
    sensors: &[(String, Option<u32>)],
) {
    let has_placeholder = rows.borrow().is_empty() && list.first_child().is_some();

    if sensors.is_empty() {
        if !has_placeholder {
            rows.borrow_mut().clear();
            clear_listbox(list);
            append_placeholder_row(list);
        }
        return;
    }

    let rebuild = {
        let rows = rows.borrow();
        has_placeholder
            || rows.len() != sensors.len()
            || sensors.iter().any(|(name, _)| !rows.contains_key(name))
    };

    if rebuild {
        let mut rows = rows.borrow_mut();
        clear_listbox(list);
        rows.clear();
        for (name, val) in sensors {
            let value_label = append_sensor_row(list, name, *val);
            rows.insert(name.clone(), value_label);
        }
        return;
    }

    let rows = rows.borrow();
    for (name, val) in sensors {
        if let Some(label) = rows.get(name) {
            label.set_text(&sensor_value_text(*val));
        }
    }
}

fn rebuild_sensor_rows(
    list: &gtk4::ListBox,
    rows: &Rc<RefCell<BTreeMap<String, gtk4::Label>>>,
    sensors: &[(String, Option<u32>)],
) {
    rows.borrow_mut().clear();
    clear_listbox(list);

    if sensors.is_empty() {
        append_placeholder_row(list);
        return;
    }

    let mut rows = rows.borrow_mut();
    for (name, val) in sensors {
        let value_label = append_sensor_row(list, name, *val);
        rows.insert(name.clone(), value_label);
    }
}

fn main() {
    register_embedded_resources();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/org/t2smccontrol/gtk")
        .build();

    app.connect_activate(|app| {
        let hwmon = Rc::new(RefCell::new(find_hwmon()));
        let rtc = Rc::new(RefCell::new(find_t2smc_rtc()));

        // Header bar
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk4::Label::new(Some("SMC Control"))));

        let status = gtk4::Label::new(None);
        status.set_halign(gtk4::Align::Start);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");

        // Charge limit
        let charge_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        charge_box.set_margin_top(14);
        charge_box.set_margin_bottom(14);
        charge_box.set_margin_start(14);
        charge_box.set_margin_end(14);

        let charge_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let charge_title = gtk4::Label::new(Some("Battery charge limit"));
        charge_title.set_halign(gtk4::Align::Start);
        charge_title.set_xalign(0.0);
        charge_title.set_hexpand(true);
        charge_title.add_css_class("heading");
        let charge_value = gtk4::Label::new(Some("--%"));
        charge_value.add_css_class("numeric");
        charge_header.append(&charge_title);
        charge_header.append(&charge_value);

        let slider = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 50.0, 100.0, 1.0);
        slider.set_draw_value(true);
        slider.set_value_pos(gtk4::PositionType::Bottom);
        slider.set_hexpand(true);
        slider.add_mark(50.0, gtk4::PositionType::Bottom, Some("50"));
        slider.add_mark(80.0, gtk4::PositionType::Bottom, Some("80"));
        slider.add_mark(100.0, gtk4::PositionType::Bottom, Some("100"));
        slider.set_sensitive(false);

        charge_box.append(&charge_header);
        charge_box.append(&slider);
        charge_box.append(&status);

        let charge_frame = gtk4::Frame::new(None);
        charge_frame.set_child(Some(&charge_box));

        // SMC RTC
        let rtc_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        rtc_box.set_margin_top(14);
        rtc_box.set_margin_bottom(14);
        rtc_box.set_margin_start(14);
        rtc_box.set_margin_end(14);

        let rtc_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let rtc_title = gtk4::Label::new(Some("SMC RTC"));
        rtc_title.set_halign(gtk4::Align::Start);
        rtc_title.set_xalign(0.0);
        rtc_title.set_hexpand(true);
        rtc_title.add_css_class("heading");
        let rtc_value = gtk4::Label::new(Some("--"));
        rtc_value.set_halign(gtk4::Align::End);
        rtc_value.add_css_class("numeric");
        rtc_header.append(&rtc_title);
        rtc_header.append(&rtc_value);

        let rtc_status = gtk4::Label::new(None);
        rtc_status.set_halign(gtk4::Align::Start);
        rtc_status.set_xalign(0.0);
        rtc_status.add_css_class("dim-label");

        rtc_box.append(&rtc_header);
        rtc_box.append(&rtc_status);

        let rtc_frame = gtk4::Frame::new(None);
        rtc_frame.set_child(Some(&rtc_box));

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

        // Layout
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        vbox.set_vexpand(true);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);
        vbox.append(&charge_frame);
        vbox.append(&rtc_frame);
        vbox.append(&sensors_title);
        vbox.append(&scroll);

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_vexpand(true);
        root.append(&header);
        root.append(&vbox);

        let window = adw::ApplicationWindow::new(app);
        window.set_title(Some("SMC Control"));
        window.set_default_size(460, 560);
        window.set_content(Some(&root));

        let updating_slider = Rc::new(RefCell::new(false));
        let last_committed = Rc::new(RefCell::new(None));
        if let Some(ref h) = *hwmon.borrow() {
            initialize_charge_limit(
                h,
                &slider,
                &charge_value,
                &status,
                &updating_slider,
                &last_committed,
            );
        } else {
            set_status(&status, "Searching for t2smc...", false);
        }

        if let Some(ref r) = *rtc.borrow() {
            if let Some(time) = read_rtc_datetime(r) {
                rtc_value.set_text(&time);
                set_status(&rtc_status, &format!("Device: {}", r.display()), false);
            } else {
                set_status(&rtc_status, "Cannot read SMC RTC", true);
            }
        } else {
            set_status(&rtc_status, "Searching for SMC RTC...", false);
        }

        let value_for_slider = charge_value.clone();
        let updating_for_slider = updating_slider.clone();
        slider.connect_value_changed(move |scale| {
            if *updating_for_slider.borrow() {
                return;
            }
            let percent = scale.value().round().clamp(0.0, 100.0) as u8;
            value_for_slider.set_text(&format!("{percent}%"));
        });

        let write_current_limit = Rc::new({
            let hw_for_release = hwmon.clone();
            let status_for_release = status.clone();
            let value_for_release = charge_value.clone();
            let slider_for_release = slider.clone();
            let last_committed_for_release = last_committed.clone();
            move || {
                let percent = slider_for_release.value().round().clamp(0.0, 100.0) as u8;
                value_for_release.set_text(&format!("{percent}%"));

                if *last_committed_for_release.borrow() == Some(percent) {
                    return;
                }

                let Some(ref h) = *hw_for_release.borrow() else {
                    set_status(&status_for_release, "t2smc hwmon device not found", true);
                    return;
                };

                match write_charge_limit(h, percent) {
                    Ok(()) => match read_charge_limit(h) {
                        Some(actual) if actual == percent => {
                            *last_committed_for_release.borrow_mut() = Some(actual);
                            set_status(
                                &status_for_release,
                                &format!("Charge limit set to {actual}%"),
                                false,
                            );
                        }
                        Some(actual) => set_status(
                            &status_for_release,
                            &format!(
                                "Write mismatch: requested {percent}%, device reports {actual}%"
                            ),
                            true,
                        ),
                        None => {
                            *last_committed_for_release.borrow_mut() = Some(percent);
                            set_status(
                                &status_for_release,
                                &format!("Charge limit write sent: {percent}%"),
                                false,
                            );
                        }
                    },
                    Err(err) => set_status(&status_for_release, &err, true),
                }
            }
        });

        let release = gtk4::EventControllerLegacy::new();
        release.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let write_on_release = write_current_limit.clone();
        release.connect_event(move |_, event| {
            if event.event_type() == gtk4::gdk::EventType::ButtonRelease {
                write_on_release();
            }
            glib::Propagation::Proceed
        });
        root.add_controller(release);

        // Poll
        let hw2 = hwmon.clone();
        let status_poll = status.clone();
        let slider_poll = slider.clone();
        let value_poll = charge_value.clone();
        let updating_poll = updating_slider.clone();
        let last_committed_poll = last_committed.clone();
        let sensor_rows_poll = sensor_rows.clone();
        let rtc_poll = rtc.clone();
        let rtc_value_poll = rtc_value.clone();
        let rtc_status_poll = rtc_status.clone();
        timeout_add_local(std::time::Duration::from_secs(1), move || {
            let current_hwmon = hw2.borrow().clone();
            if let Some(h) = current_hwmon {
                let sensors = read_sensors(&h);
                refresh_sensor_rows(&sensor_list, &sensor_rows_poll, &sensors);
            } else if let Some(h) = find_hwmon() {
                initialize_charge_limit(
                    &h,
                    &slider_poll,
                    &value_poll,
                    &status_poll,
                    &updating_poll,
                    &last_committed_poll,
                );
                *hw2.borrow_mut() = Some(h);
            }

            let current_rtc = rtc_poll.borrow().clone();
            if let Some(r) = current_rtc {
                match read_rtc_datetime(&r) {
                    Some(time) => {
                        rtc_value_poll.set_text(&time);
                        set_status(&rtc_status_poll, &format!("Device: {}", r.display()), false);
                    }
                    None => set_status(&rtc_status_poll, "Cannot read SMC RTC", true),
                }
            } else if let Some(r) = find_t2smc_rtc() {
                if let Some(time) = read_rtc_datetime(&r) {
                    rtc_value_poll.set_text(&time);
                }
                set_status(&rtc_status_poll, &format!("Device: {}", r.display()), false);
                *rtc_poll.borrow_mut() = Some(r);
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
