// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use adw::StyleManager;
use gtk4::{
    cairo, gdk, gio, glib, prelude::*, Align, Application, ApplicationWindow, Box as GtkBox, CssProvider,
    DrawingArea, Grid, Label, LinkButton, Orientation,
};

use crate::{
    plimit::PLimits,
    sensors::{Applied, SmcSensors},
    style,
    sysfs::{find_t2smc, glob_paths, read_string},
};

const APP_ID: &str = "org.t2zonepower.gtk";
const APP_VERSION: &str = "0.03";
const HISTORY_CAPACITY: usize = 90;

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

#[derive(Clone, Copy)]
struct ThemePalette {
    meta_fg: &'static str,
    heatpipe: (f64, f64, f64),
    cpu: (f64, f64, f64),
    gpu: (f64, f64, f64),
    idle: (f64, f64, f64),
    fin: (f64, f64, f64),
    igpu: (f64, f64, f64),
}

const DARK_PALETTE: ThemePalette = ThemePalette {
    meta_fg: "rgba(230,230,230,0.62)",
    heatpipe: (0.68, 0.82, 0.42),
    cpu: (0.92, 0.54, 0.28),
    gpu: (0.34, 0.60, 0.86),
    idle: (0.90, 0.28, 0.22),
    fin: (0.78, 0.52, 0.88),
    igpu: (0.40, 0.78, 0.72),
};

const LIGHT_PALETTE: ThemePalette = ThemePalette {
    meta_fg: "rgba(36,36,36,0.68)",
    heatpipe: (0.42, 0.62, 0.18),
    cpu: (0.86, 0.43, 0.18),
    gpu: (0.20, 0.45, 0.75),
    idle: (0.80, 0.18, 0.14),
    fin: (0.55, 0.30, 0.68),
    igpu: (0.15, 0.55, 0.50),
};

/// One second of SMC zones and applied limits
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    /// PZ*G is the short average, PZ*F the filtered value the SMC holds at the target
    heatpipe_w: Option<f64>,
    heatpipe_filtered_w: Option<f64>,
    heatpipe_target_w: Option<f64>,
    cpu_w: Option<f64>,
    cpu_filtered_w: Option<f64>,
    cpu_target_w: Option<f64>,
    gpu_w: Option<f64>,
    gpu_filtered_w: Option<f64>,
    gpu_target_w: Option<f64>,
    zone3_filtered_w: Option<f64>,
    zone3_target_w: Option<f64>,
    zone4_filtered_w: Option<f64>,
    zone4_target_w: Option<f64>,
    heatpipe_throttle: Option<f64>,
    cpu_throttle: Option<f64>,
    gpu_throttle: Option<f64>,
    cpu_temp_c: Option<f64>,
    gpu_temp_c: Option<f64>,
    fin_temp_c: Option<f64>,
    plimits: Option<PLimits>,
    applied: Applied,
}

impl Sample {
    fn read(sensors: &SmcSensors) -> Self {
        let plimits = sensors.plimits();
        let fin = [sensors.celsius("Th1H"), sensors.celsius("Th2H")].into_iter().flatten().reduce(f64::max);
        Self {
            heatpipe_w: sensors.micro("PZ0G"),
            heatpipe_filtered_w: sensors.micro("PZ0F"),
            heatpipe_target_w: sensors.micro("PZ0E"),
            cpu_w: sensors.micro("PZ1G"),
            cpu_filtered_w: sensors.micro("PZ1F"),
            cpu_target_w: sensors.micro("PZ1E"),
            gpu_w: sensors.micro("PZ2G"),
            gpu_filtered_w: sensors.micro("PZ2F"),
            gpu_target_w: sensors.micro("PZ2E"),
            zone3_filtered_w: sensors.micro("PZ3F"),
            zone3_target_w: sensors.micro("PZ3E"),
            zone4_filtered_w: sensors.micro("PZ4F"),
            zone4_target_w: sensors.micro("PZ4E"),
            heatpipe_throttle: sensors.micro("PZ0T"),
            cpu_throttle: sensors.micro("PZ1T"),
            gpu_throttle: sensors.micro("PZ2T"),
            cpu_temp_c: sensors.celsius("TCXC"),
            gpu_temp_c: sensors.celsius("TG0P"),
            fin_temp_c: fin,
            plimits,
            applied: Applied::read(sensors.dgpu(), plimits.map(|plimits| plimits.dgpu.step)),
        }
    }

