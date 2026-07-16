# Installing the Broadcom firmware on Linux

[Automatic installation](automatic-installation.md)

Manual installation:

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md) (you are here)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md)
8. [How to update](07-updating.md)

Previous: [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md) | Next: [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)

This guide installs the Broadcom Wi-Fi firmware copied from macOS.

It also covers UART Bluetooth firmware for Macs where Linux requests a `.hcd`
file. BCM4377 Bluetooth devices use a different `.bin` and `.ptb` firmware path.
Do not use the Bluetooth section below for BCM4377.

## Requirements

Boot Linux once without the Broadcom Wi-Fi firmware installed. The kernel has to
try loading the missing firmware so the expected Linux filename appears in the
system log.

Check this with:

```bash
journalctl -b -k --grep='Direct firmware load for brcm/brcmfmac'
```

Expected output is similar to this:

```text
kernel: brcmfmac 0000:03:00.0: Direct firmware load for brcm/brcmfmac4364b2-pcie.apple,kauai-HRPN-u-7.5-X3.bin failed with error -2
kernel: brcmfmac 0000:03:00.0: Direct firmware load for brcm/brcmfmac4364b2-pcie.apple,kauai-HRPN-u-7.5.bin failed with error -2
kernel: brcmfmac 0000:03:00.0: Direct firmware load for brcm/brcmfmac4364b2-pcie.apple,kauai-HRPN-u.bin failed with error -2
kernel: brcmfmac 0000:03:00.0: Direct firmware load for brcm/brcmfmac4364b2-pcie.apple,kauai-HRPN.bin failed with error -2
kernel: brcmfmac 0000:03:00.0: Direct firmware load for brcm/brcmfmac4364b2-pcie.apple,kauai-X3.bin failed with error -2
```

The first filename is the best match requested by the kernel. The script below
uses that filename for the main firmware file.

## Install

Copy the firmware files from macOS to the `firmware` folder in this repository.
In Linux, open a terminal in the root folder of the KaiT2en repository on your
USB drive.

The `firmware` folder should contain one file of each type:

```text
*.trx
*.clmb
*.txcb
P-*.txt
```

Then copy, paste and run:

```bash
bash <(cat << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

source_dir="firmware"

die() {
    echo "Error: $*" >&2
    exit 1
}

pick_one() {
    local label="$1"
    shift
    local files=("$@")

    if (( ${#files[@]} == 0 )); then
        die "No ${label} file found in ${source_dir}."
    fi

    if (( ${#files[@]} > 1 )); then
        echo "Found more than one ${label} file:"
        printf '  %s\n' "${files[@]}"
        die "Keep only the matching file in ${source_dir} and run the script again."
    fi

    printf '%s\n' "${files[0]}"
}

if [[ ! -d "${source_dir}" ]]; then
    die "Run this script from the root of the KaiT2en repository."
fi

trx_file="$(pick_one '.trx firmware' "${source_dir}"/*.trx)"
clm_file="$(pick_one '.clmb regulatory blob' "${source_dir}"/*.clmb)"
txcap_file="$(pick_one '.txcb TxCap blob' "${source_dir}"/*.txcb)"
nvram_file="$(pick_one 'P-*.txt NVRAM file' "${source_dir}"/P-*.txt)"

requested_bin="$(
    journalctl -b -k --no-pager |
    sed -n 's/.*Direct firmware load for \(brcm\/brcmfmac[^ ]*\.bin\) failed with error -2.*/\1/p' |
    head -n 1
)"

if [[ -z "${requested_bin}" ]]; then
    die "No missing brcmfmac .bin firmware request found in the current boot log."
fi

target_bin="${requested_bin#brcm/}"

if [[ "${requested_bin}" == "${target_bin}" ]]; then
    die "Unexpected firmware path in journal: ${requested_bin}"
fi

if [[ "${target_bin}" != brcmfmac*-pcie.apple,* ]]; then
    die "Unexpected brcmfmac filename: ${target_bin}"
fi

target_prefix="${target_bin%%.apple,*}"

clm_stem="$(basename "${clm_file}" .clmb)"
txcap_stem="$(basename "${txcap_file}" .txcb)"
nvram_name="$(basename "${nvram_file}")"

if [[ "${clm_stem}" != "${txcap_stem}" ]]; then
    die ".clmb and .txcb files do not use the same board name."
fi

if [[ "${nvram_name}" =~ ^P-([^_]+)_M-([^_]+)_V-([^_]+)__m-(.+)\.txt$ ]]; then
    nvram_board="${BASH_REMATCH[1]}"
    nvram_module="${BASH_REMATCH[2]}"
    nvram_vendor="${BASH_REMATCH[3]}"
    nvram_version="${BASH_REMATCH[4]}"
else
    die "Unexpected NVRAM filename: ${nvram_name}"
fi

if [[ "${nvram_board}" != "${clm_stem}" ]]; then
    die "NVRAM board name does not match .clmb/.txcb board name."
fi

target_clm="${target_prefix}.apple,${clm_stem}.clm_blob"
target_txcap="${target_prefix}.apple,${txcap_stem}.txcap_blob"
target_nvram="${target_prefix}.apple,${nvram_board}-${nvram_module}-${nvram_vendor}-${nvram_version}.txt"

dest_dir="/lib/firmware/brcm"

echo "Found source files:"
echo "  Firmware: ${trx_file}"
echo "  CLM:      ${clm_file}"
echo "  TxCap:    ${txcap_file}"
echo "  NVRAM:    ${nvram_file}"
echo
echo "Kernel requested:"
echo "  ${requested_bin}"
echo
echo "Will install:"
echo "  ${trx_file} -> ${dest_dir}/${target_bin}"
echo "  ${clm_file} -> ${dest_dir}/${target_clm}"
echo "  ${txcap_file} -> ${dest_dir}/${target_txcap}"
echo "  ${nvram_file} -> ${dest_dir}/${target_nvram}"
echo

read -r -p "Continue? [y/N] " answer
case "${answer}" in
    y|Y) ;;
    *) echo "Aborted."; exit 0 ;;
esac

sudo install -d -m 0755 "${dest_dir}"
sudo install -m 0644 "${trx_file}" "${dest_dir}/${target_bin}"
sudo install -m 0644 "${clm_file}" "${dest_dir}/${target_clm}"
sudo install -m 0644 "${txcap_file}" "${dest_dir}/${target_txcap}"
sudo install -m 0644 "${nvram_file}" "${dest_dir}/${target_nvram}"

echo
echo "Done. Reboot and check Wi-Fi."
EOF
)
```

