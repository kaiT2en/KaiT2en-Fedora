#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# Adapted from the KaiT2en guide. Special thanks to Deqrocks and the KaiT2en contributors.
# EFI partition transfer method credit goes to the T2 Linux guide.
set -euo pipefail
shopt -s nullglob

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"

_find_fw_dir() {
    local cand
    for cand in \
        "${SCRIPT_DIR}/firmware" \
        "firmware" \
        "${HOME}/Desktop/firmware" \
        "/boot/firmware" \
        "/boot/efi/firmware" \
        "/efi/firmware"; do
        if [[ -d "$cand" ]]; then
            local has_fw
            has_fw=$(find "$cand" -maxdepth 1 \( -name '*.trx' -o -name '*.bin' \) 2>/dev/null | head -n 1)
            if [[ -n "$has_fw" ]]; then
                printf '%s\n' "$cand"
                return 0
            fi
        fi
    done
    printf '%s\n' "${SCRIPT_DIR}/firmware"
}

SOURCE_DIR="$(_find_fw_dir)"
readonly SOURCE_DIR
readonly DEST_DIR="/lib/firmware/brcm"

_info() {
    echo "[*] $*"
}

_success() {
    echo "[+] $*"
}

_warn() {
    echo "[!] $*" >&2
}

_error() {
    echo "[-] $*" >&2
    exit 1
}

