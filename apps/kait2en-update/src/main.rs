// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
mod git;
mod map;
mod optional;
mod plan;
mod runner;
mod style;

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
};

use adw::prelude::*;
use gtk4::{
    glib, Align, Application, ApplicationWindow, Box as GtkBox, Button, DrawingArea, Grid, Label, LinkButton,
    ListBox, Orientation, Paned, PolicyType, ScrolledWindow, SelectionMode, Switch, TextView, WrapMode,
};
use optional::Part;
use plan::Plan;
use runner::{Event, Selection};

const APP_ID: &str = "org.kait2enupdate.gtk";
const APP_VERSION: &str = "0.01";
const MAX_COMMITS: usize = 60;
const MAX_REASONS: usize = 3;
const CARD_PADDING: i32 = 14;

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

struct Selected {
    name: String,
    is_app: bool,
    needs_reboot: bool,
    switch: Switch,
}

/// An optional part. The switch only marks a change, Update carries it out.
struct OptionalRow {
    part: &'static Part,
    switch: Switch,
    pending: Label,
}

impl OptionalRow {
    /// Install or remove, when the switch differs from what is installed.
    fn change(&self, home: &std::path::Path) -> Option<bool> {
        let wanted = self.switch.is_active();
        (wanted != self.part.is_installed(home)).then_some(wanted)
    }
}

struct Ui {
    title: adw::WindowTitle,
    check: Button,
    update: Button,
    status: Label,
    local_value: Label,
    remote_value: Label,
    count_value: Label,
    notice: Label,
    core_card: GtkBox,
    core_text: Label,
    install_all: Button,
    items_heading: Label,
    items: ListBox,
    unaffected: Label,
    commits_heading: Label,
    commits: ListBox,
    optional: Vec<OptionalRow>,
    syncing: Cell<bool>,
    log: TextView,
    result: Label,
    restart: Button,
    selected: RefCell<Vec<Selected>>,
    busy: Cell<bool>,
}

fn text_label(text: &str, classes: &[&str]) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_halign(Align::Start);
    label.set_wrap(true);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

fn boxed_list() -> ListBox {
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    list.add_css_class("boxed-list");
    list
}

fn clear(list: &ListBox) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
}

fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}

fn reasons_text(reasons: &[String]) -> String {
    let mut text = reasons.iter().take(MAX_REASONS).cloned().collect::<Vec<_>>().join(", ");
    if reasons.len() > MAX_REASONS {
        text.push_str(&format!(" and {} more", reasons.len() - MAX_REASONS));
    }
    text
}

