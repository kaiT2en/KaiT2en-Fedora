import dbus, { Message } from 'dbus-next';
import type { ActiveWindowBackend } from './types';
import { EMPTY } from './types';

// KDE Plasma Wayland via the touchbar_dynamicshortcuts KWin script.
//
// The KWin script hooks workspace.windowActivated, tracks captionChanged on
// the active window, and pushes every focus/title change to this backend
// over D-Bus (method calls on org.touchbar.DynamicShortcuts).  On load,
// the script polls NameHasOwner (30x at 1s intervals) via
// org.freedesktop.DBus and emits the current active window once the
// org.touchbar.DynamicShortcuts name appears.
//
// No journalctl, no dynamic script injection — the user (or the KaiT2en
// installer) enables the script once in System Settings → Window
// Management → KWin Scripts.

const SVC    = 'org.touchbar.DynamicShortcuts';
const PATH   = '/org/touchbar/DynamicShortcuts';
const IFACE  = 'org.touchbar.DynamicShortcuts';

export const plasma: ActiveWindowBackend = {
  name: 'plasma (kwin-script)',

  async start(push) {
    let bus: dbus.MessageBus;
    try { bus = dbus.sessionBus(); } catch { return null; }

    try {
      const rc = await bus.requestName(SVC, 0);
      // rc: 1 = PRIMARY_OWNER, 2 = IN_QUEUE, 3 = EXISTS, 4 = ALREADY_OWNER
      if (rc !== 1 && rc !== 4) { bus.disconnect(); return null; }

      bus.addMethodHandler((msg: Message) => {
        if (msg.path !== PATH || msg.interface !== IFACE) return false;

        if (msg.member === 'SetActiveWindow') {
          const klass = String(msg.body?.[0] ?? '');
          const title = String(msg.body?.[1] ?? '');
          const pid   = Number(msg.body?.[2]) || 0;
          push(!klass && !title ? EMPTY : { class: klass, title, pid });
          bus.send(Message.newMethodReturn(msg, '', []));
          return true;
        }

        return false;
      });

      return () => {
        bus.disconnect();
      };
    } catch {
      bus.disconnect();
      return null;
    }
  },
};
