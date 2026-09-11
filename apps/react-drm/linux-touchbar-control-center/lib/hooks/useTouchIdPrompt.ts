import { useEffect, useState } from 'react';
import dbus, { MessageBus } from 'dbus-next';

// State broadcast by the T2 Touch ID bridge (apps/t2-touchid) on the system
// bus while fprintd is asking for a finger. 'idle' means no prompt is active.
export type TouchIdState = 'idle' | 'waiting' | 'scanning' | 'matched' | 'retry' | 'failed';

const NAME = 'org.kait2en.TouchId';
const PATH = '/org/kait2en/TouchId';
const IFACE = 'org.kait2en.TouchId';

// The bridge emits a bare signal without an object server, so introspection
// would find nothing. Hand the proxy the interface XML directly, the same way
// useMediaPlayers copes with players that answer Introspect() with an empty
// node.
const INTROSPECTION = `<node>
  <interface name="org.kait2en.TouchId">
    <signal name="Changed">
      <arg name="state" type="s"/>
    </signal>
  </interface>
</node>`;

const KNOWN: TouchIdState[] = ['idle', 'waiting', 'scanning', 'matched', 'retry', 'failed'];
function normalize(raw: string): TouchIdState {
  return (KNOWN as string[]).includes(raw) ? (raw as TouchIdState) : 'idle';
}

export function useTouchIdPrompt(): TouchIdState {
  const [state, setState] = useState<TouchIdState>('idle');

  useEffect(() => {
    let alive = true;
    let bus: MessageBus | null = null;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let iface: any = null;
    const onChanged = (raw: string) => { if (alive) setState(normalize(raw)); };

    (async () => {
      try {
        bus = dbus.systemBus();
        const obj = await bus.getProxyObject(NAME, PATH, INTROSPECTION);
        if (!alive) return;
        iface = obj.getInterface(IFACE);
        iface.on('Changed', onChanged);
      } catch {
        // No bridge or no system bus: the animation simply never plays.
        if (alive) setState('idle');
      }
    })();

    return () => {
      alive = false;
      if (iface) iface.removeListener('Changed', onChanged);
      if (bus) bus.disconnect();
    };
  }, []);

  return state;
}
