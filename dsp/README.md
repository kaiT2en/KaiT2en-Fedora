# Apple T2 Audio DSP

PipeWire/WirePlumber DSP graphs for the internal speakers and microphones of
Apple T2 Macs using the snd-apple-t2 driver and the AppleT2 UCM profiles from
`modules/apple-t2-alsa-ucm-conf/ucm2/`. This is host-side processing.

The speaker graphs reproduce the model-specific processing that macOS applies
to the internal speakers: equalization and crossovers as `bq_raw` biquads,
the multiband compressor as lsp `mb_compressor_stereo`, the limiters as lsp
`limiter_stereo`, crosstalk cancellation as a side channel FIR
(`xtc-side-*.wav` or `XTC-side-*.wav`) and virtual bass as bankstown. The
graphs are generated. Changes go through the KaiT2en maintainers.

The `t2-dsp` RPM/Debian package and `scripts/fedora/install-dsp.sh` install
all profiles to `/usr/share/t2-dsp/profiles/<profile>/`. At install time the
package scripts read the DMI product name and copy the matching
`/usr/share/t2-dsp/pipewire/<profile>.conf` to
`/etc/pipewire/pipewire.conf.d/51-t2-dsp.conf`. At boot udev sets a short ALSA
card ID from the same DMI name, for example `t2-15_1`, and WirePlumber matches
that ID and the internal Speaker/Mic nodes. Headphones and headset
microphones keep their raw path. Unlisted models keep their default card name
and run without DSP.

Speaker graphs target the UCM split-PCM hardware node and keep its name, so
the UCM loopbacks stay valid. The internal microphone node gets a stable
model-specific name. Raw devices remain available.

Layout: `profiles/` owns DSP data, `config/` the model mapping,
`integration/` runtime configuration and migration, `tools/` the build-only
generator, `tests/` validation and `packaging/` RPM/Debian recipes. See
[packaging/README.md](packaging/README.md) for builds, dependencies,
migration and licenses.

| Product | Profile | Microphone graph |
|---|---|---|
| MacBookAir8,1 | `8_1` | yes |
| MacBookAir8,2 | `8_2` | yes |
| MacBookAir9,1 | `9_1` | yes |
| MacBookPro15,1 | `15_1` | yes |
| MacBookPro15,2 | `15_2` | yes |
| MacBookPro15,3 | `15_3` | yes |
| MacBookPro15,4 | `15_4` | yes |
| MacBookPro16,1 | `16_1` | yes |
| MacBookPro16,2 | `16_2` | yes |
| MacBookPro16,3 | `16_3` | yes |
| MacBookPro16,4 | `16_4` | yes |
| iMac20,1 | `imac20_1` | no |
| iMacPro1,1 | `imacpro1_1` | no |

Reboot after installation. Package scripts and `install-dsp.sh` leave running
user audio services and live card names as they are.

## Copyright

Copyright (C) 2026 André Eikmeyer, KAIT2EN. GPL-3.0-or-later with the
attribution term of `LICENSING.md`.

## Thanks

sharpenedblade contributed the udev/WirePlumber runtime-selection approach in
merged PR #63. This package adapts it to short ALSA IDs, current model coverage
and the UCM split-PCM layout.

lemmyg's [t2-apple-audio-dsp](https://github.com/lemmyg/t2-apple-audio-dsp)
did the pioneering work: the first PipeWire DSP graphs for T2 Macs, and the
template this module started from.
