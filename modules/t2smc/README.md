# t2smc

SMC driver for T2 Macs: fan control, battery charge limit, sensors through
hwmon, a watchdog, SMC events and the RTC. It replaces applesmc and only uses
the keys it needs, everything else stays with ACPI and other drivers.

## Installation

### Test without installing

```sh
make
sudo modprobe -r applesmc # (if installed)
sudo insmod t2smc.ko
```

If another RTC driver already registered `rtc0`, the t2smc RTC becomes the
next free `rtcN`.

### Install with RTC (Fedora)

To become `rtc0` and set the system clock, the driver has to be in the
initramfs:

```sh
make
sudo modprobe -r applesmc # (if installed)
sudo make install
sudo dracut --force --add-drivers "t2smc" /boot/initramfs-$(uname -r).img $(uname -r)
```

Add to the kernel command line, so that neither `rtc_cmos`, the ACPI TAD RTC
nor applesmc gets there first:

```text
initcall_blacklist=cmos_init module_blacklist=acpi_tad,applesmc
```

The kernel log then shows:

```text
t2smc APP0001:00: registered as rtc0
t2smc APP0001:00: setting system clock ...
```

`sudo make uninstall` removes it again.

## Usage

The hwmon device is named `t2smc`. Its number changes across boots:

```sh
HWMON="$(dirname "$(grep -l '^t2smc$' /sys/class/hwmon/hwmon*/name)")"
```

## Sensors

Every float or fixed point key with one of these prefixes becomes a hwmon
channel, labeled with the SMC key:

| Prefix | hwmon files                   | Unit              |
|--------|-------------------------------|-------------------|
| `T`    | `tempN_label`, `tempN_input`  | millidegree C     |
| `I`    | `currN_label`, `currN_input`  | milliampere       |
| `V`    | `inN_label`, `inN_input`      | millivolt         |
| `P`    | `powerN_label`, `powerN_input`| microwatt         |

Integer keys are skipped because their unit differs between models. Channel
numbers depend on the machine, so match sensors by label:

```sh
paste "$HWMON"/power*_label "$HWMON"/power*_input
```

## Power telemetry

Battery and charger stay with the ACPI SBS drivers. The hwmon device adds
SMC telemetry without registering another battery:

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
smc_battery_time_to_empty_s
smc_battery_time_to_full_s
smc_battery_charge_current_ua
smc_battery_charge_voltage_uv
smc_battery_cell_voltage_max_uv
```

Battery current and power are positive while charging. Files whose key the
model lacks, and the time files while the battery idles, return no data.
Values are read on demand, without polling. Every ACPI SBS power-supply
notification increments `power_event_count`.

### Fan control

T2 Fan Control (`apps/t2-fan-control`) is the GUI. By hand:

```sh
# Read current fan speeds
cat "$HWMON/fan1_input"
cat "$HWMON/fan2_input"

# Set fan 1 to 3000 RPM (enters manual mode automatically)
echo 3000 | sudo tee "$HWMON/fan1_target"

