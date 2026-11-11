// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 virtual USB host controller, transfers
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "apple-t2-vhci-transfer.h"
#include "apple-t2-vhci.h"
#include <linux/err.h>
#include <linux/usb/hcd.h>

static void bce_vhci_transfer_queue_completion(struct apple_bce_sq *sq);
static void bce_vhci_transfer_queue_giveback(struct bce_vhci_transfer_queue *q);
static void
bce_vhci_transfer_queue_remove_pending(struct bce_vhci_transfer_queue *q);

static int bce_vhci_urb_init(struct bce_vhci_urb *vurb);
static int bce_vhci_urb_update(struct bce_vhci_urb *urb,
			       struct bce_vhci_message *msg);
static int bce_vhci_urb_transfer_completion(struct bce_vhci_urb *urb,
					    struct apple_bce_completion *c);
static void bce_vhci_urb_complete(struct bce_vhci_urb *urb, int status);
static void bce_vhci_urb_free(struct bce_vhci_urb *urb);

static void bce_vhci_transfer_queue_reset_w(struct work_struct *work);
static void bce_vhci_transfer_queue_cancel_w(struct work_struct *work);

void bce_vhci_create_transfer_queue(struct bce_vhci *vhci,
				    struct bce_vhci_transfer_queue *q,
				    struct usb_host_endpoint *endp,
				    bce_vhci_device_t dev_addr,
				    enum dma_data_direction dir)
{
	char name[0x21];

	INIT_LIST_HEAD(&q->evq);
	INIT_LIST_HEAD(&q->giveback_urb_list);
	INIT_LIST_HEAD(&q->cancel_urb_list);
	spin_lock_init(&q->urb_lock);
	mutex_init(&q->pause_lock);
	init_waitqueue_head(&q->sq_out_wait_queue);
	atomic_set(&q->sq_out_pending, 0);
	q->vhci = vhci;
	q->endp = endp;
	q->dev_addr = dev_addr;
	q->endp_addr = (u8)(endp->desc.bEndpointAddress & 0x8F);
	q->state = BCE_VHCI_ENDPOINT_ACTIVE;
	q->active = true;
	q->stalled = false;
	q->paused_by = 0;
	q->max_active_requests = 1;
	if (usb_endpoint_type(&endp->desc) == USB_ENDPOINT_XFER_BULK)
		q->max_active_requests = BCE_VHCI_BULK_MAX_ACTIVE_URBS;
	q->remaining_active_requests = q->max_active_requests;
	q->cq = apple_bce_cq_create(vhci->bce, 0x100);
	INIT_WORK(&q->w_reset, bce_vhci_transfer_queue_reset_w);
	INIT_WORK(&q->w_cancel, bce_vhci_transfer_queue_cancel_w);
	q->sq_in = NULL;
	if (dir == DMA_FROM_DEVICE || dir == DMA_BIDIRECTIONAL) {
		snprintf(name, sizeof(name), "VHC1-%i-%02x", dev_addr,
			 0x80 | usb_endpoint_num(&endp->desc));
		q->sq_in = apple_bce_sq_create(
			vhci->bce, q->cq, name, 0x100, DMA_FROM_DEVICE,
			bce_vhci_transfer_queue_completion, q);
	}
	q->sq_out = NULL;
	if (dir == DMA_TO_DEVICE || dir == DMA_BIDIRECTIONAL) {
		snprintf(name, sizeof(name), "VHC1-%i-%02x", dev_addr,
			 usb_endpoint_num(&endp->desc));
		q->sq_out = apple_bce_sq_create(
			vhci->bce, q->cq, name, 0x100, DMA_TO_DEVICE,
			bce_vhci_transfer_queue_completion, q);
	}
}

void bce_vhci_destroy_transfer_queue(struct bce_vhci *vhci,
				     struct bce_vhci_transfer_queue *q)
{
	cancel_work_sync(&q->w_reset);
	cancel_work_sync(&q->w_cancel);
	bce_vhci_transfer_queue_giveback(q);
	bce_vhci_transfer_queue_remove_pending(q);
	if (q->sq_in)
		apple_bce_sq_destroy(q->sq_in);
	if (q->sq_out)
		apple_bce_sq_destroy(q->sq_out);
	apple_bce_cq_destroy(q->cq);
}

static inline bool
bce_vhci_transfer_queue_can_init_urb(struct bce_vhci_transfer_queue *q)
{
	return q->remaining_active_requests > 0;
}

static bool
bce_vhci_transfer_queue_defer_event(struct bce_vhci_transfer_queue *q,
				    struct bce_vhci_message *msg)
{
	struct bce_vhci_list_message *lm;

	lm = kmalloc_obj(*lm, GFP_ATOMIC);
	if (!lm) {
		pr_warn("[%02x] failed to defer transfer queue event cmd=%x status=%x\n",
			q->endp_addr, msg->cmd, msg->status);
		return false;
	}
	INIT_LIST_HEAD(&lm->list);
	lm->msg = *msg;
	list_add_tail(&lm->list, &q->evq);
	return true;
}

static void bce_vhci_transfer_queue_giveback(struct bce_vhci_transfer_queue *q)
{
	unsigned long flags;
	struct urb *urb;

	spin_lock_irqsave(&q->urb_lock, flags);
	while (!list_empty(&q->giveback_urb_list)) {
		urb = list_first_entry(&q->giveback_urb_list, struct urb,
				       urb_list);
		list_del(&urb->urb_list);

		spin_unlock_irqrestore(&q->urb_lock, flags);
		usb_hcd_giveback_urb(q->vhci->hcd, urb, urb->status);
		spin_lock_irqsave(&q->urb_lock, flags);
	}
	spin_unlock_irqrestore(&q->urb_lock, flags);
}

static void
bce_vhci_transfer_queue_init_pending_urbs(struct bce_vhci_transfer_queue *q);

static int bce_vhci_transfer_queue_port(struct bce_vhci_transfer_queue *q)
{
	int port;

	for (port = 1; port <= q->vhci->port_count &&
	     port < ARRAY_SIZE(q->vhci->port_to_device); port++) {
		if (q->vhci->port_to_device[port] == q->dev_addr)
			return port;
	}

	return 0;
}

static bool bce_vhci_transfer_queue_is_ep0(struct bce_vhci_transfer_queue *q)
{
	return q->endp_addr == 0x00;
}

static size_t
bce_vhci_transfer_queue_event_count(struct bce_vhci_transfer_queue *q)
{
	struct bce_vhci_list_message *lm;
	size_t count = 0;

	list_for_each_entry(lm, &q->evq, list)
		count++;

	return count;
}

