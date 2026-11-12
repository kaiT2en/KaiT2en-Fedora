#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_fedora

usage() {
	printf 'Usage: %s [install|remove] [KERNEL_RELEASE] [--no-deps]\n' "$0" >&2
	exit 2
}

ACTION=${1:-install}
shift $(( $# > 0 ? 1 : 0 ))
KVER=$(kernel_release)
KVER_SET=0
SKIP_DEPS=0
for argument in "$@"; do
	case "$argument" in
		--no-deps) SKIP_DEPS=1 ;;
		--*) usage ;;
		*)
			((KVER_SET == 0)) || usage
			KVER=$argument
			KVER_SET=1
			;;
	esac
done
HOOK_HOME="/usr/local/libexec/kait2en/kernel-source"
KERNEL_INSTALL_HOOK="/etc/kernel/install.d/44-kait2en-kernel-source.install"
FROM_REPO=1
[[ -d "$SCRIPT_DIR/../../patches" ]] || FROM_REPO=0

# Fill the pool for every kernel dnf installs later, before the GPU runtime PM
# and wireless hooks build from it. The hook never fails the transaction.
install_kernel_hook() {
	rm -rf "$HOOK_HOME"
	install -d -m 0755 "$HOOK_HOME"
	install -m 0755 "$SCRIPT_DIR/install-kernel-source.sh" "$HOOK_HOME/"
	install -m 0644 "$SCRIPT_DIR/lib.sh" "$HOOK_HOME/"

	install -d -m 0755 "${KERNEL_INSTALL_HOOK%/*}"
	cat >"$KERNEL_INSTALL_HOOK" <<HOOK
#!/usr/bin/bash

command=\${1:-}
kernelver=\${2:-}
log=/var/log/kait2en-kernel-source.log

[[ -n "\$kernelver" ]] || exit 0
case "\$command" in
	add)
		printf '[kait2en] preparing the kernel source for %s\\n' "\$kernelver"
		if ! "$HOOK_HOME/install-kernel-source.sh" install "\$kernelver" \\
				--no-deps >>"\$log" 2>&1; then
			printf '[kait2en] kernel source for %s is unavailable, see %s\\n' \\
				"\$kernelver" "\$log"
		fi
		;;
	remove)
		rm -rf "$KERNEL_SOURCE_CACHE/\$kernelver"
		;;
esac
exit 0
HOOK
	chmod 0755 "$KERNEL_INSTALL_HOOK"
}

case "$ACTION" in
	remove)
		if ((KVER_SET)); then
			rm -rf "${KERNEL_SOURCE_CACHE:?}/$KVER"
		else
			rm -f "$KERNEL_INSTALL_HOOK"
			rm -rf "$HOOK_HOME" "$KERNEL_SOURCE_CACHE"
		fi
		exit 0
		;;
	install) ;;
	*) usage ;;
esac

# Refresh the hook copy before the fetch, so a failed fetch for the running
# kernel cannot leave an outdated hook behind for the next kernel.
((FROM_REPO == 0)) || install_kernel_hook

((SKIP_DEPS)) || dnf install -y cpio curl git-core rpm tar xz
require_command cpio curl git rpm rpm2cpio tar xz

prune_kernel_sources
tree=$(kernel_source_tree "$KVER") || fail "kernel source for $KVER is unavailable"
info "kernel source for $KVER: $tree"