    // The "Applied limits" panel shows the share that is taken away, so an
    // untouched rail reads 0 and a hard cap reads high, like the forced idle
    // share. Step 0 restores the cap to the maximum, which maps to 0 percent.
    fn cpu_limit_percent(&self) -> Option<f64> {
        let applied = self.applied;
        Some(f64::from(applied.cpu_max_mhz?.saturating_sub(applied.cpu_cap_mhz?)) * 100.0 / f64::from(applied.cpu_max_mhz?))
    }

    fn igpu_limit_percent(&self) -> Option<f64> {
        let applied = self.applied;
        Some(f64::from(applied.igpu_max_mhz?.saturating_sub(applied.igpu_cap_mhz?)) * 100.0 / f64::from(applied.igpu_max_mhz?))
    }

    fn dgpu_limit_percent(&self) -> Option<f64> {
        let applied = self.applied;
        Some(f64::from(applied.dgpu_tdp_w?.saturating_sub(applied.dgpu_cap_w?)) * 100.0 / f64::from(applied.dgpu_tdp_w?))
    }
}

struct AppModel {
    sensors: Option<SmcSensors>,
    history: VecDeque<Sample>,
}

impl AppModel {
    fn new() -> Self {
        let mut model = Self {
            sensors: find_t2smc().ok().map(|t2smc| SmcSensors::discover(&t2smc)),
            history: VecDeque::with_capacity(HISTORY_CAPACITY),
        };
        model.tick();
        model
    }

    fn tick(&mut self) {
        let Some(sensors) = &self.sensors else { return };
        if self.history.len() == HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(Sample::read(sensors));
    }

    fn latest(&self) -> Sample {
        self.history.back().copied().unwrap_or_default()
    }

    fn series(&self, value: impl Fn(&Sample) -> Option<f64>) -> Vec<Option<f64>> {
        self.history.iter().map(value).collect()
    }
}

/// A labelled bar meter: the drawn bar and the numeric caption beside it
struct MeterRow {
    bar: DrawingArea,
    value: Label,
}

/// How a bar draws: fill is the length from 0 to 1, severity drives the colour
/// from green when healthy to red when limited
#[derive(Clone, Copy)]
struct Meter {
    fill: f64,
    severity: f64,
}

struct UiRefs {
    heatpipe: MeterRow,
    cpu_zone: MeterRow,
    gpu_zone: MeterRow,
    zone3: MeterRow,
    zone4: MeterRow,
    cpu_step: MeterRow,
    igpu_step: MeterRow,
    dgpu_step: MeterRow,
    idle_step: MeterRow,
    power_graph: DrawingArea,
    throttle_graph: DrawingArea,
    limit_graph: DrawingArea,
    temperature_graph: DrawingArea,
}

pub fn run() -> glib::ExitCode {
    register_embedded_resources();
    let app = Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/org/t2zonepower/gtk")
        .build();
    app.connect_activate(build_ui);
    app.run_with_args::<&str>(&[])
}