static void bce_vhci_transfer_queue_log_event(struct bce_vhci_transfer_queue *q,
					      struct bce_vhci_message *msg,
					      const char *stage)
{
	if (!bce_vhci_transfer_queue_is_ep0(q))
		return;

	pr_debug(
		"EP0 event %s dev=%u port=%d cmd=%x msg_status=%x p1=%x p2=%llx active=%u paused_by=%x stalled=%u qstate=%u remaining=%u evq=%zu urb_empty=%d\n",
		stage, q->dev_addr, bce_vhci_transfer_queue_port(q), msg->cmd,
		msg->status, msg->param1, msg->param2, q->active, q->paused_by,
		q->stalled, q->state, q->remaining_active_requests,
		bce_vhci_transfer_queue_event_count(q),
		list_empty(&q->endp->urb_list));
}

static void bce_vhci_transfer_queue_log_completion(
	struct bce_vhci_transfer_queue *q, struct apple_bce_sq *sq,
	struct apple_bce_completion *c, const char *stage)
{
	const char *sq_name = "unknown";

	if (!bce_vhci_transfer_queue_is_ep0(q))
		return;

	if (sq == q->sq_in)
		sq_name = "in";
	else if (sq == q->sq_out)
		sq_name = "out";

	pr_debug(
		"EP0 completion %s dev=%u port=%d sq=%s cstatus=%x data_size=%llu sq_out_pending=%d active=%u paused_by=%x stalled=%u qstate=%u remaining=%u evq=%zu urb_empty=%d\n",
		stage, q->dev_addr, bce_vhci_transfer_queue_port(q), sq_name,
		c->status, c->data_size, atomic_read(&q->sq_out_pending),
		q->active, q->paused_by, q->stalled, q->state,
		q->remaining_active_requests,
		bce_vhci_transfer_queue_event_count(q),
		list_empty(&q->endp->urb_list));
}

static void bce_vhci_urb_log_control(struct bce_vhci_urb *urb,
				     const char *stage)
{
	struct bce_vhci_transfer_queue *q = urb->q;
	struct usb_ctrlrequest *ctrl;

	if (!urb->is_control || !bce_vhci_transfer_queue_is_ep0(q))
		return;

	ctrl = (struct usb_ctrlrequest *)urb->urb->setup_packet;
	if (!ctrl) {
		pr_debug(
			"EP0 control %s dev=%u port=%d urb_state=%u no-setup xfer_len=%u actual=%u recv=%u request_end=%u submitted=%u status=%d active=%u paused_by=%x stalled=%u qstate=%u remaining=%u urb_reject=%d\n",
			stage, q->dev_addr, bce_vhci_transfer_queue_port(q),
			urb->state, urb->urb->transfer_buffer_length,
			urb->urb->actual_length, urb->receive_offset,
			urb->request_end, urb->submitted_length,
			urb->received_status, q->active, q->paused_by,
			q->stalled, q->state, q->remaining_active_requests,
			atomic_read(&urb->urb->reject));
		return;
	}

	pr_debug(
		"EP0 control %s dev=%u port=%d urb_state=%u dir=%s reqtype=%02x req=%02x value=%04x index=%04x wlen=%u xfer_len=%u actual=%u recv=%u request_end=%u submitted=%u status=%d active=%u paused_by=%x stalled=%u qstate=%u remaining=%u urb_reject=%d\n",
		stage, q->dev_addr, bce_vhci_transfer_queue_port(q), urb->state,
		usb_urb_dir_in(urb->urb) ? "in" : "out", ctrl->bRequestType,
		ctrl->bRequest, le16_to_cpu(ctrl->wValue),
		le16_to_cpu(ctrl->wIndex), le16_to_cpu(ctrl->wLength),
		urb->urb->transfer_buffer_length, urb->urb->actual_length,
		urb->receive_offset, urb->request_end, urb->submitted_length,
		urb->received_status, q->active, q->paused_by, q->stalled,
		q->state, q->remaining_active_requests,
		atomic_read(&urb->urb->reject));
}

static void
bce_vhci_transfer_queue_deliver_pending(struct bce_vhci_transfer_queue *q)
{
	struct urb *urb;
	struct bce_vhci_list_message *lm;

	while (!list_empty(&q->endp->urb_list) && !list_empty(&q->evq)) {
		urb = list_first_entry(&q->endp->urb_list, struct urb,
				       urb_list);

		lm = list_first_entry(&q->evq, struct bce_vhci_list_message,
				      list);
		if (bce_vhci_urb_update(urb->hcpriv, &lm->msg) == -EAGAIN) {
			bce_vhci_transfer_queue_log_event(
				q, &lm->msg, "deferred-still-waiting");
			break;
		}
		list_del(&lm->list);
		kfree(lm);
	}

	/* some of the URBs could have been completed, so initialize more URBs if possible */
	bce_vhci_transfer_queue_init_pending_urbs(q);
}

static void
bce_vhci_transfer_queue_remove_pending(struct bce_vhci_transfer_queue *q)
{
	unsigned long flags;
	struct bce_vhci_list_message *lm;

	spin_lock_irqsave(&q->urb_lock, flags);
	if (bce_vhci_transfer_queue_is_ep0(q) && !list_empty(&q->evq))
		pr_debug(
			"EP0 event remove-pending dev=%u port=%d evq=%zu active=%u paused_by=%x stalled=%u qstate=%u remaining=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			bce_vhci_transfer_queue_event_count(q), q->active,
			q->paused_by, q->stalled, q->state,
			q->remaining_active_requests);
	while (!list_empty(&q->evq)) {
		lm = list_first_entry(&q->evq, struct bce_vhci_list_message,
				      list);
		list_del(&lm->list);
		kfree(lm);
	}
	spin_unlock_irqrestore(&q->urb_lock, flags);
}

void bce_vhci_transfer_queue_event(struct bce_vhci_transfer_queue *q,
				   struct bce_vhci_message *msg)
{
	unsigned long flags;
	struct bce_vhci_urb *turb;
	struct urb *urb;

