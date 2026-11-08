# T2 Zone Power

<img class="app-shot" src="img/apps/t2-zone-power.webp" alt="T2 Zone Power with zone power, applied limits and temperatures">

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