# Hand fan 1 back to the SMC fan control
echo 0 | sudo tee "$HWMON/fan1_target"
```

`fanN_min` (writable) and `fanN_max` are the SMC limits, `fanN_label` is the
SMC fan name if there is one. Manual targets survive suspend.

### Raw SMC key access

`/dev/smc` (root only) reads and writes any SMC key through ioctls. The ABI
matches the macsmc patch `3014-mfd-macsmc-add-ioctl-to-access-smc-keys` from
t2linux/linux-t2-patches: `IOCTL_SMC_READ`, `IOCTL_SMC_WRITE`,
`IOCTL_SMC_GET_KEY_INDEX` and `IOCTL_SMC_GET_KEY_INFO` with magic `'S'`.
Keys for charging, clock, watchdog, SMC events and fan control cannot be
written this way.

### Battery charge limit

The limit uses the SMC key `CHLS` like macOS and appears on `BAT0` as
`charge_control_end_threshold`, so desktop power settings can set it. T2 SMC
Control (`apps/t2-smc-control`) does too:

```sh
echo 80 | sudo tee /sys/class/power_supply/BAT0/charge_control_end_threshold
```

1-100, 100 means no limit. The SMC keeps it across reboots and operating
systems. A leftover limit in the pre-T2 key `BCLM`, which macOS cannot lift
on T2 Macs, is cleared at probe.

A permanent limit keeps the gauge from recalibrating, so user space lifts it
for a full charge from time to time, based on the per-cell gauge data:

| Attribute | SMC keys | Meaning |
|---|---|---|
| `battery_charge_limit_suspended` | `CHLS` | 1 lifts the limit, the user limit stays |
| `smc_battery_qmax_mah` | `BQX1`.. | Qmax per cell in mAh |
| `smc_battery_dod0` | `BQD1`.. | DOD0 per cell |

### Temperature sensors

Common keys on T2 Macs:

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

### Watchdog

With `OSWD`, or `NATi` and `NATJ` on older firmware, `t2smc` registers a
watchdog for systemd's `RuntimeWatchdogSec=`. The SMC accepts at most 255
seconds, the watchdog core emulates longer timeouts. It pauses over sleep and
stops on reboot and poweroff.

### SMC events

With the SMC interrupt and the event port window available, the driver sets
`NTOK` and every SMC command sleeps until the KeyDone interrupt. Otherwise, or
when a KeyDone goes missing, it polls every millisecond and retries `NTOK` on
the next resume. The kernel log shows the active mode.

On imminent power loss (event `0x40`) the driver syncs filesystems and
flushes block devices. SMC log messages (event `0x4c`) appear as `SMC log:`,
a BridgeOS panic as a warning, thermal level changes only at debug level.

At load the driver logs the previous shutdown cause from `MSSD` and clears the
one-shot causes -64 and -62 like macOS. A T2 panic leaves `MSSD` unchanged,
so the raw x86 state keys `MSPP`, `MSPR`, `EFBS` and `MSPU` are logged as
well, as `previous x86 state:`.

`Apple` descriptions come from Apple's PowerManagement sources
(`common/CommonLib.c`, PowerManagement-1846). `community` descriptions come
from the Eclectic Light Company and George Garside lists and are unverified.
Apple wins every conflict, other values are logged as `unknown`.

| Cause | Source    | Meaning |
|-------|-----------|---------|
| 0 | Apple | Battery disconnected |
| 1 | Apple | Normal warm reset |
| 2 | Apple | Power supply disconnected |
| 3 | Apple | Power button pressed for > 4 sec |
| 5 | Apple | Software initiated shutdown |
| 7 | Apple | Normal shutdown by SOC |
| -3 | community | Multiple temperature sensors too high |
| -14 | community | Electricity spike or surge |
| -20 | community | BridgeOS (T2) initiated shutdown |
| -60 | Apple | Battery fully drained |
| -61 | community | Watchdog detected unresponsive app, shutting down |
| -62 | community | Watchdog detected unresponsive app, restarting |
| -64 | community | Kernel panic |
| -71 | community | Memory temperature too high |
| -74 | community | Battery temperature too high |
| -75 | community | Power adapter communication problem |
| -78 | community | Incorrect input current from power adapter |
| -79 | community | Incorrect current from battery |
| -81 | Apple | Thermal shutdown for overtemp |
| -86 | community | Proximity temperature too high |
| -100 | community | Power supply temperature too high |
| -101 | community | Display temperature too high |
| -102 | community | Overvoltage |
| -103 | community | Battery voltage too low |
| -104 | community | Unknown battery fault |
| -127 | community | PMU/SMC forced shutdown for another cause |

The thermal levels for CPU, IO and GPU are in `smc_thermal_level_cpu`,
`smc_thermal_level_io` and `smc_thermal_level_gpu`, which support `poll()`.

### Module parameters

| Parameter     | Default | Meaning                                         |
|---------------|---------|-------------------------------------------------|
| `wdt_timeout` | 60      | Watchdog timeout in seconds                     |
| `nowayout`    | kernel  | Watchdog cannot be stopped once started         |

### RTC

With the SMC RTC keys present, `t2smc` registers an RTC. Its number depends
on load order.

```sh
RTC="$(dirname "$(grep -l '^t2smc ' /sys/class/rtc/rtc*/name)")"
RTC_DEV="/dev/$(basename "$RTC")"

# Read the t2smc hardware clock
sudo hwclock --rtc "$RTC_DEV" --show

# Write the current system time to the t2smc hardware clock
sudo hwclock --rtc "$RTC_DEV" --systohc
```

Calendar time is `(counter + CLKO) / CLKR`. The counter comes from the `CLKL`
latch (write six `0xff` bytes, wait 20 ms, poll the 48-bit result) or from
`CLKM` where that is missing. `CLKR` defaults to 32768 Hz. Setting the RTC
writes `CLKO`, only when the offset changed by more than a quarter second.

Before sleep and at shutdown the driver syncs `CLKO` with the system clock,
since the T2 keeps its own time from the SMC RTC while the host is off.
Kait2en also sets the RTC once chrony has synchronized. The kernel's NTP sync
writes the CMOS clock instead and does not reach this RTC.

`tools/t2smc-rtc-client.c` reads the RTC at an interval to exercise `CLKL`.

## License

GPL-2.0-only.
