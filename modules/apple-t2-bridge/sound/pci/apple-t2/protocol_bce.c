// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 audio, message transport over the bridge
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "protocol_bce.h"

#include "audio.h"
#include <linux/dma-mapping.h>
#include <linux/err.h>
#include <linux/ktime.h>

static void t2audio_bce_out_queue_completion(struct apple_bce_sq *sq);
static void t2audio_bce_in_queue_completion(struct apple_bce_sq *sq);
static int t2audio_bce_queue_init(struct t2audio_device *dev,
				  struct t2audio_bce_queue *q, const char *name,
				  enum dma_data_direction direction,
				  apple_bce_sq_complete_t cfn);
static void t2audio_bce_in_queue_submit_pending(struct t2audio_bce_queue *q,
						size_t count);
static void t2audio_deferred_msg_work(struct work_struct *ws);

int t2audio_bce_init(struct t2audio_device *dev)
{
	int status;
	struct t2audio_bce *bce = &dev->bcem;

	bce->cq = apple_bce_cq_create(dev->bce, 0x80);
	spin_lock_init(&bce->spinlock);
	init_waitqueue_head(&bce->drain_wq);
	if (!bce->cq)
		return -EINVAL;
	status = t2audio_bce_queue_init(dev, &bce->qout,
					"com.apple.BridgeAudio.IntelToARM",
					DMA_TO_DEVICE,
					t2audio_bce_out_queue_completion);
	if (status)
		return status;
	status = t2audio_bce_queue_init(dev, &bce->qin,
					"com.apple.BridgeAudio.ARMToIntel",
					DMA_FROM_DEVICE,
					t2audio_bce_in_queue_completion);
	if (status)
		return status;
	t2audio_bce_in_queue_submit_pending(&bce->qin, bce->qin.el_count);
	return 0;
}

static void t2audio_bce_flush_queue(struct t2audio_device *dev,
				    struct t2audio_bce_queue *q,
				    const char *name)
{
	int status;

	if (!q->sq)
		return;
	status = apple_bce_sq_flush(q->sq);
	if (!status &&
	    !wait_event_timeout(dev->bcem.drain_wq, apple_bce_sq_idle(q->sq),
				msecs_to_jiffies(T2AUDIO_BCE_DRAIN_TIMEOUT_MS)))
		status = -ETIMEDOUT;
	if (status)
		pr_err("%s queue flush/completion wait failed (%d)\n", name,
		       status);
}

void t2audio_bce_stop_messages(struct t2audio_device *dev)
{
	struct t2audio_bce *bce = &dev->bcem;

	if (IS_ERR_OR_NULL(dev->bce))
		return;

	WRITE_ONCE(bce->teardown, true);
	apple_bce_sync_completions(dev->bce);
	if (dev->msg_wq) {
		destroy_workqueue(dev->msg_wq);
		dev->msg_wq = NULL;
	}
}

void t2audio_bce_exit(struct t2audio_device *dev)
{
	struct t2audio_bce *bce = &dev->bcem;

	if (IS_ERR_OR_NULL(dev->bce))
		return;

	t2audio_bce_stop_messages(dev);
	t2audio_bce_flush_queue(dev, &bce->qout, "out");
	t2audio_bce_flush_queue(dev, &bce->qin, "in");
	apple_bce_sync_completions(dev->bce);

	if (bce->qout.sq)
		apple_bce_sq_destroy(bce->qout.sq);
	if (bce->qin.sq)
		apple_bce_sq_destroy(bce->qin.sq);
	bce->qout.sq = NULL;
	bce->qin.sq = NULL;
	apple_bce_sync_completions(dev->bce);
	if (bce->cq)
		apple_bce_cq_destroy(bce->cq);
	bce->cq = NULL;

	if (bce->qout.data)
		dma_free_coherent(apple_bce_dma_device(dev->bce),
				  bce->qout.el_size * bce->qout.el_count,
				  bce->qout.data, bce->qout.dma_addr);
	if (bce->qin.data)
		dma_free_coherent(apple_bce_dma_device(dev->bce),
				  bce->qin.el_size * bce->qin.el_count,
				  bce->qin.data, bce->qin.dma_addr);
	bce->qout.data = NULL;
	bce->qin.data = NULL;
}