	spin_lock_irqsave(&q->urb_lock, flags);
	/* Paused queues may still complete in-flight work but must not deliver new work. */
	if (!q->active) {
		/*
		 * Ports are resumed before their endpoint queues.  bridgeOS may send
		 * a fresh transfer window as soon as the port becomes active, so keep
		 * that request until the system-resume owner reactivates the queue.
		 * Dropping it leaves bulk OUT users such as cdc_ncm waiting forever.
		 * Internal recovery only needs this treatment for EP0.
		 */
		if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST &&
		    ((q->paused_by & BCE_VHCI_PAUSE_SUSPEND) ||
		     (bce_vhci_transfer_queue_is_ep0(q) &&
		      (q->paused_by & BCE_VHCI_PAUSE_INTERNAL_WQ)))) {
			bce_vhci_transfer_queue_log_event(
				q, msg, "defer-inactive-resume");
			if (!bce_vhci_transfer_queue_defer_event(q, msg))
				bce_vhci_transfer_queue_log_event(
					q, msg, "defer-inactive-failed");
		} else {
			bce_vhci_transfer_queue_log_event(q, msg,
							  "drop-inactive");
		}
		goto complete;
	}
	bce_vhci_transfer_queue_deliver_pending(q);

	if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST &&
	    (!list_empty(&q->evq) || list_empty(&q->endp->urb_list))) {
		if (!bce_vhci_transfer_queue_defer_event(q, msg))
			bce_vhci_transfer_queue_log_event(
				q, msg, "defer-no-urb-or-backlog-failed");
		goto complete;
	}
	if (list_empty(&q->endp->urb_list)) {
		bce_vhci_transfer_queue_log_event(q, msg, "unexpected-empty");
		pr_err("[%02x] Unexpected transfer queue event\n",
		       q->endp_addr);
		goto complete;
	}
	urb = list_first_entry(&q->endp->urb_list, struct urb, urb_list);
	turb = urb->hcpriv;
	if (bce_vhci_urb_update(turb, msg) == -EAGAIN) {
		if (!bce_vhci_transfer_queue_defer_event(q, msg))
			bce_vhci_transfer_queue_log_event(
				q, msg, "defer-eagain-failed");
	} else {
		bce_vhci_transfer_queue_init_pending_urbs(q);
	}

complete:
	spin_unlock_irqrestore(&q->urb_lock, flags);
	bce_vhci_transfer_queue_giveback(q);
}

static void bce_vhci_transfer_queue_completion(struct apple_bce_sq *sq)
{
	unsigned long flags;
	struct apple_bce_completion *c;
	struct urb *urb;
	struct bce_vhci_transfer_queue *q = apple_bce_sq_priv(sq);
	bool is_sq_out = (sq == q->sq_out);

	spin_lock_irqsave(&q->urb_lock, flags);
	while ((c = apple_bce_sq_next_completion(sq))) {
		if (c->status ==
		    APPLE_BCE_STATUS_ABORTED) { /* We flushed the queue */
			if (is_sq_out &&
			    atomic_dec_if_positive(&q->sq_out_pending) == 0)
				wake_up(&q->sq_out_wait_queue);
			apple_bce_sq_complete(sq);
			continue;
		}
		if (list_empty(&q->endp->urb_list)) {
			bce_vhci_transfer_queue_log_completion(q, sq, c,
							       "empty");
			pr_err("[%02x] Got a completion while no requests are pending\n",
			       q->endp_addr);
			if (is_sq_out &&
			    atomic_dec_if_positive(&q->sq_out_pending) == 0)
				wake_up(&q->sq_out_wait_queue);
			continue;
		}
		urb = list_first_entry(&q->endp->urb_list, struct urb,
				       urb_list);
		bce_vhci_urb_transfer_completion(urb->hcpriv, c);
		if (is_sq_out &&
		    atomic_dec_if_positive(&q->sq_out_pending) == 0)
			wake_up(&q->sq_out_wait_queue);
		apple_bce_sq_complete(sq);
	}
	bce_vhci_transfer_queue_deliver_pending(q);
	spin_unlock_irqrestore(&q->urb_lock, flags);
	bce_vhci_transfer_queue_giveback(q);
}

int bce_vhci_transfer_queue_do_pause(struct bce_vhci_transfer_queue *q)
{
	unsigned long flags;
	int status;
	u8 endp_addr = (u8)(q->endp->desc.bEndpointAddress & 0x8F);
	int pending;
	long timeout;

	spin_lock_irqsave(&q->urb_lock, flags);
	q->active = false;
	spin_unlock_irqrestore(&q->urb_lock, flags);
	if (q->sq_out) {
		/* Let pending OUT submissions drain before the endpoint is paused. */
		pending = atomic_read(&q->sq_out_pending);
		if (pending > 0) {
			timeout = wait_event_timeout(
				q->sq_out_wait_queue,
				atomic_read(&q->sq_out_pending) == 0,
				msecs_to_jiffies(2000));
			if (!timeout && atomic_read(&q->sq_out_pending) > 0) {
				status = -ETIMEDOUT;
				pr_err("[%02x] pause timeout waiting for %d outputs\n",
				       q->endp_addr,
				       atomic_read(&q->sq_out_pending));
				goto restore_active;
			}
		}
	}
	bce_vhci_transfer_queue_remove_pending(q);
	status = bce_vhci_cmd_endpoint_set_state(&q->vhci->cq, q->dev_addr,
						 endp_addr,
						 BCE_VHCI_ENDPOINT_PAUSED,
						 &q->state);
	if (status)
		goto restore_active;
	if (q->state != BCE_VHCI_ENDPOINT_PAUSED) {
		status = -EINVAL;
		goto restore_active;
	}
	if (q->sq_in) {
		status = apple_bce_sq_flush(q->sq_in);
		if (status)
			goto restore_endpoint;
	}
	if (q->sq_out) {
		status = apple_bce_sq_flush(q->sq_out);
		if (status)
			goto restore_endpoint;
	}
	return 0;

restore_endpoint:
	if (bce_vhci_transfer_queue_do_resume(q))
		pr_err("[%02x] failed to restore endpoint after pause error\n",
		       q->endp_addr);
	return status;

restore_active:
	spin_lock_irqsave(&q->urb_lock, flags);
	q->active = true;
	bce_vhci_transfer_queue_deliver_pending(q);
	spin_unlock_irqrestore(&q->urb_lock, flags);
	return status;
}

static void bce_vhci_urb_resume(struct bce_vhci_urb *urb);

