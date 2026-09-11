import React from 'react';
import { Box } from 'react-drm';
import { ESC_KEY, DOCK, FN_LAYER, CUSTOM_LAYER } from '@/lib/utils/configLoader';
import { EscKey } from '@/components/EscKey';
import { SafeArea } from '@/components/SafeArea';
import { TouchIdOverlay } from '@/components/TouchIdOverlay';
import { BootScreen } from '@/components/BootScreen';
import { useBootSequence } from '@/lib/hooks/useBootSequence';
import { usePomodoroEngine } from '@/lib/hooks/usePomodoro';
import { useLayerToggle } from '@/lib/hooks/useLayerToggle';
import { useTouchIdPrompt } from '@/lib/hooks/useTouchIdPrompt';
import type { LayoutChildren } from '@/lib/routes/loadRoutes';

// No layoutConfig/initial here anymore — app/page.tsx (this segment's own
// sibling page) is root's default automatically, see loadRoutes.ts.

// Overlays toggled by a global hardware shortcut, not by navigating there —
// listed once so each binding's home fallback (below) knows not to flip
// straight into the other one.
const OVERLAYS = ['dock', 'fnkeys', 'custom-layer'];

export default function RootLayout({ width, height, children }: {
  width:    number;
  height:   number;
  children: LayoutChildren;
  path:     string; // '' — unused here, root addresses its own siblings by bare name
}) {
  useLayerToggle(DOCK.shortcut.key, 'dock', {
    mode: DOCK.shortcut.mode, longMs: DOCK.shortcut.longMs, doubleMs: DOCK.shortcut.doubleMs,
    home: 'splitted', overlays: OVERLAYS,
  });
  useLayerToggle('fn', 'fnkeys', {
    mode: FN_LAYER.mode, longMs: FN_LAYER.longMs, doubleMs: FN_LAYER.doubleMs,
    home: 'splitted', overlays: OVERLAYS,
  });
  useLayerToggle(CUSTOM_LAYER.shortcut.key, 'custom-layer', {
    mode: CUSTOM_LAYER.shortcut.mode, longMs: CUSTOM_LAYER.shortcut.longMs, doubleMs: CUSTOM_LAYER.shortcut.doubleMs,
    home: 'splitted', overlays: OVERLAYS,
  });

  const { booted, opacity } = useBootSequence();
  usePomodoroEngine();
  const touchId = useTouchIdPrompt();

  if (!booted) {
    return <BootScreen width={width} height={height} opacity={opacity} />;
  }

  // Wide Touch Bars (no physical Esc key) report a wider panel — show a fixed
  // Esc at the far left and inset the layer area by its width. Only in 'all'
  // mode; 'fn' mode renders Esc inside the Fn-key layer instead.
  const showEsc = width >= ESC_KEY.minWidth && ESC_KEY.onLayers === 'all';

  return (
    <SafeArea width={width} height={height}>
      {(w, h) => {
        const layerW = showEsc ? w - ESC_KEY.width - ESC_KEY.gap : w;
        const layerHost = children(layerW, h);

        const content = !showEsc ? layerHost : (
          <Box style={{ width: w, height: h, alignItems: 'stretch', gap: ESC_KEY.gap }}>
            <EscKey width={ESC_KEY.width} height={h} />
            {layerHost}
          </Box>
        );

        // The Touch ID prompt overlays whatever is on the bar (lock screen,
        // sudo) while fprintd is asking, and disappears again when it is idle.
        return (
          <Box style={{ width: w, height: h }}>
            {content}
            <TouchIdOverlay state={touchId} width={w} height={h} />
          </Box>
        );
      }}
    </SafeArea>
  );
}