fn overview_card(ui_labels: [&Label; 3]) -> GtkBox {
    let grid = Grid::new();
    grid.set_column_spacing(18);
    grid.set_row_spacing(6);
    grid.set_margin_start(CARD_PADDING);
    grid.set_margin_end(CARD_PADDING);
    grid.set_margin_top(CARD_PADDING);
    grid.set_margin_bottom(CARD_PADDING);
    for (row, (key, value)) in ["Local", "GitHub", "New commits"].iter().zip(ui_labels).enumerate() {
        grid.attach(&text_label(key, &["kait2en-key"]), 0, row as i32, 1, 1);
        value.set_xalign(0.0);
        value.set_hexpand(true);
        value.add_css_class("kait2en-value");
        grid.attach(value, 1, row as i32, 1, 1);
    }
    style::card(&grid)
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("KaiT2en Update")
        .default_width(1240)
        .default_height(780)
        .build();

    let title = adw::WindowTitle::new("Update", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    let check = Button::from_icon_name("view-refresh-symbolic");
    check.set_tooltip_text(Some("Check GitHub again"));
    header.pack_end(&check);
    window.set_titlebar(Some(&header));

    let root = GtkBox::new(Orientation::Vertical, 14);
    style::pad(&root);

    let status = text_label("Checking GitHub…", &["heading"]);
    root.append(&status);

    let local_value = Label::new(Some("--"));
    let remote_value = Label::new(Some("--"));
    let count_value = Label::new(Some("--"));
    root.append(&overview_card([&local_value, &remote_value, &count_value]));

    let notice = text_label("", &["kait2en-meta"]);
    notice.set_visible(false);
    root.append(&notice);

    let core_text = text_label("", &[]);
    let install_all = Button::with_label("Install Everything");
    install_all.set_halign(Align::Start);
    let core_inner = GtkBox::new(Orientation::Vertical, 10);
    core_inner.set_margin_start(CARD_PADDING);
    core_inner.set_margin_end(CARD_PADDING);
    core_inner.set_margin_top(CARD_PADDING);
    core_inner.set_margin_bottom(CARD_PADDING);
    core_inner.append(&core_text);
    core_inner.append(&install_all);
    let core_card = style::card(&core_inner);
    core_card.set_visible(false);
    root.append(&core_card);

    let items_heading = text_label("To install", &["heading"]);
    let items = boxed_list();
    let unaffected = text_label("", &["kait2en-meta"]);
    root.append(&items_heading);
    root.append(&items);
    root.append(&unaffected);

    let commits_heading = text_label("New on GitHub", &["heading"]);
    let commits = boxed_list();
    root.append(&commits_heading);
    root.append(&commits);

    // Parts the installer asks about, switched on and off right here.
    let parts = optional::available();
    let optional_list = boxed_list();
    let mut optional_rows = Vec::new();
    for part in &parts {
        let row = adw::ActionRow::new();
        row.set_title(part.title);
        row.set_subtitle(part.description);
        let pending = text_label("", &["kait2en-meta"]);
        pending.set_visible(false);
        row.add_suffix(&pending);
        let switch = Switch::new();
        switch.set_valign(Align::Center);
        row.add_suffix(&switch);
        row.set_activatable_widget(Some(&switch));
        optional_list.append(&row);
        optional_rows.push(OptionalRow { part, switch, pending });
    }
    if !parts.is_empty() {
        root.append(&text_label("Optional", &["heading"]));
        root.append(&optional_list);
    }

    let log = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(WrapMode::WordChar)
        .build();
    let log_scroll = ScrolledWindow::builder()
        .child(&log)
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    // Beside the content like in Kernel Builder, so the output is in view
    // without scrolling.
    let log_card = style::card(&log_scroll);
    log_card.add_css_class("kait2en-log");
    log_card.set_size_request(280, -1);
    log_card.set_margin_top(style::TOP_MARGIN);
    log_card.set_margin_bottom(style::BOTTOM_MARGIN);
    log_card.set_margin_end(style::SIDE_MARGIN);

    let scroll = ScrolledWindow::builder()
        .child(&root)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    scroll.set_size_request(360, -1);

    let workspace = Paned::builder()
        .orientation(Orientation::Horizontal)
        .vexpand(true)
        .wide_handle(true)
        .position(640)
        .start_child(&scroll)
        .end_child(&log_card)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .build();

    let result = text_label("", &[]);
    result.set_hexpand(true);
    let version = text_label(&format!("v{APP_VERSION}"), &["kait2en-meta"]);
    let update = Button::with_label("Update");
    update.add_css_class("suggested-action");
    update.set_sensitive(false);
    let restart = Button::with_label("Restart");
    restart.add_css_class("suggested-action");
    restart.set_visible(false);
    let donate = LinkButton::builder()
        .uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400")
        .label("Fund our bugs")
        .build();
    donate.add_css_class("donate-link");
    let footer = GtkBox::new(Orientation::Horizontal, 12);
    footer.set_margin_start(style::SIDE_MARGIN);
    footer.set_margin_end(style::SIDE_MARGIN);
    footer.set_margin_bottom(style::BOTTOM_MARGIN);
    footer.append(&donate);
    footer.append(&result);
    footer.append(&version);
    footer.append(&restart);
    footer.append(&update);

    let content = GtkBox::new(Orientation::Vertical, 0);
    content.append(&workspace);
    content.append(&footer);
    window.set_child(Some(&content));

    let ui = Rc::new(Ui {
        title,
        check,
        update,
        status,
        local_value,
        remote_value,
        count_value,
        notice,
        core_card,
        core_text,
        install_all,
        items_heading,
        items,
        unaffected,
        commits_heading,
        commits,
        optional: optional_rows,
        syncing: Cell::new(false),
        log,
        result,
        restart,
        selected: RefCell::new(Vec::new()),
        busy: Cell::new(false),
    });

    ui.check.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| start_check(&ui)));
    ui.update.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| {
        let selection = selected_items(&ui);
        start_apply(&ui, selection);
    }));
    ui.restart.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| reboot(&ui)));
    ui.install_all.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| start_apply(&ui, Selection::All)));
    for row in &ui.optional {
        row.switch.connect_active_notify(glib::clone!(
            #[weak]
            ui,
            move |_| {
                if !ui.syncing.get() {
                    show_pending(&ui);
                }
            }));
    }
    sync_optional(&ui);

    window.present();
    start_check(&ui);
}

fn home_dir() -> PathBuf {
    glib::home_dir()
}