int t2audio_bce_queue_init(struct t2audio_device *dev,
			   struct t2audio_bce_queue *q, const char *name,
			   enum dma_data_direction direction,
			   apple_bce_sq_complete_t cfn)
{
	q->cq = dev->bcem.cq;
	q->el_size = T2AUDIO_BCE_QUEUE_ELEMENT_SIZE;
	q->el_count = T2AUDIO_BCE_QUEUE_ELEMENT_COUNT;
	/* one slot of the ring always stays empty */
	q->sq = apple_bce_sq_create(dev->bce, q->cq, name,
				    (u32)(q->el_count + 1), direction, cfn,
				    dev);
	if (!q->sq)
		return -EINVAL;

	q->data = dma_alloc_coherent(apple_bce_dma_device(dev->bce),
				     q->el_size * q->el_count, &q->dma_addr,
				     GFP_KERNEL);
	if (!q->data) {
		apple_bce_sq_destroy(q->sq);
		q->sq = NULL;
		return -EINVAL;
	}
	return 0;
}

static void t2audio_send_create_tag(struct t2audio_bce *b, int *tagn,
				    char tag[4])
{
	b->tag_num = (b->tag_num + 1) % T2AUDIO_BCE_QUEUE_TAG_COUNT;
	*tagn = b->tag_num;
	tag[0] = 'S';
	tag[1] = '0' + b->tag_num / 100;
	tag[2] = '0' + (b->tag_num / 10) % 10;
	tag[3] = '0' + b->tag_num % 10;
}

int __t2audio_send_prepare(struct t2audio_bce *b, struct t2audio_send_ctx *ctx,
			   char *tag)
{
	int status;
	size_t index;
	void *dptr;
	struct t2audio_msg_header *header;

	status = apple_bce_sq_reserve(b->qout.sq, &ctx->timeout);
	if (status)
		return status;
	spin_lock_irqsave(&b->spinlock, ctx->irq_flags);
	index = b->qout.data_tail;
	dptr = (u8 *)b->qout.data + index * b->qout.el_size;
	ctx->msg.data = dptr;
	header = dptr;
	if (tag)
		*((u32 *)header->tag) = *((u32 *)tag);
	else
		t2audio_send_create_tag(b, &ctx->tag_n, header->tag);
	return 0;
}

void __t2audio_send(struct t2audio_bce *b, struct t2audio_send_ctx *ctx)
{
	print_hex_dump_debug(KBUILD_MODNAME " out: ", DUMP_PREFIX_NONE, 32, 1,
			     ctx->msg.data, ctx->msg.size, true);
	apple_bce_sq_add(b->qout.sq,
			 b->qout.dma_addr +
				 (dma_addr_t)(ctx->msg.data - b->qout.data),
			 ctx->msg.size);
	apple_bce_sq_submit(b->qout.sq);
	b->qout.data_tail = (b->qout.data_tail + 1) % b->qout.el_count;
	spin_unlock_irqrestore(&b->spinlock, ctx->irq_flags);
}

int __t2audio_send_cmd_sync(struct t2audio_bce *b, struct t2audio_send_ctx *ctx,
			    struct t2audio_msg *reply)
{
	struct t2audio_bce_queue_entry ent;
	DECLARE_COMPLETION_ONSTACK(cmpl);

	ent.msg = reply;
	ent.cmpl = &cmpl;
	b->pending_entries[ctx->tag_n] = &ent;
	__t2audio_send(b, ctx); /* drops b->spinlock */
	ctx->timeout = wait_for_completion_timeout(&cmpl, ctx->timeout);
	if (ctx->timeout == 0) {
		/*
		 * The reply may still arrive. Remove the waiter so it cannot
		 * complete a stack object that is gone.
		 */
		spin_lock_irqsave(&b->spinlock, ctx->irq_flags);
		if (b->pending_entries[ctx->tag_n] == &ent)
			b->pending_entries[ctx->tag_n] = NULL;
		spin_unlock_irqrestore(&b->spinlock, ctx->irq_flags);
		return -ETIMEDOUT;
	}
	return 0;
}

static void t2audio_handle_reply(struct t2audio_bce *b,
				 struct t2audio_msg *reply)
{
	const char *tag;
	int tagn;
	unsigned long irq_flags;
	char tag_zero[5];
	struct t2audio_bce_queue_entry *entry;

	tag = ((struct t2audio_msg_header *)reply->data)->tag;
	if (tag[0] != 'S') {
		pr_err("unexpected reply tag: %.4s\n", tag);
		return;
	}
	*((u32 *)tag_zero) = *((u32 *)tag);
	tag_zero[4] = 0;
	if (kstrtoint(&tag_zero[1], 10, &tagn)) {
		pr_err("reply tag parse failed: %.4s\n", tag);
		return;
	}

