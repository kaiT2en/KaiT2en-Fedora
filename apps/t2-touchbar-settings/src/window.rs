// SPDX-License-Identifier: GPL-3.0-or-later

//! The main window: the list of apps, saving, and the unsaved-changes guard.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use gtk4::{gio, glib};

use crate::{app_page, focus, model, pickers};

/// The header wordmark shared by the KAIT2EN apps.
const WORDMARK: &str = "/usr/local/share/kait2en/kait2en-wordmark.png";

pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub nav: adw::NavigationView,
    save: gtk4::Button,
    apps: gtk4::ListBox,
    pub file: RefCell<model::File>,
    saved: RefCell<model::File>,
    path: PathBuf,
}

pub type Shared = Rc<Ui>;

impl Ui {
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Call after every change of `file`.
    pub fn changed(self: &Rc<Self>) {
        let dirty = *self.file.borrow() != *self.saved.borrow();
        self.save.set_sensitive(dirty);
        self.refresh_apps();
    }

    fn refresh_apps(self: &Rc<Self>) {
        self.apps.remove_all();
        let file = self.file.borrow();
        if file.app.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No apps yet")
                .subtitle("Add one with the + button")
                .build();
            row.add_css_class("dim-label");
            self.apps.append(&row);
            return;
        }
        for (index, app) in file.app.iter().enumerate() {
            let problems = app.key.iter().filter(|key| key.problem().is_some()).count();
            let keys = match app.key.len() {
                1 => "1 key".to_owned(),
                count => format!("{count} keys"),
            };
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(app.title()))
                .subtitle(glib::markup_escape_text(app.wm_class.trim()))
                .activatable(true)
                .build();
            if let Some(icon) = pickers::icon_for_class(app.wm_class.trim()) {
                let image = gtk4::Image::from_gicon(&icon);
                image.set_pixel_size(32);
                row.add_prefix(&image);
            }
            if problems > 0 || app.wm_class.trim().is_empty() {
                let warning = gtk4::Image::from_icon_name("dialog-warning-symbolic");
                warning.set_tooltip_text(Some("Some entries need fixing"));
                warning.add_css_class("warning");
                row.add_suffix(&warning);
            }
            let count = gtk4::Label::new(Some(&keys));
            count.add_css_class("dim-label");
            row.add_suffix(&count);
            row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            let ui = self.clone();
            row.connect_activated(move |_| app_page::open(&ui, index));
            self.apps.append(&row);
        }
    }

    /// Opens the profile for `class`, adding it first if there is none.
    pub fn open_or_add(self: &Rc<Self>, name: String, class: String) {
        let existing = self
            .file
            .borrow()
            .app
            .iter()
            .position(|app| app.wm_class.trim().eq_ignore_ascii_case(&class));
        let index = existing.unwrap_or_else(|| {
            let mut file = self.file.borrow_mut();
            file.app.push(model::App {
                name,
                wm_class: class,
                key: Vec::new(),
            });
            file.app.len() - 1
        });
        self.changed();
        // Only one app page at a time: indexes stay valid.
        self.nav.pop_to_tag("apps");
        app_page::open(self, index);
    }

    fn problem(&self) -> Option<String> {
        let file = self.file.borrow();
        for app in &file.app {
            if app.wm_class.trim().is_empty() {
                return Some(format!("{} has no window class", app.title()));
            }
            if app.key.iter().any(|key| key.problem().is_some()) {
                return Some(format!("Fix the marked keys of {} first", app.title()));
            }
        }
        None
    }

    /// Writes apps.toml and restarts the Touch Bar. False when nothing was
    /// written.
    pub fn save(self: &Rc<Self>) -> bool {
        if let Some(problem) = self.problem() {
            self.toast(&problem);
            return false;
        }
        let file = self.file.borrow().clone();
        if let Err(error) = model::save(&self.path, &file) {
            self.toast(&format!("Could not save: {error}"));
            return false;
        }
        *self.saved.borrow_mut() = file;
        self.changed();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match restart_touchbar().await {
                Ok(()) => ui.toast("Saved. The Touch Bar uses the new rows now"),
                Err(error) => ui.toast(&format!("Saved, but the Touch Bar did not restart: {error}")),
            }
        });
        true
    }
}

async fn restart_touchbar() -> Result<(), glib::Error> {
    let process = gio::Subprocess::newv(
        &[
            "systemctl".as_ref(),
            "--user".as_ref(),
            "try-restart".as_ref(),
            "kait2en-touchbar.service".as_ref(),
        ],
        gio::SubprocessFlags::NONE,
    )?;
    process.wait_check_future().await
}

