import { execSync } from 'node:child_process';
import process from 'node:process';
import { KEY } from 'react-drm';
import type { KeyId } from 'react-drm';
import type { IconType } from 'react-icons';
import {
  FaFolder, FaTerminal, FaFirefoxBrowser, FaCode, FaMusic, FaGithub, FaGear,
} from 'react-icons/fa6';

/**
 * Central configuration for the example app.
 * Anything tunable lives here — add new sections as features grow.
 */

// ─── Display ────────────────────────────────────────────────────────────────

export const DISPLAY = {
  dimSecs:          30,   
  offSecs:          60,   
  pixelShiftSecs:   300,  
  activeBrightness: 2,
  flushFps:         30,
  partialFlush:     false, // true = not ready yet
} as const;

// ─── On-screen Esc key ───────────────────────────────────────────────────────

export const ESC_KEY: {
  minWidth: number;
  onLayers: 'all' | 'fn';
  width: number;
  gap: number;
} = {
  // Wide Touch Bars (MacBooks without a physical Esc key) report a wider
  // panel — the standard bar is 2008 px, the wide variant 2170 px. Show an
  // on-screen Esc at the far left when the auto-detected display width is at
  // least this. Set to 0 to always show, Infinity to never.
  minWidth: 2170,
  // Where the Esc key shows on wide displays:
  //   'all' — a fixed Esc button at the far left of every layer
  //   'fn'  — Esc shows only in the Fn-key layer, sized like the F-keys
  onLayers: 'fn',
  // Only used by 'all' mode (the fixed left button); 'fn' mode sizes Esc
  // like the surrounding F-keys.
  width:    110,  // px reserved on the left for the Esc button
  gap:      8,    // px between the Esc button and the layer area
} as const;

// ─── Touch Bar lifecycle ─────────────────────────────────────────────────────

export const SLEEP = {
  // In-app Touch Bar lifecycle: attach at startup, quiesce before system
  // sleep (logind delay inhibitor), re-attach + resume after. Applies to
  // every run mode — manual `npm run dev` and react-drm.service alike.
  enabled: true,
  // How long to wait for the appletbdrm card at startup and after resume
  // (covers re-enumeration, udev permission settling and config-write retries).
  cardWaitSecs: 30,
} as const;

// ─── Layer transitions ──────────────────────────────────────────────────────

export const LAYER_TRANSITION = {
  outDurationMs: 200, // leaving layer (fade-out / slide-out)
  inDurationMs:  350, // entering layer — slower so the new layer eases in
} as const;

// ─── Active window tracking ─────────────────────────────────────────────────

export const ACTIVE_WINDOW = {
  // 'auto' detects the session (Xorg vs Wayland, then Hyprland / niri / GNOME /
  // KDE Plasma — sudo-safe, via sockets not env vars) and picks the matching
  // backend. Set a backend name to skip detection and force one.
  backend: 'auto' as 'auto' | 'hyprland' | 'niri' | 'gnome' | 'plasma' | 'xorg',
};

// ─── Screenshots ────────────────────────────────────────────────────────────

// The app usually runs under sudo — save into the real user's home, not /root.
const home = process.env.SUDO_USER ? `/home/${process.env.SUDO_USER}` : (process.env.HOME ?? '.');

// Use nodejs to get the path to the images folder independent from the user's language.
let picturesDir: string;
try {
  const cmd = process.env.SUDO_USER ? `sudo -u ${process.env.SUDO_USER} xdg-user-dir PICTURES` : 'xdg-user-dir PICTURES';
  picturesDir = execSync(cmd).toString().trim();
} catch {
  picturesDir = `${home}/Pictures`;
}

export const SCREENSHOT = {
  keys: ['ctrl', 'alt', 's'] as KeyId[],
  dir:  `${picturesDir}/touchbar`,
};

// ─── Dolphin panel ──────────────────────────────────────────────────────────

export const DOLPHIN = {
  maxPlaces: 5,    // quick-jump place chips shown in the panel
  pollMs:    1000, // action-state poll interval (dolphin emits no property-change signals)
};

// ─── Konsole panel ──────────────────────────────────────────────────────────

export const KONSOLE = {
  pollMs: 1000, // tab/session sync interval (konsole emits no D-Bus signals)
};

// ─── System bar ─────────────────────────────────────────────────────────────

export const SYSTEMBAR = {
  // CPU/mem/net/temp refresh. Each tick is a full systembar re-render+blit, so
  // this drives idle cost; 2s reads fine for a status strip.
  statsPollMs: 2000,
};

// ─── Audio visualizer (CAVA) ────────────────────────────────────────────────

export const CAVA = {
  // Lower defaults for better overall responsiveness.
  // framerate drives active-audio CPU almost linearly (each frame = a full
  // commit+blit while bars move). 12 is a good CPU/smoothness balance; drop to
  // 10 for even less CPU, raise toward 30 for smoother bars.
  bars: 32,
  framerate: 24,
} as const;

// ─── Browser shortcuts ──────────────────────────────────────────────────────

/**
 * Each value is an array of Linux keycodes pressed simultaneously,
 * listed in the order they should be held down.
 * All available codes are in the KEY object from 'react-drm'.
 */
export type BrowserKeymap = {
  back:     number[];
  forward:  number[];
  reload:   number[];
  home:     number[];
  newTab:   number[];
  closeTab: number[];
  nextTab:  number[];
  prevTab:  number[];
};