fn build_ui(app: &Application) {
    let css_provider = install_css();
    let model = Rc::new(RefCell::new(AppModel::new()));
    let ui = Rc::new(build_widgets());

    let window = ApplicationWindow::builder()
        .application(app)
        .title("T2 Zone Power")
        .default_width(480 + 2 * (style::SIDE_MARGIN - 8))
        .default_height(880)
        .build();
    window.set_resizable(true);

    let root = GtkBox::new(Orientation::Vertical, 8);
    style::pad(&root);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Zone Power", "")));
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    window.set_titlebar(Some(&header));

    // Zone 2 follows the dGPU rail on Macs with a dGPU, on others it is something else
    let (zone2, zone2_key) = if has_dgpu() { ("dGPU", "dGPU zone") } else { ("Zone 2", "Zone 2") };

    root.append(&section_title("Zones"));
    let zones = meter_grid();
    attach_meter_row(&zones, 0, "Heatpipe", &ui.heatpipe);
    attach_meter_row(&zones, 1, "CPU zone", &ui.cpu_zone);
    attach_meter_row(&zones, 2, zone2_key, &ui.gpu_zone);
    attach_meter_row(&zones, 3, "Batt aux", &ui.zone3);
    attach_meter_row(&zones, 4, "Battery", &ui.zone4);
    root.append(&zones);

    root.append(&section_title("Control"));
    let control = meter_grid();
    attach_meter_row(&control, 0, "CPU clock", &ui.cpu_step);
    attach_meter_row(&control, 1, "iGPU clock", &ui.igpu_step);
    attach_meter_row(&control, 2, "dGPU power", &ui.dgpu_step);
    attach_meter_row(&control, 3, "Forced idle", &ui.idle_step);
    root.append(&control);

    let zone_legend = format!("Heatpipe  •  CPU  •  {zone2}");
    root.append(&panel("Zone power", &ui.power_graph, &zone_legend));
    root.append(&panel("Zone throttle", &ui.throttle_graph, &zone_legend));
    root.append(&panel("Applied limits", &ui.limit_graph, "CPU clock  •  iGPU clock  •  dGPU power  •  Forced idle"));
    root.append(&panel("Temperatures", &ui.temperature_graph, "CPU  •  dGPU  •  Fin stack"));

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
    connect_drawings(&model, &ui);
    connect_meters(&model, &ui);
    sync_ui(&model.borrow(), &ui);

    {
        let model = model.clone();
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
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
        label.set_halign(Align::End);
        label.set_xalign(1.0);
        label.set_width_chars(10);
        label.add_css_class("metric-value");
        label
    };
    let make_meter = || MeterRow { bar: make_bar(), value: make_value() };
    let make_graph = || {
        let graph = DrawingArea::new();
        graph.set_content_width(248);
        graph.set_content_height(88);
        graph.set_vexpand(false);
        graph.set_hexpand(true);
        graph
    };

    UiRefs {
        heatpipe: make_meter(),
        cpu_zone: make_meter(),
        gpu_zone: make_meter(),
        zone3: make_meter(),
        zone4: make_meter(),
        cpu_step: make_meter(),
        igpu_step: make_meter(),
        dgpu_step: make_meter(),
        idle_step: make_meter(),
        power_graph: make_graph(),
        throttle_graph: make_graph(),
        limit_graph: make_graph(),
        temperature_graph: make_graph(),
    }
}

fn make_bar() -> DrawingArea {
    let bar = DrawingArea::new();
    bar.set_content_width(120);
    bar.set_content_height(14);
    bar.set_hexpand(true);
    bar.set_valign(Align::Center);
    bar
}

