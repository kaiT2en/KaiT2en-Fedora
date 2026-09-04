# KAIT2EN applications

KAIT2EN includes several desktop applications for monitoring and configuring
T2 Mac hardware. The installer selects hardware-specific applications where
necessary. The apps show up in the app drawer after installation.

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

## T2 CPU Control

Shows CPU frequency, temperature, package power, and throttling state. It can
configure PL1/PL2 power limits, Turbo Boost, maximum frequency, and the CPU
thermal target, and includes an automatic power-limit benchmark. It serves
the purpose of preventing prochot on T2 Macbooks.

## T2 Power Explorer

Presents the kernel device hierarchy together with runtime power-management
state and diagnostics. It helps identify devices that remain active and keep
the system from reaching deeper power-saving states.

## T2 Power Tune

Scans for available PCIe ASPM, runtime power-management, wakeup, LTR, and other
power-saving tunables. Selected changes can be tested temporarily or installed
as a persistent systemd service. Replaces powertop/tlp for reaching deeper
(pkg) c-states.

## T2 GPU Control

Used on supported MacBook Pro models with Intel and AMD graphics. It selects
the primary GPU for the next boot and can power down the unused discrete GPU
or enable AMDGPU's power-saving profile.

## T2 Hybrid GPU Control

Used on the MacBookPro15,1. It enables an iGPU-driven desktop with PRIME
offload to the AMD GPU, which wakes on demand and returns to D3cold when idle.
A discrete-GPU boot mode remains available as a recovery option.

## T2 Kernel Builder

Provides a graphical workflow for building customized Fedora kernels with the
required T2 configuration and selected patch groups. Completed builds can be
installed or removed through restricted privileged helpers.

## react-drm / Touch Bar Configurator

Replaces the standard Touch Bar interface with a configurable control center.
It provides function and media keys, brightness and volume controls,
application-aware actions, system information, and optional widgets.
Touchbar Configurator is a GUI app that can be used for in-depth customization.

