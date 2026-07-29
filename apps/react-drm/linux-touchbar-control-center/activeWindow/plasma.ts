import dbus, { Message } from 'dbus-next';
import type { ActiveWindowBackend } from './types';
import { EMPTY } from './types';

// KDE Plasma Wayland via the touchbar_dynamicshortcuts KWin script.
//
// The KWin script hooks workspace.windowActivated, tracks captionChanged on
// the active window, and pushes every focus/title change to this backend
// over D-Bus (method calls on org.touchbar.DynamicShortcuts).  On load,
// the script does a one-shot NameHasOwner via org.freedesktop.DBus and
// emits the current active window if the name exists.
//
// This backend triggers a KWin script reload on startup, which re-executes
// the script's one-shot handshake — ensuring the current window state is
// pushed to react-drm after any restart.

const SVC    = 'org.touchbar.DynamicShortcuts';
const PATH   = '/org/touchbar/DynamicShortcuts';
const IFACE  = 'org.touchbar.DynamicShortcuts';

const SCRIPT_NAME = 'touchbar_dynamicshortcuts';
const SCRIPT_PATH = `${process.env.HOME}/.local/share/kwin/scripts/${SCRIPT_NAME}/contents/code/main.js`;

async function reloadScript(bus: dbus.MessageBus) {
  await bus.call(new dbus.Message({
    destination: 'org.kde.KWin',
    path: '/Scripting',
    interface: 'org.kde.kwin.Scripting',
    member: 'unloadScript',
    signature: 's',
    body: [SCRIPT_NAME],
  }));
  await bus.call(new dbus.Message({
    destination: 'org.kde.KWin',
    path: '/Scripting',
    interface: 'org.kde.kwin.Scripting',
    member: 'loadScript',
    signature: 'ss',
    body: [SCRIPT_PATH, SCRIPT_NAME],
  }));
}

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

      reloadScript(bus).catch(() => {});

      return () => {
        bus.disconnect();
      };
    } catch {
      bus.disconnect();
      return null;
    }
  },
};
