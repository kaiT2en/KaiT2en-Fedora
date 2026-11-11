# [PATCH] x86/cache: Restore AP cache state while bringing CPUs online on resume

- Status: submitted
- Version: v1
- Date: 2026-11-11
- Base: tip/master `c6279551a`
- Message-ID: `<20261111123747.49446-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20261111123747.49446-1-dev@deq.rocks/
- Based on: Chen Yu, https://lore.kernel.org/all/20180319041843.28218-1-yu.c.chen@intel.com/
- Supersedes: apple-t2-early-cpu-offlining-v1, withdrawn in a reply on its thread

## Recipients

To: tglx@kernel.org, mingo@redhat.com, bp@alien8.de, dave.hansen@linux.intel.com, x86@kernel.org

Cc: hpa@zytor.com, linux-kernel@vger.kernel.org, rafael@kernel.org, linux-pm@vger.kernel.org, lukas@wunner.de, yu.c.chen@intel.com
