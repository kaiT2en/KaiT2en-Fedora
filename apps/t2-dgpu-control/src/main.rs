// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

#[allow(unused_imports)]
use adw::prelude::*;
use gtk4 as gtk;

const APP_ID: &str = "org.t2dgpucontrol.gtk";
const APP_VERSION: &str = "0.02";
const HELPER: &str = "/usr/local/libexec/t2-dgpu-control-helper";
const STATUS_HELPER: &str = "/usr/local/libexec/t2-dgpu-control-status";
const EFI_VAR: &str =
    "/sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9";
const DGPU_OFF_SERVICE: &str = "kait2en-dgpu-off.service";
const DGPU_SUSPEND_SERVICE: &str = "kait2en-dgpu-suspend.service";
const POWER_PROFILE_SERVICE: &str = "kait2en-amdgpu-profile.service";
const POWER_PROFILE_RESUME_SERVICE: &str = "kait2en-amdgpu-profile-resume.service";

fn kait2en_brand() -> gtk::DrawingArea {
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let dark = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let light = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
        .unwrap_or_else(|_| dark.clone());
    let brand = gtk::DrawingArea::new();
    brand.set_content_width(80);
    brand.set_content_height(21);
    brand.set_size_request(80, 21);
    brand.set_draw_func(move |_area, context, width, height| {
        let pixbuf = if adw::StyleManager::default().is_dark() { &dark } else { &light };
        let scale = f64::min(width as f64 / pixbuf.width() as f64, height as f64 / pixbuf.height() as f64);
        let _ = context.save();
        context.translate((width as f64 - pixbuf.width() as f64 * scale) / 2.0, (height as f64 - pixbuf.height() as f64 * scale) / 2.0);
        context.scale(scale, scale);
        context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = context.paint();
        let _ = context.restore();
    });
    let weak = brand.downgrade();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(brand) = weak.upgrade() {
            brand.queue_draw();
        }
    });
    brand
}

fn palette_css(dark: bool) -> String {
    let (window_bg, window_fg, box_bg) = if dark {
        ("#151517", "#e8e8e8", "#0d0d0f")
    } else {
        ("#ffffff", "#242424", "#e2e2e8")
    };
    let (rim, line) = if dark {
        ("rgba(255,255,255,0.24)", "rgba(255,255,255,0.06)")
    } else {
        ("rgba(255,255,255,0.70)", "rgba(0,0,0,0.08)")
    };
    format!(
        "window, preferencespage, headerbar {{ background: {window_bg}; color: alpha({window_fg}, 0.72); }}
         button, button label, entry, spinbutton, dropdown {{ color: alpha({window_fg}, 0.72); }}
         .title-1, .title-2, .title-3, .title-4, .title, .heading, windowtitle .title {{ color: {window_fg}; font-weight: 600; }}
         .dim-label, windowtitle .subtitle {{ color: alpha({window_fg}, 0.72); }}
         window.csd {{ border-top: 1px solid {rim}; }}
         window.csd.maximized, window.csd.tiled, window.csd.tiled-top, window.csd.tiled-left,
         window.csd.tiled-right, window.csd.tiled-bottom, window.csd.fullscreen {{ border-top-width: 0; }}
         .boxed-list {{
             background: {box_bg};
             border: none;
             border-radius: 12px;
             box-shadow: inset 0 0 0 1px {line};
         }}
         .boxed-list row {{ background: {box_bg}; color: alpha({window_fg}, 0.72); }}
         .donate-link, .donate-link > label {{ color: alpha({window_fg}, 0.72); }}"
    )
}

