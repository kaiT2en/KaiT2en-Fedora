# SPDX-License-Identifier: GPL-3.0-or-later
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("dsp_build", SOURCE / "tools/build.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "build"
        builder.build(self.output, "/usr/share")
        self.conf = json.loads((self.output / "51-t2-dsp.conf").read_text())

    def test_all_profiles_and_short_unique_ids(self):
        profiles = list(builder.models().values())
        self.assertEqual(len(profiles), len(set(profiles)))
        self.assertEqual(set(profiles), {p.name for p in (SOURCE / "profiles").iterdir()})
        rules = (self.output / "89-t2-dsp.rules").read_text()
        self.assertNotIn("pci", rules)
        self.assertIn('ACTION=="add",', rules)
        self.assertNotIn('add|change', rules)
        self.assertIn('KERNELS=="t2bce_audio", GOTO="t2_dsp_model"', rules)
        for model, profile in builder.models().items():
            self.assertLessEqual(len(f"t2-{profile}"), 15)
            self.assertIn(f'RESULT=="{model}", ATTR{{id}}="t2-{profile}"', rules)
            self.assertTrue((self.output / "profiles" / profile / "README.md").is_file())

    def test_graph_processing_unchanged(self):
        for profile in builder.models().values():
            for source in (SOURCE / "profiles" / profile).glob("*.json"):
                expected = builder.relocate(json.loads(source.read_text()), profile, "/usr/share")
                props = "playback.props" if source.name == "graph.json" else "capture.props"
                expected[props]["target.object"] = (f"alsa_output.hw_t2-{profile}_0"
                    if source.name == "graph.json" else builder.mic_name(profile))
                if source.name == "graph.json":
                    expected[props]["node.dont-move"] = True
                actual = json.loads((self.output / "profiles" / profile / source.name).read_text())
                self.assertEqual(expected, actual)
                for asset in (SOURCE / "profiles" / profile).glob("*.wav"):
                    self.assertEqual(asset.read_bytes(), (self.output / "profiles" / profile / asset.name).read_bytes())

    def test_all_asset_references_resolve(self):
        def strings(value):
            if isinstance(value, str):
                yield value
            elif isinstance(value, dict):
                for child in value.values():
                    yield from strings(child)
            elif isinstance(value, list):
                for child in value:
                    yield from strings(child)
        for graph in (self.output / "profiles").glob("*/*.json"):
            for value in strings(json.loads(graph.read_text())):
                self.assertNotIn("/usr/share/t2-linux-audio", value)
                self.assertNotIn("/usr/share/t2linux-audio", value)
                self.assertNotIn("pci-", value)
                if value.startswith("/usr/share/t2-dsp/"):
                    self.assertTrue((self.output / value.removeprefix("/usr/share/t2-dsp/")).is_file(), value)

    def test_rules_exclude_headphones_headsets_and_split_parent(self):
        rules = self.conf["node.software-dsp.rules"]
        def matching(props):
            return [r for r in rules if any(all(props.get(k) == v for k, v in m.items()) for m in r["matches"])]
        for profile in builder.models().values():
            for stream, name in (("playback", "HiFi: Headphones: sink"),
                                 ("capture", "HiFi: Headset: source"),
                                 ("playback", None)):
                self.assertEqual([], matching({"alsa.id": f"t2-{profile}",
                    "api.alsa.pcm.stream": stream, "device.profile.name": name}))
            speakers = matching(builder.match(profile, "playback"))
            self.assertEqual(len(speakers), 1)
            self.assertFalse(speakers[0]["actions"]["create-filter"]["hide-parent"])
            mic = SOURCE / "profiles" / profile / "mic.json"
            self.assertEqual(len(matching(builder.match(profile, "capture"))), int(mic.exists()))
        self.assertEqual([], matching(builder.match("unknown", "playback")))
        self.assertEqual([], matching({"media.class": "Audio/Sink", "device.api": "dsp"}))

    def test_headphones_outrank_speaker_dsp_for_automatic_selection(self):
        for profile in builder.models().values():
            graph = json.loads((SOURCE / "profiles" / profile / "graph.json").read_text())
            self.assertEqual(graph["capture.props"]["priority.session"], 1400)
        matches = []
        for rule in self.conf["monitor.alsa.rules"]:
            if rule.get("actions", {}).get("update-props", {}).get("priority.session") == 1500:
                matches.extend(rule["matches"])
        self.assertEqual(matches, [{
            "alsa.id": "~t2-.*",
            "api.alsa.pcm.stream": "playback",
            "device.profile.name": "HiFi: Headphones: sink",
        }])

    def test_t2_default_output_uses_wireplumber_policy(self):
        components = self.conf["wireplumber.components"]
        self.assertEqual(components, [{
            "name": "t2-default-output.lua",
            "type": "script/lua",
            "provides": "hooks.t2-default-output",
        }])
        self.assertEqual(
            self.conf["wireplumber.profiles"]["main"]["hooks.t2-default-output"],
            "required")
        script = (SOURCE / "integration/wireplumber/t2-default-output.lua").read_text()
        self.assertNotIn("LocalModule", script)
        self.assertNotIn("libpipewire-module-loopback", script)
        self.assertIn('before = { "default-nodes/find-best-default-node" }', script)

    def test_build_reproducible_and_relocatable(self):
        other = self.root / "other"
        builder.build(other, "/usr/share")
        for path in self.output.rglob("*"):
            if path.is_file():
                self.assertEqual(path.read_bytes(), (other / path.relative_to(self.output)).read_bytes())
        builder.build(other, "/opt/t2/share")
        self.assertIn("/opt/t2/share/t2-dsp/", (other / "51-t2-dsp.conf").read_text())

    def write_legacy(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def migrate(self, extra_env=None):
        (self.root / "var/log").mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, T2_DSP_TEST_ROOT=str(self.root),
                   KAIT2EN_INSTALL_ERRORS=str(self.root / "errors"))
        env.update(extra_env or {})
        result = subprocess.run(["bash", str(SOURCE / "integration/libexec/package-actions"), "configure"],
                                env=env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def test_migration_is_recoverable_and_idempotent(self):
        old = self.write_legacy("etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf",
                                "# Generated by scripts/fedora/install-dsp.sh for MacBookPro15,1\nCUSTOM=1\n")
        data = old.read_bytes()
        self.migrate()
        self.assertFalse(old.exists())
        backups = list((self.root / "var/lib/kait2en/migration/t2-dsp").iterdir())
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), data)
        self.migrate()
        self.assertEqual(len(list(backups[0].parent.iterdir())), 1)

    def test_unknown_config_reported_other_steps_continue(self):
        old = self.write_legacy("etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf", "custom\n")
        quantum = self.write_legacy("etc/pipewire/pipewire.conf.d/50-kait2en-quantum.conf",
                                    "# Generated by scripts/fedora/install-dsp.sh\n")
        result = self.migrate()
        self.assertEqual(old.read_text(), "custom\n")
        self.assertFalse(quantum.exists())
        self.assertIn("completed with 1 errors", result.stdout)
        self.assertIn("unrecognized", (self.root / "errors").read_text())
        self.assertIn("unrecognized", (self.root / "var/log/t2-dsp-install.log").read_text())

    def test_failed_backup_does_not_remove_config(self):
        old = self.write_legacy("etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf",
                               "# Generated by scripts/fedora/install-dsp.sh for MacBookPro15,1\n")
        self.write_legacy("var/lib/kait2en/migration/t2-dsp", "not a directory")
        result = self.migrate()
        self.assertTrue(old.exists())
        self.assertIn("completed with", result.stdout)
        self.assertIn("Reboot to activate", result.stdout)

    def test_staging_no_activation(self):
        stage = self.root / "stage"
        subprocess.run(["make", "-C", str(SOURCE), "install", "PREFIX=/usr", f"DESTDIR={stage}"],
                       check=True, stdout=subprocess.DEVNULL)
        self.assertTrue((stage / "usr/lib/udev/rules.d/89-t2-dsp.rules").is_file())
        self.assertTrue((stage / "usr/share/wireplumber/scripts/t2-default-output.lua").is_file())
        self.assertTrue((stage / "usr/share/licenses/t2-dsp/GPL-3.0-or-later.txt").is_file())
        self.assertFalse((stage / "etc").exists())
        self.assertFalse((stage / "var").exists())


if __name__ == "__main__":
    unittest.main()
