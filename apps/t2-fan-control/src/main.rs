mod config;
mod control;
mod error;
mod ipc;
mod service;
mod style;
mod sysfs;

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::mpsc,
    thread,
    time::Duration,
};

use config::{curve, curve_percent, normalize_curve, resize_soak, speed_range, AppConfig, CurvePoint, MAX_SOAK_TEMP_C};
use control::{ControlSnapshot, Controller};
use adw::StyleManager;
use gtk4::{
    cairo,
    gio,
    glib,
    prelude::*, Align, Application, ApplicationWindow, Box as GtkBox, CheckButton, CssProvider,
    DrawingArea, DropDown, Entry, EventControllerFocus, EventControllerMotion, Expander,
    GestureDrag, Grid, Label, LinkButton, Orientation, Popover, PositionType, StringList, Switch,
    gdk,
};
use ipc::{DaemonState, Request};
use signal_hook::consts::signal::SIGHUP;
use sysfs::{discover_fans, discover_temperature_sources, FanEndpoint, TemperatureSnapshot, TemperatureSource};

fn kait2en_brand() -> DrawingArea {
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let dark = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let light = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
        .unwrap_or_else(|_| dark.clone());
    let brand = DrawingArea::new();
    brand.set_content_width(80); brand.set_content_height(21); brand.set_size_request(80, 21);
    brand.set_draw_func(move |_area, context, width, height| {
        let pixbuf = if adw::StyleManager::default().is_dark() { &dark } else { &light };
        let scale = f64::min(width as f64 / pixbuf.width() as f64, height as f64 / pixbuf.height() as f64);
        let _ = context.save();
        context.translate((width as f64 - pixbuf.width() as f64 * scale) / 2.0, (height as f64 - pixbuf.height() as f64 * scale) / 2.0);
        context.scale(scale, scale); context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = context.paint(); let _ = context.restore();
    });
    let weak = brand.downgrade();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(brand) = weak.upgrade() {
            brand.queue_draw();
        }
    });
    brand
}

const APP_ID: &str = "org.t2fancontrol.gtk";
const APP_VERSION: &str = "0.08";
const HISTORY_CAPACITY: usize = 90;

#[derive(Clone, Copy)]
struct ThemePalette {
    chip_border: &'static str,
    chip_hover_bg: &'static str,
    chip_checked_bg: &'static str,
    chip_checked_border: &'static str,
    meta_fg: &'static str,
    cpu: (f64, f64, f64),
    gpu: (f64, f64, f64),
    effective: (f64, f64, f64),
    optional: (f64, f64, f64),
    system: (f64, f64, f64),
    fan: (f64, f64, f64),
    curve: (f64, f64, f64),
}

const DARK_PALETTE: ThemePalette = ThemePalette {
    chip_border: "rgba(210,210,210,0.24)",
    chip_hover_bg: "rgba(255,255,255,0.04)",
    chip_checked_bg: "rgba(255,255,255,0.10)",
    chip_checked_border: "rgba(232,232,232,0.45)",
    meta_fg: "rgba(230,230,230,0.62)",
    cpu: (0.92, 0.54, 0.28),
    gpu: (0.34, 0.60, 0.86),
    effective: (0.68, 0.82, 0.42),
    optional: (0.92, 0.70, 0.30),
    system: (0.78, 0.52, 0.88),
    fan: (0.50, 0.76, 0.58),
    curve: (0.86, 0.86, 0.88),
};

const LIGHT_PALETTE: ThemePalette = ThemePalette {
    chip_border: "rgba(0,0,0,0.18)",
    chip_hover_bg: "rgba(0,0,0,0.05)",
    chip_checked_bg: "rgba(0,0,0,0.10)",
    chip_checked_border: "rgba(0,0,0,0.35)",
    meta_fg: "rgba(36,36,36,0.68)",
    cpu: (0.86, 0.43, 0.18),
    gpu: (0.20, 0.45, 0.75),
    effective: (0.42, 0.62, 0.18),
    optional: (0.75, 0.48, 0.08),
    system: (0.55, 0.30, 0.68),
    fan: (0.30, 0.58, 0.38),
    curve: (0.20, 0.20, 0.22),
};

#[derive(Clone, Copy, Debug, Default)]
struct HistorySample {
    cpu_temp_c: Option<u8>,
    gpu_temp_c: Option<u8>,
    effective_temp_c: Option<u8>,
    system_temp_c: Option<u8>,
    optional_curve_temp_c: Option<u8>,
}

#[derive(Clone, Copy)]
enum CurveDragTarget { Point(usize), SoakWall }

struct UiRefs {
    status: Label,
    cpu_label: Label,
    gpu_label: Label,
    effective_label: Label,
    hottest_sensor_label: Label,
    overall_hottest_sensor_label: Label,
    full_speed_reason_label: Label,
    system_label: Label,
    target_label: Label,
    fan_label: Label,
    details_label: Label,
    control_switch: Switch,
    advanced: Expander,
    any_sensor_check: CheckButton,
    any_sensor_temp: Entry,
    any_sensor_focus: EventControllerFocus,
    system_cooling_time: Entry,
    system_cooling_focus: EventControllerFocus,
    curve_sensor_dropdown: DropDown,
    sensor_choice_keys: RefCell<Vec<String>>,
    curve_title: Label,
    curve_legend: Label,
    temperature_legend: Label,
    curve_area: DrawingArea,
    temperature_graph: DrawingArea,
    fan_graph: DrawingArea,
    syncing: Cell<bool>,
}

struct AppModel {
    config: AppConfig,
    snapshot: ControlSnapshot,
    status: String,
    fans: Vec<ipc::FanStatus>,
    sensor_choices: Vec<ipc::SensorChoice>,
    temperature_history: VecDeque<HistorySample>,
    fan_percent_history: VecDeque<u8>,
}

impl AppModel {
    fn new() -> Self {
        let mut config = AppConfig::load();
        config.autostart_enabled = service::autostart_enabled();
        let mut model = Self {
            config,
            snapshot: ControlSnapshot::default(),
            status: String::new(),
            fans: Vec::new(),
            sensor_choices: Vec::new(),
            temperature_history: VecDeque::with_capacity(HISTORY_CAPACITY),
            fan_percent_history: VecDeque::with_capacity(HISTORY_CAPACITY),
        };
        let _ = model.refresh_from_daemon();
        model.push_history_sample();
        model
    }

    fn refresh_from_daemon(&mut self) -> Result<(), error::FanControlError> {
        let state = ipc::send_request(Request::GetState)?;
        self.apply_state(state);
        Ok(())
    }

    fn tick(&mut self) {
        match self.refresh_from_daemon() {
            Ok(()) => {
                self.push_history_sample();
            }
            Err(error) => {
                self.status = format!("Daemon unavailable: {error}");
            }
        }
    }

    fn apply_state(&mut self, state: DaemonState) {
        self.status = state.status;
        self.config.soak_temp_c = state.soak_temp_c;
        self.config.system_cooling_time_s = state.system_cooling_time_s;
        self.config.any_sensor_enabled = state.any_sensor_enabled;
        self.config.any_sensor_temp_c = state.any_sensor_temp_c;
        self.config.automatic_control_enabled = state.control_active;
        self.config.autostart_enabled = state.autostart_enabled;
        self.config.custom_curve = state.custom_curve;
        self.config.curve_sensor_key = state.curve_sensor_key;
        self.sensor_choices = state.sensor_choices;
        self.snapshot = ControlSnapshot {
            temperatures: TemperatureSnapshot {
                cpu_temp_c: state.cpu_temp_c,
                gpu_temp_c: state.gpu_temp_c,
                hottest_temp_c: state.effective_temp_c,
                hottest_sensor_name: state.hottest_sensor_name,
                optional_curve_temp_c: state.optional_curve_temp_c,
                overall_hottest_temp_c: state.overall_hottest_temp_c,
                overall_hottest_sensor_name: state.overall_hottest_sensor_name,
                system_temp_c: state.system_temp_c,
                system_sensor_count: state.system_sensor_count,
                monitored_sensor_count: state.monitored_sensor_count,
            },
            effective_temp_c: state.effective_temp_c,
            system_temp_c: state.system_temp_c,
            system_sensor_count: state.system_sensor_count,
            heat_soak_cooling: state.heat_soak_cooling,
            any_sensor_cooling: state.any_sensor_cooling,
            target_percent: state.target_percent,
            target_rpm_per_fan: Vec::new(),
        };
        self.fans = state.fans;
    }

    fn send_request(&mut self, request: Request, fallback_status: String) {
        match ipc::send_request(request) {
            Ok(state) => self.apply_state(state),
            Err(error) => {
                self.status = format!("{fallback_status}: {error}");
            }
        }
    }