pub fn build(app: &adw::Application) {
    let path = model::path();
    let (file, load_error) = match model::load(&path) {
        Ok(file) => (file, None),
        Err(error) => (model::File::default(), Some(error)),
    };

    let save = gtk4::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_sensitive(false);
    save.set_action_name(Some("win.save"));

    let add = gtk4::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add App")
        .build();
    let menu = gio::Menu::new();
    menu.append(Some("Detect Focused App…"), Some("win.detect"));
    menu.append(Some("Choose Installed App…"), Some("win.choose"));
    add.set_menu_model(Some(&menu));

    let header = adw::HeaderBar::new();
    if std::path::Path::new(WORDMARK).exists() {
        let brand = gtk4::Picture::for_filename(WORDMARK);
        brand.set_content_fit(gtk4::ContentFit::Contain);
        brand.set_size_request(80, 21);
        brand.set_valign(gtk4::Align::Center);
        brand.set_margin_start(10);
        brand.set_margin_end(6);
        header.pack_start(&brand);
    }
    header.pack_start(&add);
    header.pack_end(&save);

    let apps = gtk4::ListBox::new();
    apps.add_css_class("boxed-list");
    apps.set_selection_mode(gtk4::SelectionMode::None);
    let apps_group = adw::PreferencesGroup::builder()
        .title("Apps")
        .description(
            "While one of these apps has the focus, the Touch Bar shows its row of keys. \
             Swipe with two fingers to reach the other rows.",
        )
        .build();
    apps_group.add(&apps);
    let prefs = adw::PreferencesPage::new();
    prefs.add(&apps_group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);

    let load_banner = adw::Banner::new("");
    if let Some(error) = &load_error {
        load_banner.set_title(&format!(
            "apps.toml could not be read ({error}). Saving replaces it and keeps a backup."
        ));
        load_banner.set_revealed(true);
    }
    toolbar.add_top_bar(&load_banner);

    let extension_banner = adw::Banner::builder()
        .title("The Touch Bar needs the Focused Window D-Bus extension to follow apps")
        .button_label("Get It")
        .build();
    toolbar.add_top_bar(&extension_banner);
    toolbar.set_content(Some(&prefs));

    let root = adw::NavigationPage::builder()
        .title("Touch Bar Apps")
        .tag("apps")
        .child(&toolbar)
        .build();
    let nav = adw::NavigationView::new();
    nav.add(&root);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Touch Bar Settings")
        .default_width(560)
        .default_height(700)
        .width_request(360)
        .height_request(360)
        .content(&toasts)
        .build();

    let ui = Rc::new(Ui {
        window: window.clone(),
        toasts,
        nav,
        save,
        apps,
        saved: RefCell::new(file.clone()),
        file: RefCell::new(file),
        path,
    });

    let action = gio::SimpleAction::new("save", None);
    let ui_ = ui.clone();
    action.connect_activate(move |_, _| {
        ui_.save();
    });
    window.add_action(&action);

    let action = gio::SimpleAction::new("detect", None);
    let ui_ = ui.clone();
    action.connect_activate(move |_, _| pickers::detect(&ui_));
    window.add_action(&action);

    let action = gio::SimpleAction::new("choose", None);
    let ui_ = ui.clone();
    action.connect_activate(move |_, _| pickers::choose_installed(&ui_));
    window.add_action(&action);

    let window_ = window.clone();
    extension_banner.connect_button_clicked(move |_| {
        gtk4::UriLauncher::new(focus::EXTENSION_URL).launch(
            Some(&window_),
            None::<&gio::Cancellable>,
            |_| {},
        );
    });
    glib::spawn_future_local(async move {
        if !focus::available().await {
            extension_banner.set_revealed(true);
        }
    });

    let ui_ = ui.clone();
    window.connect_close_request(move |window| {
        if *ui_.file.borrow() == *ui_.saved.borrow() {
            return glib::Propagation::Proceed;
        }
        confirm_close(&ui_, window);
        glib::Propagation::Stop
    });

    ui.changed();
    window.present();
}

fn confirm_close(ui: &Shared, window: &adw::ApplicationWindow) {
    let dialog = adw::AlertDialog::new(
        Some("Save Changes?"),
        Some("The Touch Bar keeps the old rows until you save."),
    );
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        ("discard", "Discard"),
        ("save", "Save"),
    ]);
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let ui = ui.clone();
    dialog.connect_response(None, move |_, response| {
        let close = match response {
            "discard" => true,
            "save" => ui.save(),
            _ => false,
        };
        if close {
            *ui.saved.borrow_mut() = ui.file.borrow().clone();
            ui.window.close();
        }
    });
    dialog.present(Some(window));
}
