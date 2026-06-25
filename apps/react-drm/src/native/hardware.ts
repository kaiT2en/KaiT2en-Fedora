/**
 * Hardware identifiers for the Touch Bar and its host machine — DRM driver
 * names, backlight sysfs nodes and the USB id. Single source of truth for every
 * TypeScript consumer (the renderer and the control center). install.sh and the
 * udev rule keep their own literal copies; they're bash/udev and can't import
 * this — to add a name, update here plus those two files.
 */

// Touch Bar DRM panel driver. Ships under either name depending on the
// kernel/fork (appletbdrm upstream, t2bdrm on some forks); a DRM card whose
// uevent DRIVER= matches any of these is the Touch Bar.
export const TOUCHBAR_DRM_DRIVERS = ['appletbdrm', 't2bdrm'] as const;

// Touch Bar backlight sysfs node, in preference order. The HID backlight ships
// as appletb_backlight upstream or t2tb_backlight on some forks; 'display-pipe'
// is the generic DRM-pipe fallback.
export const TOUCHBAR_BACKLIGHT_NAMES = ['display-pipe', 'appletb_backlight', 't2tb_backlight'] as const;

// Host display (laptop panel) backlight candidates, in preference order. Used
// to drive the on-screen brightness slider, not the Touch Bar itself: the panel
// backlight is gmux_backlight on dual-GPU Macs, intel_backlight on single-GPU
// ones, etc. Mirrors tiny-dfr's find_display_backlight().
export const DISPLAY_BACKLIGHT_NAMES = ['apple-panel-bl', 'gmux_backlight', 'intel_backlight', 'acpi_video0'] as const;

// Touch Bar Display USB id (Apple, T2 Touch Bar — 05ac:8302).
export const TOUCHBAR_USB_VENDOR_ID = '05ac';
export const TOUCHBAR_USB_PRODUCT_ID = '8302';
