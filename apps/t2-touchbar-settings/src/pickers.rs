// SPDX-License-Identifier: GPL-3.0-or-later

//! Finding the window class of an app: from the window the user switches
//! to, or from the installed apps.

use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk4::{gio, glib};

use crate::{focus, window::Shared};

const COUNTDOWN_SECONDS: u32 = 3;

fn desktop_app(class: &str) -> Option<gio::DesktopAppInfo> {
    if class.is_empty() {
        return None;
    }
    gio::DesktopAppInfo::new(&format!("{class}.desktop"))
        .or_else(|| gio::DesktopAppInfo::new(&format!("{}.desktop", class.to_lowercase())))
        .or_else(|| {
            gio::AppInfo::all().into_iter().find_map(|info| {
                let info = info.downcast::<gio::DesktopAppInfo>().ok()?;
                info.startup_wm_class()
                    .is_some_and(|wm| wm.eq_ignore_ascii_case(class))
                    .then_some(info)
            })
        })
}

pub fn icon_for_class(class: &str) -> Option<gio::Icon> {
    desktop_app(class)?.icon()
}

fn name_for_class(class: &str) -> String {
    desktop_app(class)
        .map(|info| info.display_name().to_string())
        .unwrap_or_else(|| class.to_owned())
}

/// Counts down while the user switches to the app, then hands over the
/// class of the focused window.
pub fn countdown(ui: &Shared, found: impl Fn(String) + 'static) {
    let status = adw::StatusPage::builder()
        .icon_name("find-location-symbolic")
        .title(COUNTDOWN_SECONDS.to_string())
        .description("Switch to the app now, with Cmd+Tab or a click.")
        .build();
    let cancel = gtk4::Button::with_label("Cancel");
    cancel.set_halign(gtk4::Align::Center);
    cancel.add_css_class("pill");
    status.set_child(Some(&cancel));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&status));
    let dialog = adw::Dialog::builder()
        .title("Detect App")
        .content_width(360)
        .child(&toolbar)
        .build();

    let cancelled = Rc::new(Cell::new(false));
    let (cancelled_, dialog_) = (cancelled.clone(), dialog.clone());
    cancel.connect_clicked(move |_| {
        cancelled_.set(true);
        dialog_.close();
    });
    let cancelled_ = cancelled.clone();
    dialog.connect_closed(move |_| cancelled_.set(true));

    dialog.present(Some(&ui.window));
    let ui = ui.clone();
    let left = Cell::new(COUNTDOWN_SECONDS);
    let found = Rc::new(found);
    glib::timeout_add_seconds_local(1, move || {
        if cancelled.get() {
            return glib::ControlFlow::Break;
        }
        left.set(left.get() - 1);
        if left.get() > 0 {
            status.set_title(&left.get().to_string());
            return glib::ControlFlow::Continue;
        }
        let (ui, dialog, found) = (ui.clone(), dialog.clone(), found.clone());
        glib::spawn_future_local(async move {
            let result = focus::focused().await;
            dialog.close();
            ui.window.present();
            match result {
                Ok(window) => match window.class() {
                    Some(class) if class == crate::APP_ID => {
                        ui.toast("Touch Bar Settings still had the focus. Try again")
                    }
                    Some(class) => found(class.to_owned()),
                    None => ui.toast("That window has no class"),
                },
                Err(error) => ui.toast(&error),
            }
        });
        glib::ControlFlow::Break
    });
}

/// Adds or opens the profile of the app the user switches to.
pub fn detect(ui: &Shared) {
    let ui_ = ui.clone();
    countdown(ui, move |class| ui_.open_or_add(name_for_class(&class), class));
}

/// On Wayland the window class is the app ID, which is the name of the
/// desktop file. X11 apps name theirs in StartupWMClass.
fn class_of(info: &gio::DesktopAppInfo) -> Option<String> {
    let id = info.id()?;
    let id = id.strip_suffix(".desktop").unwrap_or(&id).to_owned();
    Some(id)
}

pub fn choose_installed(ui: &Shared) {
    let mut apps: Vec<gio::DesktopAppInfo> = gio::AppInfo::all()
        .into_iter()
        .filter(|info| info.should_show())
        .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok())
        .collect();
    apps.sort_by_key(|info| info.display_name().to_lowercase());

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let search = gtk4::SearchEntry::builder()
        .placeholder_text("Search apps")
        .margin_bottom(12)
        .build();

    let dialog = adw::Dialog::builder()
        .title("Choose App")
        .content_width(420)
        .content_height(560)
        .build();

    let mut rows = Vec::new();
    for info in apps {
        let Some(class) = class_of(&info) else {
            continue;
        };
        let name = info.display_name().to_string();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&name))
            .subtitle(glib::markup_escape_text(&class))
            .activatable(true)
            .build();
        let image = info
            .icon()
            .map(|icon| gtk4::Image::from_gicon(&icon))
            .unwrap_or_else(|| gtk4::Image::from_icon_name("application-x-executable"));
        image.set_pixel_size(32);
        row.add_prefix(&image);
        let (ui, dialog) = (ui.clone(), dialog.clone());
        let (name_, class_) = (name.clone(), class.clone());
        row.connect_activated(move |_| {
            dialog.close();
            ui.open_or_add(name_.clone(), class_.clone());
        });
        list.append(&row);
        rows.push((row, format!("{name} {class}").to_lowercase()));
    }

    search.connect_search_changed(move |search| {
        let query = search.text().to_lowercase();
        for (row, haystack) in &rows {
            row.set_visible(haystack.contains(query.trim()));
        }
    });

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(6);
    content.set_margin_bottom(12);
    content.append(&search);
    content.append(&list);
    let scroller = gtk4::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&ui.window));
    search.grab_focus();
}
