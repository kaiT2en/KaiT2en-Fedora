import fs from 'fs';
import path from 'path';
import { spawn } from 'child_process';
import dbus from 'dbus-next';
import { usbReset, TOUCHBAR_DRM_DRIVERS, TOUCHBAR_USB_VENDOR_ID, TOUCHBAR_USB_PRODUCT_ID } from 'react-drm';
import { SLEEP } from '../config';

const TOUCHBAR_DRM_RE = new RegExp(`DRIVER=(${TOUCHBAR_DRM_DRIVERS.join('|')})`, 'i');

/**
 * In-app Touch Bar lifecycle: suspend/resume handling for every run mode
 * (manual `npm run dev` and react-drm.service alike).
 *
 * Suspend tears down the apple-bce bus (suspend-fix-t2's `rmmod -f
 * apple-bce`), the Touch Bar re-enumerates in config 1 (firmware fn strip)
 * and the app would sit on a dead DRM fd showing nothing — or worse, crash
 * the kernel if it still held the bar through the forced module removal.
 *
 * This watcher holds a logind *delay* inhibitor so quiescing (closing the DRM
 * fd, killing audio PCM holders) finishes BEFORE the module removal, and on
 * resume re-attaches the Touch Bar in-process (wait for re-enumeration,
 * config switch, USB reset when wedged) and reopens the display.
 */

// ── Suspend hooks ─────────────────────────────────────────────────────────────
// Components holding hardware resources that must be released before sleep
// (e.g. the piano's fluidsynth PipeWire stream — an active PCM during the
// apple-bce removal oopses the kernel) register here. Stored on globalThis so
// hot-reload's require-cache clearing doesn't orphan registrations.

export interface SuspendHooks {
  onSleep?: () => void;
  onResume?: () => void;
}

const HOOKS_KEY = '__reactDrmSuspendHooks';

function hooks(): Map<string, SuspendHooks> {
  const g = globalThis as Record<string, unknown>;
  if (!g[HOOKS_KEY]) g[HOOKS_KEY] = new Map<string, SuspendHooks>();
  return g[HOOKS_KEY] as Map<string, SuspendHooks>;
}

/**
 * Register hooks to run around system sleep. Keyed so a module re-evaluated
 * by hot-reload replaces its previous registration instead of stacking stale
 * closures. Returns an unregister function.
 */
export function registerSuspendHooks(key: string, h: SuspendHooks): () => void {
  hooks().set(key, h);
  return () => { if (hooks().get(key) === h) hooks().delete(key); };
}

// ── Touch Bar re-attach ───────────────────────────────────────────────────────
//
// Entirely in-process — no root helpers or extra systemd units needed for a
// manual run. Attaching = switching the 05ac:8302 device to config 2; the
// display interface that appears matches appletbdrm's modalias, so the kernel
// auto-loads and binds the driver. Two USB quirks handled here:
//  - after suspend the device re-enumerates from scratch (the t2 sleep hook
//    reloads apple_bce), so it may not exist yet when resume fires — wait;
//  - the T2 firmware puts the idle display interface to sleep, and EVERY
//    transfer to a sleeping device — including the SET_CONFIGURATION behind a
//    bConfigurationValue write — fails with ETIMEDOUT; a USBDEVFS_RESET wakes
//    it. But the reset is only safe on an UNCONFIGURED device: resetting a
//    configured one (config 1 = hid-appletb-kbd bound to the HID interface)
//    stalls the bce-vhci command queue — URB-cancel storm, keyboard/trackpad
//    on the same bus die, hard freeze (boot of 2026-06-12 20:03). So the
//    first attempt writes the config plainly; if that ETIMEDOUTs, the
//    kernel's recovery reset leaves the device unconfigured (drivers gone)
//    and the retry can reset-then-write safely.
// Write access to bConfigurationValue and the devnode comes from
// system/99-react-drm.rules (group video).

const USB_DEVICES = '/sys/bus/usb/devices';

function findTouchBarUsb(): string | null {
  try {
    for (const d of fs.readdirSync(USB_DEVICES)) {
      const dir = path.join(USB_DEVICES, d);
      try {
        if (fs.readFileSync(path.join(dir, 'idVendor'), 'utf8').trim() === TOUCHBAR_USB_VENDOR_ID
         && fs.readFileSync(path.join(dir, 'idProduct'), 'utf8').trim() === TOUCHBAR_USB_PRODUCT_ID) return dir;
      } catch { /* not a device dir */ }
    }
  } catch { /* no sysfs */ }
  return null;
}

