// SPDX-License-Identifier: GPL-3.0-or-later
//! t2-truetone: settings and live state for the T2 TrueTone session daemon.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use adw::prelude::*;
use gtk4::glib;

use t2_truetone::config;
use t2_truetone::model::DEFAULT_STRENGTH;
use t2_truetone::{SERVICE, status};

const APP_ID: &str = "org.t2truetone.gtk";

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl").arg("--user").args(args).status().map(|s| s.success()).unwrap_or(false)
}

fn daemon_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SERVICE])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Strength is stored as the model value 0..STRENGTH_MAX and shown as 0..100 %.
const STRENGTH_MAX: f64 = 0.3;

fn scale_row(title: &str, tooltip: &str, min: f64, max: f64, step: f64, value: f64, unit: &'static str) -> (adw::ActionRow, gtk4::Scale) {
    let row = adw::ActionRow::builder().title(title).tooltip_text(tooltip).build();
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    scale.set_value(value);
    scale.set_digits(0);
    scale.set_draw_value(true);
    scale.set_format_value_func(move |_, v| format!("{v:.0} {unit}"));
    scale.set_hexpand(true);
    scale.set_width_request(220);
    scale.set_valign(gtk4::Align::Center);
    row.add_suffix(&scale);
    (row, scale)
}

