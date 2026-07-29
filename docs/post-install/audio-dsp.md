# Audio DSP

This page explains the audio DSP support that KAIT2EN installs automatically.
It is not an installation guide. Supported Macs receive the matching profile
during the regular KAIT2EN installation; unsupported models continue to use
the native T2 audio devices.

## DSP in general

The T2 audio driver exposes the physical speaker channels, but it does not
contain the model-specific processing Apple applies before those channels reach
the internal speakers. KAIT2EN provides that processing as a PipeWire filter
graph.

Depending on the model, the graph

- distributes stereo audio across the physical woofer and tweeter channels
- applies model-specific FIR filters to correct the frequency response;
- adds virtual-bass processing where appropriate and controls peaks before
they reach the internal speakers.

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
| MacBook Pro 13-inch 2019, two Thunderbolt ports | `MacBookPro15,4` |
| MacBook Pro 16-inch 2019 | `MacBookPro16,1` |
| MacBook Pro 13-inch 2020, four Thunderbolt ports | `MacBookPro16,2` |
| MacBook Pro 13-inch 2020, two Thunderbolt ports | `MacBookPro16,3` |
| MacBook Pro 16-inch 2019 | `MacBookPro16,4` |

The installer identifies the model from DMI and only deploys a graph when an
explicit matching profile exists. KAIT2EN does not reuse a profile on an
unlisted model.

## Automatic selection

The WirePlumber software-DSP integration gives the DSP sink a higher session
priority than the underlying speaker sink. WirePlumber should therefore select
the DSP output automatically when the profile is first created.

## Profile origins and support

The KAIT2EN DSP implementation originated from
[lemmyg's t2-apple-audio-dsp](https://github.com/lemmyg/t2-apple-audio-dsp),
but it is no longer a direct copy of that project. Its graphs have been
substantially reworked, adapted to the KAIT2EN UCM profiles and WirePlumber
software-DSP integration, and extended with model-specific measurements and
FIRs from [Asahi Linux](https://github.com/AsahiLinux/asahi-audio) or our own.

Problems with these profiles must be reported to the
[KAIT2EN issue tracker](https://github.com/kaiT2en/KaiT2en-Fedora/issues).
Do not report KAIT2EN DSP behavior to lemmyg's project: its current routing,
filter graphs and UCM integration are maintained here.

The exact source and license information for each profile is recorded in the
DSP module and alongside imported FIR files.

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