    fn update_curve_point(&mut self, index: usize, x: f64, y: f64, width: f64, height: f64) {
        let (temp_c, speed_percent) = pos_to_curve_values(plot_rect(width, height), x, y);
        let curve = &mut self.config.custom_curve;
        if index == 1 {
            curve[1] = CurvePoint { temp_c, speed_percent };
        } else {
            let (handle_u, handle_v) = handle_relative(curve);
            curve[index] = CurvePoint { temp_c, speed_percent };
            if index == 0 {
                curve[0].temp_c = curve[0].temp_c.min(curve[2].temp_c.saturating_sub(2));
            } else {
                curve[2].temp_c = curve[2].temp_c.max(curve[0].temp_c + 2);
            }
            curve[1] = handle_absolute(curve, handle_u, handle_v);
        }
        normalize_curve(curve);
        self.send_request(
            Request::SetCurve(self.config.soak_temp_c, self.config.custom_curve.clone()),
            String::from("Updating custom curve failed"),
        );
    }

    fn update_soak(&mut self, x: f64, width: f64, height: f64) {
        let (temp, _) = pos_to_curve_values(plot_rect(width, height), x, 0.0);
        resize_soak(&mut self.config, temp);
        self.send_request(
            Request::SetCurve(self.config.soak_temp_c, self.config.custom_curve.clone()),
            String::from("Updating system temperature target failed"),
        );
    }

    fn update_active(&mut self, enabled: bool) {
        self.send_request(
            Request::SetActive(enabled),
            String::from("Switching fan control mode failed"),
        );
    }

    fn update_any_sensor(&mut self, enabled: bool, temp_c: u8) {
        self.send_request(
            Request::SetAnySensor(enabled, temp_c.min(100)),
            String::from("Updating any-sensor protection failed"),
        );
    }

    fn update_system_cooling_time(&mut self, seconds: u16) {
        self.send_request(
            Request::SetSystemCoolingTime(seconds.min(600)),
            String::from("Updating minimum system cooling time failed"),
        );
    }

    fn update_curve_sensor(&mut self, key: Option<String>) {
        self.send_request(
            Request::SetCurveSensor(key),
            String::from("Updating optional curve sensor failed"),
        );
    }

    fn average_fan_percent(&self) -> Option<u8> {
        let mut total = 0u32;
        let mut count = 0u32;
        for fan in &self.fans {
            if let Some(rpm) = fan.current_speed {
                let span = fan.max_speed.saturating_sub(fan.min_speed);
                let percent = if span == 0 {
                    100
                } else {
                    ((rpm.saturating_sub(fan.min_speed)) * 100 / span).min(100)
                };
                total += percent;
                count += 1;
            }
        }
        if count == 0 {
            None
        } else {
            Some((total / count) as u8)
        }
    }

    fn average_fan_rpm(&self) -> Option<u32> {
        let speeds = self.fans.iter().filter_map(|fan| fan.current_speed).collect::<Vec<_>>();
        if speeds.is_empty() {
            None
        } else {
            Some(speeds.iter().sum::<u32>() / speeds.len() as u32)
        }
    }

    fn curve_points(&self) -> Vec<CurvePoint> {
        curve(&self.config)
    }
}
 
impl AppModel {
    fn push_history_sample(&mut self) {
        self.temperature_history.push_back(HistorySample {
            cpu_temp_c: self.snapshot.temperatures.cpu_temp_c,
            gpu_temp_c: self.snapshot.temperatures.gpu_temp_c,
            effective_temp_c: self.snapshot.effective_temp_c,
            system_temp_c: self.snapshot.system_temp_c,
            optional_curve_temp_c: self.snapshot.temperatures.optional_curve_temp_c,
        });
        if self.temperature_history.len() > HISTORY_CAPACITY {
            self.temperature_history.pop_front();
        }

        self.fan_percent_history
            .push_back(self.average_fan_percent().unwrap_or(0));
        if self.fan_percent_history.len() > HISTORY_CAPACITY {
            self.fan_percent_history.pop_front();
        }
    }
}

fn main() -> glib::ExitCode {
    if std::env::args().any(|arg| arg == "--daemon") {
        if let Err(error) = daemon_main() {
            eprintln!("{error}");
            return glib::ExitCode::from(1);
        }
        return glib::ExitCode::SUCCESS;
    }

    register_embedded_resources();
    let app = Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/org/t2fancontrol/gtk")
        .build();
    app.connect_activate(build_ui);
    app.run()
}

fn daemon_main() -> error::Result<()> {
    let listener = ipc::bind_listener()?;
    // SIGHUP arrives as None on the same channel so a sleeping daemon wakes
    let (connections_tx, connections_rx) = mpsc::channel();
    let hup_tx = connections_tx.clone();
    thread::spawn(move || loop {
        if connections_tx.send(Some(listener.accept())).is_err() {
            break;
        }
    });
    let mut signals = signal_hook::iterator::Signals::new([SIGHUP]).map_err(|source| {
        error::FanControlError::Io {
            path: std::path::PathBuf::from("<signal-hook>"),
            source,
        }
    })?;
    thread::spawn(move || {
        for _ in signals.forever() {
            if hup_tx.send(None).is_err() {
                break;
            }
        }
    });
    let mut runtime = DaemonRuntime::new()?;

    loop {
        runtime.tick();
        // With the SMC in charge the readings only feed the UI, so the
        // daemon sleeps until the UI asks again
        let next = if runtime.config.automatic_control_enabled || runtime.ui_active() {
            connections_rx.recv_timeout(runtime.controller.next_wakeup())
        } else {
            connections_rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(None) => runtime.reload_config()?,
            Ok(Some(Ok((stream, _addr)))) => {
                let request = ipc::read_request(&stream);
                match request {
                    Ok(request) => match runtime.handle_request(request) {
                        Ok(state) => {
                            let _ = ipc::write_response(&stream, &state);
                        }
                        Err(error) => {
                            let _ = ipc::write_error(&stream, &error.to_string());
                        }
                    },
                    Err(error) => {
                        let _ = ipc::write_error(&stream, &error.to_string());
                    }
                }
            }
            Ok(Some(Err(source))) => return Err(error::FanControlError::Io {
                path: std::path::PathBuf::from(ipc::SOCKET_PATH),
                source,
            }),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(
                error::FanControlError::ProcessSpawn(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "IPC listener stopped",
                )),
            ),
        }
    }
}

struct DaemonRuntime {
    config: AppConfig,
    controller: Controller,
    fans: Vec<FanEndpoint>,
    temperatures: Vec<TemperatureSource>,
    snapshot: ControlSnapshot,
    status: String,
    last_full_sensor_read: std::time::Instant,
    last_ui_request: Option<std::time::Instant>,
}

impl DaemonRuntime {
    fn new() -> error::Result<Self> {
        let mut config = AppConfig::load();
        config.autostart_enabled = service::autostart_enabled();
        config.save().map_err(|source| error::FanControlError::Io {
            path: std::path::PathBuf::from("/etc/t2-fancontrol/config.txt"),
            source,
        })?;

        let mut fans = discover_fans()?;
        for fan in &mut fans {
            let _ = fan.refresh_state();
        }
        let mut temperatures = discover_temperature_sources();
        let snapshot = ControlSnapshot {
            temperatures: TemperatureSnapshot::read_from(&mut temperatures),
            ..ControlSnapshot::default()
        };

        // The T2 SMC keeps manual fan mode across reboots, so native mode
        // has to hand the fans back explicitly
        let mut controller = Controller::new();
        let status = if config.automatic_control_enabled {
            String::new()
        } else {
            match controller.release_to_system(&mut fans) {
                Ok(()) => String::new(),
                Err(error) => format!("Handing fans to the SMC failed: {error}"),
            }
        };

        Ok(Self {
            config,
            controller,
            fans,
            temperatures,
            snapshot,
            status,
            last_full_sensor_read: std::time::Instant::now(),
            last_ui_request: None,
        })
    }

    fn ui_active(&self) -> bool {
        self.last_ui_request
            .is_some_and(|seen| seen.elapsed() <= Duration::from_secs(3))
    }

    fn tick(&mut self) {
        if self.controller.should_tick() {
            let ui_active = self.ui_active();
            if ui_active {
                for fan in &mut self.fans {
                    let _ = fan.refresh_state();
                }
            }
            let refresh_all_sensors = self.config.any_sensor_enabled
                || ui_active
                || self.last_full_sensor_read.elapsed() >= Duration::from_secs(10);
            match self.controller.tick(
                &self.config,
                &mut self.fans,
                &mut self.temperatures,
                refresh_all_sensors,
            )
            {
                Ok(snapshot) => {
                    self.snapshot = snapshot;
                    if refresh_all_sensors {
                        self.last_full_sensor_read = std::time::Instant::now();
                    }
                }
                Err(error) => {
                    self.status = format!("Fan control failed: {error}");
                }
            }
        }
        match self.controller.ramp(&mut self.fans) {
            Ok(true) => {
                self.snapshot.target_percent = self.controller.applied_percent();
                self.snapshot.target_rpm_per_fan = self.controller.target_rpm_per_fan(&self.fans);
            }
            Ok(false) => {}
            Err(error) => self.status = format!("Fan control failed: {error}"),
        }
    }

