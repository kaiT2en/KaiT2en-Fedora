import fs from 'fs';
import os from 'os';
import { Worker } from 'worker_threads';
import React from 'react';
import { reconciler } from './reconciler';
import { setRepaint } from './invalidate';
import { serializeScene, frameSignature, damageRects, toBinaryBuffer } from '../scene/serialize';
import type { DrawCommand } from '../scene/serialize';
import { computeLayoutYoga, loadYogaEngine, yogaReady } from '../scene/layout-yoga';
import { TouchRegistry, TouchRegistryContext } from '../input/touch-registry';
import { LayoutContext } from '../scene/layout-context';
import { DisplaySizeContext, NativeDrawContext } from '../scene/display-context';
import type { NativeDraw } from '../scene/display-context';
import { TouchReader, getTouchDevicePath } from '../native/input';
import { KeyboardReader, findKeyboardDevices, findPointerDevices, findLidDevice } from '../native/keyboard';
import type { SceneNode, RootContainer } from '../scene/types';
import type { LayoutBox } from '../scene/layout';
import type { DrmDisplay } from '../native/binding';
import { TOUCHBAR_BACKLIGHT_NAMES, DISPLAY_BACKLIGHT_NAMES } from '../native/hardware';

export interface RenderOptions {
  /**
   * Dim to half brightness after this many idle seconds (step 1).
   * 0 = screen-saver disabled (default).
   */
  dimSecs?: number;
  /**
   * Additional seconds after dim before the screen goes fully off (step 2).
   * Defaults to the same value as dimSecs.
   */
  offSecs?: number;
  /** @deprecated  Use dimSecs instead. */
  screenSaverSecs?: number;
  /**
   * Sweep the entire frame ±11 px horizontally (sinusoidal Y ±2 px) to spread
   * AMOLED pixel wear.  Value = seconds for one direction sweep (default: 300 s).
   * 0 = disabled.
   */
  pixelShiftSecs?: number;
  /**
   * If provided, the renderer subscribes to this KeyboardReader for idle
   * activity detection instead of opening the keyboard device a second time.
   */
  keyboardReader?: KeyboardReader;
  /**
   * Scale Touch Bar brightness to match the main display brightness.
   * Default: false.
   */
  adaptiveBrightness?: boolean;
  /**
   * Fixed brightness level when adaptiveBrightness is false.
   * Hardware supports exactly 3 levels: 0 (off), 1 (half), 2 (full).
   * Default: 2 (full brightness).
   */
  activeBrightness?: 0 | 1 | 2;
  /**
   * Max framebuffer flushes per second. Bursts (e.g. 60fps spring frames)
   * coalesce into a single trailing flush, keeping the appletbdrm USB
   * request/response handshake within its timeout window. Default: 30.
   */
  flushFps?: number;

  partialFlush?: boolean;
}

export interface RenderResult {
  /** Unmount the React tree. */
  unmount: () => void;

  /**
   * Push a new React element into the existing renderer without reopening the
   * display.  Used by hot-reload to swap in updated components.
   */
  update: (element: React.ReactNode) => void;

  /**
   * Legacy one-shot tap hit-test (fires on touch-down).
   * Use touchStart / touchMove / touchEnd for swipe support.
   */
  hitTest: (x: number, y: number) => void;

  /** Call when a finger touches down. Fires tap handlers + starts swipe tracking. */
  touchStart: (x: number, y: number) => void;

  /** Call when the finger moves. */
  touchMove: (x: number, y: number) => void;

  /** Call when the finger lifts. Fires swipe handlers if gesture qualifies. */
  touchEnd: (x: number, y: number) => void;

  /**
   * Signal user activity — resets the idle timer and wakes the display if it
   * was dimmed or off.  Call this from keyboard / custom input handlers.
   */
  wake: () => void;

  /**
   * Quiesce before system sleep: stop rendering and idle watchers and close
   * the DRM fd (the device disappears during suspend anyway). The React tree
   * stays mounted; commits keep updating the scene off-screen.
   */
  suspend: () => void;

  /**
   * Undo suspend() after the device is back (attached + driver bound):
   * reopens the display, restarts the watchers and repaints the latest scene.
   */
  resume: () => void;
}

// ── Input device helpers ──────────────────────────────────────────────────────
// Uses net.Socket (epoll-backed) with O_NONBLOCK instead of createReadStream
// (which uses the thread pool and starves when many devices are opened at once).

