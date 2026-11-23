#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

# Keep this helper compatible with the Bash 3.2 shipped by macOS.

kait2en_iso_run() {
	if declare -F run_as_calling_user >/dev/null 2>&1; then
		run_as_calling_user "$@"
	else
		"$@"
	fi
}

kait2en_verify_iso() {
	local path=$1 expected_size=$2 expected_sha=$3 actual_size actual_sha
	[[ -f "$path" ]] || return 1
	actual_size=$(stat -f %z "$path") || return 1
	[[ "$actual_size" == "$expected_size" ]] || return 1
	actual_sha=$(shasum -a 256 "$path" | awk '{print $1}') || return 1
	[[ "$actual_sha" == "$expected_sha" ]]
}

kait2en_fedora_release_path() {
	local url=$1
	local prefix=https://download.fedoraproject.org/pub/fedora/linux/releases/
	local path

	[[ "$url" == "$prefix"* ]] || return 1
	path=${url#"$prefix"}
	# MirrorManager's path parameter must not receive query strings, escaped
	# separators, dot components, or any path outside Fedora releases.
	[[ "$path" =~ ^[0-9]+(/[A-Za-z0-9._+-]+)+$ ]] || return 1
	case "/$path/" in
		*/../*|*/./*) return 1 ;;
	esac
	printf '%s\n' "$path"
}

kait2en_add_iso_candidate() {
	local candidate=$1 existing
	[[ "$candidate" == https://* ]] || return 1
	if ((${#KAIT2EN_ISO_CANDIDATES[@]} > 0)); then
		for existing in "${KAIT2EN_ISO_CANDIDATES[@]}"; do
			[[ "$existing" != "$candidate" ]] || return 1
		done
	fi
	KAIT2EN_ISO_CANDIDATES+=("$candidate")
	return 0
}

kait2en_download_candidate() {
	local candidate=$1 partial=$2 status

	if kait2en_iso_run curl --fail --location --retry 2 --retry-delay 2 \
		--connect-timeout 15 --speed-limit 1024 --speed-time 30 \
		--continue-at - --output "$partial" "$candidate"; then
		return 0
	else
		status=$?
	fi

	# curl returns 33 when a server cannot honour a resume request. Retry that
	# mirror once from byte zero so a good mirror is not discarded needlessly.
	if [[ $status -eq 33 && -e "$partial" ]]; then
		printf '  Mirror does not support resume; restarting this source.\n'
		rm -f "$partial"
		if kait2en_iso_run curl --fail --location --retry 2 --retry-delay 2 \
			--connect-timeout 15 --speed-limit 1024 --speed-time 30 \
			--output "$partial" "$candidate"; then
			return 0
		else
			return $?
		fi
	fi
	return "$status"
}

kait2en_download_iso() {
	local destination=$1 canonical_url=$2 expected_size=$3 expected_sha=$4
	local partial="$destination.part"
	local release_path mirrorlist_url archive_url mirror_file line candidate
	local mirror_count=0 attempted=0 partial_size status

	if [[ -e "$destination" ]]; then
		if kait2en_verify_iso "$destination" "$expected_size" "$expected_sha"; then
			printf 'Using verified cached ISO: %s\n' "$destination"
			return 0
		fi
		printf 'Error: cached ISO has the wrong size or checksum; remove it and retry: %s\n' \
			"$destination" >&2
		return 1
	fi

	release_path=$(kait2en_fedora_release_path "$canonical_url") || {
		printf 'Error: unsafe Fedora release URL: %s\n' "$canonical_url" >&2
		return 1
	}
	mirrorlist_url="https://mirrors.fedoraproject.org/mirrorlist?path=pub/fedora/linux/releases/$release_path"
	archive_url="https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/$release_path"
	# The preparation script runs as root but keeps network transfers under the
	# calling macOS account. Let that account create the curl output file too.
	mirror_file=$(kait2en_iso_run mktemp \
		"${TMPDIR:-/tmp}/kait2en-fedora-mirrors.XXXXXX") || return 1
	KAIT2EN_ISO_CANDIDATES=()

	if kait2en_iso_run curl --fail --location --retry 2 --retry-delay 2 \
		--connect-timeout 10 --max-time 30 --output "$mirror_file" \
		"$mirrorlist_url"; then
		while IFS= read -r line; do
			line=${line%$'\r'}
			case "$line" in
				https://*)
					if kait2en_add_iso_candidate "$line"; then
						mirror_count=$((mirror_count + 1))
						((mirror_count >= 10)) && break
					fi
					;;
			esac
		done <"$mirror_file"
	else
		printf 'Fedora mirror list is unavailable; using official fallbacks.\n'
	fi
	rm -f "$mirror_file"
	kait2en_add_iso_candidate "$canonical_url" || :
	kait2en_add_iso_candidate "$archive_url" || :

	printf 'Downloading %s...\n' "${destination##*/}"
	for candidate in "${KAIT2EN_ISO_CANDIDATES[@]}"; do
		attempted=$((attempted + 1))
		printf '  Source %d: %s\n' "$attempted" "$candidate"
		# An over-long partial makes every resume request fail with HTTP 416.
		if [[ -e "$partial" ]]; then
			partial_size=$(stat -f %z "$partial" 2>/dev/null || printf 0)
			((partial_size < expected_size)) || rm -f "$partial"
		fi
		if kait2en_download_candidate "$candidate" "$partial"; then
			if kait2en_verify_iso "$partial" "$expected_size" "$expected_sha"; then
				kait2en_iso_run mv "$partial" "$destination" || return 1
				printf 'Verified Fedora ISO: %s\n' "$destination"
				return 0
			fi
			printf '  Downloaded data failed size or SHA-256 verification; discarding it.\n' >&2
			rm -f "$partial"
		else
			status=$?
			case $status in
				23|27)
					printf 'Error: cannot write to %s (curl %d); check free space and permissions.\n' \
						"$partial" "$status" >&2
					return 1
					;;
			esac
			printf '  Source failed; trying the next official mirror.\n' >&2
		fi
	done

	if [[ -e "$partial" ]]; then
		printf 'Error: Fedora ISO download failed after %d sources; resumable partial file: %s\n' \
			"$attempted" "$partial" >&2
	else
		printf 'Error: Fedora ISO download failed after %d sources; no verified data was retained.\n' \
			"$attempted" >&2
	fi
	return 1
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
	set -Eeuo pipefail
	if [[ $# -ne 4 ]]; then
		printf 'Usage: %s DESTINATION CANONICAL_URL SIZE SHA256\n' "${0##*/}" >&2
		exit 2
	fi
	kait2en_download_iso "$1" "$2" "$3" "$4"
fi
