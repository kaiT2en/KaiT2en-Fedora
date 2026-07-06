# Apple T2 Audio Driver

This module contains the Apple T2 audio kernel driver.

The matching ALSA UCM profile lives next to it in `modules/t2bce_audio-alsa-ucm-conf`.
The driver and UCM profile should be upstreamed separately: this driver to the
Linux kernel, and the UCM profile to `alsa-ucm-conf`.