fn connect_drawings(model: &Rc<RefCell<AppModel>>, ui: &Rc<UiRefs>) {
    {
        let model = model.clone();
        ui.power_graph.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            let palette = current_palette();
            let latest = model.latest();
            let plot = draw_history_panel(cr, width as f64, height as f64, "W");
            draw_target(cr, plot, latest.heatpipe_target_w, palette.heatpipe);
            draw_target(cr, plot, latest.cpu_target_w, palette.cpu);
            draw_target(cr, plot, latest.gpu_target_w, palette.gpu);
            draw_history_line(cr, plot, &model.series(|s| s.heatpipe_w), palette.heatpipe);
            draw_history_line(cr, plot, &model.series(|s| s.cpu_w), palette.cpu);
            draw_history_line(cr, plot, &model.series(|s| s.gpu_w), palette.gpu);
            draw_history_series(cr, plot, &model.series(|s| s.heatpipe_filtered_w), palette.heatpipe);
            draw_history_series(cr, plot, &model.series(|s| s.cpu_filtered_w), palette.cpu);
            draw_history_series(cr, plot, &model.series(|s| s.gpu_filtered_w), palette.gpu);
        });
    }
    {
        let model = model.clone();
        ui.throttle_graph.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            let palette = current_palette();
            let plot = draw_history_panel(cr, width as f64, height as f64, "%");
            draw_history_series(cr, plot, &model.series(|s| s.heatpipe_throttle), palette.heatpipe);
            draw_history_series(cr, plot, &model.series(|s| s.cpu_throttle), palette.cpu);
            draw_history_series(cr, plot, &model.series(|s| s.gpu_throttle), palette.gpu);
        });
    }
    {
        let model = model.clone();
        ui.limit_graph.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            let palette = current_palette();
            let plot = draw_history_panel(cr, width as f64, height as f64, "%");
            draw_history_series(cr, plot, &model.series(Sample::cpu_limit_percent), palette.cpu);
            draw_history_series(cr, plot, &model.series(Sample::igpu_limit_percent), palette.igpu);
            draw_history_series(cr, plot, &model.series(Sample::dgpu_limit_percent), palette.gpu);
            draw_history_series(cr, plot, &model.series(|s| s.applied.idle_percent.map(f64::from)), palette.idle);
        });
    }
    {
        let model = model.clone();
        ui.temperature_graph.set_draw_func(move |_area, cr, width, height| {
            let model = model.borrow();
            let palette = current_palette();
            let plot = draw_history_panel(cr, width as f64, height as f64, " C");
            draw_history_series(cr, plot, &model.series(|s| s.cpu_temp_c), palette.cpu);
            draw_history_series(cr, plot, &model.series(|s| s.gpu_temp_c), palette.gpu);
            draw_history_series(cr, plot, &model.series(|s| s.fin_temp_c), palette.fin);
        });
    }
}

fn connect_meters(model: &Rc<RefCell<AppModel>>, ui: &Rc<UiRefs>) {
    bind_meter(&ui.heatpipe.bar, model, |s| zone_meter(s.heatpipe_filtered_w, s.heatpipe_target_w));
    bind_meter(&ui.cpu_zone.bar, model, |s| zone_meter(s.cpu_filtered_w, s.cpu_target_w));
    bind_meter(&ui.gpu_zone.bar, model, |s| zone_meter(s.gpu_filtered_w, s.gpu_target_w));
    bind_meter(&ui.zone3.bar, model, |s| zone_meter(s.zone3_filtered_w, s.zone3_target_w));
    bind_meter(&ui.zone4.bar, model, |s| zone_meter(s.zone4_filtered_w, s.zone4_target_w));
    bind_meter(&ui.cpu_step.bar, model, |s| headroom_meter(s.applied.cpu_cap_mhz, s.applied.cpu_max_mhz));
    bind_meter(&ui.igpu_step.bar, model, |s| headroom_meter(s.applied.igpu_cap_mhz, s.applied.igpu_max_mhz));
    bind_meter(&ui.dgpu_step.bar, model, |s| headroom_meter(s.applied.dgpu_cap_w, s.applied.dgpu_tdp_w));
    bind_meter(&ui.idle_step.bar, model, |s| idle_meter(s.applied.idle_percent));
}

fn bind_meter(bar: &DrawingArea, model: &Rc<RefCell<AppModel>>, meter: impl Fn(&Sample) -> Option<Meter> + 'static) {
    let model = model.clone();
    bar.set_draw_func(move |_area, cr, width, height| {
        let value = meter(&model.borrow().latest());
        draw_meter(cr, width as f64, height as f64, value);
    });
}

/// A zone fills toward its budget, so the bar grows long and red at the limit
fn zone_meter(actual: Option<f64>, budget: Option<f64>) -> Option<Meter> {
    let (actual, budget) = (actual?, budget?);
    if budget <= 0.0 {
        return None;
    }
    let ratio = actual / budget;
    Some(Meter { fill: ratio, severity: ratio })
}

