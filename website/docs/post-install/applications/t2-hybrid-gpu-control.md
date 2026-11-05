# T2 Hybrid GPU Control

Used on the MacBookPro15,1, MacBookPro16,1 and MacBookPro16,4. It enables an
iGPU-driven desktop with PRIME offload to the AMD GPU, which wakes on demand
and returns to D3cold when idle. Suspend and resume work in this mode. A
discrete-GPU boot mode remains available as a recovery option.

macOS sets the boot GPU back to the discrete GPU every time it starts. The app
therefore installs a small UEFI program, t2-gpu-preboot, that runs before
Linux. When hybrid graphics is enabled and the boot GPU has been changed, it
restores the integrated GPU, shows a short notice and restarts once. Otherwise
it starts Linux without delay. If the restart does not take effect, it shows a
warning and Linux starts on the discrete GPU until hybrid graphics is applied
again in the app.
