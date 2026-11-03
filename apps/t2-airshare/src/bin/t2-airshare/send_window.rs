// SPDX-License-Identifier: GPL-3.0-or-later
//! Send window: looks for phones over Quick Share, and for Apple devices
//! over AirDrop through the t2-drop service when it is installed. A device
//! found over both shows up once, AirDrop preselected. Sends the chosen
//! files to the one the user picks and shows the PIN and the progress.
//! AirDrop only looks while the window has the focus, its search keeps AWDL
//! running on the Macs nearby, which slows down their Wi-Fi.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use gtk4 as gtk;
use tokio::task::JoinHandle;

use t2_airshare::discover::{Discovery, Peer};
use t2_airshare::endpoint::DeviceType;
use t2_airshare::send::{self, Progress, SendOutcome};

use crate::{airdrop, bluetooth};

/// The async work runs on a tokio runtime next to the GTK main loop.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

enum Event {
    Found(Peer),
    /// AirDrop receivers of the t2-drop service, by id
    AirDrop(HashMap<String, String>),
    AirDropFailed(String),
    /// Bluetooth is off, both protocols need it to wake the devices up
    BluetoothOff,
    BluetoothOn(Result<(), String>),
    DiscoveryFailed(String),
    Waiting(String),
    Sent(u64, u64),
    Done(Result<SendOutcome, String>),
}

struct ChannelProgress(async_channel::Sender<Event>);

impl Progress for ChannelProgress {
    fn waiting(&mut self, pin: &str) {
        let _ = self.0.try_send(Event::Waiting(pin.to_string()));
    }

    fn sent(&mut self, sent: u64, total: u64) {
        let _ = self.0.try_send(Event::Sent(sent, total));
    }
}

/// Quick Share discovery. It sends its BLE advertisement only when
/// Bluetooth is on at the start, so it runs again after switching it on.
fn spawn_quick_share(tx: async_channel::Sender<Event>) -> JoinHandle<()> {
    runtime().spawn(async move {
        let mut discovery = match Discovery::start().await {
            Ok(discovery) => discovery,
            Err(err) => {
                let _ = tx.send(Event::DiscoveryFailed(format!("{err:#}"))).await;
                return;
            }
        };
        loop {
            match discovery.next().await {
                Ok(peer) => {
                    if tx.send(Event::Found(peer)).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Event::DiscoveryFailed(format!("{err:#}"))).await;
                    return;
                }
            }
        }
    })
}

/// How a device is reached.
#[derive(Clone)]
enum Via {
    /// the receiver id of the t2-drop service
    AirDrop(String),
    QuickShare(Peer),
}

/// One row: a device found over AirDrop, Quick Share or both.
struct Target {
    row: adw::ActionRow,
    name: String,
    airdrop: Option<String>,
    quick_share: Option<Peer>,
    /// AirDrop or Quick Share, shown when the device offers both
    choice: gtk::DropDown,
}

impl Target {
    fn via(&self) -> Option<Via> {
        match (&self.airdrop, &self.quick_share) {
            (Some(id), Some(peer)) => Some(if self.choice.selected() == 0 {
                Via::AirDrop(id.clone())
            } else {
                Via::QuickShare(peer.clone())
            }),
            (Some(id), None) => Some(Via::AirDrop(id.clone())),
            (None, Some(peer)) => Some(Via::QuickShare(peer.clone())),
            (None, None) => None,
        }
    }

    fn refresh(&self) {
        let both = self.airdrop.is_some() && self.quick_share.is_some();
        self.choice.set_visible(both);
        self.row.set_subtitle(match (&self.airdrop, &self.quick_share) {
            (Some(_), Some(_)) => "AirDrop and Quick Share",
            (Some(_), None) => "AirDrop",
            _ => "Quick Share",
        });
    }
}

/// The parts of the window the event handlers and rows share.
struct Ui {
    status: adw::StatusPage,
    list: gtk::ListBox,
    progress: gtk::ProgressBar,
    /// the progress bar moves until the AirDrop receiver accepted
    pulsing: Rc<Cell<bool>>,
    tx: async_channel::Sender<Event>,
    files: Vec<PathBuf>,
    /// Quick Share discovery and the running send
    tasks: RefCell<Vec<JoinHandle<()>>>,
    /// the AirDrop browse, only while the window is active
    airdrop: RefCell<Option<JoinHandle<()>>>,
    /// false once a send started, no more looking after that
    looking: Cell<bool>,
    targets: RefCell<HashMap<String, Target>>,
}

