mod action;
mod config;
mod error;
mod ipc;
mod service;
mod sysfs;

use std::{
    cell::RefCell,
    io,
    os::fd::AsRawFd,
    rc::Rc,
    sync::mpsc,
    thread,
    time::Duration,
};

use action::ActionRunner;
use adw::StyleManager;
use config::{
    ActionKind, AppConfig, CLICK_STRENGTH_FIRM, CLICK_STRENGTH_LIGHT, MAX_FORCE_CLICK_PERCENT,
    MIN_FORCE_CLICK_PERCENT,
};
use evdev::{EventSummary, KeyCode};
use gtk4::{
    gdk, glib,
    prelude::*,
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, DropDown,
    Entry, EventControllerKey, Label, LinkButton, Orientation, Scale, Stack, StringList, Window,
};
use ipc::{DaemonState, Request};

const APP_ID: &str = "org.t2forceclick.gtk";
const APP_VERSION: &str = "0.01";

fn action_kind_from_selection(selected: u32) -> ActionKind {
    match selected {
        1 => ActionKind::CopyPaste,
        2 => ActionKind::KeyCombo,
        3 => ActionKind::Command,
        _ => ActionKind::None,
    }
}

fn kait2en_brand() -> gtk4::DrawingArea {
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let brand = gtk4::DrawingArea::new();
    brand.set_content_width(80);
    brand.set_content_height(21);
    brand.set_size_request(80, 21);
    brand.set_draw_func(move |_area, context, width, height| {
        let scale = f64::min(width as f64 / pixbuf.width() as f64, height as f64 / pixbuf.height() as f64);
        let _ = context.save();
        context.translate(
            (width as f64 - pixbuf.width() as f64 * scale) / 2.0,
            (height as f64 - pixbuf.height() as f64 * scale) / 2.0,
        );
        context.scale(scale, scale);
        context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = context.paint();
        let _ = context.restore();
    });
    brand
}

fn palette_css(dark: bool) -> String {
    let (window_bg, window_fg) = if dark {
        ("#161616", "#e8e8e8")
    } else {
        ("#f2f2f2", "#242424")
    };

    format!(
        "window, popover, .force-root, .force-root headerbar {{
             background: {window_bg};
             color: alpha({window_fg}, 0.72);
             font-family: 'JetBrains Mono';
             font-size: 11pt;
             font-weight: 400;
         }}
         label, button, button label, entry, dropdown, scale {{
             color: alpha({window_fg}, 0.72);
             font-family: 'JetBrains Mono';
             font-size: 11pt;
             font-weight: 400;
         }}
         headerbar {{ background: @headerbar_bg_color; }}
         .footer-link, .footer-link > label, .footer-version {{
             font-family: 'JetBrains Mono'; font-size: 11pt; font-weight: 400;
         }}
         .footer-version {{ opacity: 0.6; }}
         .command-help {{ opacity: 0.72; }}"
    )
}

