#include "t2bce.h"
#include <linux/module.h>
#include <linux/crc32.h>
#include <linux/delay.h>
#include "audio/audio.h"
#include <linux/version.h>

static dev_t bce_chrdev;
static struct class *bce_class;
static const bool bce_use_stateful_sleep = true;

struct t2bce_device *global_bce;

static int bce_create_command_queues(struct t2bce_device *bce);
static void bce_free_command_queues(struct t2bce_device *bce);
static irqreturn_t bce_handle_mb_irq(int irq, void *dev);
static irqreturn_t bce_handle_dma_irq(int irq, void *dev);
static int bce_fw_version_handshake(struct t2bce_device *bce);
static int bce_register_command_queue(struct t2bce_device *bce, struct bce_queue_memcfg *cfg, int is_sq);
static int bce_alloc_state_buffer(struct t2bce_device *bce);
static void bce_free_state_buffer(struct t2bce_device *bce);
static int bce_pm_suspend_prepare(struct t2bce_device *bce);
static void bce_pm_suspend_abort(struct t2bce_device *bce);
static void bce_pm_resume_finish(struct t2bce_device *bce);

static int bce_alloc_state_buffer(struct t2bce_device *bce)
{
    /* Windows mba9,1/mbp16,1 keeps a persistent 0x2000 state buffer for device lifetime. */
    bce->saved_data_dma_size = 0x2000;
    bce->saved_data_dma_ptr = dma_alloc_coherent(&bce->pci->dev, bce->saved_data_dma_size,
            &bce->saved_data_dma_addr, GFP_KERNEL);
    if (!bce->saved_data_dma_ptr) {
        bce->saved_data_dma_size = 0;
        bce->saved_data_dma_addr = 0;
        return -ENOMEM;
    }

    return 0;
}

static void bce_free_state_buffer(struct t2bce_device *bce)
{
    if (!bce->saved_data_dma_ptr)
        return;

    dma_free_coherent(&bce->pci->dev, bce->saved_data_dma_size,
            bce->saved_data_dma_ptr, bce->saved_data_dma_addr);
    bce->saved_data_dma_ptr = NULL;
    bce->saved_data_dma_addr = 0;
    bce->saved_data_dma_size = 0;
}

