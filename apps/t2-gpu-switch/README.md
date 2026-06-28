# T2 GPU Switch

T2 GPU Switch is a small GTK app for switching the active GPU through
`vga_switcheroo`.

The app shows the current GPU state on launch. Depending on the active GPU it
shows either `Switch to iGPU` or `Switch to dGPU`.

Switching briefly stops the display manager and starts it again after the
kernel switch request completed.
