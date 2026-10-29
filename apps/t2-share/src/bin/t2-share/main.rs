// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings window: receiving on or off, the download folder, the trusted
//! devices and sending files. Received transfers are handled by the
//! `t2share daemon` user service and its notifications. Started with files,
//! for example from a file manager, it opens the send window for them.

mod airdrop;
mod bluetooth;
mod send_window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk4 as gtk;

use t2_share::config::Config;

const APP_ID: &str = "org.t2share.gtk";
const SERVICE: &str = "t2-share.service";

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_activate(build_ui);
    app.connect_open(|app, files, _| {
        send_window::open(app, files.iter().filter_map(|f| f.path()).collect());
    });
    app.run()
}

fn build_ui(app: &adw::Application) {
    let config = Rc::new(RefCell::new(Config::load().unwrap_or_default()));
    let page = adw::PreferencesPage::new();

    let receiving = adw::PreferencesGroup::builder().title("Receiving").build();
    let visible = adw::SwitchRow::builder()
        .title("Visible to nearby phones")
        .subtitle("Phones in the same Wi-Fi can share to this Mac")
        .active(service_active())
        .build();
    visible.connect_active_notify(|row| {
        let verb = if row.is_active() { "enable" } else { "disable" };
        if let Err(err) = systemctl(&[verb, "--now", SERVICE]) {
            eprintln!("{err}");
            row.set_active(service_active());
        }
    });
    receiving.add(&visible);

    let folder = adw::ActionRow::builder().title("Save files to").build();
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    folder.add_suffix(&choose);
    folder.set_activatable_widget(Some(&choose));
    show_folder(&folder, &config.borrow());
    receiving.add(&folder);
    {
        let config = config.clone();
        let folder = folder.clone();
        choose.connect_clicked(move |button| {
            let dialog = gtk::FileDialog::builder().title("Save files to").modal(true).build();
            if let Some(dir) = config.borrow().download_dir() {
                dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
            }
            let window = button.root().and_downcast::<gtk::Window>();
            let config = config.clone();
            let folder = folder.clone();
            dialog.select_folder(window.as_ref(), gio::Cancellable::NONE, move |result| {
                let Some(path) = result.ok().and_then(|file| file.path()) else { return };
                let mut config = config.borrow_mut();
                config.download_dir = Some(path);
                if let Err(err) = config.save() {
                    eprintln!("{err:#}");
                }
                show_folder(&folder, &config);
            });
        });
    }
    page.add(&receiving);

    let sending = adw::PreferencesGroup::builder()
        .title("Sending")
        .description("You can also drop files on this window.")
        .build();
    let send_row = adw::ActionRow::builder()
        .title("Send files to a phone")
        .subtitle("The phone needs Quick Share visible to everyone")
        .build();
    let pick = gtk::Button::builder()
        .label("Choose files…")
        .valign(gtk::Align::Center)
        .build();
    send_row.add_suffix(&pick);
    send_row.set_activatable_widget(Some(&pick));
    sending.add(&send_row);
    {
        let app = app.clone();
        pick.connect_clicked(move |button| {
            let dialog = gtk::FileDialog::builder().title("Send files").modal(true).build();
            let window = button.root().and_downcast::<gtk::Window>();
            let app = app.clone();
            dialog.open_multiple(window.as_ref(), gio::Cancellable::NONE, move |result| {
                let Ok(files) = result else { return };
                let paths: Vec<PathBuf> = (0..files.n_items())
                    .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                    .filter_map(|f| f.path())
                    .collect();
                send_window::open(&app, paths);
            });
        });
    }
    page.add(&sending);

    let trusted = adw::PreferencesGroup::builder()
        .title("Trusted devices")
        .description(
            "Transfers from these devices are accepted without asking. A device is \
             recognized by its name only, so give your phone a unique name that is \
             hard to guess.",
        )
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    trusted.add(&list);
    fill_trusted(&list, &config);
    page.add(&trusted);

    // The service adds devices when "Always trust" is pressed, pick them up
    // when the window comes back into focus.
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 Share")
        .default_width(520)
        .default_height(560)
        .build();
    {
        let config = config.clone();
        let list = list.clone();
        let folder = folder.clone();
        let visible = visible.clone();
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            if let Ok(fresh) = Config::load() {
                *config.borrow_mut() = fresh;
            }
            show_folder(&folder, &config.borrow());
            fill_trusted(&list, &config);
            if visible.is_active() != service_active() {
                visible.set_active(service_active());
            }
        });
    }

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("T2 Share"))));
    header.pack_start(&kait2en_brand());
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    window.set_content(Some(&view));

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    {
        let app = app.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(files) = value.get::<gdk::FileList>() else { return false };
            send_window::open(&app, files.files().iter().filter_map(|f| f.path()).collect());
            true
        });
    }
    view.add_controller(drop);
    window.present();
}

/// The shared KaiT2en wordmark, drawn like in the other KaiT2en apps.
fn kait2en_brand() -> gtk::DrawingArea {
    let brand = gtk::DrawingArea::new();
    brand.set_content_width(80);
    brand.set_content_height(21);
    brand.set_size_request(80, 21);
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png");
    if let Ok(dark) = pixbuf {
        let light =
            gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
                .unwrap_or_else(|_| dark.clone());
        brand.set_draw_func(move |_area, context, width, height| {
            let pixbuf = if adw::StyleManager::default().is_dark() { &dark } else { &light };
            let scale = f64::min(
                width as f64 / pixbuf.width() as f64,
                height as f64 / pixbuf.height() as f64,
            );
            let _ = context.save();
            context.translate(
                (width as f64 - pixbuf.width() as f64 * scale) / 2.0,
                (height as f64 - pixbuf.height() as f64 * scale) / 2.0,
            );
            context.scale(scale, scale);
            context.set_source_pixbuf(pixbuf, 0.0, 0.0);
            let _ = context.paint();
            let _ = context.restore();
        });
        let weak = brand.downgrade();
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(brand) = weak.upgrade() {
                brand.queue_draw();
            }
        });
    }
    brand
}

fn show_folder(row: &adw::ActionRow, config: &Config) {
    let path = config
        .download_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "No folder".to_string());
    row.set_subtitle(&path);
}

fn fill_trusted(list: &gtk::ListBox, config: &Rc<RefCell<Config>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let names = config.borrow().trusted_devices.clone();
    if names.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No trusted devices")
            .subtitle("Choose “Always trust” when a phone shares")
            .build();
        list.append(&row);
        return;
    }
    for name in names {
        let row = adw::ActionRow::builder().title(&name).build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Stop trusting this device")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&remove);
        list.append(&row);
        let config = config.clone();
        let list = list.clone();
        remove.connect_clicked(move |_| {
            {
                let mut config = config.borrow_mut();
                config.trusted_devices.retain(|trusted| trusted != &name);
                if let Err(err) = config.save() {
                    eprintln!("{err:#}");
                }
            }
            fill_trusted(&list, &config);
        });
    }
}

fn service_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SERVICE])
        .status()
        .is_ok_and(|status| status.success())
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|err| format!("systemctl: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
