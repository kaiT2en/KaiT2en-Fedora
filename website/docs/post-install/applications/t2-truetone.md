# T2 TrueTone

<img class="app-shot" src="img/apps/t2-truetone.webp" alt="T2 TrueTone with status, True Tone and evening mode">

T2 TrueTone keeps the white of the display paper white in any light, warms
it in the evening and sets the display brightness from the ambient light.
T2 TrueTone uses the ambient light sensor of the T2 and is installed with
GNOME on the T2 MacBooks, the iMac20,1 and the iMac Pro.

After the first installation it is off. Open T2 TrueTone and press Start
next to Service. This starts the user service `t2-truetone.service`, which
also runs after the next login. Stop turns it off again.

While it runs, GNOME Night Light, GNOME's automatic brightness and the
dimming of the screen when idle are turned off, the app says so at the top.
Each comes back when its feature is switched off in the app.

## True Tone

The sensor measures the color of the ambient light. The display white follows
it, warmer under lamps, cooler in daylight.

- Strength sets how far the display follows the light. The default is marked
  on the slider. Lower values keep the display closer to neutral white.
- Transition sets how long the display takes to settle after the light
  changes.
- In dim light the white settles on a fixed warm white.

## Evening mode

Warms the display white between From and Until every day. Color temperature
sets the white at full evening mode, Fade the minutes it takes to fade in at
the start and out at the end. Evening mode and True Tone work together.

## Auto brightness

Adjusts the display brightness to the ambient light the way macOS does.

- Changes fade in slowly over a few seconds, in steps too small to see.
- Changes in light of less than about 20 % are ignored, so the screen does
  not follow every shadow.
- Setting the brightness by hand pauses it. One second later it learns the
  new brightness for the current light and keeps following from there.
- The slider and the brightness keys always start from the real brightness.
  The GNOME Shell extension KaiT2en Auto Brightness keeps the slider in sync
  and is switched on with auto brightness.

The learned curve is in `~/.local/state/t2-truetone/brightness/curve`.
Deleting it while the service is stopped starts again from the factory curve
of the model.

T2 TrueTone replaces the former automatic brightness service. The update
takes over its learned curve and keeps auto brightness on for the user who
runs it.