static int t2bce_probe(struct pci_dev *dev, const struct pci_device_id *id)
{
    struct t2bce_device *bce = NULL;
    int status = 0;
    int nvec;

    pr_info("t2bce: capturing our device\n");

    if (pci_enable_device(dev))
        return -ENODEV;
    if (pci_request_regions(dev, "t2bce")) {
        status = -ENODEV;
        goto fail;
    }
    pci_set_master(dev);
    nvec = pci_alloc_irq_vectors(dev, 1, 8, PCI_IRQ_MSI);
    if (nvec < 5) {
        status = -EINVAL;
        goto fail;
    }

    bce = kzalloc(sizeof(struct t2bce_device), GFP_KERNEL);
    if (!bce) {
        status = -ENOMEM;
        goto fail;
    }

    bce->pci = dev;
    pci_set_drvdata(dev, bce);

    bce->devt = bce_chrdev;
    bce->dev = device_create(bce_class, &dev->dev, bce->devt, NULL, "t2bce");
    if (IS_ERR_OR_NULL(bce->dev)) {
        status = PTR_ERR(bce_class);
        goto fail;
    }

    bce->reg_mem_mb = pci_iomap(dev, 4, 0);
    bce->reg_mem_dma = pci_iomap(dev, 2, 0);

    if (IS_ERR_OR_NULL(bce->reg_mem_mb) || IS_ERR_OR_NULL(bce->reg_mem_dma)) {
        dev_warn(&dev->dev, "t2bce: Failed to pci_iomap required regions\n");
        goto fail;
    }

    bce_mailbox_init(&bce->mbox, bce->reg_mem_mb);
    bce_xhci_pm_init(&bce->xhci_pm, bce->reg_mem_mb);

    spin_lock_init(&bce->queues_lock);
    ida_init(&bce->queue_ida);
    mutex_init(&bce->pm_lock);
    bce->mailbox_channel_active = true;

    if ((status = pci_request_irq(dev, 0, bce_handle_mb_irq, NULL, dev, "bce_mbox")))
        goto fail;
    if ((status = pci_request_irq(dev, 4, NULL, bce_handle_dma_irq, dev, "bce_dma")))
        goto fail_interrupt_0;

    if ((status = dma_set_mask_and_coherent(&dev->dev, DMA_BIT_MASK(37)))) {
        dev_warn(&dev->dev, "dma: Setting mask failed\n");
        goto fail_interrupt;
    }

    if ((status = bce_alloc_state_buffer(bce)))
        goto fail_interrupt;

    /* Gets the function 0's interface. This is needed because Apple only accepts DMA on our function if function 0
       is a bus master, so we need to work around this. */
    bce->pci0 = pci_get_slot(dev->bus, PCI_DEVFN(PCI_SLOT(dev->devfn), 0));
#ifndef WITHOUT_NVME_PATCH
    if ((status = pci_enable_device_mem(bce->pci0))) {
        dev_warn(&dev->dev, "t2bce: failed to enable function 0\n");
        goto fail_dev0;
    }
#endif
    pci_set_master(bce->pci0);

    /* Windows device start writes the opaque -4 XHCI sentinel before the controller is considered active. */
    bce_xhci_pm_start(&bce->xhci_pm, true);

    if ((status = bce_fw_version_handshake(bce)))
        goto fail_ts;
    pr_info("t2bce: handshake done\n");

    if ((status = bce_create_command_queues(bce))) {
        pr_info("t2bce: Creating command queues failed\n");
        goto fail_ts;
    }

    global_bce = bce;

    bce_vhci_create(bce, &bce->vhci);

    return 0;

fail_ts:
    bce_free_state_buffer(bce);
    bce_xhci_pm_stop(&bce->xhci_pm);
#ifndef WITHOUT_NVME_PATCH
    pci_disable_device(bce->pci0);
fail_dev0:
#endif
    pci_dev_put(bce->pci0);
fail_interrupt:
    pci_free_irq(dev, 4, dev);
fail_interrupt_0:
    pci_free_irq(dev, 0, dev);
fail:
    if (bce && bce->dev) {
        device_destroy(bce_class, bce->devt);

        if (!IS_ERR_OR_NULL(bce->reg_mem_mb))
            pci_iounmap(dev, bce->reg_mem_mb);
        if (!IS_ERR_OR_NULL(bce->reg_mem_dma))
            pci_iounmap(dev, bce->reg_mem_dma);

        kfree(bce);
    }

    pci_free_irq_vectors(dev);
    pci_release_regions(dev);
    pci_disable_device(dev);

    if (!status)
        status = -EINVAL;
    return status;
}

static int bce_create_command_queues(struct t2bce_device *bce)
{
    int status;
    struct bce_queue_memcfg *cfg;

    bce->cmd_cq = bce_alloc_cq(bce, 0, 0x20);
    bce->cmd_cmdq = bce_alloc_cmdq(bce, 1, 0x20);
    if (bce->cmd_cq == NULL || bce->cmd_cmdq == NULL) {
        status = -ENOMEM;
        goto err;
    }
    bce->queues[0] = (struct bce_queue *) bce->cmd_cq;
    bce->queues[1] = (struct bce_queue *) bce->cmd_cmdq->sq;

    cfg = kzalloc(sizeof(struct bce_queue_memcfg), GFP_KERNEL);
    if (!cfg) {
        status = -ENOMEM;
        goto err;
    }
    bce_get_cq_memcfg(bce->cmd_cq, cfg);
    if ((status = bce_register_command_queue(bce, cfg, false)))
        goto err;
    bce_get_sq_memcfg(bce->cmd_cmdq->sq, bce->cmd_cq, cfg);
    if ((status = bce_register_command_queue(bce, cfg, true)))
        goto err;
    kfree(cfg);

    return 0;

err:
    if (bce->cmd_cq)
        bce_free_cq(bce, bce->cmd_cq);
    if (bce->cmd_cmdq)
        bce_free_cmdq(bce, bce->cmd_cmdq);
    return status;
}

