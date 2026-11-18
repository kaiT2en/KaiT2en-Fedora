# BCM4377 missing D3 ACK workaround v2

- Status: superseded, the mailbox over the control ring from Reeves 05 makes the BCM4377 ACK D3
- Version: v2
- Date: 2026-10-01
- Base: wireless-next `1631d79ae57dce2c5f88ad278307028638a8b4d9`
- Tested: MacBookAir9,1, MacBookPro15,4 and MacBookPro16,3
- Message-ID: `<20261001085308.1688049-1-dev@deq.rocks>`
- In-Reply-To: `<20260918213014.1890677-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261001085308.1688049-1-dev@deq.rocks/

## Changes from v1

- Removed the redundant DMI allowlist because BCM4377 is Apple-exclusive and
  only appears in the three affected MacBook models.
- Kept the device check inside `CONFIG_PM`, fixing the `CONFIG_PM=n` unused
  function warning reported by the kernel test robot.
- Rebased onto wireless-next and included the base commit.

## Validation

- `scripts/checkpatch.pl --strict`: no errors, warnings or checks
- `git apply --reverse --check`: passed against the patched wireless-next tree
- W=1 build of `drivers/net/wireless/broadcom/brcm80211/brcmfmac/pcie.o`: passed

## Recipients

To:

- Arend van Spriel <arend.vanspriel@broadcom.com>
- Jakub Kicinski <kuba@kernel.org>

Cc:

- Aditya Garg <gargaditya08@live.com>
- kernel test robot <lkp@intel.com>
- oe-kbuild-all@lists.linux.dev
- linux-wireless@vger.kernel.org
- brcm80211@lists.linux.dev
- brcm80211-dev-list.pdl@broadcom.com
- linux-kernel@vger.kernel.org

## Review

- 2026-11-18: declared superseded in the reply to Arend on brcmfmac-d3-ack-reload-v1.
