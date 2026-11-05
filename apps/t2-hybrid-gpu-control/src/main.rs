use std::fs;
use std::path::Path;
use std::process::Command;

#[allow(unused_imports)]
use adw::prelude::*;
use gtk4 as gtk;

const APP_ID: &str = "org.t2hybridgpucontrol.gtk";
const APP_VERSION: &str = "0.02";
const HELPER: &str = "/usr/local/libexec/t2-hybrid-gpu-control-helper";
const STATUS_HELPER: &str = "/usr/local/libexec/t2-hybrid-gpu-control-status";
const EFI_VAR: &str =
    "/sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9";

fn kait2en_brand() -> gtk::DrawingArea {
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let dark = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let light = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
        .unwrap_or_else(|_| dark.clone());
    let brand = gtk::DrawingArea::new();
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

struct GpuStatus {
    active: Gpu,
    discrete_state: String,
    runtime_pm: bool,
    support: &'static str,
}

impl GpuStatus {
    fn unavailable(support: &'static str) -> Self {
        Self {
            active: Gpu::Unknown,
            discrete_state: "Unavailable".into(),
            runtime_pm: false,
            support,
        }
    }
}

fn switcheroo_status() -> GpuStatus {
    let Ok(output) = Command::new("pkexec").arg(STATUS_HELPER).output() else {
        return GpuStatus::unavailable("Could not start the status helper");
    };
    if !output.status.success() {
        return GpuStatus::unavailable("Could not read the vga_switcheroo state");
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

    let Some(discrete_state) = discrete_state else {
        return GpuStatus {
            active,
            ..GpuStatus::unavailable("No discrete GPU is registered with vga_switcheroo")
        };
    };
    let runtime_pm = matches!(discrete_state.as_str(), "DynPwr" | "DynOff");
    GpuStatus {
        active,
        discrete_state,
        runtime_pm,
        support: if runtime_pm {
            "Available"
        } else {
            "Runtime power management is not enabled for the discrete GPU"
        },
    }
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

fn update_runtime_status(
    current_row: &adw::ActionRow,
    runtime_row: &adw::ActionRow,
    discrete_state_row: &adw::ActionRow,
    hybrid_button: &gtk::Button,
) {
    let status = switcheroo_status();
    current_row.set_subtitle(status.active.label());
    runtime_row.set_subtitle(status.support);
    discrete_state_row.set_subtitle(&status.discrete_state);
    hybrid_button.set_sensitive(status.runtime_pm);
}

fn build_ui(app: &adw::Application) {
    install_palette();
    let status = switcheroo_status();
    let configured = configured_gpu();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 Hybrid GPU Control")
        .default_width(520)
        .default_height(740)
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
        .title("Current display GPU")
        .subtitle(status.active.label())
        .build();
    let next_row = adw::ActionRow::builder()
        .title("Configured boot GPU")
        .subtitle(configured.label())
        .build();
    let runtime_row = adw::ActionRow::builder()
        .title("Hybrid graphics support")
        .subtitle(status.support)
        .build();
    let discrete_state_row = adw::ActionRow::builder()
        .title("Discrete GPU runtime state")
        .subtitle(&status.discrete_state)
        .build();
    status_group.add(&current_row);
    status_group.add(&next_row);
    status_group.add(&runtime_row);
    status_group.add(&discrete_state_row);
    page.add(&status_group);

    let config_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .description("Hybrid graphics uses the integrated GPU for the display and wakes the AMD GPU automatically for PRIME offload.")
        .build();

    let hybrid_row = adw::ActionRow::builder()
        .title("Hybrid graphics")
        .subtitle("Recommended for performance, battery life, and suspend")
        .build();
    let hybrid_button = gtk::Button::with_label(if configured == Gpu::Integrated {
        "Reapply"
    } else {
        "Enable"
    });
    hybrid_button.add_css_class("suggested-action");
    hybrid_button.set_valign(gtk::Align::Center);
    hybrid_button.set_sensitive(status.runtime_pm);
    hybrid_row.add_suffix(&hybrid_button);
    config_group.add(&hybrid_row);

    let discrete_row = adw::ActionRow::builder()
        .title("Discrete GPU boot")
        .subtitle("Recovery option for configurations that cannot use hybrid graphics")
        .build();
    let discrete_button = gtk::Button::with_label("Use Next Boot");
    discrete_button.set_valign(gtk::Align::Center);
    discrete_row.add_suffix(&discrete_button);
    config_group.add(&discrete_row);
    page.add(&config_group);

    let actions_group = adw::PreferencesGroup::new();
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let message = gtk::Label::new(None);
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.set_visible(false);
    let reboot = gtk::Button::with_label("Reboot");
    reboot.add_css_class("suggested-action");
    reboot.set_halign(gtk::Align::End);
    reboot.set_sensitive(false);
    actions.append(&message);
    actions.append(&reboot);
    actions_group.add(&actions);
    page.add(&actions_group);

    {
        let message = message.clone();
        let next_row = next_row.clone();
        let reboot = reboot.clone();
        hybrid_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            match run_helper(&["enable-hybrid"]) {
                Ok(()) => {
                    next_row.set_subtitle(Gpu::Integrated.label());
                    set_status(
                        &message,
                        "Hybrid graphics configured. Reboot to use the integrated display GPU. After booting macOS, the setting is restored automatically on the next Linux start.",
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
        let next_row = next_row.clone();
        let reboot = reboot.clone();
        discrete_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            match run_helper(&["use-discrete"]) {
                Ok(()) => {
                    next_row.set_subtitle(Gpu::Discrete.label());
                    set_status(
                        &message,
                        "Discrete GPU boot configured. Reboot to apply the recovery configuration.",
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
            if let Err(error) = run_helper(&["reboot"]) {
                set_status(&message, &error, true);
            }
        });
    }

    {
        let current_row = current_row.clone();
        let runtime_row = runtime_row.clone();
        let discrete_state_row = discrete_state_row.clone();
        let hybrid_button = hybrid_button.clone();
        gtk::glib::timeout_add_seconds_local(3, move || {
            update_runtime_status(
                &current_row,
                &runtime_row,
                &discrete_state_row,
                &hybrid_button,
            );
            gtk::glib::ControlFlow::Continue
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
