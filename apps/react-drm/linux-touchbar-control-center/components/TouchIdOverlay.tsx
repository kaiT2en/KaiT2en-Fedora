import React, { useEffect } from 'react';
import { Box, Text, Svg, animated, useSpringValue } from 'react-drm';
import type { TouchIdState } from '@/lib/hooks/useTouchIdPrompt';

// The Touch ID sensor is the power key at the far right of the Touch Bar, so
// the prompt lives there and points at the physical sensor, the way macOS
// orients it. Colour and caption follow the state; 'idle' draws nothing.
const LOOK: Record<Exclude<TouchIdState, 'idle'>, { color: string; label: string }> = {
  waiting:  { color: '#ff375f', label: 'Touch ID' },
  scanning: { color: '#ff5f7a', label: 'Touch ID' },
  matched:  { color: '#34c759', label: 'Unlocked' },
  retry:    { color: '#ff9f0a', label: 'Try again' },
  failed:   { color: '#ff453a', label: 'Use password' },
};

// A fingerprint whorl: nested rounded arcs with a short core, tinted per state.
function fingerprint(color: string): string {
  return `<svg viewBox="0 0 40 40" xmlns="http://www.w3.org/2000/svg">
    <g fill="none" stroke="${color}" stroke-width="2.1" stroke-linecap="round">
      <path d="M20 7.5c-6.9 0-12.5 5.6-12.5 12.5v6"/>
      <path d="M32.5 22.5V20c0-6.9-5.6-12.5-12.5-12.5"/>
      <path d="M11.7 20c0-4.6 3.7-8.3 8.3-8.3s8.3 3.7 8.3 8.3v6.5"/>
      <path d="M15.9 20a4.1 4.1 0 0 1 8.2 0v8.5c0 1.6-.3 3.1-.9 4.5"/>
      <path d="M20 20v9.2c0 2.2-.6 4.3-1.7 6.1"/>
      <path d="M11.7 24.8c0 2.1-.5 4.1-1.4 5.9"/>
    </g>
  </svg>`;
}

export function TouchIdOverlay({ state, width, height }: {
  state:  TouchIdState;
  width:  number;
  height: number;
}) {
  // One breathing glow drives the fingerprint's opacity. Layout-only props
  // (scale, translate) are not animatable in this renderer, so the motion is
  // carried entirely by opacity, which reads as a gentle pulse.
  const glow = useSpringValue(1);
  const breathing = state === 'waiting' || state === 'scanning';

  useEffect(() => {
    glow.stop();
    if (breathing) {
      const period = state === 'scanning' ? 420 : 900;
      glow.set(1);
      glow.start({ to: 0.35, loop: { reverse: true }, config: { duration: period } });
    } else {
      glow.start({ to: 1, config: { duration: 140 } });
    }
  }, [breathing, state, glow]);

  if (state === 'idle') return null;

  const look = LOOK[state];
  const cap  = Math.round(height * 0.82);
  const glyph = Math.round(cap * 0.78);

  return (
    <Box style={{
      position: 'absolute', top: 0, left: 0, width, height,
      flexDirection: 'row', alignItems: 'center', justifyContent: 'flex-end',
      gap: 12, paddingRight: Math.round(height * 0.16),
      backgroundColor: '#000000e6',
    }}>
      <Text style={{ fontSize: Math.round(height * 0.3), color: '#e8e8ea' }}>
        {look.label}
      </Text>
      <Box style={{
        width: cap, height: cap, borderRadius: Math.round(cap * 0.28),
        backgroundColor: '#1c1c1e', alignItems: 'center', justifyContent: 'center',
      }}>
        <animated.Box style={{ width: glyph, height: glyph, opacity: glow }}>
          <Svg src={fingerprint(look.color)} width={glyph} height={glyph} />
        </animated.Box>
      </Box>
    </Box>
  );
}
