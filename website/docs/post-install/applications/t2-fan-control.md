# T2 Fan Control

<img class="app-shot" src="img/apps/t2-fan-control.webp" alt="T2 Fan Control with temperatures, fan curve and fan speeds">

Shows temperatures and fan speeds and drives both fans along an editable
curve. The service `t2-fancontrol.service` keeps fan control active across
login, suspend, resume and reboot. The app talks to it over a Unix socket.

## Native SMC or custom curve

The switch in the header bar selects who drives the fans.

- On: the curve below controls both fans.
- Off: the SMC controls the fans, as under macOS. The app keeps showing
  temperatures and fan speeds.

## Curve

The curve follows the hotter of CPU and GPU. CPU is the mean of the SMC
sensors `TC0E` and `TC0F`, GPU the highest GPU reading. Under **Advanced
settings** a third sensor can be added. The curve then follows the highest of
the three.

Drag the start and end points to set the speed at low and high temperature.
The hollow ring bends the curve. Above the rightmost point both fans run at
100 %. Fan percentages are relative to each fan's minimum and maximum RPM, so
`0%` is the hardware minimum speed.

## System temperature wall

The red vertical line is the system temperature target, 45 °C by default. The
system temperature is the mean of all positive SMC readings except the CPU
core, die and package sensors. It reflects the heat held in the logic board,
heatpipe and case.

- 2 °C above the target, both fans go to 100 %.
- They return to the curve once the system temperature has stayed 2 °C below
  the target for 20 seconds and the **Overrun time** (default 5 s) has passed.

This keeps the case from heating up until the SMC throttles the CPU through
PROCHOT.

**Advanced settings** also offers an any-sensor limit: both fans go to 100 %
when any sensor reaches the threshold, with a 5 °C release hysteresis. It is
off by default.

## Settings

The configuration is `/etc/t2-fancontrol/config.txt`. The default curve is
`0:10,70:25,95:65` in `temperature:percent` pairs.
