#!/usr/bin/env bash

# Keep this script compatible with the Bash 3.2 shipped by macOS.
# Run this from macOS. It copies the Touch ID Catacomb and the AppleKeyStore
# keybags to a directory Linux can read, normally the EFI partition. It only
# reads; nothing in macOS or in the Secure Enclave is changed.
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
OUTPUT_DIR="$SCRIPT_DIR"
WANT_CATACOMB=1
WANT_KEYBAGS=1
FAILURES=0

usage() {
	cat <<EOF
Usage: ${0##*/} [options]

Options:
  --output DIR      Where to write the archives (default: this script's directory)
  --catacomb-only   Skip the keybags
  --keybags-only    Skip the Catacomb
  -h, --help        Show this help

The Catacomb is what LoadCatacomb needs before the Secure Enclave will match a
finger. The keybags are needed later, to unlock the keystore.
EOF
}

fail() {
	printf 'Error: %s\n' "$*" >&2
	FAILURES=$((FAILURES + 1))
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--output)
		[[ $# -ge 2 ]] || { printf 'Error: --output needs a directory\n' >&2; exit 2; }
		OUTPUT_DIR="$2"
		shift 2
		;;
	--catacomb-only) WANT_KEYBAGS=0; shift ;;
	--keybags-only) WANT_CATACOMB=0; shift ;;
	-h|--help) usage; exit 0 ;;
	*) printf 'Error: unknown option %s\n' "$1" >&2; usage >&2; exit 2 ;;
	esac
done

[[ "$(uname -s)" == Darwin ]] || { printf 'Error: run this from macOS\n' >&2; exit 1; }
[[ -d "$OUTPUT_DIR" ]] || { printf 'Error: %s is not a directory\n' "$OUTPUT_DIR" >&2; exit 1; }

printf 'Writing to %s\n' "$OUTPUT_DIR"
printf 'macOS will ask for your administrator password.\n'
sudo -v

if [[ "$WANT_CATACOMB" == 1 ]]; then
	if sudo test -d /Library/Catacomb; then
		size="$(sudo du -sh /Library/Catacomb | awk '{print $1}')"
		printf 'Archiving /Library/Catacomb (%s)...\n' "$size"
		if sudo tar -C / -czf "$OUTPUT_DIR/catacomb.tar.gz" Library/Catacomb; then
			sudo chmod 644 "$OUTPUT_DIR/catacomb.tar.gz"
			printf 'Wrote catacomb.tar.gz\n'
		else
			fail "could not archive /Library/Catacomb"
		fi
	else
		fail "/Library/Catacomb does not exist; has a finger ever been enrolled?"
	fi
fi

if [[ "$WANT_KEYBAGS" == 1 ]]; then
	work="$(mktemp -d /tmp/t2-keybags.XXXXXX)"
	trap 'rm -rf -- "$work"' EXIT
	list="$work/paths.txt"
	denied="$OUTPUT_DIR/keybag-scan-denied.txt"
	: >"$list"
	: >"$denied"
	# System volumes hold most of these and the scan is slow, so say where we are.
	for root in /System/Volumes/Preboot /private/var /Library /Users; do
		sudo test -d "$root" || continue
		printf 'Scanning %s for keybags...\n' "$root"
		sudo find "$root" -xdev -type f \
			\( -iname 'user.kb' -o -iname 'stash.kb' -o -iname '*.kb' \) \
			-size -16M -print 2>>"$denied" >>"$list" || true
	done
	found="$(wc -l <"$list" | tr -d ' ')"
	unreadable="$(wc -l <"$denied" | tr -d ' ')"
	if [[ "$unreadable" != 0 ]]; then
		printf '%s paths were not readable, which is normal under SIP.\n' "$unreadable"
		printf 'The list is in %s if you want to check it.\n' "$denied"
	else
		rm -f -- "$denied"
	fi
	if [[ "$found" == 0 ]]; then
		fail "no keybag files found"
	elif sudo tar -czf "$OUTPUT_DIR/keybags.tar.gz" -T "$list"; then
		sudo chmod 644 "$OUTPUT_DIR/keybags.tar.gz"
		printf 'Wrote keybags.tar.gz (%s files)\n' "$found"
	else
		fail "could not archive the keybags"
	fi
fi

sync

if [[ "$FAILURES" != 0 ]]; then
	printf '\nFinished with %s error(s). See above.\n' "$FAILURES" >&2
	exit 1
fi

printf '\nDone. Reboot into Linux.\n'
printf 'Keep these archives private. They are key material, not diagnostics.\n'
