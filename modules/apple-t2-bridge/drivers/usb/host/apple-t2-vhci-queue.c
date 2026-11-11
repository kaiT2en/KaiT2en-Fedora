// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 virtual USB host controller, message queues
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "apple-t2-vhci-queue.h"
#include "apple-t2-vhci.h"

#include <linux/dma-mapping.h>

static void bce_vhci_message_queue_completion(struct apple_bce_sq *sq);

int bce_vhci_message_queue_create(struct bce_vhci *vhci,
				  struct bce_vhci_message_queue *ret,
				  const char *name)
{
	int status;

	ret->cq = apple_bce_cq_create(vhci->bce, VHCI_EVENT_QUEUE_EL_COUNT);
	if (!ret->cq)
		return -EINVAL;
	ret->sq = apple_bce_sq_create(vhci->bce, ret->cq, name,
				      VHCI_EVENT_QUEUE_EL_COUNT, DMA_TO_DEVICE,
				      bce_vhci_message_queue_completion, ret);
	if (!ret->sq) {
		status = -EINVAL;
		goto fail_cq;
	}
	ret->data = dma_alloc_coherent(apple_bce_dma_device(vhci->bce),
				       sizeof(struct bce_vhci_message) *
					       VHCI_EVENT_QUEUE_EL_COUNT,
				       &ret->dma_addr, GFP_KERNEL);
	if (!ret->data) {
		status = -EINVAL;
		goto fail_sq;
	}
	return 0;

fail_sq:
	apple_bce_sq_destroy(ret->sq);
	ret->sq = NULL;
fail_cq:
	apple_bce_cq_destroy(ret->cq);
	ret->cq = NULL;
	return status;
}

void bce_vhci_message_queue_destroy(struct bce_vhci *vhci,
				    struct bce_vhci_message_queue *q)
{
	if (!q->cq)
		return;
	dma_free_coherent(apple_bce_dma_device(vhci->bce),
			  sizeof(struct bce_vhci_message) *
				  VHCI_EVENT_QUEUE_EL_COUNT,
			  q->data, q->dma_addr);
	apple_bce_sq_destroy(q->sq);
	apple_bce_cq_destroy(q->cq);
}

void bce_vhci_message_queue_write(struct bce_vhci_message_queue *q,
				  struct bce_vhci_message *req)
{
	int sidx;

	sidx = apple_bce_sq_tail(q->sq);
	q->data[sidx] = *req;
	apple_bce_sq_add(q->sq,
			 q->dma_addr + sizeof(struct bce_vhci_message) * sidx,
			 sizeof(struct bce_vhci_message));
	apple_bce_sq_submit(q->sq);
}

static void bce_vhci_message_queue_completion(struct apple_bce_sq *sq)
{
	while (apple_bce_sq_next_completion(sq))
		apple_bce_sq_complete(sq);
}

static void bce_vhci_event_queue_completion(struct apple_bce_sq *sq);

int __bce_vhci_event_queue_create(struct bce_vhci *vhci,
				  struct bce_vhci_event_queue *ret,
				  const char *name,
				  apple_bce_sq_complete_t compl)
{
	ret->vhci = vhci;

	ret->sq = apple_bce_sq_create(vhci->bce, vhci->ev_cq, name,
				      VHCI_EVENT_QUEUE_EL_COUNT,
				      DMA_FROM_DEVICE, compl, ret);
	if (!ret->sq)
		return -EINVAL;
	ret->data = dma_alloc_coherent(apple_bce_dma_device(vhci->bce),
				       sizeof(struct bce_vhci_message) *
					       VHCI_EVENT_QUEUE_EL_COUNT,
				       &ret->dma_addr, GFP_KERNEL);
	if (!ret->data) {
		apple_bce_sq_destroy(ret->sq);
		ret->sq = NULL;
		return -EINVAL;
	}

	init_completion(&ret->queue_empty_completion);
	bce_vhci_event_queue_submit_pending(ret, VHCI_EVENT_PENDING_COUNT);
	return 0;
}

int bce_vhci_event_queue_create(struct bce_vhci *vhci,
				struct bce_vhci_event_queue *ret,
				const char *name,
				bce_vhci_event_queue_callback cb)
{
	ret->cb = cb;
	return __bce_vhci_event_queue_create(vhci, ret, name,
					     bce_vhci_event_queue_completion);
}

void bce_vhci_event_queue_destroy(struct bce_vhci *vhci,
				  struct bce_vhci_event_queue *q)
{
	if (!q->sq)
		return;
	dma_free_coherent(apple_bce_dma_device(vhci->bce),
			  sizeof(struct bce_vhci_message) *
				  VHCI_EVENT_QUEUE_EL_COUNT,
			  q->data, q->dma_addr);
	apple_bce_sq_destroy(q->sq);
}

static void bce_vhci_event_queue_completion(struct apple_bce_sq *sq)
{
	struct apple_bce_completion *cd;
	struct bce_vhci_event_queue *ev = apple_bce_sq_priv(sq);
	struct bce_vhci_message *msg;
	size_t cnt = 0;

	while ((cd = apple_bce_sq_next_completion(sq))) {
		if (cd->status == APPLE_BCE_STATUS_ABORTED) {
			apple_bce_sq_complete(sq);
			continue;
		}
		msg = &ev->data[apple_bce_sq_head(sq)];
		ev->cb(ev, msg);

		apple_bce_sq_complete(sq);
		++cnt;
	}
	bce_vhci_event_queue_submit_pending(ev, cnt);
	if (apple_bce_sq_idle(sq))
		complete(&ev->queue_empty_completion);
}

