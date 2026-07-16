# Get Broadcom firmware from macOS

[Automatic installation](automatic-installation.md)

Manual installation:

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md) (you are here)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md)
8. [How to update](07-updating.md)

Previous: [Introduction](00-introduction.md) | Next: [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)

*When you already have T2 Linux Fedora installed and you are using our revert
script, you can jump to [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md).*

Boot into macOS.
Open the macOS Terminal in the root folder of this repository on your USB drive.
Then execute the code below. It takes some time to complete. This is normal:

```
bash <(cat << 'EOF'
#!/bin/bash

BT_BASE="/usr/share/firmware/bluetooth"
DEST="${1:-firmware}"

WIFI_FILES=$(log show --last boot --info \
  --predicate 'eventMessage contains "/usr/share/firmware/"' 2>/dev/null \
  | grep "Copying" \
  | grep -oE '"[^"]*"' | tr -d '"' | sort -u)

BT_CHIPSET=$(system_profiler SPBluetoothDataType 2>/dev/null \
  | grep "Chipset:" \
  | grep -oE 'BCM_[0-9A-Z]+' \
  | tr -d '_')
BT_FILE=""
if [[ -n "$BT_CHIPSET" && -f "$BT_BASE/${BT_CHIPSET}-MiniDriver-uart.hex" ]]; then
  BT_FILE="$BT_BASE/${BT_CHIPSET}-MiniDriver-uart.hex"
fi

echo "[Wi-Fi]"
echo "$WIFI_FILES"
echo ""
echo "[Bluetooth]"
if [[ -n "$BT_FILE" ]]; then
  echo "$BT_FILE"
else
  echo "No UART Bluetooth MiniDriver found. This is expected on some Macs."
fi
echo ""

read -p "Copy files to '$DEST'? [y/N] " ANS
if [[ "$ANS" == "y" || "$ANS" == "Y" ]]; then
  mkdir -p "$DEST"
  while IFS= read -r f; do
    cp "$f" "$DEST/" && echo "  copied: $(basename "$f")"
  done <<< "$WIFI_FILES"
  if [[ -n "$BT_FILE" ]]; then
    cp "$BT_FILE" "$DEST/" && echo "  copied: $(basename "$BT_FILE")"
  fi
  echo ""
  echo "Done."
fi
EOF
)
```

As an example, running the script on a MacBook Pro 15,1, this is the expected output:

```
[Wi-Fi]
/usr/share/firmware/wifi/C-4364__s-B2/ekans.trx
/usr/share/firmware/wifi/C-4364__s-B2/kauai.clmb
/usr/share/firmware/wifi/C-4364__s-B2/kauai.txcb
/usr/share/firmware/wifi/C-4364__s-B2/P-kauai_M-HRPN_V-u__m-7.5.txt

[Bluetooth]
/usr/share/firmware/bluetooth/BCM4364B0-MiniDriver-uart.hex

Copy files to 'firmware'? [y/N] y
  copied: ekans.trx
  copied: kauai.clmb
  copied: kauai.txcb
  copied: P-kauai_M-HRPN_V-u__m-7.5.txt
  copied: BCM4364B0-MiniDriver-uart.hex

Done.
```

Next: [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
