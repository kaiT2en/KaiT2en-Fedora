# How to configure GPUs

If a Mac has a dGPU, it will use it for boot and it will also use it as primary
display adapter by default. An iMac is no exception in that aspect, but it is
not able to switch between internal and dedicated GPU because the display lines
from iGPU to display are missing. So on iMacs, the iGPU is only used for offloading.
Thus, if you are an iMac user, this guide is not for you.
Same for Mac Pro users, since Mac Pros have no iGPU.
This guide is only for Macbook Pro users.

## Enable hybrid graphics

KaiT2en installs **T2 GPU Control** on MacBook Pro models that have both Intel
and AMD graphics. Open it from the application menu and enable **Hybrid
graphics**.

Hybrid graphics makes the integrated GPU the display GPU. Applications can
still use the AMD GPU through PRIME offload. The kernel wakes it automatically
for accelerated work and returns it to D3cold when it becomes idle. This keeps
the dGPU available without paying its idle power cost.

This mode requires the apple-gmux, AMDGPU, and HDA patches in the repository's
`patches` directory. The app reports whether the required runtime-PM support is
active and will not use the older manual power-off workaround when it is
missing.

The discrete-GPU boot option remains available as a recovery setting. Rebooting
is always a separate action so changing the stored boot GPU does not restart the
system unexpectedly.

## MacBook Pro 15,1 A1990 dGPU suspend issues

The AMDGPU driver cannot reliably resume the Radeon GPU in
the MacBookPro15,1. Suspend can therefore fail or resume to a black screen,
regardless of the configuration selected in T2 GPU Control.

Our upstream fix has been accepted and will be released with Linux 7.3.
Until then you can patch AMDGPU yourself using the patch in the patches
folder.
