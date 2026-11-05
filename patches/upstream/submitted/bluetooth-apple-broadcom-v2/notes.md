# [PATCH v2 0/2] Bluetooth: Broadcom controllers of the Apple T2 Macs

- Status: submitted
- Version: v2
- Date: 2026-11-05
- Base: bluetooth-next `3f425f95ba3`
- Message-ID: `<20261105085750.120392-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261105085750.120392-1-dev@deq.rocks/
- Previous: v1, https://lore.kernel.org/all/20261104151601.70493-1-dev@deq.rocks/

## Changes from v1

- Dropped "Bluetooth: btbcm: clear vendor bits in LE advertising report
  types" and the revert of 88397423671f. Luiz Augusto von Dentz prefers
  the mask in hci_event.c and sees no need to limit it to the T2
  controllers while the bits are reserved.
- Removed "Called with each received event, also from atomic context."
  from the btbcm_set_acl_priority_on_connect() comment.

## Mails

- `<20261105085750.120392-1-dev@deq.rocks>` [PATCH v2 0/2] Bluetooth: Broadcom controllers of the Apple T2 Macs
- `<20261105085750.120392-2-dev@deq.rocks>` [PATCH v2 1/2] Bluetooth: btbcm: raise ACL priority on Apple Broadcom controllers
- `<20261105085750.120392-3-dev@deq.rocks>` [PATCH v2 2/2] Bluetooth: set the strict duplicate filter quirk on Apple Broadcom controllers

## Recipients

To: marcel@holtmann.org, luiz.dentz@gmail.com

Cc: linux-bluetooth@vger.kernel.org, linux-kernel@vger.kernel.org, sven@kernel.org, j@jannau.net, neal@gompa.dev, asahi@lists.linux.dev, linux-arm-kernel@lists.infradead.org, the Tested-by addresses on the patches
