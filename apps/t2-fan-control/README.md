# T2 Fan Control

<p align="center">
<img src="assets/fancontrol.svg" alt="T2 Fan Control logo" width="96">
</p>

<p align="center">
<img src="assets/t2fancontrol_dark.png" alt="T2 Fan Control dark mode screenshot" style="height: 400px;">
</p>
Fan control for T2 Macs on Linux, written in Rust. Works with applesmc, macsmc
and t2smc. A systemd daemon drives the fans across boot, suspend and resume,
the GTK app monitors temperatures and edits the curve over a Unix socket.

## Native SMC or custom curve

The `Custom curve` switch in the header bar selects who drives the fans. The choice is stored as `automatic_control_enabled` and survives reboots.

- On: the custom curve described below controls both fans.
- Off: the fans run on the native SMC fan control, the same control macOS leaves them on. The app keeps showing temperatures and fan speeds but does not write fan speeds. The curve and the advanced settings are greyed out.

The T2 SMC keeps manual fan mode across reboots. In native mode the daemon therefore hands both fans back to the SMC at startup. With t2smc this needs a driver that accepts `0` in `fanN_target` as hand back.

## How fan control works

Component cooling and retained system heat are kept separate. Temperatures are sampled every two seconds.

### Curve temperature

The editable fan curve follows the hotter of the CPU and GPU. It does not use an average and it does not use unrelated board, storage or enclosure sensors.

- CPU is the arithmetic mean of the SMC sensors `TC0E` (CPU 1 Diode Virtual) and `TC0F` (CPU 1 Diode Filtered). If only one is readable, that reading is used.
- GPU is the highest available reading from GPU Proximity, GPU Die digital, GPU Die analog and GPU Voltage Regulator.
- If no GPU temperature is available, the curve follows CPU alone. The UI changes its description accordingly.
- If only a GPU temperature is available, the curve follows GPU alone.

CPU and dGPU are fixed curve inputs and cannot be removed or replaced. An optional third curve sensor can be selected from the other valid monitored sensors. With a third sensor selected, the curve follows the highest value among CPU, dGPU and that sensor. `None` is the default.

Curve points are limited to 100 C. Once the curve temperature rises above the rightmost point, both fans are commanded to 100%, regardless of that point's configured speed.

The optional sensor is drawn as its own line in the curve and temperature graphs. Its controls, the any-sensor protection and the overrun time are grouped under `Advanced settings` to keep the main view focused on normal fan control.

`Curve temp` shows the value currently fed into the curve. `Curve sensor` identifies the CPU or GPU sensor responsible for that value.

### System temperature and system target

The system temperature is the mean of all positive SMC temperatures except CPU core, die and package sensors. It reflects heat retained by the logic board, cooling assembly and enclosure.

The red vertical wall in the curve editor is the configurable system-temperature target:

- Both fans are forced to 100% when the system average reaches 2 C above the target.
- Forced cooling remains active until the system average reaches 2 C below the target.
- For example, a 45 C target engages at 47 C and releases at 43 C.
- The resulting 4 C hysteresis band prevents rapid fan cycling around the target.
- The wall can be moved from 30 C to 100 C.

Forced system cooling also has a configurable `Overrun time` from 0 to 600 seconds, with a default of 5 seconds. Once engaged, cooling is released only after both conditions are true:

- The configured minimum cooling time has elapsed.
- The system average has remained at or below `target - 2 C` continuously for 20 seconds.

If the temperature rises above the lower boundary during that 20-second confirmation window, the window starts over. There is deliberately no minimum off time, so rising temperatures can always engage cooling immediately.

The system target is controlled only by the system average. A high CPU or GPU temperature continues to follow the normal curve and cannot independently engage the system-target latch.

### Optional any-sensor protection

An optional checkbox can force both fans to 100% when the highest positive reading from any monitored temperature sensor reaches an adjustable threshold. The threshold accepts values from 0 C to 100 C. This protection is disabled by default and is independent of both the CPU/GPU curve and the system-average wall.

The any-sensor protection uses a 5 C release hysteresis: a threshold of 90 C engages at 90 C and releases at 85 C. A value of 0 C therefore intentionally keeps forced cooling active whenever any valid positive sensor is available.

### Editing the curve

The curve has a start point, an end point and one bend handle (the hollow ring) that sits on the curve:

- Start and end can be moved horizontally and vertically. Moving them keeps the bend's relative position.
- Moving the handle along the curve shifts where the bend sits; moving it up or down bends the curve. With the handle on the straight line between start and end, the curve is straight.
- The curve is a smooth, monotone cubic through the three points, so there are no kinks. Below the start point the fans hold the start speed.
- Configurations with the former four-point curve keep their first, third and fourth point.
- Curve points may sit on either side of the system wall because the two controls are independent.
- Moving the system wall never moves, scales or limits the fan curve.

Fan percentages are relative to each fan's reported minimum and maximum RPM. Therefore `0%` means the hardware minimum RPM, not a stopped fan.

Fan speed follows the curve in 1% steps: every 100 ms upward (10%/s) and every 500 ms downward (2%/s). Temperatures are still read every two seconds; the short wakeups only happen while a ramp runs. A new ramp starts once the curve target differs by 3% or more, so a steady temperature causes no fan writes. Exceeding the curve's end point, the system target or the any-sensor protection jumps to 100% immediately.

### Monitoring values

`Hottest sensor` is informational and is independent of the fan-curve input. It reports the highest positive reading among all monitored sensors using a human-readable name, for example `PCH Diode · 78 C`. The fan details show how many valid temperature sensors are currently being monitored.

## Default configuration

New installations use:

```ini
config_version=2
automatic_control_enabled=true
autostart_enabled=false
system_temp_target_c=45
system_cooling_time_s=5
any_sensor_enabled=false
any_sensor_temp_c=100
curve_sensor_key=
custom_curve=0:10,70:25,95:65
```

Curve entries use `temperature:speed-percent` pairs. The active configuration is stored at:

```text
/etc/t2-fancontrol/config.txt
```

A file without `config_version=2`, from the earlier preset-based controller, is replaced with the defaults when the daemon starts.

## Installation

```bash
make
sudo make install
```

This installs the binary, desktop entry, icon and systemd unit under `/usr/local`, enables `t2-fancontrol.service` and stops `t2fanrd.service` if present.

## Uninstall

```bash
sudo make uninstall
```

This removes everything again and re-enables `t2fanrd.service` if present.

## Build Dependencies

You need a Rust toolchain and the usual native build dependencies for GTK4 and Libadwaita.

- `cargo`
- `make`
- `pkg-config`
- `glib-compile-resources`
- GTK4 development files
- Libadwaita development files

## Support

[Fund my bugs](https://donate.stripe.com/eVq14n8a7agh2lQdqq14400)
