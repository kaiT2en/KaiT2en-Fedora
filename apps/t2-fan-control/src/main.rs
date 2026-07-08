mod config;
mod control;
mod error;
mod ipc;
mod service;
mod sysfs;

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    io::ErrorKind,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use config::{curve_for_preset, normalize_curve, AppConfig, CurvePoint, PresetKind};
use control::{ControlSnapshot, Controller};
use adw::StyleManager;
use gtk4::{
    cairo,
    gio,
    glib,
    prelude::*,
    Align, Application, ApplicationWindow, Box as GtkBox, CssProvider, DrawingArea, GestureDrag,
    Grid, Label, LinkButton, Orientation, ToggleButton, gdk,
};
use ipc::{DaemonState, Request};
use signal_hook::consts::signal::SIGHUP;
use sysfs::{discover_fans, discover_temperature_sources, FanEndpoint, TemperatureSnapshot, TemperatureSource};

const APP_ID: &str = "org.t2fancontrol.gtk";
const APP_VERSION: &str = "0.04";
const HISTORY_CAPACITY: usize = 90;

#[derive(Clone, Copy)]
struct ThemePalette {
    window_bg: &'static str,
    window_fg: &'static str,
    chip_fg: &'static str,
    chip_border: &'static str,
    chip_hover_bg: &'static str,
    chip_checked_bg: &'static str,
    chip_checked_border: &'static str,
    meta_fg: &'static str,
    details_fg: &'static str,
    panel_fill: (f64, f64, f64),
    panel_border: (f64, f64, f64),
    grid: (f64, f64, f64),
    cpu: (f64, f64, f64),
    gpu: (f64, f64, f64),
    effective: (f64, f64, f64),
    fan: (f64, f64, f64),
    curve: (f64, f64, f64),
    label: (f64, f64, f64),
}

const DARK_PALETTE: ThemePalette = ThemePalette {
    window_bg: "#181818",
    window_fg: "#efefef",
    chip_fg: "#cfcfcf",
    chip_border: "rgba(210,210,210,0.24)",
    chip_hover_bg: "rgba(255,255,255,0.04)",
    chip_checked_bg: "rgba(255,255,255,0.10)",
    chip_checked_border: "rgba(232,232,232,0.45)",
    meta_fg: "rgba(230,230,230,0.62)",
    details_fg: "rgba(232,232,232,0.74)",
    panel_fill: (0.115, 0.115, 0.115),
    panel_border: (0.90, 0.90, 0.92),
    grid: (0.28, 0.30, 0.32),
    cpu: (0.92, 0.54, 0.28),
    gpu: (0.34, 0.60, 0.86),
    effective: (0.68, 0.82, 0.42),
    fan: (0.50, 0.76, 0.58),
    curve: (0.86, 0.86, 0.88),
    label: (0.93, 0.96, 0.99),
};

const LIGHT_PALETTE: ThemePalette = ThemePalette {
    window_bg: "#f4f1ea",
    window_fg: "#1f1e1b",
    chip_fg: "#3d392f",
    chip_border: "rgba(73,66,55,0.24)",
    chip_hover_bg: "rgba(42,34,24,0.05)",
    chip_checked_bg: "rgba(42,34,24,0.10)",
    chip_checked_border: "rgba(42,34,24,0.40)",
    meta_fg: "rgba(52,48,43,0.68)",
    details_fg: "rgba(40,37,32,0.82)",
    panel_fill: (0.955, 0.945, 0.925),
    panel_border: (0.34, 0.30, 0.25),
    grid: (0.78, 0.75, 0.70),
    cpu: (0.86, 0.43, 0.18),
    gpu: (0.20, 0.45, 0.75),
    effective: (0.42, 0.62, 0.18),
    fan: (0.30, 0.58, 0.38),
    curve: (0.20, 0.20, 0.22),
    label: (0.18, 0.18, 0.18),
};

#[derive(Clone, Copy, Debug, Default)]
struct HistorySample {
    cpu_temp_c: Option<u8>,
    gpu_temp_c: Option<u8>,
    effective_temp_c: Option<u8>,
}

