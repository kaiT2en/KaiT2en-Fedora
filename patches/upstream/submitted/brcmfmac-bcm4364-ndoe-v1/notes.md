# [PATCH] wifi: brcmfmac: keep ARP and ND offload off on the BCM4364

- Status: superseded by brcmfmac-apple-ndoe-v2
- Version: v1
- Date: 2026-11-04
- Base: wireless-next `991a73f4909`
- Message-ID: `<20261104151620.70671-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261104151620.70671-1-dev@deq.rocks/

## Mails

- `<20261104151620.70671-1-dev@deq.rocks>` [PATCH] wifi: brcmfmac: keep ARP and ND offload off on the BCM4364

## Recipients of the first mail

To: arend.vanspriel@broadcom.com

Cc: linux-wireless@vger.kernel.org, brcm80211@lists.linux.dev, brcm80211-dev-list.pdl@broadcom.com, linux-kernel@vger.kernel.org, Andre Eikmeyer <dev@deq.rocks>, Yossef Osman <Yossef.Osman@proton.me>, Aditya Garg <aditya.garg@linux.dev>

## Review

- 2026-11-16 Arend `<20261116203719.281605-1-arend.vanspriel@broadcom.com>`: ND only unless there is data for ARP, restrict by board_type "apple," instead of chip ID, Fixes plus Cc stable, target tree in the subject. Replied 2026-11-18.
