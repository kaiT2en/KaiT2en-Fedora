// SPDX-License-Identifier: GPL-3.0-or-later
//! systemd-logind: the sleep inhibitor that lets the service take AWDL
//! down before suspend, and the session state of the agent's user.

use anyhow::Result;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
    #[zbus(name = "GetUserByPID")]
    fn get_user_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[proxy(interface = "org.freedesktop.login1.User", default_service = "org.freedesktop.login1")]
trait User {
    #[zbus(property)]
    fn display(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[proxy(interface = "org.freedesktop.login1.Session", default_service = "org.freedesktop.login1")]
trait Session {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

/// A delay lock on sleep, sleep waits until it is dropped.
pub async fn inhibit_sleep(connection: &Connection) -> Result<OwnedFd> {
    let manager = ManagerProxy::new(connection).await?;
    Ok(manager.inhibit("sleep", "t2-drop", "Ending AirDrop before sleep", "delay").await?)
}

/// The graphical session of the user running `pid`. Desktop programs run
/// in scopes of the user manager, which belong to no session, so the
/// session comes from the user's Display property.
pub async fn session_of(connection: &Connection, pid: u32) -> Result<OwnedObjectPath> {
    let user = ManagerProxy::new(connection).await?.get_user_by_pid(pid).await?;
    let (_, session) = UserProxy::builder(connection).path(user)?.build().await?.display().await?;
    anyhow::ensure!(session.as_str() != "/", "the user has no graphical session");
    Ok(session)
}

/// Whether the session is in the foreground and unlocked, the user can
/// see and answer a request then.
pub async fn session_usable(connection: &Connection, session: &OwnedObjectPath) -> bool {
    let Ok(builder) = SessionProxy::builder(connection).path(session.clone()) else { return false };
    let Ok(proxy) = builder.cache_properties(zbus::proxy::CacheProperties::No).build().await else {
        return false;
    };
    proxy.active().await.unwrap_or(false) && !proxy.locked_hint().await.unwrap_or(true)
}