/** Fallback shortcuts, used for any browser without an override below. */
export const DEFAULT_BROWSER_KEYS: BrowserKeymap = {
  back:     [KEY.LEFTALT,  KEY.LEFT],
  forward:  [KEY.LEFTALT,  KEY.RIGHT],
  reload:   [KEY.LEFTCTRL, KEY.KEY_R],
  home:     [KEY.LEFTALT,  102],
  newTab:   [KEY.LEFTCTRL, KEY.KEY_T],
  closeTab: [KEY.LEFTCTRL, KEY.KEY_W],
  nextTab:  [KEY.LEFTCTRL, KEY.TAB],
  prevTab:  [KEY.LEFTCTRL, KEY.LEFTSHIFT, KEY.TAB],
};


export const BROWSER_KEY_OVERRIDES: Record<string, Partial<BrowserKeymap>> = {
  // firefox:       { reload: [KEY.F5] },
  // chromium:      { nextTab: [KEY.LEFTCTRL, KEY.PAGEDOWN], prevTab: [KEY.LEFTCTRL, KEY.PAGEUP] },
  // 'google-chrome': {},
};

/** Resolve the effective keymap for a window class. */
export function browserKeysFor(windowClass: string): BrowserKeymap {
  const overrides = BROWSER_KEY_OVERRIDES[windowClass.toLowerCase()] ?? {};
  return { ...DEFAULT_BROWSER_KEYS, ...overrides };
}

// ─── App dock (Plank-style) ───────────────────────────────────────────────────

export type DockApp = {
  id:          string;     // stable React key
  label:       string;     // app name (for reference / future tooltips)
  iconName?:   string;     // freedesktop icon name — real app icon from the theme
  icon:        IconType;   // react-icons fallback when no theme icon is found
  color:       string;     // tint for the react-icons fallback
  command:     string;     // executable to launch on tap
  args?:       string[];   // launch arguments
  matchClass?: string[];   // window-class substrings that mark this app "running"
};

/**
 * The Plank-style dock layer. Edit `apps` to pin/unpin entries — each one is a
 * launchable command plus the window classes that count as "this app running",
 * which drives the indicator dot under the icon (matched against the focused
 * window from the active-window backend).
 *
 * Icons use the real app icon from your icon theme via `iconName` (a
 * freedesktop name like `org.kde.dolphin` or `firefox` — usually the `Icon=`
 * value in the app's .desktop file). If that name can't be found in the theme,
 * the dock falls back to the react-icons `icon` glyph.
 */
export const DOCK = {
  iconSize:  50,    // px — icon glyph size
  slot:      65,    // px — square tap target per app
  gap:       14,    // px — space between icons
  lift:      10,    // px — how far an icon rises while pressed (Plank bounce)
  panel: {
    color:  'rgba(20, 22, 30, 0.78)', // translucent dock background
    radius: 20,
    padX:   20,     // horizontal padding inside the panel
    padY:   4,      // vertical padding inside the panel
  },
  indicator: {
    color: '#7dd3fc',
    size:  5,       // running/focused dot diameter
  },
  apps: [
    { id: 'files',    label: 'Files',    iconName: 'org.kde.dolphin',        icon: FaFolder,         color: '#7dd3fc', command: 'dolphin',        matchClass: ['dolphin'] },
    { id: 'terminal', label: 'Terminal', iconName: 'org.kde.konsole',        icon: FaTerminal,       color: '#cccccc', command: 'konsole',        matchClass: ['konsole'] },
    { id: 'firefox',  label: 'Firefox',  iconName: 'firefox',                icon: FaFirefoxBrowser, color: '#ff9d5c', command: 'firefox',        matchClass: ['firefox'] },
    { id: 'code',     label: 'Code',     iconName: 'visual-studio-code',     icon: FaCode,           color: '#60a5fa', command: 'code',           matchClass: ['code', 'vscodium'] },
    { id: 'music',    label: 'Music',    iconName: 'vlc',                    icon: FaMusic,          color: '#c084fc', command: 'vlc',            matchClass: ['vlc'] },
    { id: 'github',   label: 'GitHub',   iconName: 'github',                 icon: FaGithub,         color: '#cccccc', command: 'xdg-open',       args: ['https://github.com'] },
    { id: 'settings', label: 'Settings', iconName: 'systemsettings',         icon: FaGear,           color: '#94a3b8', command: 'systemsettings', matchClass: ['systemsettings'] },
  ] as DockApp[],

  // Keyboard gesture that toggles the dock layer on/off. Long-press the key to
  // show the dock; long-press again to return to the previous layer.
  //
  // 'ralt' = Right Option/Alt on the MacBook keyboard. Swap to any KEY
  // name from react-drm (e.g. 'rmeta', 'rctrl', 'menu') if you prefer.
  shortcut: {
    key:    'ralt' as KeyId,
    longMs: 900,   // hold time that counts as a long-press
  },
};

// ─── Fn-key layer ─────────────────────────────────────────────────────────────

export const FN_LAYER = {
  // How the Fn key reaches the F-keys layer:
  //   'hold'   — momentary: the layer shows only while Fn is held (original).
  //   'toggle' — long-press Fn to switch to it, long-press again to return.
  mode:   'toggle' as 'hold' | 'toggle',
  longMs: 350,   // long-press duration when mode === 'toggle'
};
