# Audio DSP

KAIT2EN installs the audio DSP automatically, with the profiles for all
supported models. Other models keep the native T2 audio devices.

## DSP in general

The T2 audio driver exposes the physical speaker channels. The model-specific
processing runs on the host, under macOS as well. KAIT2EN provides that processing as a PipeWire
filter graph per model.

Depending on the model, the graph

- distributes stereo audio across the physical woofer and tweeter channels
- applies the model's equalization and crossovers as biquad filters
- applies a multiband compressor and limiters
- widens the stereo image with crosstalk cancellation
- adds virtual-bass processing

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

The installation reads the model from DMI and writes the matching graph to
`/etc/pipewire/pipewire.conf.d/51-t2-dsp.conf`. At boot udev assigns a short
ALSA card ID, for example `t2-15_1`, and WirePlumber matches that ID and the
UCM Speaker/Mic nodes. Reboot after installing or removing `t2-dsp`.

## Automatic selection

The DSP sink has session priority 1400, above the raw speaker sink, and the
headphone sink 1500. While a T2 output is the default, the WirePlumber script
`t2-default-output.lua` selects the headphones when they are plugged in and the
DSP speakers otherwise. A USB, HDMI or Bluetooth output chosen by the user stays
selected.

## Profile origins and support

The graphs are generated per model by KAIT2EN. License information is in
`/usr/share/licenses/t2-dsp/`. Thanks to
[lemmyg](https://github.com/lemmyg/t2-apple-audio-dsp), whose PipeWire DSP
graphs for T2 Macs were the starting point. Report problems to the
[KAIT2EN issue tracker](https://github.com/kaiT2en/KaiT2en-Fedora/issues).

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
