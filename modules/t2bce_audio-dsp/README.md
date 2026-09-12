# Apple T2 Audio DSP

PipeWire/WirePlumber DSP graphs for the internal speakers of Apple T2 Macs
using t2bce_audio as a driver on KaiT2en Fedora.

The speaker graphs reproduce the model-specific processing that macOS applies
to the internal speakers: equalization and crossovers as `bq_raw` biquads,
the multiband compressor as lsp `mb_compressor_stereo`, the limiters as lsp
`limiter_stereo`, crosstalk cancellation as a side channel FIR
(`xtc-side-*.wav`) and virtual bass as bankstown. The graphs are generated;
edit them only through the KaiT2en maintainers.

The profiles in `profiles/` are installed by `scripts/fedora/install-dsp.sh`
to `/usr/share/kait2en/audio-dsp/<profile>/`, which also writes the
WirePlumber configuration
`/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf`.

| Product | Profile |
|---|---|
| MacBookAir8,1 | `8_1` |
| MacBookAir8,2 | `8_2` |
| MacBookAir9,1 | `9_1` |
| MacBookPro15,1 | `15_1` |
| MacBookPro15,2 | `15_2` |
| MacBookPro15,3 | `15_3` |
| MacBookPro15,4 | `15_4` |
| MacBookPro16,1 | `16_1` |
| MacBookPro16,2 | `16_2` |
| MacBookPro16,3 | `16_3` |
| MacBookPro16,4 | `16_4` |
| iMac20,1 | `imac20_1` |
| iMacPro1,1 | `imacpro1_1` |


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
