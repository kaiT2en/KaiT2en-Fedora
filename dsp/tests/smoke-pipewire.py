#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
"""Load every DSP graph in a private PipeWire daemon with no hardware monitor.

Requires installed runtime plugins and PipeWire tools. Never connects to the
user session, loads ALSA devices, restarts services or produces audible audio.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

source = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("dsp_build", source / "tools/build.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)

with tempfile.TemporaryDirectory(prefix="t2-dsp-smoke-") as temporary:
    root = Path(temporary)
    output = root / "share/t2-dsp"
    builder.build(output, str(root / "share"))
    modules = [{"name": f"libpipewire-module-{name}"} for name in (
        "protocol-native", "client-node", "adapter", "link-factory", "access")]
    expected = set()
    for path in sorted((output / "profiles").glob("*/*.json")):
        graph = json.loads(path.read_text())
        modules.append({"name": "libpipewire-module-filter-chain", "args": graph})
        expected.add(graph["capture.props"]["node.name"])
        expected.add(graph["playback.props"]["node.name"])
    config = {
        "context.properties": {"core.daemon": True, "core.name": "t2-dsp-test"},
        "context.spa-libs": {"audio.convert.*": "audioconvert/libspa-audioconvert",
                             "support.*": "support/libspa-support"},
        "context.modules": modules,
    }
    path = root / "pipewire.conf"
    path.write_text(json.dumps(config, indent=4))
    (root / "client.conf").write_text(json.dumps({
        "context.spa-libs": config["context.spa-libs"],
        "context.modules": [{"name": "libpipewire-module-protocol-native"}],
    }, indent=4))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(root), PIPEWIRE_RUNTIME_DIR=str(root),
               PIPEWIRE_CORE="t2-dsp-test", PIPEWIRE_REMOTE="t2-dsp-test", PIPEWIRE_CONFIG_DIR=str(root),
               PIPEWIRE_CONFIG_PREFIX="", XDG_CONFIG_HOME=str(root / "config"))
    with (root / "pipewire.log").open("w+") as log:
        daemon = subprocess.Popen(["pipewire", "-c", path.name], env=env, stdout=log, stderr=log)
        try:
            nodes = set()
            last_error = "Private PipeWire socket did not appear"
            for attempt in range(100):
                if daemon.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(log.read())
                if (root / "t2-dsp-test").exists():
                    result = subprocess.run(["pw-dump", "-r", "t2-dsp-test"], env=env,
                                            capture_output=True, text=True, timeout=5)
                    last_error = result.stderr
                    if result.returncode == 0:
                        nodes = {obj.get("info", {}).get("props", {}).get("node.name")
                                 for obj in json.loads(result.stdout)}
                        if expected <= nodes:
                            print(f"PASS: {len(modules) - 5} DSP graphs loaded, {len(expected)} nodes. No hardware opened.")
                            break
                time.sleep(0.1)
            else:
                log.seek(0)
                raise RuntimeError(f"Missing DSP nodes: {expected - nodes!r}\n" + last_error + log.read())
        finally:
            daemon.terminate()
            daemon.wait(timeout=5)