static void bce_free_command_queues(struct t2bce_device *bce)
{
    bce_free_cq(bce, bce->cmd_cq);
    bce_free_cmdq(bce, bce->cmd_cmdq);
    bce->cmd_cq = NULL;
    bce->queues[0] = NULL;
}

static irqreturn_t bce_handle_mb_irq(int irq, void *dev)
{
    struct t2bce_device *bce = pci_get_drvdata(dev);
    bce_mailbox_handle_interrupt(&bce->mbox);
    return IRQ_HANDLED;
}

static irqreturn_t bce_handle_dma_irq(int irq, void *dev)
{
    int i;
    size_t ce = 0;
    struct t2bce_device *bce = pci_get_drvdata(dev);
    spin_lock(&bce->queues_lock);
    for (i = 0; i < BCE_MAX_QUEUE_COUNT; i++)
        if (bce->queues[i] && bce->queues[i]->type == BCE_QUEUE_CQ)
            bce_handle_cq_completions_locked(bce, (struct bce_queue_cq *) bce->queues[i], &ce);
    spin_unlock(&bce->queues_lock);
    bce_dispatch_pending_sq_completions(bce, ce);
    return IRQ_HANDLED;
}

static int bce_fw_version_handshake(struct t2bce_device *bce)
{
    u64 result;
    u32 fw_version;
    int status;

    if ((status = bce_mailbox_send(&bce->mbox, BCE_MB_MSG(BCE_MB_SET_FW_PROTOCOL_VERSION, BC_PROTOCOL_VERSION),
            &result)))
        return status;
    fw_version = (u32) BCE_MB_VALUE(result);
    if (BCE_MB_TYPE(result) != BCE_MB_SET_FW_PROTOCOL_VERSION ||
        fw_version > BC_PROTOCOL_VERSION) {
        pr_err("t2bce: FW version handshake failed %x:%llx\n", BCE_MB_TYPE(result), BCE_MB_VALUE(result));
        return -EINVAL;
    }
    bce->fw_version = fw_version;
    return 0;
}

static bool bce_stateful_supported(struct t2bce_device *bce)
{
    /* Windows mba9,1/mbp16,1 gates stateful sleep and wake on negotiated protocol version >= 0x20001. */
    return bce_use_stateful_sleep && bce->fw_version >= BC_PROTOCOL_VERSION;
}

static int bce_pm_wait_mailbox_idle(struct t2bce_device *bce)
{
    int retries = 100;

    while (atomic_read(&bce->mbox.mb_status) != 0) {
        if (!--retries) {
            pr_err("t2bce: timeout waiting for BCE mailbox channel to quiesce\n");
            return -ETIMEDOUT;
        }
        usleep_range(5000, 10000);
    }

    return 0;
}

static int bce_pm_channel_pause(struct t2bce_device *bce)
{
    int status;

    status = bce_mailbox_channel_pause(&bce->mbox);
    if (status)
        return status;

    status = bce_pm_wait_mailbox_idle(bce);

    if (status)
        bce_mailbox_channel_resume(&bce->mbox);
    if (status)
        return status;

    /* Windows vtable +0x100 drains the BCE channel before the suspend callback. */
    bce->mailbox_channel_active = false;
    return 0;
}

static void bce_pm_channel_resume(struct t2bce_device *bce)
{
    /* Windows vtable +0x108 marks the BCE channel active again after wake completes. */
    bce->mailbox_channel_active = true;
    bce_mailbox_channel_resume(&bce->mbox);
}

static int bce_pm_suspend_prepare(struct t2bce_device *bce)
{
    int status;

    status = bce_pm_channel_pause(bce);
    if (status)
        return status;

    /* Windows suspend unconditionally writes the opaque -2 XHCI sentinel before 0x17/0x14. */
    bce_xhci_pm_stop(&bce->xhci_pm);
    return 0;
}