int bce_vhci_transfer_queue_do_resume(struct bce_vhci_transfer_queue *q)
{
	unsigned long flags;
	int status;
	struct urb *urb, *urbt;
	struct bce_vhci_urb *vurb;
	u8 endp_addr = (u8)(q->endp->desc.bEndpointAddress & 0x8F);

	if (bce_vhci_transfer_queue_is_ep0(q))
		pr_debug(
			"tq resume begin dev=%u port=%d ep=%02x active=%u paused_by=%x stalled=%u state=%u remaining=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, q->active, q->paused_by, q->stalled,
			q->state, q->remaining_active_requests);
	status = bce_vhci_cmd_endpoint_set_state(&q->vhci->cq, q->dev_addr,
						 endp_addr,
						 BCE_VHCI_ENDPOINT_ACTIVE,
						 &q->state);
	if (status) {
		pr_warn("tq resume set-active failed dev=%u port=%d ep=%02x status=%d ret_state=%u active=%u paused_by=%x stalled=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, status, q->state, q->active, q->paused_by,
			q->stalled);
		return status;
	}
	if (q->state != BCE_VHCI_ENDPOINT_ACTIVE) {
		pr_warn("tq resume set-active returned non-active dev=%u port=%d ep=%02x ret_state=%u active=%u paused_by=%x stalled=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, q->state, q->active, q->paused_by,
			q->stalled);
		return -EINVAL;
	}
	spin_lock_irqsave(&q->urb_lock, flags);
	q->active = true;
	list_for_each_entry_safe(urb, urbt, &q->endp->urb_list, urb_list) {
		vurb = urb->hcpriv;
		if (vurb->state == BCE_VHCI_URB_INIT_PENDING) {
			if (!bce_vhci_transfer_queue_can_init_urb(q))
				break;
			bce_vhci_urb_init(vurb);
		} else {
			bce_vhci_urb_resume(vurb);
		}
	}
	bce_vhci_transfer_queue_deliver_pending(q);
	spin_unlock_irqrestore(&q->urb_lock, flags);
	if (bce_vhci_transfer_queue_is_ep0(q))
		pr_debug(
			"tq resume done dev=%u port=%d ep=%02x active=%u paused_by=%x stalled=%u state=%u remaining=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, q->active, q->paused_by, q->stalled,
			q->state, q->remaining_active_requests);
	return 0;
}

int bce_vhci_transfer_queue_pause(struct bce_vhci_transfer_queue *q,
				  enum bce_vhci_pause_source src)
{
	int ret = 0;
	u32 old_paused_by;

	mutex_lock(&q->pause_lock);
	old_paused_by = q->paused_by;
	if ((q->paused_by & src) != src) {
		if (!q->paused_by)
			ret = bce_vhci_transfer_queue_do_pause(q);
		if (!ret)
			q->paused_by |= src;
	}
	if (bce_vhci_transfer_queue_is_ep0(q) || ret)
		pr_debug(
			"tq pause dev=%u port=%d ep=%02x src=%x ret=%d paused_by=%x->%x active=%u stalled=%u state=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, src, ret, old_paused_by, q->paused_by,
			q->active, q->stalled, q->state);
	mutex_unlock(&q->pause_lock);
	return ret;
}

int bce_vhci_transfer_queue_resume(struct bce_vhci_transfer_queue *q,
				   enum bce_vhci_pause_source src)
{
	int ret = 0;
	u32 old_paused_by;
	bool did_resume = false;

	mutex_lock(&q->pause_lock);
	old_paused_by = q->paused_by;
	if (q->paused_by & src) {
		if (!(q->paused_by & ~src)) {
			ret = bce_vhci_transfer_queue_do_resume(q);
			did_resume = true;
		}
		if (!ret)
			q->paused_by &= ~src;
	}
	if (src == BCE_VHCI_PAUSE_SUSPEND)
		pr_debug(
			"system resume dev=%u ep=%02x src=%x paused_by=%x->%x firmware_resume=%u ret=%d state=%x active=%u\n",
			q->dev_addr, q->endp_addr, src, old_paused_by,
			q->paused_by, did_resume, ret, q->state, q->active);
	else if (q->endp_addr == 0x00)
		pr_debug(
			"EP0 resume dev=%u src=%x paused_by=%x->%x did_resume=%u ret=%d state=%x active=%u\n",
			q->dev_addr, src, old_paused_by, q->paused_by,
			did_resume, ret, q->state, q->active);
	mutex_unlock(&q->pause_lock);
	return ret;
}

static void bce_vhci_transfer_queue_reset_w(struct work_struct *work)
{
	unsigned long flags;
	struct bce_vhci_transfer_queue *q =
		container_of(work, struct bce_vhci_transfer_queue, w_reset);

	mutex_lock(&q->pause_lock);
	spin_lock_irqsave(&q->urb_lock, flags);
	pr_debug(
		"tq reset start dev=%u port=%d ep=%02x active=%u paused_by=%x stalled=%u state=%u remaining=%u\n",
		q->dev_addr, bce_vhci_transfer_queue_port(q), q->endp_addr,
		q->active, q->paused_by, q->stalled, q->state,
		q->remaining_active_requests);
	if (!q->stalled) {
		spin_unlock_irqrestore(&q->urb_lock, flags);
		mutex_unlock(&q->pause_lock);
		pr_debug(
			"tq reset skipped dev=%u port=%d ep=%02x stalled=0 active=%u paused_by=%x state=%u\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q),
			q->endp_addr, q->active, q->paused_by, q->state);
		return;
	}
	q->active = false;
	spin_unlock_irqrestore(&q->urb_lock, flags);
	q->paused_by |= BCE_VHCI_PAUSE_INTERNAL_WQ;
	bce_vhci_transfer_queue_remove_pending(q);
	if (q->sq_in)
		apple_bce_sq_flush(q->sq_in);
	if (q->sq_out)
		apple_bce_sq_flush(q->sq_out);
	pr_debug("tq reset command dev=%u port=%d ep=%02x\n", q->dev_addr,
		 bce_vhci_transfer_queue_port(q), q->endp_addr);
	bce_vhci_cmd_endpoint_reset(&q->vhci->cq, q->dev_addr,
				    (u8)(q->endp->desc.bEndpointAddress &
					 0x8F));
	spin_lock_irqsave(&q->urb_lock, flags);
	q->stalled = false;
	spin_unlock_irqrestore(&q->urb_lock, flags);
	mutex_unlock(&q->pause_lock);
	bce_vhci_transfer_queue_resume(q, BCE_VHCI_PAUSE_INTERNAL_WQ);
	pr_debug(
		"tq reset done dev=%u port=%d ep=%02x active=%u paused_by=%x stalled=%u state=%u remaining=%u\n",
		q->dev_addr, bce_vhci_transfer_queue_port(q), q->endp_addr,
		q->active, q->paused_by, q->stalled, q->state,
		q->remaining_active_requests);
}

