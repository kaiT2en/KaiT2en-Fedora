#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
TARGET="fedora-45"

usage() {
	printf 'Usage: %s [--target fedora-N]\n' "${0##*/}"
}

while (($# > 0)); do
	case "$1" in
		--target)
			(($# >= 2)) || { usage >&2; exit 2; }
			TARGET=$2
			shift 2
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			usage >&2
			exit 2
			;;
	esac
done

[[ "$TARGET" =~ ^fedora-[0-9]+$ ]] || {
	printf 'invalid installer target: %s\n' "$TARGET" >&2
	exit 2
}

TARGET_REL="auto-installer/targets/$TARGET.conf"
TARGET_FILE="$REPO_ROOT/$TARGET_REL"
[[ -f "$TARGET_FILE" ]] || {
	printf 'unsupported installer target: %s\n' "$TARGET" >&2
	exit 1
}

cd "$REPO_ROOT"
git ls-files --error-unmatch "$TARGET_REL" >/dev/null 2>&1 || {
	printf 'target file must be tracked by Git: %s\n' "$TARGET_REL" >&2
	exit 1
}

# shellcheck disable=SC1090
source "$TARGET_FILE"

required_variables=(
	TARGET_ID FEDORA_RELEASE ARCH DEFAULT_EDITION EDITIONS_FILE
	ISO_KERNEL_PATH ISO_INITRD_PATH ISO_KERNEL_RELEASE ANACONDA_VERSION
	KERNEL_DEVEL_URL KERNEL_DEVEL_SHA256 FEDORA_METALINK FEDORA_ARCHIVE_BASEURL
	CONTAINER_IMAGE
	ARTIFACT_BASENAME
)
for variable in "${required_variables[@]}"; do
	[[ -n ${!variable:-} ]] || {
		printf 'missing target variable %s in %s\n' "$variable" "$TARGET_REL" >&2
		exit 1
	}
done
[[ "$TARGET_ID" == "$TARGET" ]] || {
	printf 'TARGET_ID in %s does not match %s\n' "$TARGET_REL" "$TARGET" >&2
	exit 1
}
[[ "$EDITIONS_FILE" =~ ^fedora-[0-9]+-editions\.tsv$ ]] || {
	printf 'invalid EDITIONS_FILE in %s: %s\n' "$TARGET_REL" "$EDITIONS_FILE" >&2
	exit 1
}
EDITIONS_REL="auto-installer/targets/$EDITIONS_FILE"
[[ -s "$REPO_ROOT/$EDITIONS_REL" ]] || {
	printf 'edition catalog is missing: %s\n' "$EDITIONS_REL" >&2
	exit 1
}
git ls-files --error-unmatch "$EDITIONS_REL" >/dev/null 2>&1 || {
	printf 'edition catalog must be tracked by Git: %s\n' "$EDITIONS_REL" >&2
	exit 1
}
for command in curl git gzip sha256sum tar; do
	command -v "$command" >/dev/null 2>&1 || {
		printf 'missing command: %s\n' "$command" >&2
		exit 1
	}
done

if [[ -n ${CONTAINER_ENGINE:-} ]]; then
	ENGINE=$CONTAINER_ENGINE
elif command -v docker >/dev/null 2>&1; then
	ENGINE=docker
elif command -v podman >/dev/null 2>&1; then
	ENGINE=podman
else
	printf 'docker or podman is required\n' >&2
	exit 1
fi

SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]

container_options=(--user 0:0)
output_uid=$(id -u)
output_gid=$(id -g)
if [[ ${ENGINE##*/} == podman ]]; then
	# Container root maps to the invoking user, regardless of userns defaults.
	container_options+=("--userns=keep-id:uid=0,gid=0")
	output_uid=0
	output_gid=0
fi

git diff --quiet HEAD -- && git diff --cached --quiet HEAD -- || {
	printf 'tracked files must be committed before building the installer\n' >&2
	exit 1
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/kait2en-installer.XXXXXX")"
cleanup() {
	rm -rf "$WORK"
}
trap cleanup EXIT
mkdir -p "$WORK/source" "$WORK/out"

# Feed tar an explicit NUL-delimited Git index list. This neither reads nor
# packages untracked files from the working tree.
git ls-files -z | tar --null --files-from=- -cf "$WORK/source.tar"
tar -C "$WORK/source" -xf "$WORK/source.tar"

if [[ -n ${KERNEL_DEVEL_RPM:-} ]]; then
	cp -- "$KERNEL_DEVEL_RPM" "$WORK/kernel-devel.rpm"
else
	printf 'downloading kernel-devel for %s\n' "$ISO_KERNEL_RELEASE"
	curl --fail --location --retry 3 --output "$WORK/kernel-devel.rpm" "$KERNEL_DEVEL_URL"
fi
printf '%s  %s\n' "$KERNEL_DEVEL_SHA256" "$WORK/kernel-devel.rpm" |
	sha256sum --check --status || {
		printf 'kernel-devel checksum mismatch for %s\n' "$ISO_KERNEL_RELEASE" >&2
		exit 1
	}

printf 'building %s with %s\n' "$TARGET" "$ENGINE"
"$ENGINE" run --rm \
	"${container_options[@]}" \
	-e TARGET_ID="$TARGET_ID" \
	-e FEDORA_RELEASE="$FEDORA_RELEASE" \
	-e ARCH="$ARCH" \
	-e DEFAULT_EDITION="$DEFAULT_EDITION" \
	-e EDITIONS_FILE="$EDITIONS_FILE" \
	-e ISO_KERNEL_PATH="$ISO_KERNEL_PATH" \
	-e ISO_INITRD_PATH="$ISO_INITRD_PATH" \
	-e ISO_KERNEL_RELEASE="$ISO_KERNEL_RELEASE" \
	-e ANACONDA_VERSION="$ANACONDA_VERSION" \
	-e KERNEL_DEVEL_SHA256="$KERNEL_DEVEL_SHA256" \
	-e FEDORA_METALINK="$FEDORA_METALINK" \
	-e FEDORA_ARCHIVE_BASEURL="$FEDORA_ARCHIVE_BASEURL" \
	-e ARTIFACT_BASENAME="$ARTIFACT_BASENAME" \
	-e SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
	-e HOST_UID="$output_uid" \
	-e HOST_GID="$output_gid" \
	-v "$WORK:/work:Z" \
	"$CONTAINER_IMAGE" \
	bash -c 'set +e; bash /work/source/auto-installer/build-in-container.sh /work/source /work/kernel-devel.rpm /work/out; status=$?; chown -R "$HOST_UID:$HOST_GID" /work; exit "$status"'

mkdir -p "$REPO_ROOT/dist"
install -m 0644 "$WORK/out/$ARTIFACT_BASENAME.zip" "$REPO_ROOT/dist/"
install -m 0644 "$WORK/out/$ARTIFACT_BASENAME.zip.sha256" "$REPO_ROOT/dist/"

printf 'Installer artifact: %s\n' "$REPO_ROOT/dist/$ARTIFACT_BASENAME.zip"
printf 'Checksum:        %s\n' "$REPO_ROOT/dist/$ARTIFACT_BASENAME.zip.sha256"
