# [PATCH v3 0/3] Bluetooth: hci_bcm4377: resume after ACPI S3 and ASPM

- Status: submitted
- Version: v3
- Date: 2026-11-27
- Base: bluetooth-next `f57f78086ab5`
- Message-ID: `<20261127132501.262651-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261127132501.262651-1-dev@deq.rocks/
- Previous: bluetooth-bcm4377-resume-v2
- Companion: brcmfmac-t2-suspend
- Reviewed-by: Sven Peter on 1/3
- Acked-by: Sven Peter on 3/3

## Mails

- `<20261127132501.262651-1-dev@deq.rocks>` [PATCH v3 0/3] Bluetooth: hci_bcm4377: resume after ACPI S3 and ASPM
- `<20261127132501.262651-2-dev@deq.rocks>` [PATCH v3 1/3] Bluetooth: hci_bcm4377: check hci_register_dev() for a negative return
- `<20261127132501.262651-3-dev@deq.rocks>` [PATCH v3 2/3] Bluetooth: hci_bcm4377: probe again after a resume through firmware
- `<20261127132501.262651-4-dev@deq.rocks>` [PATCH v3 3/3] Bluetooth: hci_bcm4377: keep ASPM off only while the firmware boots

## Recipients

To: sven@kernel.org, marcel@holtmann.org, luiz.dentz@gmail.com

Cc: j@jannau.net, neal@gompa.dev, asahi@lists.linux.dev, linux-arm-kernel@lists.infradead.org, linux-bluetooth@vger.kernel.org, linux-kernel@vger.kernel.org, Andre Eikmeyer <dev@deq.rocks>