static void bce_vhci_transfer_queue_cancel_w(struct work_struct *work)
{
	unsigned long flags;
	struct bce_vhci_transfer_queue *q =
		container_of(work, struct bce_vhci_transfer_queue, w_cancel);
	struct bce_vhci_urb *vurb;
	struct urb *urb;
	int status;
	bool was_active;

	for (;;) {
		spin_lock_irqsave(&q->urb_lock, flags);
		if (list_empty(&q->cancel_urb_list)) {
			spin_unlock_irqrestore(&q->urb_lock, flags);
			return;
		}

		vurb = list_first_entry(&q->cancel_urb_list,
					struct bce_vhci_urb, cancel_list);
		list_del_init(&vurb->cancel_list);
		urb = vurb->urb;
		status = vurb->cancel_status;
		was_active = vurb->cancel_was_active;
		spin_unlock_irqrestore(&q->urb_lock, flags);

		/* Endpoint pause talks to bridgeOS and may sleep. */
		if (was_active) {
			bce_vhci_transfer_queue_pause(
				q, BCE_VHCI_PAUSE_INTERNAL_WQ);
		}

		spin_lock_irqsave(&q->urb_lock, flags);
		if (was_active)
			++q->remaining_active_requests;
		usb_hcd_unlink_urb_from_ep(q->vhci->hcd, urb);
		urb->hcpriv = NULL;
		spin_unlock_irqrestore(&q->urb_lock, flags);

		if (was_active)
			bce_vhci_transfer_queue_resume(
				q, BCE_VHCI_PAUSE_INTERNAL_WQ);

		usb_hcd_giveback_urb(q->vhci->hcd, urb, status);

		bce_vhci_urb_free(vurb);
	}
}

void bce_vhci_transfer_queue_request_reset(struct bce_vhci_transfer_queue *q)
{
	queue_work(q->vhci->tq_state_wq, &q->w_reset);
}

static void
bce_vhci_transfer_queue_init_pending_urbs(struct bce_vhci_transfer_queue *q)
{
	struct urb *urb, *urbt;
	struct bce_vhci_urb *vurb;

	list_for_each_entry_safe(urb, urbt, &q->endp->urb_list, urb_list) {
		vurb = urb->hcpriv;
		if (!bce_vhci_transfer_queue_can_init_urb(q))
			break;
		if (vurb->state == BCE_VHCI_URB_INIT_PENDING)
			bce_vhci_urb_init(vurb);
	}
}

static int bce_vhci_urb_data_start(struct bce_vhci_urb *urb,
				   unsigned long *timeout);

static bool bce_vhci_urb_uses_sg(struct bce_vhci_urb *urb)
{
	return urb->sg_list;
}

static bool bce_vhci_urb_is_zlp(struct bce_vhci_urb *urb)
{
	return urb->dir == DMA_TO_DEVICE &&
	       usb_endpoint_xfer_bulk(&urb->urb->ep->desc) &&
	       !urb->urb->transfer_buffer_length;
}

static void bce_vhci_urb_free(struct bce_vhci_urb *urb)
{
	if (!urb)
		return;

	apple_bce_sglist_destroy(urb->q->vhci->bce, urb->sg_list);

	kfree(urb);
}

static int bce_vhci_urb_init_sg_segments(struct bce_vhci_urb *vurb,
					 gfp_t mem_flags)
{
	if (!vurb->urb->sg || vurb->urb->num_mapped_sgs <= 0)
		return 0;

	vurb->sg_list =
		apple_bce_sglist_create(vurb->q->vhci->bce, vurb->urb->sg,
					vurb->urb->num_mapped_sgs, mem_flags);
	if (IS_ERR(vurb->sg_list)) {
		int status = PTR_ERR(vurb->sg_list);

		vurb->sg_list = NULL;
		return status;
	}

	return 0;
}

int bce_vhci_urb_create(struct bce_vhci_transfer_queue *q, struct urb *urb,
			gfp_t mem_flags)
{
	unsigned long flags;
	int status = 0;
	struct bce_vhci_urb *vurb;
	bool is_control = (usb_endpoint_num(&urb->ep->desc) == 0);

	vurb = kzalloc_obj(*vurb, mem_flags);
	if (!vurb)
		return -ENOMEM;

	urb->hcpriv = vurb;

	vurb->q = q;
	vurb->urb = urb;
	vurb->dir = usb_urb_dir_in(urb) ? DMA_FROM_DEVICE : DMA_TO_DEVICE;
	vurb->is_control = is_control;
	INIT_LIST_HEAD(&vurb->cancel_list);
	status = bce_vhci_urb_init_sg_segments(vurb, mem_flags);
	if (status) {
		urb->hcpriv = NULL;
		bce_vhci_urb_free(vurb);
		return status;
	}

	spin_lock_irqsave(&q->urb_lock, flags);
	if (vurb->is_control && (!q->active || q->stalled || q->paused_by))
		pr_debug(
			"EP0 enqueue dev=%u port=%d active=%u paused_by=%x stalled=%u state=%u remaining=%u urb_reject=%d\n",
			q->dev_addr, bce_vhci_transfer_queue_port(q), q->active,
			q->paused_by, q->stalled, q->state,
			q->remaining_active_requests,
			atomic_read(&urb->reject));

	status = usb_hcd_link_urb_to_ep(q->vhci->hcd, urb);
	if (status) {
		if (vurb->is_control)
			pr_warn("EP0 enqueue link failed dev=%u port=%d status=%d active=%u paused_by=%x stalled=%u state=%u remaining=%u urb_reject=%d\n",
				q->dev_addr, bce_vhci_transfer_queue_port(q),
				status, q->active, q->paused_by, q->stalled,
				q->state, q->remaining_active_requests,
				atomic_read(&urb->reject));
		spin_unlock_irqrestore(&q->urb_lock, flags);
		urb->hcpriv = NULL;
		bce_vhci_urb_free(vurb);
		return status;
	}

	if (q->active) {
		if (bce_vhci_transfer_queue_can_init_urb(vurb->q))
			status = bce_vhci_urb_init(vurb);
		else
			vurb->state = BCE_VHCI_URB_INIT_PENDING;
	} else {
		if (q->stalled)
			bce_vhci_transfer_queue_request_reset(q);
		vurb->state = BCE_VHCI_URB_INIT_PENDING;
	}
	if (status) {
		usb_hcd_unlink_urb_from_ep(q->vhci->hcd, urb);
		urb->hcpriv = NULL;
		bce_vhci_urb_free(vurb);
	} else {
		bce_vhci_transfer_queue_deliver_pending(q);
	}
	spin_unlock_irqrestore(&q->urb_lock, flags);
	return status;
}

static int bce_vhci_urb_init(struct bce_vhci_urb *vurb)
{
	int status = 0;

	if (vurb->q->remaining_active_requests == 0) {
		pr_err("cannot init request (remaining_active_requests = 0)\n");
		return -EINVAL;
	}

	if (vurb->is_control)
		vurb->state = BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_REQUEST;
	else
		status = bce_vhci_urb_data_start(vurb, NULL);

	if (!status)
		--vurb->q->remaining_active_requests;
	return status;
}

