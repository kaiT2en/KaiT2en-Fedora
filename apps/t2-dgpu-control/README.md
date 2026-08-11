# T2 GPU Control

T2 GPU Control configures hybrid graphics on T2 MacBook Pro models with Intel
and AMD graphics. The integrated GPU drives the display while PRIME offload
wakes the AMD GPU on demand. Runtime PM returns the AMD GPU to D3cold when it
is no longer in use.

The app requires the KaiT2en apple-gmux, AMDGPU, and HDA kernel patches. It
detects their runtime-PM support through vgaswitcheroo and does not fall back to
the older manual dGPU power-off and suspend workaround.

Installing this version disables and removes the former dGPU power-off,
suspend, and AMDGPU power-profile services.

The main KaiT2en installer invokes the app-specific installer automatically on
supported hardware. It can also be run directly from the repository:

```bash
sudo ./apps/t2-dgpu-control/install.sh
```
