# T2 GPU Control

<img class="app-shot" src="img/blog/t2-gpu-control.jpg" alt="T2 GPU Control with primary GPU and power options">

For MacBook Pro models with Intel and AMD graphics other than the
MacBookPro15,1, 16,1 and 16,4, which use
[T2 Hybrid GPU Control](t2-hybrid-gpu-control.md).

- **Primary GPU**: Integrated or Discrete for the next boot.
- **Power off discrete GPU**: after an integrated boot, the dGPU is powered
  off to save idle power. It stays unavailable until the next reboot.
- **AMDGPU power-saving profile**: applies the driver power-saving profile
  after boot and resume while the dGPU stays on.

**GPU status** shows the active GPU, the configured boot GPU and the dGPU
state. **Apply Changes** stores the selection and asks for your password.
**Reboot** activates it. To go back, select **Discrete** and apply again.

The integrated GPU as primary lowers power use and makes suspend more
reliable. See [Configuring GPUs](../configuring-gpus.md).
