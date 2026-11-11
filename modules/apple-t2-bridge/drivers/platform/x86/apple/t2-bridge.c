// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 Bridge Controller
 *
 * Intel Macs with a T2 coprocessor expose it as one multi-function PCI
 * device:
 *
 *   function 0  106b:2005  NVMe controller (nvme driver)
 *   function 1  106b:1801  Bridge Controller (this driver)
 *   function 2  106b:1802  Secure Enclave Processor
 *   function 3  106b:1803  audio (snd-apple-t2)
 *
 * The bridge controller is the host interface of the Buffer Copy Engine
 * (BCE), a DMA engine that connects host memory to services running on the
 * T2. A mailbox in BAR 4 negotiates the protocol version and registers the
 * command queues. All further queues are registered through the command
 * queue. Doorbells live in BAR 2.
 *
 * The virtual USB host controller behind which the T2 exposes keyboard,
 * trackpad, Touch Bar and other internal devices is a client of this
 * driver and binds to an auxiliary device. The audio function finds the
 * bridge with apple_bce_get().
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#include <linux/auxiliary_bus.h>
#include <linux/bitfield.h>
#include <linux/device.h>
#include <linux/dma-mapping.h>
#include <linux/interrupt.h>
#include <linux/module.h>
#include <linux/pci.h>
#include <linux/pm.h>
#include <linux/sizes.h>
#include <linux/slab.h>

#include "t2-bridge.h"

#define PCI_DEVICE_ID_APPLE_T2_BRIDGE	0x1801

#define BCE_BAR_DMA		2
#define BCE_BAR_MBOX		4

#define BCE_IRQ_MBOX		0
#define BCE_IRQ_DMA		4

/*
 * SAVE_STATE_AND_SLEEP and RESTORE_STATE_AND_WAKE pass a page aligned host
 * buffer: the DMA address in bits 43:12 and the size in pages in bits 11:0.
 * The T2 asks for a larger buffer with SAVE_STATE_AND_SLEEP_GROW.
 */
#define BCE_STATE_ADDR		GENMASK_ULL(43, 12)
#define BCE_STATE_PAGES		GENMASK_ULL(11, 0)
#define BCE_STATE_PAGE_SIZE	SZ_4K
#define BCE_STATE_MAX_SIZE	SZ_1M

static const char * const bce_aux_devices[] = { "vhci", "ave" };
static_assert(ARRAY_SIZE(bce_aux_devices) <= BCE_MAX_AUX);

static struct pci_driver apple_bce_driver;

static irqreturn_t bce_mbox_irq(int irq, void *data)
{
	struct apple_bce *bce = data;

	bce_mbox_handle_irq(&bce->mbox);
	return IRQ_HANDLED;
}

static irqreturn_t bce_dma_irq(int irq, void *data)
{
	struct apple_bce *bce = data;

	guard(mutex)(&bce->completion_lock);
	bce_dma_process_completions(bce);
	return IRQ_HANDLED;
}

static u64 bce_mb_msg(enum bce_mb_type type, u64 value)
{
	return FIELD_PREP(BCE_MB_TYPE, type) | FIELD_PREP(BCE_MB_VALUE, value);
}

static int bce_handshake(struct apple_bce *bce)
{
	u64 reply;
	int ret;

	ret = bce_mbox_send(&bce->mbox,
			    bce_mb_msg(BCE_MB_SET_FW_PROTOCOL_VERSION,
				       BCE_PROTOCOL_VERSION), &reply);
	if (ret)
		return ret;

	bce->fw_version = FIELD_GET(BCE_MB_VALUE, reply);
	if (FIELD_GET(BCE_MB_TYPE, reply) != BCE_MB_SET_FW_PROTOCOL_VERSION ||
	    bce->fw_version > BCE_PROTOCOL_VERSION) {
		dev_err(&bce->pdev->dev, "protocol handshake failed: %#llx\n",
			reply);
		return -EINVAL;
	}
	return 0;
}

static void bce_free_state(struct apple_bce *bce)
{
	kfree(bce->state);
	bce->state = NULL;
	bce->state_size = 0;
}