// Spawns a Worker thread that does a blocking fs.readSync loop on the character
// device.  Worker threads have their own OS thread — blocking there doesn't
// stall the libuv thread pool, so any number of devices can be monitored at once.
// Each read returns exactly one 24-byte input_event; the worker posts it as a
// Buffer to the main thread, which receives it via the normal event loop.
function openEvdevStream(
  dev: string,
  onData: (chunk: Buffer) => void,
  onError: (err: Error) => void,
  onOpen?: () => void,
): () => void {
  let active = true;

  let workerFd: number | null = null;

  const worker = new Worker(`
    const { workerData, parentPort } = require('worker_threads');
    const fs = require('fs');
    let fd;
    try { fd = fs.openSync(workerData.dev, 'r'); }
    catch (e) { parentPort.postMessage({ error: e.message }); process.exit(0); }
    parentPort.postMessage({ fd });
    const buf = Buffer.alloc(24);
    for (;;) {
      let n;
      try { n = fs.readSync(fd, buf, 0, 24, null); }
      catch { break; }
      if (n !== 24) break;
      const ab = new ArrayBuffer(24);
      new Uint8Array(ab).set(buf);
      parentPort.postMessage(ab, [ab]);
    }
    try { fs.closeSync(fd); } catch {}
  `, { eval: true, workerData: { dev } });

  worker.on('message', (msg: ArrayBuffer | { fd: number } | { error: string }) => {
    if (!active) return;
    if (msg instanceof ArrayBuffer) { onData(Buffer.from(msg)); return; }
    if (typeof msg === 'object' && 'fd' in msg) { workerFd = (msg as { fd: number }).fd; onOpen?.(); return; }
    onError(new Error((msg as { error: string }).error));
  });
  worker.on('error', (err: Error) => { if (active) onError(err); });
  worker.on('exit', (code) => {
    if (active && code !== 0) onError(new Error(`evdev worker for ${dev} exited: ${code}`));
  });

  return () => {
    active = false;
    // Close the fd from the main thread — workers share the process fd table,
    // so this unblocks the readSync with EBADF and lets the worker exit cleanly.
    // Without this, process.exit() hangs waiting for the blocked worker threads.
    if (workerFd !== null) { try { fs.closeSync(workerFd); } catch {} workerFd = null; }
    worker.terminate();
  };
}

function parseEvdev(
  carry: { buf: Buffer },
  chunk: Buffer,
  onEvent: (type: number, code: number, value: number) => void,
): void {
  const buf = carry.buf.length ? Buffer.concat([carry.buf, chunk]) : chunk;
  const count = Math.floor(buf.length / 24);
  for (let i = 0; i < count; i++) {
    const off = i * 24;
    onEvent(buf.readUInt16LE(off + 16), buf.readUInt16LE(off + 18), buf.readInt32LE(off + 20));
  }
  carry.buf = count * 24 < buf.length ? buf.slice(count * 24) : Buffer.alloc(0);
}

// ── evdev watcher (shared) ────────────────────────────────────────────────────
// watchPointer / watchKeyboard / watchLid were near-identical: enumerate evdev
// nodes, stream each through a worker, combine the stops. They now share this
// helper, which adds reopen retries so a node that hasn't re-enumerated yet after
// an apple-bce resume gets picked up once it appears:
//   • enumeration — if the device list is empty, retry up to INPUT_RETRY_MAX
//                   times on the INPUT_RETRY_MS cadence, then log and give up.
//   • per-device  — if a stream fails to open (or later dies), reopen that one
//                   device, bounded the same way; the budget resets on a clean
//                   open so a device that works then drops gets a fresh quota.
const INPUT_RETRY_MS  = 3000; // matches the keyboard's RECONNECT_DELAY_MS
const INPUT_RETRY_MAX = 3;    // → ~9s of reopen attempts before giving up

