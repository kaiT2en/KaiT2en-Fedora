// SPDX-License-Identifier: GPL-2.0
/*
 * Power-management ordering quirk for Thunderbolt controllers in Apple T2
 * Macs. The in-tree Thunderbolt driver creates equivalent links for older
 * Apple controllers, but does not cover the Titan Ridge and Ice Lake NHIs
 * used by this generation.
 */

#include <linux/acpi.h>
#include <linux/device.h>
#include <linux/list.h>
#include <linux/module.h>
#include <linux/pci.h>
#include <linux/platform_data/x86/apple.h>
#include <linux/slab.h>

#define PCI_DEVICE_ID_INTEL_TITAN_RIDGE_2C_NHI 0x15e8
#define PCI_DEVICE_ID_INTEL_TITAN_RIDGE_4C_NHI 0x15eb
#define PCI_DEVICE_ID_INTEL_ICL_NHI1 0x8a0d
#define PCI_DEVICE_ID_INTEL_ICL_NHI0 0x8a17
#define PCI_DEVICE_ID_APPLE_T2_BRIDGE 0x1801

struct t2thunderbolt_link {
	struct list_head list;
	struct device_link *link;
};

struct t2thunderbolt_no_d3 {
	struct list_head list;
	struct pci_dev *pdev;
	bool already_set;
};

static LIST_HEAD(t2thunderbolt_links);
static LIST_HEAD(t2thunderbolt_no_d3_devices);

static int t2thunderbolt_disable_d3(struct pci_dev *pdev, const char *name)
{
	struct t2thunderbolt_no_d3 *entry;

	entry = kzalloc(sizeof(*entry), GFP_KERNEL);
	if (!entry)
		return -ENOMEM;

	entry->pdev = pci_dev_get(pdev);
	entry->already_set = pdev->dev_flags & PCI_DEV_FLAGS_NO_D3;
	pdev->dev_flags |= PCI_DEV_FLAGS_NO_D3;
	list_add_tail(&entry->list, &t2thunderbolt_no_d3_devices);
	pci_info(pdev, "D3 disabled for Thunderbolt %s\n", name);

	return 0;
}

static int t2thunderbolt_disable_xhci_bridge_d3(struct pci_dev *bridge)
{
	struct pci_dev *child;
	int ret;

	if (!bridge->subordinate)
		return 0;

	child = pci_get_slot(bridge->subordinate, PCI_DEVFN(0, 0));
	if (!child)
		return 0;

	if (child->class != PCI_CLASS_SERIAL_USB_XHCI) {
		pci_dev_put(child);
		return 0;
	}

	ret = t2thunderbolt_disable_d3(bridge, "xHCI port");
	if (!ret)
		ret = t2thunderbolt_disable_d3(child, "xHCI controller");
	pci_dev_put(child);

	return ret;
}

static int t2thunderbolt_add_link(struct pci_dev *consumer,
				  struct pci_dev *nhi)
{
	struct t2thunderbolt_link *entry;
	struct device_link *link;

	link = device_link_add(&consumer->dev, &nhi->dev,
			       DL_FLAG_STATELESS | DL_FLAG_PM_RUNTIME);
	if (!link) {
		pci_err(nhi, "failed to create PM link from %s\n",
			pci_name(consumer));
		return -EINVAL;
	}

	entry = kmalloc(sizeof(*entry), GFP_KERNEL);
	if (!entry) {
		device_link_del(link);
		return -ENOMEM;
	}

	entry->link = link;
	list_add_tail(&entry->list, &t2thunderbolt_links);
	pci_info(nhi, "PM ordering link from %s enabled\n", pci_name(consumer));

	return 0;
}

static int t2thunderbolt_add_switch_links(struct pci_dev *nhi)
{
	struct pci_dev *upstream = pci_upstream_bridge(nhi);
	struct pci_dev *pdev;
	int count = 0;
	int ret;

	while (upstream) {
		if (!pci_is_pcie(upstream))
			return -ENODEV;
		if (pci_pcie_type(upstream) == PCI_EXP_TYPE_UPSTREAM)
			break;
		upstream = pci_upstream_bridge(upstream);
	}

	if (!upstream || !upstream->subordinate)
		return -ENODEV;

	for_each_pci_bridge(pdev, upstream->subordinate) {
		if (!pci_is_pcie(pdev) ||
		    pci_pcie_type(pdev) != PCI_EXP_TYPE_DOWNSTREAM)
			continue;

		ret = t2thunderbolt_disable_xhci_bridge_d3(pdev);
		if (ret)
			return ret;

		if (!pdev->is_pciehp)
			continue;

		ret = t2thunderbolt_add_link(pdev, nhi);
		if (ret)
			return ret;
		count++;
	}

	return count ? count : -ENODEV;
}

