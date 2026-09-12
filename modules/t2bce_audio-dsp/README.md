# Apple T2 Audio DSP

PipeWire/WirePlumber DSP graphs for the internal speakers of Apple T2 Macs
using t2bce_audio as a driver on KaiT2en Fedora.

The speaker graphs are generated from Apple's own speaker tunings. bridgeOS
carries the DSP graph and the tuning parameters that macOS applies to each
model (`/Library/Audio/Tunings/<J>/DSP`, used by `bridgeaudiod` for the T2's
own playback). The T2 does not apply them to audio coming from the host, so
KaiT2en applies them in PipeWire:

- `tunings/<J>.json`: the decoded tuning of each model (parameter sets of the
  Apple AudioUnits plus the wiring of the graph)
- `tools/dsplib.py`: the biquad designs of Apple's libAudioDSP (bell with
  Gunness Q, shelves, Butterworth, fractional order shelf, RBJ bands)
- `tools/gen_graph.py`: builds a filter-chain graph from a tuning: EQ and
  crossover as `bq_raw` biquads, the multiband compressor as lsp
  `mb_compressor_stereo`, the limiters as lsp `limiter_stereo`, crosstalk
  cancellation as a side channel FIR, virtual bass as bankstown
- `tools/xtc.py`: the crosstalk cancellation FIR
- `tools/tonemeister.py`: prints the EQ tables of a tuning

Apple's loudness boosters (UpComp, Norm, Mozart) are left out by default and
bankstown keeps the 40 to 150 Hz band of the earlier FIR profiles; both
sounded worse on a MacBookPro15,1. `gen_graph.py --help` lists the options.

Regenerate a profile with

```text
python3 tools/gen_graph.py J680 alsa_output.hw_Audio_0 profiles/15_1/graph.json
```

The profiles in `profiles/` are installed by `scripts/fedora/install-dsp.sh`
to `/usr/share/kait2en/audio-dsp/<profile>/`, which also writes the
WirePlumber configuration
`/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf`.

| Product | Profile | Apple tuning |
|---|---|---|
| MacBookAir8,1 | `8_1` | J140K |
| MacBookAir8,2 | `8_2` | J140a |
| MacBookAir9,1 | `9_1` | J230k |
| MacBookPro15,1 | `15_1` | J680 |
| MacBookPro15,2 | `15_2` | J132 |
| MacBookPro15,3 | `15_3` | J780 |
| MacBookPro15,4 | `15_4` | J213 |
| MacBookPro16,1 | `16_1` | J152f |
| MacBookPro16,2 | `16_2` | J214k |
| MacBookPro16,3 | `16_3` | J223 |
| MacBookPro16,4 | `16_4` | J215 |
| iMac20,1 | `imac20_1` | J185 |
| iMacPro1,1 | `imacpro1_1` | J137 |

Only the 15,1 profile has been listened to so far. The Mac Pro (J160) has a
mono wiring the generator does not handle yet.

The microphone graphs (`mic.json`) still originate from lemmyg's
`t2-apple-audio-dsp`; see `LICENSE`.

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