function watchEvdev(
  label: string,
  enumerate: () => string[],
  onEvent: (type: number, code: number, value: number) => void,
): () => void {
  let stopped = false;
  let enumRetries = 0;
  let enumTimer: ReturnType<typeof setTimeout> | null = null;
  let hotplugTimer: ReturnType<typeof setTimeout> | null = null;
  let dirWatcher: fs.FSWatcher | null = null;
  const deviceStops = new Map<string, () => void>();

  // Stream one device, reopening (bounded) if it fails to open or later dies.
  function openDevice(dev: string): void {
    let attempts = 0;
    let streamStop: (() => void) | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    function start(): void {
      if (stopped) return;
      const carry = { buf: Buffer.alloc(0) };
      streamStop = openEvdevStream(
        dev,
        chunk => parseEvdev(carry, chunk, onEvent),
        err => {
          if (stopped) return;
          console.warn(`[react-drm] ${label}: ${dev}: ${err.message}`);
          if (streamStop) { streamStop(); streamStop = null; }
          if (attempts < INPUT_RETRY_MAX) {
            attempts++;
            retryTimer = setTimeout(() => { retryTimer = null; start(); }, INPUT_RETRY_MS);
          } else {
            console.warn(`[react-drm] ${label}: ${dev}: giving up after ${attempts} reopen attempts`);
          }
        },
        () => { attempts = 0; }, // clean open — reset the reopen budget
      );
    }

    start();
    deviceStops.set(dev, () => {
      if (retryTimer) { clearTimeout(retryTimer); retryTimer = null; }
      if (streamStop) { streamStop(); streamStop = null; }
    });
  }

  // Open devices that have appeared and drop ones that vanished. Runs on initial
  // bring-up and on every /dev/input change, so a mouse plugged in later, dock
  // devices that enumerate after startup, and post-resume renumbering are all
  // picked up — not just whatever existed the instant the watcher started.
  function reconcile(initial: boolean): void {
    if (stopped) return;
    let devices: string[] = [];
    try { devices = enumerate(); } catch { /**/ }

    if (devices.length === 0) {
      // Nothing yet — retry on the bring-up cadence, but only during the initial
      // window; after that the /dev/input watcher drives re-checks on hotplug.
      if (initial && enumRetries < INPUT_RETRY_MAX) {
        enumRetries++;
        enumTimer = setTimeout(() => { enumTimer = null; reconcile(true); }, INPUT_RETRY_MS);
      } else if (initial) {
        console.warn(`[react-drm] ${label}: no devices found, giving up after ${enumRetries} retries`);
      }
      return;
    }
    enumRetries = 0;

    const want = new Set(devices);
    let changed = false;
    for (const dev of devices) {
      if (!deviceStops.has(dev)) { openDevice(dev); changed = true; }
    }
    for (const dev of [...deviceStops.keys()]) {
      if (!want.has(dev)) { deviceStops.get(dev)!(); deviceStops.delete(dev); changed = true; }
    }
    if (changed) {
      console.log(`[react-drm] ${label}: monitoring ${[...deviceStops.keys()].join(', ') || '(none)'}`);
    }
  }

  // Re-enumerate when /dev/input gains or loses nodes. Debounced — a single
  // hotplug fires several rename events. Best-effort: if the watch can't be set
  // up we still have the initial enumeration (just no late-add detection).
  try {
    dirWatcher = fs.watch('/dev/input', () => {
      if (stopped) return;
      if (hotplugTimer) clearTimeout(hotplugTimer);
      hotplugTimer = setTimeout(() => { hotplugTimer = null; reconcile(false); }, 300);
    });
  } catch (e) {
    console.warn(`[react-drm] ${label}: hotplug watch unavailable: ${(e as NodeJS.ErrnoException).message}`);
  }

  reconcile(true);
  return () => {
    stopped = true;
    if (enumTimer) { clearTimeout(enumTimer); enumTimer = null; }
    if (hotplugTimer) { clearTimeout(hotplugTimer); hotplugTimer = null; }
    if (dirWatcher) { dirWatcher.close(); dirWatcher = null; }
    deviceStops.forEach(stop => stop());
    deviceStops.clear();
  };
}

// Pointer: any non-SYN event from a touchpad/touchscreen/mouse = user activity.
// The Touch Bar surface is excluded — TouchReader already opens it and wakes on
// touch, so watching it here would be a redundant second open + wake. Resolved
// fresh each enumeration so it tracks event-number changes across re-enumeration.
function watchPointer(onActivity: () => void): () => void {
  return watchEvdev('watchPointer', () => {
    const touch = getTouchDevicePath();
    const devices = findPointerDevices();
    return touch ? devices.filter(d => d !== touch) : devices;
  }, (type) => {
    if (type !== 0) onActivity();
  });
}

// Keyboard: EV_KEY key-down = user activity. Only used when the caller does not
// supply its own KeyboardReader (which has its own reconnect path).
function watchKeyboard(onActivity: () => void): () => void {
  return watchEvdev('watchKeyboard', findKeyboardDevices, (type, _code, value) => {
    if (type === 1 && value === 1) onActivity();
  });
}

// Lid: EV_SW + SW_LID — value 1 = closed, 0 = open. Single-device, wrapped to the
// array contract the helper expects.
function watchLid(onLid: (closed: boolean) => void): () => void {
  return watchEvdev(
    'watchLid',
    () => { try { const d = findLidDevice(); return d ? [d] : []; } catch { return []; } },
    (type, code, value) => { if (type === 5 && code === 0) onLid(value === 1); },
  );
}

// ── Backlight control ─────────────────────────────────────────────────────────
// Controls the Touch Bar backlight via sysfs so the "off" state actually turns
// the panel off and wake from off reliably restores it.

const TB_BACKLIGHT_NAMES  = TOUCHBAR_BACKLIGHT_NAMES;
const DISP_BACKLIGHT_NAMES = DISPLAY_BACKLIGHT_NAMES;

// After resume the appletb_backlight HID interface re-binds late; re-apply and
// verify the level on this cadence until the panel confirms it (or the window
// expires — past that, the self-healing write() fixes it on the next wake/dim).
const SETTLE_INTERVAL_MS = 1000;
const SETTLE_WINDOW_MS   = 20_000;

function findBacklightDir(candidates: readonly string[]): string | null {
  try {
    const base = '/sys/class/backlight';
    const name = fs.readdirSync(base).find(n => candidates.some(c => n.includes(c)));
    return name ? `${base}/${name}` : null;
  } catch { return null; }
}

function readInt(path: string): number {
  try { return parseInt(fs.readFileSync(path, 'utf8').trim(), 10) || 0; } catch { return 0; }
}

class Backlight {
  private tbDir:    string | null;
  private tbFile:   string | null;
  private tbMax:    number;
  private dispFile: string | null;
  private dispMax:  number;
  private lidClosed = false;
  private activeHwLevel = 2; // raw level currently written while active
  private settleTimer: ReturnType<typeof setInterval> | null = null;

