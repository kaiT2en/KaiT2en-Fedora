import fs from 'fs';
import dbus from 'dbus-next';
import { findHyprDir } from './hyprland';
import { findNiriSocket } from './niri';

// Session detection that works under sudo, where XDG_SESSION_TYPE,
// WAYLAND_DISPLAY etc. are usually stripped: sockets on disk are the source
// of truth, env vars are only a fast path. XWayland keeps an X11 socket
// alive on every Wayland desktop, so Wayland must be checked first —
// an X11 socket alone proves nothing.

export interface Session {
  type: 'wayland' | 'xorg' | 'unknown';
  /** Compositor (Wayland) or desktop (Xorg) when we can tell. */
  desktop: 'hyprland' | 'niri' | 'gnome' | 'plasma' | 'unknown';
}

// Candidate runtime dirs: the env one, the invoking user's when under sudo,
// then everything in /run/user as a last resort.
function runtimeDirs(): string[] {
  const dirs = new Set<string>();
  if (process.env.XDG_RUNTIME_DIR) dirs.add(process.env.XDG_RUNTIME_DIR);
  if (process.env.SUDO_UID) dirs.add(`/run/user/${process.env.SUDO_UID}`);
  try {
    for (const e of fs.readdirSync('/run/user')) dirs.add(`/run/user/${e}`);
  } catch { /**/ }
  return [...dirs];
}

function hasWaylandSocket(): boolean {
  if (process.env.WAYLAND_DISPLAY) return true;
  for (const dir of runtimeDirs()) {
    try {
      if (fs.readdirSync(dir).some(f => /^wayland-\d+$/.test(f))) return true;
    } catch { /**/ }
  }
  return false;
}

function hasX11Socket(): boolean {
  if (process.env.DISPLAY) return true;
  try { return fs.readdirSync('/tmp/.X11-unix').length > 0; } catch { return false; }
}

// gnome-shell owns this name on both Wayland and Xorg sessions.
async function gnomeShellRunning(): Promise<boolean> {
  let bus: dbus.MessageBus;
  try { bus = dbus.sessionBus(); } catch { return false; }
  try {
    const obj   = await bus.getProxyObject('org.freedesktop.DBus', '/org/freedesktop/DBus');
    const iface = obj.getInterface('org.freedesktop.DBus');
    return Boolean(await iface.NameHasOwner('org.gnome.Shell'));
  } catch {
    return false;
  } finally {
    bus.disconnect();
  }
}

// kwin_wayland/kwin_x11 owns this name on both Wayland and Xorg sessions.
async function kwinRunning(): Promise<boolean> {
  let bus: dbus.MessageBus;
  try { bus = dbus.sessionBus(); } catch { return false; }
  try {
    const obj   = await bus.getProxyObject('org.freedesktop.DBus', '/org/freedesktop/DBus');
    const iface = obj.getInterface('org.freedesktop.DBus');
    return Boolean(await iface.NameHasOwner('org.kde.KWin'));
  } catch {
    return false;
  } finally {
    bus.disconnect();
  }
}

async function detectDesktop(): Promise<Session['desktop']> {
  if (findHyprDir()) return 'hyprland';
  if (findNiriSocket()) return 'niri';
  const xdg = (process.env.XDG_CURRENT_DESKTOP ?? '').toLowerCase();
  if (xdg.includes('gnome') || await gnomeShellRunning()) return 'gnome';
  if (xdg.includes('kde') || xdg.includes('plasma') || await kwinRunning()) return 'plasma';
  return 'unknown';
}

export async function detectSession(): Promise<Session> {
  if (hasWaylandSocket()) return { type: 'wayland', desktop: await detectDesktop() };
  if (hasX11Socket())     return { type: 'xorg',    desktop: await detectDesktop() };
  return { type: 'unknown', desktop: 'unknown' };
}