impl Ui {
    fn stop_looking(&self) {
        self.looking.set(false);
        for task in self.tasks.borrow_mut().drain(..) {
            task.abort();
        }
        if let Some(task) = self.airdrop.borrow_mut().take() {
            task.abort();
        }
    }

    /// AirDrop looks for receivers only while the window is active. Its
    /// search pauses the Quick Share advertisements of the receiver
    /// service, a send window left open in the background must not keep
    /// this machine hidden from phones.
    fn set_active(&self, active: bool) {
        let mut task = self.airdrop.borrow_mut();
        if active && self.looking.get() {
            if task.is_none() {
                *task = Some(spawn_airdrop(self.tx.clone()));
            }
        } else if let Some(task) = task.take() {
            task.abort();
        }
    }

    /// The row for `key`, created on first use.
    fn target(self: &Rc<Self>, key: &str, name: &str, icon_name: &str) {
        if self.targets.borrow().contains_key(key) {
            return;
        }
        let row = adw::ActionRow::builder().title(name).activatable(true).build();
        row.add_prefix(&gtk::Image::from_icon_name(icon_name));
        let choice = gtk::DropDown::from_strings(&["AirDrop", "Quick Share"]);
        choice.set_valign(gtk::Align::Center);
        choice.set_visible(false);
        row.add_suffix(&choice);
        let ui = Rc::downgrade(self);
        let row_key = key.to_string();
        row.connect_activated(move |_| {
            let Some(ui) = ui.upgrade() else { return };
            let via = ui.targets.borrow().get(&row_key).and_then(|t| t.via().map(|v| (t.name.clone(), v)));
            if let Some((name, via)) = via {
                ui.send(&name, via);
            }
        });
        self.list.append(&row);
        self.targets.borrow_mut().insert(
            key.to_string(),
            Target { row, name: name.to_string(), airdrop: None, quick_share: None, choice },
        );
    }

    fn update(&self, key: &str, change: impl FnOnce(&mut Target)) {
        let mut targets = self.targets.borrow_mut();
        let Some(target) = targets.get_mut(key) else { return };
        change(target);
        if target.airdrop.is_none() && target.quick_share.is_none() {
            self.list.remove(&target.row);
            targets.remove(key);
        } else {
            target.refresh();
        }
        let any = !targets.is_empty();
        drop(targets);
        if any {
            self.status.set_title("Choose a device");
        }
    }

