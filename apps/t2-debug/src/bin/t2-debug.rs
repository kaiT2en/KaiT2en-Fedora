// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! t2-debug: a gtk4-libadwaita front end for the T2's Instruments dtservicehub.
//! Tab 1 is a per-process "top" (sysmontap) with deviceinfo selector buttons;
//! tab 2 is the decoded live os_log stream (activitytracetap) with a regex
//! filter and clickable subsystem chips. Read/observe only. Visual style mirrors
//! the other kait2en gtk apps.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gdk, gio, glib};

use t2_debug::logfmt::LogLine;
use t2_debug::session::{Event, Mode, Selector, Session};

const APP_ID: &str = "org.t2debug.gtk";
const APP_VERSION: &str = "0.1.0";
const LOG_RING: usize = 5000;

/// A GObject wrapper around one sysmon process row, for the ColumnView model.
mod proc_object {
    use std::cell::{Cell, RefCell};

    use gtk4::glib;
    use gtk4::subclass::prelude::*;

    #[derive(Default)]
    pub struct ProcObjectImp {
        pub pid: Cell<i64>,
        pub name: RefCell<String>,
        pub cpu: Cell<f64>,
        pub power: Cell<f64>,
        pub wakeups: Cell<i64>,
        pub foot_mb: Cell<f64>,
        pub disk_kb: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProcObjectImp {
        const NAME: &'static str = "T2DebugProc";
        type Type = super::ProcObject;
    }
    impl ObjectImpl for ProcObjectImp {}
}

gtk4::glib::wrapper! {
    pub struct ProcObject(ObjectSubclass<proc_object::ProcObjectImp>);
}

impl ProcObject {
    fn new(p: &t2_debug::sysmon::Proc) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.pid.set(p.pid);
        *imp.name.borrow_mut() = p.name.clone();
        imp.cpu.set(p.cpu);
        imp.power.set(p.power);
        imp.wakeups.set(p.wakeups);
        imp.foot_mb.set(p.foot_mb);
        imp.disk_kb.set(p.disk_kb);
        obj
    }
    fn imp(&self) -> &proc_object::ProcObjectImp {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        ObjectSubclassIsExt::imp(self)
    }
    fn pid(&self) -> i64 {
        self.imp().pid.get()
    }
    fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }
    fn cpu(&self) -> f64 {
        self.imp().cpu.get()
    }
    fn power(&self) -> f64 {
        self.imp().power.get()
    }
    fn wakeups(&self) -> i64 {
        self.imp().wakeups.get()
    }
    fn foot_mb(&self) -> f64 {
        self.imp().foot_mb.get()
    }
    fn disk_kb(&self) -> f64 {
        self.imp().disk_kb.get()
    }
}