/// A controlled rail is full and green at step 0 and shrinks toward red as the
/// applied cap drops below the maximum
fn headroom_meter(cap: Option<u32>, max: Option<u32>) -> Option<Meter> {
    let (cap, max) = (cap?, max?);
    if max == 0 {
        return None;
    }
    let headroom = f64::from(cap) / f64::from(max);
    Some(Meter { fill: headroom, severity: 1.0 - headroom })
}

/// Forced idle has no maximum, the run share is what is left after the idle cut
fn idle_meter(idle_percent: Option<u32>) -> Option<Meter> {
    let idle = f64::from(idle_percent?) / 100.0;
    Some(Meter { fill: 1.0 - idle, severity: idle })
}

fn sync_ui(model: &AppModel, ui: &UiRefs) {
    let sample = model.latest();
    let applied = sample.applied;
    set_zone_value(&ui.heatpipe, sample.heatpipe_filtered_w, sample.heatpipe_target_w);
    set_zone_value(&ui.cpu_zone, sample.cpu_filtered_w, sample.cpu_target_w);
    set_zone_value(&ui.gpu_zone, sample.gpu_filtered_w, sample.gpu_target_w);
    set_zone_value(&ui.zone3, sample.zone3_filtered_w, sample.zone3_target_w);
    set_zone_value(&ui.zone4, sample.zone4_filtered_w, sample.zone4_target_w);

    let plimits = sample.plimits.unwrap_or_default();
    set_step_value(&ui.cpu_step, applied.cpu_cap_mhz, plimits.cpu.hard, " MHz");
    set_step_value(&ui.igpu_step, applied.igpu_cap_mhz, plimits.igpu.hard, " MHz");
    set_step_value(&ui.dgpu_step, applied.dgpu_cap_w, plimits.dgpu.hard, " W");
    set_step_value(&ui.idle_step, applied.idle_percent, plimits.idle.hard, " %");

    for row in [
        &ui.heatpipe, &ui.cpu_zone, &ui.gpu_zone, &ui.zone3, &ui.zone4,
        &ui.cpu_step, &ui.igpu_step, &ui.dgpu_step, &ui.idle_step,
    ] {
        row.bar.queue_draw();
    }
    ui.power_graph.queue_draw();
    ui.throttle_graph.queue_draw();
    ui.limit_graph.queue_draw();
    ui.temperature_graph.queue_draw();
}

fn set_zone_value(row: &MeterRow, actual: Option<f64>, budget: Option<f64>) {
    row.value.set_label(&match (actual, budget) {
        (Some(actual), Some(budget)) => format!("{actual:.0} / {budget:.0} W"),
        (Some(actual), None) => format!("{actual:.0} W"),
        _ => String::from("—"),
    });
    // The zone sits at its limit when the filtered power is above the budget
    let over = matches!((actual, budget), (Some(actual), Some(budget)) if actual > budget);
    if over {
        row.value.add_css_class("over-budget");
    } else {
        row.value.remove_css_class("over-budget");
    }
}

fn set_step_value(row: &MeterRow, applied: Option<u32>, hard: bool, unit: &str) {
    let hard = if hard { "!" } else { "" };
    row.value.set_label(&match applied {
        Some(applied) => format!("{applied}{unit}{hard}"),
        None => String::from("—"),
    });
}

fn has_dgpu() -> bool {
    glob_paths("/sys/class/drm/card*/device/hwmon/hwmon*/name")
        .unwrap_or_default()
        .iter()
        .any(|name| read_string(name).is_ok_and(|name| name == "amdgpu"))
}

fn panel(title: &str, widget: &impl IsA<gtk4::Widget>, legend: &str) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Vertical, 6);
    let label = Label::new(Some(title));
    label.set_halign(Align::Start);
    label.add_css_class("panel-title");
    box_.append(&label);
    box_.append(&style::card(widget));
    let legend_label = Label::new(None);
    legend_label.set_halign(Align::Start);
    legend_label.set_xalign(0.0);
    legend_label.add_css_class("meta-text");
    legend_label.set_markup(&legend_markup(legend));
    box_.append(&legend_label);
    box_
}

