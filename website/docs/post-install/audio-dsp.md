# Audio DSP

This page explains the audio DSP support that KAIT2EN installs automatically.
It is not an installation guide. Supported Macs receive the matching profile
during the regular KAIT2EN installation; unsupported models continue to use
the native T2 audio devices.

## DSP in general

The T2 audio driver exposes the physical speaker channels, but the T2 does not
apply the model-specific processing to audio coming from the host; macOS does
that in coreaudiod. KAIT2EN provides that processing as a PipeWire filter
graph generated from Apple's own speaker tunings, which bridgeOS carries for
every T2 Mac.

Depending on the model, the graph

- distributes stereo audio across the physical woofer and tweeter channels
- applies Apple's equalization and crossovers as biquad filters
- applies Apple's multiband compressor and limiters
- widens the stereo image with Apple's crosstalk cancellation
- adds virtual-bass processing

The biquad chains are cheap compared with the FIR convolution used before.

On supported Macs, the resulting output appears as **DSP Speakers** device. The
unprocessed **Apple Internal Speakers** device remains available for comparison
and diagnostics. Headphones do not pass through the speaker DSP graph.

## Supported models

| Model | DSP profile |
| --- | --- |
| MacBook Air 2018 | `MacBookAir8,1` |
| MacBook Air 2019 | `MacBookAir8,2` |
| MacBook Air 2020 (Intel) | `MacBookAir9,1` |
| MacBook Pro 15-inch 2018/2019 | `MacBookPro15,1` |
| MacBook Pro 13-inch 2018/2019, four Thunderbolt ports | `MacBookPro15,2` |
| MacBook Pro 15-inch 2018/2019, Radeon Pro Vega | `MacBookPro15,3` |
| MacBook Pro 13-inch 2019, two Thunderbolt ports | `MacBookPro15,4` |
| MacBook Pro 16-inch 2019 | `MacBookPro16,1` |
| MacBook Pro 13-inch 2020, four Thunderbolt ports | `MacBookPro16,2` |
| MacBook Pro 13-inch 2020, two Thunderbolt ports | `MacBookPro16,3` |
| MacBook Pro 16-inch 2019 | `MacBookPro16,4` |
| iMac 27-inch 2020 | `iMac20,1` |
| iMac Pro 2017 | `iMacPro1,1` |

The installer identifies the model from DMI and only deploys a graph when an
explicit matching profile exists. KAIT2EN does not reuse a profile on an
unlisted model.

## Automatic selection

The WirePlumber software-DSP integration gives the DSP sink a higher session
priority than the underlying speaker sink. WirePlumber should therefore select
the DSP output automatically when the profile is first created.

## Profile origins and support

The speaker graphs are generated from the speaker tunings that bridgeOS ships
for each T2 Mac, translated to PipeWire filter-chain nodes; the filter designs
follow Apple's libAudioDSP. Apple's loudness boosters are deliberately left
out. Earlier KAIT2EN versions used measured FIR filters from
[lemmyg's t2-apple-audio-dsp](https://github.com/lemmyg/t2-apple-audio-dsp)
and [Asahi Linux](https://github.com/AsahiLinux/asahi-audio); the microphone
graphs still come from lemmyg's project.

Problems with these profiles must be reported to the
[KAIT2EN issue tracker](https://github.com/kaiT2en/KaiT2en-Fedora/issues).
Do not report KAIT2EN DSP behavior to lemmyg's project: its current routing,
filter graphs and UCM integration are maintained here.

The generator, the decoded tunings and the license information live in the
DSP module of the repository.

## Check the active output

The sound settings should show the model-specific DSP output on a supported
Mac. PipeWire can also list it directly:

```bash
wpctl status
```

The active sink is marked with an asterisk. If the DSP device is missing after
the regular installation and a reboot, include the model identifier and this
output in a KAIT2EN issue:

```bash
cat /sys/class/dmi/id/product_name
wpctl status
journalctl --user -b -u wireplumber -u pipewire -u pipewire-pulse
```