_pick_one() {
    local label="$1"
    shift
    local files=("$@")

    if (( ${#files[@]} == 0 )); then
        _error "No ${label} file found in ${SOURCE_DIR}."
    fi

    if (( ${#files[@]} > 1 )); then
        _warn "Found more than one ${label} file:"
        printf '  %s\n' "${files[@]}" >&2
        _error "Keep only the matching file in ${SOURCE_DIR} and run the script again."
    fi

    printf '%s\n' "${files[0]}"
}

_check_conflicting_pkg() {
    local pkgs=("apple-bcm-firmware" "apple-firmware" "t2-apple-firmware")
    local found_pkg="" remove_cmd=""

    for pkg in "${pkgs[@]}"; do
        if command -v pacman &>/dev/null && pacman -Q "$pkg" &>/dev/null; then
            found_pkg="$pkg"
            remove_cmd="sudo pacman -R ${pkg}"
            break
        elif command -v dpkg-query &>/dev/null && dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q 'ok installed'; then
            found_pkg="$pkg"
            remove_cmd="sudo apt remove ${pkg}"
            break
        elif command -v rpm &>/dev/null && rpm -q "$pkg" &>/dev/null; then
            found_pkg="$pkg"
            remove_cmd="sudo dnf remove ${pkg}"
            break
        elif command -v xbps-query &>/dev/null && xbps-query "$pkg" &>/dev/null; then
            found_pkg="$pkg"
            remove_cmd="sudo xbps-remove ${pkg}"
            break
        elif command -v apk &>/dev/null && apk info -e "$pkg" &>/dev/null; then
            found_pkg="$pkg"
            remove_cmd="sudo apk del ${pkg}"
            break
        fi
    done

    if [[ -n "$found_pkg" ]]; then
        _warn "Detected conflicting package '${found_pkg}' installed via your package manager."
        _error "Please remove it first before installing firmware manually: '${remove_cmd}'"
    fi
}

get_fw() {
    local bt_base="/usr/share/firmware/bluetooth"
    local wifi_files bt_chipset bt_files=""
    local stage_dir="" tmp_mount="" mounted_by_us=false

    wifi_files=$(/usr/sbin/ioreg -l -w0 2>/dev/null | grep '"RequestedFiles"' | grep -oE '"[^"]+"' | tr -d '"' | grep -E '/([^/]+\.(trx|clmb|txcb)|P-[^/]+\.txt)$' | sed 's|^|/usr/share/firmware/wifi/|' | sort -u)
    bt_chipset=$(system_profiler SPBluetoothDataType 2>/dev/null | grep "Chipset:" | grep -oE 'BCM_[0-9A-Z]+' | tr -d '_' | head -n 1)

    if [[ -n "$bt_chipset" && -d "$bt_base" ]]; then
        bt_files=$(find "$bt_base" -maxdepth 1 -type f \( -iname "${bt_chipset}*.bin" -o -iname "${bt_chipset}*.ptb" \) | sort)
    fi

    echo "[Wi-Fi Firmware]"
    if [[ -n "$wifi_files" ]]; then
        echo "$wifi_files"
    else
        _info "No Wi-Fi firmware files found."
    fi

    echo "[Bluetooth PCIe Firmware]"
    if [[ -n "$bt_files" ]]; then
        echo "$bt_files"
    else
        _info "No PCIe Bluetooth firmware found for ${bt_chipset:-the detected chip}."
    fi
    echo ""

    stage_dir=$(mktemp -d "${TMPDIR:-/tmp}/brcm_fw_stage.XXXXXX")

    _cleanup_get_fw() {
        if [[ "$mounted_by_us" == true && -n "$tmp_mount" && -d "$tmp_mount" ]]; then
            sudo umount "$tmp_mount" 2>/dev/null || diskutil unmount "$tmp_mount" 2>/dev/null || true
            rmdir "$tmp_mount" 2>/dev/null || true
        fi
        if [[ -n "$stage_dir" && -d "$stage_dir" ]]; then
            rm -rf "$stage_dir" 2>/dev/null || true
        fi
    }
    trap _cleanup_get_fw EXIT INT TERM

    echo "[Gathering Files into Temporary Staging Area]"
    if [[ -n "$wifi_files" ]]; then
        while IFS= read -r f; do
            [[ -n "$f" ]] && cp "$f" "$stage_dir/" && echo "  staged: $(basename "$f")"
        done <<< "$wifi_files"
    fi

    if [[ -n "$bt_files" ]]; then
        while IFS= read -r f; do
            [[ -n "$f" ]] && cp "$f" "$stage_dir/" && echo "  staged: $(basename "$f")"
        done <<< "$bt_files"
    fi

    local staged_count
    staged_count=$(find "$stage_dir" -type f | wc -l | tr -d ' ')
    if (( staged_count == 0 )); then
        _error "No firmware files could be gathered from macOS."
    fi

    echo ""
    # If a custom destination was passed as $1, copy there and return
    if [[ -n "${1:-}" ]]; then
        mkdir -p "$1"
        cp -R "$stage_dir"/* "$1/"
        _success "Firmware saved to: $1"
        _cleanup_get_fw
        trap - EXIT INT TERM
        return 0
    fi

    local saved_to_efi=false
    local efi_dev=""

    # Attempt to locate and write to the internal EFI System Partition (ESP)
    efi_dev=$(diskutil list internal 2>/dev/null | awk '$2 == "EFI" {print $NF; exit}')
    if [[ -z "$efi_dev" ]]; then
        efi_dev=$(diskutil list 2>/dev/null | awk '$2 == "EFI" {print $NF; exit}')
    fi

    if [[ -n "$efi_dev" ]]; then
        _info "Detected EFI partition: /dev/${efi_dev}"
        local existing_mount
        existing_mount=$(mount | awk -v dev="/dev/${efi_dev}" '$1 == dev {print $3; exit}')

        if [[ -n "$existing_mount" && -d "$existing_mount" ]]; then
            _info "EFI partition is already mounted at: ${existing_mount}"
            mkdir -p "${existing_mount}/firmware"
            cp -R "$stage_dir"/* "${existing_mount}/firmware/"
            saved_to_efi=true
        else
            tmp_mount=$(mktemp -d "${TMPDIR:-/tmp}/efi_mount.XXXXXX")
            _info "Mounting /dev/${efi_dev} to temporary directory ${tmp_mount}..."
            if sudo mount -t msdos "/dev/${efi_dev}" "$tmp_mount" 2>/dev/null || sudo mount_msdos "/dev/${efi_dev}" "$tmp_mount" 2>/dev/null; then
                mounted_by_us=true
                mkdir -p "${tmp_mount}/firmware"
                cp -R "$stage_dir"/* "${tmp_mount}/firmware/"
                saved_to_efi=true
                _info "Unmounting temporary EFI mount point..."
                sudo umount "$tmp_mount" 2>/dev/null || diskutil unmount "$tmp_mount" 2>/dev/null || true
                rmdir "$tmp_mount" 2>/dev/null || true
                tmp_mount=""
                mounted_by_us=false
            else
                _warn "Could not mount /dev/${efi_dev} as FAT32/vfat."
                rmdir "$tmp_mount" 2>/dev/null || true
                tmp_mount=""
            fi
        fi
    else
        _warn "No EFI partition detected."
    fi

    echo ""
    if [[ "$saved_to_efi" == true ]]; then
        _success "Firmware successfully copied directly to EFI partition (/dev/${efi_dev})."
        _info "When booting into Linux, firmware will be automatically detected under /boot/firmware or /boot/efi/firmware."
    else
        local desktop_dir="${HOME}/Desktop"
        local fallback_dest="${desktop_dir}/firmware"
        [[ -d "$desktop_dir" ]] || fallback_dest="${SCRIPT_DIR}/firmware"

        mkdir -p "$fallback_dest"
        cp -R "$stage_dir"/* "$fallback_dest/"

        _warn "Could not write directly to the EFI partition."
        _warn "Firmware folder has been saved to: ${fallback_dest}"
        echo ""
        _info "Next Steps to use this firmware on Linux:"
        echo "  - Copy the 'firmware' folder onto your EFI partition (e.g., /boot/firmware or /boot/efi/firmware)."
        echo "  - OR copy the 'firmware' folder onto a USB flash drive to transfer it to Linux."
        echo "  - OR keep it on your Desktop alongside brcm_fw.sh on Linux."
        echo ""
        _success "Firmware saved to: ${fallback_dest}"
    fi

    _cleanup_get_fw
    trap - EXIT INT TERM
}

install_wl_fw() {
    [[ -d "$SOURCE_DIR" ]] || _error "Need to copy firmware from macOS first."

    local trx_file clm_file txcap_file nvram_file
    trx_file="$(_pick_one '.trx firmware' "${SOURCE_DIR}"/*.trx)"
    clm_file="$(_pick_one '.clmb regulatory blob' "${SOURCE_DIR}"/*.clmb)"
    txcap_file="$(_pick_one '.txcb TxCap blob' "${SOURCE_DIR}"/*.txcb)"
    nvram_file="$(_pick_one 'P-*.txt NVRAM file' "${SOURCE_DIR}"/P-*.txt)"

    local requested_bin target_bin target_prefix
    requested_bin="$(
        journalctl -b -k |
        sed -n 's/.*Direct firmware load for \(brcm\/brcmfmac[^ ]*\.bin\) failed with error -2.*/\1/p' |
        head -n 1
    )"

    if [[ -z "${requested_bin}" ]]; then
        _error "No missing brcmfmac .bin firmware request found in the current boot log."
    fi

    target_bin="${requested_bin#brcm/}"

    if [[ "${requested_bin}" == "${target_bin}" ]]; then
        _error "Unexpected firmware path in journal: ${requested_bin}"
    fi

    local clm_stem txcap_stem nvram_name nvram_board nvram_module nvram_vendor nvram_version board_base
    clm_stem="$(basename "${clm_file}" .clmb)"
    txcap_stem="$(basename "${txcap_file}" .txcb)"
    nvram_name="$(basename "${nvram_file}")"

    if [[ "${clm_stem}" != "${txcap_stem}" ]]; then
        _error ".clmb and .txcb files do not use the same board name."
    fi

    if [[ "${nvram_name}" =~ ^P-([^_]+)_M-([^_]+)_V-([^_]+)__m-(.+)\.txt$ ]]; then
        nvram_board="${BASH_REMATCH[1]}"
        nvram_module="${BASH_REMATCH[2]}"
        nvram_vendor="${BASH_REMATCH[3]}"
        nvram_version="${BASH_REMATCH[4]}"
    else
        _error "Unexpected NVRAM filename: ${nvram_name}"
    fi

    if [[ "${nvram_board}" != "${clm_stem}" ]]; then
        _error "NVRAM board name does not match .clmb/.txcb board name."
    fi

    board_base="$(basename "${trx_file}" .trx)"

    if [[ "${clm_stem}" != "${board_base}" && "${clm_stem}" != "${board_base}-"* ]]; then
        _error "Board name ${clm_stem} does not extend the .trx board name ${board_base}."
    fi

    local target_clm target_txcap target_nvram
    if [[ "${target_bin}" == brcmfmac*-pcie.apple,* ]]; then
        target_prefix="${target_bin%%.apple,*}"
        target_clm="${target_prefix}.apple,${clm_stem}.clm_blob"
        target_txcap="${target_prefix}.apple,${txcap_stem}.txcap_blob"
        target_nvram="${target_prefix}.apple,${board_base}-${nvram_module}-${nvram_vendor}-${nvram_version}.txt"
    elif [[ "${target_bin}" == brcmfmac*-pcie.bin ]]; then
        target_prefix="${target_bin%%.bin}"
        target_clm="${target_prefix}.clm_blob"
        target_txcap="${target_prefix}.txcap_blob"
        target_nvram="${target_prefix}.txt"
    else
        _error "Unexpected brcmfmac filename: ${target_bin}"
    fi

    _info "Found source Wi-Fi files:"
    echo "  Firmware: ${trx_file}"
    echo "  CLM:      ${clm_file}"
    echo "  TxCap:    ${txcap_file}"
    echo "  NVRAM:    ${nvram_file}"
    echo
    _info "Kernel requested:"
    echo "  ${requested_bin}"
    echo
    _info "Will install:"
    echo "  ${trx_file} -> ${DEST_DIR}/${target_bin}"
    echo "  ${clm_file} -> ${DEST_DIR}/${target_clm}"
    echo "  ${txcap_file} -> ${DEST_DIR}/${target_txcap}"
    echo "  ${nvram_file} -> ${DEST_DIR}/${target_nvram}"
    echo

    sudo install -d -m 0755 "${DEST_DIR}"
    sudo install -m 0644 "${trx_file}" "${DEST_DIR}/${target_bin}"
    sudo install -m 0644 "${clm_file}" "${DEST_DIR}/${target_clm}"
    sudo install -m 0644 "${txcap_file}" "${DEST_DIR}/${target_txcap}"
    sudo install -m 0644 "${nvram_file}" "${DEST_DIR}/${target_nvram}"

    echo ""
    _info "Reloading brcmfmac_wcc driver..."
    sudo modprobe -rv brcmfmac_wcc
    sleep 3
    sudo modprobe -v brcmfmac_wcc
    _success "Wi-Fi firmware installed."
}

install_bt_fw() {
    [[ -d "$SOURCE_DIR" ]] || _error "Need to copy firmware from macOS first."
    lspci -Dn | grep -qi '14e4:5fa0' || _error "No BCM4377 Bluetooth PCIe function (14e4:5fa0) was found."

    local bin_file bin_name name_re chip stepping board vendor ptb_file vendor_suffix target_base target_bin target_ptb
    bin_file="$(_pick_one 'BCM4377 production firmware' "$SOURCE_DIR"/BCM4377*_PROD.signed.bin)"
    bin_name="${bin_file##*/}"
    name_re='^BCM([0-9]{4})([A-Z][0-9])_.*_PCIE_([^_]+)_MFG_([^_]+)_.*_PROD\.signed\.bin$'

    [[ "$bin_name" =~ $name_re ]] || _error "Unexpected Apple Bluetooth firmware filename: ${bin_name}"

    chip="${BASH_REMATCH[1]}"
    stepping="${BASH_REMATCH[2]}"
    board="${BASH_REMATCH[3]}"
    vendor="${BASH_REMATCH[4]}"

    ptb_file="$(_pick_one 'matching BCM4377 production PTB' "${SOURCE_DIR}/BCM${chip}${stepping}__PCIE_${board}_MFG_${vendor}__PRODK_R_.ptb")"

    case "$vendor" in
        GEN) vendor_suffix="" ;;
        MUR) vendor_suffix="-m" ;;
        USI) vendor_suffix="-u" ;;
        *) _error "Unsupported antenna vendor in Apple filename: ${vendor}" ;;
    esac

    target_base="brcmbt${chip}${stepping,,}-apple,${board,,}${vendor_suffix}"
    target_bin="${DEST_DIR}/${target_base}.bin"
    target_ptb="${DEST_DIR}/${target_base}.ptb"

    _info "Found Apple BCM4377 firmware:"
    echo "  ${bin_file}"
    echo "  ${ptb_file}"
    echo
    _info "Will install:"
    echo "  ${target_bin}"
    echo "  ${target_ptb}"
    echo

    sudo install -d -m 0755 "$DEST_DIR"
    sudo install -m 0644 "$bin_file" "$target_bin"
    sudo install -m 0644 "$ptb_file" "$target_ptb"

    echo ""
    _info "Reloading hci_bcm4377 driver..."
    sudo modprobe -rv hci_bcm4377
    sleep 3
    sudo modprobe -v hci_bcm4377
    _success "Bluetooth firmware installed."
}

main() {
    if [[ "$(uname)" == "Darwin" ]]; then
        _info "Running on macOS, preparing to gather firmware..."
        _info "Tip: For best results, ensure macOS is up to date before gathering to obtain the latest firmware."
        local ans
        read -r -p "Proceed with gathering Wi-Fi and Bluetooth firmware? [y/N] " ans
        if [[ "$ans" == [yY] || "$ans" == [yY][eE][sS] ]]; then
            get_fw "$@"
        else
            _warn "Gathering aborted."
        fi
    elif [[ "$(uname)" == "Linux" ]]; then
        local distro
        distro=$(awk -F= '$1=="ID" { gsub(/"/, "", $2); print toupper(substr($2,1,1)) substr($2,2) }' /etc/os-release 2>/dev/null); distro="${distro:-Linux}"
        _info "Running on ${distro}, preparing to install firmware..."
        _info "Note: If you previously installed firmware via your package manager (e.g. 'apple-bcm-firmware'), please remove it first to avoid conflicts."

        _check_conflicting_pkg

        if [[ ! -d "$SOURCE_DIR" ]] || ! find "$SOURCE_DIR" -maxdepth 1 \( -name '*.trx' -o -name '*.bin' \) 2>/dev/null | grep -q .; then
            _warn "Firmware files not found in '${SCRIPT_DIR}' or EFI boot directories (/boot/firmware, /boot/efi/firmware)."
            _error "Need to copy firmware from macOS first."
        fi

        _info "Found firmware in: ${SOURCE_DIR}"

        local ans
        read -r -p "Proceed with installing Wi-Fi and Bluetooth firmware? [y/N] " ans
        if [[ "$ans" == [yY] || "$ans" == [yY][eE][sS] ]]; then
            install_wl_fw
            install_bt_fw
            _success "All firmware successfully installed."
        else
            _warn "Installation aborted."
        fi
    else
        _error "Unsupported operating system."
    fi
}

main "$@"