static int bce_alloc_state(struct apple_bce *bce, size_t size)
{
	bce_free_state(bce);
	/* kmalloc memory of at least one page is page aligned */
	bce->state = kzalloc(size, GFP_KERNEL);
	if (!bce->state)
		return -ENOMEM;
	bce->state_size = size;
	return 0;
}

static int bce_state_msg(struct apple_bce *bce, enum bce_mb_type type,
			 enum dma_data_direction dir, u64 *reply)
{
	struct device *dev = &bce->pdev->dev;
	dma_addr_t addr;
	int ret;

	addr = dma_map_single(dev, bce->state, bce->state_size, dir);
	if (dma_mapping_error(dev, addr))
		return -ENOMEM;

	if (addr & (BCE_STATE_PAGE_SIZE - 1)) {
		ret = -EINVAL;
		goto unmap;
	}

	ret = bce_mbox_send(&bce->mbox,
			    bce_mb_msg(type, (addr & BCE_STATE_ADDR) |
				       bce->state_size / BCE_STATE_PAGE_SIZE),
			    reply);
unmap:
	dma_unmap_single(dev, addr, bce->state_size, dir);
	return ret;
}

static int bce_save_state(struct apple_bce *bce)
{
	size_t size = BCE_STATE_PAGE_SIZE;
	u64 reply;
	int ret;

	for (;;) {
		ret = bce_alloc_state(bce, size);
		if (ret)
			return ret;

		ret = bce_state_msg(bce, BCE_MB_SAVE_STATE_AND_SLEEP,
				    DMA_FROM_DEVICE, &reply);
		if (ret)
			break;

		switch (FIELD_GET(BCE_MB_TYPE, reply)) {
		case BCE_MB_SAVE_RESTORE_STATE_COMPLETE:
			bce->state_saved = true;
			return 0;
		case BCE_MB_SAVE_STATE_AND_SLEEP_GROW:
			size = FIELD_GET(BCE_MB_VALUE, reply);
			if (size > bce->state_size && size <= BCE_STATE_MAX_SIZE) {
				size = ALIGN(size, BCE_STATE_PAGE_SIZE);
				continue;
			}
			fallthrough;
		default:
			ret = -EINVAL;
			break;
		}
		break;
	}

	dev_err(&bce->pdev->dev, "saving T2 state failed: %d\n", ret);
	bce_free_state(bce);
	return ret;
}

static int bce_restore_state(struct apple_bce *bce)
{
	u64 reply;
	int ret;

	if (!bce->state_saved)
		return -EINVAL;
	bce->state_saved = false;

	ret = bce_state_msg(bce, BCE_MB_RESTORE_STATE_AND_WAKE, DMA_TO_DEVICE,
			    &reply);
	if (!ret && FIELD_GET(BCE_MB_TYPE, reply) !=
		    BCE_MB_SAVE_RESTORE_STATE_COMPLETE)
		ret = -EINVAL;

	bce_free_state(bce);
	return ret;
}

static void bce_dma_irq_enable(struct apple_bce *bce)
{
	if (!bce->dma_irq_disabled)
		return;
	enable_irq(pci_irq_vector(bce->pdev, BCE_IRQ_DMA));
	bce->dma_irq_disabled = false;
}

static void bce_dma_irq_disable(struct apple_bce *bce)
{
	if (bce->dma_irq_disabled)
		return;
	disable_irq(pci_irq_vector(bce->pdev, BCE_IRQ_DMA));
	bce->dma_irq_disabled = true;
}

static void bce_wake(struct apple_bce *bce)
{
	bce_hosttime_start(&bce->hosttime, false);
	bce_dma_open(bce);
	/* completions posted while the gate was closed raise no interrupt */
	bce_dma_process_completions(bce);
}

/*
 * Children and consumers (the USB host controller, audio) are suspended
 * already and no longer submit. Drain the command queue, then let the T2
 * save its state into a host buffer.
 */
