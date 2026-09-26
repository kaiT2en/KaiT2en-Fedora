# SPDX-License-Identifier: GPL-3.0-or-later
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("lifecycle", Path(__file__).with_name("kait2en-lifecycle.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def put(self, name, data, mode=0o644):
        path = self.root / name.lstrip("/")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        path.chmod(mode)
        return path

    def engine(self, name="t2-touchid"):
        return module.Lifecycle(name, str(self.root))

    def touchid(self):
        self.put("/usr/bin/t2-touchid", b"\x7fELFt2-touchid new", 0o755)
        self.put("/usr/local/bin/t2-touchid", b"\x7fELFt2-touchid old", 0o755)
        self.put("/usr/lib/systemd/system/kait2en-t2-touchid.service", "ExecStart=/usr/bin/t2-touchid\n")
        self.put("/etc/systemd/system/kait2en-t2-touchid.service", "ExecStart=/usr/local/bin/t2-touchid\n")
        self.put("/usr/lib/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf", "[Unit]\nRequires=kait2en-t2-touchid.service\n")
        self.put("/etc/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf", "[Unit]\nRequires=kait2en-t2-touchid.service\n")

    def test_source_migrate_repeat_remove(self):
        self.touchid()
        engine = self.engine()
        engine.record_source()
        engine.migrate_services()
        self.assertEqual(engine.errors, [])
        self.assertFalse((self.root / "usr/local/bin/t2-touchid").exists())
        self.assertFalse((self.root / "etc/systemd/system/kait2en-t2-touchid.service").exists())
        self.assertEqual(len(engine.state["retired"]), 3)
        engine = self.engine()
        engine.migrate_services()
        self.assertEqual(len(engine.state["retired"]), 3)
        engine.remove()
        self.assertEqual(engine.errors, [])
        self.assertFalse((self.root / "var/lib/kait2en/migration/t2-touchid").exists())
        self.assertFalse(engine.state_path.exists())
        self.assertTrue((self.root / "usr/bin/t2-touchid").exists())  # owned by the package manager

    def test_unknown_override_and_modified_backup_preserved(self):
        self.touchid()
        custom = self.put("/etc/systemd/system/kait2en-t2-touchid.service", "CUSTOM ADMIN UNIT\n")
        engine = self.engine()
        engine.migrate_services()
        self.assertTrue(custom.exists())
        self.assertTrue(engine.errors)
        self.assertTrue((self.root / "usr/local/bin/t2-touchid").exists())
        # With the override resolved, migration can safely retire the binary.
        custom.write_text("ExecStart=/usr/local/bin/t2-touchid\n")
        engine = self.engine()
        engine.migrate_services()
        name = next(iter(engine.state["retired"]))
        (self.root / name.lstrip("/")).write_text("administrator modified this backup")
        engine.remove()
        self.assertTrue((self.root / name.lstrip("/")).exists())

    def test_unknown_wrapper_not_deleted(self):
        self.touchid()
        old = self.put("/usr/local/bin/t2-touchid", "#!/bin/sh\necho admin wrapper\n", 0o755)
        engine = self.engine()
        engine.attempt(engine.migrate_binary, "t2-touchid")
        self.assertTrue(old.exists())
        self.assertTrue(engine.errors)

    def test_missing_replacement_keeps_binary(self):
        old = self.put("/usr/local/bin/t2journal", b"\x7fELFt2journal", 0o755)
        engine = self.engine("t2-journal")
        engine.migrate_services()
        self.assertTrue(old.exists())
        self.assertTrue(engine.errors)

    def test_identical_helper_with_literal_local_paths_is_recognized(self):
        self.put("/usr/local/libexec/t2-services/lifecycle.py", "legacy_path = '/usr/local/bin/t2remote'\n")
        self.put("/usr/libexec/t2-services/lifecycle.py", "legacy_path = '/usr/local/bin/t2remote'\n")
        engine = self.engine("t2-services-common")
        engine.compare_retire("/usr/local/libexec/t2-services/lifecycle.py", "/usr/libexec/t2-services/lifecycle.py")
        self.assertFalse((self.root / "usr/local/libexec/t2-services/lifecycle.py").exists())

    def test_rpm_interpreter_normalization(self):
        self.put("/usr/local/libexec/t2-services/t2-ncm-sleep", "#!/usr/bin/env bash\necho fixture\n")
        self.put("/usr/libexec/t2-services/t2-ncm-sleep", "#!/usr/bin/bash\necho fixture\n")
        engine = self.engine("t2-services-common")
        engine.compare_retire("/usr/local/libexec/t2-services/t2-ncm-sleep", "/usr/libexec/t2-services/t2-ncm-sleep")
        self.assertFalse((self.root / "usr/local/libexec/t2-services/t2-ncm-sleep").exists())

    def test_symlinks_refused(self):
        self.touchid()
        old = self.root / "usr/local/bin/t2-touchid"
        old.unlink()
        old.symlink_to(self.root / "usr/bin/t2-touchid")
        engine = self.engine()
        engine.migrate_services()
        self.assertTrue(old.is_symlink())
        self.assertTrue(engine.errors)

    def test_owned_pam_reverted_but_preexisting_not_claimed(self):
        engine = self.engine()
        current = ["local", "with-mdns4"]
        def command(*args):
            if args[1] == "current":
                return " ".join(current)
            if args[1] == "enable-feature":
                current.append("with-fingerprint")
            if args[1] == "disable-feature":
                current.remove("with-fingerprint")
            return ""
        engine.command = command
        engine.enable_pam()
        engine.enable_pam()
        engine.remove_pam()
        self.assertEqual(current, ["local", "with-mdns4"])
        current.append("with-fingerprint")
        engine.enable_pam()
        self.assertNotIn("pam", engine.state)
        engine.remove_pam()
        self.assertIn("with-fingerprint", current)

    def test_modified_pam_not_reverted(self):
        engine = self.engine()
        engine.state["pam"] = {"after": ["local", "with-fingerprint"]}
        engine.command = lambda *args: "custom/administrator with-fingerprint"
        engine.attempt(engine.remove_pam)
        self.assertIn("pam", engine.state)
        self.assertTrue(engine.errors)

    def test_policy_checksum_ownership(self):
        engine = self.engine()
        calls = []
        current = "400 kait2en-t2-touchid pp sha256:ABC"
        def command(*args):
            calls.append(args)
            return current if "-lfull" in args else ""
        engine.command = command
        engine.install_policy()
        engine.remove_policy()
        self.assertIn(("semodule", "-X", "400", "-r", "kait2en-t2-touchid"), calls)
        engine.install_policy()
        current = "400 kait2en-t2-touchid pp sha256:MODIFIED"
        calls.clear()
        engine.attempt(engine.remove_policy)
        self.assertFalse(any("-r" in args for args in calls))
        self.assertTrue(engine.errors)

    def test_network_preserves_uuid_and_unrelated_profiles(self):
        def profile(uuid, mac="AC:DE:48:00:11:22"):
            return f"[connection]\nid=Apple T2 Bridge\nuuid={uuid}\n[ethernet]\nmac-address={mac}\n[ipv4]\nmethod=disabled\n[ipv6]\nmethod=link-local\n"
        base = "/etc/NetworkManager/system-connections/"
        self.put(base + "t2-ncm.nmconnection", profile("6ab6bd67-a58f-47a6-a7c5-b03c2de64f2b"))
        self.put(base + "old.nmconnection", profile("existing-user-uuid"))
        other = self.put(base + "other.nmconnection", profile("other", "12:34:56:78:90:00"))
        engine = self.engine("t2-services-common")
        engine.migrate_network()
        canonical = self.root / (base + "t2-ncm.nmconnection").lstrip("/")
        self.assertIn("existing-user-uuid", canonical.read_text())
        self.assertFalse((self.root / (base + "old.nmconnection").lstrip("/")).exists())
        self.assertTrue(other.exists())
        engine.migrate_network()
        engine.remove()
        self.assertFalse(canonical.exists())
        self.assertTrue(other.exists())

    def test_dsp_old_data_and_backups_cleaned_on_remove(self):
        self.put("/usr/share/t2-dsp/profiles/15_1/graph.json", '{"target.object":"new","filter.graph":{}}')
        old = self.put("/usr/share/kait2en/audio-dsp/15_1/graph.json", '{"target.object":"old-pci","filter.graph":{}}')
        custom = self.put("/usr/share/kait2en/audio-dsp/15_1/custom.txt", "user notes")
        config = self.put("/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf", "# Generated by scripts/fedora/install-dsp.sh for MacBookPro15,1\n")
        engine = self.engine("t2-dsp")
        engine.migrate_dsp()
        self.assertFalse(old.exists())
        self.assertFalse(config.exists())
        self.assertTrue(custom.exists())
        engine.remove()
        self.assertFalse(engine.state_path.exists())
        self.assertTrue(custom.exists())
        self.assertFalse((self.root / "var/lib/kait2en/migration/t2-dsp").exists())

    def test_dsp_unowned_asset_change_archives_instead_of_repeating_forever(self):
        self.put("/usr/share/t2-dsp/profiles/15_4/graph.json", '{"target.object":"new","filter.graph":{"gain": 2}}')
        old = self.put("/usr/share/kait2en/audio-dsp/15_4/graph.json", '{"target.object":"old-pci","filter.graph":{"gain": 1}}')
        engine = self.engine("t2-dsp")
        engine.migrate_dsp()
        self.assertFalse(old.exists())
        self.assertEqual(sum("archived, not discarded" in error for error in engine.errors), 1)
        engine.migrate_dsp()
        self.assertEqual(sum("archived, not discarded" in error for error in engine.errors), 1)

    def test_failure_reported_and_independent_cleanup_continues(self):
        engine = self.engine()
        engine.state["policy"] = "owned"
        def broken(*args):
            raise ValueError("injected command failure")
        engine.command = broken
        old = self.put("/usr/local/bin/t2-touchid", "owned")
        engine.retire("/usr/local/bin/t2-touchid", True)
        engine.remove()
        self.assertFalse(old.exists())
        self.assertFalse(engine.state["retired"])
        self.assertGreaterEqual(len(engine.errors), 2)


if __name__ == "__main__":
    unittest.main()