fn install_palette() {
    let provider = gtk4::CssProvider::new();
    let style = StyleManager::default();
    provider.load_from_data(&palette_css(style.is_dark()));
    if let Some(display) = gdk::Display::default() {
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

fn main() -> glib::ExitCode {
    if std::env::args().any(|arg| arg == "--daemon") {
        if let Err(error) = daemon_main() {
            eprintln!("{error}");
            return glib::ExitCode::from(1);
        }
        return glib::ExitCode::SUCCESS;
    }

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

enum DaemonEvent {
    Request(std::os::unix::net::UnixStream, error::Result<Request>),
    ForceClick,
    TrackpadAvailable,
}

fn daemon_main() -> error::Result<()> {
    /* t2_precision_trackpad reports force clicks as BTN_TASK on a separate
     * input device. */
    let listener = ipc::bind_listener()?;
    let (tx, rx) = mpsc::channel();

    let connection_tx = tx.clone();
    thread::spawn(move || loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let request_tx = connection_tx.clone();
                thread::spawn(move || {
                    // A client may disappear halfway through a request.  It
                    // must not stall physical Force Click handling.
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let request = ipc::read_request(&stream);
                    let _ = request_tx.send(DaemonEvent::Request(stream, request));
                });
            }
            Err(error) => eprintln!("t2-force-click: socket accept failed: {error}"),
        }
    });

    let device_tx = tx.clone();
    thread::spawn(move || loop {
        match sysfs::find_trackpad_device_with_path() {
            Ok((device_path, mut device)) => {
                if device_tx.send(DaemonEvent::TrackpadAvailable).is_err() {
                    return;
                }
                loop {
                    match wait_for_input(&device) {
                        Ok(false) => {
                            let reenumerated = sysfs::find_trackpad_device_with_path()
                                .map(|(path, _)| path != device_path)
                                .unwrap_or(true);
                            if reenumerated {
                                eprintln!("t2-force-click: trackpad event device changed");
                                break;
                            }
                            continue;
                        }
                        Err(error) => {
                            eprintln!("t2-force-click: lost trackpad device: {error}");
                            break;
                        }
                        Ok(true) => {}
                    }
                    match device.fetch_events() {
                        Ok(events) => {
                            for event in events {
                                if let EventSummary::Key(_, KeyCode::BTN_TASK, 1) = event.destructure() {
                                    if device_tx.send(DaemonEvent::ForceClick).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            eprintln!("t2-force-click: lost trackpad device: {error}");
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("t2-force-click: {error}, retrying in 5s");
            }
        }
        thread::sleep(Duration::from_secs(5));
    });

    let mut config = AppConfig::load();
    apply_thresholds(&config);
    let mut action_runner = ActionRunner::new();

    for event in rx {
        match event {
            DaemonEvent::ForceClick => {
                action_runner.run(&config, service::active_session().ok().as_ref())
            }
            DaemonEvent::TrackpadAvailable => apply_thresholds(&config),
            DaemonEvent::Request(stream, request) => {
                let authorized = ipc::peer_uid(&stream)
                    .and_then(|uid| {
                        if uid == 0
                            || service::active_session().is_ok_and(|session| session.uid == uid)
                        {
                            Ok(())
                        } else {
                            Err(error::ForceClickError::Protocol(
                                "request is not from the active local user".to_owned(),
                            ))
                        }
                    });
                if let Err(error) = authorized {
                    let _ = ipc::write_error(&stream, &error.to_string());
                    continue;
                }
                match request {
                    Ok(Request::GetState) => {
                        let _ = ipc::write_response(&stream, &state_from_config(&config));
                    }
                    Ok(Request::SetConfig(new_config)) => {
                        config = new_config;
                        apply_thresholds(&config);
                        if let Err(error) = config.save() {
                            eprintln!("t2-force-click: failed to save config: {error}");
                        }
                        let _ = ipc::write_response(&stream, &state_from_config(&config));
                    }
                    Err(error) => {
                        let _ = ipc::write_error(&stream, &error.to_string());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Wait at most one second for a force-click event. A finite wait lets the
/// listener detect an evdev device that disappeared during a driver reload.
/// `fetch_events()` alone blocks indefinitely on such a stale descriptor.
fn wait_for_input(device: &evdev::Device) -> io::Result<bool> {
    let mut fd = libc::pollfd {
        fd: device.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut fd, 1, 1000) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if result == 0 {
        return Ok(false);
    }
    if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
        return Err(io::Error::from(io::ErrorKind::BrokenPipe));
    }
    Ok(fd.revents & libc::POLLIN != 0)
}

fn apply_thresholds(config: &AppConfig) {
    /* MTForceThresholding::getClickThresholdMultiplier maps click strength.
     * force_click_threshold_percent controls the downstream second threshold.
     * force_click_enabled gates that second press independently, so the
     * plain click (click_strength) keeps working when it is turned off. */
    if let Err(error) = sysfs::write_click_strength(config.click_strength) {
        eprintln!("t2-force-click: {error}");
    }
    if let Err(error) = sysfs::write_force_click_threshold_percent(config.force_click_threshold_percent) {
        eprintln!("t2-force-click: {error}");
    }
    if let Err(error) = sysfs::write_force_click_enabled(!config.force_click_disabled) {
        eprintln!("t2-force-click: {error}");
    }
}

fn state_from_config(config: &AppConfig) -> DaemonState {
    DaemonState {
        driver_loaded: sysfs::module_loaded(),
        device_found: sysfs::find_trackpad_device().is_ok(),
        click_strength: sysfs::read_click_strength().unwrap_or(config.click_strength),
        force_click_threshold_percent: sysfs::read_force_click_threshold_percent()
            .unwrap_or(config.force_click_threshold_percent),
        force_click_disabled: config.force_click_disabled,
        action_kind: config.action_kind.as_str().to_owned(),
        key_combo: config.key_combo.clone(),
        command: config.command.clone(),
    }
}

fn build_ui(app: &Application) {
    install_palette();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Force Click")
        .default_width(420)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 14);
    root.add_css_class("force-root");
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let header = adw::HeaderBar::new();
    if std::path::Path::new("/usr/local/share/kait2en/kait2en-wordmark.png").exists() {
        let brand = kait2en_brand();
        brand.set_margin_start(10);
        brand.set_margin_end(10);
        header.pack_start(&brand);
    }
    /* A title widget suppresses the automatic window-title label. */
    header.set_title_widget(Some(&Label::new(None)));
    window.set_titlebar(Some(&header));

    let click_label = Label::new(Some("Click force"));
    click_label.set_halign(Align::Start);
    root.append(&click_label);
    let click_scale = Scale::with_range(
        Orientation::Horizontal,
        CLICK_STRENGTH_LIGHT as f64,
        CLICK_STRENGTH_FIRM as f64,
        1.0,
    );
    click_scale.set_draw_value(false);
    for (value, text) in [
        (CLICK_STRENGTH_LIGHT as f64, "Light"),
        (1.0, "Medium"),
        (CLICK_STRENGTH_FIRM as f64, "Firm"),
    ] {
        click_scale.add_mark(value, gtk4::PositionType::Bottom, Some(text));
    }
    root.append(&click_scale);

    let force_label = Label::new(Some("Force click force (relative to click)"));
    force_label.set_halign(Align::Start);
    root.append(&force_label);
    let force_scale = Scale::with_range(
        Orientation::Horizontal,
        MIN_FORCE_CLICK_PERCENT as f64,
        MAX_FORCE_CLICK_PERCENT as f64,
        5.0,
    );
    force_scale.set_draw_value(true);
    root.append(&force_scale);

    let action_label = Label::new(Some("On Force Click"));
    action_label.set_halign(Align::Start);
    action_label.set_margin_top(6);
    root.append(&action_label);
    let action_list = StringList::new(&[
        "None",
        "Copy / Paste",
        "Custom key combo",
        "Run command",
    ]);
    let action_dropdown = DropDown::new(Some(action_list), gtk4::Expression::NONE);
    root.append(&action_dropdown);

    let action_details = Stack::new();
    action_details.set_vhomogeneous(true);

    let no_action_details = GtkBox::new(Orientation::Vertical, 0);
    action_details.add_named(&no_action_details, Some("none"));

    let copy_paste_details = GtkBox::new(Orientation::Vertical, 0);
    action_details.add_named(&copy_paste_details, Some("copy-paste"));

    let key_combo_details = GtkBox::new(Orientation::Vertical, 8);
    let key_combo = Rc::new(RefCell::new(String::new()));
    let record_shortcut_button = Button::with_label("Record shortcut…");
    key_combo_details.append(&record_shortcut_button);
    action_details.add_named(&key_combo_details, Some("key-combo"));

    let command_details = GtkBox::new(Orientation::Vertical, 8);
    let command_entry = Entry::new();
    command_entry.set_placeholder_text(Some("e.g. notify-send 'Force Click'"));
    let command_help = Label::new(Some(
        "Advanced: runs a shell command as you. Examples: notify-send 'Force Click' · nautilus ~/Downloads · xdg-open https://…",
    ));
    command_help.set_halign(Align::Start);
    command_help.set_wrap(true);
    command_help.add_css_class("command-help");
    command_details.append(&command_entry);
    command_details.append(&command_help);
    action_details.add_named(&command_details, Some("command"));
    root.append(&action_details);

    let update_field_visibility = {
        let action_details = action_details.clone();
        move |selected: u32| {
            action_details.set_visible_child_name(match selected {
                1 => "copy-paste",
                2 => "key-combo",
                3 => "command",
                _ => "none",
            });
        }
    };
    update_field_visibility(0);
    action_dropdown.connect_selected_notify(glib::clone!(
        #[strong]
        update_field_visibility,
        move |dropdown| update_field_visibility(dropdown.selected())
    ));
    record_shortcut_button.connect_clicked(glib::clone!(
        #[strong]
        window,
        #[strong]
        key_combo,
        #[strong]
        record_shortcut_button,
        move |_| record_shortcut(&window, &key_combo, &record_shortcut_button)
    ));

    let disable_force_click_check =
        CheckButton::with_label("Disable Force Click");
    disable_force_click_check.set_margin_top(6);
    root.append(&disable_force_click_check);
    disable_force_click_check.connect_toggled(glib::clone!(
        #[strong]
        force_scale,
        move |check| force_scale.set_sensitive(!check.is_active())
    ));

    let button_row = GtkBox::new(Orientation::Horizontal, 8);
    let apply_button = Button::with_label("Apply");
    apply_button.add_css_class("suggested-action");
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    button_row.append(&spacer);
    button_row.append(&apply_button);
    root.append(&button_row);

    let footer = GtkBox::new(Orientation::Horizontal, 8);
    footer.set_margin_top(10);
    let donate = LinkButton::builder()
        .uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400")
        .label("Fund our bugs")
        .build();
    donate.set_halign(Align::Start);
    donate.add_css_class("footer-link");
    footer.append(&donate);
    let footer_spacer = GtkBox::new(Orientation::Horizontal, 0);
    footer_spacer.set_hexpand(true);
    footer.append(&footer_spacer);
    let version = Label::new(Some(&format!("v{APP_VERSION}")));
    version.set_halign(Align::End);
    version.add_css_class("footer-version");
    footer.append(&version);
    root.append(&footer);

    let refresh = {
        let click_scale = click_scale.clone();
        let force_scale = force_scale.clone();
        let action_dropdown = action_dropdown.clone();
        let key_combo = key_combo.clone();
        let record_shortcut_button = record_shortcut_button.clone();
        let command_entry = command_entry.clone();
        let disable_force_click_check = disable_force_click_check.clone();
        let action_details = action_details.clone();
        let update_field_visibility = update_field_visibility.clone();
        let apply_button = apply_button.clone();
        move || match ipc::send_request(Request::GetState) {
            Ok(state) => {
                click_scale.set_value(state.click_strength as f64);
                force_scale.set_value(state.force_click_threshold_percent as f64);
                let selected = match state.action_kind.as_str() {
                    "copy_paste" => 1,
                    "key_combo" => 2,
                    "command" => 3,
                    _ => 0,
                };
                action_dropdown.set_selected(selected);
                update_field_visibility(selected);
                *key_combo.borrow_mut() = state.key_combo;
                record_shortcut_button.set_label(
                    if key_combo.borrow().is_empty() {
                        "Record shortcut…"
                    } else {
                        "Change shortcut…"
                    },
                );
                command_entry.set_text(&state.command);
                disable_force_click_check.set_active(state.force_click_disabled);

                /* driver_loaded gates writes to kernel module parameters. */
                click_scale.set_sensitive(state.driver_loaded);
                force_scale.set_sensitive(state.driver_loaded && !state.force_click_disabled);
                action_dropdown.set_sensitive(state.driver_loaded);
                action_details.set_sensitive(state.driver_loaded);
                disable_force_click_check.set_sensitive(state.driver_loaded);
                apply_button.set_sensitive(state.driver_loaded);
            }
            Err(_error) => {
                click_scale.set_sensitive(false);
                force_scale.set_sensitive(false);
                action_dropdown.set_sensitive(false);
                action_details.set_sensitive(false);
                disable_force_click_check.set_sensitive(false);
                apply_button.set_sensitive(false);
            }
        }
    };
    refresh();

    apply_button.connect_clicked(glib::clone!(
        #[strong]
        click_scale,
        #[strong]
        force_scale,
        #[strong]
        action_dropdown,
        #[strong]
        key_combo,
        #[strong]
        command_entry,
        #[strong]
        disable_force_click_check,
        #[strong]
        apply_button,
        move |_| {
            let action_kind = action_kind_from_selection(action_dropdown.selected());
            let config = AppConfig {
                click_strength: click_scale.value().round() as u8,
                force_click_threshold_percent: force_scale.value().round() as u32,
                force_click_disabled: disable_force_click_check.is_active(),
                action_kind,
                key_combo: if action_kind == ActionKind::KeyCombo {
                    key_combo.borrow().clone()
                } else {
                    String::new()
                },
                command: if action_kind == ActionKind::Command {
                    command_entry.text().to_string()
                } else {
                    String::new()
                },
            };
            match ipc::send_request(Request::SetConfig(config)) {
                Ok(_) => {
                    apply_button.set_label("Applied");
                    let apply_button = apply_button.clone();
                    glib::timeout_add_seconds_local_once(2, move || {
                        apply_button.set_label("Apply");
                    });
                }
                Err(error) => {
                    apply_button.set_label("Not applied");
                    apply_button.set_tooltip_text(Some(&error.to_string()));
                    let apply_button = apply_button.clone();
                    glib::timeout_add_seconds_local_once(3, move || {
                        apply_button.set_label("Apply");
                    });
                }
            }
        }
    ));

    window.set_child(Some(&root));
    window.present();
}

fn record_shortcut(parent: &ApplicationWindow, target: &Rc<RefCell<String>>, button: &Button) {
    let dialog = Window::builder()
        .title("Record shortcut")
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .default_width(300)
        .default_height(100)
        .build();
    let message = Label::new(Some("Press the desired shortcut now…\nWaiting for 5 seconds."));
    message.set_margin_top(24);
    message.set_margin_bottom(24);
    message.set_margin_start(24);
    message.set_margin_end(24);
    dialog.set_child(Some(&message));

    let controller = EventControllerKey::new();
    controller.connect_key_pressed(glib::clone!(
        #[strong]
        dialog,
        #[strong]
        target,
        #[strong]
        button,
        move |_controller, key, _keycode, modifiers| {
            if let Some(combo) = captured_shortcut(key, modifiers) {
                *target.borrow_mut() = combo;
                button.set_label("Change shortcut…");
                dialog.close();
            }
            glib::Propagation::Stop
        }
    ));
    dialog.add_controller(controller);

    let timeout_dialog = dialog.clone();
    glib::timeout_add_seconds_local_once(5, move || timeout_dialog.close());
    dialog.present();
}

fn captured_shortcut(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<String> {
    let name = key.name()?.to_ascii_lowercase();
    if matches!(name.as_str(), "control_l" | "control_r" | "alt_l" | "alt_r" | "shift_l" | "shift_r" | "super_l" | "super_r") {
        return None;
    }

    let mut parts = Vec::new();
    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("leftctrl");
    }
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        parts.push("leftalt");
    }
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("leftshift");
    }
    if modifiers.contains(gdk::ModifierType::SUPER_MASK) {
        parts.push("leftmeta");
    }
    let key = match name.as_str() {
        "return" => "enter",
        "escape" => "esc",
        "backspace" => "backspace",
        "page_up" => "pageup",
        "page_down" => "pagedown",
        other => other,
    };
    parts.push(key);
    Some(parts.join("+"))
}