    fn reload_config(&mut self) -> error::Result<()> {
        let next_config = AppConfig::load();
        let was_active = self.config.automatic_control_enabled;
        let next_active = next_config.automatic_control_enabled;
        self.config = next_config;
        self.config.autostart_enabled = service::autostart_enabled();

        if was_active && !next_active {
            self.controller.release_to_system(&mut self.fans)?;
            self.snapshot.target_percent = None;
            self.snapshot.target_rpm_per_fan.clear();
        }
        if !was_active && next_active {
            self.controller = Controller::new();
        }
        self.status.clear();
        Ok(())
    }

    fn handle_request(&mut self, request: Request) -> error::Result<DaemonState> {
        self.last_ui_request = Some(std::time::Instant::now());
        // The status line only carries errors, a successful change clears it
        if !matches!(request, Request::GetState) {
            self.status.clear();
        }
        match request {
            Request::GetState => {}
            Request::SetActive(enabled) => {
                if !enabled {
                    self.controller.release_to_system(&mut self.fans)?;
                    self.snapshot.target_percent = None;
                    self.snapshot.target_rpm_per_fan.clear();
                    self.config.automatic_control_enabled = false;
                } else {
                    self.config.automatic_control_enabled = true;
                    self.controller = Controller::new();
                }
                self.save_config()?;
            }
            Request::SetAutostart(enabled) => {
                service::set_autostart(enabled)?;
                self.config.autostart_enabled = enabled;
                self.save_config()?;
            }
            Request::SetCurve(soak_temp_c, curve) => {
                self.config.soak_temp_c = soak_temp_c;
                self.config.custom_curve = curve;
                self.save_config()?;
            }
            Request::SetAnySensor(enabled, temp_c) => {
                self.config.any_sensor_enabled = enabled;
                self.config.any_sensor_temp_c = temp_c.min(100);
                self.save_config()?;
            }
            Request::SetSystemCoolingTime(seconds) => {
                self.config.system_cooling_time_s = seconds.min(600);
                self.save_config()?;
            }
            Request::SetCurveSensor(key) => {
                self.config.curve_sensor_key = key.filter(|key| {
                    self.temperatures.iter().any(|source| {
                        source.role == sysfs::TemperatureRole::System && source.key == *key
                    })
                });
                self.save_config()?;
            }
        }
        self.tick();
        Ok(self.state())
    }

    fn save_config(&mut self) -> error::Result<()> {
        self.config.save().map_err(|source| error::FanControlError::Io {
            path: std::path::PathBuf::from("/etc/t2-fancontrol/config.txt"),
            source,
        })
    }

    fn state(&self) -> DaemonState {
        ipc::state_from(
            &self.config,
            self.status.clone(),
            (
                self.snapshot.temperatures.cpu_temp_c,
                self.snapshot.temperatures.gpu_temp_c,
                self.snapshot.effective_temp_c,
                self.snapshot.temperatures.hottest_sensor_name.clone(),
                self.snapshot.temperatures.optional_curve_temp_c,
                self.snapshot.temperatures.overall_hottest_temp_c,
                self.snapshot.temperatures.overall_hottest_sensor_name.clone(),
                self.snapshot.system_temp_c,
                self.snapshot.system_sensor_count,
                self.snapshot.temperatures.monitored_sensor_count,
                self.snapshot.heat_soak_cooling,
                self.snapshot.any_sensor_cooling,
            ),
            self.snapshot.target_percent,
            &self.fans,
            &self.temperatures,
        )
    }
}

