# [PATCH v2 0/5] wifi: brcmfmac: pcie: S3 and deep sleep on the T2 Mac chips

- Status: submitted
- Version: v2
- Date: 2026-11-11
- Base: wireless-next `991a73f4909` with Michael Reeves' BCM4388 series 01-05
- Depends on: https://lore.kernel.org/all/20260925-brcm4388-submit-v1-0-b5cbc2cde2fb@gmail.com/
- Message-ID: `<20261111120748.23795-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261111120748.23795-1-dev@deq.rocks/
- Previous: brcmfmac-t2-suspend-v1, https://lore.kernel.org/all/20261107154810.89780-1-dev@deq.rocks/
- Companion: bluetooth-bcm4377-resume-v2

## Mails

- `<20261111120748.23795-1-dev@deq.rocks>` [PATCH v2 0/5] wifi: brcmfmac: pcie: S3 and deep sleep on the T2 Mac chips
- `<20261111120748.23795-2-dev@deq.rocks>` [PATCH v2 1/5] wifi: brcmfmac: pcie: set the state before D0_INFORM on resume
- `<20261111120748.23795-3-dev@deq.rocks>` [PATCH v2 2/5] wifi: brcmfmac: pcie: skip the host ready doorbell at init once it is a capability
- `<20261111120748.23795-4-dev@deq.rocks>` [PATCH v2 3/5] wifi: brcmfmac: pcie: probe again after a resume through firmware
- `<20261111120748.23795-5-dev@deq.rocks>` [PATCH v2 4/5] wifi: brcmfmac: msgbuf: send the flowring delete without waiting for tx status
- `<20261111120748.23795-6-dev@deq.rocks>` [PATCH v2 5/5] wifi: brcmfmac: pcie: support inband device wake and deep sleep

## Recipients

To: arend.vanspriel@broadcom.com

Cc: linux-wireless@vger.kernel.org, brcm80211@lists.linux.dev, brcm80211-dev-list.pdl@broadcom.com, linux-kernel@vger.kernel.org, michael.reeves077@gmail.com, marcan@marcan.st, j@jannau.net, asahi@lists.linux.dev, sebastian.reichel@collabora.com
