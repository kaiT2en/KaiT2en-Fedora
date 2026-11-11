# [PATCH v2 0/3] Bluetooth: hci_bcm4377: resume after ACPI S3 and ASPM

- Status: submitted
- Version: v2
- Date: 2026-11-11
- Base: bluetooth-next `3f425f95ba3`
- Message-ID: `<20261111120248.21062-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261111120248.21062-1-dev@deq.rocks/
- Previous: bluetooth-bcm4377-resume-v1, https://lore.kernel.org/all/20261107154839.89941-1-dev@deq.rocks/
- Companion: brcmfmac-t2-suspend

## Mails

- `<20261111120248.21062-1-dev@deq.rocks>` [PATCH v2 0/3] Bluetooth: hci_bcm4377: resume after ACPI S3 and ASPM
- `<20261111120248.21062-2-dev@deq.rocks>` [PATCH v2 1/3] Bluetooth: hci_bcm4377: treat only negative hci_register_dev() returns as errors
- `<20261111120248.21062-3-dev@deq.rocks>` [PATCH v2 2/3] Bluetooth: hci_bcm4377: probe again after a resume through firmware
- `<20261111120248.21062-4-dev@deq.rocks>` [PATCH v2 3/3] Bluetooth: hci_bcm4377: keep ASPM off only while the firmware boots

## Recipients

To: sven@kernel.org, marcel@holtmann.org, luiz.dentz@gmail.com

Cc: j@jannau.net, neal@gompa.dev, asahi@lists.linux.dev, linux-arm-kernel@lists.infradead.org, linux-bluetooth@vger.kernel.org, linux-kernel@vger.kernel.org