static int apple_bce_suspend(struct device *dev)
{
	struct apple_bce *bce = dev_get_drvdata(dev);
	int ret;

	if (bce->fw_version < BCE_PROTOCOL_VERSION) {
		dev_warn(dev, "protocol %#x does not support sleep\n",
			 bce->fw_version);
		return -EOPNOTSUPP;
	}

	ret = bce_dma_close(bce);
	if (ret)
		return ret;
	bce_hosttime_stop(&bce->hosttime);

	ret = bce_save_state(bce);
	if (ret) {
		guard(mutex)(&bce->completion_lock);
		bce_wake(bce);
		return ret;
	}

	bce_dma_irq_disable(bce);
	return 0;
}

static int apple_bce_resume(struct device *dev)
{
	struct apple_bce *bce = dev_get_drvdata(dev);
	int ret;

	guard(mutex)(&bce->completion_lock);

	pci_set_master(bce->pdev);
	pci_set_master(bce->fn0);
	/* the T2 posts completions while it restores its state */
	bce_dma_irq_enable(bce);

	ret = bce_restore_state(bce);
	if (ret)
		dev_err(dev, "restoring T2 state failed: %d\n", ret);

	bce_wake(bce);
	return ret;
}

static const struct dev_pm_ops apple_bce_pm_ops = {
	.suspend = apple_bce_suspend,
	.resume = apple_bce_resume,
};

static void bce_aux_destroy(struct apple_bce *bce)
{
	int i;

	for (i = ARRAY_SIZE(bce_aux_devices) - 1; i >= 0; i--) {
		if (bce->aux[i])
			auxiliary_device_destroy(bce->aux[i]);
		bce->aux[i] = NULL;
	}
}

static void bce_aux_create(struct apple_bce *bce)
{
	int i;

	for (i = 0; i < ARRAY_SIZE(bce_aux_devices); i++) {
		bce->aux[i] = auxiliary_device_create(&bce->pdev->dev,
						      KBUILD_MODNAME,
						      bce_aux_devices[i], bce, 0);
		if (!bce->aux[i])
			dev_warn(&bce->pdev->dev, "failed to add %s device\n",
				 bce_aux_devices[i]);
	}
}

static int apple_bce_probe(struct pci_dev *pdev, const struct pci_device_id *id)
{
	struct device *dev = &pdev->dev;
	struct apple_bce *bce;
	int ret;

	bce = devm_kzalloc(dev, sizeof(*bce), GFP_KERNEL);
	if (!bce)
		return -ENOMEM;
	bce->pdev = pdev;
	pci_set_drvdata(pdev, bce);

	ret = pcim_enable_device(pdev);
	if (ret)
		return ret;
	pci_set_master(pdev);

	bce->mbox_regs = pcim_iomap_region(pdev, BCE_BAR_MBOX, KBUILD_MODNAME);
	if (IS_ERR(bce->mbox_regs))
		return PTR_ERR(bce->mbox_regs);
	bce->dma_regs = pcim_iomap_region(pdev, BCE_BAR_DMA, KBUILD_MODNAME);
	if (IS_ERR(bce->dma_regs))
		return PTR_ERR(bce->dma_regs);

	/* The BCE addresses 37 bits. */
	ret = dma_set_mask_and_coherent(dev, DMA_BIT_MASK(37));
	if (ret)
		return ret;

	ret = pci_alloc_irq_vectors(pdev, BCE_IRQ_DMA + 1, 8, PCI_IRQ_MSI);
	if (ret < 0)
		return ret;

	bce_mbox_init(&bce->mbox, bce->mbox_regs);
	bce_hosttime_init(&bce->hosttime, bce->mbox_regs);

	ret = devm_request_irq(dev, pci_irq_vector(pdev, BCE_IRQ_MBOX),
			       bce_mbox_irq, 0, "apple-bce-mbox", bce);
	if (ret)
		return ret;
	ret = devm_request_threaded_irq(dev, pci_irq_vector(pdev, BCE_IRQ_DMA),
					NULL, bce_dma_irq, IRQF_ONESHOT,
					"apple-bce-dma", bce);
	if (ret)
		return ret;

	/*
	 * DMA of the bridge only works while function 0 is a bus master. The
	 * T2 also needs it while it saves its state on suspend and before it
	 * restores the state on resume. The nvme driver disables function 0
	 * on suspend, so make it our PM supplier.
	 */
	bce->fn0 = pci_get_slot(pdev->bus, PCI_DEVFN(PCI_SLOT(pdev->devfn), 0));
	if (!bce->fn0)
		return -ENODEV;
	pci_set_master(bce->fn0);
	bce->fn0_link = device_link_add(dev, &bce->fn0->dev, DL_FLAG_STATELESS);
	if (!bce->fn0_link)
		dev_warn(dev, "failed to order PM after function 0\n");

	bce_hosttime_start(&bce->hosttime, true);

	ret = bce_handshake(bce);
	if (ret)
		goto err;

	ret = bce_dma_init(bce);
	if (ret)
		goto err;

	dev_pm_set_driver_flags(dev, DPM_FLAG_NO_DIRECT_COMPLETE);
	bce_aux_create(bce);
	return 0;

err:
	bce_hosttime_stop(&bce->hosttime);
	if (bce->fn0_link)
		device_link_del(bce->fn0_link);
	pci_dev_put(bce->fn0);
	return ret;
}

