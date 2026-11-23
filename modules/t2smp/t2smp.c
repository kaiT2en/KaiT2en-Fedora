// SPDX-License-Identifier: GPL-2.0
/*
 * Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
 *
 * CPU resume ordering quirk for Apple T2 Macs
 *
 * Apple firmware selects a different suspend path when _OSI("Darwin") is
 * active. On T2 Macs, bringing secondary CPUs online during the kernel's
 * early resume phase can then take several seconds per CPU. Taking them
 * offline before suspend preparation keeps them out of the suspend core's
 * frozen CPU mask. They can be restored normally after platform resume.
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/cpu.h>
#include <linux/cpumask.h>
#include <linux/module.h>
#include <linux/notifier.h>
#include <linux/pci.h>
#include <linux/platform_data/x86/apple.h>
#include <linux/suspend.h>

#define PCI_DEVICE_ID_APPLE_T2_BRIDGE 0x1801

static cpumask_var_t t2smp_offlined_cpus;

static void t2smp_restore_cpus(void)
{
	unsigned int cpu;
	unsigned int restored = 0;
	int ret;

	for_each_cpu(cpu, t2smp_offlined_cpus) {
		ret = add_cpu(cpu);
		if (ret) {
			pr_err("failed to restore CPU%u: %d\n", cpu, ret);
			continue;
		}

		cpumask_clear_cpu(cpu, t2smp_offlined_cpus);
		restored++;
	}

	if (restored)
		pr_info("restored %u secondary CPUs\n", restored);
}

static void t2smp_offline_cpus(void)
{
	unsigned int cpu;
	unsigned int offlined = 0;
	int ret;

	if (!cpumask_empty(t2smp_offlined_cpus)) {
		pr_err("CPUs from the previous suspend remain offline\n");
		t2smp_restore_cpus();
		if (!cpumask_empty(t2smp_offlined_cpus)) {
			pr_err("secondary CPU workaround skipped\n");
			return;
		}
	}

	for_each_present_cpu(cpu) {
		if (cpu == 0 || !cpu_online(cpu))
			continue;

		ret = remove_cpu(cpu);
		if (ret) {
			pr_err("failed to offline CPU%u: %d\n", cpu, ret);
			continue;
		}

		cpumask_set_cpu(cpu, t2smp_offlined_cpus);
		offlined++;
	}

	pr_info("took %u secondary CPUs offline\n", offlined);
}

static int t2smp_prepare_notify(struct notifier_block *nb,
				unsigned long action, void *unused)
{
	if (action == PM_SUSPEND_PREPARE)
		t2smp_offline_cpus();

	return NOTIFY_OK;
}

static int t2smp_restore_notify(struct notifier_block *nb,
				unsigned long action, void *unused)
{
	if (action == PM_POST_SUSPEND)
		t2smp_restore_cpus();

	return NOTIFY_OK;
}

/*
 * The CPU core PM notifier runs at priority 0. Offline CPUs before it blocks
 * hotplug, then restore them after it enables hotplug again.
 */
static struct notifier_block t2smp_prepare_notifier = {
	.notifier_call = t2smp_prepare_notify,
	.priority = 1,
};

static struct notifier_block t2smp_restore_notifier = {
	.notifier_call = t2smp_restore_notify,
	.priority = -1,
};

static int __init t2smp_init(void)
{
	struct pci_dev *t2;
	int ret;

	if (!x86_apple_machine)
		return -ENODEV;

	t2 = pci_get_device(PCI_VENDOR_ID_APPLE, PCI_DEVICE_ID_APPLE_T2_BRIDGE,
			    NULL);
	if (!t2)
		return -ENODEV;
	pci_dev_put(t2);

	if (!alloc_cpumask_var(&t2smp_offlined_cpus, GFP_KERNEL))
		return -ENOMEM;

	ret = register_pm_notifier(&t2smp_prepare_notifier);
	if (ret) {
		free_cpumask_var(t2smp_offlined_cpus);
		return ret;
	}

	ret = register_pm_notifier(&t2smp_restore_notifier);
	if (ret) {
		unregister_pm_notifier(&t2smp_prepare_notifier);
		free_cpumask_var(t2smp_offlined_cpus);
		return ret;
	}

	pr_info("initialized\n");
	return 0;
}

static void __exit t2smp_exit(void)
{
	unregister_pm_notifier(&t2smp_restore_notifier);
	unregister_pm_notifier(&t2smp_prepare_notifier);
	t2smp_restore_cpus();
	free_cpumask_var(t2smp_offlined_cpus);
}

module_init(t2smp_init);
module_exit(t2smp_exit);

MODULE_AUTHOR("Andre Eikmeyer <dev@deq.rocks>");
MODULE_DESCRIPTION("Apple T2 secondary CPU resume ordering quirk");
MODULE_LICENSE("GPL");
MODULE_VERSION("0.1");

MODULE_ALIAS("pci:v0000106Bd00001801sv*sd*bc*sc*i*");