fn install_palette() {
    let provider = gtk::CssProvider::new();
    let style = adw::StyleManager::default();
    provider.load_from_string(&palette_css(style.is_dark()));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    style.connect_dark_notify(move |manager| {
        provider.load_from_string(&palette_css(manager.is_dark()));
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gpu {
    Integrated,
    Discrete,
    Unknown,
}

impl Gpu {
    fn label(self) -> &'static str {
        match self {
            Self::Integrated => "Integrated GPU",
            Self::Discrete => "Discrete GPU",
            Self::Unknown => "Unknown",
        }
    }
}

fn switcheroo_status() -> (Gpu, String) {
    let Ok(output) = Command::new("pkexec").arg(STATUS_HELPER).output() else {
        return (Gpu::Unknown, "Unavailable".into());
    };
    if !output.status.success() {
        return (Gpu::Unknown, "Unavailable".into());
    }

    let mut active = Gpu::Unknown;
    let mut discrete_state = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields: Vec<_> = line.split(':').collect();
        if fields.len() < 4 {
            continue;
        }
        let gpu = match fields[1] {
            "IGD" => Gpu::Integrated,
            "DIS" => Gpu::Discrete,
            _ => continue,
        };
        if fields[2] == "+" {
            active = gpu;
        }
        if gpu == Gpu::Discrete {
            discrete_state = Some(fields[3].to_owned());
        }
    }

    (
        active,
        discrete_state.unwrap_or_else(|| "Unavailable".into()),
    )
}

fn configured_gpu() -> Gpu {
    let Ok(value) = fs::read(EFI_VAR) else {
        return Gpu::Unknown;
    };

    match value.get(4..8) {
        Some([1, 0, 0, 0]) => Gpu::Integrated,
        Some([0, 0, 0, 0]) => Gpu::Discrete,
        _ => Gpu::Unknown,
    }
}

fn service_enabled(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-enabled", "--quiet", unit])
        .status()
        .is_ok_and(|status| status.success())
}

fn power_saving_enabled() -> bool {
    [POWER_PROFILE_SERVICE, POWER_PROFILE_RESUME_SERVICE]
        .iter()
        .all(|unit| service_enabled(unit))
}

fn run_helper(args: &[&str]) -> Result<(), String> {
    let output = Command::new("pkexec")
        .arg(HELPER)
        .args(args)
        .output()
        .map_err(|error| format!("Could not start the privileged helper: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if message.is_empty() {
        Err("The operation was cancelled or failed.".into())
    } else {
        Err(message)
    }
}

fn set_status(label: &gtk::Label, message: &str, error: bool) {
    label.set_label(message);
    label.remove_css_class("error");
    label.remove_css_class("success");
    label.add_css_class(if error { "error" } else { "success" });
    label.set_visible(true);
}

fn build_ui(app: &adw::Application) {
    install_palette();
    let (current, discrete_state) = switcheroo_status();
    let configured = configured_gpu();
    let configured_power_off = service_enabled(DGPU_OFF_SERVICE);
    let configured_suspend_restore = service_enabled(DGPU_SUSPEND_SERVICE);
    let configured_power_saving = power_saving_enabled();
    let initial_gpu = if configured == Gpu::Unknown {
        current
    } else {
        configured
    };

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 GPU Control")
        .default_width(520)
        .default_height(760)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    content.append(&header);

    let page = adw::PreferencesPage::new();
    let status_group = adw::PreferencesGroup::builder().title("GPU status").build();
    let current_row = adw::ActionRow::builder()
        .title("Current active GPU")
        .subtitle(current.label())
        .build();
    let next_row = adw::ActionRow::builder()
        .title("Configured boot GPU")
        .subtitle(configured.label())
        .build();
    let power_row = adw::ActionRow::builder()
        .title("Discrete GPU at next boot")
        .subtitle(if configured_power_off {
            "Will be powered off"
        } else {
            "Remains powered on"
        })
        .build();
    let discrete_state_row = adw::ActionRow::builder()
        .title("Discrete GPU state")
        .subtitle(discrete_state)
        .build();
    status_group.add(&current_row);
    status_group.add(&next_row);
    status_group.add(&power_row);
    status_group.add(&discrete_state_row);
    page.add(&status_group);

    let config_group = adw::PreferencesGroup::builder()
        .title("Next boot")
        .description("Choose the primary GPU and whether the discrete GPU remains available.")
        .build();

    let boot_row = adw::ActionRow::builder().title("Primary GPU").build();
    let choices = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    choices.add_css_class("linked");
    choices.set_valign(gtk::Align::Center);
    let integrated = gtk::ToggleButton::with_label("Integrated");
    let discrete = gtk::ToggleButton::with_label("Discrete");
    discrete.set_group(Some(&integrated));
    choices.append(&integrated);
    choices.append(&discrete);
    boot_row.add_suffix(&choices);
    config_group.add(&boot_row);

    let power_off_row = adw::ActionRow::builder()
        .title("Power off discrete GPU")
        .subtitle("Reduces idle power use; the dGPU remains unavailable until the next reboot.")
        .build();
    let power_off = gtk::Switch::new();
    power_off.set_valign(gtk::Align::Center);
    power_off_row.add_suffix(&power_off);
    power_off_row.set_activatable_widget(Some(&power_off));
    config_group.add(&power_off_row);

    let power_saving_row = adw::ActionRow::builder()
        .title("AMDGPU power-saving profile")
        .subtitle("Applies the driver power-saving profile after boot and resume.")
        .build();
    let power_saving = gtk::Switch::new();
    power_saving.set_valign(gtk::Align::Center);
    power_saving_row.add_suffix(&power_saving);
    power_saving_row.set_activatable_widget(Some(&power_saving));
    config_group.add(&power_saving_row);
    page.add(&config_group);

    let actions_group = adw::PreferencesGroup::new();
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::End);
    let apply = gtk::Button::with_label("Apply Changes");
    apply.add_css_class("suggested-action");
    let reboot = gtk::Button::with_label("Reboot");
    reboot.set_sensitive(false);
    buttons.append(&apply);
    buttons.append(&reboot);
    let message = gtk::Label::new(None);
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.set_visible(false);
    actions.append(&message);
    actions.append(&buttons);
    actions_group.add(&actions);
    page.add(&actions_group);

    let selected_gpu = Rc::new(Cell::new(initial_gpu));
    let saved_gpu = Rc::new(Cell::new(configured));
    let saved_power_off = Rc::new(Cell::new(configured_power_off));
    let saved_suspend_restore = Rc::new(Cell::new(configured_suspend_restore));
    let saved_power_saving = Rc::new(Cell::new(configured_power_saving));

    match initial_gpu {
        Gpu::Discrete => discrete.set_active(true),
        _ => integrated.set_active(true),
    }
    power_off.set_active(configured_power_off && initial_gpu == Gpu::Integrated);
    power_off.set_sensitive(initial_gpu == Gpu::Integrated);
    power_saving.set_active(configured_power_saving && !power_off.is_active());
    power_saving.set_sensitive(!power_off.is_active());

    let update_apply = {
        let apply = apply.clone();
        let message = message.clone();
        let power_off = power_off.clone();
        let selected_gpu = selected_gpu.clone();
        let saved_gpu = saved_gpu.clone();
        let saved_power_off = saved_power_off.clone();
        let saved_suspend_restore = saved_suspend_restore.clone();
        let power_saving = power_saving.clone();
        let saved_power_saving = saved_power_saving.clone();
        let reboot = reboot.clone();
        move || {
            let gpu = selected_gpu.get();
            let changed = gpu != saved_gpu.get()
                || power_off.is_active() != saved_power_off.get()
                || power_off.is_active() != saved_suspend_restore.get()
                || power_saving.is_active() != saved_power_saving.get();
            apply.set_sensitive(changed);
            if changed {
                message.set_visible(false);
                reboot.set_sensitive(false);
            }
        }
    };

    {
        let selected_gpu = selected_gpu.clone();
        let power_off = power_off.clone();
        let update_apply = update_apply.clone();
        integrated.connect_toggled(move |button| {
            if button.is_active() {
                selected_gpu.set(Gpu::Integrated);
                power_off.set_sensitive(true);
                update_apply();
            }
        });
    }
    {
        let selected_gpu = selected_gpu.clone();
        let power_off = power_off.clone();
        let power_saving = power_saving.clone();
        let update_apply = update_apply.clone();
        discrete.connect_toggled(move |button| {
            if button.is_active() {
                selected_gpu.set(Gpu::Discrete);
                power_off.set_active(false);
                power_off.set_sensitive(false);
                power_saving.set_active(false);
                power_saving.set_sensitive(true);
                update_apply();
            }
        });
    }
    {
        let power_saving = power_saving.clone();
        let update_apply = update_apply.clone();
        power_off.connect_active_notify(move |button| {
            if button.is_active() {
                power_saving.set_active(false);
                power_saving.set_sensitive(false);
            } else {
                power_saving.set_sensitive(true);
            }
            update_apply();
        });
    }
    {
        let update_apply = update_apply.clone();
        power_saving.connect_active_notify(move |_| update_apply());
    }
    update_apply();

    {
        let selected_gpu = selected_gpu.clone();
        let saved_gpu = saved_gpu.clone();
        let saved_power_off = saved_power_off.clone();
        let saved_suspend_restore = saved_suspend_restore.clone();
        let saved_power_saving = saved_power_saving.clone();
        let power_off = power_off.clone();
        let power_saving = power_saving.clone();
        let message = message.clone();
        let reboot = reboot.clone();
        let next_row = next_row.clone();
        let power_row = power_row.clone();
        apply.connect_clicked(move |button| {
            let gpu = selected_gpu.get();
            let power = power_off.is_active();
            let profile = power_saving.is_active();
            button.set_sensitive(false);
            message.set_visible(true);
            message.set_label("Applying GPU configuration...");
            message.remove_css_class("error");
            message.remove_css_class("success");

            let result = match gpu {
                Gpu::Integrated => run_helper(&[
                    "apply",
                    "integrated",
                    if power { "power-off" } else { "keep-powered" },
                    if profile {
                        "power-saving"
                    } else {
                        "default-profile"
                    },
                ]),
                Gpu::Discrete => run_helper(&[
                    "apply",
                    "discrete",
                    "keep-powered",
                    if profile {
                        "power-saving"
                    } else {
                        "default-profile"
                    },
                ]),
                Gpu::Unknown => Err("No boot GPU was selected.".into()),
            };

            match result {
                Ok(()) => {
                    saved_gpu.set(gpu);
                    saved_power_off.set(power);
                    saved_suspend_restore.set(power);
                    saved_power_saving.set(profile);
                    next_row.set_subtitle(gpu.label());
                    power_row.set_subtitle(if power {
                        "Will be powered off"
                    } else {
                        "Remains powered on"
                    });
                    set_status(
                        &message,
                        "Changes applied. Reboot to use the new configuration.",
                        false,
                    );
                    reboot.set_sensitive(true);
                }
                Err(error) => {
                    set_status(&message, &error, true);
                    button.set_sensitive(true);
                }
            }
        });
    }

    {
        let message = message.clone();
        reboot.connect_clicked(move |_| {
            message.set_visible(true);
            if let Err(error) = run_helper(&["reboot"]) {
                set_status(&message, &error, true);
            }
        });
    }

    content.append(&page);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_margin_start(18); footer.set_margin_end(18); footer.set_margin_bottom(8);
    let donate = gtk::LinkButton::builder().uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400").label("Fund our bugs").build();
    donate.add_css_class("donate-link"); footer.append(&donate);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); footer.append(&spacer);
    footer.append(&gtk::Label::new(Some(&format!("v{APP_VERSION}"))));
    content.append(&footer);
    window.set_content(Some(&content));
    window.present();
}

fn main() {
    if !Path::new(HELPER).exists() {
        eprintln!("Privileged helper is not installed at {HELPER}");
    }
    if !Path::new(STATUS_HELPER).exists() {
        eprintln!("Status helper is not installed at {STATUS_HELPER}");
    }

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run();
}
