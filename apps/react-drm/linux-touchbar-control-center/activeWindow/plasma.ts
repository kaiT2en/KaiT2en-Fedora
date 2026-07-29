import dbus, { Message } from 'dbus-next';
import type { ActiveWindowBackend } from './types';
import { EMPTY } from './types';

const SVC   = 'org.touchbar.DynamicShortcuts';
const PATH  = '/org/touchbar/DynamicShortcuts';
const IFACE = 'org.touchbar.DynamicShortcuts';

const SCRIPT_NAME = 'touchbar_dynamicshortcuts';

async function reloadScript(bus: dbus.MessageBus) {
  try {
    await bus.call(new Message({
      destination: 'org.kde.KWin',
      path: '/Scripting',
      interface: 'org.kde.kwin.Scripting',
      member: 'unloadScript',
      signature: 's',
      body: [SCRIPT_NAME],
    }));
    await bus.call(new Message({
      destination: 'org.kde.KWin',
      path: '/Scripting',
      interface: 'org.kde.kwin.Scripting',
      member: 'start',
    }));
  } catch (e: any) {
    console.warn('[plasma] reloadScript:', e?.message ?? e);
  }
}

export const plasma: ActiveWindowBackend = {
  name: 'plasma (kwin-script)',

  async start(push) {
    let bus: dbus.MessageBus;
    try { bus = dbus.sessionBus(); } catch { return null; }

    try {
      const rc = await bus.requestName(SVC, 0);
      if (rc !== 1 && rc !== 4) { bus.disconnect(); return null; }

      bus.addMethodHandler((msg: Message) => {
        if (msg.path !== PATH || msg.interface !== IFACE) return false;

        if (msg.member === 'SetActiveWindow') {
          const klass = String(msg.body?.[0] ?? '');
          const title = String(msg.body?.[1] ?? '');
          const pid   = Number(msg.body?.[2]) || 0;
          console.warn('[plasma] active:', klass, title, pid);
          push(!klass && !title ? EMPTY : { class: klass, title, pid });
          bus.send(Message.newMethodReturn(msg, '', []));
          return true;
        }

        return false;
      });

      reloadScript(bus);

      return () => {
        bus.disconnect();
      };
    } catch {
      bus.disconnect();
      return null;
    }
  },
};