fn set_busy(ui: &Ui, busy: bool) {
    ui.busy.set(busy);
    ui.check.set_sensitive(!busy);
    ui.install_all.set_sensitive(!busy);
    for row in &ui.optional {
        row.switch.set_sensitive(!busy);
    }
    update_button(ui);
}

/// Shows what is installed, also after a run that failed or was cancelled.
fn sync_optional(ui: &Ui) {
    let home = home_dir();
    ui.syncing.set(true);
    for row in &ui.optional {
        row.switch.set_active(row.part.is_installed(&home));
    }
    ui.syncing.set(false);
    show_pending(ui);
}

fn show_pending(ui: &Ui) {
    let home = home_dir();
    for row in &ui.optional {
        let change = row.change(&home);
        row.pending.set_label(match change {
            Some(true) => "will be installed",
            Some(false) => "will be removed",
            None => "",
        });
        row.pending.set_visible(change.is_some());
    }
    update_button(ui);
}

fn optional_changes(ui: &Ui) -> Vec<Selection> {
    let home = home_dir();
    ui.optional
        .iter()
        .filter_map(|row| row.change(&home).map(|install| Selection::Optional { part: row.part, install }))
        .collect()
}

fn update_button(ui: &Ui) {
    let any = ui.selected.borrow().iter().any(|item| item.switch.is_active())
        || !optional_changes(ui).is_empty();
    ui.update.set_sensitive(any && !ui.busy.get());
}

fn start_check(ui: &Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    set_busy(ui, true);
    ui.title.set_subtitle("Checking GitHub");
    let home = home_dir();
    runner::background(
        move || plan::check(&git::Repository::system(), &home),
        glib::clone!(
        #[strong]
        ui,
        move |checked: Result<Plan, String>| {
            set_busy(&ui, false);
            sync_optional(&ui);
            match checked {
                Ok(plan) => show_plan(&ui, &plan),
                Err(error) => {
                    ui.title.set_subtitle("Check failed");
                    ui.status.set_label("Could not compare with GitHub");
                    ui.notice.set_label(&error);
                    ui.notice.set_visible(true);
                }
            }
        }),
    );
}

fn show_plan(ui: &Rc<Ui>, plan: &Plan) {
    ui.local_value.set_label(&format!("{}  {}", short(&plan.local), plan.local_date));
    ui.remote_value.set_label(&format!("{}  {}", short(&plan.remote), plan.remote_date));
    ui.count_value.set_label(&plan.commits.len().to_string());

    ui.status.set_label(match (plan.items.is_empty(), plan.commits.is_empty()) {
        (true, true) => "KaiT2en is up to date",
        (true, false) => "Nothing needs to be installed",
        (false, _) => "Updates are available",
    });
    ui.title.set_subtitle(if plan.items.is_empty() { "Up to date" } else { "Updates available" });

    let mut notes = Vec::new();
    if let Some(error) = &plan.fetch_error {
        notes.push(format!("Could not fetch from GitHub, showing the last fetched state. {error}"));
    }
    if plan.estimated {
        notes.push("Some parts have no install record yet, they are compared with the local checkout.".to_owned());
    }
    ui.notice.set_label(&notes.join("\n"));
    ui.notice.set_visible(!notes.is_empty());

    ui.core_card.set_visible(!plan.core_changes.is_empty());
    ui.core_text.set_label(&format!(
        "The installer itself changed ({}). Installing everything is recommended.",
        reasons_text(&plan.core_changes)
    ));

    clear(&ui.items);
    let mut selected = Vec::new();
    for item in &plan.items {
        let row = adw::ActionRow::new();
        row.set_title(&escape(&item.label));
        row.set_subtitle(&escape(&reasons_text(&item.reasons)));
        let kind = text_label(if item.is_app { "app" } else { "system" }, &["kait2en-meta"]);
        row.add_suffix(&kind);
        let switch = Switch::new();
        switch.set_active(true);
        switch.set_valign(Align::Center);
        switch.connect_active_notify(glib::clone!(
            #[weak]
            ui,
            move |_| update_button(&ui)));
        row.add_suffix(&switch);
        row.set_activatable_widget(Some(&switch));
        ui.items.append(&row);
        selected.push(Selected {
            name: item.name.clone(),
            is_app: item.is_app,
            needs_reboot: item.needs_reboot(),
            switch,
        });
    }
    ui.selected.replace(selected);
    ui.items_heading.set_visible(!plan.items.is_empty());
    ui.items.set_visible(!plan.items.is_empty());
    ui.unaffected.set_label(&match plan.unaffected.len() {
        0 => String::new(),
        1 => "1 changed file needs no installation.".to_owned(),
        count => format!("{count} changed files need no installation."),
    });
    ui.unaffected.set_visible(!plan.unaffected.is_empty());

    clear(&ui.commits);
    for commit in plan.commits.iter().take(MAX_COMMITS) {
        let row = adw::ActionRow::new();
        row.set_title(&escape(&commit.subject));
        row.set_subtitle(&format!("{}  {}", commit.short, commit.date));
        ui.commits.append(&row);
    }
    if plan.commits.len() > MAX_COMMITS {
        let row = adw::ActionRow::new();
        row.set_title(&format!("and {} older commits", plan.commits.len() - MAX_COMMITS));
        ui.commits.append(&row);
    }
    ui.commits_heading.set_visible(!plan.commits.is_empty());
    ui.commits.set_visible(!plan.commits.is_empty());
    update_button(ui);
}

