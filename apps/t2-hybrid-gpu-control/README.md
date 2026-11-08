# T2 Hybrid GPU Control

T2 Hybrid GPU Control configures hybrid graphics on the MacBookPro15,1,
MacBookPro16,1 and MacBookPro16,4. The integrated GPU drives the display while
PRIME offload wakes the AMD GPU on demand. Runtime PM returns the AMD GPU to
D3cold when it is no longer in use, and suspend and resume work in this mode.

The app requires the KaiT2en apple-gmux, AMDGPU, and HDA kernel patches. It
detects their runtime-PM support through vgaswitcheroo and does not fall back to
the older manual dGPU power-off and suspend workaround.

macOS resets the boot GPU to the discrete GPU on every boot. Enabling hybrid
graphics therefore also installs t2-gpu-preboot as a UEFI boot entry. It is put
first in `BootOrder` only when the entry is created, so a boot manager such as
rEFInd placed ahead of it later stays the default. It runs before shim, and when the boot GPU no longer matches the configured
hybrid mode it rewrites gpu-power-prefs, shows a short notice and restarts once.
Otherwise it starts shim without delay. If the restart does not take effect, it
shows a warning and boots with the discrete GPU until hybrid graphics is
applied again. The UEFI image is built from `preboot/` and needs the
`rust-std-static-x86_64-unknown-uefi` package.

Installing this version disables and removes the former dGPU power-off,
suspend, and AMDGPU power-profile services.

## Installed files

| Path | Purpose |
|---|---|
| `/usr/local/bin/t2-hybrid-gpu-control` | GTK app |
| `/usr/local/libexec/t2-hybrid-gpu-control-helper` | privileged helper for the boot GPU, t2-gpu-preboot and reboot |
| `/usr/local/libexec/t2-hybrid-gpu-control-status` | reads the vga_switcheroo state |
| `/usr/local/share/applications/org.t2hybridgpucontrol.gtk.desktop` | desktop entry |
| `/usr/local/share/icons/hicolor/scalable/apps/org.t2hybridgpucontrol.gtk.svg` | icon |
| `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.policy` | polkit action for the helper |
| `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.status.policy` | polkit action for the status helper |
| `/etc/environment.d/90-kait2en-gsk-renderer.conf` | GTK renderer setting |
| `/boot/efi/EFI/kait2en/t2-gpu-preboot.efi` | t2-gpu-preboot UEFI image |
| UEFI boot entry `KaiT2en GPU` | starts t2-gpu-preboot, created first in `BootOrder` when hybrid graphics is enabled |
| EFI variable `boot-gpu-cf9aa22b-9505-4c88-aed0-5673e31b00c2` | wanted mode and restart counter for t2-gpu-preboot |

`make uninstall` removes the boot entry, the UEFI image and the EFI variable
together with the other files.

The main KaiT2en installer invokes the app-specific installer automatically on
supported hardware. It can also be run directly from the repository:

```bash
sudo ./apps/t2-hybrid-gpu-control/install.sh
```