static bool t2thunderbolt_is_trp(struct pci_dev *pdev)
{
	struct acpi_device *adev = ACPI_COMPANION(&pdev->dev);
	const char *bid;

	if (!adev)
		return false;

	bid = acpi_device_bid(adev);
	return bid && !strncmp(bid, "TRP", 3);
}

static int t2thunderbolt_add_icl_links(struct pci_dev *nhi)
{
	struct pci_dev *pdev = NULL;
	int count = 0;
	int ret;

	for_each_pci_dev(pdev) {
		if (pci_domain_nr(pdev->bus) != pci_domain_nr(nhi->bus) ||
		    pdev->bus != nhi->bus || !pci_is_pcie(pdev) ||
		    pci_pcie_type(pdev) != PCI_EXP_TYPE_ROOT_PORT ||
		    !t2thunderbolt_is_trp(pdev))
			continue;

		ret = t2thunderbolt_add_link(pdev, nhi);
		if (ret) {
			pci_dev_put(pdev);
			return ret;
		}
		count++;
	}

	return count ? count : -ENODEV;
}

static void t2thunderbolt_remove_links(void)
{
	struct t2thunderbolt_link *entry, *tmp;

	list_for_each_entry_safe(entry, tmp, &t2thunderbolt_links, list) {
		device_link_del(entry->link);
		list_del(&entry->list);
		kfree(entry);
	}
}

static void t2thunderbolt_restore_d3(void)
{
	struct t2thunderbolt_no_d3 *entry, *tmp;

	list_for_each_entry_safe(entry, tmp, &t2thunderbolt_no_d3_devices,
				 list) {
		if (!entry->already_set)
			entry->pdev->dev_flags &= ~PCI_DEV_FLAGS_NO_D3;
		pci_dev_put(entry->pdev);
		list_del(&entry->list);
		kfree(entry);
	}
}

static int __init t2thunderbolt_init(void)
{
	static const u16 ids[] = {
		PCI_DEVICE_ID_INTEL_TITAN_RIDGE_2C_NHI,
		PCI_DEVICE_ID_INTEL_TITAN_RIDGE_4C_NHI,
		PCI_DEVICE_ID_INTEL_ICL_NHI0,
		PCI_DEVICE_ID_INTEL_ICL_NHI1,
	};
	struct pci_dev *nhi;
	struct pci_dev *t2;
	int total = 0;
	int ret;
	int i;

	if (!x86_apple_machine)
		return -ENODEV;

	t2 = pci_get_device(PCI_VENDOR_ID_APPLE, PCI_DEVICE_ID_APPLE_T2_BRIDGE,
			    NULL);
	if (!t2)
		return -ENODEV;
	pci_dev_put(t2);

	for (i = 0; i < ARRAY_SIZE(ids); i++) {
		nhi = NULL;
		while ((nhi = pci_get_device(PCI_VENDOR_ID_INTEL, ids[i], nhi))) {
			if (ids[i] == PCI_DEVICE_ID_INTEL_ICL_NHI0 ||
			    ids[i] == PCI_DEVICE_ID_INTEL_ICL_NHI1)
				ret = t2thunderbolt_add_icl_links(nhi);
			else
				ret = t2thunderbolt_add_switch_links(nhi);

			if (ret < 0) {
				pci_err(nhi, "no usable Thunderbolt PM links found: %d\n",
					ret);
				pci_dev_put(nhi);
				t2thunderbolt_restore_d3();
				t2thunderbolt_remove_links();
				return ret;
			}
			total += ret;
		}
	}

	if (!total) {
		t2thunderbolt_restore_d3();
		return -ENODEV;
	}

	pr_info("t2thunderbolt: initialized with %d PM ordering links\n", total);
	return 0;
}

static void __exit t2thunderbolt_exit(void)
{
	t2thunderbolt_restore_d3();
	t2thunderbolt_remove_links();
}

module_init(t2thunderbolt_init);
module_exit(t2thunderbolt_exit);

MODULE_AUTHOR("Andre Eikmeyer <dev@deq.rocks>");
MODULE_DESCRIPTION("Apple T2 Thunderbolt power-management ordering quirks");
MODULE_LICENSE("GPL");
MODULE_VERSION("0.2");

MODULE_ALIAS("pci:v00008086d000015E8sv*sd*bc*sc*i*");
MODULE_ALIAS("pci:v00008086d000015EBsv*sd*bc*sc*i*");
MODULE_ALIAS("pci:v00008086d00008A0Dsv*sd*bc*sc*i*");
MODULE_ALIAS("pci:v00008086d00008A17sv*sd*bc*sc*i*");
