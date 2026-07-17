# Apple T2 Audio DSP

PipeWire/WirePlumber DSP graphs and FIR files for Apple T2 audio.

The profiles in `firs/` are installed by `scripts/fedora/install-dsp.sh`.
Most FIR files originate from lemmyg's `t2-apple-audio-dsp`; the MacBook Pro
15,1 FIRs were generated from UMIK-1 measurements of that model by deqrocks.
The MacBook Pro 16,2 speaker graph and FIRs are adapted from Asahi Linux's
MIT-licensed J293 profile; its license is included with that profile.

Supported profiles:

- `MacBookPro16,1` -> `16_1`
- `MacBookPro16,2` -> `16_2`
- `MacBookPro16,4` -> `16_4`
- `MacBookAir9,1` -> `9_1`
- `MacBookPro15,1` -> `15_1`

The installer copies the matching files to:

```text
/usr/share/kait2en/audio-dsp/<profile>/
```

It generates a WirePlumber configuration at:

```text
/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf
```

The graph target is rewritten at install time to match the detected Apple T2
audio PCI device and KaiT2en UCM sink/source names.

Required Fedora packages are installed by `install-dsp.sh`, not by the common
dependency installer:

- `pipewire`
- `pipewire-pulseaudio`
- `wireplumber`
- `pipewire-module-filter-chain-lv2`
- `lv2-bankstown`
- `lv2-triforce`
- `lsp-plugins-lv2`
- `lv2-swh-plugins`

License: see `LICENSE`.
