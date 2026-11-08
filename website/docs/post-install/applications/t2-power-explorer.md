# T2 Power Explorer

<img class="app-shot" src="img/apps/t2-power-explorer.webp" alt="T2 Power Explorer with the PCI device tree and runtime power states">

Shows the kernel device tree with the runtime power state of every device,
grouped into PCI, platform and virtual devices. It helps find devices that keep
the system out of deeper package C-states.

Each device has an assessment:

- **runtime PM failed**: the driver reported a runtime PM error.
- **active leaf device under automatic runtime PM**: the device may suspend
  but stays active. This is the usual candidate for a blocked C-state.
- **runtime PM disabled by policy**: `power/control` is `on`. It can be
  enabled in [T2 Power Tune](t2-power-tune.md).
- **runtime PM disabled on a parent or supplier**: the same on a device with
  children, which keeps them active too.

A parent marked "A descendant contains a power-management issue" has a child
with one of these findings. Selecting a device shows its driver, runtime PM
counters, PCI power states, wake support, link speed and width and device
links.