function usbDevNode(sysDir: string): string {
  const busnum = Number(fs.readFileSync(path.join(sysDir, 'busnum'), 'utf8'));
  const devnum = Number(fs.readFileSync(path.join(sysDir, 'devnum'), 'utf8'));
  return `/dev/bus/usb/${String(busnum).padStart(3, '0')}/${String(devnum).padStart(3, '0')}`;
}

function readConfigValue(sysDir: string): string {
  try { return fs.readFileSync(path.join(sysDir, 'bConfigurationValue'), 'utf8').trim(); }
  catch { return ''; }
}

function appletbdrmCardNode(): string | null {
  try {
    const cards = fs.readdirSync('/sys/class/drm').filter(n => /^card\d+$/.test(n));
    for (const card of cards) {
      try {
        if (TOUCHBAR_DRM_RE.test(fs.readFileSync(`/sys/class/drm/${card}/device/uevent`, 'utf8')))
          return `/dev/dri/${card}`;
      } catch { /* card with no uevent */ }
    }
  } catch { /* no sysfs */ }
  return null;
}

function appletbdrmCardPresent(): boolean {
  return appletbdrmCardNode() !== null;
}

const sleep = (ms: number) => new Promise<void>(r => setTimeout(r, ms));

/**
 * Switch the Touch Bar to config 2 (appletbdrm auto-loads) and wait for the
 * DRM card to appear.
 */
export async function attachTouchBar(): Promise<void> {
  const deadline = Date.now() + SLEEP.cardWaitSecs * 1000;

  // After resume, apple-bce is reloaded by the resume half of the t2 sleep
  // unit and the whole bce-vhci bus enumerates from scratch — the 8302 device
  // may take several seconds to appear.
  let dev = findTouchBarUsb();
  while (!dev) {
    if (Date.now() > deadline) throw new Error('Touch Bar USB device (05ac:8302) did not enumerate');
    await sleep(500);
    dev = findTouchBarUsb();
  }

  while (readConfigValue(dev) !== '2') {
    // The sysfs device appears the instant the kernel registers it, but udev
    // applies the rule's GROUP/MODE to the devnode (and the chown on
    // bConfigurationValue) a beat later — EACCES right after (re-)enumeration
    // is that race, not a missing rule, so wait for access instead of failing.
    const node = usbDevNode(dev);
    const cfg = path.join(dev, 'bConfigurationValue');
    try {
      fs.accessSync(node, fs.constants.W_OK);
      fs.accessSync(cfg, fs.constants.W_OK);
    } catch {
      if (Date.now() > deadline) throw new Error(`no write access to ${node} — install system/99-react-drm.rules and re-trigger`);
      await sleep(250);
      continue;
    }
    try {
      // Reset only when no config is active (fresh wedge or a failed write's
      // recovery reset) — never on a configured device with bound drivers.
      // A configured-but-asleep device gets there too: its first write fails,
      // the kernel's recovery reset deconfigures it, and the next pass resets.
      if (readConfigValue(dev) === '') usbReset(node);
      fs.writeFileSync(cfg, '0');
      fs.writeFileSync(cfg, '2');
    } catch (e) {
      console.warn('[suspend] attach attempt failed, retrying:', e instanceof Error ? e.message : e);
    }
    if (readConfigValue(dev) === '2') break;
    if (Date.now() > deadline) throw new Error('could not switch Touch Bar to config 2');
    await sleep(5000);
    // A failed config write makes the kernel reset the device; devnum (and
    // even the sysfs dir) can change — re-resolve before the next try.
    dev = findTouchBarUsb() ?? dev;
  }

  let card = appletbdrmCardNode();
  let nextReprobe = Date.now() + 2000;
  while (!card) {
    if (Date.now() > deadline) throw new Error('appletbdrm card did not appear');

    // The USB device can reach config 2 before the T2 suspend service has
    // finished restoring its modules. In that window appletbdrm may probe
    // once, fail with ETIMEDOUT, and never be probed again even though config
    // 2 remains active. Re-apply the configuration until a DRM card appears.
    if (Date.now() >= nextReprobe) {
      dev = findTouchBarUsb() ?? dev;
      const node = usbDevNode(dev);
      let cfg = path.join(dev, 'bConfigurationValue');
      try {
        fs.accessSync(node, fs.constants.W_OK);
        fs.accessSync(cfg, fs.constants.W_OK);
        if (readConfigValue(dev) === '2')
          console.warn('[suspend] config 2 active but no DRM card — reprobe');
        if (readConfigValue(dev) === '') usbReset(node);
        fs.writeFileSync(cfg, '0');
        await sleep(500);
        dev = findTouchBarUsb() ?? dev;
        cfg = path.join(dev, 'bConfigurationValue');
        fs.writeFileSync(cfg, '2');
      } catch (e) {
        console.warn('[suspend] reprobe attempt failed, retrying:', e instanceof Error ? e.message : e);
      }
      nextReprobe = Date.now() + 2000;
    }

    await sleep(250);
    card = appletbdrmCardNode();
  }

  // The /dev/dri node shows up before udev finishes applying its permission
  // rules to it — opening in that window fails with EACCES. Wait until the
  // node is actually openable, not just present in sysfs.
  for (;;) {
    try { fs.accessSync(card, fs.constants.R_OK | fs.constants.W_OK); break; }
    catch {
      if (Date.now() > deadline) throw new Error(`${card} present but not accessible — udev rules applied?`);
      await sleep(250);
    }
  }
}

