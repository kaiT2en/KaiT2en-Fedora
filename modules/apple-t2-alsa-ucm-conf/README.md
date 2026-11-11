# apple-t2-alsa-ucm-conf

This directory contains the ALSA UCM profile for the `snd-apple-t2` driver in
`modules/apple-t2-bridge/sound/pci/apple-t2`.

The `ucm2/` tree mirrors the `alsa-ucm-conf` upstream layout so the profile can
be prepared as a separate upstream submission from the kernel driver. The
driver names its card `AppleT2x2`, `AppleT2x4` or `AppleT2x6` after the number
of speaker channels, `ucm2/conf.d/` selects the profile by that name.
