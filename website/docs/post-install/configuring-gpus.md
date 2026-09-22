# How to configure GPUs

If a Mac has a dGPU, it will use it for boot and it will also use it as primary
display adapter by default. An iMac is no exception in that aspect, but it is
not able to switch between internal and dedicated GPU because the display lines
from iGPU to display are missing. So on iMacs, the iGPU is only used for offloading.
Thus, if you are an iMac user, this guide is not for you.
Same for Mac Pro users, since Mac Pros have no iGPU.
This guide is only for Macbook Pro users.

## MacBookPro15,1, MacBookPro16,1 and MacBookPro16,4: enable hybrid graphics

KaiT2en installs **T2 Hybrid GPU Control** on the MacBookPro15,1,
MacBookPro16,1 and MacBookPro16,4. Open it from the application menu and enable
**Hybrid graphics**, then reboot.

Hybrid graphics makes the integrated GPU the display GPU. Applications can
still use the AMD GPU through PRIME offload. The kernel wakes it automatically
for accelerated work and returns it to D3cold when it becomes idle. This keeps
the dGPU available without paying its idle power cost. System suspend and
resume are fully supported in hybrid mode on all three models.

The installer builds the required AMDGPU and HDA modules for the current Fedora
kernel. The app reports whether the required runtime-PM support is active.

These two modules are not managed by DKMS. After Fedora installs a new kernel,
first reboot into that kernel and then rebuild its hybrid-graphics modules:

```bash
cd /usr/local/src/KaiT2en-Fedora
sudo ./scripts/fedora/install-gpu-runtime-pm.sh
sudo reboot
```

Running the script before booting the new kernel only rebuilds the modules for
the old, currently running kernel. Until the rebuild and second reboot are
complete, hybrid runtime PM is not available on the new kernel.

The discrete-GPU boot option remains available as a recovery setting. Rebooting
is always a separate action so changing the stored boot GPU does not restart the
system unexpectedly.

## Other MacBooks with dGPU

Other Intel/AMD MacBook Pro models use **T2 GPU Control**. Hybrid runtime PM is
not enabled on those models because their dGPU power-on path is not yet
reliable. **T2 GPU Control** has options to switch between iGPU and
dGPU and to enable power saving for the dGPU. For the changes to take effect
you need to reboot.
Usually, users prefer iGPU as primary to save some energy and make suspend
more reliable.