fn main() -> glib::ExitCode {
    register_embedded_resources();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/org/t2debug/gtk")
        .build();
    app.connect_startup(|_| install_palette());
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let (events_tx, events_rx) = channel::<Event>();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some("T2 Debug"))));
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    let status = gtk4::Label::new(Some("connecting…"));
    status.add_css_class("dim-label");
    header.pack_end(&status);

    let switcher = gtk4::StackSwitcher::new();
    let stack = gtk4::Stack::builder()
        .vexpand(true)
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .build();
    switcher.set_stack(Some(&stack));
    header.set_title_widget(Some(&switcher));

    let device = build_device_tab();
    let log = build_log_tab();
    stack.add_titled(&device.root, Some("deviceinfo"), "deviceinfo");
    stack.add_titled(&log.root, Some("logs"), "live log");

    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    footer.set_margin_start(18);
    footer.set_margin_end(18);
    footer.set_margin_top(6);
    footer.set_margin_bottom(6);
    let donate = gtk4::LinkButton::builder()
        .uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400")
        .label("Fund our bugs")
        .build();
    donate.add_css_class("donate-link");
    footer.append(&donate);
    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    footer.append(&gtk4::Label::new(Some(&format!("v{APP_VERSION}"))));

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("debug-root");
    root.append(&header);
    root.append(&stack);
    root.append(&footer);

    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some("T2 Debug"));
    window.set_default_size(1112, 760);
    window.set_content(Some(&root));

    // One long-lived connection for the whole app; the visible tab just picks
    // the mode. No reconnect per tab, so the deviceinfo backend is not churned.
    let session = Rc::new(Session::start(None, None, events_tx.clone()));

    let mode_for = |name: &str| if name == "logs" { Mode::Logs } else { Mode::Sysmon };
    {
        let session = session.clone();
        stack.connect_visible_child_name_notify(move |stack| {
            if let Some(name) = stack.visible_child_name() {
                session.set_mode(mode_for(name.as_str()));
            }
        });
    }

    device.wire_selectors(session.clone());

    // Drain worker events on the main loop and update the visible tab.
    let drain = DrainState {
        status: status.clone(),
        device: device.clone(),
        log: log.clone(),
    };
    let events_rx = Rc::new(events_rx);
    glib::timeout_add_local(Duration::from_millis(150), move || {
        drain.pump(&events_rx);
        glib::ControlFlow::Continue
    });

    // Stop the worker cleanly when the window closes.
    {
        let session = session.clone();
        window.connect_close_request(move |_| {
            session.shutdown();
            glib::Propagation::Proceed
        });
    }

    session.set_mode(Mode::Sysmon); // the initial deviceinfo tab
    window.present();
}

/// Drains events and routes them to the two tabs' widgets.
struct DrainState {
    status: gtk4::Label,
    device: DeviceTab,
    log: LogTab,
}