static void bce_pm_suspend_abort(struct t2bce_device *bce)
{
    /* Failed suspend must unwind the wrapper order back to the running state. */
    bce_xhci_pm_start(&bce->xhci_pm, false);
    bce_pm_channel_resume(bce);
}

static void bce_pm_resume_finish(struct t2bce_device *bce)
{
    /* Windows resume unconditionally writes the opaque -3 XHCI sentinel after 0x18/0x15. */
    bce_xhci_pm_start(&bce->xhci_pm, false);
    bce_pm_channel_resume(bce);
}

static int bce_register_command_queue(struct t2bce_device *bce, struct bce_queue_memcfg *cfg, int is_sq)
{
    int status;
    int cmd_type;
    u64 result;
    // OS X uses an bidirectional direction, but that's not really needed
    dma_addr_t a = dma_map_single(&bce->pci->dev, cfg, sizeof(struct bce_queue_memcfg), DMA_TO_DEVICE);
    if (dma_mapping_error(&bce->pci->dev, a))
        return -ENOMEM;
    cmd_type = is_sq ? BCE_MB_REGISTER_COMMAND_SQ : BCE_MB_REGISTER_COMMAND_CQ;
    status = bce_mailbox_send(&bce->mbox, BCE_MB_MSG(cmd_type, a), &result);
    dma_unmap_single(&bce->pci->dev, a, sizeof(struct bce_queue_memcfg), DMA_TO_DEVICE);
    if (status)
        return status;
    if (BCE_MB_TYPE(result) != BCE_MB_REGISTER_COMMAND_QUEUE_REPLY)
        return -EINVAL;
    return 0;
}

static void t2bce_remove(struct pci_dev *dev)
{
    struct t2bce_device *bce = pci_get_drvdata(dev);
    bce->is_being_removed = true;

    bce_vhci_destroy(&bce->vhci);

    bce_free_state_buffer(bce);
    bce_xhci_pm_stop(&bce->xhci_pm);
#ifndef WITHOUT_NVME_PATCH
    pci_disable_device(bce->pci0);
#endif
    pci_dev_put(bce->pci0);
    pci_free_irq(dev, 0, dev);
    pci_free_irq(dev, 4, dev);
    bce_free_command_queues(bce);
    pci_iounmap(dev, bce->reg_mem_mb);
    pci_iounmap(dev, bce->reg_mem_dma);
    device_destroy(bce_class, bce->devt);
    pci_free_irq_vectors(dev);
    pci_release_regions(dev);
    pci_disable_device(dev);
    kfree(bce);
}

static int bce_pm_suspend_fallback_no_state(struct t2bce_device *bce)
{
    pr_info("t2bce: suspend: forcing SLEEP_NO_STATE (no reply expected)\n");
    if (bce_mailbox_send_no_reply_locked(&bce->mbox, BCE_MB_MSG(BCE_MB_SLEEP_NO_STATE, 0))) {
        pr_err("t2bce: suspend: SLEEP_NO_STATE send failed\n");
        return -EIO;
    }
    return 0;
}

static int bce_pm_suspend_try_state(struct t2bce_device *bce)
{
    int status;
    u64 resp;

    /* Windows mba9,1/mbp16,1 tries stateful first and decides fallback from the 0x17 reply. */
    bce->stateful_suspend_valid = false;

    if (!bce->saved_data_dma_ptr || !bce->saved_data_dma_addr || !bce->saved_data_dma_size) {
        pr_err("t2bce: suspend failed (persistent state buffer missing)\n");
        return -ENOMEM;
    }

    BUG_ON((bce->saved_data_dma_addr % 4096) != 0);
    status = bce_mailbox_send_locked(&bce->mbox,
            BCE_MB_MSG(BCE_MB_SAVE_STATE_AND_SLEEP,
                    (bce->saved_data_dma_addr & ~(4096LLU - 1)) | (bce->saved_data_dma_size / 4096)),
            &resp);
    if (status) {
        pr_err("t2bce: suspend failed (mailbox send)\n");
        return status;
    }

    if (BCE_MB_TYPE(resp) == BCE_MB_SAVE_RESTORE_STATE_COMPLETE) {
        pr_info("t2bce: suspend: remote response: restore state saved  \n");
        bce->stateful_suspend_valid = true;
        return 0;
    }

    if (BCE_MB_TYPE(resp) == BCE_MB_SAVE_STATE_AND_SLEEP_REJECTED) {
        pr_err("t2bce: remote rejected stateful suspend payload\n");
        return -EAGAIN;
    }
    return -EINVAL;
}