fn build_ui(app: &Application) {
    let css_provider = install_css();
    let model = Rc::new(RefCell::new(AppModel::new()));
    let ui = Rc::new(build_widgets());
    let dragged_point = Rc::new(RefCell::new(None::<CurveDragTarget>));
    let drag_origin = Rc::new(RefCell::new(None::<(f64, f64)>));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("T2 Fan Control")
        .default_width(380 + 2 * (style::SIDE_MARGIN - 8))
        .default_height(640)
        .build();
    window.set_resizable(true);

    let root = GtkBox::new(Orientation::Vertical, 8);
    style::pad(&root);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Fan Control", "")));
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    ui.control_switch.set_valign(Align::Center);
    header.pack_end(&ui.control_switch);
    window.set_titlebar(Some(&header));

    ui.status.set_halign(Align::Start);
    ui.status.set_wrap(true);
    ui.status.set_xalign(0.0);
    ui.status.add_css_class("meta-text");

    let summary = Grid::builder()
        .column_spacing(12)
        .row_spacing(4)
        .column_homogeneous(true)
        .hexpand(true)
        .build();
    attach_pair_at(&summary, 0, 0, "CPU", &ui.cpu_label);
    attach_pair_at(&summary, 2, 0, "GPU", &ui.gpu_label);
    attach_pair_at(&summary, 0, 1, "Curve temp", &ui.effective_label);
    attach_pair_at(&summary, 2, 1, "Fan target", &ui.target_label);
    attach_pair_at(&summary, 0, 2, "Fans", &ui.fan_label);
    attach_pair_at(&summary, 2, 2, "System", &ui.system_label);
    attach_pair_at(&summary, 0, 3, "Curve sensor", &ui.hottest_sensor_label);
    attach_wide_pair_at(&summary, 4, "Hottest sensor", &ui.overall_hottest_sensor_label);
    attach_wide_pair_at(&summary, 5, "100% reason", &ui.full_speed_reason_label);
    root.append(&summary);

    install_delayed_tooltip(
        &ui.full_speed_reason_label,
        "Shows which protection or curve input is currently requesting 100% fan speed. System chill cooldown includes its overrun and hysteresis hold.",
    );

    let cooling_time_row = GtkBox::new(Orientation::Horizontal, 8);
    cooling_time_row.set_halign(Align::End);
    cooling_time_row.append(&Label::new(Some("Overrun time")));
    cooling_time_row.append(&unit_entry(&ui.system_cooling_time, "s"));

    let any_sensor_row = GtkBox::new(Orientation::Horizontal, 8);
    any_sensor_row.set_halign(Align::End);
    any_sensor_row.append(&ui.any_sensor_check);
    any_sensor_row.append(&unit_entry(&ui.any_sensor_temp, "°C"));

    let curve_sensor_row = GtkBox::new(Orientation::Horizontal, 8);
    curve_sensor_row.set_halign(Align::End);
    curve_sensor_row.append(&Label::new(Some("Optional curve sensor")));
    curve_sensor_row.append(&ui.curve_sensor_dropdown);

    let advanced_content = GtkBox::new(Orientation::Vertical, 6);
    advanced_content.append(&cooling_time_row);
    advanced_content.append(&any_sensor_row);
    advanced_content.append(&curve_sensor_row);
    ui.advanced.add_css_class("secondary-control");
    ui.advanced.set_child(Some(&advanced_content));
    root.append(&ui.advanced);

    install_delayed_tooltip(&ui.system_cooling_time,
        "Minimum time for forced system cooling. Release still requires 20 seconds below System chill minus 5 C."
    );
    install_delayed_tooltip(&ui.any_sensor_check,
        "Optionally force both fans to 100% when any positive temperature reaches the selected threshold."
    );
    install_delayed_tooltip(&ui.any_sensor_temp, "Any-sensor protection threshold from 0 to 100 C.");
    install_delayed_tooltip(&ui.curve_sensor_dropdown,
        "Optionally add one board sensor to the fixed CPU/dGPU curve inputs. The hottest of the available inputs controls the curve."
    );
    root.append(&dynamic_panel(&ui.curve_title, &ui.curve_area, &ui.curve_legend));
    let temperature_title = Label::new(Some("Temperatures"));
    root.append(&dynamic_panel(&temperature_title, &ui.temperature_graph, &ui.temperature_legend));
    root.append(&panel("Fans", &ui.fan_graph, Some("Fans")));

    let details_title = Label::new(Some("Fans"));
    details_title.set_halign(Align::Start);
    details_title.add_css_class("panel-title");
    root.append(&details_title);
    ui.details_label.set_halign(Align::Start);
    ui.details_label.set_wrap(true);
    ui.details_label.set_xalign(0.0);
    ui.details_label.add_css_class("details-text");
    root.append(&ui.details_label);
    root.append(&ui.status);

    let footer = GtkBox::new(Orientation::Horizontal, 8);
    let donate = LinkButton::builder()
        .uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400")
        .label("Fund our bugs")
        .build();
    donate.set_halign(Align::Start);
    donate.add_css_class("footer-link");
    footer.append(&donate);
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    let version = Label::new(Some(&format!("v{APP_VERSION}")));
    version.set_halign(Align::End);
    version.add_css_class("footer-version");
    footer.append(&version);
    root.append(&footer);

    window.set_child(Some(&root));
    watch_theme_changes(&css_provider, &ui);

    connect_drawings(&model, &ui, &dragged_point, &drag_origin);
    sync_ui(&model.borrow(), &ui);

    {
        let model = model.clone();
        let ui_refs = ui.clone();
        ui.control_switch.connect_active_notify(move |switch| {
            if ui_refs.syncing.get() { return; }
            model.borrow_mut().update_active(switch.is_active());
            // Falls back to the daemon state if the request failed
            sync_ui(&model.borrow(), &ui_refs);
        });
    }
    {
        let model = model.clone();
        let ui_refs = ui.clone();
        ui.any_sensor_check.connect_toggled(move |check| {
            if ui_refs.syncing.get() { return; }
            model.borrow_mut().update_any_sensor(check.is_active(), entry_value(&ui_refs.any_sensor_temp, 100) as u8);
            sync_ui(&model.borrow(), &ui_refs);
        });
    }
    {
        let model = model.clone();
        let ui_refs = ui.clone();
        ui.curve_sensor_dropdown.connect_selected_notify(move |dropdown| {
            if ui_refs.syncing.get() { return; }
            // Position 0 is "None", the sensors follow in sensor_choice_keys order
            let key = dropdown.selected().checked_sub(1)
                .and_then(|index| ui_refs.sensor_choice_keys.borrow().get(index as usize).cloned());
            model.borrow_mut().update_curve_sensor(key);
            sync_ui(&model.borrow(), &ui_refs);
        });
    }
    {
        let model = model.clone();
        let ui_refs = ui.clone();
        ui.system_cooling_time.connect_changed(move |entry| {
            if ui_refs.syncing.get() { return; }
            let Some(value) = parse_entry_value(entry, 600) else { return; };
            model.borrow_mut().update_system_cooling_time(value);
            sync_ui(&model.borrow(), &ui_refs);
        });
    }
    {
        let model = model.clone();
        let ui_refs = ui.clone();
        ui.any_sensor_temp.connect_changed(move |entry| {
            if ui_refs.syncing.get() { return; }
            let Some(value) = parse_entry_value(entry, 100) else { return; };
            model.borrow_mut().update_any_sensor(ui_refs.any_sensor_check.is_active(), value as u8);
            sync_ui(&model.borrow(), &ui_refs);
        });
    }

    {
        let model = model.clone();
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_millis(900), move || {
            let mut model = model.borrow_mut();
            model.tick();
            sync_ui(&model, &ui);
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

fn build_widgets() -> UiRefs {
    let make_value = || {
        let label = Label::new(None);
        label.set_halign(Align::Start);
        label.set_xalign(0.0);
        label.set_max_width_chars(18);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.add_css_class("metric-value");
        label
    };

    let curve_area = DrawingArea::new();
    curve_area.set_content_width(248);
    curve_area.set_content_height(138);
    curve_area.set_vexpand(false);
    curve_area.set_hexpand(true);

    let temperature_graph = DrawingArea::new();
    temperature_graph.set_content_width(248);
    temperature_graph.set_content_height(88);
    temperature_graph.set_vexpand(false);
    temperature_graph.set_hexpand(true);

    let fan_graph = DrawingArea::new();
    fan_graph.set_content_width(248);
    fan_graph.set_content_height(88);
    fan_graph.set_vexpand(false);
    fan_graph.set_hexpand(true);

    let any_sensor_check = CheckButton::with_label("Force 100% if any sensor reaches");
    let any_sensor_temp = Entry::new();
    any_sensor_temp.set_width_chars(4);
    any_sensor_temp.set_max_width_chars(4);
    any_sensor_temp.set_input_purpose(gtk4::InputPurpose::Digits);
    let system_cooling_time = Entry::new();
    system_cooling_time.set_width_chars(4);
    system_cooling_time.set_max_width_chars(4);
    system_cooling_time.set_input_purpose(gtk4::InputPurpose::Digits);
    let any_sensor_focus = EventControllerFocus::new();
    any_sensor_temp.add_controller(any_sensor_focus.clone());
    let system_cooling_focus = EventControllerFocus::new();
    system_cooling_time.add_controller(system_cooling_focus.clone());
    let curve_sensor_dropdown = DropDown::new(Some(StringList::new(&["None"])), gtk4::Expression::NONE);

    UiRefs {
        status: make_value(),
        cpu_label: make_value(),
        gpu_label: make_value(),
        effective_label: make_value(),
        hottest_sensor_label: make_value(),
        overall_hottest_sensor_label: make_value(),
        full_speed_reason_label: make_value(),
        system_label: make_value(),
        target_label: make_value(),
        fan_label: make_value(),
        details_label: make_value(),
        control_switch: Switch::new(),
        advanced: Expander::new(Some("Advanced settings")),
        any_sensor_check,
        any_sensor_temp,
        any_sensor_focus,
        system_cooling_time,
        system_cooling_focus,
        curve_sensor_dropdown,
        sensor_choice_keys: RefCell::new(Vec::new()),
        curve_title: Label::new(None),
        curve_legend: Label::new(None),
        temperature_legend: Label::new(None),
        curve_area,
        temperature_graph,
        fan_graph,
        syncing: Cell::new(false),
    }
}

fn connect_drawings(
    model: &Rc<RefCell<AppModel>>,
    ui: &Rc<UiRefs>,
    dragged_point: &Rc<RefCell<Option<CurveDragTarget>>>,
    drag_origin: &Rc<RefCell<Option<(f64, f64)>>>,
) {
    install_curve_hover(&ui.curve_area, model, dragged_point);

    {
        let model = model.clone();
        ui.curve_area.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            draw_curve_panel(&model, cr, width as f64, height as f64);
        });
    }
    {
        let model = model.clone();
        ui.temperature_graph
            .set_draw_func(move |_area, cr, width, height| {
                let model = model.borrow();
                draw_temperature_history(&model, cr, width as f64, height as f64);
            });
    }
    {
        let model = model.clone();
        ui.fan_graph.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            draw_fan_history(&model, cr, width as f64, height as f64);
        });
    }

    let gesture = GestureDrag::new();
    {
        let model = model.clone();
        let dragged_point = dragged_point.clone();
        let drag_origin = drag_origin.clone();
        let area = ui.curve_area.clone();
        gesture.connect_drag_begin(move |_gesture, x, y| {
            let model = model.borrow();
            let curve = &model.config.custom_curve;
            let width = area.width() as f64;
            let height = area.height() as f64;
            let plot = plot_rect(width, height);
            let target = curve_hit_target(curve, model.config.soak_temp_c, plot, x, y)
                .filter(|_| model.config.automatic_control_enabled);
            *drag_origin.borrow_mut() = match target {
                Some(CurveDragTarget::Point(index)) => Some(curve_to_pos(plot, &curve[index])),
                Some(CurveDragTarget::SoakWall) => Some((x, y)),
                None => None,
            };
            area.set_cursor_from_name(match target {
                Some(CurveDragTarget::Point(_)) => Some("grabbing"),
                Some(CurveDragTarget::SoakWall) => Some("ew-resize"),
                None => None,
            });
            *dragged_point.borrow_mut() = target;
        });
    }
    {
        let model = model.clone();
        let ui = ui.clone();
        let dragged_point = dragged_point.clone();
        let drag_origin = drag_origin.clone();
        let area = ui.curve_area.clone();
        gesture.connect_drag_update(move |_gesture, offset_x, offset_y| {
            let Some(target) = *dragged_point.borrow() else {
                return;
            };
            let Some(start) = *drag_origin.borrow() else {
                return;
            };
            let x = start.0 + offset_x;
            let y = start.1 + offset_y;
            {
                let mut model = model.borrow_mut();
                match target {
                    CurveDragTarget::Point(index) => model.update_curve_point(index, x, y, area.width() as f64, area.height() as f64),
                    CurveDragTarget::SoakWall => model.update_soak(x, area.width() as f64, area.height() as f64),
                }
                sync_ui(&model, &ui);
            }
        });
    }
    {
        let dragged_point = dragged_point.clone();
        let drag_origin = drag_origin.clone();
        let area = ui.curve_area.clone();
        gesture.connect_drag_end(move |_, _, _| {
            *dragged_point.borrow_mut() = None;
            *drag_origin.borrow_mut() = None;
            area.set_cursor_from_name(None);
        });
    }
    ui.curve_area.add_controller(gesture);
}

