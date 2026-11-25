# [PATCH wireless v2] wifi: brcmfmac: keep ARP and ND offload off on Apple BCM4355 and BCM4364

- Status: superseded by brcmfmac-apple-ndoe-v3
- Version: v2
- Date: 2026-11-18
- Base: wireless main `763b13f622e2`
- Message-ID: `<20261118123933.74517-1-dev@deq.rocks>`
- In-Reply-To: `<20261104151620.70671-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261118123933.74517-1-dev@deq.rocks/
- Previous: brcmfmac-bcm4364-ndoe-v1
- Fixes: 52f22fb21764 ("brcmfmac: add neighbor discovery offload ip address table configuration")

## Recipients

To: arend.vanspriel@broadcom.com

Cc: linux-wireless@vger.kernel.org, brcm80211@lists.linux.dev, brcm80211-dev-list.pdl@broadcom.com, linux-kernel@vger.kernel.org, Andre Eikmeyer <dev@deq.rocks>, Yossef Osman <Yossef.Osman@proton.me>, Aditya Garg <aditya.garg@linux.dev>

stable@vger.kernel.org only as a tag (--suppress-cc=bodycc).

## Changes from v1

- board_type "apple," check, BCM4355 added from Aditya's rework, Co-developed-by Aditya
- ARP stays off, commit message gives the reasons (no benefit awake, no sleep use without WoWL, stale host IP table, existing promisc and soft AP disables)
- Fixes plus Cc stable, target wireless
