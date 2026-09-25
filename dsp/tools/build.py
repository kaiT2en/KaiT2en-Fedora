#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build hardware-independent DSP data. Never probe or configure the build host."""
import argparse
import json
from pathlib import Path
import shutil

SOURCE = Path(__file__).resolve().parents[1]


def models():
    return json.loads((SOURCE / "config/models.json").read_text())


def mic_name(profile):
    return f"alsa_input.t2-{profile}.RawMic"


def match(profile, stream):
    return {
        "alsa.id": f"t2-{profile}",
        "api.alsa.pcm.stream": stream,
        "device.profile.name": "HiFi: Speaker: sink" if stream == "playback" else "HiFi: Mic: source",
    }


def relocate(value, profile, datadir):
    if isinstance(value, dict):
        return {k: relocate(v, profile, datadir) for k, v in value.items()}
    if isinstance(value, list):
        return [relocate(v, profile, datadir) for v in value]
    if isinstance(value, str):
        for root in ("/usr/share/t2-linux-audio", "/usr/share/t2linux-audio", "/usr/share/kait2en/audio-dsp"):
            value = value.replace(f"{root}/{profile}/", f"{datadir}/t2-dsp/profiles/{profile}/")
    return value


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=4) + "\n")


def build(output, datadir):
    monitor, filters = [], []
    # Match the driver ancestry, not a PCI address or bus topology. Use short
    # IDs because struct snd_card.id is only 16 bytes including the terminator.
    udev = [
        "# SPDX-License-Identifier: GPL-3.0-or-later",
        "# Based on sharpenedblade's PR #63. Generated from config/models.json.",
        'ACTION=="add", SUBSYSTEM=="sound", KERNEL=="card[0-9]*", KERNELS=="t2bce_audio", GOTO="t2_dsp_model"',
        'GOTO="t2_dsp_end"',
        'LABEL="t2_dsp_model"',
        'PROGRAM!="/usr/bin/cat /sys/class/dmi/id/product_name", GOTO="t2_dsp_end"',
    ]
    for product, profile in models().items():
        card_id = f"t2-{profile}"
        if len(card_id) > 15:
            raise ValueError(f"ALSA card ID too long: {card_id}")
        udev.append(f'RESULT=="{product}", ATTR{{id}}="{card_id}"')
        source = SOURCE / "profiles" / profile
        dest = output / "profiles" / profile
        dest.mkdir(parents=True, exist_ok=True)
        for asset in sorted(source.iterdir()):
            if asset.suffix != ".json":
                shutil.copyfile(asset, dest / asset.name)
        for filename, stream, props, target in (
            ("graph.json", "playback", "playback.props", f"alsa_output.hw_{card_id}_0"),
            ("mic.json", "capture", "capture.props", mic_name(profile)),
        ):
            path = source / filename
            if not path.exists() and filename == "mic.json":
                continue
            graph = relocate(json.loads(path.read_text()), profile, datadir)
            graph[props]["target.object"] = target
            if stream == "playback":
                # This is the DSP filter's internal stream, not an application
                # stream. WirePlumber's follow-default-target policy must move
                # clients between the DSP sink and headphones without moving
                # this stream away from its raw speaker backend.
                graph[props]["node.dont-move"] = True
            write_json(dest / filename, graph)
            filters.append({"matches": [match(profile, stream)], "actions": {
                "create-filter": {"filter-path": f"{datadir}/t2-dsp/profiles/{profile}/{filename}",
                                  "hide-parent": False}}})
            if stream == "capture":
                monitor.append({"matches": [match(profile, stream)], "actions": {
                    "update-props": {"node.name": target}}})
    udev.append('LABEL="t2_dsp_end"')
    (output / "89-t2-dsp.rules").write_text("\n".join(udev) + "\n")
    # Constrain the raw T2 speakers to the rates the hardware DSP path expects.
    # Losing this in the packaging refactor caused startup distortion. No rename
    # here, so the WirePlumber-generated names stay intact for UCM loopbacks.
    monitor.append({
        "matches": [{"alsa.id": "~t2-.*", "api.alsa.pcm.stream": "playback"}],
        "actions": {"update-props": {
            "audio.allowed-rates": [96000, 88200, 48000, 44100]}},
    })
    # The speaker DSP sink has priority.session=1400.  Keep the headphone
    # PCM as a separate node, but let it win automatic default selection while
    # its jack-backed UCM route is available.
    monitor.append({
        "matches": [{
            "alsa.id": "~t2-.*",
            "api.alsa.pcm.stream": "playback",
            "device.profile.name": "HiFi: Headphones: sink",
        }],
        "actions": {"update-props": {"priority.session": 1500}},
    })
    # Strict JSON is accepted as SPA-JSON. Speaker PCM split parents must keep
    # their WirePlumber-generated names so UCM loopbacks keep linking correctly.
    write_json(output / "51-t2-dsp.conf", {
        "monitor.alsa.rules": monitor,
        "node.software-dsp.rules": filters,
        "wireplumber.profiles": {"main": {
            "node.software-dsp": "required",
            "hooks.t2-default-output": "required",
        }},
        "wireplumber.components": [{
            "name": "t2-default-output.lua",
            "type": "script/lua",
            "provides": "hooks.t2-default-output",
        }],
    })


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=SOURCE / "build")
    parser.add_argument("--datadir", default="/usr/share")
    args = parser.parse_args()
    if not args.datadir.startswith("/"):
        parser.error("--datadir must be absolute")
    build(args.output, args.datadir.rstrip("/"))
