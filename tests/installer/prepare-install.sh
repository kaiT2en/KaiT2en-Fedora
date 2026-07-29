#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
prepare="$repo_root/packaging/installer/runtime/kait2en-prepare"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-prepare-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

target=7.0.1-100.fc44.x86_64
old=6.19.10-300.fc44.x86_64
fake_repo="$work/repository"
fake_transition="$work/transition-source"
fake_boot="$work/boot"
fake_modules="$work/modules"
fake_bin="$work/bin"
fake_state="$work/state"
fake_autostart="$work/kait2en-install.desktop"
fake_prepare_launcher="$work/kait2en-prepare"
fake_terminal_launcher="$work/kait2en-launch-terminal"
fake_os_release="$work/os-release"
log="$work/commands.log"
mkdir -p "$fake_repo/.git" "$fake_repo/scripts/fedora" \
	"$fake_transition/packaging/installer" \
	"$fake_boot" "$fake_modules/$target" "$fake_bin"
touch "$fake_boot/vmlinuz-$old" "$fake_boot/vmlinuz-$target"
printf '1\n' >"$fake_transition/source-date-epoch"
printf 'ID=fedora\nVERSION_ID=44\n' >"$fake_os_release"

cat >"$fake_repo/scripts/fedora/install-dependencies.sh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ ${KERNEL_RELEASE:-} == "$KAIT2EN_TEST_TARGET" ]]
printf 'install-dependencies %s\n' "$KERNEL_RELEASE" >>"$KAIT2EN_TEST_LOG"
EOF
cat >"$fake_transition/packaging/installer/build-input-kmod.sh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ $# -eq 3 ]]
printf 'build-input-kmod kernel=%s arguments=%s\n' "$2" "$#" >>"$KAIT2EN_TEST_LOG"
mkdir -p "$3"
touch "$3/kmod-kait2en-input-test.rpm"
printf '%s\n' "$3/kmod-kait2en-input-test.rpm"
EOF
chmod 0755 \
	"$fake_repo/scripts/fedora/install-dependencies.sh" \
	"$fake_transition/packaging/installer/build-input-kmod.sh"

cat >"$fake_bin/uname" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$KAIT2EN_TEST_RUNNING_RELEASE"
EOF
cat >"$fake_bin/dnf" <<'EOF'
#!/usr/bin/env bash
printf 'dnf %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
EOF
cat >"$fake_bin/grubby" <<'EOF'
#!/usr/bin/env bash
if [[ ${1:-} == --default-kernel ]]; then
	printf '%s\n' "$KAIT2EN_TEST_OLD_KERNEL"
else
	printf 'grubby %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
fi
EOF
cat >"$fake_bin/rpm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
	'-q kernel-core --qf '* ) printf '%s\n' "$KAIT2EN_TEST_TARGET" ;;
	'-q kmod-kait2en-input') exit 0 ;;
	*) printf 'rpm %s\n' "$*" >>"$KAIT2EN_TEST_LOG" ;;
esac
EOF
cat >"$fake_bin/rpm2cpio" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$fake_bin/cpio" <<'EOF'
#!/usr/bin/env bash
destination="$PWD/usr/lib/modules/$KAIT2EN_TEST_TARGET/updates/kait2en"
mkdir -p "$destination"
for module in t2bce_dma t2hid hid_t2magicmouse t2bce_core t2bce_vhci; do
	touch "$destination/$module.ko"
done
EOF
cat >"$fake_bin/depmod" <<'EOF'
#!/usr/bin/env bash
printf 'depmod %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
EOF
cat >"$fake_bin/dracut" <<'EOF'
#!/usr/bin/env bash
printf 'dracut %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
[[ ${KAIT2EN_TEST_DRACUT_FAIL:-0} == 0 ]]
EOF
cat >"$fake_bin/lsinitrd" <<'EOF'
#!/usr/bin/env bash
for module in t2bce_dma t2hid hid_t2magicmouse t2bce_core t2bce_vhci; do
	printf 'usr/lib/modules/test/updates/%s.ko.xz\n' "$module"
done
EOF
chmod 0755 "$fake_bin"/*

run_prepare() {
	local dracut_fail=$1 running_release=$2
	shift 2
	env \
		PATH="$fake_bin:/usr/sbin:/usr/bin:/sbin:/bin" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TRANSITION_SOURCE="$fake_transition" \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_BOOT_ROOT="$fake_boot" \
		KAIT2EN_MODULES_ROOT="$fake_modules" \
		KAIT2EN_OS_RELEASE="$fake_os_release" \
		KAIT2EN_AUTOSTART_FILE="$fake_autostart" \
		KAIT2EN_PREPARE_LAUNCHER="$fake_prepare_launcher" \
		KAIT2EN_TERMINAL_LAUNCHER="$fake_terminal_launcher" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_RUNNING_RELEASE="$running_release" \
		KAIT2EN_TEST_OLD_KERNEL="$fake_boot/vmlinuz-$old" \
		KAIT2EN_TEST_LOG="$log" \
		KAIT2EN_TEST_DRACUT_FAIL="$dracut_fail" \
		KAIT2EN_SKIP_BOOTSTRAP=1 \
		KAIT2EN_TEST_MODE=1 \
		KAIT2EN_WORK_PARENT="$work" \
		bash "$prepare" "$@"
}

run_prepare 0 "$old" >/dev/null
grep -Fq 'dnf upgrade --refresh -y --setopt=installonly_limit=0' "$log"
grep -Fq "install-dependencies $target" "$log"
grep -Fq "build-input-kmod kernel=$target arguments=3" "$log"
# Forced, not merely added: the modules are deleted from the disk below.
grep -Fq \
	'dracut --force --force-drivers t2bce_dma t2hid hid_t2magicmouse t2bce_core t2bce_vhci' \
	"$log"
grep -Fq 'rpm -e kmod-kait2en-input' "$log"
[[ ! -d "$fake_modules/$target/updates/kait2en-transition" ]]
[[ $(tail -n 1 "$log") == "grubby --set-default $fake_boot/vmlinuz-$target" ]]
grep -Fxq 'phase=reboot_pending' "$fake_state/state"
grep -Fxq "target_kernel=$target" "$fake_state/state"

: >"$log"
rm -f "$fake_state/state"
if run_prepare 1 "$old" >/dev/null 2>&1; then
	printf 'installer preparation unexpectedly succeeded after a dracut failure\n' >&2
	exit 1
fi
[[ ! -d "$fake_modules/$target/updates/kait2en-transition" ]]
[[ $(tail -n 1 "$log") == "grubby --set-default $fake_boot/vmlinuz-$old" ]]

printf 'phase=reboot_pending\ntarget_kernel=%s\n' "$target" >"$fake_state/state"
touch "$fake_autostart" "$fake_prepare_launcher" "$fake_terminal_launcher"
run_prepare 0 "$target" --complete >/dev/null
[[ ! -e "$fake_autostart" ]]
[[ ! -e "$fake_prepare_launcher" ]]
[[ ! -e "$fake_terminal_launcher" ]]
[[ ! -e "$fake_transition" ]]
[[ ! -e "$fake_state/lock" ]]
grep -Fxq 'phase=complete' "$fake_state/state"
grep -Fxq "target_kernel=$target" "$fake_state/state"
