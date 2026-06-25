import net from 'net';
import fs from 'fs';
import type { ActiveWindow, ActiveWindowBackend } from './types';
import { EMPTY } from './types';

interface NiriWindow {
  id:         number;
  title:      string | null;
  app_id:     string | null;
  pid:        number | null;
  is_focused: boolean;
}

// Locate the niri IPC socket. $NIRI_SOCKET is the fast path; fall back to
// scanning the runtime dir (sudo strips the env var) for niri's
// `niri.<wayland-display>.<pid>.sock` naming. Also used by detect.ts as the
// "is niri running" probe.
export function findNiriSocket(): string | null {
  const sock = process.env.NIRI_SOCKET;
  if (sock) { try { fs.accessSync(sock); return sock; } catch { /**/ } }

  const runtime = process.env.XDG_RUNTIME_DIR ?? '/run/user/1000';
  try {
    const match = fs.readdirSync(runtime).find(f => /^niri\..*\.sock$/.test(f));
    if (match) return `${runtime}/${match}`;
  } catch { /**/ }
  return null;
}

function toActive(w: NiriWindow): ActiveWindow {
  return { class: w.app_id ?? '', title: w.title ?? '', pid: w.pid ?? 0 };
}

export const niri: ActiveWindowBackend = {
  name: 'niri',

  async start(push) {
    const sockPath = findNiriSocket();
    if (!sockPath) return null;
    let alive = true;

    // A single EventStream connection carries everything: niri replies
    // {"Ok":"Handled"}, dumps every window via WindowsChanged (each flagged
    // is_focused), then streams WindowOpenedOrChanged / WindowClosed /
    // WindowFocusChanged. Tracking the window map locally means focus changes
    // never need a follow-up query — title and pid are already in hand.
    const sock = net.createConnection(sockPath);
    sock.write('"EventStream"\n');

    const windows = new Map<number, NiriWindow>();
    let focusedId: number | null = null;
    let carry = '';

    const emit = () => {
      if (!alive) return;
      const w = focusedId != null ? windows.get(focusedId) : undefined;
      push(w ? toActive(w) : EMPTY);
    };

    sock.on('data', (chunk: Buffer) => {
      carry += chunk.toString('utf8');
      const lines = carry.split('\n');
      carry = lines.pop() ?? '';
      for (const line of lines) {
        if (!line.trim()) continue;
        let msg: Record<string, any>;
        try { msg = JSON.parse(line); } catch { continue; }

        if (msg.WindowsChanged) {
          windows.clear();
          focusedId = null;
          for (const w of msg.WindowsChanged.windows as NiriWindow[]) {
            windows.set(w.id, w);
            if (w.is_focused) focusedId = w.id;
          }
          emit();
        } else if (msg.WindowOpenedOrChanged) {
          const w = msg.WindowOpenedOrChanged.window as NiriWindow;
          windows.set(w.id, w);
          if (w.is_focused) focusedId = w.id;
          if (w.id === focusedId) emit(); // title/app_id of the focused window may have changed
        } else if (msg.WindowClosed) {
          const id = msg.WindowClosed.id as number;
          windows.delete(id);
          if (id === focusedId) { focusedId = null; emit(); }
        } else if (msg.WindowFocusChanged) {
          focusedId = (msg.WindowFocusChanged.id as number | null) ?? null;
          emit();
        }
      }
    });

    sock.on('error', () => { if (alive) push(EMPTY); });

    return () => { alive = false; sock.destroy(); };
  },
};