static void bce_vhci_urb_complete(struct bce_vhci_urb *urb, int status)
{
	struct bce_vhci_transfer_queue *q = urb->q;
	struct bce_vhci *vhci = q->vhci;
	struct urb *real_urb = urb->urb;

	usb_hcd_unlink_urb_from_ep(vhci->hcd, real_urb);
	real_urb->hcpriv = NULL;
	real_urb->status = status;
	if (urb->state != BCE_VHCI_URB_INIT_PENDING)
		++urb->q->remaining_active_requests;
	bce_vhci_urb_free(urb);
	list_add_tail(&real_urb->urb_list, &q->giveback_urb_list);
}

static int bce_vhci_urb_fail_data_transfer(struct bce_vhci_urb *urb, int status)
{
	struct bce_vhci_transfer_queue *q = urb->q;

	/* Stop new URBs until endpoint reset has discarded the open data window. */
	q->active = false;
	q->stalled = true;
	urb->urb->actual_length = urb->receive_offset;
	bce_vhci_urb_complete(urb, status);
	bce_vhci_transfer_queue_request_reset(q);
	return -ENOENT;
}

int bce_vhci_urb_request_cancel(struct bce_vhci_transfer_queue *q,
				struct urb *urb, int status)
{
	struct bce_vhci_urb *vurb;
	unsigned long flags;
	int ret;
	enum bce_vhci_urb_state old_state;

	spin_lock_irqsave(&q->urb_lock, flags);
	ret = usb_hcd_check_unlink_urb(q->vhci->hcd, urb, status);
	if (ret) {
		spin_unlock_irqrestore(&q->urb_lock, flags);
		return ret;
	}

	vurb = urb->hcpriv;
	if (!vurb) {
		spin_unlock_irqrestore(&q->urb_lock, flags);
		return -ENOENT;
	}

	old_state = vurb->state;
	bce_vhci_urb_log_control(vurb, "cancel-request");

	if (old_state == BCE_VHCI_URB_CANCELLED) {
		spin_unlock_irqrestore(&q->urb_lock, flags);
		pr_debug("URB %p is already cancelled, skipping\n", urb);
		return 0;
	}

	vurb->state = BCE_VHCI_URB_CANCELLED;
	vurb->cancel_status = status;
	vurb->cancel_was_active = old_state != BCE_VHCI_URB_INIT_PENDING;

	if (list_empty(&vurb->cancel_list))
		list_add_tail(&vurb->cancel_list, &q->cancel_urb_list);
	spin_unlock_irqrestore(&q->urb_lock, flags);
	queue_work(q->vhci->tq_state_wq, &q->w_cancel);

	return 0;
}

static int bce_vhci_urb_prepare_in_submission(struct bce_vhci_urb *urb,
					      u32 requested_size)
{
	size_t submitted_size = requested_size;
	u32 offset = urb->receive_offset;
	int status;

	if (bce_vhci_urb_uses_sg(urb) && requested_size) {
		status = apple_bce_sq_add_sglist(urb->q->sq_in, urb->sg_list,
						 offset, requested_size,
						 &submitted_size);
		if (status)
			return status;
	} else {
		apple_bce_sq_add(urb->q->sq_in, urb->urb->transfer_dma + offset,
				 requested_size);
	}

	urb->submitted_length = submitted_size;
	return 0;
}

static int bce_vhci_urb_continue_in(struct bce_vhci_urb *urb)
{
	unsigned long timeout = 0;
	int status;

	if (apple_bce_sq_reserve(urb->q->sq_in, &timeout))
		return -EPIPE;

	status = bce_vhci_urb_prepare_in_submission(
		urb, urb->request_end - urb->receive_offset);
	if (status) {
		apple_bce_sq_unreserve(urb->q->sq_in);
		return status;
	}

	apple_bce_sq_submit(urb->q->sq_in);
	urb->state = BCE_VHCI_URB_WAITING_FOR_COMPLETION;
	return 0;
}

static int bce_vhci_urb_data_transfer_in(struct bce_vhci_urb *urb,
					 unsigned long *timeout)
{
	struct bce_vhci_message msg;
	u32 request_length;
	int reservation1, reservation2 = -EFAULT;
	int status;

	/* The firmware request and the inbound DMA submission must be queued as a pair. */
	reservation1 = apple_bce_sq_reserve(urb->q->vhci->msg_asynchronous.sq,
					    timeout);
	if (!reservation1)
		reservation2 = apple_bce_sq_reserve(urb->q->sq_in, timeout);
	if (reservation1 || reservation2) {
		pr_err("Failed to reserve a submission for URB data transfer\n");
		if (!reservation1)
			apple_bce_sq_unreserve(
				urb->q->vhci->msg_asynchronous.sq);
		return -ENOMEM;
	}

	request_length = urb->urb->transfer_buffer_length - urb->receive_offset;
	urb->request_end = urb->receive_offset + request_length;
	status = bce_vhci_urb_prepare_in_submission(urb, request_length);
	if (status) {
		apple_bce_sq_unreserve(urb->q->sq_in);
		apple_bce_sq_unreserve(urb->q->vhci->msg_asynchronous.sq);
		pr_err("[%02x] failed to prepare SG IN submission: %d\n",
		       urb->q->endp_addr, status);
		return status;
	}

	spin_lock(&urb->q->vhci->msg_asynchronous_lock);
	msg.cmd = BCE_VHCI_CMD_TRANSFER_REQUEST;
	msg.status = 0;
	msg.param1 = ((urb->urb->ep->desc.bEndpointAddress & 0x8Fu) << 8) |
		     urb->q->dev_addr;
	msg.param2 = request_length;
	bce_vhci_message_queue_write(&urb->q->vhci->msg_asynchronous, &msg);
	spin_unlock(&urb->q->vhci->msg_asynchronous_lock);

	apple_bce_sq_submit(urb->q->sq_in);

	urb->state = BCE_VHCI_URB_WAITING_FOR_COMPLETION;
	return 0;
}

static int bce_vhci_urb_data_start(struct bce_vhci_urb *urb,
				   unsigned long *timeout)
{
	if (urb->dir == DMA_TO_DEVICE) {
		if (urb->urb->transfer_buffer_length > 0 ||
		    bce_vhci_urb_is_zlp(urb))
			urb->state = BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST;
		else
			urb->state = BCE_VHCI_URB_DATA_TRANSFER_COMPLETE;
		return 0;
	} else {
		return bce_vhci_urb_data_transfer_in(urb, timeout);
	}
}