fn build(app: &adw::Application) {
    let (loaded, load_error) = config::load_checked();
    let cfg = Rc::new(RefCell::new(loaded));
    let save = {
        let cfg = cfg.clone();
        move || {
            if let Err(e) = config::save(&cfg.borrow()) {
                eprintln!("t2-truetone: could not save {}: {e}", config::path().display());
            }
        }
    };

    let banner = gtk4::Label::builder()
        .label("GNOME Night Light and auto brightness are turned off while T2 TrueTone is active")
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let page = adw::PreferencesPage::new();

    // Live state
    let state = adw::PreferencesGroup::builder().title("Status").build();
    let row_daemon = adw::ActionRow::builder().title("Service").build();
    let btn_daemon = gtk4::Button::builder().valign(gtk4::Align::Center).build();
    row_daemon.add_suffix(&btn_daemon);
    let row_light = adw::ActionRow::builder().title("Ambient light").build();
    let row_white = adw::ActionRow::builder().title("Display white").build();
    let row_bright = adw::ActionRow::builder().title("Brightness").build();
    for r in [&row_daemon, &row_light, &row_white, &row_bright] {
        state.add(r);
    }
    page.add(&state);

    // True Tone
    let tt = adw::PreferencesGroup::builder().title("True Tone").build();
    let sw_tt = adw::SwitchRow::builder()
        .title("True Tone")
        .tooltip_text("Adapts the white of the display to the color of the ambient light")
        .active(cfg.borrow().true_tone)
        .build();
    let default_pct = DEFAULT_STRENGTH / STRENGTH_MAX * 100.0;
    let strength_pct = cfg.borrow().strength / STRENGTH_MAX * 100.0;
    let (row_strength, sc_strength) = scale_row(
        "Strength",
        &format!("How far the display follows the ambient light. Default {default_pct:.0} %"),
        0.0,
        100.0,
        1.0,
        strength_pct,
        "%",
    );
    sc_strength.add_mark(default_pct, gtk4::PositionType::Bottom, Some("Default"));
    let (row_fade, sc_fade) =
        scale_row("Transition", "Time to settle after the ambient light changes", 0.0, 10.0, 1.0, cfg.borrow().fade_s, "s");
    tt.add(&sw_tt);
    tt.add(&row_strength);
    tt.add(&row_fade);
    page.add(&tt);

    // Evening mode
    let ev = adw::PreferencesGroup::builder().title("Evening mode").build();
    let sw_night = adw::SwitchRow::builder()
        .title("Evening mode")
        .tooltip_text("A warmer display white on a daily schedule")
        .active(cfg.borrow().night)
        .build();
    let en_from = adw::EntryRow::builder().title("From (HH:MM)").text(cfg.borrow().night_from.as_str()).build();
    let en_to = adw::EntryRow::builder().title("Until (HH:MM)").text(cfg.borrow().night_to.as_str()).build();
    let (row_k, sc_k) = scale_row("Color temperature", "Display white at full evening mode", 1900.0, 6000.0, 100.0, cfg.borrow().night_kelvin, "K");
    let (row_ramp, sc_ramp) =
        scale_row("Fade", "Time to fade in at the start and out at the end", 0.0, 10.0, 1.0, cfg.borrow().night_ramp_min, "min");
    for w in [sw_night.upcast_ref::<gtk4::Widget>(), en_from.upcast_ref(), en_to.upcast_ref(), row_k.upcast_ref(), row_ramp.upcast_ref()] {
        ev.add(w);
    }
    page.add(&ev);

    // Brightness
    let br = adw::PreferencesGroup::builder().title("Auto brightness").build();
    let sw_auto = adw::SwitchRow::builder()
        .title("Auto brightness")
        .tooltip_text("Follows the ambient light like macOS. Setting the brightness by hand teaches it the new point.")
        .active(cfg.borrow().auto_brightness)
        .build();
    br.add(&sw_auto);
    page.add(&br);

    let update_banner = {
        let banner = banner.clone();
        let cfg = cfg.clone();
        move || {
            let c = cfg.borrow();
            banner.set_visible(c.true_tone || c.night || c.auto_brightness);
        }
    };
    update_banner();

    macro_rules! on_switch {
        ($sw:expr, $field:ident) => {{
            let (cfg, save, ub) = (cfg.clone(), save.clone(), update_banner.clone());
            $sw.connect_active_notify(move |s| {
                cfg.borrow_mut().$field = s.is_active();
                save();
                ub();
            });
        }};
    }
    on_switch!(sw_tt, true_tone);
    on_switch!(sw_night, night);
    on_switch!(sw_auto, auto_brightness);

    macro_rules! on_scale {
        ($sc:expr, $field:ident) => {{
            let (cfg, save) = (cfg.clone(), save.clone());
            $sc.connect_value_changed(move |s| {
                cfg.borrow_mut().$field = s.value();
                save();
            });
        }};
    }
    {
        let (cfg, save) = (cfg.clone(), save.clone());
        // Near the mark the slider stores the default exactly.
        sc_strength.connect_value_changed(move |s| {
            let v = s.value();
            cfg.borrow_mut().strength =
                if (v - default_pct).abs() < 1.0 { DEFAULT_STRENGTH } else { v / 100.0 * STRENGTH_MAX };
            save();
        });
    }
    on_scale!(sc_fade, fade_s);
    on_scale!(sc_k, night_kelvin);
    on_scale!(sc_ramp, night_ramp_min);

    macro_rules! on_time {
        ($en:expr, $field:ident) => {{
            let (cfg, save) = (cfg.clone(), save.clone());
            $en.connect_changed(move |e| {
                let t = e.text().to_string();
                if config::parse_hhmm(&t).is_some() {
                    e.remove_css_class("error");
                    cfg.borrow_mut().$field = t;
                    save();
                } else {
                    e.add_css_class("error");
                }
            });
        }};
    }
    on_time!(en_from, night_from);
    on_time!(en_to, night_to);

    btn_daemon.connect_clicked(|_| {
        if daemon_active() {
            systemctl(&["disable", "--now", SERVICE]);
        } else {
            systemctl(&["enable", "--now", SERVICE]);
        }
    });

    let refresh = {
        let cfg = cfg.clone();
        move || {
            let active = daemon_active();
            row_daemon.set_subtitle(if active { "Running" } else { "Stopped" });
            btn_daemon.set_label(if active { "Stop" } else { "Start" });
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let st = status::read().filter(|s| active && now.saturating_sub(s.unix_time) < 5);
            match st {
                Some(s) if s.sensor => {
                    row_light.set_subtitle(&format!("{:.1} lx, {:.0} K", s.lux, s.ambient_cct));
                    let night = if s.night_factor > 0.0 {
                        format!(", evening mode {:.0}%", s.night_factor * 100.0)
                    } else {
                        String::new()
                    };
                    row_white.set_subtitle(&format!("{:.0} K{}", s.white_cct, night));
                }
                Some(s) => {
                    row_light.set_subtitle(s.error.as_deref().unwrap_or("Waiting for the sensor"));
                    row_white.set_subtitle("D65");
                }
                None => {
                    row_light.set_subtitle("Service not running");
                    row_white.set_subtitle("Not controlled");
                }
            }
            let c = cfg.borrow();
            row_bright.set_subtitle(&match status::read().and_then(|s| s.brightness) {
                Some(b) if c.auto_brightness && active => format!("Automatic, {:.0}%", b * 100.0),
                _ if c.auto_brightness => "Automatic".to_string(),
                _ => "Manual".to_string(),
            });
        }
    };
    refresh();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        refresh();
        glib::ControlFlow::Continue
    });

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    if let Some(e) = load_error {
        let err = gtk4::Label::builder()
            .label(format!("The settings could not be read and the defaults are shown. Saving replaces them. {e}"))
            .wrap(true)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["error"])
            .build();
        content.append(&err);
    }
    content.append(&banner);
    content.append(&page);
    page.set_vexpand(true);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));

    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 TrueTone")
        .default_width(560)
        .default_height(760)
        .content(&view)
        .build();
    win.present();
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
}
