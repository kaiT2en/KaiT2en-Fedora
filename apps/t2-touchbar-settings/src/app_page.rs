// SPDX-License-Identifier: GPL-3.0-or-later

//! The page of one app: its window class, a preview of its row, its keys.

use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;

use crate::{key_dialog, model, pickers, window::Shared};

pub struct Page {
    pub ui: Shared,
    pub index: usize,
    keys: gtk4::ListBox,
    preview: gtk4::Box,
    overflow: gtk4::Label,
    nav_page: adw::NavigationPage,
}

impl Page {
    /// Rebuilds the key rows and the preview from the model.
    pub fn refresh(self: &Rc<Self>) {
        let file = self.ui.file.borrow();
        let app = &file.app[self.index];
        self.nav_page.set_title(app.title());

        while let Some(child) = self.preview.first_child() {
            self.preview.remove(&child);
        }
        let esc = gtk4::Label::new(Some("esc"));
        esc.add_css_class("esc");
        self.preview.append(&esc);
        for key in app.key.iter().take(model::KEYS_THAT_FIT) {
            let label = gtk4::Label::new(Some(key.label.trim()));
            if key.problem().is_some() {
                label.set_opacity(0.4);
            }
            self.preview.append(&label);
        }
        let hidden = app.key.len().saturating_sub(model::KEYS_THAT_FIT);
        self.overflow.set_visible(hidden > 0);
        self.overflow.set_label(&format!(
            "{} {} past the first {} {} not shown on the Touch Bar.",
            hidden,
            if hidden == 1 { "key" } else { "keys" },
            model::KEYS_THAT_FIT,
            if hidden == 1 { "is" } else { "are" },
        ));

        self.keys.remove_all();
        let count = app.key.len();
        for (index, key) in app.key.iter().enumerate() {
            self.keys.append(&self.key_row(index, count, key));
        }
        let add = adw::ButtonRow::builder()
            .title("Add Key")
            .start_icon_name("list-add-symbolic")
            .build();
        let page = self.clone();
        add.connect_activated(move |_| key_dialog::open(&page, None));
        self.keys.append(&add);
    }

    fn key_row(self: &Rc<Self>, index: usize, count: usize, key: &model::Key) -> adw::ActionRow {
        let subtitle = match (key.problem(), &key.send, &key.run) {
            (Some(problem), _, _) => problem,
            (None, Some(send), _) => model::pretty_combination(send),
            (None, None, Some(run)) => format!("Runs {run}"),
            (None, None, None) => String::new(),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(key.label.trim()))
            .subtitle(glib::markup_escape_text(&subtitle))
            .subtitle_lines(1)
            .activatable(true)
            .build();
        if key.problem().is_some() {
            row.add_css_class("error");
            row.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
        } else if index >= model::KEYS_THAT_FIT {
            row.add_css_class("dim-label");
        }

        let up = icon_button("go-up-symbolic", "Move Up");
        up.set_sensitive(index > 0);
        let page = self.clone();
        up.connect_clicked(move |_| page.move_key(index, index - 1));
        let down = icon_button("go-down-symbolic", "Move Down");
        down.set_sensitive(index + 1 < count);
        let page = self.clone();
        down.connect_clicked(move |_| page.move_key(index, index + 1));
        let remove = icon_button("user-trash-symbolic", "Remove");
        let page = self.clone();
        remove.connect_clicked(move |_| {
            page.ui.file.borrow_mut().app[page.index].key.remove(index);
            page.changed();
        });
        row.add_suffix(&up);
        row.add_suffix(&down);
        row.add_suffix(&remove);

        let page = self.clone();
        row.connect_activated(move |_| key_dialog::open(&page, Some(index)));
        row
    }

    fn move_key(self: &Rc<Self>, from: usize, to: usize) {
        self.ui.file.borrow_mut().app[self.index].key.swap(from, to);
        self.changed();
    }

