# Apple T2 Audio DSP

PipeWire/WirePlumber DSP graphs and FIR files for Apple T2 audio.

The profiles in `firs/` are installed by `scripts/fedora/install-dsp.sh`.
Most FIR files originate from lemmyg's `t2-apple-audio-dsp`; the MacBook Pro
15,1 FIRs were generated from UMIK-1 measurements of that model by deqrocks.
The MacBook Air 8,1, 8,2 and 9,1 profiles use J313 FIRs from Asahi Linux's
[`asahi-audio`](https://github.com/AsahiLinux/asahi-audio) project. The MacBook
Pro 15,4, 16,2 and 16,3 profiles use its J293/J493 FIRs. The corresponding MIT
license is included with each profile.

Supported profiles:

- `MacBookAir8,1` -> `8_1`
- `MacBookAir8,2` -> `8_2`
- `MacBookAir9,1` -> `9_1`
- `MacBookPro15,1` -> `15_1`
- `MacBookPro15,4` -> `15_4`
- `MacBookPro16,1` -> `16_1`
- `MacBookPro16,2` -> `16_2`
- `MacBookPro16,3` -> `16_3`
- `MacBookPro16,4` -> `16_4`

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
