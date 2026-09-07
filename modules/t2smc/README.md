# t2smc

Minimal SMC driver for T2 Macs. Provides fan control, battery charge limit,
temperature, power sensor and RTC access. Hardware monitoring is exposed via
the standard Linux hwmon interface. Requires no other SMC driver.

This is based on applesmc with macsmc patches but rebuilt from scratch.
The idea is to only use keys we really need and leave the rest to ACPI and drivers
for maximum stability.

## Installation

### One-time test / non persistent

Build and load the module for the currently running kernel without persistant installation:

```sh
make
sudo modprobe -r applesmc # (if installed)
sudo insmod t2smc.ko
```

This is enough to test fan control, temperature sensors, battery charge limit,
and RTC registration without installing anything. If another RTC driver already
registered `rtc0`, the t2smc RTC appears as the next free `rtcN`.

### Persistent Installation with RTC

This is for Fedora (dracut). For other distros please use equivalent commands.

To make `t2smc` available early enough to become `rtc0` and set the system clock,
install it for the current kernel and rebuild only that kernel's initramfs with
the driver included:

```sh
make
sudo modprobe -r applesmc # (if installed)
sudo make install
sudo dracut --force --add-drivers "t2smc" /boot/initramfs-$(uname -r).img $(uname -r)
```

For the matching boot entry, add these kernel command line parameters:

```text
initcall_blacklist=cmos_init module_blacklist=acpi_tad,applesmc
```

`initcall_blacklist=cmos_init` prevents the built-in `rtc_cmos` driver from
registering as `rtc0`. `module_blacklist=acpi_tad,applesmc` prevents the ACPI
TAD RTC and `applesmc` module from loading. With `t2smc` in the initramfs, the
expected result is:

```text
t2smc APP0001:00: registered as rtc0
t2smc APP0001:00: setting system clock ...
```

To uninstall the module:

```sh
sudo make uninstall
```

## Usage

After loading, the hwmon device appears at `/sys/class/hwmon/hwmonN/` with
`name` set to `t2smc`. The exact number `N` depends on your system and can
change across boots. This commands will list available paths:

```sh
for f in /sys/class/hwmon/hwmon*/name; do
    echo "$f: $(cat "$f")"
done

HWMON="$(dirname "$(grep -l '^t2smc$' /sys/class/hwmon/hwmon*/name)")"
```

## Power telemetry

Every SMC key whose name starts with `P` is discovered dynamically and exposed
through standard hwmon `powerN_label` and `powerN_input` files. The label is the
four-character SMC key and the input value is in microwatts:

```sh
paste "$HWMON"/power*_label "$HWMON"/power*_input
```

`t2smc` leaves the system battery and charger under the control of the
mainline ACPI SBS drivers. It exposes additional SMC telemetry on its hwmon
device without registering another battery:

```text
power_event_count
power_last_event_ns
smc_battery_capacity_percent
smc_battery_voltage_uv
smc_battery_current_ua
smc_battery_power_uw
smc_battery_charge_full_uah
smc_battery_charge_now_uah
smc_battery_cycle_count
smc_adapter_voltage_uv
smc_adapter_current_ua
smc_adapter_power_uw
```

Files for SMC keys not available on a particular model return no data. Values
are read on demand. There is no periodic kernel polling.

The driver subscribes to the standard power-supply notifier chain. An ACPI SBS
battery or adapter notification schedules an SMC status snapshot and increments
`power_event_count`.

To verify the event path, read the counter, connect or disconnect the charger,
and read it again:

```sh
HWMON="$(dirname "$(grep -l '^t2smc$' /sys/class/hwmon/hwmon*/name)")"
cat "$HWMON/power_event_count"
# Connect or disconnect the charger.
cat "$HWMON/power_event_count"
```

### Fan control

We do offer a program to control the fans on T2 MacBooks (https://github.com/deqrocks/t2-fancontrol)
Anyways here are examples to manual control the fans:

```sh
# Identify the t2smc hwmon device
HWMON="$(dirname "$(grep -l '^t2smc$' /sys/class/hwmon/hwmon*/name)")"

# Read current fan speeds
cat "$HWMON/fan1_input"
cat "$HWMON/fan2_input"

# Set fan 1 to 3000 RPM (enters manual mode automatically)
echo 3000 | sudo tee "$HWMON/fan1_target"
```

The fan speed values are in RPM. Writing to `fanN_target` switches the fan to
manual mode and sets the target speed.

The attributes `fanN_min` and `fanN_max` are limits reported by the SMC.
`fanN_min` is writable; `fanN_max` is read-only.

### Battery charge limit

`t2smc` also attaches the limit to `BAT0` as the standard
`charge_control_end_threshold` property, so any desktop environment that
speaks that interface can set it through its own power settings.

We do offer a GUI program to inspect sensor data and the battery charge limit:
(https://github.com/deqrocks/t2-smc-control).
Anyways here is how to set battery charge limit manually:

```sh
# Read current charge limit
cat "$HWMON/battery_charge_limit"

# Set charge limit to 80 percent
echo 80 | sudo tee "$HWMON/battery_charge_limit"
echo 80 | sudo tee /sys/class/power_supply/BAT0/charge_control_end_threshold
```

Valid range is 0-100. Persistence across reboot, shutdown, or SMC reset is not
guaranteed and may depend on the machine and firmware state.

### Temperature sensors

```sh
# List all sensor labels
cat "$HWMON"/temp*_label

# Read a specific sensor, e.g. GPU proximity
cat "$HWMON/temp21_input"
```

Temperature sensor labels are the SMC key names. Relevant GPU sensors on T2
Macs include:

| Label  | Sensor                  |
|--------|-------------------------|
| TG0P   | GPU proximity           |
| TGDD   | GPU die (digital)       |
| TGDF   | GPU die (filtered)      |
| TGVP   | GPU voltage regulator   |
| TC0E   | CPU 1 Diode Virtual     |
| TC0F   | CPU 1 Diode Filtered    |
| TC0P   | CPU proximity           |
| TB0T   | Battery temperature     |

Values are in millidegrees Celsius. Divide by 1000 for degrees.

### RTC

If the SMC RTC keys are present, `t2smc` registers a standard Linux RTC device.
The exact number `N` depends on load order. With early setup it can be `rtc0`;
with a late `insmod` it may be `rtc1`, `rtc2`, or higher.

```sh
# List RTC devices
for f in /sys/class/rtc/rtc*/name; do
    echo "$f: $(cat "$f")"
done

# Select the t2smc RTC
RTC="$(dirname "$(grep -l '^t2smc ' /sys/class/rtc/rtc*/name)")"
RTC_DEV="/dev/$(basename "$RTC")"

# Read the t2smc hardware clock
sudo hwclock --rtc "$RTC_DEV" --show

# Write the current system time to the t2smc hardware clock
sudo hwclock --rtc "$RTC_DEV" --systohc
```

## License

GPL-2.0-only.