static int bce_vhci_urb_send_out_data(struct bce_vhci_urb *urb, dma_addr_t addr,
				      size_t size)
{
	unsigned long timeout = 0;

	if (apple_bce_sq_reserve(urb->q->sq_out, &timeout)) {
		pr_err("Failed to reserve a submission for URB data transfer\n");
		return -EPIPE;
	}

	apple_bce_sq_add(urb->q->sq_out, addr, size);
	atomic_inc(&urb->q->sq_out_pending);
	apple_bce_sq_submit(urb->q->sq_out);
	return 0;
}

static int bce_vhci_urb_send_out_transfer_data(struct bce_vhci_urb *urb,
					       u32 offset, u32 *size)
{
	unsigned long timeout = 0;
	size_t submitted_size;
	int status;

	if (apple_bce_sq_reserve(urb->q->sq_out, &timeout)) {
		pr_err("Failed to reserve a submission for URB data transfer\n");
		return -EPIPE;
	}

	if (!*size) {
		apple_bce_sq_add(urb->q->sq_out, 0, 0);
	} else if (bce_vhci_urb_uses_sg(urb)) {
		status = apple_bce_sq_add_sglist(urb->q->sq_out, urb->sg_list,
						 offset, *size,
						 &submitted_size);
		if (status) {
			apple_bce_sq_unreserve(urb->q->sq_out);
			pr_err("[%02x] failed to prepare SG OUT submission: %d\n",
			       urb->q->endp_addr, status);
			return status;
		}
		*size = submitted_size;
	} else {
		apple_bce_sq_add(urb->q->sq_out,
				 urb->urb->transfer_dma + offset, *size);
	}

	atomic_inc(&urb->q->sq_out_pending);
	apple_bce_sq_submit(urb->q->sq_out);
	return 0;
}

static int bce_vhci_urb_continue_out(struct bce_vhci_urb *urb)
{
	u32 tr_len;
	int status;

	if (urb->receive_offset < urb->request_end)
		tr_len = urb->request_end - urb->receive_offset;
	else if (bce_vhci_urb_is_zlp(urb))
		tr_len = 0;
	else
		return -EINVAL;

	status = bce_vhci_urb_send_out_transfer_data(urb, urb->receive_offset,
						     &tr_len);
	if (status)
		return status;

	urb->submitted_length = tr_len;
	urb->state = BCE_VHCI_URB_WAITING_FOR_COMPLETION;
	return 0;
}

static int bce_vhci_urb_data_update(struct bce_vhci_urb *urb,
				    struct bce_vhci_message *msg)
{
	int status;

	if (urb->state == BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST) {
		if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST) {
			u32 remaining = urb->urb->transfer_buffer_length -
					urb->receive_offset;
			u32 request_length = min_t(u64, remaining, msg->param2);

			if (!remaining && !bce_vhci_urb_is_zlp(urb))
				return -EAGAIN;

			if (remaining && !request_length) {
				pr_err("[%02x] device requested an empty OUT window with %u bytes remaining\n",
				       urb->q->endp_addr, remaining);
				return bce_vhci_urb_fail_data_transfer(urb,
								       -EPROTO);
			}

			urb->request_end = urb->receive_offset + request_length;
			status = bce_vhci_urb_continue_out(urb);
			if (status)
				return bce_vhci_urb_fail_data_transfer(urb,
								       status);
			return 0;
		}
	}

	/* OUT endpoints can receive extra transfer requests while waiting for host data. */
	if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST && urb->q->sq_out)
		return -EAGAIN;
	pr_err("[%02x] %s URB unexpected message (state = %x, msg: %x %x %x %llx)\n",
	       urb->q->endp_addr,
	       (urb->is_control ? "Control (data update)" : "Data"), urb->state,
	       msg->cmd, msg->status, msg->param1, msg->param2);
	return -EAGAIN;
}

static int bce_vhci_urb_data_transfer_completion(struct bce_vhci_urb *urb,
						 struct apple_bce_completion *c)
{
	if (urb->state == BCE_VHCI_URB_WAITING_FOR_COMPLETION) {
		int status;

		if (urb->receive_offset > urb->urb->transfer_buffer_length ||
		    c->data_size > urb->submitted_length ||
		    c->data_size > urb->urb->transfer_buffer_length -
					   urb->receive_offset) {
			pr_err("[%02x] completion overrun (offset=%u size=%llu submitted=%u total=%u)\n",
			       urb->q->endp_addr, urb->receive_offset,
			       c->data_size, urb->submitted_length,
			       urb->urb->transfer_buffer_length);
			return bce_vhci_urb_fail_data_transfer(urb, -EOVERFLOW);
		}

		urb->receive_offset += c->data_size;
		if (c->status != APPLE_BCE_STATUS_SUCCESS) {
			pr_err("[%02x] data submission failed (status=%u, transferred=%llu)\n",
			       urb->q->endp_addr, c->status, c->data_size);
			return bce_vhci_urb_fail_data_transfer(urb, -EIO);
		}

		if (urb->dir == DMA_TO_DEVICE &&
		    c->data_size != urb->submitted_length) {
			pr_err("[%02x] short OUT completion (%llu != %u)\n",
			       urb->q->endp_addr, c->data_size,
			       urb->submitted_length);
			return bce_vhci_urb_fail_data_transfer(urb, -EREMOTEIO);
		}

		if (urb->dir == DMA_FROM_DEVICE &&
		    c->data_size < urb->submitted_length) {
			urb->urb->actual_length = urb->receive_offset;
			if (urb->urb->transfer_flags & URB_SHORT_NOT_OK) {
				bce_vhci_urb_complete(urb, -EREMOTEIO);
				return -ENOENT;
			}

			urb->state = BCE_VHCI_URB_DATA_TRANSFER_COMPLETE;
			if (!urb->is_control) {
				bce_vhci_urb_complete(urb, 0);
				return -ENOENT;
			}
			return 0;
		}

		if (urb->dir == DMA_FROM_DEVICE &&
		    urb->receive_offset < urb->request_end) {
			status = bce_vhci_urb_continue_in(urb);
			if (status)
				return bce_vhci_urb_fail_data_transfer(urb,
								       status);
		} else if (urb->dir == DMA_TO_DEVICE &&
			   urb->receive_offset < urb->request_end) {
			status = bce_vhci_urb_continue_out(urb);
			if (status)
				return bce_vhci_urb_fail_data_transfer(urb,
								       status);
		} else if (urb->dir == DMA_TO_DEVICE &&
			   urb->receive_offset <
				   urb->urb->transfer_buffer_length) {
			urb->state = BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST;
		} else if (urb->dir == DMA_FROM_DEVICE ||
			   urb->receive_offset >=
				   urb->urb->transfer_buffer_length) {
			urb->urb->actual_length = (u32)urb->receive_offset;
			urb->state = BCE_VHCI_URB_DATA_TRANSFER_COMPLETE;
			if (!urb->is_control) {
				bce_vhci_urb_complete(urb, 0);
				return -ENOENT;
			}
		}
	} else {
		pr_warn_ratelimited(
			"[%02x] data URB unexpected completion state=%x status=%x size=%llu\n",
			urb->q->endp_addr, urb->state, c->status, c->data_size);
	}
	return 0;
}

