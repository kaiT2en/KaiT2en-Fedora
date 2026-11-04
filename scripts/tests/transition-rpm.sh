#!/usr/bin/env bash

set -Eeuo pipefail

[[ -f /run/.containerenv || -f /.dockerenv ]] || {
	printf 'This RPM integration test requires a disposable build container.\n' >&2
	exit 1
}

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
input_rpm=${1:?input RPM is required}
kernel_release=${2:?kernel release is required}
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-transition-rpm-test.XXXXXX")
cleanup() {
	rpm -e kmod-kait2en-input kait2en-test-kernel >/dev/null 2>&1 || :
	rm -rf "$work"
}
trap cleanup EXIT

cat >"$work/kernel.spec" <<SPEC
Name: kait2en-test-kernel
Version: 1
Release: 1
Summary: Kernel dependency metadata for the disposable installer RPM test
License: MIT
BuildArch: noarch
Provides: kernel-core-uname-r = $kernel_release
%description
Test-only provider of the ISO kernel dependency, inside the build container.
%files
SPEC
rpmbuild -bb --define "_topdir $work/rpmbuild" "$work/kernel.spec" >"$work/build.log" 2>&1
rpm --install --nosignature "$work/rpmbuild/RPMS/noarch/kait2en-test-kernel-1-1.noarch.rpm"

python3 - "$repo_root" "$input_rpm" "$kernel_release" "$work" <<'PY'
import ast
import io
import os
import pathlib
import shutil
import subprocess
import sys
from types import SimpleNamespace

repo, input_rpm, kernel, work = sys.argv[1:]
repo, input_rpm, work = map(pathlib.Path, (repo, input_rpm, work))
source = work / "source"
source.mkdir()
source_rpm = source / input_rpm.name
shutil.copy2(input_rpm, source_rpm)
service = repo / "auto-installer/anaconda-addon/com_kait2en_input/service"
module_ast = ast.parse((service / "constants.py.in").read_text())
modules = next(ast.literal_eval(n.value) for n in module_ast.body
               if isinstance(n, ast.Assign) and n.targets[0].id == "MODULES")

def rpm(*args):
    return subprocess.run(["rpm", "--define", "_pkgverify_level all", *args],
                          text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

baseline = rpm("--install", str(source_rpm))
assert baseline.returncode != 0, baseline.stdout
assert "no signature" in baseline.stdout, baseline.stdout
module_root = pathlib.Path("/usr/lib/modules") / kernel / "updates/kait2en"
assert not (module_root / "t2bce_dma.ko").exists()

calls = []
def redirect(command, arguments, **kwargs):
    calls.append(command)
    if command == "rpm":
        result = rpm(*arguments)
        kwargs["stdout"].write(result.stdout)
        return result.returncode
    if command == "lsinitrd":
        kwargs["stdout"].write("\n".join(module + ".ko" for module in modules))
    return 0

env = dict(io=io, os=os, shutil=shutil, Task=object, MODULES=modules,
           KERNEL_RELEASE=kernel, RPM_FILENAME=input_rpm.name,
           INSTALLER_ROOT=str(source),
           conf=SimpleNamespace(target=SimpleNamespace(system_root="/")),
           util=SimpleNamespace(execWithRedirect=redirect))
tree = ast.parse((service / "installation.py").read_text())
cls = next(n for n in tree.body if isinstance(n, ast.ClassDef)
           and n.name == "InstallTransitionDriversTask")
exec(compile(ast.Module(body=[cls], type_ignores=[]), str(service / "installation.py"), "exec"), env)
task = env["InstallTransitionDriversTask"]()
task.run()
assert calls == ["rpm", "depmod", "dracut", "lsinitrd"], calls
for module in modules:
    assert (module_root / (module + ".ko")).is_file(), module
assert not (pathlib.Path("/tmp") / input_rpm.name).exists()
assert rpm("--erase", "kmod-kait2en-input").returncode == 0

damaged = bytearray(input_rpm.read_bytes())
damaged[-1] ^= 0xFF
source_rpm.write_bytes(damaged)
calls.clear()
try:
    task.run()
except RuntimeError as error:
    assert "rpm failed" in str(error), str(error)
else:
    raise AssertionError("A damaged RPM was accepted")
assert calls == ["rpm"], calls
assert not (module_root / "t2bce_dma.ko").exists()
assert not (pathlib.Path("/tmp") / input_rpm.name).exists()

shutil.copy2(input_rpm, source_rpm)
assert rpm("--erase", "kait2en-test-kernel").returncode == 0
calls.clear()
try:
    task.run()
except RuntimeError as error:
    assert "rpm failed" in str(error), str(error)
    assert "kernel-core-uname-r" in str(error), str(error)
else:
    raise AssertionError("An input RPM without its required kernel was accepted")
assert calls == ["rpm"], calls
assert not (pathlib.Path("/tmp") / input_rpm.name).exists()
print("Real transition RPM tests passed: install, digest rejection, kernel dependency rejection.")
PY
