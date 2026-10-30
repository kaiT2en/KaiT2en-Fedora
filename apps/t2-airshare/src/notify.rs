// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop notifications over `org.freedesktop.Notifications`, the same on
//! GNOME, KDE and the other desktops.

use std::collections::HashMap;
use std::time::Duration;

use std::path::PathBuf;

use anyhow::Result;
use futures_util::StreamExt;
use log::{info, warn};
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, &Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.FileManager1",
    default_service = "org.freedesktop.FileManager1",
    default_path = "/org/freedesktop/FileManager1"
)]
trait FileManager1 {
    fn show_items(&self, uris: &[&str], startup_id: &str) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.portal.OpenURI",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait OpenUri {
    #[zbus(name = "OpenURI")]
    fn open_uri(
        &self,
        parent_window: &str,
        uri: &str,
        options: HashMap<&str, &Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

/// What a button on a finished transfer opens.
pub enum Launch {
    /// Opens the file manager on the folder with these files selected.
    ShowItems(Vec<PathBuf>),
    /// Opens a link with the default handler.
    OpenUri(String),
}

const APP_NAME: &str = "T2 AirShare";
const ICON: &str = "org.t2airshare.gtk";

pub struct Notifier {
    connection: Connection,
    proxy: NotificationsProxy<'static>,
}

impl Notifier {
    pub async fn new() -> Result<Self> {
        let connection = Connection::session().await?;
        let proxy = NotificationsProxy::new(&connection).await?;
        Ok(Self { connection, proxy })
    }

    /// Shows a notification with buttons and waits for one of them. Returns
    /// the key of the pressed button, None when the notification was
    /// dismissed or nobody answered within `wait`.
    pub async fn ask(
        &self,
        summary: &str,
        body: &str,
        actions: &[(&str, &str)],
        wait: Duration,
    ) -> Result<Option<String>> {
        let mut invoked = self.proxy.receive_action_invoked().await?;
        let mut closed = self.proxy.receive_notification_closed().await?;
        let id = self.show(summary, body, actions, true).await?;

        let answer = tokio::time::timeout(wait, async {
            loop {
                tokio::select! {
                    Some(signal) = invoked.next() => {
                        let args = signal.args()?;
                        if args.id == id {
                            return Ok::<_, zbus::Error>(Some(args.action_key));
                        }
                    }
                    Some(signal) = closed.next() => {
                        if signal.args()?.id == id {
                            return Ok(None);
                        }
                    }
                    else => return Ok(None),
                }
            }
        })
        .await;
        // Some servers keep a notification after a button press, it is
        // answered either way.
        let _ = self.proxy.close_notification(id).await;
        match answer {
            Ok(answer) => Ok(answer?),
            Err(_) => Ok(None),
        }
    }

    /// Shows a notification without waiting for it.
    pub async fn info(&self, summary: &str, body: &str) -> Result<()> {
        self.show(summary, body, &[], false).await?;
        Ok(())
    }

    /// Shows a notification with one button in the background. The button,
    /// or a click on the notification itself, runs `launch` within `wait`.
    pub fn offer(self, summary: &str, body: &str, label: &str, wait: Duration, launch: Launch) {
        let (summary, body, label) = (summary.to_string(), body.to_string(), label.to_string());
        tokio::spawn(async move {
            let actions = [("default", ""), ("open", label.as_str())];
            match self.ask(&summary, &body, &actions, wait).await {
                Ok(Some(pressed)) if pressed == "default" || pressed == "open" => {
                    info!("notification action {pressed:?}");
                    if let Err(err) = self.launch(&launch).await {
                        warn!("open failed: {err:#}");
                    }
                }
                Ok(_) => {}
                Err(err) => warn!("notification: {err:#}"),
            }
        });
    }

    async fn launch(&self, launch: &Launch) -> Result<()> {
        match launch {
            Launch::ShowItems(paths) => {
                let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
                let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
                info!("showing {uris:?}");
                FileManager1Proxy::new(&self.connection).await?.show_items(&uris, "").await?;
            }
            Launch::OpenUri(uri) => {
                info!("opening {uri}");
                OpenUriProxy::new(&self.connection)
                    .await?
                    .open_uri("", uri, HashMap::new())
                    .await?;
            }
        }
        Ok(())
    }

    async fn show(
        &self,
        summary: &str,
        body: &str,
        actions: &[(&str, &str)],
        urgent: bool,
    ) -> Result<u32> {
        let flat: Vec<&str> = actions.iter().flat_map(|(key, label)| [*key, *label]).collect();
        let urgency = Value::U8(if urgent { 2 } else { 1 });
        let hints = HashMap::from([("urgency", &urgency)]);
        Ok(self
            .proxy
            .notify(APP_NAME, 0, ICON, summary, body, &flat, hints, -1)
            .await?)
    }
}

/// `file://` URI with everything outside the unreserved set and `/`
/// percent encoded.
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_escapes_spaces_and_umlauts() {
        let uri = file_uri(std::path::Path::new("/home/a/Bild (1) ä.jpg"));
        assert_eq!(uri, "file:///home/a/Bild%20%281%29%20%C3%A4.jpg");
    }
}