impl DrainState {
    fn pump(&self, rx: &Receiver<Event>) {
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Connected => self.status.set_text("connected"),
                Event::Error(error) => self.status.set_text(&format!("error: {error}")),
                Event::Sysmon { cpu_total, system, procs } => {
                    self.device.update(cpu_total, &system, &procs)
                }
                Event::Selector { name, text } => self.device.show_selector(&name, &text),
                Event::Log(line) => self.log.push(line),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 1: deviceinfo "top"
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DeviceTab {
    root: gtk4::Box,
    sys_label: gtk4::Label,
    store: gio::ListStore,
    result: gtk4::TextView,
    result_expander: gtk4::Expander,
    ls_entry: gtk4::Entry,
    buttons: Rc<Vec<(gtk4::Button, &'static str)>>,
}

fn build_device_tab() -> DeviceTab {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    root.set_margin_start(18);
    root.set_margin_end(18);
    root.set_margin_top(12);
    root.set_margin_bottom(12);

    let sys_label = gtk4::Label::new(Some("system: --"));
    sys_label.set_xalign(0.0);
    sys_label.add_css_class("overview-value");
    root.append(&sys_label);

    // Sortable process table with a fixed header and fixed column widths.
    let store = gio::ListStore::new::<ProcObject>();
    let sort_model = gtk4::SortListModel::new(Some(store.clone()), None::<gtk4::Sorter>);
    let selection = gtk4::NoSelection::new(Some(sort_model.clone()));
    let column_view = gtk4::ColumnView::new(Some(selection));
    column_view.set_vexpand(true);

    add_column(&column_view, "pid", 70, 1.0, false, |p| p.pid().to_string(), |a, b| {
        a.pid().cmp(&b.pid())
    });
    let name_col =
        add_column(&column_view, "name", 220, 0.0, true, |p| p.name(), |a, b| a.name().cmp(&b.name()));
    name_col.set_expand(true);
    let cpu_col = add_column(&column_view, "cpu%", 80, 1.0, false, |p| format!("{:.2}", p.cpu()), |a, b| {
        order(a.cpu(), b.cpu())
    });
    add_column(&column_view, "power", 80, 1.0, false, |p| format!("{:.2}", p.power()), |a, b| {
        order(a.power(), b.power())
    });
    add_column(&column_view, "wakeups", 90, 1.0, false, |p| p.wakeups().to_string(), |a, b| {
        a.wakeups().cmp(&b.wakeups())
    });
    add_column(&column_view, "footMB", 90, 1.0, false, |p| format!("{:.1}", p.foot_mb()), |a, b| {
        order(a.foot_mb(), b.foot_mb())
    });
    add_column(&column_view, "diskWkB", 90, 1.0, false, |p| format!("{:.0}", p.disk_kb()), |a, b| {
        order(a.disk_kb(), b.disk_kb())
    });

    sort_model.set_sorter(column_view.sorter().as_ref());
    column_view.sort_by_column(Some(&cpu_col), gtk4::SortType::Descending);

    let procs_scroll = gtk4::ScrolledWindow::builder().vexpand(true).child(&column_view).build();
    procs_scroll.add_css_class("debug-panel");
    root.append(&procs_scroll);

    let buttons_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let mut buttons = Vec::new();
    for (label, id) in [
        ("machTime", "machTime"),
        ("network", "network"),
        ("traceCodes", "traceCodes"),
        ("capabilities", "capabilities"),
    ] {
        let button = gtk4::Button::with_label(label);
        buttons_box.append(&button);
        buttons.push((button, id));
    }
    let ls_entry = gtk4::Entry::builder().placeholder_text("/var/db").hexpand(true).build();
    let ls_button = gtk4::Button::with_label("ls");
    buttons_box.append(&ls_entry);
    buttons_box.append(&ls_button);
    buttons.push((ls_button, "ls"));
    root.append(&buttons_box);

    let result = gtk4::TextView::new();
    result.set_editable(false);
    result.set_monospace(true);
    let result_scroll = gtk4::ScrolledWindow::builder()
        .height_request(200)
        .child(&result)
        .build();
    result_scroll.add_css_class("debug-panel");
    // Collapsed until a selector runs, so it does not reserve empty space.
    let result_expander = gtk4::Expander::new(Some("selector output"));
    result_expander.set_child(Some(&result_scroll));
    root.append(&result_expander);

    DeviceTab { root, sys_label, store, result, result_expander, ls_entry, buttons: Rc::new(buttons) }
}

/// Compare two f64 sort keys, treating NaN as equal.
fn order(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// A byte count as a short unit string (e.g. "9.5 GB"), for the system strip.
fn human_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Add one sortable, fixed-width column rendering a `ProcObject`.
fn add_column<F, S>(
    view: &gtk4::ColumnView,
    title: &str,
    width: i32,
    xalign: f32,
    expand: bool,
    fmt: F,
    cmp: S,
) -> gtk4::ColumnViewColumn
where
    F: Fn(&ProcObject) -> String + 'static,
    S: Fn(&ProcObject, &ProcObject) -> std::cmp::Ordering + 'static,
{
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk4::Label::new(None);
        label.set_xalign(xalign);
        item.downcast_ref::<gtk4::ListItem>().unwrap().set_child(Some(&label));
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
        if let (Some(obj), Some(label)) =
            (item.item().and_downcast::<ProcObject>(), item.child().and_downcast::<gtk4::Label>())
        {
            label.set_text(&fmt(&obj));
        }
    });
    let column = gtk4::ColumnViewColumn::new(Some(title), Some(factory));
    column.set_fixed_width(width);
    column.set_expand(expand);
    column.set_resizable(true);
    let sorter = gtk4::CustomSorter::new(move |a, b| {
        let a = a.downcast_ref::<ProcObject>().unwrap();
        let b = b.downcast_ref::<ProcObject>().unwrap();
        cmp(a, b).into()
    });
    column.set_sorter(Some(&sorter));
    view.append_column(&column);
    column
}

impl DeviceTab {
    fn update(&self, cpu_total: f64, system: &[(String, String)], procs: &[t2_debug::sysmon::Proc]) {
        // Parse a system counter as a number (decode_system renders them as
        // plain strings).
        let num = |name: &str| -> f64 {
            system
                .iter()
                .find(|(k, _)| k == name)
                .and_then(|(_, v)| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        // VM counters are in pages; the T2's user page size is 16 KiB.
        let page = 16384.0;
        // Fixed field widths keep the monospace strip from shifting as values
        // change (cpu 0 vs 100, short vs long byte counts).
        self.sys_label.set_text(&format!(
            "cpu {:>3.0}%    mem {:>9} / {:>9}    net ↓{:>9} ↑{:>9}    disk ↓{:>9} ↑{:>9}    threads {:>4}",
            cpu_total,
            human_bytes(num("vmUsedCount") * page),
            human_bytes(num("physMemSize") * page),
            human_bytes(num("netBytesIn")),
            human_bytes(num("netBytesOut")),
            human_bytes(num("diskBytesRead")),
            human_bytes(num("diskBytesWritten")),
            num("threadCount") as i64,
        ));
        self.store.remove_all();
        for p in procs {
            self.store.append(&ProcObject::new(p));
        }
    }

    fn show_selector(&self, name: &str, text: &str) {
        self.result.buffer().set_text(&format!("== {name}\n{text}\n"));
        self.result_expander.set_expanded(true);
    }

    fn wire_selectors(&self, session: Rc<Session>) {
        for (button, id) in self.buttons.iter() {
            let id = *id;
            let session = session.clone();
            let ls_entry = self.ls_entry.clone();
            button.connect_clicked(move |_| {
                let selector = match id {
                    "machTime" => Selector::MachTime,
                    "network" => Selector::NetworkInfo,
                    "traceCodes" => Selector::TraceCodes,
                    "capabilities" => Selector::Capabilities,
                    "ls" => Selector::Ls(ls_path(&ls_entry)),
                    _ => return,
                };
                session.request(selector);
            });
        }
        // Enter in the path field runs `ls`, like clicking the button.
        self.ls_entry.connect_activate(move |entry| {
            session.request(Selector::Ls(ls_path(entry)));
        });
    }
}

fn ls_path(entry: &gtk4::Entry) -> String {
    let path = entry.text();
    if path.is_empty() { "/".to_owned() } else { path.into() }
}

/// Scroll a text view to its end, deferred to idle so it runs after the layout
/// that the just-inserted text triggers (an inline scroll misses the new line).
fn scroll_to_tail(view: &gtk4::TextView) {
    let view = view.clone();
    glib::idle_add_local_once(move || {
        let buffer = view.buffer();
        let mut end = buffer.end_iter();
        view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    });
}

/// The colour tag name for a log level, or None for the default level.
fn level_tag(level: &str) -> Option<&'static str> {
    match level.to_ascii_lowercase().as_str() {
        "fault" => Some("fault"),
        "error" => Some("error"),
        "info" => Some("info"),
        "debug" => Some("debug"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tab 2: live log
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LogTab {
    root: gtk4::Box,
    view: gtk4::TextView,
    filter: gtk4::Entry,
    no_hid: gtk4::CheckButton,
    /// Vertical list of one check button per subsystem currently in the buffer.
    subsystems: gtk4::Box,
    ring: Rc<RefCell<VecDeque<LogLine>>>,
    /// Checkbox per subsystem present in the ring (or still ticked).
    rows: Rc<RefCell<HashMap<String, gtk4::CheckButton>>>,
    /// How many ring lines each subsystem has, to prune dead ones.
    counts: Rc<RefCell<HashMap<String, usize>>>,
    active: Rc<RefCell<HashSet<String>>>,
    /// Whether the view is pinned to the tail (set by real scroll events).
    follow: Rc<Cell<bool>>,
}

fn build_log_tab() -> LogTab {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    root.set_margin_start(18);
    root.set_margin_end(18);
    root.set_margin_top(12);
    root.set_margin_bottom(12);

    let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let filter = gtk4::Entry::builder()
        .placeholder_text("regex filter (lower-case = case-insensitive)")
        .hexpand(true)
        .build();
    let no_hid = gtk4::CheckButton::with_label("hide HID");
    controls.append(&filter);
    controls.append(&no_hid);
    root.append(&controls);

    // Body: log on the left, the subsystem filter list on the right, split by a
    // draggable divider so the list's width is adjustable and never grows on its
    // own; long subsystem names scroll horizontally inside the fixed panel.
    let body = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    body.set_vexpand(true);
    body.set_wide_handle(true);

    let view = gtk4::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk4::WrapMode::None); // long lines scroll, not wrap
    // dmesg-style colour tags by log level.
    let buffer = view.buffer();
    for (name, color, bold) in [
        ("fault", "#ff3b30", true),
        ("error", "#ff5c57", false),
        ("info", "#9aa0a6", false),
        ("debug", "#6b7177", false),
    ] {
        if let Some(tag) = buffer.create_tag(Some(name), &[]) {
            tag.set_foreground(Some(color));
            if bold {
                tag.set_weight(700);
            }
        }
    }
    let scroll = gtk4::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .child(&view)
        .build();
    scroll.add_css_class("debug-panel");
    // Tail-follow driven by discrete events only (no position math): scrolling
    // up with the wheel/touchpad stops following; reaching the bottom edge or
    // clicking the button resumes it. The actual pin to the tail is deferred to
    // idle so it happens after layout.
    let follow = Rc::new(Cell::new(true));

    // A floating "jump to latest" button, shown only while scrolled up.
    let jump = gtk4::Button::with_label("↓ latest");
    jump.add_css_class("osd");
    jump.set_halign(gtk4::Align::End);
    jump.set_valign(gtk4::Align::End);
    jump.set_margin_end(16);
    jump.set_margin_bottom(16);
    jump.set_visible(false);

    let scroll_ctrl = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    {
        let follow = follow.clone();
        let jump = jump.clone();
        scroll_ctrl.connect_scroll(move |_, _dx, dy| {
            if dy < 0.0 {
                // scrolling up: stop following and reveal the jump button
                follow.set(false);
                jump.set_visible(true);
            }
            glib::Propagation::Proceed
        });
    }
    view.add_controller(scroll_ctrl);
    {
        let follow = follow.clone();
        let jump = jump.clone();
        scroll.connect_edge_reached(move |_, pos| {
            if pos == gtk4::PositionType::Bottom {
                follow.set(true);
                jump.set_visible(false);
            }
        });
    }
    {
        let follow = follow.clone();
        let view = view.clone();
        jump.connect_clicked(move |button| {
            follow.set(true);
            button.set_visible(false);
            scroll_to_tail(&view);
        });
    }

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.add_overlay(&jump);
    body.set_start_child(Some(&overlay));
    body.set_resize_start_child(true);

    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    panel.set_width_request(120);
    let heading = gtk4::Label::new(Some("subsystem filter"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    panel.append(&heading);
    let subsystems = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let list_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .propagate_natural_width(false)
        .child(&subsystems)
        .build();
    list_scroll.add_css_class("debug-panel");
    panel.append(&list_scroll);
    body.set_end_child(Some(&panel));
    body.set_resize_end_child(false);
    body.set_shrink_end_child(false);
    body.set_position(820); // initial split; the user can drag it

    root.append(&body);

    let tab = LogTab {
        root,
        view,
        filter,
        no_hid,
        subsystems,
        ring: Rc::new(RefCell::new(VecDeque::with_capacity(LOG_RING))),
        rows: Rc::new(RefCell::new(HashMap::new())),
        counts: Rc::new(RefCell::new(HashMap::new())),
        active: Rc::new(RefCell::new(HashSet::new())),
        follow,
    };
    tab.wire_filters();
    tab
}

impl LogTab {
    fn wire_filters(&self) {
        let this = self.clone();
        self.filter.connect_changed(move |_| this.rerender());
        let this = self.clone();
        self.no_hid.connect_toggled(move |_| this.rerender());
    }

    /// Does a line pass the regex entry, the HID toggle and the ticked subsystems?
    fn passes(&self, line: &LogLine, regex: &Option<regex::Regex>) -> bool {
        if self.no_hid.is_active() {
            let s = line.subsystem.to_lowercase();
            if s.contains("hid") || s.contains("inputdevice") {
                return false;
            }
        }
        let active = self.active.borrow();
        if !active.is_empty() && !active.contains(&line.subsystem) {
            return false;
        }
        if let Some(regex) = regex {
            if !regex.is_match(&line.text) {
                return false;
            }
        }
        true
    }

    fn current_regex(&self) -> Option<regex::Regex> {
        let pattern = self.filter.text();
        if pattern.is_empty() {
            return None;
        }
        let insensitive = pattern.chars().all(|c| !c.is_ascii_uppercase());
        regex::RegexBuilder::new(&pattern).case_insensitive(insensitive).build().ok()
    }

    fn push(&self, line: LogLine) {
        {
            let mut ring = self.ring.borrow_mut();
            if ring.len() == LOG_RING {
                if let Some(old) = ring.pop_front() {
                    self.release_subsystem(&old.subsystem);
                }
            }
            ring.push_back(line.clone());
        }
        self.retain_subsystem(&line.subsystem);
        let regex = self.current_regex();
        if self.passes(&line, &regex) {
            self.insert_line(&line);
            // While following, keep the buffer bounded and pinned to the tail.
            // Trimming the top only when following means a scrolled-up reader is
            // never shifted.
            if self.follow.get() {
                self.trim_buffer();
                scroll_to_tail(&self.view);
            }
        }
    }

    /// Drop the oldest lines so the buffer never exceeds the ring size.
    fn trim_buffer(&self) {
        let buffer = self.view.buffer();
        let excess = buffer.line_count() - LOG_RING as i32;
        if excess > 0 {
            let mut start = buffer.start_iter();
            if let Some(mut cut) = buffer.iter_at_line(excess) {
                buffer.delete(&mut start, &mut cut);
            }
        }
    }

    fn rerender(&self) {
        let regex = self.current_regex();
        self.view.buffer().set_text("");
        let ring = self.ring.borrow();
        for line in ring.iter() {
            if self.passes(line, &regex) {
                self.insert_line(line);
            }
        }
        if self.follow.get() {
            scroll_to_tail(&self.view);
        }
    }

    /// Append one line, coloured by its level (dmesg style).
    fn insert_line(&self, line: &LogLine) {
        let buffer = self.view.buffer();
        let start_offset = buffer.end_iter().offset();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, &line.text);
        if let Some(tag) = level_tag(&line.level) {
            let start = buffer.iter_at_offset(start_offset);
            let end = buffer.end_iter();
            buffer.apply_tag_by_name(tag, &start, &end);
        }
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, "\n");
    }

    /// Record that a ring line of this subsystem was added; add its row if new.
    fn retain_subsystem(&self, subsystem: &str) {
        if subsystem.is_empty() {
            return;
        }
        *self.counts.borrow_mut().entry(subsystem.to_owned()).or_insert(0) += 1;
        if !self.rows.borrow().contains_key(subsystem) {
            self.add_row(subsystem);
        }
    }

    /// Record that a ring line of this subsystem aged out; drop its row once the
    /// buffer holds none of it, unless the user has it ticked.
    fn release_subsystem(&self, subsystem: &str) {
        if subsystem.is_empty() {
            return;
        }
        let mut counts = self.counts.borrow_mut();
        if let Some(count) = counts.get_mut(subsystem) {
            *count = count.saturating_sub(1);
            if *count > 0 {
                return;
            }
            counts.remove(subsystem);
        }
        drop(counts);
        let ticked =
            self.rows.borrow().get(subsystem).map(|c| c.is_active()).unwrap_or(false);
        if ticked {
            return; // keep an explicitly selected subsystem on the list
        }
        if let Some(check) = self.rows.borrow_mut().remove(subsystem) {
            self.subsystems.remove(&check);
        }
        self.active.borrow_mut().remove(subsystem);
    }

    /// Add a checkbox row for a subsystem, inserted alphabetically.
    fn add_row(&self, subsystem: &str) {
        let check = gtk4::CheckButton::with_label(subsystem);
        let name = subsystem.to_owned();
        let this = self.clone();
        check.connect_toggled(move |button| {
            if button.is_active() {
                this.active.borrow_mut().insert(name.clone());
            } else {
                this.active.borrow_mut().remove(&name);
            }
            this.rerender();
        });
        // Insert alphabetically: after the last row whose label sorts before
        // this one (None prepends, i.e. this one sorts first).
        let mut anchor: Option<gtk4::Widget> = None;
        let mut sibling = self.subsystems.first_child();
        while let Some(widget) = sibling {
            let label = widget.downcast_ref::<gtk4::CheckButton>().and_then(|c| c.label());
            if label.map(|l| l.as_str() < subsystem).unwrap_or(false) {
                anchor = Some(widget.clone());
                sibling = widget.next_sibling();
            } else {
                break;
            }
        }
        self.subsystems.insert_child_after(&check, anchor.as_ref());
        self.rows.borrow_mut().insert(subsystem.to_owned(), check);
    }
}

// ---------------------------------------------------------------------------
// Style and resources (mirrors the other kait2en gtk apps)
// ---------------------------------------------------------------------------

fn register_embedded_resources() {
    gio::resources_register_include!("t2-debug.gresource")
        .expect("failed to register embedded resources");
}

fn kait2en_brand() -> gtk4::DrawingArea {
    let brand = gtk4::DrawingArea::new();
    brand.set_content_width(80);
    brand.set_content_height(21);
    brand.set_size_request(80, 21);
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png");
    if let Ok(dark) = pixbuf {
        let light = gtk4::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
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
    }
    brand
}

fn install_palette() {
    let provider = gtk4::CssProvider::new();
    let style = adw::StyleManager::default();
    provider.load_from_string(&palette_css(style.is_dark()));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    style.connect_dark_notify(move |manager| {
        provider.load_from_string(&palette_css(manager.is_dark()));
    });
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
        "window, .debug-root, headerbar {{
             background: {window_bg};
             color: alpha({window_fg}, 0.72);
         }}
         button, button label, entry, textview, textview text {{ color: alpha({window_fg}, 0.72); }}
         textview, textview text {{ font-family: 'JetBrains Mono'; }}
         .title-1, .title-2, .heading, windowtitle .title {{ color: {window_fg}; font-weight: 600; }}
         .dim-label, windowtitle .subtitle {{ color: alpha({window_fg}, 0.72); }}
         window.csd {{ border-top: 1px solid {rim}; }}
         window.csd.maximized, window.csd.tiled, window.csd.tiled-top, window.csd.tiled-left,
         window.csd.tiled-right, window.csd.tiled-bottom, window.csd.fullscreen {{ border-top-width: 0; }}
         .debug-panel {{
             background: {box_bg};
             color: alpha({window_fg}, 0.72);
             border: none;
             border-radius: 12px;
             box-shadow: inset 0 0 0 1px {line};
         }}
         .debug-panel text {{ background: {box_bg}; }}
         .overview-value {{ font-family: 'JetBrains Mono'; font-feature-settings: 'tnum'; }}
         .donate-link, .donate-link > label {{ color: alpha({window_fg}, 0.72); }}"
    )
}