static int bce_pm_resume_no_state(struct t2bce_device *bce)
{
    int status;
    u64 resp;

    if ((status = bce_mailbox_send_locked(&bce->mbox, BCE_MB_MSG(BCE_MB_RESTORE_NO_STATE, 0), &resp))) {
        pr_err("t2bce: resume with no state failed (mailbox send)\n");
        return status;
    }
    if (BCE_MB_TYPE(resp) != BCE_MB_RESTORE_NO_STATE) {
        pr_err("t2bce: resume with no state failed (invalid device response)\n");
        return -EINVAL;
    }
    return 0;
}

static int bce_pm_resume_stateful(struct t2bce_device *bce)
{
    int status;
    u64 resp;

    if ((status = bce_mailbox_send_locked(&bce->mbox, BCE_MB_MSG(BCE_MB_RESTORE_STATE_AND_WAKE,
            (bce->saved_data_dma_addr & ~(4096LLU - 1)) | (bce->saved_data_dma_size / 4096)), &resp))) {
        pr_err("t2bce: resume with state failed (mailbox send)\n");
        return status;
    }
    if (BCE_MB_TYPE(resp) != BCE_MB_SAVE_RESTORE_STATE_COMPLETE) {
        pr_err("t2bce: resume with state failed (invalid device response)\n");
        return -EINVAL;
    }

    return 0;
}

static int t2bce_suspend(struct device *dev)
{
    struct t2bce_device *bce = pci_get_drvdata(to_pci_dev(dev));
    int status;

    pr_info("t2bce: suspend: entry\n");
    mutex_lock(&bce->pm_lock);

    bce->stateful_suspend_valid = false;
    bce->no_state_fallback = false;
    bce->vhci.no_state_resume = false;

    status = bce_pm_suspend_prepare(bce);
    if (status)
        goto out_unlock;

    if (!bce_stateful_supported(bce)) {
        /* No-state keeps the current Linux HCD teardown path until the Windows no-state path is rebuilt separately. */
        bce_vhci_remove_hcd(&bce->vhci);
        status = bce_pm_suspend_fallback_no_state(bce);
        if (!status) {
            bce->no_state_fallback = true;
            bce->vhci.no_state_resume = true;
        } else {
            bce_pm_suspend_abort(bce);
        }
        goto out_unlock;
    }

    status = bce_pm_suspend_try_state(bce);
    if (!status)
        goto out_unlock;

    if (status != -EAGAIN) {
        bce_pm_suspend_abort(bce);
        goto out_unlock;
    }

    /* Current Linux path treats the traced 0x19 not-ready reply as stateful reject. */
    pr_info("t2bce: suspend: stateful path not ready, falling back to no-state\n");
    bce_vhci_remove_hcd(&bce->vhci);
    status = bce_pm_suspend_fallback_no_state(bce);
    if (!status) {
        bce->no_state_fallback = true;
        bce->vhci.no_state_resume = true;
    } else {
        bce_pm_suspend_abort(bce);
    }

out_unlock:
    mutex_unlock(&bce->pm_lock);
    pr_info("t2bce: suspend: exit status=%d stateful_valid=%d no_state_resume=%d no_state_fallback=%d\n",
            status, bce->stateful_suspend_valid, bce->vhci.no_state_resume, bce->no_state_fallback);
    return status;
}