struct UiRefs {
    status: Label,
    cpu_label: Label,
    gpu_label: Label,
    effective_label: Label,
    target_label: Label,
    fan_label: Label,
    details_label: Label,
    preset_quiet: ToggleButton,
    preset_balanced: ToggleButton,
    preset_performance: ToggleButton,
    preset_custom: ToggleButton,
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
            status: String::from("Waiting for daemon"),
            fans: Vec::new(),
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
        self.config.active_preset = state.active_preset;
        self.config.automatic_control_enabled = state.control_active;
        self.config.autostart_enabled = state.autostart_enabled;
        self.config.custom_curve = state.custom_curve;
        self.snapshot = ControlSnapshot {
            temperatures: TemperatureSnapshot {
                cpu_temp_c: state.cpu_temp_c,
                gpu_temp_c: state.gpu_temp_c,
            },
            effective_temp_c: state.effective_temp_c,
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

    fn set_preset(&mut self, preset: PresetKind) {
        self.send_request(
            Request::SetPreset(preset),
            String::from("Updating preset failed"),
        );
    }

    fn update_curve_point(&mut self, index: usize, x: f64, y: f64, width: f64, height: f64) {
        if self.config.active_preset != PresetKind::Custom {
            return;
        }
        if let Some(point) = self.config.custom_curve.get_mut(index) {
            let plot = plot_rect(width, height);
            let (temp_c, speed_percent) = pos_to_curve_values(plot, x, y);
            point.temp_c = temp_c;
            point.speed_percent = speed_percent;
            normalize_curve(&mut self.config.custom_curve);
            self.send_request(
                Request::SetCurve(self.config.custom_curve.clone()),
                String::from("Updating custom curve failed"),
            );
        }
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

    fn curve_points(&self) -> Vec<CurvePoint> {
        curve_for_preset(&self.config)
    }
}
 
impl AppModel {
    fn push_history_sample(&mut self) {
        self.temperature_history.push_back(HistorySample {
            cpu_temp_c: self.snapshot.temperatures.cpu_temp_c,
            gpu_temp_c: self.snapshot.temperatures.gpu_temp_c,
            effective_temp_c: self.snapshot.effective_temp_c,
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
    let reload_requested = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGHUP, reload_requested.clone()).map_err(|source| {
        error::FanControlError::Io {
            path: std::path::PathBuf::from("<signal-hook>"),
            source,
        }
    })?;
    let mut runtime = DaemonRuntime::new()?;

    loop {
        if reload_requested.swap(false, Ordering::Relaxed) {
            runtime.reload_config()?;
        }

        runtime.tick();
        loop {
            match listener.accept() {
                Ok((stream, _addr)) => {
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
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    return Err(error::FanControlError::Io {
                        path: std::path::PathBuf::from(ipc::SOCKET_PATH),
                        source: error,
                    });
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
}

struct DaemonRuntime {
    config: AppConfig,
    controller: Controller,
    fans: Vec<FanEndpoint>,
    temperatures: Vec<TemperatureSource>,
    snapshot: ControlSnapshot,
    status: String,
}

impl DaemonRuntime {
    fn new() -> error::Result<Self> {
        let mut config = AppConfig::load();
        config.autostart_enabled = service::autostart_enabled();

        let mut fans = discover_fans()?;
        for fan in &mut fans {
            let _ = fan.refresh_state();
        }
        let mut temperatures = discover_temperature_sources();
        let snapshot = ControlSnapshot {
            temperatures: TemperatureSnapshot::read_from(&mut temperatures),
            ..ControlSnapshot::default()
        };

        Ok(Self {
            config,
            controller: Controller::new(),
            fans,
            temperatures,
            snapshot,
            status: String::from("Daemon ready"),
        })
    }

    fn tick(&mut self) {
        if self.controller.should_tick() {
            match self
                .controller
                .tick(&self.config, &mut self.fans, &mut self.temperatures)
            {
                Ok(snapshot) => {
                    self.snapshot = snapshot;
                }
                Err(error) => {
                    self.status = format!("Fan control failed: {error}");
                }
            }
        } else {
            for fan in &mut self.fans {
                let _ = fan.refresh_state();
            }
            self.snapshot.temperatures = TemperatureSnapshot::read_from(&mut self.temperatures);
            self.snapshot.effective_temp_c = self.snapshot.temperatures.effective_temp_c();
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
        self.status = String::from("Configuration reloaded");
        Ok(())
    }

    fn handle_request(&mut self, request: Request) -> error::Result<DaemonState> {
        match request {
            Request::GetState => {}
            Request::SetActive(enabled) => {
                if !enabled {
                    self.controller.release_to_system(&mut self.fans)?;
                    self.snapshot.target_percent = None;
                    self.snapshot.target_rpm_per_fan.clear();
                    self.config.automatic_control_enabled = false;
                    self.status = String::from("Fan control inactive");
                } else {
                    self.config.automatic_control_enabled = true;
                    self.controller = Controller::new();
                    self.status = format!(
                        "Fan control active with {} preset",
                        self.config.active_preset.as_str()
                    );
                }
                self.save_config()?;
            }
            Request::SetAutostart(enabled) => {
                service::set_autostart(enabled)?;
                self.config.autostart_enabled = enabled;
                self.status = if enabled {
                    String::from("Autostart enabled through systemd")
                } else {
                    String::from("Autostart disabled")
                };
                self.save_config()?;
            }
            Request::SetPreset(preset) => {
                self.config.active_preset = preset;
                self.status = format!("Preset changed to {}", preset.as_str());
                self.save_config()?;
            }
            Request::SetCurve(curve) => {
                self.config.custom_curve = curve;
                self.status = String::from("Custom preset updated");
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
            ),
            self.snapshot.target_percent,
            &self.fans,
        )
    }
}

fn build_ui(app: &Application) {
    let css_provider = install_css();
    let model = Rc::new(RefCell::new(AppModel::new()));
    let ui = Rc::new(build_widgets());
    let dragged_point = Rc::new(RefCell::new(None::<usize>));
    let drag_origin = Rc::new(RefCell::new(None::<(f64, f64)>));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("T2 Fan Control")
        .default_width(248)
        .default_height(640)
        .build();
    window.set_resizable(true);

    let root = GtkBox::new(Orientation::Vertical, 8);
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(8);
    root.set_margin_end(8);

    let header = GtkBox::new(Orientation::Vertical, 0);
    ui.status.set_halign(Align::Start);
    ui.status.set_wrap(true);
    ui.status.set_xalign(0.0);
    ui.status.add_css_class("meta-text");
    header.append(&ui.status);
    root.append(&header);

    let preset_grid = Grid::builder()
        .column_spacing(6)
        .row_spacing(6)
        .hexpand(true)
        .build();
    ui.preset_quiet.set_hexpand(true);
    ui.preset_quiet.set_size_request(112, -1);
    ui.preset_balanced.set_hexpand(true);
    ui.preset_balanced.set_size_request(112, -1);
    ui.preset_performance.set_hexpand(true);
    ui.preset_performance.set_size_request(112, -1);
    ui.preset_custom.set_hexpand(true);
    ui.preset_custom.set_size_request(112, -1);
    preset_grid.attach(&ui.preset_quiet, 0, 0, 1, 1);
    preset_grid.attach(&ui.preset_balanced, 1, 0, 1, 1);
    preset_grid.attach(&ui.preset_performance, 0, 1, 1, 1);
    preset_grid.attach(&ui.preset_custom, 1, 1, 1, 1);
    root.append(&preset_grid);

    let summary = Grid::builder()
        .column_spacing(12)
        .row_spacing(4)
        .hexpand(true)
        .build();
    attach_pair_at(&summary, 0, 0, "CPU", &ui.cpu_label);
    attach_pair_at(&summary, 2, 0, "GPU", &ui.gpu_label);
    attach_pair_at(&summary, 0, 1, "Effective", &ui.effective_label);
    attach_pair_at(&summary, 2, 1, "Target", &ui.target_label);
    attach_pair_at(&summary, 0, 2, "Fans", &ui.fan_label);
    root.append(&summary);

    root.append(&panel(
        "Curve",
        &ui.curve_area,
        Some("Curve  •  CPU  •  GPU  •  Effective"),
    ));
    root.append(&panel(
        "Temperatures",
        &ui.temperature_graph,
        Some("CPU  •  GPU  •  Effective"),
    ));
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

    let footer = GtkBox::new(Orientation::Horizontal, 8);
    let donate = LinkButton::builder()
        .uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400")
        .label("Fund my bugs")
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
        let ui = ui.clone();
        connect_preset_button(&ui.preset_quiet, PresetKind::Quiet, &model, &ui);
        connect_preset_button(&ui.preset_balanced, PresetKind::Balanced, &model, &ui);
        connect_preset_button(&ui.preset_performance, PresetKind::Performance, &model, &ui);
        connect_preset_button(&ui.preset_custom, PresetKind::Custom, &model, &ui);
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
        label.add_css_class("metric-value");
        label
    };

    let preset_quiet = ToggleButton::with_label(PresetKind::Quiet.ui_label());
    let preset_balanced = ToggleButton::with_label(PresetKind::Balanced.ui_label());
    let preset_performance = ToggleButton::with_label(PresetKind::Performance.ui_label());
    let preset_custom = ToggleButton::with_label(PresetKind::Custom.ui_label());
    for button in [
        &preset_quiet,
        &preset_balanced,
        &preset_performance,
        &preset_custom,
    ] {
        button.add_css_class("preset-chip");
    }

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

    UiRefs {
        status: make_value(),
        cpu_label: make_value(),
        gpu_label: make_value(),
        effective_label: make_value(),
        target_label: make_value(),
        fan_label: make_value(),
        details_label: make_value(),
        preset_quiet,
        preset_balanced,
        preset_performance,
        preset_custom,
        curve_area,
        temperature_graph,
        fan_graph,
        syncing: Cell::new(false),
    }
}

fn connect_drawings(
    model: &Rc<RefCell<AppModel>>,
    ui: &Rc<UiRefs>,
    dragged_point: &Rc<RefCell<Option<usize>>>,
    drag_origin: &Rc<RefCell<Option<(f64, f64)>>>,
) {
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
            if model.config.active_preset != PresetKind::Custom {
                *dragged_point.borrow_mut() = None;
                *drag_origin.borrow_mut() = None;
                return;
            }
            let curve = &model.config.custom_curve;
            let width = area.allocated_width() as f64;
            let height = area.allocated_height() as f64;
            let plot = plot_rect(width, height);
            let selected = curve
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    let left_pos = curve_to_pos(plot, left);
                    let right_pos = curve_to_pos(plot, right);
                    let left_dist = squared_distance(left_pos, (x, y));
                    let right_dist = squared_distance(right_pos, (x, y));
                    left_dist
                        .partial_cmp(&right_dist)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(index, point)| (index, point));
            if let Some((index, point)) = selected {
                *dragged_point.borrow_mut() = Some(index);
                *drag_origin.borrow_mut() = Some(curve_to_pos(plot, point));
            } else {
                *dragged_point.borrow_mut() = None;
                *drag_origin.borrow_mut() = None;
            }
        });
    }
    {
        let model = model.clone();
        let ui = ui.clone();
        let dragged_point = dragged_point.clone();
        let drag_origin = drag_origin.clone();
        let area = ui.curve_area.clone();
        gesture.connect_drag_update(move |_gesture, offset_x, offset_y| {
            let Some(index) = *dragged_point.borrow() else {
                return;
            };
            let Some(start) = *drag_origin.borrow() else {
                return;
            };
            let x = start.0 + offset_x;
            let y = start.1 + offset_y;
            {
                let mut model = model.borrow_mut();
                model.update_curve_point(
                    index,
                    x,
                    y,
                    area.allocated_width() as f64,
                    area.allocated_height() as f64,
                );
                sync_ui(&model, &ui);
            }
        });
    }
    {
        let dragged_point = dragged_point.clone();
        let drag_origin = drag_origin.clone();
        gesture.connect_drag_end(move |_, _, _| {
            *dragged_point.borrow_mut() = None;
            *drag_origin.borrow_mut() = None;
        });
    }
    ui.curve_area.add_controller(gesture);
}

fn panel(
    title: &str,
    widget: &impl IsA<gtk4::Widget>,
    legend: Option<&str>,
) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Vertical, 3);
    let label = Label::new(Some(title));
    label.set_halign(Align::Start);
    label.add_css_class("panel-title");
    box_.append(&label);
    box_.append(widget);
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

fn attach_pair_at(grid: &Grid, column: i32, row: i32, title: &str, value: &Label) {
    let key = Label::new(Some(title));
    key.set_halign(Align::Start);
    key.set_xalign(0.0);
    key.add_css_class("metric-key");
    grid.attach(&key, column, row, 1, 1);
    grid.attach(value, column + 1, row, 1, 1);
}

fn sync_ui(model: &AppModel, ui: &UiRefs) {
    ui.syncing.set(true);
    ui.status.set_label(&model.status);
    ui.cpu_label.set_label(&format_temp(model.snapshot.temperatures.cpu_temp_c));
    ui.gpu_label.set_label(&format_temp(model.snapshot.temperatures.gpu_temp_c));
    ui.effective_label
        .set_label(&format_temp(model.snapshot.effective_temp_c));
    ui.target_label.set_label(
        &model
            .snapshot
            .target_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| String::from("system managed")),
    );
    ui.fan_label.set_label(
        &model
            .average_fan_percent()
            .map(|value| format!("{value}% average"))
            .unwrap_or_else(|| String::from("unavailable")),
    );

    let details = if model.fans.is_empty() {
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
    ui.details_label.set_label(&details);

    ui.preset_quiet
        .set_active(model.config.active_preset == PresetKind::Quiet);
    ui.preset_balanced
        .set_active(model.config.active_preset == PresetKind::Balanced);
    ui.preset_performance
        .set_active(model.config.active_preset == PresetKind::Performance);
    ui.preset_custom
        .set_active(model.config.active_preset == PresetKind::Custom);

    ui.curve_area.queue_draw();
    ui.temperature_graph.queue_draw();
    ui.fan_graph.queue_draw();
    ui.syncing.set(false);
}

fn draw_curve_panel(model: &AppModel, cr: &cairo::Context, width: f64, height: f64) {
    let palette = current_palette();
    let plot = draw_panel(cr, width, height);
    draw_grid(cr, plot);
    draw_curve_scale_labels(cr, plot);

    let curve = model.curve_points();
    draw_curve_line(cr, plot, &curve, palette.curve, 2.2);

    for point in &curve {
        let (x, y) = curve_to_pos(plot, point);
        cr.set_source_rgba(0.95, 0.98, 1.0, 0.14);
        cr.arc(x, y, 7.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgb(0.96, 0.98, 1.0);
        cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        set_color(cr, palette.curve);
        cr.arc(x, y, 3.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
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
    draw_history_scale_labels(cr, plot, "C");
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
    if curve.is_empty() {
        return;
    }
    set_color(cr, color);
    cr.set_line_width(line_width);
    for (index, point) in curve.iter().enumerate() {
        let (x, y) = curve_to_pos(plot, point);
        if index == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();
}

fn draw_panel(_cr: &cairo::Context, width: f64, height: f64) -> (f64, f64, f64, f64) {
    let palette = current_palette();
    _cr.set_source_rgb(palette.panel_fill.0, palette.panel_fill.1, palette.panel_fill.2);
    _cr.rectangle(0.5, 0.5, width - 1.0, height - 1.0);
    let _ = _cr.fill();
    _cr.set_source_rgba(
        palette.panel_border.0,
        palette.panel_border.1,
        palette.panel_border.2,
        0.20,
    );
    _cr.set_line_width(1.0);
    _cr.rectangle(0.5, 0.5, width - 1.0, height - 1.0);
    let _ = _cr.stroke();
    plot_rect(width, height)
}

fn draw_grid(cr: &cairo::Context, plot: (f64, f64, f64, f64)) {
    set_color(cr, current_palette().grid);
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
    draw_label(cr, plot.0, plot.1 + plot.3 - 2.0, "20 C", 9.0, 0.70);
    draw_label(
        cr,
        plot.0 + plot.2 - 24.0,
        plot.1 + plot.3 - 2.0,
        "100 C",
        9.0,
        0.70,
    );
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
        remap(point.temp_c as f64, 20.0, 100.0, plot.0, plot.0 + plot.2),
        remap(
            point.speed_percent as f64,
            0.0,
            100.0,
            plot.1 + plot.3,
            plot.1,
        ),
    )
}

fn pos_to_curve_values(plot: (f64, f64, f64, f64), x: f64, y: f64) -> (u8, u8) {
    let x = x.clamp(plot.0, plot.0 + plot.2);
    let y = y.clamp(plot.1, plot.1 + plot.3);
    let temp = remap(x, plot.0, plot.0 + plot.2, 20.0, 100.0).round() as u8;
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
                "Effective" => (current_palette().effective, "Effective"),
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

fn draw_label(cr: &cairo::Context, x: f64, y: f64, text: &str, size: f64, alpha: f64) {
    let palette = current_palette();
    cr.select_font_face(
        "Sans",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(size);
    cr.set_source_rgba(palette.label.0, palette.label.1, palette.label.2, alpha);
    cr.move_to(x, y);
    let _ = cr.show_text(text);
}

fn install_css() -> CssProvider {
    let provider = CssProvider::new();
    provider.load_from_data(&build_css(current_palette()));
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

fn connect_preset_button(
    button: &ToggleButton,
    preset: PresetKind,
    model: &Rc<RefCell<AppModel>>,
    ui: &Rc<UiRefs>,
) {
    let model = model.clone();
    let ui = ui.clone();
    let button = button.clone();
    button.connect_toggled(move |btn| {
        if ui.syncing.get() {
            return;
        }
        if !btn.is_active() {
            ui.syncing.set(true);
            btn.set_active(true);
            ui.syncing.set(false);
            return;
        }
        let mut model = model.borrow_mut();
        model.set_preset(preset);
        match preset {
            PresetKind::Quiet => {
                ui.preset_balanced.set_active(false);
                ui.preset_performance.set_active(false);
                ui.preset_custom.set_active(false);
            }
            PresetKind::Balanced => {
                ui.preset_quiet.set_active(false);
                ui.preset_performance.set_active(false);
                ui.preset_custom.set_active(false);
            }
            PresetKind::Performance => {
                ui.preset_quiet.set_active(false);
                ui.preset_balanced.set_active(false);
                ui.preset_custom.set_active(false);
            }
            PresetKind::Custom => {
                ui.preset_quiet.set_active(false);
                ui.preset_balanced.set_active(false);
                ui.preset_performance.set_active(false);
            }
        }
        sync_ui(&model, &ui);
    });
}

fn format_temp(value: Option<u8>) -> String {
    value
        .map(|temp| format!("{temp} C"))
        .unwrap_or_else(|| String::from("unavailable"))
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
        "window {{
  background: {};
  color: {};
}}
.top-strip {{
  border-spacing: 0;
}}
.panel-title {{
  font-weight: 700;
  letter-spacing: 0.01em;
}}
.toggle-chip {{
  padding: 5px 10px;
  background: transparent;
  background-image: none;
  color: {};
  border: 1px solid {};
  border-radius: 8px;
  box-shadow: none;
  outline: none;
}}
.toggle-chip:hover {{
  background: {};
}}
.toggle-chip:checked {{
  background: {};
  color: {};
  border-color: {};
}}
.preset-chip {{
  padding: 5px 8px;
  background: transparent;
  background-image: none;
  color: {};
  border: 1px solid {};
  border-radius: 8px;
  box-shadow: none;
  outline: none;
}}
.preset-chip:hover {{
  background: {};
}}
.preset-chip:checked {{
  background: {};
  color: {};
  border-color: {};
}}
.toggle-chip label,
.preset-chip label {{
  font-weight: 600;
}}
.meta-text {{
  color: {};
  font-size: 0.9em;
}}
.metric-key {{
  color: {};
}}
.metric-value {{
  font-weight: 650;
}}
.details-text {{
  color: {};
  font-family: Monospace;
  line-height: 1.28;
}}
.footer-link,
.footer-version {{
  color: {};
  font-size: 0.88em;
}}",
        palette.window_bg,
        palette.window_fg,
        palette.chip_fg,
        palette.chip_border,
        palette.chip_hover_bg,
        palette.chip_checked_bg,
        palette.window_fg,
        palette.chip_checked_border,
        palette.chip_fg,
        palette.chip_border,
        palette.chip_hover_bg,
        palette.chip_checked_bg,
        palette.window_fg,
        palette.chip_checked_border,
        palette.meta_fg,
        palette.meta_fg,
        palette.details_fg,
        palette.meta_fg,
    )
}

fn watch_theme_changes(provider: &CssProvider, ui: &UiRefs) {
    let style_manager = StyleManager::default();
    let provider_dark = provider.clone();
    let curve = ui.curve_area.clone();
    let temp = ui.temperature_graph.clone();
    let fan = ui.fan_graph.clone();
    style_manager.connect_dark_notify(move |_| {
        provider_dark.load_from_data(&build_css(current_palette()));
        curve.queue_draw();
        temp.queue_draw();
        fan.queue_draw();
    });
}
