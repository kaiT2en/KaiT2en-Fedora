# MacBookPro15,1 power-management baseline

Last verified: 2026-08-15

This is the known-good baseline to keep system suspend/resume correctness.

## Verified configuration

- Fedora kernel: `7.1.7-200.fc44.x86_64`
- In-tree `thunderbolt` driver, without the experimental NHI RTD3 quirk
- `t2thunderbolt` 0.5
- Native PCIe port services enabled with `pcie_ports=native`
- Immediate Titan Ridge xHCI downstream ports kept out of D3 by
  `t2thunderbolt`; the xHCI controllers may enter D3hot
- Thunderbolt PM ordering links created by `t2thunderbolt`

## Observed result

- Package idle state: PC3
- System suspend: working
- System resume: working
- EC interrupt unblock: about 1.06 seconds
- USB 3 hotplug and UAS: working in the tested configuration

The two Thunderbolt trees on this model are:

```text
00:01.1 -> 04:00.0 Alpine Ridge -> 05:00.0 Titan Ridge
                                      05:00.0 -> 06:00.0 NHI
                                      05:02.0 -> 07:00.0 xHCI

00:01.2 -> 7a:00.0 Alpine Ridge -> 7b:00.0 Titan Ridge
                                      7b:00.0 -> 7c:00.0 NHI
                                      7b:02.0 -> 7d:00.0 xHCI
```

With the known-good workaround, the direct Titan Ridge xHCI ports stay in D0
while the xHCI controllers runtime suspend into D3hot. The NHIs also stay
active in D0 because the in-tree driver does not enable RTD3 for this pre-USB4
root switch. Their upstream Titan Ridge, Alpine Ridge and CPU root ports
consequently remain active.

## Reproduced resume regression

Commit `ab90411` removed the xHCI and xHCI-port D3 restriction. The devices
then runtime-suspended successfully and USB 3 hotplug continued to work with
native PCIe port services, but system resume regressed: the interval from
`ACPI: PM: Waking up from system sleep state S3` to
`ACPI: EC: interrupt unblocked` increased from about one second to about 20
seconds. A subsequent isolation test allowed both xHCI controllers to enter
D3hot while keeping only their immediate downstream ports in D0. Resume
remained fast, UAS continued to work and the package remained in PC3. The
resume regression is therefore tied to D3 on the downstream ports, not D3hot
on the xHCI controllers.

This means USB hotplug and system resume are separate requirements. Successful
runtime suspend and hotplug do not prove that the system-suspend path is safe.

## Failed RTPC experiment

The direct xHCI ports are `DSB2` below `PEG1` and `PEG2` in the Apple ACPI
tables. Linux identifies itself through `_OSI("Darwin")`, so their `_PS3`
methods call `PCDA()`. That method only performs the platform link and GPIO
power-down sequence when `RUSB` is zero. Apple exposes `RTPC()` on each xHCI
device to update `RUSB`, but the standard xHCI PCI driver does not call it.

A test version set `RTPC(0)` for both Titan Ridge xHCI functions while the
quirk module was loaded. After resume, sysfs briefly reported both xHCI
functions and both direct ports as suspended in D3hot, but the ports later
returned to D0. This post-resume observation does not prove their state during
S3 or that `PCDA()` completed successfully.

The test failed all practical gates: EC interrupt unblocking took about 21
seconds, both xHCI controllers reported `USBSTS 0x401` and reinitialized, UAS
stopped working, and the package remained in PC3. Setting `RUSB` alone is
therefore not a usable replacement for keeping the direct xHCI ports in D0.

## Failed RTD3 experiment

The experimental Thunderbolt driver enabled runtime PM for the two Titan Ridge
NHIs. Both controllers entered D3cold and became inaccessible on resume. The
driver logged PCI power-state failures, active ring warnings, configuration
space timeouts and failed TMU restoration. Disabling D3cold during the early
NHI quirk setup did not persist; sysfs still reported `d3cold_allowed=1`.

Do not use this experiment as the PC7 starting point. The next attempt must
first establish and verify the complete PCI power policy before allowing the
NHI's initial autosuspend.

## Patch and upstream status

| Work | Repository state | Upstream state | Baseline relevance |
| --- | --- | --- | --- |
| Apple T2 Thunderbolt PM ordering links | `t2thunderbolt` | v7 not accepted; v8 preparing and untested | Required downstream until a revised series is merged and present in the supported kernel |
| Keep MacBookPro15,1 Titan Ridge xHCI port active | `t2thunderbolt` implementation | Withdrawn xHCI patch no longer matches the isolated requirement | Still required for fast resume; the xHCI controller itself may enter D3hot |
| Enable Titan Ridge NHI RTD3 | Removed experimental patch | Revoked / not viable | Not part of the baseline |
| Native PCIe port services | Kernel argument | Configuration, not an upstream patch | Required for tested USB 3 hotplug and UAS behavior |

## Gate for future PC7 work

Any PC7 candidate must retain all baseline behavior:

1. USB 3 hotplug and UAS work after idle and after resume.
2. System suspend completes.
3. EC interrupt unblock remains near the one-second baseline.
4. NHI and bridge power states are recorded for the complete trees, not only
   for the NHI and xHCI endpoints.
5. A failed runtime resume, inaccessible PCI function or Thunderbolt ring
   warning rejects the candidate even if PC7 was reached.
