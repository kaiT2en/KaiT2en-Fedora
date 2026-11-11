# Apple T2 early CPU offlining

- Status: superseded by x86-cache-mtrr-resume-v1
- Version: v1
- Base: Linux master
- Tested: MacBookPro15,1, MacBookPro16,2, MacBookAir9,1 and a 27-inch T2 iMac
- Message-ID: `<20260812120326.155226-1-dev@deq.rocks>`
- Link: https://lore.kernel.org/all/20260812120326.155226-1-dev@deq.rocks/

## Recipients

To:

- Rafael J. Wysocki <rafael@kernel.org>
- linux-acpi@vger.kernel.org

Cc:

- Len Brown <lenb@kernel.org>
- Thomas Gleixner <tglx@kernel.org>
- Peter Zijlstra <peterz@infradead.org>
- linux-pm@vger.kernel.org
- linux-kernel@vger.kernel.org

The ACPI recipients come from `scripts/get_maintainer.pl`. The CPU hotplug
maintainers and `linux-pm` are included because the patch changes CPU-hotplug
ordering around the suspend notifier chain.