fn curve_hit_target(
    curve: &[CurvePoint],
    soak_temp_c: u8,
    plot: (f64, f64, f64, f64),
    x: f64,
    y: f64,
) -> Option<CurveDragTarget> {
    let point = curve
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_dist = squared_distance(curve_to_pos(plot, left), (x, y));
            let right_dist = squared_distance(curve_to_pos(plot, right), (x, y));
            left_dist
                .partial_cmp(&right_dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|(_, point)| squared_distance(curve_to_pos(plot, point), (x, y)) <= 14.0_f64.powi(2))
        .map(|(index, _)| CurveDragTarget::Point(index));
    if point.is_some() {
        return point;
    }

    let over_wall = y >= plot.1
        && y <= plot.1 + plot.3
        && (temperature_to_x(plot, soak_temp_c) - x).abs() <= 12.0;
    over_wall.then_some(CurveDragTarget::SoakWall)
}

fn install_curve_hover(
    area: &DrawingArea,
    model: &Rc<RefCell<AppModel>>,
    dragged_point: &Rc<RefCell<Option<CurveDragTarget>>>,
) {
    let tooltip = "System chill is the average-temperature target. Forced cooling starts at target +2 C and releases at target -2 C after the overrun and stability windows. Drag the vertical wall to change it.";
    let popover = Popover::new();
    popover.set_autohide(false);
    popover.set_has_arrow(true);
    popover.set_position(PositionType::Bottom);
    let label = Label::new(Some(tooltip));
    label.set_wrap(true);
    label.set_max_width_chars(48);
    label.set_margin_top(6);
    label.set_margin_bottom(6);
    label.set_margin_start(8);
    label.set_margin_end(8);
    popover.set_child(Some(&label));
    popover.set_parent(area);

    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let over_wall = Rc::new(Cell::new(false));
    let motion = EventControllerMotion::new();
    {
        let area = area.clone();
        let model = model.clone();
        let popover = popover.clone();
        let pending = pending.clone();
        let over_wall = over_wall.clone();
        let dragged_point = dragged_point.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(target) = *dragged_point.borrow() {
                area.set_cursor_from_name(Some(match target {
                    CurveDragTarget::Point(_) => "grabbing",
                    CurveDragTarget::SoakWall => "ew-resize",
                }));
                over_wall.set(false);
                if let Some(source) = pending.borrow_mut().take() { source.remove(); }
                popover.popdown();
                return;
            }
            let model = model.borrow();
            let plot = plot_rect(area.width() as f64, area.height() as f64);
            let target = curve_hit_target(&model.config.custom_curve, model.config.soak_temp_c, plot, x, y);
            area.set_cursor_from_name(match target {
                Some(CurveDragTarget::Point(_)) => Some("grab"),
                Some(CurveDragTarget::SoakWall) => Some("ew-resize"),
                None => None,
            });

            let is_over_wall = matches!(target, Some(CurveDragTarget::SoakWall));
            if is_over_wall == over_wall.replace(is_over_wall) {
                return;
            }
            if let Some(source) = pending.borrow_mut().take() { source.remove(); }
            if !is_over_wall {
                popover.popdown();
                return;
            }

            let wall_x = temperature_to_x(plot, model.config.soak_temp_c).round() as i32;
            popover.set_pointing_to(Some(&gdk::Rectangle::new(wall_x - 1, y.round() as i32, 2, 2)));
            let popover = popover.clone();
            let pending_for_timeout = pending.clone();
            let over_wall_for_timeout = over_wall.clone();
            let source = glib::timeout_add_local_once(Duration::from_secs(2), move || {
                pending_for_timeout.borrow_mut().take();
                if over_wall_for_timeout.get() {
                    popover.popup();
                }
            });
            *pending.borrow_mut() = Some(source);
        });
    }
    {
        let area = area.clone();
        let popover = popover.clone();
        let pending = pending.clone();
        let over_wall = over_wall.clone();
        motion.connect_leave(move |_| {
            over_wall.set(false);
            if let Some(source) = pending.borrow_mut().take() { source.remove(); }
            popover.popdown();
            area.set_cursor_from_name(None);
        });
    }
    area.add_controller(motion);
}

fn panel(
    title: &str,
    widget: &impl IsA<gtk4::Widget>,
    legend: Option<&str>,
) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Vertical, 6);
    let label = Label::new(Some(title));
    label.set_halign(Align::Start);
    label.add_css_class("panel-title");
    box_.append(&label);
    box_.append(&style::card(widget));
    if let Some(legend) = legend {
        let legend_label = Label::new(None);
        legend_label.set_halign(Align::Start);
        legend_label.set_xalign(0.0);
        legend_label.add_css_class("meta-text");
        legend_label.set_markup(&legend_markup(legend));
        box_.append(&legend_label);
    }
    box_
}

fn unit_entry(entry: &Entry, unit: &str) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Horizontal, 3);
    box_.add_css_class("inline-input");
    entry.add_css_class("inline-input-entry");
    let suffix = Label::new(Some(unit));
    suffix.add_css_class("inline-input-unit");
    box_.append(entry);
    box_.append(&suffix);
    box_
}

fn install_delayed_tooltip(widget: &impl IsA<gtk4::Widget>, text: &str) {
    let popover = Popover::new();
    popover.set_autohide(false);
    popover.set_has_arrow(true);
    popover.set_position(PositionType::Bottom);
    let label = Label::new(Some(text));
    label.set_wrap(true);
    label.set_max_width_chars(48);
    label.set_margin_top(6);
    label.set_margin_bottom(6);
    label.set_margin_start(8);
    label.set_margin_end(8);
    popover.set_child(Some(&label));
    popover.set_parent(widget);

    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let motion = EventControllerMotion::new();
    {
        let popover = popover.clone();
        let pending = pending.clone();
        motion.connect_enter(move |_, _, _| {
            if let Some(source) = pending.borrow_mut().take() { source.remove(); }
            let popover = popover.clone();
            let pending_for_timeout = pending.clone();
            let source = glib::timeout_add_local_once(Duration::from_secs(2), move || {
                pending_for_timeout.borrow_mut().take();
                popover.popup();
            });
            *pending.borrow_mut() = Some(source);
        });
    }
    {
        let popover = popover.clone();
        let pending = pending.clone();
        motion.connect_leave(move |_| {
            if let Some(source) = pending.borrow_mut().take() { source.remove(); }
            popover.popdown();
        });
    }
    widget.add_controller(motion);
}

fn dynamic_panel(title: &Label, widget: &impl IsA<gtk4::Widget>, legend: &Label) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Vertical, 6);
    title.set_halign(Align::Start);
    title.add_css_class("panel-title");
    legend.set_halign(Align::Start);
    legend.set_xalign(0.0);
    legend.add_css_class("meta-text");
    box_.append(title);
    box_.append(&style::card(widget));
    box_.append(legend);
    box_
}

fn attach_pair_at(grid: &Grid, column: i32, row: i32, title: &str, value: &Label) {
    let key = Label::new(Some(title));
    key.set_halign(Align::Start);
    key.set_xalign(0.0);
    key.add_css_class("metric-key");
    key.set_hexpand(true);
    key.set_max_width_chars(12);
    key.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    key.set_tooltip_text(Some(title));
    value.set_hexpand(true);
    value.set_max_width_chars(14);
    value.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    grid.attach(&key, column, row, 1, 1);
    grid.attach(value, column + 1, row, 1, 1);
}

fn attach_wide_pair_at(grid: &Grid, row: i32, title: &str, value: &Label) {
    let key = Label::new(Some(title));
    key.set_halign(Align::Start);
    key.set_xalign(0.0);
    key.add_css_class("metric-key");
    key.set_tooltip_text(Some(title));
    value.set_hexpand(true);
    value.set_max_width_chars(-1);
    value.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    grid.attach(&key, 0, row, 1, 1);
    grid.attach(value, 1, row, 3, 1);
}

