#!/usr/bin/env bash

# Keep this script compatible with the Bash 3.2 shipped by macOS.
set -Eeuo pipefail
shopt -s nullglob

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
SCRIPT_PATH="$SCRIPT_DIR/${BASH_SOURCE[0]##*/}"
KIT_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
ORIGINAL_ARGC=$#
ORIGINAL_ARGS=("$@")
ISO_PATH=
FEDORA_DISK=
FIRMWARE_DIR=
DOWNLOAD_DIR=
REQUESTED_EDITION=
REUSE_MEDIA=0
LIST_EDITIONS=0
WORK=
DISK_TOUCHED=0
EFI_MOUNT=

EDITION_IDS=()
EDITION_NAMES=()
EDITION_VARIANTS=()
EDITION_SUBVARIANTS=()
EDITION_FILES=()
EDITION_URLS=()
EDITION_SIZES=()
EDITION_SHAS=()
EDITION_INDEX=-1

usage() {
	cat <<EOF
Usage: ${0##*/} [options]

Options:
  --edition ID       Select a Fedora edition without the menu
  --iso FILE         Use a supported, already downloaded Fedora ISO
  --disk /dev/diskN  Select the external target disk without the menu
  --download-dir DIR Store automatically downloaded ISOs in DIR
  --firmware-dir DIR Use Apple Wi-Fi firmware from DIR
  --reuse-media      Reuse an existing matching Fedora image
  --list-editions    List supported Fedora editions and exit
  -h, --help         Show this help
EOF
}

die() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

cleanup() {
	if [[ -n "$EFI_MOUNT" ]]; then
		/sbin/umount "$EFI_MOUNT" >/dev/null 2>&1 || :
	fi
	if [[ $DISK_TOUCHED -eq 1 && -n "$FEDORA_DISK" ]]; then
		diskutil unmountDisk "$FEDORA_DISK" >/dev/null 2>&1 || :
	fi
	if [[ -n "$WORK" && -d "$WORK" ]]; then
		rm -rf "$WORK"
	fi
}
trap cleanup EXIT

on_error() {
	local status=$?
	local line=$1
	if ((BASH_SUBSHELL > 0)); then
		exit "$status"
	fi
	printf 'Error: command failed at line %s (status %s)\n' "$line" "$status" >&2
	exit "$status"
}
trap 'on_error "$LINENO"' ERR

plist_value() {
	diskutil info -plist "$1" |
		plutil -extract "$2" raw -o - - 2>/dev/null
}

format_size() {
	awk -v bytes="$1" 'BEGIN {printf "%.1f GB", bytes / 1000000000}'
}

check_startup_security() {
	local policy
	policy=$(/usr/sbin/nvram \
		94b73556-2197-4702-82a8-3e1337dafbfb:AppleSecureBootPolicy \
		2>/dev/null | awk '{print $NF}' || :)
	if [[ "$policy" == %00 ]]; then
		printf 'Good: Secure Boot has been disabled.\n'
		return
	fi
	printf 'Warning: Secure Boot is not disabled or could not be verified.\n'
	printf 'Before booting the USB drive:\n'
	printf '  1. Shut down the Mac, turn it back on, and hold Command-R to enter Recovery.\n'
	printf '  2. Open Utilities > Startup Security Utility.\n'
	printf '  3. Set Secure Boot to No Security.\n'
	printf '  4. Set Allowed Boot Media to Allow booting from external or removable media.\n'
}

pick_one() {
	local label=$1
	shift
	local files=("$@")

	((${#files[@]} == 1)) ||
		die "expected exactly one $label file, found ${#files[@]}"
	[[ -s "${files[0]}" ]] || die "$label file is empty: ${files[0]}"
	printf '%s\n' "${files[0]}"
}

copy_file_contents() {
	local source=$1
	local destination=$2

	# Apple firmware can carry BSD flags that an unprivileged user cannot
	# reproduce. Copy only the bytes needed by Linux, not macOS metadata.
	/bin/cat "$source" >"$destination"
	chmod 0644 "$destination"
}

copy_firmware_candidates() {
	local source_dir=$1
	local destination=$2
	local candidate

	[[ -d "$source_dir" ]] || die "firmware directory not found: $source_dir"
	for candidate in \
		"$source_dir"/*.trx \
		"$source_dir"/*.clmb \
		"$source_dir"/*.txcb \
		"$source_dir"/P-*.txt; do
		[[ -f "$candidate" ]] || continue
		copy_file_contents "$candidate" "$destination/${candidate##*/}"
	done
}

collect_macos_firmware() {
	local destination=$1
	local path_list="$WORK/macos-firmware-paths.txt"
	local firmware_path

	printf 'Looking for Apple Wi-Fi firmware used during the current macOS boot...\n'
	(
		log show --last boot --info \
			--predicate 'eventMessage contains "/usr/share/firmware/"' 2>/dev/null |
			grep 'Copying' |
			grep -oE '"[^"]*"' |
			tr -d '"' |
			sort -u >"$path_list"
	) || :

	while IFS= read -r firmware_path; do
		[[ "$firmware_path" == /usr/share/firmware/wifi/* ]] || continue
		[[ -f "$firmware_path" ]] || continue
		case "${firmware_path##*/}" in
			*.trx|*.clmb|*.txcb|P-*.txt)
				copy_file_contents "$firmware_path" \
					"$destination/${firmware_path##*/}"
				;;
		esac
	done <"$path_list"
}

load_editions() {
	local catalog=$1
	local id display variant subvariant filename url size sha
	local seen=" "

	[[ -s "$catalog" ]] || die "installer edition catalog is missing: $catalog"
	while IFS=$'\t' read -r id display variant subvariant filename url size sha; do
		[[ -n "$id" ]] || continue
		[[ "$id" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "invalid edition ID: $id"
		[[ "$seen" != *" $id "* ]] || die "duplicate edition ID: $id"
		[[ -n "$display" && -n "$variant" && -n "$subvariant" ]] ||
			die "incomplete edition entry: $id"
		[[ "$filename" == *.iso && "$url" == https://download.fedoraproject.org/* ]] ||
			die "invalid Fedora download for edition: $id"
		[[ "$size" =~ ^[0-9]+$ && "$sha" =~ ^[0-9a-f]{64}$ ]] ||
			die "invalid size or checksum for edition: $id"
		seen="$seen$id "
		EDITION_IDS+=("$id")
		EDITION_NAMES+=("$display")
		EDITION_VARIANTS+=("$variant")
		EDITION_SUBVARIANTS+=("$subvariant")
		EDITION_FILES+=("$filename")
		EDITION_URLS+=("$url")
		EDITION_SIZES+=("$size")
		EDITION_SHAS+=("$sha")
	done <"$catalog"
	((${#EDITION_IDS[@]} > 0)) || die 'the installer edition catalog is empty'
}

find_edition_by_id() {
	local wanted=$1
	local index
	for ((index = 0; index < ${#EDITION_IDS[@]}; index++)); do
		if [[ "${EDITION_IDS[index]}" == "$wanted" ]]; then
			printf '%s\n' "$index"
			return 0
		fi
	done
	return 1
}

find_edition_by_sha() {
	local wanted=$1
	local index
	for ((index = 0; index < ${#EDITION_SHAS[@]}; index++)); do
		if [[ "${EDITION_SHAS[index]}" == "$wanted" ]]; then
			printf '%s\n' "$index"
			return 0
		fi
	done
	return 1
}

select_edition() {
	local answer default_index index

	if [[ -n "$REQUESTED_EDITION" ]]; then
		EDITION_INDEX=$(find_edition_by_id "$REQUESTED_EDITION") ||
			die "unsupported Fedora edition: $REQUESTED_EDITION"
		return
	fi

	default_index=$(find_edition_by_id "$DEFAULT_EDITION") ||
		die "default edition is not in the catalog: $DEFAULT_EDITION"
	printf '\nChoose a Fedora desktop:\n'
	for ((index = 0; index < ${#EDITION_IDS[@]}; index++)); do
		printf '  %2d) %s' "$((index + 1))" "${EDITION_NAMES[index]}"
		if ((index == default_index)); then
			printf ' (default)'
		fi
		printf '\n'
	done
	printf 'Selection [%d]: ' "$((default_index + 1))"
	read -r answer
	[[ -n "$answer" ]] || answer=$((default_index + 1))
	[[ "$answer" =~ ^[0-9]+$ ]] || die 'edition selection must be a number'
	((answer >= 1 && answer <= ${#EDITION_IDS[@]})) || die 'edition selection is out of range'
	EDITION_INDEX=$((answer - 1))
}

list_external_disks() {
	local disk internal whole_disk media size
	DISK_PATHS=()
	DISK_NAMES=()
	DISK_SIZES=()
	while IFS= read -r disk; do
		[[ "$disk" =~ ^/dev/disk[0-9]+$ ]] || continue
		internal=$(plist_value "$disk" Internal || :)
		whole_disk=$(plist_value "$disk" WholeDisk || :)
		[[ "$internal" == false && "$whole_disk" == true ]] || continue
		media=$(plist_value "$disk" MediaName || :)
		size=$(plist_value "$disk" TotalSize || :)
		[[ "$size" =~ ^[0-9]+$ ]] || continue
		DISK_PATHS+=("$disk")
		DISK_NAMES+=("${media:-unknown external disk}")
		DISK_SIZES+=("$size")
	done < <(diskutil list external physical | awk '/^\/dev\/disk[0-9]+/{print $1}')
}

select_disk() {
	local answer index
	if [[ -n "$FEDORA_DISK" ]]; then
		return
	fi
	list_external_disks
	((${#DISK_PATHS[@]} > 0)) || die 'no external physical disk was found'
	printf '\nChoose the USB drive to erase:\n'
	for ((index = 0; index < ${#DISK_PATHS[@]}; index++)); do
		printf '  %2d) %s — %s (%s)\n' "$((index + 1))" \
			"${DISK_PATHS[index]}" "${DISK_NAMES[index]}" \
			"$(format_size "${DISK_SIZES[index]}")"
	done
	printf 'Selection: '
	read -r answer
	[[ "$answer" =~ ^[0-9]+$ ]] || die 'disk selection must be a number'
	((answer >= 1 && answer <= ${#DISK_PATHS[@]})) || die 'disk selection is out of range'
	FEDORA_DISK=${DISK_PATHS[answer - 1]}
}

run_as_calling_user() {
	if [[ ${SUDO_USER:-root} != root ]]; then
		sudo -u "$SUDO_USER" "$@"
	else
		"$@"
	fi
}

verify_iso() {
	local path=$1 expected_size=$2 expected_sha=$3 actual_size actual_sha
	[[ -f "$path" ]] || return 1
	actual_size=$(stat -f %z "$path")
	[[ "$actual_size" == "$expected_size" ]] || return 1
	actual_sha=$(shasum -a 256 "$path" | awk '{print $1}')
	[[ "$actual_sha" == "$expected_sha" ]]
}

download_iso() {
	local destination=$1 url=$2 expected_size=$3 expected_sha=$4
	local partial="$destination.part"

	if [[ -e "$destination" ]]; then
		if verify_iso "$destination" "$expected_size" "$expected_sha"; then
			printf 'Using verified cached ISO: %s\n' "$destination"
			return
		fi
		die "cached ISO has the wrong size or checksum; remove it and retry: $destination"
	fi

	printf 'Downloading %s...\n' "${destination##*/}"
	run_as_calling_user curl --fail --location --retry 3 \
		--continue-at - --output "$partial" "$url"
	verify_iso "$partial" "$expected_size" "$expected_sha" ||
		die "downloaded ISO failed verification: $partial"
	run_as_calling_user mv "$partial" "$destination"
	printf 'Verified Fedora ISO: %s\n' "$destination"
}

while (($# > 0)); do
	case "$1" in
		--edition)
			(($# >= 2)) || { usage >&2; exit 2; }
			REQUESTED_EDITION=$2
			shift 2
			;;
		--iso)
			(($# >= 2)) || { usage >&2; exit 2; }
			ISO_PATH=$2
			shift 2
			;;
		--disk)
			(($# >= 2)) || { usage >&2; exit 2; }
			FEDORA_DISK=$2
			shift 2
			;;
		--download-dir)
			(($# >= 2)) || { usage >&2; exit 2; }
			DOWNLOAD_DIR=$2
			shift 2
			;;
		--firmware-dir)
			(($# >= 2)) || { usage >&2; exit 2; }
			FIRMWARE_DIR=$2
			shift 2
			;;
		--reuse-media)
			REUSE_MEDIA=1
			shift
			;;
		--list-editions)
			LIST_EDITIONS=1
			shift
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

for file in installer-target.conf installer-editions.tsv grub.cfg.in kait2en-input-initramfs.img; do
	[[ -s "$KIT_ROOT/$file" ]] || die "installer kit file is missing: $file"
done
# shellcheck disable=SC1091
source "$KIT_ROOT/installer-target.conf"
[[ -n ${TARGET_ID:-} && -n ${FEDORA_RELEASE:-} && -n ${DEFAULT_EDITION:-} ]] ||
	die 'installer-target.conf is incomplete'
load_editions "$KIT_ROOT/installer-editions.tsv"

if ((LIST_EDITIONS == 1)); then
	for ((index = 0; index < ${#EDITION_IDS[@]}; index++)); do
		printf '%-12s %s\n' "${EDITION_IDS[index]}" "${EDITION_NAMES[index]}"
	done
	exit 0
fi

[[ $(uname -s) == Darwin ]] || die 'this installer preparation script must run on macOS'
if [[ $EUID -ne 0 ]]; then
	printf 'Administrator access is required to prepare the USB drive.\n'
	if ((ORIGINAL_ARGC == 0)); then
		exec sudo /bin/bash "$SCRIPT_PATH"
	fi
	exec sudo /bin/bash "$SCRIPT_PATH" "${ORIGINAL_ARGS[@]}"
fi

for command in diskutil plutil shasum stat dd cpio gzip find sort cmp install \
	log awk df tr curl sudo dscl sed; do
	command -v "$command" >/dev/null 2>&1 || die "required macOS command is missing: $command"
done
[[ -x /sbin/mount_msdos && -x /sbin/umount ]] ||
	die 'required macOS FAT mount tools are missing'
[[ -x /usr/sbin/nvram ]] || die 'required macOS NVRAM tool is missing'

check_startup_security
printf '\n'

if [[ -n "$ISO_PATH" ]]; then
	[[ -f "$ISO_PATH" ]] || die "ISO file not found: $ISO_PATH"
	printf 'Identifying and verifying the Fedora ISO...\n'
	actual_iso_sha=$(shasum -a 256 "$ISO_PATH" | awk '{print $1}')
	matched_index=$(find_edition_by_sha "$actual_iso_sha") ||
		die 'the ISO is not one of the supported, unmodified Fedora desktop images'
	if [[ -n "$REQUESTED_EDITION" ]]; then
		requested_index=$(find_edition_by_id "$REQUESTED_EDITION") ||
			die "unsupported Fedora edition: $REQUESTED_EDITION"
		[[ "$matched_index" -eq "$requested_index" ]] ||
			die "the ISO does not match requested edition $REQUESTED_EDITION"
	fi
	EDITION_INDEX=$matched_index
else
	select_edition
fi
expected_name=${EDITION_FILES[EDITION_INDEX]}
expected_url=${EDITION_URLS[EDITION_INDEX]}
expected_size=${EDITION_SIZES[EDITION_INDEX]}
expected_sha=${EDITION_SHAS[EDITION_INDEX]}
if [[ -n "$ISO_PATH" ]]; then
	verify_iso "$ISO_PATH" "$expected_size" "$expected_sha" ||
		die "ISO verification failed for $expected_name"
fi

select_disk
[[ "$FEDORA_DISK" =~ ^/dev/disk[0-9]+$ ]] ||
	die 'provide a whole external disk such as /dev/disk4'
internal=$(plist_value "$FEDORA_DISK" Internal || :)
whole=$(plist_value "$FEDORA_DISK" WholeDisk || :)
[[ "$internal" == false ]] || die "$FEDORA_DISK is an internal disk; refusing to erase it"
[[ "$whole" == true ]] || die "$FEDORA_DISK is not a whole disk"

if [[ -z "$ISO_PATH" ]]; then
	if [[ -z "$DOWNLOAD_DIR" ]]; then
		calling_user=${SUDO_USER:-root}
		if [[ "$calling_user" != root ]]; then
			calling_home=$(dscl . -read "/Users/$calling_user" NFSHomeDirectory |
				awk 'NR == 1 {print $2}')
		else
			calling_home=$HOME
		fi
		[[ -n "$calling_home" ]] || die 'could not determine the calling user home directory'
		DOWNLOAD_DIR="$calling_home/Downloads"
	fi
	[[ -d "$DOWNLOAD_DIR" ]] || die "download directory not found: $DOWNLOAD_DIR"
	ISO_PATH="$DOWNLOAD_DIR/$expected_name"
	download_iso "$ISO_PATH" "$expected_url" "$expected_size" "$expected_sha"
fi

WORK=$(mktemp -d /tmp/kait2en-installer.XXXXXX)
pvd_sector="$WORK/iso-pvd-sector"
LC_ALL=C dd if="$ISO_PATH" of="$pvd_sector" bs=512 skip=64 count=1 2>/dev/null
pvd_magic=$(LC_ALL=C dd if="$pvd_sector" bs=1 skip=1 count=5 2>/dev/null)
[[ "$pvd_magic" == CD001 ]] || die 'the Fedora ISO has no ISO9660 primary volume descriptor'
iso_volume_label=$(
	LC_ALL=C dd if="$pvd_sector" bs=1 skip=40 count=32 2>/dev/null |
		tr -d ' '
)
[[ "$iso_volume_label" =~ ^[A-Za-z0-9_.-]+$ ]] ||
	die "unexpected Fedora ISO volume label: ${iso_volume_label:-empty}"
sed "s|@ISO_VOLUME_LABEL@|$iso_volume_label|g" \
	"$KIT_ROOT/grub.cfg.in" >"$WORK/grub.cfg"
! grep -Fq '@ISO_VOLUME_LABEL@' "$WORK/grub.cfg" ||
	die 'could not render the Fedora volume label into GRUB configuration'

firmware_stage="$WORK/firmware"
archive_root="$WORK/archive-root"
mkdir -p "$firmware_stage" "$archive_root/kait2en-wifi-firmware"
if [[ -n "$FIRMWARE_DIR" ]]; then
	copy_firmware_candidates "$FIRMWARE_DIR" "$firmware_stage"
else
	collect_macos_firmware "$firmware_stage"
fi

trx_file=$(pick_one '.trx firmware' "$firmware_stage"/*.trx)
clm_file=$(pick_one '.clmb regulatory blob' "$firmware_stage"/*.clmb)
txcap_file=$(pick_one '.txcb TxCap blob' "$firmware_stage"/*.txcb)
nvram_file=$(pick_one 'P-*.txt NVRAM' "$firmware_stage"/P-*.txt)

clm_stem=$(basename "$clm_file" .clmb)
txcap_stem=$(basename "$txcap_file" .txcb)
nvram_name=$(basename "$nvram_file")
[[ "$clm_stem" == "$txcap_stem" ]] || die '.clmb and .txcb board names differ'
if [[ "$nvram_name" =~ ^P-([^_]+)_M-([^_]+)_V-([^_]+)__m-(.+)\.txt$ ]]; then
	[[ "${BASH_REMATCH[1]}" == "$clm_stem" ]] ||
		die 'NVRAM board name does not match .clmb/.txcb board name'
else
	die "unexpected NVRAM filename: $nvram_name"
fi

printf 'Validated Apple Wi-Fi firmware:\n'
printf '  %s\n' "${trx_file##*/}" "${clm_file##*/}" \
	"${txcap_file##*/}" "${nvram_file##*/}"

firmware_archive_dir="$archive_root/kait2en-wifi-firmware"
install -m 0644 "$trx_file" "$clm_file" "$txcap_file" "$nvram_file" \
	"$firmware_archive_dir/"
(
	cd "$firmware_archive_dir"
	shasum -a 256 "${trx_file##*/}" "${clm_file##*/}" \
		"${txcap_file##*/}" "${nvram_file##*/}" >SHA256SUMS
)
firmware_image="$WORK/kait2en-wifi-initramfs.img"
(
	cd "$archive_root"
	export COPYFILE_DISABLE=1
	find . -print | LC_ALL=C sort |
		cpio -o -H newc 2>/dev/null |
		gzip -n -9 >"$firmware_image"
)
gzip -t "$firmware_image"

media_name=$(plist_value "$FEDORA_DISK" MediaName || :)
disk_size=$(plist_value "$FEDORA_DISK" TotalSize || :)
iso_size=$(stat -f %z "$ISO_PATH")
[[ "$disk_size" =~ ^[0-9]+$ && "$disk_size" -ge "$iso_size" ]] ||
	die 'the selected disk is smaller than the Fedora ISO'

printf '\nFedora edition: %s\n' "${EDITION_NAMES[EDITION_INDEX]}"
printf 'Fedora ISO:     %s\n' "$ISO_PATH"
printf 'Target disk:    %s (%s, %s)\n' "$FEDORA_DISK" \
	"${media_name:-unknown}" "$(format_size "$disk_size")"
if ((REUSE_MEDIA == 1)); then
	printf 'The existing Fedora image will be reused. Type REUSE %s to continue: ' \
		"${FEDORA_DISK##*/}"
	read -r confirmation
	[[ "$confirmation" == "REUSE ${FEDORA_DISK##*/}" ]] || die 'cancelled'
else
	printf 'All data on %s will be erased. Type ERASE %s to continue: ' \
		"$FEDORA_DISK" "${FEDORA_DISK##*/}"
	read -r confirmation
	[[ "$confirmation" == "ERASE ${FEDORA_DISK##*/}" ]] || die 'cancelled'
fi

DISK_TOUCHED=1
diskutil unmountDisk "$FEDORA_DISK" >/dev/null
raw_disk="/dev/r${FEDORA_DISK#/dev/}"
if ((REUSE_MEDIA == 1)); then
	printf 'Validating the existing Fedora image before adding KaiT2en...\n'
else
	printf 'Writing the verified Fedora ISO. This can take several minutes...\n'
	dd if="$ISO_PATH" of="$raw_disk" bs=4m
	sync
fi

iso_partition="${FEDORA_DISK}s1"
efi_partition="${FEDORA_DISK}s2"
for _attempt in 1 2 3 4 5 6 7 8 9 10; do
	[[ -e "$efi_partition" ]] && break
	sleep 1
done
[[ -e "$iso_partition" && -e "$efi_partition" ]] ||
	die 'Fedora partitions did not appear after writing the ISO'

written_pvd="$WORK/written-pvd-sector"
LC_ALL=C dd if="$raw_disk" of="$written_pvd" bs=512 skip=64 count=1 2>/dev/null
written_magic=$(LC_ALL=C dd if="$written_pvd" bs=1 skip=1 count=5 2>/dev/null)
[[ "$written_magic" == CD001 ]] || die 'the written media has no ISO9660 primary volume descriptor'
written_label=$(
	LC_ALL=C dd if="$written_pvd" bs=1 skip=40 count=32 2>/dev/null |
		tr -d ' '
)
[[ "$written_label" == "$iso_volume_label" ]] ||
	die "unexpected Fedora volume label after writing: ${written_label:-unknown}"

efi_mount="$WORK/efi"
for _attempt in 1 2 3; do
	mkdir -p "$efi_mount"
	if [[ -e "$efi_partition" ]] &&
		/sbin/mount_msdos "$efi_partition" "$efi_mount"; then
		EFI_MOUNT=$efi_mount
		break
	fi
	sleep 2
done
[[ -n "$EFI_MOUNT" ]] ||
	die 'could not mount the Fedora EFI partition; reconnect the USB drive and retry with --reuse-media'

grub_destination="$efi_mount/EFI/BOOT/grub.cfg"
[[ -f "$grub_destination" ]] || die 'the original Fedora GRUB configuration is missing'
kait2en_directory="$efi_mount/EFI/BOOT/kait2en"
required_bytes=$((
	$(stat -f %z "$WORK/grub.cfg") +
	$(stat -f %z "$KIT_ROOT/kait2en-input-initramfs.img") +
	$(stat -f %z "$firmware_image") + 1048576
))
available_kib=$(df -k "$efi_mount" | awk 'END {print $4}')
[[ "$available_kib" =~ ^[0-9]+$ && $((available_kib * 1024)) -ge $required_bytes ]] ||
	die 'the Fedora EFI partition does not have enough free space for KaiT2en'
install -d -m 0755 "$kait2en_directory"
cp -p "$grub_destination" "$grub_destination.fedora-original"
install -m 0644 "$WORK/grub.cfg" "$grub_destination"
install -m 0644 "$KIT_ROOT/kait2en-input-initramfs.img" \
	"$kait2en_directory/kait2en-input-initramfs.img"
install -m 0644 "$firmware_image" \
	"$kait2en_directory/kait2en-wifi-initramfs.img"
install -m 0644 "$firmware_archive_dir/SHA256SUMS" \
	"$kait2en_directory/wifi-firmware.sha256"

cmp "$WORK/grub.cfg" "$grub_destination"
cmp "$KIT_ROOT/kait2en-input-initramfs.img" \
	"$kait2en_directory/kait2en-input-initramfs.img"
cmp "$firmware_image" "$kait2en_directory/kait2en-wifi-initramfs.img"
sync
/sbin/umount "$efi_mount"
EFI_MOUNT=
diskutil unmountDisk "$FEDORA_DISK" >/dev/null 2>&1 || :
DISK_TOUCHED=0

printf '\nKaiT2en Fedora installer prepared successfully.\n'
printf 'The ISO was verified OK.\n'
printf '\nNext steps:\n'
printf '  - Boot your T2 Mac from this USB drive.\n'
printf '  - Keyboard and trackpad should work in the Fedora installer. Wi-Fi is expected after the first boot.\n'
printf '  - Complete the Fedora installation, remove the USB drive, and boot the installed system.\n'
printf '  - Sign in to Fedora. The KaiT2en installation will continue automatically in a terminal.\n'
