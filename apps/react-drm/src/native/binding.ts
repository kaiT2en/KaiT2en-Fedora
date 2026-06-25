import fs from 'fs';
import { loadAddon } from './load-addon';
import { TOUCHBAR_DRM_DRIVERS } from './hardware';
import type { DrawCommand, BinaryFrame } from '../scene/serialize';

const TOUCHBAR_DRM_RE = new RegExp(`DRIVER=(${TOUCHBAR_DRM_DRIVERS.join('|')})`, 'i');

function resolveCardPath(devicePath?: string): string {
  if (devicePath) return devicePath;

  const envPath = process.env.REACT_DRM_DEVICE_PATH;
  if (envPath) return envPath;

  // Prefer the Touch Bar DRM card if present.
  try {
    const cards = fs.readdirSync('/sys/class/drm').filter(n => /^card\d+$/.test(n));
    for (const card of cards) {
      const uevent = fs.readFileSync(`/sys/class/drm/${card}/device/uevent`, 'utf8');
      if (TOUCHBAR_DRM_RE.test(uevent)) return `/dev/dri/${card}`;
    }
  } catch (_) { /* fall through */ }

  return '/dev/dri/card1';
}

interface NativeModule {
  DrmDisplay: new (devicePath: string) => NativeHandle;
  usbReset(devnode: string): void;
}

// Lazy-load the native addon — fails with a clear message if not built yet.
function loadNative(): NativeModule {
  return loadAddon() as NativeModule;
}

export interface DamageRect { x: number; y: number; w: number; h: number; }

export interface BarsOpts {
  x0: number; baseY: number; barW: number; gap: number; fullHeight: number;
  bg: [number, number, number];
  heights: number[];   // px, bottom-aligned at baseY
  colors: number[];    // flat r,g,b per bar
}

interface NativeHandle {
  setup(): { width: number; height: number };
  render(commands: DrawCommand[], clips?: DamageRect[]): void;
  renderBinary(data: Float32Array, strings: string[], buffers: Buffer[], clips?: DamageRect[]): void;
  drawBars(opts: BarsOpts): void;
  screenshot(filePath: string): void;
  getWidth(): number;
  getHeight(): number;
  close(): void;
}

/**
 * USBDEVFS_RESET ioctl on a USB device node (`/dev/bus/usb/BBB/DDD`).
 * Wakes the Touch Bar firmware's display interface out of its idle sleep —
 * the state where every transfer (including config switches) fails with
 * ETIMEDOUT. Needs write access to the node (see system/99-react-drm.rules).
 */
export function usbReset(devnode: string): void {
  loadNative().usbReset(devnode);
}

export class DrmDisplay {
  private handle: NativeHandle;
  private readonly devicePath?: string;
  private closed = false;

  readonly width: number;
  readonly height: number;

  constructor(devicePath?: string) {
    this.devicePath = devicePath;
    const native = loadNative();
    const resolvedPath = resolveCardPath(devicePath);
    this.handle = new native.DrmDisplay(resolvedPath);
    const info = this.handle.setup();
    this.width = info.width;
    this.height = info.height;
    console.log(`[react-drm] DRM display ready: ${this.width}×${this.height} on ${resolvedPath}`);
  }

  // clips: damage rects (logical coords) for partial flush. Omit → whole-FB.
  render(commands: DrawCommand[], clips?: DamageRect[]): void {
    this.handle.render(commands, clips);
  }

  // Binary render path: passes a Float64Array command buffer + string/buffer
  // tables to native, cutting N-API property-lookup overhead vs the JS-object path.
  renderBinary(frame: BinaryFrame, clips?: DamageRect[]): void {
    this.handle.renderBinary(frame.data, frame.strings, frame.buffers, clips);
  }

  // Draw an audio-bar strip natively + flush only its full-height band.
  // Bypasses the React commit loop (no layout/serialize) for cheap bar updates.
  drawBars(opts: BarsOpts): void {
    this.handle.drawBars(opts);
  }

  /** Write the currently displayed frame to a PNG file (logical orientation). */
  screenshot(filePath: string): void {
    this.handle.screenshot(filePath);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    try { this.handle.close(); } catch { /* fd may already be dead (device gone) */ }
  }

  /**
   * Reopen the display after the device was lost and came back (e.g. the
   * Touch Bar re-enumerates after suspend). Re-resolves the card path — the
   * card number can change across re-enumeration.
   */
  reopen(): void {
    this.close();
    const native = loadNative();
    const resolvedPath = resolveCardPath(this.devicePath);
    this.handle = new native.DrmDisplay(resolvedPath);
    const info = this.handle.setup();
    this.closed = false;
    if (info.width !== this.width || info.height !== this.height) {
      console.warn(`[react-drm] display size changed on reopen: ${info.width}×${info.height} (was ${this.width}×${this.height})`);
    }
    console.log(`[react-drm] DRM display reopened on ${resolvedPath}`);
  }
}
