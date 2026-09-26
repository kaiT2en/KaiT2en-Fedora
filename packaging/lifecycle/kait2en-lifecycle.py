#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Owned migration and removal, shared as source but installed independently.

--root is an offline test/staging root. It never invokes host system services.
Every independent operation is attempted. Failures are logged and summarized.
"""
import argparse
import configparser
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

UNITS = {"t2-services-common": "t2-services-suspend.service",
         "t2-touchid": "kait2en-t2-touchid.service", "t2-ave": "kait2en-t2-remote.service"}
BINS = {"t2-touchid": "t2-touchid", "t2-ave": "t2remote", "t2-journal": "t2journal"}
POLICY = "kait2en-t2-touchid"


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Lifecycle:
    def __init__(self, component, root="", assets=None):
        self.component = component
        self.root = Path(root) if root else None
        self.assets = Path(assets) if assets else Path(__file__).parent / "migration"
        self.errors = []
        self.state_path = self.path(f"/var/lib/kait2en/ownership/{component}.json")
        self.state = json.loads(self.state_path.read_text()) if self.state_path.exists() else {}
        self.state.setdefault("retired", {})
        self.state.setdefault("created", {})

    def path(self, absolute):
        if not absolute.startswith("/") or ".." in Path(absolute).parts:
            raise ValueError(f"invalid managed path: {absolute}")
        path = (self.root / absolute.lstrip("/")) if self.root else Path(absolute)
        # Never follow administrator symlinks outside a managed path.
        for parent in (path, *path.parents):
            if parent.is_symlink():
                raise ValueError(f"refusing symlink: {parent}")
            if parent == self.root:
                break
        return path

    def error(self, message):
        self.errors.append(str(message))
        print(f"[{self.component}] error: {message}", flush=True)
        for name in (f"/var/log/{self.component}-install.log",):
            try:
                path = self.path(name)
                path.parent.mkdir(parents=True, exist_ok=True)
                with path.open("a") as log:
                    log.write(str(message) + "\n")
            except OSError as exc:
                print(f"[{self.component}] cannot save log: {exc}", flush=True)
        ledger = os.environ.get("KAIT2EN_INSTALL_ERRORS")
        if ledger:
            try:
                with open(ledger, "a") as log:
                    log.write(f"{self.component}: {message}\n")
            except OSError as exc:
                print(f"[{self.component}] cannot append installer report: {exc}", flush=True)

    def attempt(self, function, *args):
        try:
            return function(*args)
        except (OSError, ValueError, KeyError, configparser.Error) as exc:
            self.error(f"{function.__name__}: {exc}")
            return None

    def command(self, *args):
        if self.root:
            return ""  # offline roots never access the host bus or policy store
        if args[0] == "systemctl" and not Path("/run/systemd/system").is_dir():
            if args[1] in ("reenable", "disable"):
                args = ("systemctl", "--root=/", *(arg for arg in args[1:] if arg != "--now"))
            else:
                return ""  # no running manager, only update enablement on disk
        result = subprocess.run(args, text=True, capture_output=True)
        if result.returncode:
            raise ValueError(f"{' '.join(args)} failed ({result.returncode}): {result.stderr.strip()}")
        return result.stdout.strip()

    def save(self):
        self.state_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.atomic(self.state_path, (json.dumps(self.state, indent=2) + "\n").encode(), 0o600)

    @staticmethod
    def atomic(path, data, mode=0o644):
        path.parent.mkdir(parents=True, exist_ok=True)
        fd, temporary = tempfile.mkstemp(dir=path.parent, prefix=".kait2en-")
        try:
            with os.fdopen(fd, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(temporary, mode)
            os.replace(temporary, path)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def owned(self, name, data):
        return self.state.get("source_files", {}).get(name) == digest(data)

    def retire(self, name, verified=False):
        path = self.path(name)
        if not path.exists():
            return
        data = path.read_bytes()
        sha = digest(data)
        verified = verified or self.owned(name, data)
        archive_name = f"/var/lib/kait2en/migration/{self.component}/{sha}-{path.name}"
        archive = self.path(archive_name)
        archive.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.atomic(archive, data, path.stat().st_mode & 0o777)
        self.state["retired"][archive_name] = {"sha256": sha, "verified": verified, "original": name}
        self.save()  # persist recovery information BEFORE removing the old file
        path.unlink()
        print(f"[{self.component}] retired {name}. Backup: {archive_name}")
        if not verified:
            self.error(f"unverified legacy file archived, not discarded: {name}")

    def compare_retire(self, old, new):
        path = self.path(old)
        if not path.exists():
            return
        data = path.read_bytes()
        reference = self.path(new).read_bytes()
        def normalized_script(content):
            # Fedora's brp-mangle-shebangs rewrites these at package build time.
            # Only normalize the exact interpreter line, never arbitrary code.
            for interpreter in (b"bash", b"python3"):
                prefix = b"#!/usr/bin/env " + interpreter + b"\n"
                if content.startswith(prefix):
                    return b"#!/usr/bin/" + interpreter + b"\n" + content[len(prefix):]
            return content
        actual, expected = normalized_script(data), normalized_script(reference)
        if actual != expected and actual.replace(b"/usr/local/", b"/usr/") != expected and not self.owned(old, data):
            raise ValueError(f"modified local override preserved: {old}. Review before activating the package")
        self.retire(old, True)

    def migrate_services(self):
        if self.component == "t2-dsp":
            return self.migrate_dsp()
        unit = UNITS.get(self.component)
        if unit:
            old = f"/etc/systemd/system/{unit}"
            self.attempt(self.compare_retire, old, f"/usr/lib/systemd/system/{unit}")
            self.attempt(self.compare_retire, f"/usr/local/lib/systemd/system/{unit}", f"/usr/lib/systemd/system/{unit}")
            if self.path(old).exists() or self.path(f"/usr/local/lib/systemd/system/{unit}").exists():
                # A retained administrator unit may still execute /usr/local.
                # Do not strand it by retiring its binary or integrations.
                self.save()
                return
            # systemctl reenable repairs enablement links still pointing at /etc.
            self.state["was_enabled"] = self.state.get("was_enabled", False) or any(
                self.path(f"/etc/systemd/system/{target}.wants").joinpath(unit).is_symlink()
                for target in ("multi-user.target", "sleep.target"))
        binary = BINS.get(self.component)
        if binary:
            self.attempt(self.migrate_binary, binary)
        if self.component == "t2-touchid":
            self.attempt(self.compare_retire,
                         "/etc/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf",
                         "/usr/lib/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf")
            self.attempt(self.compare_retire, "/etc/dbus-1/system.d/org.kait2en.TouchId.conf",
                         "/usr/share/dbus-1/system.d/org.kait2en.TouchId.conf")
            self.attempt(self.compare_retire, "/usr/local/share/dbus-1/system.d/org.kait2en.TouchId.conf",
                         "/usr/share/dbus-1/system.d/org.kait2en.TouchId.conf")
            if "pam" not in self.state and not self.root and shutil.which("authselect"):
                current = self.attempt(self.command, "authselect", "current", "--raw")
                if current and "with-fingerprint" in current.split():
                    self.error("existing PAM fingerprint feature has no ownership receipt. It will not be disabled on removal")
        elif self.component == "t2-ave":
            self.attempt(self.compare_retire, "/usr/local/libexec/t2-services/sleep.d/t2-ave",
                         "/usr/libexec/t2-services/sleep.d/t2-ave")
        elif self.component == "t2-services-common":
            self.attempt(self.compare_retire, "/usr/local/libexec/t2-services/t2-ncm-sleep",
                         "/usr/libexec/t2-services/t2-ncm-sleep")
            # Reporter has no runtime role outside the old source layout.
            old = "/usr/local/libexec/t2-services/package-actions"
            if self.path(old).exists():
                self.attempt(self.retire, old, True)
            for suffix in ("lifecycle.py", "migration/kait2en-suspend.sh", "migration/suspend-hashes.json"):
                self.attempt(self.compare_retire, f"/usr/local/libexec/t2-services/{suffix}",
                             f"/usr/libexec/t2-services/{suffix}")
            self.attempt(self.migrate_suspend)
            self.attempt(self.migrate_network)
            for name, markers in (
                ("/etc/systemd/system/kait2en-t2-ncm-down.service", (b"kait2en-t2-ncm-down.sh",)),
                ("/usr/local/libexec/kait2en/kait2en-t2-ncm-down.sh", (b"cdc_ncm",)),
                ("/etc/udev/rules.d/90-kait2en-t2-network.rules", (b"05ac", b"8233")),
                ("/etc/udev/rules.d/90-kait2en-t2-network-managed.rules", (b"05ac", b"8233")),
                ("/etc/NetworkManager/conf.d/99-network-t2-ncm.conf", (b"t2_ncm",)),
                ("/etc/NetworkManager/conf.d/10-kait2en-t2-no-auto-default.conf", (b"no-auto-default",)),
            ):
                self.attempt(self.retire_marked, name, markers)
        if unit and self.state.get("was_enabled"):
            self.attempt(self.command, "systemctl", "daemon-reload")
            self.attempt(self.command, "systemctl", "reenable", unit)
        self.save()

    def retire_marked(self, name, markers):
        path = self.path(name)
        if not path.exists():
            return
        data = path.read_bytes()
        if not all(marker in data for marker in markers) and not self.owned(name, data):
            raise ValueError(f"unrecognized legacy file preserved: {name}")
        if name.endswith("kait2en-t2-ncm-down.service"):
            self.command("systemctl", "disable", "kait2en-t2-ncm-down.service")
        self.retire(name, True)

    def migrate_binary(self, binary):
        old, new = f"/usr/local/bin/{binary}", f"/usr/bin/{binary}"
        if not self.path(old).exists():
            return
        replacement = self.path(new)
        if not replacement.is_file() or not os.access(replacement, os.X_OK):
            raise ValueError(f"replacement missing or not executable: {new}")
        data = self.path(old).read_bytes()
        # Legacy source binaries had no receipts. Only adopt the exact legacy
        # pathname and an ELF identifying this application, never shell wrappers.
        if data[:4] != b"\x7fELF" or binary.encode() not in data:
            if not self.owned(old, data):
                raise ValueError(f"unrecognized executable preserved: {old}")
        self.retire(old, True)

    @staticmethod
    def normalized_graph(value):
        if isinstance(value, dict):
            return {key: Lifecycle.normalized_graph(child) for key, child in value.items() if key != "target.object"}
        if isinstance(value, list):
            return [Lifecycle.normalized_graph(child) for child in value]
        if isinstance(value, str):
            for prefix in ("/usr/share/kait2en/audio-dsp/", "/usr/share/t2-dsp/profiles/",
                           "/usr/share/t2-linux-audio/", "/usr/share/t2linux-audio/"):
                value = value.replace(prefix, "@PROFILES@/")
        return value

    def migrate_dsp(self):
        configurations = [
            ("/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf", b"# Generated by scripts/fedora/install-dsp.sh for "),
            ("/etc/pipewire/pipewire.conf.d/50-kait2en-quantum.conf", b"# Generated by scripts/fedora/install-dsp.sh"),
        ]
        for name, marker in configurations:
            def migrate_config(name=name, marker=marker):
                path = self.path(name)
                if path.exists():
                    if marker not in path.read_bytes() and not self.owned(name, path.read_bytes()):
                        raise ValueError(f"unrecognized local configuration preserved: {name}")
                    self.retire(name, True)
            self.attempt(migrate_config)
        base = "/usr/share/kait2en/audio-dsp"
        for new in sorted(self.path("/usr/share/t2-dsp/profiles").glob("*/*")):
            if not new.is_file():
                continue
            name = f"{base}/{new.parent.name}/{new.name}"
            def migrate_asset(name=name, new=new):
                old = self.path(name)
                if not old.exists():
                    return
                same = old.read_bytes() == new.read_bytes()
                if new.suffix == ".json":
                    same = self.normalized_graph(json.loads(old.read_text())) == self.normalized_graph(json.loads(new.read_text()))
                # owned() never confirms these (record_source has no t2-dsp case); retire() archives unverified content anyway.
                self.retire(name, same or self.owned(name, old.read_bytes()))
            self.attempt(migrate_asset)
        # Retire backups produced by the previous DSP migration helper too.
        for backup in self.path("/var/lib/t2-dsp/migration").glob("*"):
            def migrate_backup(backup=backup):
                name = "/var/lib/t2-dsp/migration/" + backup.name
                data = self.path(name).read_bytes()
                if b"# Generated by scripts/fedora/install-dsp.sh" not in data:
                    raise ValueError(f"unverified old migration backup preserved: {name}")
                self.retire(name, True)
            self.attempt(migrate_backup)
        for pattern in ("/etc/pipewire/pipewire.conf.d/t2_*_speakers.conf",
                        "/etc/pipewire/pipewire.conf.d/t2_*_mic.conf",
                        "/etc/wireplumber/wireplumber.conf.d/51-t2-dsp.conf",
                        "/etc/udev/rules.d/99-t2-audio-rename.rules",
                        "/usr/lib/udev/rules.d/99-t2-audio-rename.rules"):
            parent, filename = pattern.rsplit("/", 1)
            for path in self.path(parent).glob(filename):
                self.error(f"unrecognized legacy configuration preserved: {parent}/{path.name}")
        self.empty_directories(base)
        self.empty_directories("/var/lib/t2-dsp/migration")
        self.save()
        print("[t2-dsp] Reboot to activate. No audio services were restarted")

    def empty_directories(self, name):
        base = self.path(name)
        if not base.is_dir():
            return
        for path in sorted(base.rglob("*"), reverse=True):
            if path.is_dir() and not path.is_symlink() and not any(path.iterdir()):
                path.rmdir()
        if not any(base.iterdir()):
            base.rmdir()

    def migrate_suspend(self):
        old = "/usr/local/libexec/kait2en/kait2en-suspend.sh"
        if not self.path(old).exists():
            return
        data = self.path(old).read_bytes()
        if b"unbind_t2_ncm()" not in data:
            return
        known = json.loads((self.assets / "suspend-hashes.json").read_text())
        if digest(data) not in known:
            raise ValueError("unknown combined suspend helper. Common suspend unit must not be activated until NCM ownership is resolved")
        replacement = (self.assets / "kait2en-suspend.sh").read_bytes()
        # Preserve the independent Wi-Fi/Bluetooth source installation. Its
        # existing unit keeps this path. This file is not owned by common.
        self.retire(old, True)
        self.atomic(self.path(old), replacement, 0o755)

    def migrate_network(self):
        folder = self.path("/etc/NetworkManager/system-connections")
        canonical = folder / "t2-ncm.nmconnection"
        candidates = []
        for path in sorted(folder.glob("*.nmconnection")):
            path = self.path("/etc/NetworkManager/system-connections/" + path.name)
            parser = configparser.ConfigParser(interpolation=None)
            parser.read_string(path.read_text())
            mac = parser.get("ethernet", "mac-address", fallback="").lower()
            if mac != "ac:de:48:00:11:22":
                continue
            if parser.get("ipv4", "method", fallback="") != "disabled" or parser.get("ipv6", "method", fallback="") != "link-local":
                self.error(f"custom T2 network profile preserved: {path.name}")
                continue
            candidates.append((path, parser))
        # Prefer an existing script profile over the package's default UUID.
        legacy = [(p, c) for p, c in candidates if p != canonical]
        if legacy:
            if canonical.exists():
                existing = configparser.ConfigParser(interpolation=None)
                existing.read_string(canonical.read_text())
                uuid = existing.get("connection", "uuid", fallback="")
                if uuid != "6ab6bd67-a58f-47a6-a7c5-b03c2de64f2b" and self.state["created"].get(
                        "/etc/NetworkManager/system-connections/t2-ncm.nmconnection") != digest(canonical.read_bytes()):
                    raise ValueError("existing canonical NCM profile is administrator-owned. Preserving it and other profiles")
            selected, config = next(((p, c) for p, c in legacy
                if c.get("connection", "id", fallback="") == "Apple T2 Bridge"), legacy[0])
            if canonical.exists():
                self.retire("/etc/NetworkManager/system-connections/t2-ncm.nmconnection", True)
            data = selected.read_bytes()
            self.atomic(canonical, data, 0o600)
            self.state["created"]["/etc/NetworkManager/system-connections/t2-ncm.nmconnection"] = digest(data)
            self.save()
            for path, config in legacy:
                name = "/etc/NetworkManager/system-connections/" + path.name
                known_name = config.get("connection", "id", fallback="") in ("Apple T2 Bridge", "kait2en-t2-ncm")
                self.attempt(self.retire, name, known_name)
        # Do not disconnect or reload live connections. New files apply on boot.

    def enable_pam(self):
        current = self.command("authselect", "current", "--raw").split()
        if "with-fingerprint" in current:
            return  # do not claim somebody else's existing setting
        self.state["pam"] = {"before": current, "pending": True}
        self.save()
        self.command("authselect", "enable-feature", "with-fingerprint")
        self.state["pam"] = {"before": current, "after": self.command("authselect", "current", "--raw").split()}
        self.save()

    def remove_pam(self):
        receipt = self.state.get("pam")
        if not receipt or "after" not in receipt:
            return
        current = self.command("authselect", "current", "--raw").split()
        if "with-fingerprint" not in current:
            self.state.pop("pam", None)
        elif current != receipt["after"]:
            raise ValueError("PAM configuration changed since our activation. Preserving fingerprint feature")
        else:
            self.command("authselect", "disable-feature", "with-fingerprint")
            self.state.pop("pam", None)
        self.save()

    def policy_fingerprint(self):
        lines = self.command("semodule", "-lfull", "-m").splitlines()
        return next((line.strip() for line in lines if len(line.split()) > 1
                     and line.split()[0] == "400" and line.split()[1] == POLICY), "")

    def install_policy(self):
        self.command("semodule", "-X", "400", "-i", "/usr/share/selinux/packages/kait2en-t2-touchid.pp")
        checksum = self.policy_fingerprint()
        if not checksum and not self.root:
            raise ValueError("installed SELinux module checksum could not be recorded")
        self.state["policy"] = checksum
        self.save()

    def remove_policy(self):
        if "policy" not in self.state:
            return
        current = self.policy_fingerprint()
        if current and current != self.state["policy"]:
            raise ValueError("SELinux module changed since package installation. Preserving it")
        if current:
            self.command("semodule", "-X", "400", "-r", POLICY)
        self.state.pop("policy", None)
        self.save()

    def remove(self):
        if self.component in UNITS:
            self.attempt(self.command, "systemctl", "disable", "--now", UNITS[self.component])
        if self.component == "t2-touchid":
            self.attempt(self.remove_pam)
            self.attempt(self.remove_policy)
        for name, sha in list(self.state["created"].items()):
            self.attempt(self.remove_owned, name, sha)
        for name, record in list(self.state["retired"].items()):
            if record["verified"]:
                self.attempt(self.remove_owned, name, record["sha256"])
            else:
                self.error(f"unverified backup preserved for administrator review: {name}")
        self.save()
        self.empty_directories(f"/var/lib/kait2en/migration/{self.component}")
        if not self.state["retired"] and not self.state["created"] and not any(
                key in self.state for key in ("pam", "policy")) and not self.errors:
            self.state_path.unlink(missing_ok=True)

    def remove_owned(self, name, sha):
        path = self.path(name)
        if path.exists():
            if digest(path.read_bytes()) != sha:
                raise ValueError(f"modified owned file preserved: {name}")
            path.unlink()
        self.state["created"].pop(name, None)
        self.state["retired"].pop(name, None)
        self.save()

    def record_source(self):
        names = []
        if self.component in BINS:
            names.append(f"/usr/local/bin/{BINS[self.component]}")
        if self.component in UNITS:
            names.append(f"/etc/systemd/system/{UNITS[self.component]}")
        if self.component == "t2-touchid":
            names.append("/etc/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf")
        if self.component == "t2-ave":
            names.append("/usr/local/libexec/t2-services/sleep.d/t2-ave")
        if self.component == "t2-services-common":
            names.extend("/usr/local/libexec/t2-services/" + suffix for suffix in (
                "t2-ncm-sleep", "package-actions", "lifecycle.py",
                "migration/kait2en-suspend.sh", "migration/suspend-hashes.json"))
        self.state["source_files"] = {name: digest(self.path(name).read_bytes()) for name in names}
        self.save()

    def summary(self):
        print(f"[{self.component}] completed with {len(self.errors)} errors/warnings")
        for error in self.errors:
            print(f"  - {error}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("component", choices=[*UNITS, "t2-journal", "t2-dsp"])
    parser.add_argument("action", choices=["migrate", "remove", "enable-pam", "install-policy", "record-source"])
    parser.add_argument("--root", default="")
    parser.add_argument("--assets")
    args = parser.parse_args()
    try:
        lifecycle = Lifecycle(args.component, args.root, args.assets)
        function = {"migrate": lifecycle.migrate_services, "remove": lifecycle.remove,
                    "enable-pam": lifecycle.enable_pam, "install-policy": lifecycle.install_policy,
                    "record-source": lifecycle.record_source}[args.action]
        lifecycle.attempt(function)
        lifecycle.summary()
        # A nonzero diagnostic result is caught by the non-aborting package and
        # source wrappers. Never pretend a failed migration was successful.
        return int(bool(lifecycle.errors))
    except (OSError, ValueError) as exc:
        print(f"[{args.component}] lifecycle state unavailable: {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
