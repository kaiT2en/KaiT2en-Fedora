// SPDX-License-Identifier: GPL-3.0-or-later

//! Adding or editing one key: its label and its shortcut or command. The
//! shortcut can be recorded by pressing it.

use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk4::{gdk, glib};

use crate::{app_page::Page, model};

const SHORTCUT: u32 = 0;
const COMMAND: u32 = 1;

/// The shortcut of a key press, or None for a lone modifier.
fn recorded(keycode: u32, state: gdk::ModifierType) -> Option<String> {
    // GDK key codes are evdev codes plus 8 on Linux.
    let name = model::key_name(keycode.checked_sub(8)?)?;
    let mut parts = Vec::new();
    for (mask, modifier) in [
        (gdk::ModifierType::CONTROL_MASK, "ctrl"),
        (gdk::ModifierType::SHIFT_MASK, "shift"),
        (gdk::ModifierType::ALT_MASK, "alt"),
        (gdk::ModifierType::SUPER_MASK, "super"),
    ] {
        if state.contains(mask) {
            parts.push(modifier);
        }
    }
    parts.push(name);
    Some(parts.join("+"))
}

pub fn open(page: &Rc<Page>, index: Option<usize>) {
    let key = index
        .map(|index| page.ui.file.borrow().app[page.index].key[index].clone())
        .unwrap_or(model::Key {
            label: String::new(),
            send: Some(String::new()),
            run: None,
        });

    let label_row = adw::EntryRow::builder()
        .title("Label")
        .text(&key.label)
        .build();
    let kind_row = adw::ComboRow::builder()
        .title("Action")
        .model(&gtk4::StringList::new(&["Send Shortcut", "Run Command"]))
        .selected(if key.run.is_some() { COMMAND } else { SHORTCUT })
        .build();
    let shortcut_row = adw::EntryRow::builder()
        .title("Shortcut, like ctrl+shift+t")
        .text(key.send.as_deref().unwrap_or_default())
        .build();
    let record = gtk4::ToggleButton::builder()
        .label("Record")
        .valign(gtk4::Align::Center)
        .tooltip_text("Press the shortcut to fill it in")
        .build();
    shortcut_row.add_suffix(&record);
    let command_row = adw::EntryRow::builder()
        .title("Command, like notify-send hi")
        .text(key.run.as_deref().unwrap_or_default())
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&label_row);
    group.add(&kind_row);
    group.add(&shortcut_row);
    group.add(&command_row);

    let hint = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .margin_top(12)
        .build();
    hint.add_css_class("caption");
    group.add(&hint);

    let cancel = gtk4::Button::with_label("Cancel");
    let done = gtk4::Button::with_label(if index.is_some() { "Done" } else { "Add" });
    done.add_css_class("suggested-action");
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&done);

    let prefs = adw::PreferencesPage::new();
    prefs.add(&group);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let dialog = adw::Dialog::builder()
        .title(if index.is_some() { "Edit Key" } else { "Add Key" })
        .content_width(440)
        .child(&toolbar)
        .build();

    // The key as the widgets describe it.
    let draft = {
        let (label_row, kind_row, shortcut_row, command_row) = (
            label_row.clone(),
            kind_row.clone(),
            shortcut_row.clone(),
            command_row.clone(),
        );
        Rc::new(move || {
            let command = kind_row.selected() == COMMAND;
            model::Key {
                label: label_row.text().trim().to_owned(),
                send: (!command).then(|| shortcut_row.text().trim().to_owned()),
                run: command.then(|| command_row.text().trim().to_owned()),
            }
        })
    };
    let update = {
        let (draft, kind_row, shortcut_row, command_row, hint, done) = (
            draft.clone(),
            kind_row.clone(),
            shortcut_row.clone(),
            command_row.clone(),
            hint.clone(),
            done.clone(),
        );
        Rc::new(move || {
            let command = kind_row.selected() == COMMAND;
            shortcut_row.set_visible(!command);
            command_row.set_visible(command);
            let problem = draft().problem();
            done.set_sensitive(problem.is_none());
            match problem {
                Some(problem) => {
                    hint.set_label(&problem);
                    hint.add_css_class("error");
                    hint.remove_css_class("dim-label");
                }
                None => {
                    hint.set_label(if command {
                        "Runs through systemd-run --user, like a command typed in a shell."
                    } else {
                        "Keys name their US layout position. The shortcut reaches the \
                         app as is, past key remappers like Toshy."
                    });
                    hint.remove_css_class("error");
                    hint.add_css_class("dim-label");
                }
            }
        })
    };
    for row in [&label_row, &shortcut_row, &command_row] {
        let update = update.clone();
        row.connect_changed(move |_| update());
    }
    let update_ = update.clone();
    kind_row.connect_selected_notify(move |_| update_());
    update();

    let recording = Rc::new(Cell::new(false));
    let (recording_, shortcut_row_) = (recording.clone(), shortcut_row.clone());
    record.connect_toggled(move |button| {
        recording_.set(button.is_active());
        if button.is_active() {
            button.set_label("Press Keys…");
            shortcut_row_.grab_focus();
        } else {
            button.set_label("Record");
        }
    });
    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let (recording_, record_, shortcut_row_) =
        (recording.clone(), record.clone(), shortcut_row.clone());
    keys.connect_key_pressed(move |_, _, keycode, state| {
        if !recording_.get() {
            return glib::Propagation::Proceed;
        }
        if let Some(shortcut) = recorded(keycode, state) {
            shortcut_row_.set_text(&shortcut);
            shortcut_row_.set_position(-1);
            record_.set_active(false);
        }
        glib::Propagation::Stop
    });
    dialog.add_controller(keys);

    let dialog_ = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_.close();
    });
    let (dialog_, page_) = (dialog.clone(), page.clone());
    done.connect_clicked(move |_| {
        let key = draft();
        if key.problem().is_some() {
            return;
        }
        {
            let mut file = page_.ui.file.borrow_mut();
            let keys = &mut file.app[page_.index].key;
            match index {
                Some(index) => keys[index] = key,
                None => keys.push(key),
            }
        }
        page_.changed();
        dialog_.close();
    });
    let done_ = done.clone();
    label_row.connect_entry_activated(move |_| done_.emit_clicked());

    dialog.present(Some(&page.ui.window));
    if index.is_none() {
        label_row.grab_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_presses_become_shortcuts() {
        let ctrl_shift = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        assert_eq!(recorded(20 + 8, ctrl_shift).as_deref(), Some("ctrl+shift+t"));
        assert_eq!(
            recorded(105 + 8, gdk::ModifierType::ALT_MASK).as_deref(),
            Some("alt+left")
        );
        assert_eq!(recorded(42 + 8, gdk::ModifierType::SHIFT_MASK), None);
    }
}
