import dbus, { Message } from 'dbus-next';
import type { ActiveWindowBackend } from './types';

// KDE Plasma Wayland via the touchbar_dynamicshortcuts KWin script.
//
// The KWin script hooks workspace.windowActivated and pushes focus changes to
// this backend over D-Bus (method calls on org.touchbar.DynamicShortcuts).
// No journalctl, no dynamic script injection — the user enables the script
// once in System Settings → Window Management → KWin Scripts.

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
          push({ class: klass, title, pid: 0 });
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
