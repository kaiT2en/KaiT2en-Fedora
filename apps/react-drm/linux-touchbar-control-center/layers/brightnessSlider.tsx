import React, { useState, useRef, useEffect } from 'react';
import { execFile, execFileSync } from 'child_process';
import fs from 'fs';
import { Box, Text, Button, DISPLAY_BACKLIGHT_NAMES } from 'react-drm';
import { MdBrightness4, MdBrightness6, MdBrightness7, MdKeyboard } from 'react-icons/md';
import { BackButton } from '../components/BackButton';
import { useLayers } from './index';

// ── Helpers ───────────────────────────────────────────────────────────────────

// Device names vary by hardware: the panel backlight is gmux_backlight on
// dual-GPU Macs but intel_backlight on single-GPU ones (e.g. 2020 13" Intel),
// and the keyboard LED is exposed under names like ':white:kbd_backlight' or
// 'apple::kbd_backlight'. Auto-detect instead of hardcoding so the sliders work
// across machines and don't spam "Device not found" from the poll loop.
// Display candidate list mirrors tiny-dfr's find_display_backlight().
function findDevice(base: string, match: (name: string) => boolean): string | null {
  try { return fs.readdirSync(base).find(match) ?? null; } catch { return null; }
}

const DISPLAY_DEVICE  = findDevice('/sys/class/backlight', n => DISPLAY_BACKLIGHT_NAMES.some(c => n.includes(c)));
const KEYBOARD_DEVICE = findDevice('/sys/class/leds', n => n.includes('kbd_backlight'));
const AUTO_HIDE_MS = 5000;

function readBrightness(device: string | null): number {
  if (!device) return 0.5;
  try {
    const cur = parseInt(execFileSync('brightnessctl', ['--device', device, 'get'], { encoding: 'utf8' }).trim());
    const max = parseInt(execFileSync('brightnessctl', ['--device', device, 'max'], { encoding: 'utf8' }).trim());
    return max > 0 ? Math.min(1, cur / max) : 0.5;
  } catch { return 0.5; }
}

function applyBrightness(device: string | null, pct: number, minimumPct: number): void {
  if (!device) return;
  const value = Math.max(minimumPct, Math.round(pct * 100));
  execFile('brightnessctl', ['--device', device, 'set', `${value}%`], () => {});
}

// ── Track ─────────────────────────────────────────────────────────────────────

const TRACK_W  = 700;
const HANDLE_D = 14;

function Track({ fill, color }: { fill: number; color: string }) {
  const fillW   = Math.round(fill * TRACK_W);
  const handleX = Math.max(0, Math.min(TRACK_W - HANDLE_D, fillW - HANDLE_D / 2));

  return (
    <Box style={{ width: TRACK_W, height: HANDLE_D }}>
      {/* Dim track */}
      <Box style={{ position: 'absolute', left: 0,     top: 6, width: TRACK_W, height: 2, backgroundColor: '#1e293b' }} />
      {/* Filled portion */}
      {fillW > 0 && (
        <Box style={{ position: 'absolute', left: 0, top: 6, width: fillW, height: 2, backgroundColor: color }} />
      )}
      {/* Handle */}
      <Box style={{ position: 'absolute', left: handleX, top: 0, width: HANDLE_D, height: HANDLE_D, borderRadius: HANDLE_D / 2, backgroundColor: color }}>
        <Box style={{ position: 'absolute', left: 4, top: 4, width: 6, height: 6, borderRadius: 3, backgroundColor: '#0f172a' }} />
      </Box>
    </Box>
  );
}

interface BrightnessControlProps {
  label: string;
  value: number;
  color: string;
  icon: React.ReactNode;
  dragRef: React.MutableRefObject<{ x: number; v: number } | null>;
  onChange: (value: number) => void;
  onInteractionStart: () => void;
  onInteractionEnd: () => void;
}

