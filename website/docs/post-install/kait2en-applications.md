# KAIT2EN applications

KAIT2EN includes several applications for monitoring and configuring T2 Mac
hardware. The installer selects hardware-specific applications where
necessary. Graphical apps show up in the app drawer after installation. T2
Journal is used from a terminal. Their names always start with "T2", which
makes them easy to find.

## T2 Fan Control

Monitors temperatures and fan speeds and provides an editable fan curve. A
background service keeps automatic fan control active across login, suspend,
resume, and reboot. It also features an adjustable "system-chill wall" that
sets the fans to 100% to prevent case heating and in effect prochot.

## T2 SMC Control

Displays SMC temperatures, fan speeds, power data, and the hardware clock, and
can write the current system time to that clock. The battery charge limit is
shown but not set here: `t2smc` exposes it as the standard
`charge_control_end_threshold`, so the desktop environment offers it in its own
power settings. Note that this data shown in this app is direct hardware readings. It is
the single source of truth for temperatures and battery statistics. 
Desktop environment's readings of battery charge are only estimations.
As we promised to ship a vanilla Fedora with KaiT2en sugar on top, we did not
manipulate it to show SMC values in Gnome/Plasma. This will later be solved
when upstreaming SMC. 

With a charge limit set, the background service `t2-smc-gaugingd` lifts it for
a full charge from time to time, like macOS does. A battery that never charges
to full keeps its gauge from recalibrating, and the reported charge can then
collapse under load.

## T2 Zone Power

Applies the power limits of the SMC the way macOS does. The SMC shares the
cooling budget between CPU, integrated GPU and discrete GPU by itself and
asks the operating system to lower each of them in steps. Linux used to
ignore these requests, so the SMC kept escalating until it throttled the CPU
hard through prochot.

A background service follows the SMC steps without any settings. It caps the
CPU clock, the integrated GPU clock and the power of the discrete GPU, and
uses forced idle when the SMC asks for it. On a combined CPU and GPU load the
SMC then settles on steady clocks instead of dropping to 800 MHz.

The app itself only shows what happens: zone power against its target, zone
throttling, the applied limits and the CPU, GPU and heatpipe temperatures.
T2 Zone Power replaces T2 CPU Control, which the installer removes.

## T2 Power Explorer

Presents the kernel device hierarchy together with runtime power-management
state and diagnostics. It helps identify devices that remain active and keep
the system from reaching deeper power-saving states. It also helps to get
an idea of the platform architecture. Similar to the Windows Device Manager,
it shows the devices in a collapsible tree view.

## T2 Journal

Fetches Apple T2 bridgeOS logs over the internal `Apple T2 Bridge` network
link and merges them chronologically with the Linux journal. It can select one
boot or all retained boots, filter by regular expression or source, and emit
text or JSONL. The first query downloads a BridgeOS snapshot automatically; use
`t2journal refresh` to replace it explicitly.

The link is set up by the installer and managed by `kait2en-t2-remote`, the
same service that holds the T2 video encoder open. It is disconnected before
suspend and reconnected after resume, so there is nothing to configure.

Typical queries are:

```bash
t2journal -b
t2journal -b -1 --grep 'suspend|watchdog'
t2journal --allboots --source t2
t2journal -b --output jsonl > merged.jsonl
```

`t2journal refresh --sysdiagnose` additionally keeps a copy of the downloaded
sysdiagnose archive in the current directory, e.g. to extract panic logs from
it yourself.

Run `t2journal --help` for all filtering and refresh options.

## T2 Debug

A live window into the T2's own operating system (bridgeOS), over the internal
`Apple T2 Bridge` link. It talks to the T2's built-in Instruments diagnostic
service and is read-only: it observes, it changes nothing.

Two tabs:

- **deviceinfo** is a "top" for the T2: every bridgeOS process with its CPU,
  power score, interrupt wakeups, memory footprint and disk writes, updated once
  a second. Click a column header to sort. Buttons run one-shot queries (mach
  time, network info, the kdebug trace-code table, the full service capability
  list) and list a bridgeOS directory.
- **live log** streams the T2's `os_log` in real time, decoded to readable
  lines and coloured by level like `dmesg`. A regex box filters, a "hide HID"
  toggle drops the input-device chatter, and the subsystem list on the right
  lets you tick exactly which subsystems to show.

This is the tool for watching what the T2 does in the moment, for example the
SMC and power events around a GPU switch or a suspend. Logs the T2 marks as
private appear as `<private>`: that redaction happens inside bridgeOS and cannot
be lifted from Linux.

A headless command, `t2debug`, exposes the same data without a GUI, which is
handy over SSH (`t2debug procs`, `t2debug logs --no-hid`,
`t2debug sysmon`). Run `t2debug --help` for all subcommands.

## T2 Power Tune

Scans for available PCIe ASPM, runtime power-management, wakeup, LTR, and other
power-saving tunables. Selected changes can be tested temporarily or installed
as a persistent systemd service. Replaces powertop/tlp for reaching deeper
(pkg) c-states. Do not change anything without reason. Enabling more options
does not mean more savings. It is more likely to cause regressions.

## T2 Force Click

Configures the Force Touch trackpad's normal-click pressure and its harder
Force Click threshold. Force Click is available as a separate event, so it can
be bound without changing normal clicks, tap-to-click, scrolling, or gestures.
Those remain libinput's job.

One action can be selected for a Force Click. Alternating copy/paste, a
recorded keyboard shortcut, or an advanced shell command. A physical
three-finger click already produces a middle click directly from the trackpad,
so it does not need a Force Click binding.

## T2 Touch Bar

Optional. The installer only asks on models with a Touch Bar. Without it,
Apple's native Touch Bar keeps working.

Keeps the Touch Bar dark until it is touched or Fn is pressed.

- Offers a media row, an F-key row and a row with print, insert, delete,
  home, end and page keys.
- Two-finger swipes change the volume on the dark bar.
- Two-finger swipes change the row on a lit bar.
- Three-finger swipes change the display brightness on a dark bar.
- Touch ID shows an "Unlock with Touch ID" prompt.
- Tracks from media players briefly appear on the dark bar.
- Remembers the last active row.
- Holding Fn switches between the media row and the last remembered row.
- Up to four personal keys can be configured in
  `~/.config/kait2en-touchbar/keys.toml`.
- App profiles in `~/.config/kait2en-touchbar/apps.toml` add a row of keys
  per app. It appears while that app has the focus. This needs the
  "Focused Window D-Bus" GNOME Shell extension. The Touch Bar Settings app
  edits them without touching the file.

Reboot once after the first install.

Settings are in `/etc/kait2en/touchbar.toml`. To return to the native Touch
Bar, run `sudo systemctl --global disable kait2en-touchbar.service` and
reboot, or remove it with
`sudo /usr/local/src/KaiT2en-Fedora/apps/t2-touchbar/uninstall.sh`.

## T2 Hybrid GPU Control

Used on the MacBookPro15,1, MacBookPro16,1 and MacBookPro16,4. It enables an
iGPU-driven desktop with PRIME offload to the AMD GPU, which wakes on demand
and returns to D3cold when idle. Suspend and resume work in this mode. A
discrete-GPU boot mode remains available as a recovery option.

## T2 GPU Control

Used on supported MacBook Pro models with Intel and AMD graphics. It selects
the primary GPU for the next boot and can power down the unused discrete GPU
or enable AMDGPU's power-saving profile.

## T2 Kernel Builder

Provides a graphical workflow for building customized Fedora kernels with the
required T2 configuration and selected patch groups. Completed builds can be
installed or removed through restricted privileged helpers.
