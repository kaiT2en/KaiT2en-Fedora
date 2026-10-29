// SPDX-License-Identifier: GPL-3.0-or-later
//! Send window: looks for phones, and for Apple devices through the t2-drop
//! service when it is installed, sends the chosen files to the one the
//! user picks and shows the PIN and the progress.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use gtk4 as gtk;
use tokio::task::JoinHandle;

use t2_share::discover::{Discovery, Peer};
use t2_share::endpoint::DeviceType;
use t2_share::send::{self, Progress, SendOutcome};

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

pub fn open(app: &adw::Application, files: Vec<PathBuf>) {
    let files: Vec<PathBuf> = files.into_iter().filter(|f| f.is_file()).collect();
    if files.is_empty() {
        return;
    }

    let status = adw::StatusPage::builder()
        .icon_name("network-wireless-symbolic")
        .title("Looking for devices")
        .description(
            "Phones have to be in the same Wi-Fi, with Bluetooth on and Quick Share \
             visible to everyone. Macs, iPhones and iPads need AirDrop for everyone.",
        )
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
        .title("Send with T2 Share")
        .default_width(420)
        .default_height(520)
        .content(&view)
        .build();

    let (tx, rx) = async_channel::unbounded::<Event>();
    let tasks: Rc<RefCell<Vec<JoinHandle<()>>>> = Rc::default();
    tasks.borrow_mut().push(spawn_quick_share(tx.clone()));
    {
        let tx = tx.clone();
        tasks.borrow_mut().push(runtime().spawn(async move {
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
        }));
    }
    {
        let tx = tx.clone();
        tasks.borrow_mut().push(runtime().spawn(async move {
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
        let tasks = tasks.clone();
        window.connect_close_request(move |_| {
            for task in tasks.borrow_mut().drain(..) {
                task.abort();
            }
            glib::Propagation::Proceed
        });
    }

    let files = Rc::new(files);
    let mut airdrop_rows: HashMap<String, adw::ActionRow> = HashMap::new();
    let mut phones_seen: HashSet<String> = HashSet::new();
    glib::spawn_future_local(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                Event::BluetoothOff => banner.set_revealed(true),
                Event::BluetoothOn(Ok(())) => {
                    banner.set_revealed(false);
                    banner.set_button_label(Some("Turn On"));
                    tasks.borrow_mut().push(spawn_quick_share(tx.clone()));
                }
                Event::BluetoothOn(Err(err)) => {
                    banner.set_title(&format!("Bluetooth could not be switched on: {err}"));
                    banner.set_button_label(Some("Try Again"));
                }
                Event::AirDropFailed(err) => {
                    log::warn!("AirDrop: {err}");
                    if airdrop_rows.is_empty() {
                        status.set_description(Some(&format!("AirDrop is not available: {err}")));
                    }
                }
                Event::AirDrop(found) => {
                    airdrop_rows.retain(|id, row| {
                        let keep = found.contains_key(id);
                        if !keep {
                            list.remove(row);
                        }
                        keep
                    });
                    for (id, name) in found {
                        if airdrop_rows.contains_key(&id) {
                            continue;
                        }
                        let row = adw::ActionRow::builder()
                            .title(&name)
                            .subtitle("AirDrop")
                            .activatable(true)
                            .build();
                        row.add_prefix(&gtk::Image::from_icon_name("computer-symbolic"));
                        let tx = tx.clone();
                        let tasks = tasks.clone();
                        let files = files.clone();
                        let row_status = status.clone();
                        let row_list = list.clone();
                        let row_progress = progress.clone();
                        let row_id = id.clone();
                        row.connect_activated(move |_| {
                            for task in tasks.borrow_mut().drain(..) {
                                task.abort();
                            }
                            row_list.set_sensitive(false);
                            row_status.set_title(&format!("Accept on {name}"));
                            row_status.set_description(None);
                            row_progress.set_visible(true);
                            row_progress.pulse();
                            let tx = tx.clone();
                            let files = files.as_ref().clone();
                            let id = row_id.clone();
                            tasks.borrow_mut().push(runtime().spawn(async move {
                                let result = airdrop::send(&id, &files, &local_name())
                                    .await
                                    .map(|()| SendOutcome::Sent)
                                    .map_err(|err| format!("{err:#}"));
                                let _ = tx.send(Event::Done(result)).await;
                            }));
                        });
                        list.append(&row);
                        airdrop_rows.insert(id, row);
                    }
                    if !airdrop_rows.is_empty() {
                        status.set_title("Choose a device");
                    }
                }
                Event::Found(peer) => {
                    // A second discovery after switching Bluetooth on finds
                    // the same phones again
                    if !phones_seen.insert(peer.endpoint_id.clone()) {
                        continue;
                    }
                    let row = adw::ActionRow::builder()
                        .title(peer.label())
                        .subtitle(peer.addr.ip().to_string())
                        .activatable(true)
                        .build();
                    row.add_prefix(&gtk::Image::from_icon_name(icon(peer.device_type)));
                    let tx = tx.clone();
                    let tasks = tasks.clone();
                    let files = files.clone();
                    let row_status = status.clone();
                    let row_list = list.clone();
                    row.connect_activated(move |_| {
                        // Stop looking, the BLE advertisement ends with it.
                        for task in tasks.borrow_mut().drain(..) {
                            task.abort();
                        }
                        row_list.set_sensitive(false);
                        row_status.set_title(&format!("Connecting to {}", peer.label()));
                        row_status.set_description(None);
                        let tx = tx.clone();
                        let files = files.as_ref().clone();
                        let addr = peer.addr;
                        tasks.borrow_mut().push(runtime().spawn(async move {
                            let mut progress = ChannelProgress(tx.clone());
                            let result = send::send(addr, &local_name(), &files, &mut progress)
                                .await
                                .map_err(|err| format!("{err:#}"));
                            let _ = tx.send(Event::Done(result)).await;
                        }));
                    });
                    list.append(&row);
                    status.set_title("Choose a device");
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
                    status.set_title("Sending");
                    progress.set_fraction(sent as f64 / total.max(1) as f64);
                }
                Event::Done(result) => {
                    progress.set_visible(false);
                    match result {
                        Ok(SendOutcome::Sent) => {
                            status.set_icon_name(Some("emblem-ok-symbolic"));
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