	spin_lock_irqsave(&b->spinlock, irq_flags);
	entry = b->pending_entries[tagn];
	if (entry) {
		if (reply->size < entry->msg->size)
			entry->msg->size = reply->size;
		memcpy(entry->msg->data, reply->data, entry->msg->size);
		complete(entry->cmpl);

		b->pending_entries[tagn] = NULL;
	} else {
		pr_warn_ratelimited(
			"stale reply without queued command tag=%.4s\n", tag);
	}
	spin_unlock_irqrestore(&b->spinlock, irq_flags);
}

static void t2audio_bce_out_queue_completion(struct apple_bce_sq *sq)
{
	struct t2audio_device *dev = apple_bce_sq_priv(sq);

	while (apple_bce_sq_next_completion(sq))
		apple_bce_sq_complete(sq);
	wake_up(&dev->bcem.drain_wq);
}

static void t2audio_bce_in_queue_handle_msg(struct t2audio_device *a,
					    struct t2audio_msg *msg);

static void t2audio_deferred_msg_work(struct work_struct *ws)
{
	struct t2audio_deferred_msg *work =
		container_of(ws, struct t2audio_deferred_msg, ws);
	struct t2audio_msg_header *header = work->msg.data;

	if (header->type == T2AUDIO_MSG_TYPE_COMMAND)
		t2audio_handle_command(work->a, &work->msg);
	else if (header->type == T2AUDIO_MSG_TYPE_NOTIFICATION)
		t2audio_handle_notification(work->a, &work->msg);

	kfree(work->msg.data);
	kfree(work);
}

static void t2audio_bce_in_queue_completion(struct apple_bce_sq *sq)
{
	struct t2audio_msg msg;
	struct t2audio_device *dev = apple_bce_sq_priv(sq);
	struct t2audio_bce_queue *q = &dev->bcem.qin;
	struct apple_bce_completion *c;
	size_t cnt = 0;

	/* read the messages only after their completions */
	mb();
	while ((c = apple_bce_sq_next_completion(sq))) {
		msg.data = (u8 *)q->data + q->data_head * q->el_size;
		msg.size = c->data_size;
		msg.received_ns = ktime_get_raw_ns();
		print_hex_dump_debug(KBUILD_MODNAME " in: ", DUMP_PREFIX_NONE,
				     32, 1, msg.data, min(msg.size, 128UL),
				     true);
		t2audio_bce_in_queue_handle_msg(dev, &msg);

		q->data_head = (q->data_head + 1) % q->el_count;

		apple_bce_sq_complete(sq);
		++cnt;
	}
	if (!READ_ONCE(dev->bcem.teardown))
		t2audio_bce_in_queue_submit_pending(q, cnt);
	wake_up(&dev->bcem.drain_wq);
}

static void t2audio_bce_in_queue_handle_msg(struct t2audio_device *a,
					    struct t2audio_msg *msg)
{
	struct t2audio_msg_header *header =
		(struct t2audio_msg_header *)msg->data;
	struct t2audio_deferred_msg *work;

	if (READ_ONCE(a->bcem.teardown))
		return;
	if (msg->size < sizeof(struct t2audio_msg_header)) {
		pr_err("Msg size smaller than header (%lx)", msg->size);
		return;
	}
	if (header->type == T2AUDIO_MSG_TYPE_RESPONSE) {
		t2audio_handle_reply(&a->bcem, msg);
		return;
	}

	work = kzalloc(sizeof(*work), GFP_ATOMIC);
	if (!work)
		return;

	work->msg.data = kmemdup(msg->data, msg->size, GFP_ATOMIC);
	if (!work->msg.data) {
		kfree(work);
		pr_err("Failed to copy deferred message\n");
		return;
	}

	work->a = a;
	work->msg.size = msg->size;
	work->msg.received_ns = msg->received_ns;
	INIT_WORK(&work->ws, t2audio_deferred_msg_work);
	queue_work(a->msg_wq, &work->ws);
}

static void t2audio_bce_in_queue_submit_pending(struct t2audio_bce_queue *q,
						size_t count)
{
	while (count--) {
		if (apple_bce_sq_reserve(q->sq, NULL)) {
			pr_err("Failed to reserve an event queue submission\n");
			break;
		}
		apple_bce_sq_add(
			q->sq,
			q->dma_addr + (dma_addr_t)(q->data_tail * q->el_size),
			q->el_size);
		q->data_tail = (q->data_tail + 1) % q->el_count;
	}
	apple_bce_sq_submit(q->sq);
}

struct t2audio_msg t2audio_reply_alloc(void)
{
	struct t2audio_msg ret;

	ret.size = T2AUDIO_BCE_QUEUE_ELEMENT_SIZE;
	ret.data = kmalloc(ret.size, GFP_KERNEL);
	return ret;
}

void t2audio_reply_free(struct t2audio_msg *reply)
{
	kfree(reply->data);
}