fn sync_ui(model: &AppModel, ui: &UiRefs) {
    ui.syncing.set(true);
    ui.status.set_label(&model.status);
    ui.status.set_visible(!model.status.is_empty());
    ui.cpu_label.set_label(&format_temp(model.snapshot.temperatures.cpu_temp_c));
    ui.gpu_label.set_label(&format_temp(model.snapshot.temperatures.gpu_temp_c));
    ui.effective_label
        .set_label(&format_temp(model.snapshot.effective_temp_c));
    ui.system_label.set_label(&format_temp(model.snapshot.system_temp_c));
    ui.hottest_sensor_label.set_label(
        model.snapshot.temperatures.hottest_sensor_name.as_deref().unwrap_or("unavailable")
    );
    ui.hottest_sensor_label.set_tooltip_text(model.snapshot.temperatures.hottest_sensor_name.as_deref());
    let overall_hottest = match (
        model.snapshot.temperatures.overall_hottest_sensor_name.as_deref(),
        model.snapshot.temperatures.overall_hottest_temp_c,
    ) {
        (Some(name), Some(temp)) => format!("{name} · {temp} °C"),
        _ => String::from("unavailable"),
    };
    ui.overall_hottest_sensor_label.set_label(&overall_hottest);
    ui.overall_hottest_sensor_label.set_tooltip_text(Some(&overall_hottest));
    let full_speed_reason = full_speed_reason(model);
    ui.full_speed_reason_label.set_label(&full_speed_reason);
    ui.full_speed_reason_label.set_tooltip_text(Some(&full_speed_reason));

    let cpu_available = model.snapshot.temperatures.cpu_temp_c.is_some();
    let gpu_available = model.snapshot.temperatures.gpu_temp_c.is_some();
    let mut descriptors = match (cpu_available, gpu_available) {
        (true, true) => (
            "Fan curve: hotter of CPU/GPU",
            "Curve  •  System  •  CPU  •  GPU  •  CPU/GPU max",
            "CPU  •  GPU  •  CPU/GPU max  •  System",
        ),
        (true, false) => (
            "Fan curve: CPU temperature",
            "Curve  •  System  •  CPU",
            "CPU  •  System",
        ),
        (false, true) => (
            "Fan curve: GPU temperature",
            "Curve  •  System  •  GPU",
            "GPU  •  System",
        ),
        (false, false) => (
            "Fan curve: no CPU/GPU sensor",
            "Curve  •  System",
            "System",
        ),
    };
    if model.config.curve_sensor_key.is_some() {
        descriptors = match (cpu_available, gpu_available) {
            (true, true) => (
                "Fan curve: CPU/GPU plus optional sensor",
                "Curve  •  System  •  CPU  •  GPU  •  CPU/GPU max  •  Optional",
                "CPU  •  GPU  •  CPU/GPU max  •  Optional  •  System",
            ),
            (true, false) => (
                "Fan curve: CPU plus optional sensor",
                "Curve  •  System  •  CPU  •  Optional",
                "CPU  •  Optional  •  System",
            ),
            (false, true) => (
                "Fan curve: GPU plus optional sensor",
                "Curve  •  System  •  GPU  •  Optional",
                "GPU  •  Optional  •  System",
            ),
            (false, false) => (
                "Fan curve: optional sensor",
                "Curve  •  System  •  Optional",
                "Optional  •  System",
            ),
        };
    }
    if !model.config.automatic_control_enabled {
        descriptors.0 = "Fan curve: native SMC";
    }
    ui.curve_title.set_label(descriptors.0);
    ui.curve_legend.set_markup(&legend_markup(descriptors.1));
    ui.temperature_legend.set_markup(&legend_markup(descriptors.2));
    ui.target_label.set_label(
        &model
            .snapshot
            .target_percent
            .map(|value| if model.snapshot.heat_soak_cooling && model.snapshot.any_sensor_cooling {
                format!("{value}% · limits")
            } else if model.snapshot.heat_soak_cooling {
                format!("{value}% · system")
            } else if model.snapshot.any_sensor_cooling {
                format!("{value}% · any sensor")
            } else {
                format!("{value}%")
            })
            .unwrap_or_else(|| String::from("system managed")),
    );
    let active = model.config.automatic_control_enabled;
    ui.control_switch.set_active(active);
    ui.advanced.set_sensitive(active);
    ui.curve_area.set_sensitive(active);
    if !active {
        ui.target_label.set_label("native SMC");
    }
    ui.any_sensor_check.set_active(model.config.any_sensor_enabled);
    if !ui.any_sensor_focus.contains_focus() {
        ui.any_sensor_temp.set_text(&model.config.any_sensor_temp_c.to_string());
    }
    ui.any_sensor_temp.set_sensitive(model.config.any_sensor_enabled);
    if !ui.system_cooling_focus.contains_focus() {
        ui.system_cooling_time.set_text(&model.config.system_cooling_time_s.to_string());
    }
    let choice_keys = model.sensor_choices.iter().map(|sensor| sensor.key.clone()).collect::<Vec<_>>();
    if *ui.sensor_choice_keys.borrow() != choice_keys {
        let names = std::iter::once("None")
            .chain(model.sensor_choices.iter().map(|sensor| sensor.name.as_str()))
            .collect::<Vec<_>>();
        ui.curve_sensor_dropdown.set_model(Some(&StringList::new(&names)));
        *ui.sensor_choice_keys.borrow_mut() = choice_keys;
    }
    let selected = model.config.curve_sensor_key.as_deref()
        .and_then(|key| ui.sensor_choice_keys.borrow().iter().position(|choice| choice == key))
        .map_or(0, |index| index as u32 + 1);
    ui.curve_sensor_dropdown.set_selected(selected);
    // Percent only means something against the custom curve
    let fan_average = if model.config.automatic_control_enabled {
        model.average_fan_percent().map(|value| format!("{value}% average"))
    } else {
        model.average_fan_rpm().map(|value| format!("{value} rpm average"))
    };
    ui.fan_label.set_label(&fan_average.unwrap_or_else(|| String::from("unavailable")));

    let mut details = if model.fans.is_empty() {
        String::from("No T2 fan endpoints found.")
    } else {
        model
            .fans
            .iter()
            .map(|fan| {
                format!(
                    "{}  {} RPM  {}-{}  {}",
                    fan.name,
                    fan.current_speed
                        .map(|rpm| rpm.to_string())
                        .unwrap_or_else(|| String::from("unknown")),
                    fan.min_speed,
                    fan.max_speed,
                    fan.app_controlled
                        .map(|manual| {
                            if manual {
                                "app controlled"
                            } else {
                                "system controlled"
                            }
                        })
                        .unwrap_or("unknown")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    details.push_str(&format!(
        "\nMonitoring {} temperature sensors.",
        model.snapshot.temperatures.monitored_sensor_count
    ));
    ui.details_label.set_label(&details);

    ui.curve_area.queue_draw();
    ui.temperature_graph.queue_draw();
    ui.fan_graph.queue_draw();
    ui.syncing.set(false);
}

fn full_speed_reason(model: &AppModel) -> String {
    if !model.config.automatic_control_enabled {
        return String::from("disabled");
    }
    let mut reasons = Vec::new();
    if model.snapshot.heat_soak_cooling {
        let phase = match model.snapshot.system_temp_c {
            Some(temp) if temp >= model.config.soak_temp_c.saturating_add(2) => "limit reached",
            _ => "cooldown",
        };
        reasons.push(match model.snapshot.system_temp_c {
            Some(temp) => format!("System chill {phase} · {temp} C average"),
            None => format!("System chill {phase}"),
        });
    }
    if model.snapshot.any_sensor_cooling {
        reasons.push(match (
            model.snapshot.temperatures.overall_hottest_sensor_name.as_deref(),
            model.snapshot.temperatures.overall_hottest_temp_c,
        ) {
            (Some(name), Some(temp)) => format!("Any sensor · {name} · {temp} °C"),
            _ => String::from("Any-sensor protection"),
        });
    }
    if reasons.is_empty() && model.snapshot.target_percent == Some(100) {
        reasons.push(match (
            model.snapshot.temperatures.hottest_sensor_name.as_deref(),
            model.snapshot.effective_temp_c,
        ) {
            (Some(name), Some(temp)) => format!("Fan curve · {name} · {temp} °C"),
            _ => String::from("Fan curve"),
        });
    }
    if reasons.is_empty()
        && model.snapshot.target_percent.is_none()
        && model.average_fan_percent().is_some_and(|percent| percent >= 99)
    {
        reasons.push(String::from("System / firmware control"));
    }
    if reasons.is_empty() {
        String::from("Not active")
    } else {
        reasons.join(" + ")
    }
}

fn draw_curve_panel(model: &AppModel, cr: &cairo::Context, width: f64, height: f64) {
    let palette = current_palette();
    let plot = draw_panel(cr, width, height);
    draw_grid(cr, plot);
    draw_curve_scale_labels(cr, plot);

    let curve = model.curve_points();
    // Native SMC mode greys out the custom curve, live markers stay in color
    cr.push_group();
    let wall_x = temperature_to_x(plot, model.config.soak_temp_c);
    let release_x = temperature_to_x(plot, model.config.soak_temp_c.saturating_sub(2));
    let engage_x = temperature_to_x(plot, model.config.soak_temp_c.saturating_add(2));
    cr.set_source_rgba(0.90, 0.28, 0.22, 0.08);
    cr.rectangle(release_x, plot.1, engage_x - release_x, plot.3);
    let _ = cr.fill();
    cr.set_source_rgba(0.90, 0.28, 0.22, 0.05);
    cr.rectangle(engage_x, plot.1, plot.0 + plot.2 - engage_x, plot.3);
    let _ = cr.fill();
    cr.set_source_rgba(0.90, 0.28, 0.22, 0.88);
    cr.set_line_width(2.0);
    cr.move_to(wall_x, plot.1);
    cr.line_to(wall_x, plot.1 + plot.3);
    let _ = cr.stroke();
    draw_wall_temperature(cr, plot, wall_x, model.config.soak_temp_c);
    draw_curve_line(cr, plot, &curve, palette.curve, 2.2);

    for (index, point) in curve.iter().enumerate() {
        let (x, y) = curve_to_pos(plot, point);
        cr.set_source_rgba(0.95, 0.98, 1.0, 0.14);
        cr.arc(x, y, 7.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgb(0.96, 0.98, 1.0);
        cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        set_color(cr, palette.curve);
        cr.arc(x, y, 3.0, 0.0, std::f64::consts::TAU);
        if index == 1 {
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        } else {
            let _ = cr.fill();
        }
    }
    if cr.pop_group_to_source().is_ok() {
        let _ = cr.paint_with_alpha(if model.config.automatic_control_enabled { 1.0 } else { 0.25 });
    }

    draw_live_marker(
        cr,
        plot,
        model.snapshot.temperatures.cpu_temp_c,
        model.average_fan_percent().or(model.snapshot.target_percent),
        palette.cpu,
    );
    draw_live_marker(
        cr,
        plot,
        model.snapshot.temperatures.gpu_temp_c,
        model.average_fan_percent().or(model.snapshot.target_percent),
        palette.gpu,
    );
    draw_live_marker(
        cr,
        plot,
        model.snapshot.effective_temp_c,
        model.snapshot.target_percent.or(model.average_fan_percent()),
        palette.effective,
    );
    draw_live_marker(
        cr,
        plot,
        model.snapshot.system_temp_c,
        model.snapshot.target_percent.or(model.average_fan_percent()),
        palette.system,
    );
    draw_live_marker(
        cr,
        plot,
        model.snapshot.temperatures.optional_curve_temp_c,
        model.snapshot.target_percent.or(model.average_fan_percent()),
        palette.optional,
    );
}

fn draw_live_marker(
    cr: &cairo::Context,
    plot: (f64, f64, f64, f64),
    temp_c: Option<u8>,
    speed_percent: Option<u8>,
    color: (f64, f64, f64),
) {
    let (Some(temp_c), Some(speed_percent)) = (temp_c, speed_percent) else {
        return;
    };
    let (x, y) = curve_to_pos(
        plot,
        &CurvePoint {
            temp_c,
            speed_percent,
        },
    );
    cr.set_source_rgba(color.0, color.1, color.2, 0.22);
    cr.arc(x, y, 6.5, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    set_color(cr, color);
    cr.arc(x, y, 3.7, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn draw_temperature_history(model: &AppModel, cr: &cairo::Context, width: f64, height: f64) {
    let palette = current_palette();
    let plot = draw_panel(cr, width, height);
    draw_grid(cr, plot);
    draw_history_scale_labels(cr, plot, "°C");
    draw_history_series(
        cr,
        plot,
        &model
            .temperature_history
            .iter()
            .map(|sample| sample.cpu_temp_c)
            .collect::<Vec<_>>(),
        palette.cpu,
    );
    draw_history_series(
        cr,
        plot,
        &model
            .temperature_history
            .iter()
            .map(|sample| sample.gpu_temp_c)
            .collect::<Vec<_>>(),
        palette.gpu,
    );
    draw_history_series(
        cr,
        plot,
        &model
            .temperature_history
            .iter()
            .map(|sample| sample.effective_temp_c)
            .collect::<Vec<_>>(),
        palette.effective,
    );
    draw_history_series(
        cr,
        plot,
        &model
            .temperature_history
            .iter()
            .map(|sample| sample.optional_curve_temp_c)
            .collect::<Vec<_>>(),
        palette.optional,
    );
    draw_history_series(
        cr,
        plot,
        &model
            .temperature_history
            .iter()
            .map(|sample| sample.system_temp_c)
            .collect::<Vec<_>>(),
        palette.system,
    );
}

fn draw_fan_history(model: &AppModel, cr: &cairo::Context, width: f64, height: f64) {
    let palette = current_palette();
    let plot = draw_panel(cr, width, height);
    draw_grid(cr, plot);
    draw_history_scale_labels(cr, plot, "%");
    let series = model
        .fan_percent_history
        .iter()
        .copied()
        .map(Some)
        .collect::<Vec<_>>();
    draw_history_series(cr, plot, &series, palette.fan);
}

fn draw_history_series(
    cr: &cairo::Context,
    plot: (f64, f64, f64, f64),
    values: &[Option<u8>],
    color: (f64, f64, f64),
) {
    let denominator = (HISTORY_CAPACITY.saturating_sub(1)).max(1) as f64;
    let start_index = HISTORY_CAPACITY.saturating_sub(values.len());
    let mut first = true;
    cr.set_source_rgba(color.0, color.1, color.2, 0.18);
    let mut filled = false;
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        let x = plot.0 + plot.2 * ((start_index + index) as f64 / denominator);
        let y = remap(*value as f64, 0.0, 100.0, plot.1 + plot.3, plot.1);
        if !filled {
            cr.move_to(x, plot.1 + plot.3);
            cr.line_to(x, y);
            filled = true;
        } else {
            cr.line_to(x, y);
        }
    }
    if filled {
        cr.line_to(plot.0 + plot.2, plot.1 + plot.3);
        cr.close_path();
        let _ = cr.fill();
    }

    set_color(cr, color);
    cr.set_line_width(2.0);

    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            first = true;
            continue;
        };
        let x = plot.0 + plot.2 * ((start_index + index) as f64 / denominator);
        let y = remap(*value as f64, 0.0, 100.0, plot.1 + plot.3, plot.1);
        if first {
            cr.move_to(x, y);
            first = false;
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();
}

fn draw_curve_line(
    cr: &cairo::Context,
    plot: (f64, f64, f64, f64),
    curve: &[CurvePoint],
    color: (f64, f64, f64),
    line_width: f64,
) {
    let (Some(first), Some(last)) = (curve.first(), curve.last()) else {
        return;
    };
    set_color(cr, color);
    cr.set_line_width(line_width);
    let (start_x, _) = curve_to_pos(plot, first);
    let (end_x, _) = curve_to_pos(plot, last);
    let steps = (end_x - start_x).ceil().max(1.0) as usize;
    for step in 0..=steps {
        let x = start_x + (end_x - start_x) * step as f64 / steps as f64;
        let temp = remap(x, plot.0, plot.0 + plot.2, 0.0, MAX_SOAK_TEMP_C as f64);
        let y = remap(curve_percent(curve, temp), 0.0, 100.0, plot.1 + plot.3, plot.1);
        if step == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    cr.line_to(end_x, plot.1);
    let _ = cr.stroke();
}

/// Handle position relative to the start/end box, so moving an end point keeps the bend.
fn handle_relative(curve: &[CurvePoint]) -> (f64, f64) {
    let (start, handle, end) = (&curve[0], &curve[1], &curve[2]);
    let u = remap(handle.temp_c as f64, start.temp_c as f64, end.temp_c as f64, 0.0, 1.0);
    let v = if start.speed_percent == end.speed_percent {
        0.5
    } else {
        remap(handle.speed_percent as f64, start.speed_percent as f64, end.speed_percent as f64, 0.0, 1.0)
    };
    (u, v)
}

fn handle_absolute(curve: &[CurvePoint], u: f64, v: f64) -> CurvePoint {
    let (start, end) = (&curve[0], &curve[2]);
    let (low, high) = speed_range(curve);
    CurvePoint {
        temp_c: remap(u, 0.0, 1.0, start.temp_c as f64, end.temp_c as f64).round() as u8,
        speed_percent: (remap(v, 0.0, 1.0, start.speed_percent as f64, end.speed_percent as f64).round() as u8).clamp(low, high),
    }
}

fn draw_panel(_cr: &cairo::Context, width: f64, height: f64) -> (f64, f64, f64, f64) {
    plot_rect(width, height)
}

fn draw_grid(cr: &cairo::Context, plot: (f64, f64, f64, f64)) {
    set_color(cr, style::graph_colors().grid);
    cr.set_line_width(0.8);
    for step in 1..4 {
        let x = plot.0 + plot.2 * (step as f64 / 4.0);
        let y = plot.1 + plot.3 * (step as f64 / 4.0);
        cr.move_to(x, plot.1);
        cr.line_to(x, plot.1 + plot.3);
        cr.move_to(plot.0, y);
        cr.line_to(plot.0 + plot.2, y);
    }
    let _ = cr.stroke();
}

fn draw_curve_scale_labels(cr: &cairo::Context, plot: (f64, f64, f64, f64)) {
    draw_label(cr, plot.0, plot.1 + 8.0, "100%", 9.0, 0.70);
    draw_label(cr, plot.0, plot.1 + plot.3 - 2.0, "0 °C", 9.0, 0.70);
    draw_label(
        cr,
        plot.0 + plot.2 - 24.0,
        plot.1 + plot.3 - 2.0,
        &format!("{} °C", MAX_SOAK_TEMP_C),
        9.0,
        0.70,
    );
}

fn draw_wall_temperature(
    cr: &cairo::Context,
    plot: (f64, f64, f64, f64),
    wall_x: f64,
    soak_temp_c: u8,
) {
    let text = format!("{soak_temp_c} °C");
    let estimated_width = text.chars().count() as f64 * 5.5;
    let x = if wall_x + estimated_width + 6.0 <= plot.0 + plot.2 {
        wall_x + 6.0
    } else {
        wall_x - estimated_width - 6.0
    };
    draw_label(cr, x, plot.1 + 10.0, &text, 9.0, 0.85);
}

fn draw_history_scale_labels(cr: &cairo::Context, plot: (f64, f64, f64, f64), unit: &str) {
    draw_label(cr, plot.0, plot.1 + 8.0, &format!("100{unit}"), 9.0, 0.68);
    draw_label(
        cr,
        plot.0,
        plot.1 + plot.3 - 2.0,
        &format!("0{unit}"),
        9.0,
        0.68,
    );
}

fn plot_rect(width: f64, height: f64) -> (f64, f64, f64, f64) {
    (10.0, 10.0, (width - 20.0).max(10.0), (height - 20.0).max(10.0))
}

fn curve_to_pos(plot: (f64, f64, f64, f64), point: &CurvePoint) -> (f64, f64) {
    (
        temperature_to_x(plot, point.temp_c),
        remap(
            point.speed_percent as f64,
            0.0,
            100.0,
            plot.1 + plot.3,
            plot.1,
        ),
    )
}

fn temperature_to_x(plot: (f64, f64, f64, f64), temp_c: u8) -> f64 {
    remap(temp_c.min(MAX_SOAK_TEMP_C) as f64, 0.0, MAX_SOAK_TEMP_C as f64, plot.0, plot.0 + plot.2)
}

fn pos_to_curve_values(plot: (f64, f64, f64, f64), x: f64, y: f64) -> (u8, u8) {
    let x = x.clamp(plot.0, plot.0 + plot.2);
    let y = y.clamp(plot.1, plot.1 + plot.3);
    let temp = remap(x, plot.0, plot.0 + plot.2, 0.0, MAX_SOAK_TEMP_C as f64).round() as u8;
    let speed = remap(y, plot.1 + plot.3, plot.1, 0.0, 100.0).round() as u8;
    (temp, speed)
}

fn remap(value: f64, src_min: f64, src_max: f64, dst_min: f64, dst_max: f64) -> f64 {
    if (src_max - src_min).abs() <= f64::EPSILON {
        return dst_min;
    }
    let t = (value - src_min) / (src_max - src_min);
    dst_min + t * (dst_max - dst_min)
}

fn squared_distance(left: (f64, f64), right: (f64, f64)) -> f64 {
    let dx = left.0 - right.0;
    let dy = left.1 - right.1;
    dx * dx + dy * dy
}

fn set_color(cr: &cairo::Context, color: (f64, f64, f64)) {
    cr.set_source_rgb(color.0, color.1, color.2);
}

fn legend_markup(legend: &str) -> String {
    legend
        .split("  •  ")
        .map(|item| {
            let (dot, text) = match item {
                "Curve" => (current_palette().curve, "Curve"),
                "CPU" => (current_palette().cpu, "CPU"),
                "GPU" => (current_palette().gpu, "GPU"),
                "CPU/GPU max" => (current_palette().effective, "CPU/GPU max"),
                "System" => (current_palette().system, "System"),
                "Optional" => (current_palette().optional, "Optional"),
                "System target" => ((0.90, 0.28, 0.22), "System chill"),
                "Fans" => (current_palette().fan, "Fans"),
                _ => (current_palette().curve, item),
            };
            format!(
                "<span foreground=\"{}\">●</span> {}",
                rgb_to_hex(dot),
                glib::markup_escape_text(text)
            )
        })
        .collect::<Vec<_>>()
        .join("    ")
}

fn rgb_to_hex(color: (f64, f64, f64)) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (color.0 * 255.0).round() as u8,
        (color.1 * 255.0).round() as u8,
        (color.2 * 255.0).round() as u8
    )
}

fn draw_label(cr: &cairo::Context, x: f64, y: f64, text: &str, _size: f64, _alpha: f64) {
    let label = style::graph_colors().label;
    cr.select_font_face(
        "JetBrains Mono",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(10.0);
    cr.set_source_rgba(label.0, label.1, label.2, 1.0);
    cr.move_to(x, y);
    let _ = cr.show_text(text);
}

fn install_css() -> CssProvider {
    style::install();
    let provider = CssProvider::new();
    provider.load_from_string(&build_css(current_palette()));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}

fn register_embedded_resources() {
    gio::resources_register_include!("t2-fancontrol.gresource")
        .expect("failed to register embedded GTK resources");
}

fn format_temp(value: Option<u8>) -> String {
    value
        .map(|temp| format!("{temp} °C"))
        .unwrap_or_else(|| String::from("unavailable"))
}

fn parse_entry_value(entry: &Entry, maximum: u16) -> Option<u16> {
    entry.text().trim().parse::<u16>().ok().map(|value| value.min(maximum))
}

fn entry_value(entry: &Entry, maximum: u16) -> u16 {
    parse_entry_value(entry, maximum).unwrap_or(maximum)
}

fn current_palette() -> ThemePalette {
    if is_dark_theme() {
        DARK_PALETTE
    } else {
        LIGHT_PALETTE
    }
}

fn is_dark_theme() -> bool {
    StyleManager::default().is_dark()
}

fn build_css(palette: ThemePalette) -> String {
    format!(
        ".top-strip {{
  border-spacing: 0;
}}
.panel-title {{
  font-weight: 600;
}}
.toggle-chip,
.preset-chip {{
  background: transparent;
  background-image: none;
  color: {meta};
  border: 1px solid {border};
  border-radius: 8px;
  box-shadow: none;
  outline: none;
}}
.toggle-chip {{
  padding: 5px 10px;
}}
.preset-chip {{
  padding: 5px 8px;
}}
.toggle-chip:hover,
.preset-chip:hover {{
  background: {hover};
}}
.toggle-chip:checked,
.preset-chip:checked {{
  background: {checked};
  color: {meta};
  border-color: {checked_border};
}}
.meta-text,
.metric-key,
.secondary-control > title > label,
.footer-link,
.footer-version {{
  color: {meta};
}}
.metric-value,
.details-text,
.inline-input-entry,
.inline-input-unit {{
  font-family: \"JetBrains Mono\";
  font-feature-settings: \"tnum\" 1;
}}
.details-text {{
  line-height: 1.28;
}}
.inline-input {{
  background: transparent;
  border: 1px solid currentColor;
  border-radius: 6px;
  padding: 0 6px;
}}
.inline-input-entry {{
  background: transparent;
  background-image: none;
  border: none;
  box-shadow: none;
  padding: 4px 0;
}}
.inline-input-unit {{
  opacity: 0.68;
}}
combobox button {{
  background: transparent;
  background-image: none;
}}",
        meta = palette.meta_fg,
        border = palette.chip_border,
        hover = palette.chip_hover_bg,
        checked = palette.chip_checked_bg,
        checked_border = palette.chip_checked_border,
    )
}

fn watch_theme_changes(provider: &CssProvider, ui: &UiRefs) {
    let style_manager = StyleManager::default();
    let provider_dark = provider.clone();
    let curve = ui.curve_area.clone();
    let temp = ui.temperature_graph.clone();
    let fan = ui.fan_graph.clone();
    style_manager.connect_dark_notify(move |_| {
        provider_dark.load_from_string(&build_css(current_palette()));
        curve.queue_draw();
        temp.queue_draw();
        fan.queue_draw();
    });
}