  constructor() {
    this.tbDir   = findBacklightDir(TB_BACKLIGHT_NAMES);
    this.tbFile  = this.tbDir ? `${this.tbDir}/brightness` : null;
    this.tbMax   = this.tbDir ? readInt(`${this.tbDir}/max_brightness`) : 0;
    const dispDir = findBacklightDir(DISP_BACKLIGHT_NAMES);
    this.dispFile = dispDir ? `${dispDir}/brightness` : null;
    this.dispMax  = dispDir ? readInt(`${dispDir}/max_brightness`) : 0;
  }

  // Re-resolve the Touch Bar backlight node. After S3 resume the
  // appletb_backlight HID interface re-binds a beat after the appletbdrm card
  // attachTouchBar() waits for, so the cached path can be null or stale.
  private resolveTb(): void {
    this.tbDir  = findBacklightDir(TB_BACKLIGHT_NAMES);
    this.tbFile = this.tbDir ? `${this.tbDir}/brightness` : null;
    this.tbMax  = this.tbDir ? readInt(`${this.tbDir}/max_brightness`) : 0;
  }

  private write(value: number): void {
    // Self-heal: if the node is absent (not yet re-enumerated after resume) try
    // to re-resolve it before giving up, so writes start landing the instant the
    // HID backlight interface re-appears instead of no-op'ing until next resume.
    if (!this.tbFile || !fs.existsSync(this.tbFile)) this.resolveTb();
    if (!this.tbFile) return;
    try { fs.writeFileSync(this.tbFile, String(Math.round(value))); } catch (e) {
      this.tbFile = null; // drop the stale path so the next write re-resolves
      console.warn('[react-drm] backlight write failed (need root?):', (e as NodeJS.ErrnoException).code);
    }
  }

  // Scale display brightness to Touch Bar brightness (sqrt for perceptual linearity)
  private adaptiveLevel(): number {
    if (!this.dispFile || !this.dispMax || !this.tbMax) return this.tbMax;
    const normalized = readInt(this.dispFile) / this.dispMax;
    return Math.min(this.tbMax, Math.round(Math.sqrt(normalized) * this.tbMax) + 1);
  }

  setLid(closed: boolean): void {
    this.lidClosed = closed;
    if (closed) this.write(0);
  }

  // level: 0 | 1 | 2 — raw hardware level (3 states only)
  on(adaptive: boolean, level: 0 | 1 | 2 = 2): void {
    if (this.lidClosed) return;
    const target = adaptive ? this.adaptiveLevel() : level;
    this.activeHwLevel = Math.max(1, Math.min(this.tbMax || 2, target));
    this.write(this.activeHwLevel);
  }

  /**
   * Dim via hardware LED to level 1. Returns false (no-op) when activeBrightness
   * is already at or below the dim level so the user's chosen level is respected.
   */
  dim(): boolean {
    if (this.lidClosed) return false;
    const dimLevel = Math.max(1, Math.round((this.tbMax || 2) * 0.2)); // = 1 for tbMax=2
    if (this.activeHwLevel <= dimLevel) return false; // already at or below dim level
    this.write(dimLevel);
    return true;
  }

  off(): void {
    this.activeHwLevel = 0;
    this.write(0);
  }

  /**
   * Re-resolve the backlight sysfs paths after the device re-enumerated
   * (e.g. after S3 suspend/resume).  The DRM card re-uses DrmDisplay.reopen();
   * the backlight needs the same treatment so the cached tbFile/dispFile paths
   * don't point at a stale or not-yet-available node.
   */
  reopen(): void {
    this.resolveTb();

    const dispDir = findBacklightDir(DISP_BACKLIGHT_NAMES);
    this.dispFile = dispDir ? `${dispDir}/brightness` : null;
    this.dispMax  = dispDir ? readInt(`${dispDir}/max_brightness`) : 0;

    this.activeHwLevel = 2; // reset tracking — hardware state is unknown after re-enumeration
  }

  /**
   * Apply the active level after resume and keep re-applying until the panel
   * confirms it. The appletb_backlight node may still be absent (write no-ops)
   * or get reset to its probe default (level 1 = "50%") as the HID interface
   * re-enumerates during attachTouchBar()'s config reprobes. A single on() in
   * resume() therefore races the re-bind and can leave the backlight stuck at
   * the default. Verify against actual_brightness and retry on a short schedule.
   */
  onVerified(adaptive: boolean, level: 0 | 1 | 2): void {
    this.stopSettle();
    const deadline = Date.now() + SETTLE_WINDOW_MS;
    const apply = (): void => {
      this.on(adaptive, level);
      const actual = this.tbDir ? readInt(`${this.tbDir}/actual_brightness`) : -1;
      const settled = !!this.tbFile && actual === this.activeHwLevel;
      // Stop once the panel confirms the level, or after the window expires.
      // Past the window the self-healing write() is the safety net: the next
      // wake()/dim() re-resolves the node and lands the write anyway.
      if (settled || Date.now() > deadline) this.stopSettle();
    };
    apply();
    if (!this.settleTimer) this.settleTimer = setInterval(apply, SETTLE_INTERVAL_MS);
  }