static int t2bce_resume(struct device *dev)
{
    struct t2bce_device *bce = pci_get_drvdata(to_pci_dev(dev));
    int status;
    bool used_stateful;

    pr_info("t2bce: resume: entry\n");
    mutex_lock(&bce->pm_lock);

    pci_set_master(bce->pci);
    pci_set_master(bce->pci0);

    /* Windows resumes from the suspend result, not a preselected mode. */
    used_stateful = bce_stateful_supported(bce) && bce->stateful_suspend_valid;
    pr_info("t2bce: resume path: %s\n", used_stateful ? "stateful" : "no-state");
    if (used_stateful)
        status = bce_pm_resume_stateful(bce);
    else
        status = bce_pm_resume_no_state(bce);
    if (status)
        goto out_unlock;

    if (used_stateful)
        bce->stateful_suspend_valid = false;

    bce_pm_resume_finish(bce);

out_unlock:
    mutex_unlock(&bce->pm_lock);
    pr_info("t2bce: resume: exit status=%d path=%s stateful_valid=%d no_state_resume=%d no_state_fallback=%d\n",
            status, used_stateful ? "stateful" : "no-state",
            bce->stateful_suspend_valid, bce->vhci.no_state_resume, bce->no_state_fallback);
    return status;
}

static void t2bce_complete(struct device *dev)
{
    struct t2bce_device *bce = pci_get_drvdata(to_pci_dev(dev));

    pr_info("t2bce: complete: entry no_state_fallback=%d no_state_resume=%d\n",
            bce->no_state_fallback, bce->vhci.no_state_resume);
    if (bce->no_state_fallback && bce->vhci.no_state_resume) {
        /* Re-add the VHCI HCD after the PM core completed resume ordering. */
        pr_info("t2bce: complete: scheduling VHCI HCD re-add after no-state wake\n");
        queue_work(bce->vhci.tq_state_wq, &bce->vhci.w_add_hcd);
        bce->no_state_fallback = false;
    }

    aaudio_resume_post_vhci(bce->aaudio);
    pr_info("t2bce: complete: exit\n");
}

static struct pci_device_id t2bce_ids[  ] = {
        { PCI_DEVICE(PCI_VENDOR_ID_APPLE, 0x1801) },
        { 0, },
};

MODULE_DEVICE_TABLE(pci, t2bce_ids);

struct dev_pm_ops t2bce_pci_driver_pm = {
        .suspend = t2bce_suspend,
        .resume = t2bce_resume,
        .complete = t2bce_complete
};
struct pci_driver t2bce_pci_driver = {
        .name = "t2bce",
        .id_table = t2bce_ids,
        .probe = t2bce_probe,
        .remove = t2bce_remove,
        .driver = {
                .pm = &t2bce_pci_driver_pm
        }
};


static int __init t2bce_module_init(void)
{
    int result;

    if ((result = alloc_chrdev_region(&bce_chrdev, 0, 1, "t2bce")))
        goto fail_chrdev;
#if LINUX_VERSION_CODE < KERNEL_VERSION(6,4,0)
    bce_class = class_create(THIS_MODULE, "t2bce");
#else
    bce_class = class_create("t2bce");
#endif
    if (IS_ERR(bce_class)) {
        result = PTR_ERR(bce_class);
        goto fail_class;
    }
    if ((result = bce_vhci_module_init())) {
        pr_err("t2bce: bce-vhci init failed");
        goto fail_class;
    }

    result = pci_register_driver(&t2bce_pci_driver);
    if (result)
        goto fail_drv;

    aaudio_module_init();

    return 0;

fail_drv:
    pci_unregister_driver(&t2bce_pci_driver);
fail_class:
    class_destroy(bce_class);
fail_chrdev:
    unregister_chrdev_region(bce_chrdev, 1);
    if (!result)
        result = -EINVAL;
    return result;
}
static void __exit t2bce_module_exit(void)
{
    pci_unregister_driver(&t2bce_pci_driver);

    aaudio_module_exit();
    bce_vhci_module_exit();
    class_destroy(bce_class);
    unregister_chrdev_region(bce_chrdev, 1);
}

MODULE_LICENSE("GPL");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@gmail.com>");
MODULE_DESCRIPTION("T2 BCE Driver");
MODULE_VERSION("0.041");
module_init(t2bce_module_init);
module_exit(t2bce_module_exit);
