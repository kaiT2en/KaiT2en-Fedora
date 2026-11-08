# T2 SMC Control

<img class="app-shot" src="img/apps/t2-smc-control.webp" alt="T2 SMC Control with battery, power telemetry and thermals">

Displays SMC temperatures, fan speeds, power data, and the hardware clock, and
can write the current system time to that clock. The values are read directly
from the SMC. GNOME and Plasma show their own battery estimates.

The battery charge limit is shown here and set in the desktop's power
settings. `t2smc` exposes it as the standard `charge_control_end_threshold`.

With a charge limit set, the background service `t2-smc-gaugingd` lifts it for
a full charge from time to time, like macOS does. A battery that never charges
to full keeps its gauge from recalibrating, and the reported charge can then
collapse under load.