static void apple_bce_remove(struct pci_dev *pdev)
{
	struct apple_bce *bce = pci_get_drvdata(pdev);

	bce_aux_destroy(bce);

	bce->removing = true;
	bce_hosttime_stop(&bce->hosttime);
	bce_dma_exit(bce);
	bce_free_state(bce);
	if (bce->fn0_link)
		device_link_del(bce->fn0_link);
	pci_dev_put(bce->fn0);
}

static void apple_bce_shutdown(struct pci_dev *pdev)
{
	struct apple_bce *bce = pci_get_drvdata(pdev);

	/* the T2 stays quiet, queues are no longer unregistered */
	bce->removing = true;
	bce_hosttime_stop(&bce->hosttime);
}

/**
 * apple_bce_get - look up the bridge from another function of the T2
 * @consumer: device of a sibling PCI function, usually during its probe
 *
 * Links @consumer to the bridge so that it suspends before and resumes after
 * the bridge and is unbound before the bridge goes away.
 *
 * Return: the bridge, ERR_PTR(-EPROBE_DEFER) if it is not bound yet or
 * ERR_PTR(-ENODEV).
 */
struct apple_bce *apple_bce_get(struct device *consumer)
{
	struct pci_dev *pdev = to_pci_dev(consumer);
	struct apple_bce *bce = ERR_PTR(-EPROBE_DEFER);
	struct pci_dev *bridge;

	bridge = pci_get_slot(pdev->bus, PCI_DEVFN(PCI_SLOT(pdev->devfn), 1));
	if (!bridge)
		return ERR_PTR(-ENODEV);

	if (bridge->vendor != PCI_VENDOR_ID_APPLE ||
	    bridge->device != PCI_DEVICE_ID_APPLE_T2_BRIDGE) {
		bce = ERR_PTR(-ENODEV);
		goto out;
	}

	device_lock(&bridge->dev);
	if (bridge->dev.driver == &apple_bce_driver.driver &&
	    device_is_bound(&bridge->dev)) {
		if (device_link_add(consumer, &bridge->dev,
				    DL_FLAG_AUTOREMOVE_CONSUMER))
			bce = pci_get_drvdata(bridge);
		else
			bce = ERR_PTR(-ENODEV);
	}
	device_unlock(&bridge->dev);
out:
	pci_dev_put(bridge);
	return bce;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_get, "APPLE_T2_BRIDGE");

static const struct pci_device_id apple_bce_ids[] = {
	{ PCI_VDEVICE(APPLE, PCI_DEVICE_ID_APPLE_T2_BRIDGE) },
	{ }
};
MODULE_DEVICE_TABLE(pci, apple_bce_ids);

static struct pci_driver apple_bce_driver = {
	.name = KBUILD_MODNAME,
	.id_table = apple_bce_ids,
	.probe = apple_bce_probe,
	.remove = apple_bce_remove,
	.shutdown = apple_bce_shutdown,
	.driver.pm = pm_sleep_ptr(&apple_bce_pm_ops),
};
module_pci_driver(apple_bce_driver);

MODULE_AUTHOR("Paul Pawlowski <paul@mrarm.io>");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 Bridge Controller");
MODULE_LICENSE("GPL");