  private stopSettle(): void {
    if (this.settleTimer) { clearInterval(this.settleTimer); this.settleTimer = null; }
  }

  stop(): void { this.stopSettle(); }
}

// ── Pixel shift (AMOLED burn-in protection) ───────────────────────────────────
// Smooth bidirectional sweep: X travels ±11 px over pixelShiftSecs, Y follows a
// sinusoidal path. Pauses ~1 s at each endpoint before reversing direction.
// Larger range than a tight orbit — covers more pixel area with no visible jump.
// The shift is folded into toBinaryBuffer() at encode time (no separate pass).

// ── Renderer ──────────────────────────────────────────────────────────────────

export function render(
  element: React.ReactNode,
  display: DrmDisplay,
  options: RenderOptions = {},
): RenderResult {
  // Resolve timing — support deprecated screenSaverSecs as alias for dimSecs
  const dimMs = ((options.dimSecs ?? options.screenSaverSecs) ?? 0) * 1000;
  const offMs = ((options.offSecs ?? options.dimSecs ?? options.screenSaverSecs) ?? 0) * 1000;

  const adaptive    = options.adaptiveBrightness ?? false;
  const activeLevel = options.activeBrightness ?? 2;
  const backlight = new Backlight();

  const registry  = new TouchRegistry();
  const layoutRef: { current: Map<SceneNode, LayoutBox> } = { current: new Map() };
  let layoutValid = false;

  const container: RootContainer = {
    type: 'root',
    children: [],
    width:  display.width,
    height: display.height,
  };

  // ── Pixel shift ───────────────────────────────────────────────────────────
  const MAX_X_SHIFT    = 11;
  const MAX_Y_SHIFT    = 2;
  const SWEEP_STEP_MS  = 50;
  const SWEEP_PAUSE_MS = 1000;
  const randPhaseY     = Math.random() * Math.PI * 2;
  const psMs           = (options.pixelShiftSecs ?? 300) * 1000; // one-direction sweep duration

  let sweepPhase   = 0.5;   // 0 = leftmost (−MAX_X), 1 = rightmost (+MAX_X)
  let sweepDir     = 1;
  let sweepPauseMs = 0;
  let shiftX       = 0;
  let shiftY       = 0;

  function updateSweep(): boolean {
    if (sweepPauseMs > 0) {
      sweepPauseMs = Math.max(0, sweepPauseMs - SWEEP_STEP_MS);
      return false;
    }
    sweepPhase += sweepDir * (SWEEP_STEP_MS / psMs);
    if (sweepPhase >= 1.0) {
      sweepPhase = 1.0;
      sweepDir = -1;
      sweepPauseMs = SWEEP_PAUSE_MS;
    } else if (sweepPhase <= 0.0) {
      sweepPhase = 0.0;
      sweepDir = 1;
      sweepPauseMs = SWEEP_PAUSE_MS;
    }
    const nx = Math.round((sweepPhase * 2 - 1) * MAX_X_SHIFT);
    const ny = Math.round(Math.sin(sweepPhase * Math.PI * 4 + randPhaseY) * MAX_Y_SHIFT);
    if (nx === shiftX && ny === shiftY) return false;
    shiftX = nx;
    shiftY = ny;
    return true;
  }

  const shiftTimer = psMs > 0
    ? setInterval(() => {
        if (!updateSweep()) return;
        renderCurrent();           // update display first
        registry.setShift(shiftX, shiftY); // then sync touch coords
      }, SWEEP_STEP_MS)
    : null;

  // ── Screen-saver state ────────────────────────────────────────────────────
  type SsState = 'active' | 'dim' | 'off';
  let state: SsState = 'active';
  let suspended = false;
  let lastCmds: DrawCommand[] = [];
  let dimTimer:  ReturnType<typeof setTimeout> | null = null;
  let offTimer:  ReturnType<typeof setTimeout> | null = null;

  // Blit deduplication: skip display.render() when the frame is byte-identical
  // to what's already on screen (same commands + same pixel-shift). Makes idle
  // output free at the DRM level — cava silence, resting springs, no-op commits.
  let lastSig: string | null = '\0'; // sentinel: never matches a real frame
  let lastShiftX = NaN;
  let lastShiftY = NaN;

  // Flush-rate cap. appletbdrm runs a synchronous USB request/response handshake
  // per flush with a 1000ms timeout; driven too fast under load the device misses
  // the window, the response stream desyncs, and it cascades into a freeze. Cap
  // blits to FLUSH_FPS_CAP and coalesce bursts (e.g. 60fps spring frames) into a
  // single trailing flush, so the latest frame still lands without overrunning
  // the device's handshake.
  const MIN_FLUSH_MS = 1000 / (options.flushFps ?? 30); // flush-rate cap (default 30fps)
  let lastFlushAt = 0;
  let pendingFlush: ReturnType<typeof setTimeout> | null = null;

  // Opt-in partial flush (off by default — partial DirtyFB desyncs appletbdrm
  // under load → freeze; see RenderOptions.partialFlush). When on, flush only
  // the changed region; a pixel-shift change forces whole-FB.
  const PARTIAL = options.partialFlush ?? false;
  let prevCmds: DrawCommand[] | null = null;
  let prevShiftX = NaN;
  let prevShiftY = NaN;

  // Frame profiler — set REACT_DRM_PROFILE=1 to log a per-second breakdown of
  // where each frame's time goes (commits/s, blits/s, layout/serialize/blit ms,
  // draw_svg count). Pairs with the native [native] breakdown (cairo_renderer.cpp,
  // binding.cpp). Off by default; kept as a standing diagnostic tool.
  const PROFILE = !!process.env.REACT_DRM_PROFILE;
  const cpuCores = Math.max(1, os.cpus().length);
  let cpuPrev = process.cpuUsage();
  let wallPrev = performance.now();
  const cpuWindow: number[] = [];
  const prof = {
    commits: 0,
    blits: 0,
    partialBlits: 0,   // blits that flushed a band instead of the whole FB
    bandFracSum: 0,    // sum of (band width / display width) over partial blits
    fullLayout: 0,
    skippedLayout: 0,
    layoutMs: 0,
    serMs: 0,
    blitMs: 0,
    svg: 0,
    cmds: 0,
  };
  if (PROFILE) setInterval(() => {
    const now = performance.now();
    const cpu = process.cpuUsage(cpuPrev);
    const wallMs = Math.max(1, now - wallPrev);
    const cpuMs = (cpu.user + cpu.system) / 1000;
    const cpuPct1Core = (cpuMs / wallMs) * 100;
    const cpuPctAllCores = cpuPct1Core / cpuCores;
    cpuWindow.push(cpuPct1Core);
    if (cpuWindow.length > 10) cpuWindow.shift();
    const cpuAvg10s = cpuWindow.reduce((s, v) => s + v, 0) / cpuWindow.length;
    cpuPrev = process.cpuUsage();
    wallPrev = now;

    const c = prof.commits || 1, b = prof.blits || 1;
    const skipPct = ((prof.skippedLayout / c) * 100).toFixed(0);
    console.log(`[profile] commits/s=${prof.commits} blits/s=${prof.blits} | `
      + `layout(full=${prof.fullLayout}, skip=${prof.skippedLayout}, skip%=${skipPct}) | `
      + `layout=${(prof.layoutMs/c).toFixed(2)}ms ser=${(prof.serMs/c).toFixed(2)}ms blit=${(prof.blitMs/b).toFixed(2)}ms | `
      + `draw_svg/frame=${(prof.svg/c).toFixed(1)} cmds/frame=${(prof.cmds/c).toFixed(0)} | `
      + `partial=${prof.partialBlits}/${prof.blits} band=${prof.partialBlits ? ((prof.bandFracSum/prof.partialBlits)*100).toFixed(0) : '—'}% | `
      + `cpu(1core)=${cpuPct1Core.toFixed(1)}% cpu(10s)=${cpuAvg10s.toFixed(1)}% cpu(all-cores)=${cpuPctAllCores.toFixed(1)}%`);
    prof.commits = 0;
    prof.blits = 0;
    prof.partialBlits = 0;
    prof.bandFracSum = 0;
    prof.fullLayout = 0;
    prof.skippedLayout = 0;
    prof.layoutMs = 0;
    prof.serMs = 0;
    prof.blitMs = 0;
    prof.svg = 0;
    prof.cmds = 0;
  }, 1000);

  // The actual blit (always the current lastCmds + shift). Guarded so a deferred
  // trailing flush can't fire after suspend / screen-off.
  function doBlit(): void {
    if (pendingFlush) { clearTimeout(pendingFlush); pendingFlush = null; }
    if (suspended || state === 'off') return;
    lastFlushAt = performance.now();

    // Whole-FB by default; partial only when opted in AND the pixel-shift is
    // unchanged (a shift moves the whole frame → must repaint all).
    let clips: { x: number; y: number; w: number; h: number }[] | undefined;
    if (PARTIAL && prevCmds && shiftX === prevShiftX && shiftY === prevShiftY) {
      const d = damageRects(prevCmds, lastCmds);
      if (d && d.length) {
        // Full-screen-HEIGHT band: keep only the changed horizontal extent, span
        // the full height. On the 90°-rotated panel this is a contiguous run of
        // full FB rows — the shape mac-touchbar-plus's full-height clips produce.
        let minX = Infinity, maxX = -Infinity;
        for (const r of d) { if (r.x < minX) minX = r.x; if (r.x + r.w > maxX) maxX = r.x + r.w; }
        clips = [{ x: minX + shiftX, y: 0, w: maxX - minX, h: display.height }];
        if (PROFILE) { prof.partialBlits++; prof.bandFracSum += (maxX - minX) / display.width; }
      }
      // d === null → whole-FB (clips left undefined)
    }
    prevCmds = lastCmds;
    prevShiftX = shiftX;
    prevShiftY = shiftY;

    if (PROFILE) {
      const t = performance.now();
      display.renderBinary(toBinaryBuffer(lastCmds, shiftX, shiftY), clips);
      prof.blitMs += performance.now() - t;
      prof.blits++;
      prof.svg += lastCmds.reduce((n, c) => n + (c.cmd === 'draw_svg' ? 1 : 0), 0);
      prof.cmds += lastCmds.length;
    } else {
      display.renderBinary(toBinaryBuffer(lastCmds, shiftX, shiftY), clips);
    }
  }

  
  const nativeDraw: NativeDraw = {

    drawBars: (o) => {
      if (suspended || state === 'off') return;
 
      if (performance.now() - lastFlushAt < MIN_FLUSH_MS) return;
      display.drawBars({ ...o, x0: o.x0 + shiftX, baseY: o.baseY + shiftY });
      lastFlushAt = performance.now();
    },
  };

  function renderCurrent(force = false): void {
    if (suspended) return;       // DRM fd is closed during system sleep
    if (state === 'off') return; // screen stays on the black frame already rendered
    const sig = frameSignature(lastCmds);
    if (!force && sig !== null && sig === lastSig
        && shiftX === lastShiftX && shiftY === lastShiftY) {
      return; // identical frame already on screen — skip the blit
    }
    lastSig = sig;
    lastShiftX = shiftX;
    lastShiftY = shiftY;

    // Rate cap: flush now if enough time has passed (or forced); otherwise
    // schedule a single trailing flush. A burst collapses to one flush that
    // picks up the latest lastCmds when it fires.
    const since = performance.now() - lastFlushAt;
    if (force || since >= MIN_FLUSH_MS) {
      doBlit();
    } else if (!pendingFlush) {
      pendingFlush = setTimeout(doBlit, MIN_FLUSH_MS - since);
    }
  }

  function clearTimers(): void {
    if (dimTimer) { clearTimeout(dimTimer); dimTimer = null; }
    if (offTimer) { clearTimeout(offTimer); offTimer = null; }
  }

  function startIdleTimers(): void {
    clearTimers();
    if (dimMs <= 0) return;

    dimTimer = setTimeout(() => {
      state = 'dim';
      // Dim via hardware LED only — content unchanged on screen.
      // backlight.dim() is a no-op (returns false) when activeBrightness is
      // already at or below the dim level, so the user's chosen level is respected.
      backlight.dim();

      offTimer = setTimeout(() => {
        state = 'off';
        backlight.off();
        display.render([{ cmd: 'clear', r: 0, g: 0, b: 0 }]);
      }, offMs);
    }, dimMs);
  }

  function wake(): void {
    if (suspended) return; // reconnecting input during sleep is not activity
    const wasInactive = state !== 'active';
    const wasOff = state === 'off';
    state = 'active';
    clearTimers();
    if (wasOff || wasInactive) backlight.on(adaptive, activeLevel);
    if (wasOff) renderCurrent(true); // screen was cleared to black — force a repaint past the dedup cache
    startIdleTimers();
  }

  function onLid(closed: boolean): void {
    backlight.setLid(closed);
    if (closed) {
      // Lid closed — blank screen, keep idle timers running
      display.render([{ cmd: 'clear', r: 0, g: 0, b: 0 }]);
    } else {
      // Lid opened — treat as activity
      wake();
    }
  }

  backlight.on(adaptive, activeLevel);
  if (dimMs > 0) startIdleTimers();
  // ──────────────────────────────────────────────────────────────────────────

  container._onCommit = (needsLayout = true) => {
    if (!yogaReady()) return; // pre-engine commits are re-rendered once yoga loads
    const t0 = PROFILE ? performance.now() : 0;
    if (needsLayout || !layoutValid) {
      layoutRef.current = computeLayoutYoga(container, container.width, container.height);
      layoutValid = true;
      if (PROFILE) prof.fullLayout++;
    } else if (PROFILE) {
      prof.skippedLayout++;
    }
    const t1 = PROFILE ? performance.now() : 0;
    const commands = serializeScene(container, layoutRef.current);
    if (PROFILE) { prof.commits++; prof.layoutMs += t1 - t0; prof.serMs += performance.now() - t1; }
    lastCmds = commands;
    renderCurrent(); // respects current dim/off state
  };
  setRepaint((needsLayout = false) => container._onCommit?.(needsLayout));

  const root = reconciler.createContainer(
    container, 0, null, false, null, 'react-drm',
    (err: Error) => console.error('[react-drm] recoverable error:', err),
    null,
  );

  let latestEl: React.ReactNode = element;
  function doUpdate(el: React.ReactNode): void {
    latestEl = el;
    const wrapped = React.createElement(
      TouchRegistryContext.Provider, { value: registry },
      React.createElement(DisplaySizeContext.Provider, { value: { width: display.width, height: display.height } },
        React.createElement(NativeDrawContext.Provider, { value: nativeDraw },
          React.createElement(LayoutContext.Provider, { value: layoutRef }, el),
        ),
      ),
    );
    reconciler.updateContainer(wrapped, root, null, null);
  }

  doUpdate(element);
  // yoga loads async (ESM/WASM); commits before then are skipped, so re-commit
  // the latest tree once the engine is up.
  if (!yogaReady()) {
    loadYogaEngine()
      .then(() => doUpdate(latestEl))
      .catch(err => console.error('[react-drm] layout engine failed to load:', err));
  }

  // The evdev watchers' worker fds die silently when the devices disappear
  // (e.g. the apple-bce bus teardown during suspend), so suspend()/resume()
  // stop and recreate them. A caller-supplied KeyboardReader is driven the same
  // way — suspend()/resume() release and re-open its fd so we never hold it
  // across the teardown — while its onKey listener persists throughout.
  let stopLid     = watchLid(onLid);
  let stopPointer = dimMs > 0 ? watchPointer(wake) : () => {};
  let stopKeyboard  = () => {};
  const ownKeyboardWatch = dimMs > 0 && !options.keyboardReader;
  if (ownKeyboardWatch) {
    stopKeyboard = watchKeyboard(wake);
  } else if (dimMs > 0 && options.keyboardReader) {
    // Reuse the caller's KeyboardReader — no second fd open.
    stopKeyboard = options.keyboardReader.onKey((_code, value) => {
      if (value === 1) wake(); // key-down = activity
    });
  }

  // Open the Touch Bar touchpad. Wrapped so suspend() can drop its fd and
  // resume() can re-open against the freshly re-enumerated node — otherwise the
  // held fd survives the apple-bce teardown, goes stale ("(deleted)") and leaks
  // one fd per suspend cycle while delivering no events.
  let stopTouch = (): void => {};
  let touchRetryTimer: ReturnType<typeof setTimeout> | null = null;
  function startTouch(): void {
    if (touchRetryTimer) { clearTimeout(touchRetryTimer); touchRetryTimer = null; }
    try {
      const touchDevice = new TouchReader({ width: display.width, height: display.height });
      touchDevice.startWithGestures({
        onTouchStart: (x, y) => { wake(); registry.touchStart(x, y); },
        onTouchMove:  (x, y) => { registry.touchMove(x, y); },
        onTouchEnd:   (x, y) => { registry.touchEnd(x, y); },
      });
      stopTouch = () => { touchDevice.stop(); stopTouch = () => {}; };
      console.log('[react-drm] touch device ready');
    } catch (e) {
      console.warn('[react-drm] no touch device:', (e as Error).message ?? e);
      if (!suspended) {
        touchRetryTimer = setTimeout(() => {
          touchRetryTimer = null;
          if (!suspended) startTouch();
        }, INPUT_RETRY_MS);
      }
    }
  }
  startTouch();

  function suspend(): void {
    if (suspended) return;
    suspended = true;
    if (pendingFlush) { clearTimeout(pendingFlush); pendingFlush = null; }
    clearTimers();
    stopLid();
    stopPointer();
    stopTouch(); // drop the touch fd too — don't let it go stale across the teardown
    if (touchRetryTimer) { clearTimeout(touchRetryTimer); touchRetryTimer = null; } // cancel any in-flight touch retry
    if (ownKeyboardWatch) stopKeyboard();
    else options.keyboardReader?.suspend(); // release the caller's kbd fd too — don't hold it across teardown
    backlight.stop(); // cancel any in-flight resume settle loop
    backlight.off();
    display.close(); // device disappears during suspend — drop the fd cleanly
    console.log('[react-drm] suspended (display closed)');
  }

  function resume(): void {
    if (!suspended) return;
    display.reopen(); // throws if the card is not back yet — caller retries
    backlight.reopen(); // re-resolve sysfs paths after device re-enumeration
    suspended = false;
    state = 'active';
    stopLid = watchLid(onLid);
    stopPointer = dimMs > 0 ? watchPointer(wake) : () => {};
    if (ownKeyboardWatch) stopKeyboard = watchKeyboard(wake);
    else options.keyboardReader?.resume(); // re-open the caller's kbd fd closed in suspend()
    startTouch(); // re-open the touch fd against the re-enumerated node
    backlight.onVerified(adaptive, activeLevel); // retry until the panel confirms — HID backlight re-binds late
    startIdleTimers();
    renderCurrent(true); // display was closed during suspend — force a repaint past the dedup cache
    console.log('[react-drm] resumed');
  }

  return {
    unmount: () => {
      setRepaint(null);
      reconciler.updateContainer(null, root, null, null);
      if (pendingFlush) { clearTimeout(pendingFlush); pendingFlush = null; }
      clearTimers();
      if (shiftTimer) clearInterval(shiftTimer);
      stopLid();
      stopPointer();
      stopKeyboard();
      stopTouch();
      backlight.stop(); // cancel any in-flight resume settle loop
      if (touchRetryTimer) { clearTimeout(touchRetryTimer); touchRetryTimer = null; } // cancel any in-flight touch retry
    },
    update: doUpdate,
    hitTest:    (x, y) => registry.hitTest(x, y),
    touchStart: (x, y) => { wake(); registry.touchStart(x, y); },
    touchMove:  (x, y) => { wake(); registry.touchMove(x, y); },
    touchEnd:   (x, y) => { wake(); registry.touchEnd(x, y); },
    wake,
    suspend,
    resume,
  };
}