After reboot, check that the firmware loaded:

```bash
journalctl -b -k --grep='brcmfmac'
```

Successful output should include a firmware version and should not include
missing `brcmfmac` firmware errors. Wi-Fi should work now.

## Bluetooth (experimental, only non-BCM4377 controllers)

If your Mac uses BCM4377 Bluetooth, stop here. The UART Bluetooth steps below do
not apply to BCM4377.

Check this with:

```bash
journalctl -b -k --grep='BCM:.*hcd\|hci_bcm4377'
```

If the output shows something with `hci_bcm4377`, this guide does not apply to
you and you can continue with the next document. If it shows the output below,
the guide applies to you.

```text
Bluetooth: hci0: BCM: firmware Patch file not found, tried:
Bluetooth: hci0: BCM: 'brcm/BCM.hcd'
```

The message means the kernel is searching for a missing patch/firmware file.
The guide below explains how to convert the .hex file we extracted
from macOS earlier into a .hcd file Linux understands and where to put it.

Note, this is experimental. Using the Apple BT firmware on Linux may
or may not work on your device. We are not sure yet why the behaviour is
inconsistent. There are log messages below that indicate a fatal state. When you
see those, just remove `BCM.hcd` again.

However, when it works it seems to cure BT-Audio stutters, but
you will need to unload `hci_uart` on suspend using a systemd service because
the Bluetooth controller will refuse to go into `D3cold` as a trade-off.
Interestingly, this is the same behaviour the single-chip BCM4377 chip shows OOTB.

The KaiT2en `firmware` folder should contain one UART Bluetooth MiniDriver file:

```text
*-MiniDriver-uart.hex
```

Run this from the root folder of the KaiT2en repository on your USB drive:

```bash
bash <(cat << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

source_dir="firmware"
converter="third_party/hex2hcd/hex2hcd"

die() {
    echo "Error: $*" >&2
    exit 1
}

pick_one() {
    local label="$1"
    shift
    local files=("$@")

    if (( ${#files[@]} == 0 )); then
        die "No ${label} file found in ${source_dir}."
    fi

    if (( ${#files[@]} > 1 )); then
        echo "Found more than one ${label} file:"
        printf '  %s\n' "${files[@]}"
        die "Keep only the matching file in ${source_dir} and run the script again."
    fi

    printf '%s\n' "${files[0]}"
}

if [[ ! -d "${source_dir}" || ! -x "${converter}" ]]; then
    die "Run this script from the root of the KaiT2en repository."
fi

if journalctl -b -k --no-pager | grep -q 'hci_bcm4377'; then
    die "This machine uses hci_bcm4377. The UART .hcd Bluetooth path does not apply."
fi

requested_hcd="$(
    journalctl -b -k --no-pager |
    sed -n "s/.*BCM: 'brcm\/\\([^']*\\.hcd\\)'.*/brcm\/\\1/p" |
    head -n 1
)"

if [[ -z "${requested_hcd}" ]]; then
    die "No missing Broadcom .hcd firmware request found in the current boot log."
fi

target_hcd="${requested_hcd#brcm/}"

if [[ "${requested_hcd}" == "${target_hcd}" || "${target_hcd}" == */* ]]; then
    die "Unexpected Bluetooth firmware path in journal: ${requested_hcd}"
fi

hex_file="$(pick_one 'UART Bluetooth MiniDriver' "${source_dir}"/*-MiniDriver-uart.hex)"
hcd_file="${source_dir}/${target_hcd}"

"${converter}" "${hex_file}" "${hcd_file}"

dest_dir="/lib/firmware/brcm"

echo "Found source file:"
echo "  MiniDriver: ${hex_file}"
echo
echo "Kernel requested:"
echo "  ${requested_hcd}"
echo
echo "Will install:"
echo "  ${hcd_file} -> ${dest_dir}/${target_hcd}"
echo

read -r -p "Continue? [y/N] " answer
case "${answer}" in
    y|Y) ;;
    *) echo "Aborted."; exit 0 ;;
esac

sudo install -d -m 0755 "${dest_dir}"
sudo install -m 0644 "${hcd_file}" "${dest_dir}/${target_hcd}"

echo
echo "Done. Reboot and check Bluetooth."
EOF
)
```

After reboot, check that the firmware loaded:

```bash
journalctl -b -k --grep='Bluetooth|BCM|hci0'
```

The log should no longer include `firmware Patch file not found`.

**How to know your BT does not work with the Apple firmware:**

```text
Frame reassembly failed (-84) # fatal, bad HCD or wrong patch stream
command 0xfc18 tx timeout # fatal, controller no longer responds
Reset failed (-110) # fatal follow-up
missing Bluetooth: MGMT ver # fatal, controller did not come up
```

Known non-fatal message, but likely related to audio stutters:

```text
failed to write update baudrate (-16)
```

Next: [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
