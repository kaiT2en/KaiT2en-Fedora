# [PATCH wireless-next v3 0/5] wifi: brcmfmac: pcie: S3 and deep sleep on the T2 Mac chips

- Status: submitted
- Version: v3
- Date: 2026-11-27
- Base: wireless-next `18d0bedbb5f0`
- Message-ID: `<20261127173549.29335-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261127173549.29335-1-dev@deq.rocks/
- Previous: brcmfmac-t2-suspend-v2
- Acked-by: Arend van Spriel on 1/5
- Tested-by: Yossef Osman on all patches
- Related: "wifi: brcmfmac: pcie: recover from fatal PCIe errors", https://lore.kernel.org/all/20261105215954.184290-1-sorencoremans@gmail.com/
- Companion: bluetooth-bcm4377-resume-v3

## Mails

- `<20261127173549.29335-1-dev@deq.rocks>` [PATCH wireless-next v3 0/5] wifi: brcmfmac: pcie: S3 and deep sleep on the T2 Mac chips
- `<20261127173549.29335-2-dev@deq.rocks>` [PATCH wireless-next v3 1/5] wifi: brcmfmac: pcie: set the state before D0_INFORM on resume
- `<20261127173549.29335-3-dev@deq.rocks>` [PATCH wireless-next v3 2/5] wifi: brcmfmac: pcie: skip the host ready doorbell at init on the BCM4364
- `<20261127173549.29335-4-dev@deq.rocks>` [PATCH wireless-next v3 3/5] wifi: brcmfmac: pcie: probe again after a resume through firmware
- `<20261127173549.29335-5-dev@deq.rocks>` [PATCH wireless-next v3 4/5] wifi: brcmfmac: msgbuf: do not wait for tx status on the BCM4377
- `<20261127173549.29335-6-dev@deq.rocks>` [PATCH wireless-next v3 5/5] wifi: brcmfmac: pcie: support inband device wake and deep sleep

## Recipients

To: arend.vanspriel@broadcom.com

Cc: linux-wireless@vger.kernel.org, brcm80211@lists.linux.dev, brcm80211-dev-list.pdl@broadcom.com, linux-kernel@vger.kernel.org, michael.reeves077@gmail.com, marcan@marcan.st, j@jannau.net, asahi@lists.linux.dev, sebastian.reichel@collabora.com, Soeren Coremans <sorencoremans@gmail.com>, Andre Eikmeyer <dev@deq.rocks>, Yossef Osman <Yossef.Osman@proton.me> (patches only)