function BrightnessControl({
  label,
  value,
  color,
  icon,
  dragRef,
  onChange,
  onInteractionStart,
  onInteractionEnd,
}: BrightnessControlProps) {
  function clamp(v: number) { return Math.max(0, Math.min(1, v)); }

  function onMove(x: number) {
    if (!dragRef.current) return;
    const next = clamp(dragRef.current.v + (x - dragRef.current.x) / TRACK_W);
    onChange(next);
    dragRef.current = { x, v: next };
  }

  return (
    <Box style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
      <Box style={{ width: 92, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 6 }}>
        {icon}
        <Text style={{ fontSize: 13, color: '#64748b', fontFamily: 'IosevkaTerm Nerd Font' }}>{label}</Text>
      </Box>

      <Button
        width={TRACK_W}
        height={60}
        color="transparent"
        activeColor="transparent"
        style={{ justifyContent: 'center', alignItems: 'center' }}
        onTouchStart={(x) => {
          onInteractionStart();
          dragRef.current = { x, v: value };
        }}
        onTouchMove={onMove}
        onTouchEnd={() => {
          dragRef.current = null;
          onInteractionEnd();
        }}
      >
        <Track fill={value} color={color} />
      </Button>

      <Text style={{ width: 52, fontSize: 18, color: '#94a3b8', fontFamily: 'IosevkaTerm Nerd Font' }}>
        {`${Math.round(value * 100)}%`}
      </Text>
    </Box>
  );
}

// ── Component ─────────────────────────────────────────────────────────────────

export function BrightnessSliderLayer({ width, height }: { width: number; height: number }) {
  const { go } = useLayers();
  const [displayBrightness, setDisplayBrightness] = useState(() => readBrightness(DISPLAY_DEVICE));
  const [keyboardBrightness, setKeyboardBrightness] = useState(() => readBrightness(KEYBOARD_DEVICE));
  const displayDrag = useRef<{ x: number; v: number } | null>(null);
  const keyboardDrag = useRef<{ x: number; v: number } | null>(null);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function clearHideTimer() {
    if (hideTimer.current) {
      clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  }

  function scheduleHide() {
    clearHideTimer();
    hideTimer.current = setTimeout(() => {
      displayDrag.current = null;
      keyboardDrag.current = null;
      go('splitted', 'slide-down');
    }, AUTO_HIDE_MS);
  }

  useEffect(() => {
    scheduleHide();
    const id = setInterval(() => {
      if (!displayDrag.current) {
        const current = readBrightness(DISPLAY_DEVICE);
        setDisplayBrightness(previous => Math.abs(previous - current) > 0.01 ? current : previous);
      }
      if (!keyboardDrag.current) {
        const current = readBrightness(KEYBOARD_DEVICE);
        setKeyboardBrightness(previous => Math.abs(previous - current) > 0.01 ? current : previous);
      }
    }, 500);
    return () => {
      clearHideTimer();
      clearInterval(id);
    };
  }, []);

  function updateDisplay(value: number) {
    setDisplayBrightness(value);
    applyBrightness(DISPLAY_DEVICE, value, 1);
  }

  function updateKeyboard(value: number) {
    setKeyboardBrightness(value);
    applyBrightness(KEYBOARD_DEVICE, value, 0);
  }

  const DisplayIcon = displayBrightness < 0.3
    ? MdBrightness4
    : displayBrightness < 0.7
      ? MdBrightness6
      : MdBrightness7;

  return (
    <Box style={{ flex: 1, flexDirection: 'row', alignItems: 'center' }}>
      <BackButton to="splitted" animation="slide-down" />
      <Box style={{ flex: 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 20 }}>
        <BrightnessControl
          label="KEYS"
          value={keyboardBrightness}
          color="#7dd3fc"
          icon={<MdKeyboard style={{ width: 28, height: 28 }} fill="#7dd3fc" stroke="none" />}
          dragRef={keyboardDrag}
          onChange={updateKeyboard}
          onInteractionStart={clearHideTimer}
          onInteractionEnd={scheduleHide}
        />
        <BrightnessControl
          label="DISPLAY"
          value={displayBrightness}
          color="#fbbf24"
          icon={<DisplayIcon style={{ width: 28, height: 28 }} fill="#fbbf24" stroke="none" />}
          dragRef={displayDrag}
          onChange={updateDisplay}
          onInteractionStart={clearHideTimer}
          onInteractionEnd={scheduleHide}
        />
      </Box>
    </Box>
  );
}