void bce_vhci_event_queue_submit_pending(struct bce_vhci_event_queue *q,
					 size_t count)
{
	int idx;

	while (count--) {
		if (apple_bce_sq_reserve(q->sq, NULL)) {
			pr_err("Failed to reserve an event queue submission\n");
			break;
		}
		idx = apple_bce_sq_tail(q->sq);
		apple_bce_sq_add(q->sq,
				 q->dma_addr +
					 idx * sizeof(struct bce_vhci_message),
				 sizeof(struct bce_vhci_message));
	}
	apple_bce_sq_submit(q->sq);
}

int bce_vhci_event_queue_pause(struct bce_vhci_event_queue *q)
{
	int status;
	unsigned long timeout;

	reinit_completion(&q->queue_empty_completion);
	status = apple_bce_sq_flush(q->sq);
	if (status) {
		pr_err("failed to flush event queue: %d\n", status);
		return status;
	}
	timeout = msecs_to_jiffies(5000);
	while (!apple_bce_sq_idle(q->sq)) {
		timeout = wait_for_completion_timeout(
			&q->queue_empty_completion, timeout);
		if (timeout == 0) {
			pr_err("waiting for queue to be flushed timed out\n");
			return -ETIMEDOUT;
		}
	}
	return 0;
}

void bce_vhci_event_queue_resume(struct bce_vhci_event_queue *q)
{
	if (!apple_bce_sq_idle(q->sq)) {
		pr_err("resume of a queue with pending submissions\n");
		return;
	}
	bce_vhci_event_queue_submit_pending(q, VHCI_EVENT_PENDING_COUNT);
}

void bce_vhci_command_queue_create(struct bce_vhci_command_queue *ret,
				   struct bce_vhci_message_queue *mq)
{
	ret->mq = mq;
	ret->completion.result = NULL;
	init_completion(&ret->completion.completion);
	spin_lock_init(&ret->completion_lock);
	mutex_init(&ret->mutex);
}

void bce_vhci_command_queue_destroy(struct bce_vhci_command_queue *cq)
{
	spin_lock(&cq->completion_lock);
	if (cq->completion.result) {
		memset(cq->completion.result, 0,
		       sizeof(struct bce_vhci_message));
		cq->completion.result->status = BCE_VHCI_ABORT;
		complete(&cq->completion.completion);
		cq->completion.result = NULL;
	}
	spin_unlock(&cq->completion_lock);
	mutex_lock(&cq->mutex);
	mutex_unlock(&cq->mutex);
	mutex_destroy(&cq->mutex);
}

void bce_vhci_command_queue_deliver_completion(
	struct bce_vhci_command_queue *cq, struct bce_vhci_message *msg)
{
	struct bce_vhci_command_queue_completion *c = &cq->completion;

	spin_lock(&cq->completion_lock);
	if (c->result) {
		*c->result = *msg;
		complete(&c->completion);
		c->result = NULL;
	}
	spin_unlock(&cq->completion_lock);
}

static int __bce_vhci_command_queue_execute(struct bce_vhci_command_queue *cq,
					    struct bce_vhci_message *req,
					    struct bce_vhci_message *res,
					    unsigned long timeout)
{
	int status;
	struct bce_vhci_command_queue_completion *c;
	struct bce_vhci_message creq;

	c = &cq->completion;

	status = apple_bce_sq_reserve(cq->mq->sq, &timeout);
	if (status)
		return status;

	spin_lock(&cq->completion_lock);
	c->result = res;
	reinit_completion(&c->completion);
	spin_unlock(&cq->completion_lock);

	bce_vhci_message_queue_write(cq->mq, req);

	if (!wait_for_completion_timeout(&c->completion, timeout)) {
		/* Ask bridgeOS to cancel the command before reporting timeout. */
		pr_debug("command timed out req=%x\n", req->cmd);
		status = apple_bce_sq_reserve(cq->mq->sq, &timeout);
		if (status)
			return status;

		creq = *req;
		creq.cmd |= 0x4000;
		bce_vhci_message_queue_write(cq->mq, &creq);

		if (!wait_for_completion_timeout(&c->completion, 1000)) {
			pr_err("Possible desync, cmd cancel timed out\n");

			spin_lock(&cq->completion_lock);
			c->result = NULL;
			spin_unlock(&cq->completion_lock);
			return -ETIMEDOUT;
		}
		if ((res->cmd & ~0x8000) == creq.cmd)
			return -ETIMEDOUT;
		/* The original reply arrived after the cancellation was queued. */
	}

	if ((res->cmd & ~0x8000) != req->cmd) {
		pr_err("Possible desync, cmd reply mismatch req=%x, res=%x\n",
		       req->cmd, res->cmd);
		return -EIO;
	}
	if (res->status == BCE_VHCI_SUCCESS)
		return 0;
	return res->status;
}

int bce_vhci_command_queue_execute(struct bce_vhci_command_queue *cq,
				   struct bce_vhci_message *req,
				   struct bce_vhci_message *res,
				   unsigned long timeout)
{
	int status;

	mutex_lock(&cq->mutex);
	status = __bce_vhci_command_queue_execute(cq, req, res, timeout);
	mutex_unlock(&cq->mutex);
	return status;
}