    fn send(&self, name: &str, via: Via) {
        self.stop_looking();
        self.list.set_sensitive(false);
        self.status.set_description(None);
        let tx = self.tx.clone();
        let files = self.files.clone();
        let task = match via {
            Via::AirDrop(id) => {
                self.status.set_title(&format!("Accept on {name}"));
                self.progress.set_visible(true);
                self.progress.set_fraction(0.0);
                // Moves while the receiver has not accepted yet
                self.pulsing.set(true);
                let bar = self.progress.clone();
                let pulsing = self.pulsing.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                    if pulsing.get() && bar.is_visible() {
                        bar.pulse();
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
                let progress_tx = tx.clone();
                runtime().spawn(async move {
                    let report = move |p| {
                        let _ = progress_tx.try_send(match p {
                            airdrop::SendState::Accepted => Event::Sent(0, 1),
                            airdrop::SendState::Sent(sent, total) => Event::Sent(sent, total),
                        });
                    };
                    let result = airdrop::send(&id, &files, &local_name(), report)
                        .await
                        .map(|()| SendOutcome::Sent)
                        .map_err(|err| format!("{err:#}"));
                    let _ = tx.send(Event::Done(result)).await;
                })
            }
            Via::QuickShare(peer) => {
                self.status.set_title(&format!("Connecting to {name}"));
                runtime().spawn(async move {
                    let mut progress = ChannelProgress(tx.clone());
                    let result = send::send(peer.addr, &local_name(), &files, &mut progress)
                        .await
                        .map_err(|err| format!("{err:#}"));
                    let _ = tx.send(Event::Done(result)).await;
                })
            }
        };
        self.tasks.borrow_mut().push(task);
    }
}

/// Rows are matched by name, the same device announces the same name over
/// both protocols. A hidden Quick Share device keeps a row of its own.
fn key_for(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The AirDrop browse of the t2-drop service, reporting to the window.
fn spawn_airdrop(tx: async_channel::Sender<Event>) -> JoinHandle<()> {
    runtime().spawn(async move {
        // The receiver service takes its Quick Share advertisements off the
        // air while this task lives, the controller sends the AirDrop one
        // more often then. Ending the task drops the connection, the
        // service resumes.
        let _pause = t2_airshare::bluetooth::pause_receiver_advertising().await;
        let report = |update| {
            let event = match update {
                airdrop::Update::Receivers(list) => Event::AirDrop(list),
                airdrop::Update::BluetoothOff => Event::BluetoothOff,
                airdrop::Update::Failed(err) => Event::AirDropFailed(err),
            };
            tx.try_send(event).is_ok()
        };
        if let Err(err) = airdrop::browse(report).await {
            log::info!("no AirDrop receivers: {err:#}");
        }
    })
}

pub fn open(app: &adw::Application, files: Vec<PathBuf>) {
    let files: Vec<PathBuf> = files.into_iter().filter(|f| f.is_file()).collect();
    if files.is_empty() {
        return;
    }

    let status = adw::StatusPage::builder()
        .icon_name("network-wireless-symbolic")
        .title("Looking for devices")
        .description("Quick Share and AirDrop devices")
        .vexpand(true)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    let progress = gtk::ProgressBar::builder()
        .margin_start(24)
        .margin_end(24)
        .margin_bottom(24)
        .visible(false)
        .build();
    let summary = gtk::Label::builder()
        .label(describe(&files))
        .wrap(true)
        .css_classes(["dim-label"])
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let banner = adw::Banner::builder()
        .title("Bluetooth is off. Phones and Apple devices only show up with Bluetooth on.")
        .button_label("Turn On")
        .revealed(false)
        .build();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&banner);
    body.append(&summary);
    body.append(&status);
    body.append(&list);
    body.append(&progress);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    let window = adw::Window::builder()
        .application(app)
        .title("Send with T2 AirShare")
        .default_width(420)
        .default_height(520)
        .content(&view)
        .build();

    let (tx, rx) = async_channel::unbounded::<Event>();
    let ui = Rc::new(Ui {
        status: status.clone(),
        list: list.clone(),
        progress: progress.clone(),
        pulsing: Rc::default(),
        tx: tx.clone(),
        files,
        tasks: RefCell::default(),
        airdrop: RefCell::default(),
        looking: Cell::new(true),
        targets: RefCell::default(),
    });
    ui.tasks.borrow_mut().push(spawn_quick_share(tx.clone()));
    {
        let tx = tx.clone();
        ui.tasks.borrow_mut().push(runtime().spawn(async move {
            if bluetooth::powered().await == Some(false) {
                let _ = tx.send(Event::BluetoothOff).await;
            }
        }));
    }
    {
        let tx = tx.clone();
        banner.connect_button_clicked(move |banner| {
            banner.set_button_label(None);
            let tx = tx.clone();
            runtime().spawn(async move {
                let result = bluetooth::power_on().await.map_err(|err| format!("{err:#}"));
                let _ = tx.send(Event::BluetoothOn(result)).await;
            });
        });
    }
    {
        let ui = Rc::downgrade(&ui);
        window.connect_is_active_notify(move |window| {
            if let Some(ui) = ui.upgrade() {
                ui.set_active(window.is_active());
            }
        });
    }
    {
        let ui = ui.clone();
        window.connect_close_request(move |_| {
            ui.stop_looking();
            glib::Propagation::Proceed
        });
    }

    let mut phones_seen: HashMap<String, String> = HashMap::new();
    let mut airdrop_keys: HashMap<String, String> = HashMap::new();
    let ui_events = ui.clone();
    glib::spawn_future_local(async move {
        let ui = ui_events;
        while let Ok(event) = rx.recv().await {
            match event {
                Event::BluetoothOff => banner.set_revealed(true),
                Event::BluetoothOn(Ok(())) => {
                    banner.set_revealed(false);
                    banner.set_button_label(Some("Turn On"));
                    if ui.looking.get() {
                        ui.tasks.borrow_mut().push(spawn_quick_share(tx.clone()));
                    }
                }
                Event::BluetoothOn(Err(err)) => {
                    banner.set_title(&format!("Bluetooth could not be switched on: {err}"));
                    banner.set_button_label(Some("Try Again"));
                }
                Event::AirDropFailed(err) => {
                    log::warn!("AirDrop: {err}");
                    if ui.targets.borrow().is_empty() {
                        status.set_description(Some(&format!("AirDrop is not available: {err}")));
                    }
                }
                Event::AirDrop(found) => {
                    let gone: Vec<(String, String)> = airdrop_keys
                        .iter()
                        .filter(|(id, _)| !found.contains_key(*id))
                        .map(|(id, key)| (id.clone(), key.clone()))
                        .collect();
                    for (id, key) in gone {
                        airdrop_keys.remove(&id);
                        ui.update(&key, |t| t.airdrop = None);
                    }
                    for (id, name) in found {
                        if airdrop_keys.contains_key(&id) {
                            continue;
                        }
                        let key = key_for(&name);
                        ui.target(&key, &name, "computer-symbolic");
                        ui.update(&key, |t| t.airdrop = Some(id.clone()));
                        airdrop_keys.insert(id, key);
                    }
                }
                Event::Found(peer) => {
                    // A second discovery after switching Bluetooth on finds
                    // the same phones again
                    if phones_seen.contains_key(&peer.endpoint_id) {
                        continue;
                    }
                    let key = match &peer.name {
                        Some(name) => key_for(name),
                        None => format!("hidden {}", peer.endpoint_id),
                    };
                    phones_seen.insert(peer.endpoint_id.clone(), key.clone());
                    ui.target(&key, &peer.label(), icon(peer.device_type));
                    ui.update(&key, |t| t.quick_share = Some(peer));
                }
                Event::DiscoveryFailed(err) => {
                    status.set_icon_name(Some("dialog-warning-symbolic"));
                    status.set_title("Cannot look for phones");
                    status.set_description(Some(&err));
                }
                Event::Waiting(pin) => {
                    status.set_title("Accept on the phone");
                    status.set_description(Some(&format!("PIN {pin}")));
                    progress.set_visible(true);
                    progress.set_fraction(0.0);
                }
                Event::Sent(sent, total) => {
                    ui.pulsing.set(false);
                    status.set_title("Sending");
                    progress.set_fraction(sent as f64 / total.max(1) as f64);
                }
                Event::Done(result) => {
                    ui.pulsing.set(false);
                    progress.set_visible(false);
                    match result {
                        Ok(SendOutcome::Sent) => {
                            status.set_icon_name(Some("object-select-symbolic"));
                            status.set_title("Sent");
                            status.set_description(None);
                        }
                        Ok(SendOutcome::Declined(_)) => {
                            status.set_icon_name(Some("action-unavailable-symbolic"));
                            status.set_title("Declined on the phone");
                            status.set_description(None);
                        }
                        Err(err) => {
                            status.set_icon_name(Some("dialog-warning-symbolic"));
                            status.set_title("Sending failed");
                            status.set_description(Some(&err));
                        }
                    }
                    list.set_visible(false);
                }
            }
        }
    });

    window.present();
}

fn describe(files: &[PathBuf]) -> String {
    let names: Vec<String> = files
        .iter()
        .take(3)
        .filter_map(|f| f.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    let mut text = names.join(", ");
    if files.len() > 3 {
        text.push_str(&format!(" and {} more", files.len() - 3));
    }
    text
}

fn icon(device_type: DeviceType) -> &'static str {
    match device_type {
        DeviceType::Phone => "phone-symbolic",
        DeviceType::Tablet => "tablet-symbolic",
        DeviceType::Laptop => "computer-symbolic",
        DeviceType::Unknown => "network-wireless-symbolic",
    }
}

fn local_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux".to_string())
}
