# [PATCH 0/4] wifi: brcmfmac: pcie: S3 suspend and resume on the T2 Mac chips

- Status: submitted
- Version: v1
- Date: 2026-11-07
- Base: wireless-next `991a73f4909` with Michael Reeves' BCM4388 series 01-05
- Depends on: https://lore.kernel.org/all/20260925-brcm4388-submit-v1-0-b5cbc2cde2fb@gmail.com/
- Message-ID: `<20261107154810.89780-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261107154810.89780-1-dev@deq.rocks/
- Related: D0_INFORM report on Reeves patch 5, https://lore.kernel.org/all/20261106153612.2aadef2a@kait2en-mbp15-1/
- Companion: bluetooth-bcm4377-resume-v1

## Mails

- `<20261107154810.89780-1-dev@deq.rocks>` [PATCH 0/4] wifi: brcmfmac: pcie: S3 suspend and resume on the T2 Mac chips
- `<20261107154810.89780-2-dev@deq.rocks>` [PATCH 1/4] wifi: brcmfmac: pcie: set the state before D0_INFORM on resume
- `<20261107154810.89780-3-dev@deq.rocks>` [PATCH 2/4] wifi: brcmfmac: pcie: skip the host ready doorbell at init once it is a capability
- `<20261107154810.89780-4-dev@deq.rocks>` [PATCH 3/4] wifi: brcmfmac: pcie: probe again after a resume through firmware
- `<20261107154810.89780-5-dev@deq.rocks>` [PATCH 4/4] wifi: brcmfmac: msgbuf: send the flowring delete without waiting for tx status

## Recipients

To: arend.vanspriel@broadcom.com

Cc: linux-wireless@vger.kernel.org, brcm80211@lists.linux.dev, brcm80211-dev-list.pdl@broadcom.com, linux-kernel@vger.kernel.org, michael.reeves077@gmail.com, marcan@marcan.st, j@jannau.net, asahi@lists.linux.dev, sebastian.reichel@collabora.com