    pub fn changed(self: &Rc<Self>) {
        self.ui.changed();
        self.refresh();
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk4::Align::Center);
    button.add_css_class("flat");
    button
}

pub fn open(ui: &Shared, index: usize) {
    let (name, class) = {
        let file = ui.file.borrow();
        let app = &file.app[index];
        (app.name.clone(), app.wm_class.clone())
    };

    let preview = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    preview.add_css_class("touchbar-preview");
    let scroller = gtk4::ScrolledWindow::builder()
        .child(&preview)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .build();
    let overflow = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .margin_top(6)
        .build();
    overflow.add_css_class("dim-label");
    overflow.add_css_class("caption");
    let preview_group = adw::PreferencesGroup::builder()
        .title("Touch Bar")
        .description("The row appears after the special keys while the app has the focus.")
        .build();
    preview_group.add(&scroller);
    preview_group.add(&overflow);

    let name_row = adw::EntryRow::builder().title("Name").text(&name).build();
    let class_row = adw::EntryRow::builder()
        .title("Window Class")
        .text(&class)
        .build();
    let detect = icon_button("find-location-symbolic", "Detect From the Focused Window");
    class_row.add_suffix(&detect);
    let app_group = adw::PreferencesGroup::builder()
        .title("App")
        .description(
            "The window class decides which app this row belongs to. Detect reads it \
             from the window you switch to.",
        )
        .build();
    app_group.add(&name_row);
    app_group.add(&class_row);

    let keys = gtk4::ListBox::new();
    keys.add_css_class("boxed-list");
    keys.set_selection_mode(gtk4::SelectionMode::None);
    let keys_group = adw::PreferencesGroup::builder()
        .title("Keys")
        .description("Each key sends a shortcut or runs a command.")
        .build();
    keys_group.add(&keys);

    let remove = adw::ButtonRow::builder().title("Remove App").build();
    remove.add_css_class("destructive-action");
    let remove_list = gtk4::ListBox::new();
    remove_list.add_css_class("boxed-list");
    remove_list.set_selection_mode(gtk4::SelectionMode::None);
    remove_list.append(&remove);
    let remove_group = adw::PreferencesGroup::new();
    remove_group.add(&remove_list);

    let prefs = adw::PreferencesPage::new();
    prefs.add(&preview_group);
    prefs.add(&app_group);
    prefs.add(&keys_group);
    prefs.add(&remove_group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&prefs));
    let nav_page = adw::NavigationPage::builder()
        .title("App")
        .tag("app")
        .child(&toolbar)
        .build();

    let page = Rc::new(Page {
        ui: ui.clone(),
        index,
        keys,
        preview,
        overflow,
        nav_page: nav_page.clone(),
    });

    let page_ = page.clone();
    name_row.connect_changed(move |row| {
        page_.ui.file.borrow_mut().app[page_.index].name = row.text().to_string();
        page_.ui.changed();
        let title = page_.ui.file.borrow().app[page_.index].title().to_owned();
        page_.nav_page.set_title(&title);
    });
    let page_ = page.clone();
    class_row.connect_changed(move |row| {
        let class = row.text().trim().to_owned();
        if class.is_empty() {
            row.add_css_class("error");
        } else {
            row.remove_css_class("error");
        }
        page_.ui.file.borrow_mut().app[page_.index].wm_class = class;
        page_.ui.changed();
    });
    let class_row_ = class_row.clone();
    let page_ = page.clone();
    detect.connect_clicked(move |_| {
        let class_row = class_row_.clone();
        pickers::countdown(&page_.ui, move |class| class_row.set_text(&class));
    });
    let page_ = page.clone();
    remove.connect_activated(move |_| {
        page_.ui.file.borrow_mut().app.remove(page_.index);
        page_.ui.changed();
        page_.ui.nav.pop_to_tag("apps");
    });

    page.refresh();
    ui.nav.push(&nav_page);
}