fn meter_grid() -> Grid {
    Grid::builder().column_spacing(12).row_spacing(8).hexpand(true).build()
}

fn section_title(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_halign(Align::Start);
    label.add_css_class("panel-title");
    label
}

fn row_name(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_halign(Align::Start);
    label.set_xalign(0.0);
    label.set_width_chars(11);
    label.add_css_class("metric-key");
    label
}

/// Name on the left, the hexpanding bar in the middle, the caption on the right
fn attach_meter_row(grid: &Grid, row: i32, name: &str, cells: &MeterRow) {
    grid.attach(&row_name(name), 0, row, 1, 1);
    grid.attach(&cells.bar, 1, row, 1, 1);
    grid.attach(&cells.value, 2, row, 1, 1);
}

/// A rounded bar: a muted trough with a filled portion coloured by severity
fn draw_meter(cr: &cairo::Context, width: f64, height: f64, meter: Option<Meter>) {
    let trough = style::graph_colors().grid;
    let radius = height / 2.0;
    rounded_rect(cr, 0.5, 0.5, width - 1.0, height - 1.0, radius);
    cr.set_source_rgb(trough.0, trough.1, trough.2);
    let _ = cr.fill();

    let Some(meter) = meter else { return };
    let fill = meter.fill.clamp(0.0, 1.0) * (width - 1.0);
    if fill <= 0.0 {
        return;
    }
    rounded_rect(cr, 0.5, 0.5, fill.max(height - 1.0), height - 1.0, radius);
    set_color(cr, severity_color(meter.severity));
    let _ = cr.fill();
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Green while there is headroom, amber midway, red once limited
fn severity_color(severity: f64) -> (f64, f64, f64) {
    let good = (0.36, 0.72, 0.40);
    let warn = (0.92, 0.66, 0.20);
    let bad = (0.88, 0.28, 0.24);
    let severity = severity.clamp(0.0, 1.0);
    if severity < 0.5 {
        lerp_color(good, warn, severity / 0.5)
    } else {
        lerp_color(warn, bad, (severity - 0.5) / 0.5)
    }
}

fn lerp_color(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// Draws the grid and returns the plot area. Every graph
/// runs from 0 to 100 in its unit.
fn draw_history_panel(cr: &cairo::Context, width: f64, height: f64, unit: &str) -> (f64, f64, f64, f64) {
    let plot = plot_rect(width, height);
    draw_grid(cr, plot);
    draw_label(cr, plot.0, plot.1 + 8.0, &format!("100{unit}"));
    draw_label(cr, plot.0, plot.1 + plot.3 - 2.0, &format!("0{unit}"));
    plot
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

/// Dashed line for a zone target
fn draw_target(cr: &cairo::Context, plot: (f64, f64, f64, f64), target: Option<f64>, color: (f64, f64, f64)) {
    let Some(target) = target else { return };
    let y = remap(target.clamp(0.0, 100.0), 0.0, 100.0, plot.1 + plot.3, plot.1);
    cr.set_source_rgba(color.0, color.1, color.2, 0.7);
    cr.set_line_width(1.0);
    cr.set_dash(&[4.0, 3.0], 0.0);
    cr.move_to(plot.0, y);
    cr.line_to(plot.0 + plot.2, y);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

fn draw_history_series(cr: &cairo::Context, plot: (f64, f64, f64, f64), values: &[Option<f64>], color: (f64, f64, f64)) {
    let denominator = (HISTORY_CAPACITY.saturating_sub(1)).max(1) as f64;
    let start_index = HISTORY_CAPACITY.saturating_sub(values.len());
    let point = |index: usize, value: f64| {
        (
            plot.0 + plot.2 * ((start_index + index) as f64 / denominator),
            remap(value.clamp(0.0, 100.0), 0.0, 100.0, plot.1 + plot.3, plot.1),
        )
    };

    cr.set_source_rgba(color.0, color.1, color.2, 0.18);
    let mut filled = false;
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else { continue };
        let (x, y) = point(index, *value);
        if !filled {
            cr.move_to(x, plot.1 + plot.3);
            filled = true;
        }
        cr.line_to(x, y);
    }
    if filled {
        cr.line_to(plot.0 + plot.2, plot.1 + plot.3);
        cr.close_path();
        let _ = cr.fill();
    }

    set_color(cr, color);
    cr.set_line_width(2.0);
    let mut first = true;
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            first = true;
            continue;
        };
        let (x, y) = point(index, *value);
        if first {
            cr.move_to(x, y);
            first = false;
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();
}

/// Thin pale line without fill for the short average behind the filtered value
fn draw_history_line(cr: &cairo::Context, plot: (f64, f64, f64, f64), values: &[Option<f64>], color: (f64, f64, f64)) {
    let denominator = (HISTORY_CAPACITY.saturating_sub(1)).max(1) as f64;
    let start_index = HISTORY_CAPACITY.saturating_sub(values.len());
    cr.set_source_rgba(color.0, color.1, color.2, 0.45);
    cr.set_line_width(1.0);
    let mut first = true;
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            first = true;
            continue;
        };
        let x = plot.0 + plot.2 * ((start_index + index) as f64 / denominator);
        let y = remap(value.clamp(0.0, 100.0), 0.0, 100.0, plot.1 + plot.3, plot.1);
        if first {
            cr.move_to(x, y);
            first = false;
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();
}

fn plot_rect(width: f64, height: f64) -> (f64, f64, f64, f64) {
    (10.0, 10.0, (width - 20.0).max(10.0), (height - 20.0).max(10.0))
}

fn remap(value: f64, src_min: f64, src_max: f64, dst_min: f64, dst_max: f64) -> f64 {
    if (src_max - src_min).abs() < f64::EPSILON {
        return dst_min;
    }
    dst_min + (value - src_min) * (dst_max - dst_min) / (src_max - src_min)
}

fn set_color(cr: &cairo::Context, color: (f64, f64, f64)) {
    cr.set_source_rgb(color.0, color.1, color.2);
}

fn legend_markup(legend: &str) -> String {
    let palette = current_palette();
    legend
        .split("  •  ")
        .map(|item| {
            let dot = match item {
                "Heatpipe" => palette.heatpipe,
                "CPU" | "CPU clock" => palette.cpu,
                "dGPU" | "dGPU power" | "Zone 2" => palette.gpu,
                "Forced idle" => palette.idle,
                "iGPU clock" => palette.igpu,
                "Fin stack" => palette.fin,
                _ => style::graph_colors().label,
            };
            format!("<span foreground=\"{}\">●</span> {}", rgb_to_hex(dot), glib::markup_escape_text(item))
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

fn draw_label(cr: &cairo::Context, x: f64, y: f64, text: &str) {
    let label = style::graph_colors().label;
    cr.select_font_face("JetBrains Mono", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
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
        gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    provider
}

fn register_embedded_resources() {
    gio::resources_register_include!("t2-zonepower.gresource").expect("failed to register embedded GTK resources");
}

fn current_palette() -> ThemePalette {
    if StyleManager::default().is_dark() { DARK_PALETTE } else { LIGHT_PALETTE }
}

fn build_css(palette: ThemePalette) -> String {
    format!(
        ".panel-title {{
  font-weight: 600;
}}
.meta-text,
.metric-key,
.footer-link,
.footer-version {{
  color: {meta};
}}
.metric-value {{
  font-family: \"JetBrains Mono\";
  font-feature-settings: \"tnum\" 1;
}}
.over-budget {{
  color: @warning_color;
}}",
        meta = palette.meta_fg,
    )
}

fn watch_theme_changes(provider: &CssProvider, ui: &Rc<UiRefs>) {
    let provider = provider.clone();
    let ui = ui.clone();
    StyleManager::default().connect_dark_notify(move |_| {
        provider.load_from_string(&build_css(current_palette()));
        ui.power_graph.queue_draw();
        ui.throttle_graph.queue_draw();
        ui.limit_graph.queue_draw();
        ui.temperature_graph.queue_draw();
    });
}