static int bce_vhci_urb_control_check_status(struct bce_vhci_urb *urb)
{
	struct bce_vhci_transfer_queue *q = urb->q;

	if (urb->received_status == 0)
		return 0;
	if (urb->state == BCE_VHCI_URB_DATA_TRANSFER_COMPLETE ||
	    (urb->received_status != BCE_VHCI_SUCCESS &&
	     urb->state != BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_REQUEST &&
	     urb->state != BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_COMPLETION)) {
		urb->state = BCE_VHCI_URB_CONTROL_COMPLETE;
		if (urb->received_status != BCE_VHCI_SUCCESS) {
			if (urb->received_status == 3 &&
			    urb->q->endp_addr == 0x00) {
				q->active = false;
				q->stalled = true;
				q->state = BCE_VHCI_ENDPOINT_STALLED;
				pr_debug(
					"EP0 status=3 dev=%u state=%x; completing urb and scheduling endpoint reset paused_by=%x remaining=%u\n",
					q->dev_addr, urb->state, q->paused_by,
					q->remaining_active_requests);
				bce_vhci_urb_complete(urb, -EPIPE);
				bce_vhci_transfer_queue_request_reset(q);
				return -ENOENT;
			}

			pr_err("[%02x] URB failed: %x\n", urb->q->endp_addr,
			       urb->received_status);
			urb->q->active = false;
			urb->q->stalled = true;
			bce_vhci_urb_complete(urb, -EPIPE);
			if (!list_empty(&q->endp->urb_list))
				bce_vhci_transfer_queue_request_reset(q);
			return -ENOENT;
		}
		bce_vhci_urb_complete(urb, 0);
		return -ENOENT;
	}
	return 0;
}

static int bce_vhci_urb_control_update(struct bce_vhci_urb *urb,
				       struct bce_vhci_message *msg)
{
	int status;

	if (msg->cmd == BCE_VHCI_CMD_CONTROL_TRANSFER_STATUS) {
		urb->received_status = msg->status;
		return bce_vhci_urb_control_check_status(urb);
	}

	if (urb->state == BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_REQUEST) {
		if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST) {
			if (bce_vhci_urb_send_out_data(
				    urb, urb->urb->setup_dma,
				    sizeof(struct usb_ctrlrequest))) {
				pr_err("[%02x] Failed to start URB setup transfer\n",
				       urb->q->endp_addr);
				/* the T2 reports the failure with a later transfer status */
				return 0;
			}
			urb->state =
				BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_COMPLETION;
			return 0;
		}
	} else if (urb->state == BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST ||
		   urb->state == BCE_VHCI_URB_WAITING_FOR_COMPLETION) {
		status = bce_vhci_urb_data_update(urb, msg);
		if (status)
			return status;
		return bce_vhci_urb_control_check_status(urb);
	}

	/* OUT endpoints can receive extra transfer requests while waiting for host data. */
	if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST && urb->q->sq_out)
		return -EAGAIN;
	pr_err("[%02x] Control URB unexpected message (state = %x, msg: %x %x %x %llx)\n",
	       urb->q->endp_addr, urb->state, msg->cmd, msg->status,
	       msg->param1, msg->param2);
	return -EAGAIN;
}

static int
bce_vhci_urb_control_transfer_completion(struct bce_vhci_urb *urb,
					 struct apple_bce_completion *c)
{
	int status;
	unsigned long timeout;

	if (urb->state == BCE_VHCI_URB_CONTROL_WAITING_FOR_SETUP_COMPLETION) {
		if (c->data_size != sizeof(struct usb_ctrlrequest))
			pr_err("[%02x] transfer complete data size mistmatch for usb_ctrlrequest (%llx instead of %lx)\n",
			       urb->q->endp_addr, c->data_size,
			       sizeof(struct usb_ctrlrequest));

		timeout = 1000;
		status = bce_vhci_urb_data_start(urb, &timeout);
		if (status) {
			bce_vhci_urb_complete(urb, status);
			return -ENOENT;
		}
		return 0;
	} else if (urb->state == BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST ||
		   urb->state == BCE_VHCI_URB_WAITING_FOR_COMPLETION) {
		status = bce_vhci_urb_data_transfer_completion(urb, c);
		if (status)
			return status;
		return bce_vhci_urb_control_check_status(urb);
	}
	pr_err("[%02x] Control URB unexpected completion (state = %x)\n",
	       urb->q->endp_addr, urb->state);
	return 0;
}

static int bce_vhci_urb_update(struct bce_vhci_urb *urb,
			       struct bce_vhci_message *msg)
{
	if (urb->state == BCE_VHCI_URB_INIT_PENDING)
		return -EAGAIN;
	if (urb->is_control)
		return bce_vhci_urb_control_update(urb, msg);
	else
		return bce_vhci_urb_data_update(urb, msg);
}

static int bce_vhci_urb_transfer_completion(struct bce_vhci_urb *urb,
					    struct apple_bce_completion *c)
{
	if (urb->is_control)
		return bce_vhci_urb_control_transfer_completion(urb, c);
	else
		return bce_vhci_urb_data_transfer_completion(urb, c);
}

static void bce_vhci_urb_resume(struct bce_vhci_urb *urb)
{
	int status = 0;

	if (urb->state != BCE_VHCI_URB_WAITING_FOR_COMPLETION)
		return;

	if (urb->dir == DMA_FROM_DEVICE) {
		status = bce_vhci_urb_data_transfer_in(urb, NULL);
	} else {
		/* The paused OUT submission was flushed; wait for a fresh window. */
		urb->request_end = urb->receive_offset;
		urb->submitted_length = 0;
		urb->state = BCE_VHCI_URB_WAITING_FOR_TRANSFER_REQUEST;
	}
	if (status)
		bce_vhci_urb_complete(urb, status);
}