/**
 * Startup attach: nothing else switches the Touch Bar to config 2, and
 * DrmDisplay would fall back to the iGPU card. No-op when the appletbdrm
 * card is already there (e.g. a restart right after a previous run).
 */
export async function ensureTouchBarAttached(): Promise<void> {
  if (appletbdrmCardPresent()) return;
  if (!findTouchBarUsb()) return; // no Touch Bar hardware — nothing to attach
  console.log('[suspend] no appletbdrm card — attaching Touch Bar');
  await attachTouchBar();
}

// ── logind sleep watcher ──────────────────────────────────────────────────────

export interface SleepCallbacks {
  /** Quiesce: must finish within logind's inhibitor delay (default 5 s). */
  onSleep: () => void | Promise<void>;
  /** Device is back in config 1; re-attach and reopen here. */
  onResume: () => void | Promise<void>;
}

export async function watchSleep(cb: SleepCallbacks): Promise<void> {
  const bus = dbus.systemBus();
  const obj = await bus.getProxyObject('org.freedesktop.login1', '/org/freedesktop/login1');
  const manager = obj.getInterface('org.freedesktop.login1.Manager');

  // The delay lock is held by a systemd-inhibit child instead of a D-Bus
  // Inhibit() fd: dbus-next's unix-fd passing needs the abandoned usocket
  // native module (broken on current Node). The child blocks reading its
  // stdin pipe, so the lock is released when we close the pipe — or
  // automatically when this process dies, however it dies.
  let holder: ReturnType<typeof spawn> | null = null;

  function takeLock(): void {
    if (holder) return;
    holder = spawn('systemd-inhibit', [
      '--what=sleep', '--who=react-drm', '--mode=delay',
      '--why=Release Touch Bar DRM fd and audio before apple-bce teardown',
      '/bin/sh', '-c', 'read _ || true',
    ], { stdio: ['pipe', 'ignore', 'inherit'] });
    holder.on('error', e => console.warn('[suspend] systemd-inhibit failed:', e.message));
    holder.on('exit', () => { holder = null; });
  }

  function releaseLock(): void {
    if (!holder) return;
    holder.stdin?.end(); // EOF unblocks the read — child exits, lock drops
    holder = null;
  }

  takeLock();

  manager.on('PrepareForSleep', (sleeping: boolean) => {
    void (async () => {
      if (sleeping) {
        console.log('[suspend] system going to sleep — quiescing');
        for (const h of hooks().values()) { try { h.onSleep?.(); } catch { /* hook error must not block sleep */ } }
        try { await cb.onSleep(); } catch (e) {
          console.error('[suspend] quiesce failed:', e instanceof Error ? e.message : e);
        } finally {
          releaseLock(); // let the suspend proceed
        }
      } else {
        console.log('[suspend] system resumed — re-attaching Touch Bar');
        takeLock();
        try {
          await cb.onResume();
          for (const h of hooks().values()) { try { h.onResume?.(); } catch { /* keep going */ } }
          console.log('[suspend] resume complete');
        } catch (e) {
          console.error('[suspend] resume failed:', e instanceof Error ? e.message : e);
        }
      }
    })();
  });

  console.log('[suspend] logind sleep watcher active (delay inhibitor held)');
}