fn selected_items(ui: &Ui) -> Selection {
    let mut steps = Vec::new();
    let mut apps = Vec::new();
    for item in ui.selected.borrow().iter().filter(|item| item.switch.is_active()) {
        if item.is_app {
            apps.push(item.name.trim_start_matches("app:").to_owned());
        } else {
            steps.push(item.name.clone());
        }
    }
    Selection::Only { steps, apps }
}

fn append_log(ui: &Ui, line: &str) {
    let buffer = ui.log.buffer();
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, line);
    buffer.insert(&mut end, "\n");
    let mark = buffer.create_mark(None, &buffer.end_iter(), false);
    ui.log.scroll_mark_onscreen(&mark);
    buffer.delete_mark(&mark);
}

/// Runs the selection and then the marked optional changes, one after the
/// other. The first failure stops the rest.
fn start_apply(ui: &Rc<Ui>, selection: Selection) {
    if ui.busy.get() {
        return;
    }
    let reboot = match &selection {
        Selection::All => true,
        Selection::Optional { .. } => false,
        Selection::Only { .. } => ui
            .selected
            .borrow()
            .iter()
            .any(|item| item.switch.is_active() && item.needs_reboot),
    };
    let mut queue: VecDeque<Selection> = optional_changes(ui).into();
    let has_work = match &selection {
        Selection::Only { steps, apps } => !steps.is_empty() || !apps.is_empty(),
        _ => true,
    };
    if has_work {
        queue.push_front(selection);
    }
    let only_removals = queue
        .iter()
        .all(|selection| matches!(selection, Selection::Optional { install: false, .. }));

    set_busy(ui, true);
    ui.title.set_subtitle(if only_removals { "Removing" } else { "Installing" });
    ui.result.set_label("");
    ui.result.remove_css_class("success");
    ui.result.remove_css_class("error");
    ui.restart.set_visible(false);
    ui.log.buffer().set_text("");
    run_next(ui.clone(), queue, reboot, only_removals);
}

fn reboot(ui: &Rc<Ui>) {
    ui.restart.set_sensitive(false);
    runner::background(
        || std::process::Command::new("systemctl").arg("reboot").output(),
        glib::clone!(
        #[strong]
        ui,
        move |output: std::io::Result<std::process::Output>| {
            let error = match output {
                Ok(output) if output.status.success() => return,
                Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                Err(error) => error.to_string(),
            };
            ui.restart.set_sensitive(true);
            ui.result.set_label(&format!("Could not restart: {error}"));
            ui.result.remove_css_class("success");
            ui.result.add_css_class("error");
        }),
    );
}

fn run_next(ui: Rc<Ui>, mut queue: VecDeque<Selection>, reboot: bool, only_removals: bool) {
    let Some(selection) = queue.pop_front() else {
        set_busy(&ui, false);
        ui.result.set_label(match (reboot, only_removals) {
            (true, _) => "Installed. Restart to load the new modules.",
            (false, true) => "Removed.",
            (false, false) => "Installed.",
        });
        ui.result.add_css_class("success");
        ui.restart.set_visible(reboot);
        start_check(&ui);
        return;
    };
    let mut queue = Some(queue);
    runner::apply(&selection, move |event| match event {
        Event::Line(line) => append_log(&ui, &line),
        Event::Finished(Ok(())) => {
            if let Some(queue) = queue.take() {
                run_next(ui.clone(), queue, reboot, only_removals);
            }
        }
        Event::Finished(Err(error)) => {
            set_busy(&ui, false);
            ui.result.set_label(&error);
            ui.result.add_css_class("error");
            start_check(&ui);
        }
    });
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| style::install());
    app.connect_activate(build_ui);
    app.run()
}
