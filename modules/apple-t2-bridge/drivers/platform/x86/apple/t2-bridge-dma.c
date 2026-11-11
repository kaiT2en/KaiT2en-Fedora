// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 Bridge Controller DMA queues.
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#include <linux/bitfield.h>
#include <linux/dma-mapping.h>
#include <linux/dmapool.h>
#include <linux/err.h>
#include <linux/io.h>
#include <linux/module.h>
#include <linux/overflow.h>
#include <linux/pci.h>
#include <linux/scatterlist.h>
#include <linux/slab.h>
#include <linux/string.h>

#include "t2-bridge.h"

/*
 * Each queue has a 32 bit doorbell at REG_DOORBELL + 4 * qid. For an SQ the
 * host writes the new tail, for a CQ the index of the next element it will
 * read.
 */
#define REG_DOORBELL		0x44000

#define BCE_CMD_QUEUE_SIZE	0x20
#define BCE_CMD_SIZE		0x40
#define BCE_CMD_TIMEOUT_MS	5000
#define BCE_CMD_SLOT_TIMEOUT_MS	(5 * 60 * 1000)

/* Queue configuration, for the command queues passed through the mailbox */
struct bce_queue_memcfg {
	u16 qid;
	u16 el_count;
	u16 vector_or_cq;	/* CQ: interrupt vector, SQ: qid of its CQ */
	u16 _pad;
	u64 addr;
	u64 length;
} __packed;
static_assert(sizeof(struct bce_queue_memcfg) == 24);

enum bce_cmd {
	BCE_CMD_REGISTER_MEMORY_QUEUE = 0x20,
	BCE_CMD_UNREGISTER_MEMORY_QUEUE = 0x30,
	BCE_CMD_FLUSH_MEMORY_QUEUE = 0x40,
};

#define BCE_CMD_FLAG_TO_DEVICE	BIT(0)
#define BCE_CMD_FLAG_NAMED	BIT(1)

struct bce_cmd_simple {
	u16 cmd;
	u16 flags;
	u16 qid;
} __packed;

struct bce_cmd_register_queue {
	u16 cmd;
	u16 flags;
	u16 qid;
	u16 _pad;
	u16 el_count;
	u16 vector_or_cq;
	u16 _pad2;
	u16 name_len;
	char name[32];
	u64 addr;
	u64 length;
} __packed;
static_assert(sizeof(struct bce_cmd_register_queue) <= BCE_CMD_SIZE);
static_assert(sizeof(struct bce_qe_completion) == 24);

/*
 * SQ element. A single buffer uses addr and length. A segment list submission
 * puts the list in segl_addr/segl_length and selects a byte range of the
 * listed data with addr (offset) and length.
 */
struct bce_qe_submission {
	u64 length;
	u64 addr;
	u64 segl_addr;
	u64 segl_length;
} __packed;

/*
 * A segment list is one page: this header followed by the elements. The T2
 * accepts at most 32 elements per list. Chaining lists through next_segl_*
 * makes the T2 raise a CATERR, so longer lists are split into chunks that are
 * submitted separately.
 */
struct bce_sglist_header {
	u64 element_count;
	u64 data_size;
	u64 next_segl_addr;
	u64 next_segl_length;
} __packed;

struct bce_sglist_element {
	u64 addr;
	u64 length;
} __packed;

#define BCE_SGLIST_MAX_ELEMENTS						\
	min_t(size_t, 32,						\
	      (PAGE_SIZE - sizeof(struct bce_sglist_header)) /		\
	      sizeof(struct bce_sglist_element))

struct bce_sglist_chunk {
	void *data;
	dma_addr_t dma;
	size_t data_offset;
	size_t data_size;
};

struct apple_bce_sglist {
	unsigned int chunk_count;
	size_t data_size;
	struct bce_sglist_chunk chunks[] __counted_by(chunk_count);
};

static void __iomem *bce_doorbell(struct apple_bce *bce, u16 qid)
{
	return bce->dma_regs + REG_DOORBELL + qid * sizeof(u32);
}

static struct apple_bce_cq *bce_cq_alloc(struct apple_bce *bce, u16 qid,
					 u32 count)
{
	struct apple_bce_cq *cq;

	cq = kzalloc_obj(*cq);
	if (!cq)
		return NULL;

	cq->q.bce = bce;
	cq->q.qid = qid;
	cq->q.type = BCE_QUEUE_CQ;
	cq->el_count = count;
	cq->data = dma_alloc_coherent(&bce->pdev->dev,
				      count * sizeof(*cq->data), &cq->dma,
				      GFP_KERNEL);
	if (!cq->data) {
		kfree(cq);
		return NULL;
	}
	return cq;
}

static void bce_cq_free(struct apple_bce_cq *cq)
{
	dma_free_coherent(&cq->q.bce->pdev->dev,
			  cq->el_count * sizeof(*cq->data), cq->data, cq->dma);
	kfree(cq);
}

static struct apple_bce_sq *bce_sq_alloc(struct apple_bce *bce, u16 qid,
					 u32 el_size, u32 count,
					 apple_bce_sq_complete_t complete,
					 void *priv)
{
	struct apple_bce_sq *sq;

	sq = kzalloc_obj(*sq);
	if (!sq)
		return NULL;

	sq->q.bce = bce;
	sq->q.qid = qid;
	sq->q.type = BCE_QUEUE_SQ;
	sq->el_size = el_size;
	sq->el_count = count;
	sq->complete = complete;
	sq->priv = priv;
	atomic_set(&sq->available, count - 1);
	init_completion(&sq->slot_freed);
	atomic_set(&sq->slot_waiters, 0);
	init_waitqueue_head(&sq->idle_wait);

	sq->completions = kzalloc_objs(*sq->completions, count);
	sq->data = dma_alloc_coherent(&bce->pdev->dev, count * el_size,
				      &sq->dma, GFP_KERNEL);
	if (!sq->completions || !sq->data) {
		if (sq->data)
			dma_free_coherent(&bce->pdev->dev, count * el_size,
					  sq->data, sq->dma);
		kfree(sq->completions);
		kfree(sq);
		return NULL;
	}
	return sq;
}

static void bce_sq_free(struct apple_bce_sq *sq)
{
	dma_free_coherent(&sq->q.bce->pdev->dev, sq->el_count * sq->el_size,
			  sq->data, sq->dma);
	kfree(sq->completions);
	kfree(sq);
}

static void bce_cq_memcfg(struct apple_bce_cq *cq, struct bce_queue_memcfg *cfg)
{
	*cfg = (struct bce_queue_memcfg) {
		.qid = cq->q.qid,
		.el_count = cq->el_count,
		.addr = cq->dma,
		.length = cq->el_count * sizeof(*cq->data),
	};
}

static void bce_sq_memcfg(struct apple_bce_sq *sq, struct apple_bce_cq *cq,
			  struct bce_queue_memcfg *cfg)
{
	*cfg = (struct bce_queue_memcfg) {
		.qid = sq->q.qid,
		.el_count = sq->el_count,
		.vector_or_cq = cq->q.qid,
		.addr = sq->dma,
		.length = sq->el_count * sq->el_size,
	};
}

static void bce_cq_handle_one(struct apple_bce *bce,
			      struct bce_qe_completion *e, size_t *ncompleted)
{
	struct apple_bce_completion *c;
	struct apple_bce_sq *sq;
	struct bce_queue *q;

	q = e->qid < BCE_MAX_QUEUES ? bce->queues[e->qid] : NULL;
	if (!q || q->type != BCE_QUEUE_SQ) {
		dev_err(&bce->pdev->dev, "completion for unknown queue %u\n",
			e->qid);
		return;
	}

	sq = container_of(q, struct apple_bce_sq, q);
	if (!sq->completion_queued) {
		sq->completion_queued = true;
		bce->completed[(*ncompleted)++] = sq;
	}

	c = &sq->completions[sq->completion_tail];
	if (e->completion_index != sq->completion_tail) {
		dev_err(&bce->pdev->dev,
			"queue %u: completion index %u, expected %u\n",
			e->qid, e->completion_index, sq->completion_tail);
		*c = (struct apple_bce_completion) {
			.status = APPLE_BCE_STATUS_ERROR,
		};
	} else {
		c->status = e->status;
		c->data_size = e->data_size;
		c->result = e->result;
	}
	/* publish the slot before the tail that makes it visible */
	wmb();
	sq->completion_tail = (sq->completion_tail + 1) % sq->el_count;
}

static void bce_cq_handle(struct apple_bce *bce, struct apple_bce_cq *cq,
			  size_t *ncompleted)
{
	struct bce_qe_completion *e = &cq->data[cq->index];

	if (!(e->flags & BCE_COMPLETION_FLAG_PENDING))
		return;

	/* read the elements only after the pending flag */
	mb();
	while (true) {
		e = &cq->data[cq->index];
		if (!(e->flags & BCE_COMPLETION_FLAG_PENDING))
			break;
		bce_cq_handle_one(bce, e, ncompleted);
		e->flags = 0;
		cq->index = (cq->index + 1) % cq->el_count;
	}
	/* the cleared flags must be visible before the T2 reuses the slots */
	mb();
	iowrite32(cq->index, bce_doorbell(bce, cq->q.qid));
}

void bce_dma_process_completions(struct apple_bce *bce)
{
	struct apple_bce_sq *sq;
	size_t ncompleted = 0;
	int i;

	lockdep_assert_held(&bce->completion_lock);

	scoped_guard(spinlock, &bce->queues_lock) {
		for (i = 0; i < BCE_MAX_QUEUES; i++) {
			struct bce_queue *q = bce->queues[i];

			if (q && q->type == BCE_QUEUE_CQ)
				bce_cq_handle(bce, container_of(q, struct apple_bce_cq, q),
					      &ncompleted);
		}
	}

	while (ncompleted) {
		sq = bce->completed[--ncompleted];
		sq->complete(sq);
		sq->completion_queued = false;
	}
}

static void *bce_sq_next_element(struct apple_bce_sq *sq)
{
	void *el = sq->data + sq->el_size * sq->tail;

	sq->tail = (sq->tail + 1) % sq->el_count;
	return el;
}

/* Command queue, carries the queue (un)registration of all other queues */

static void bce_cmdq_complete(struct apple_bce_sq *sq)
{
	struct apple_bce *bce = sq->priv;
	struct bce_cmdq *cmdq = &bce->cmdq;
	struct apple_bce_completion *c;
	struct bce_cmdq_waiter *w;

	guard(spinlock)(&cmdq->lock);
	while ((c = apple_bce_sq_next_completion(sq))) {
		w = cmdq->waiters[sq->head];
		if (w && w->generation == cmdq->generation[sq->head]) {
			w->status = c->status;
			w->result = c->result;
			complete(&w->done);
		}
		cmdq->waiters[sq->head] = NULL;
		apple_bce_sq_complete(sq);
	}
}

static void *bce_cmd_start(struct bce_cmdq *cmdq, struct bce_cmdq_waiter *w)
{
	unsigned long timeout = msecs_to_jiffies(BCE_CMD_SLOT_TIMEOUT_MS);
	void *cmd;

	init_completion(&w->done);
	/* the waiter must be initialised before its slot is published */
	mb();
	if (apple_bce_sq_reserve(cmdq->sq, &timeout))
		return NULL;

	/* released in bce_cmd_finish() */
	spin_lock(&cmdq->lock);
	w->slot = cmdq->sq->tail;
	w->generation = cmdq->generation[w->slot];
	cmdq->waiters[w->slot] = w;
	cmd = bce_sq_next_element(cmdq->sq);
	return cmd;
}

static int bce_cmd_finish(struct bce_cmdq *cmdq, struct bce_cmdq_waiter *w)
{
	struct apple_bce *bce = cmdq->sq->q.bce;

	apple_bce_sq_submit(cmdq->sq);
	spin_unlock(&cmdq->lock);

	if (!wait_for_completion_timeout(&w->done,
					 msecs_to_jiffies(BCE_CMD_TIMEOUT_MS))) {
		dev_err(&bce->pdev->dev, "command timeout in slot %u\n", w->slot);
		/*
		 * The waiter lives on our stack, a late completion must not
		 * find it. The slot stays with the T2 until it completes it,
		 * bce_cmdq_complete() releases it then in queue order.
		 */
		scoped_guard(spinlock, &cmdq->lock) {
			cmdq->waiters[w->slot] = NULL;
			cmdq->generation[w->slot]++;
		}
		return -ETIMEDOUT;
	}
	/* read the result only after the completion */
	mb();
	return w->status == APPLE_BCE_STATUS_SUCCESS ? 0 : -EIO;
}

static int bce_cmd_register_queue(struct apple_bce *bce,
				  struct bce_queue_memcfg *cfg,
				  const char *name, bool to_device)
{
	struct bce_cmd_register_queue *cmd;
	struct bce_cmdq_waiter w;

	cmd = bce_cmd_start(&bce->cmdq, &w);
	if (!cmd)
		return -ESHUTDOWN;

	cmd->cmd = BCE_CMD_REGISTER_MEMORY_QUEUE;
	cmd->flags = (name ? BCE_CMD_FLAG_NAMED : 0) |
		     (to_device ? BCE_CMD_FLAG_TO_DEVICE : 0);
	cmd->qid = cfg->qid;
	cmd->el_count = cfg->el_count;
	cmd->vector_or_cq = cfg->vector_or_cq;
	memset(cmd->name, 0, sizeof(cmd->name));
	if (name) {
		cmd->name_len = min(strlen(name), sizeof(cmd->name));
		memcpy(cmd->name, name, cmd->name_len);
	} else {
		cmd->name_len = 0;
	}
	cmd->addr = cfg->addr;
	cmd->length = cfg->length;

	return bce_cmd_finish(&bce->cmdq, &w);
}

static int bce_cmd_simple(struct apple_bce *bce, u16 command, u16 qid)
{
	struct bce_cmd_simple *cmd;
	struct bce_cmdq_waiter w;

	cmd = bce_cmd_start(&bce->cmdq, &w);
	if (!cmd)
		return -ESHUTDOWN;

	cmd->cmd = command;
	cmd->flags = 0;
	cmd->qid = qid;
	return bce_cmd_finish(&bce->cmdq, &w);
}

static int bce_mbox_register_queue(struct apple_bce *bce,
				   struct bce_queue_memcfg *cfg,
				   enum bce_mb_type type)
{
	struct device *dev = &bce->pdev->dev;
	dma_addr_t addr;
	u64 reply;
	int ret;

	addr = dma_map_single(dev, cfg, sizeof(*cfg), DMA_TO_DEVICE);
	if (dma_mapping_error(dev, addr))
		return -ENOMEM;

	ret = bce_mbox_send(&bce->mbox, FIELD_PREP(BCE_MB_TYPE, type) |
				FIELD_PREP(BCE_MB_VALUE, addr), &reply);
	dma_unmap_single(dev, addr, sizeof(*cfg), DMA_TO_DEVICE);
	if (ret)
		return ret;
	if (FIELD_GET(BCE_MB_TYPE, reply) != BCE_MB_REGISTER_COMMAND_QUEUE_REPLY)
		return -EIO;
	return 0;
}

static int bce_cmdq_init(struct apple_bce *bce)
{
	struct bce_cmdq *cmdq = &bce->cmdq;
	struct bce_queue_memcfg *cfg;
	int ret = -ENOMEM;

	spin_lock_init(&cmdq->lock);
	cmdq->waiters = kzalloc_objs(*cmdq->waiters, BCE_CMD_QUEUE_SIZE);
	cmdq->generation = kzalloc_objs(*cmdq->generation, BCE_CMD_QUEUE_SIZE);
	bce->cmd_cq = bce_cq_alloc(bce, BCE_QID_CMD_CQ, BCE_CMD_QUEUE_SIZE);
	cmdq->sq = bce_sq_alloc(bce, BCE_QID_CMD_SQ, BCE_CMD_SIZE,
				BCE_CMD_QUEUE_SIZE, bce_cmdq_complete, bce);
	/* kmalloc memory, the mailbox hands it to the T2 by DMA */
	cfg = kzalloc_obj(*cfg);
	if (!cmdq->waiters || !cmdq->generation || !bce->cmd_cq || !cmdq->sq ||
	    !cfg)
		goto err;

	bce->queues[BCE_QID_CMD_CQ] = &bce->cmd_cq->q;
	bce->queues[BCE_QID_CMD_SQ] = &cmdq->sq->q;

	bce_cq_memcfg(bce->cmd_cq, cfg);
	ret = bce_mbox_register_queue(bce, cfg, BCE_MB_REGISTER_COMMAND_CQ);
	if (ret)
		goto err;
	bce_sq_memcfg(cmdq->sq, bce->cmd_cq, cfg);
	ret = bce_mbox_register_queue(bce, cfg, BCE_MB_REGISTER_COMMAND_SQ);
	if (ret)
		goto err;

	kfree(cfg);
	return 0;

err:
	bce->queues[BCE_QID_CMD_CQ] = NULL;
	bce->queues[BCE_QID_CMD_SQ] = NULL;
	kfree(cfg);
	if (cmdq->sq)
		bce_sq_free(cmdq->sq);
	if (bce->cmd_cq)
		bce_cq_free(bce->cmd_cq);
	kfree(cmdq->generation);
	kfree(cmdq->waiters);
	return ret;
}

static void bce_cmdq_exit(struct apple_bce *bce)
{
	struct bce_cmdq *cmdq = &bce->cmdq;

	scoped_guard(spinlock, &bce->queues_lock) {
		bce->queues[BCE_QID_CMD_CQ] = NULL;
		bce->queues[BCE_QID_CMD_SQ] = NULL;
	}
	bce_sq_free(cmdq->sq);
	bce_cq_free(bce->cmd_cq);
	kfree(cmdq->generation);
	kfree(cmdq->waiters);
}

int bce_dma_init(struct apple_bce *bce)
{
	int ret;

	spin_lock_init(&bce->queues_lock);
	ida_init(&bce->qids);
	mutex_init(&bce->completion_lock);
	atomic_set(&bce->submit_open, 1);

	bce->sglist_pool = dma_pool_create("apple-bce-sglist", &bce->pdev->dev,
					   PAGE_SIZE, PAGE_SIZE, 0);
	if (!bce->sglist_pool)
		return -ENOMEM;

	ret = bce_cmdq_init(bce);
	if (ret)
		dma_pool_destroy(bce->sglist_pool);
	return ret;
}

void bce_dma_exit(struct apple_bce *bce)
{
	bce_cmdq_exit(bce);
	dma_pool_destroy(bce->sglist_pool);
	ida_destroy(&bce->qids);
}

/*
 * Close the submission gate and wait for the command queue to drain. Clients
 * get -ESHUTDOWN from apple_bce_sq_reserve() until bce_dma_open().
 *
 * Return: 0 or -ETIMEDOUT if the T2 still holds commands, the gate is open
 * again then.
 */
int bce_dma_close(struct apple_bce *bce)
{
	struct apple_bce_sq *sq = bce->cmdq.sq;

	atomic_set(&bce->submit_open, 0);
	/* close the gate before draining, reservations re-check it */
	smp_mb();

	scoped_guard(mutex, &bce->completion_lock)
		bce_dma_process_completions(bce);

	if (!wait_event_timeout(sq->idle_wait, apple_bce_sq_idle(sq),
				msecs_to_jiffies(BCE_CMD_TIMEOUT_MS))) {
		dev_err(&bce->pdev->dev, "command queue did not drain\n");
		bce_dma_open(bce);
		return -ETIMEDOUT;
	}
	return 0;
}

void bce_dma_open(struct apple_bce *bce)
{
	atomic_set(&bce->submit_open, 1);
	/* open the gate before completions are processed again */
	smp_mb();
}

/* Client interface */

/**
 * apple_bce_dma_device - device to use for DMA mappings of a client
 * @bce: bridge instance
 *
 * Return: the bridge PCI device.
 */
struct device *apple_bce_dma_device(struct apple_bce *bce)
{
	return &bce->pdev->dev;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_dma_device, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sync_completions - wait for running completion callbacks
 * @bce: bridge instance
 */
void apple_bce_sync_completions(struct apple_bce *bce)
{
	synchronize_irq(pci_irq_vector(bce->pdev, 4));
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sync_completions, "APPLE_T2_BRIDGE");

static int bce_qid_alloc(struct apple_bce *bce)
{
	return ida_alloc_range(&bce->qids, BCE_QID_FIRST, BCE_QID_LAST,
			       GFP_KERNEL);
}

static void bce_queue_publish(struct apple_bce *bce, struct bce_queue *q)
{
	guard(spinlock)(&bce->queues_lock);
	bce->queues[q->qid] = q;
}

static void bce_queue_unpublish(struct apple_bce *bce, struct bce_queue *q)
{
	scoped_guard(spinlock, &bce->queues_lock)
		bce->queues[q->qid] = NULL;
	ida_free(&bce->qids, q->qid);
}

/**
 * apple_bce_cq_create - create and register a completion queue
 * @bce: bridge instance
 * @count: number of elements
 *
 * Return: the CQ or NULL on failure.
 */
struct apple_bce_cq *apple_bce_cq_create(struct apple_bce *bce, u32 count)
{
	struct bce_queue_memcfg cfg;
	struct apple_bce_cq *cq;
	int qid;

	qid = bce_qid_alloc(bce);
	if (qid < 0)
		return NULL;

	cq = bce_cq_alloc(bce, qid, count);
	if (!cq)
		goto err_qid;

	bce_cq_memcfg(cq, &cfg);
	if (bce_cmd_register_queue(bce, &cfg, NULL, false)) {
		dev_err(&bce->pdev->dev, "failed to register CQ %d\n", qid);
		bce_cq_free(cq);
		goto err_qid;
	}

	bce_queue_publish(bce, &cq->q);
	return cq;

err_qid:
	ida_free(&bce->qids, qid);
	return NULL;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_cq_create, "APPLE_T2_BRIDGE");

/**
 * apple_bce_cq_destroy - unregister and free a completion queue
 * @cq: CQ, all its SQs must be destroyed already
 */
void apple_bce_cq_destroy(struct apple_bce_cq *cq)
{
	struct apple_bce *bce = cq->q.bce;

	if (!bce->removing &&
	    bce_cmd_simple(bce, BCE_CMD_UNREGISTER_MEMORY_QUEUE, cq->q.qid))
		dev_err(&bce->pdev->dev, "failed to unregister CQ %u\n", cq->q.qid);
	bce_queue_unpublish(bce, &cq->q);
	bce_cq_free(cq);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_cq_destroy, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_create - create and register a submission queue
 * @bce: bridge instance
 * @cq: CQ that receives the completions
 * @name: T2 service to connect to
 * @count: number of elements, one of them always stays unused
 * @dir: %DMA_TO_DEVICE or %DMA_FROM_DEVICE
 * @complete: completion callback
 * @priv: client data, see apple_bce_sq_priv()
 *
 * Return: the SQ or NULL on failure.
 */
struct apple_bce_sq *apple_bce_sq_create(struct apple_bce *bce,
					 struct apple_bce_cq *cq,
					 const char *name, u32 count,
					 enum dma_data_direction dir,
					 apple_bce_sq_complete_t complete,
					 void *priv)
{
	struct bce_queue_memcfg cfg;
	struct apple_bce_sq *sq;
	int qid;

	if (!cq || !name ||
	    (dir != DMA_TO_DEVICE && dir != DMA_FROM_DEVICE))
		return NULL;

	qid = bce_qid_alloc(bce);
	if (qid < 0)
		return NULL;

	sq = bce_sq_alloc(bce, qid, sizeof(struct bce_qe_submission), count,
			  complete, priv);
	if (!sq)
		goto err_qid;

	bce_sq_memcfg(sq, cq, &cfg);
	if (bce_cmd_register_queue(bce, &cfg, name, dir == DMA_TO_DEVICE)) {
		dev_err(&bce->pdev->dev, "failed to register SQ %s\n", name);
		bce_sq_free(sq);
		goto err_qid;
	}

	bce_queue_publish(bce, &sq->q);
	return sq;

err_qid:
	ida_free(&bce->qids, qid);
	return NULL;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_create, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_destroy - unregister and free a submission queue
 * @sq: SQ
 */
void apple_bce_sq_destroy(struct apple_bce_sq *sq)
{
	struct apple_bce *bce = sq->q.bce;

	if (!bce->removing &&
	    bce_cmd_simple(bce, BCE_CMD_UNREGISTER_MEMORY_QUEUE, sq->q.qid))
		dev_err(&bce->pdev->dev, "failed to unregister SQ %u\n", sq->q.qid);
	bce_queue_unpublish(bce, &sq->q);
	bce_sq_free(sq);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_destroy, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_flush - abort all submissions of a queue
 * @sq: SQ
 *
 * Pending submissions complete with %APPLE_BCE_STATUS_ABORTED.
 *
 * Return: 0 or a negative errno.
 */
int apple_bce_sq_flush(struct apple_bce_sq *sq)
{
	return bce_cmd_simple(sq->q.bce, BCE_CMD_FLUSH_MEMORY_QUEUE, sq->q.qid);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_flush, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_priv - client data of a submission queue
 * @sq: SQ
 *
 * Return: the @priv passed to apple_bce_sq_create().
 */
void *apple_bce_sq_priv(struct apple_bce_sq *sq)
{
	return sq->priv;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_priv, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_reserve - reserve one submission slot
 * @sq: SQ
 * @timeout: in jiffies, updated with the remaining time. NULL or 0 does not
 *	wait.
 *
 * Every successful reservation is followed by apple_bce_sq_add() or
 * apple_bce_sq_add_sglist(), or released with apple_bce_sq_unreserve().
 *
 * Return: 0, -EAGAIN if the queue is full or -ESHUTDOWN while the bridge
 * is suspended.
 */
int apple_bce_sq_reserve(struct apple_bce_sq *sq, unsigned long *timeout)
{
	atomic_t *open = &sq->q.bce->submit_open;

	if (!atomic_read(open))
		return -ESHUTDOWN;

	while (atomic_dec_if_positive(&sq->available) < 0) {
		if (!timeout || !*timeout)
			return -EAGAIN;

		atomic_inc(&sq->slot_waiters);
		*timeout = wait_for_completion_timeout(&sq->slot_freed, *timeout);
		if (!*timeout &&
		    atomic_dec_if_positive(&sq->slot_waiters) < 0)
			try_wait_for_completion(&sq->slot_freed);

		if (!atomic_read(open))
			return -ESHUTDOWN;
	}

	if (!atomic_read(open)) {
		atomic_inc(&sq->available);
		return -ESHUTDOWN;
	}
	return 0;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_reserve, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_unreserve - release a reservation that was not used
 * @sq: SQ
 */
void apple_bce_sq_unreserve(struct apple_bce_sq *sq)
{
	atomic_inc(&sq->available);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_unreserve, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_add - queue a single buffer
 * @sq: SQ with a reserved slot
 * @addr: DMA address
 * @size: size in bytes
 */
void apple_bce_sq_add(struct apple_bce_sq *sq, dma_addr_t addr, size_t size)
{
	struct bce_qe_submission *el = bce_sq_next_element(sq);

	*el = (struct bce_qe_submission) {
		.addr = addr,
		.length = size,
	};
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_add, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_add_sglist - queue a byte range of a segment list
 * @sq: SQ with a reserved slot
 * @list: segment list
 * @offset: start of the range within the list
 * @size: size of the range
 * @added: bytes actually queued
 *
 * One submission covers at most one chunk of the list. If @added is less than
 * @size the caller queues the rest with further submissions.
 *
 * Return: 0 or -EINVAL.
 */
int apple_bce_sq_add_sglist(struct apple_bce_sq *sq,
			    const struct apple_bce_sglist *list,
			    size_t offset, size_t size, size_t *added)
{
	const struct bce_sglist_chunk *chunk = NULL;
	struct bce_qe_submission *el;
	size_t chunk_offset;
	unsigned int i;

	if (!list || !size || offset >= list->data_size ||
	    size > list->data_size - offset)
		return -EINVAL;

	for (i = 0; i < list->chunk_count; i++) {
		if (offset >= list->chunks[i].data_offset &&
		    offset - list->chunks[i].data_offset <
		    list->chunks[i].data_size) {
			chunk = &list->chunks[i];
			break;
		}
	}
	if (!chunk)
		return -EINVAL;

	chunk_offset = offset - chunk->data_offset;
	*added = min(size, chunk->data_size - chunk_offset);

	el = bce_sq_next_element(sq);
	*el = (struct bce_qe_submission) {
		.addr = chunk_offset,
		.length = *added,
		.segl_addr = chunk->dma,
		.segl_length = PAGE_SIZE,
	};
	return 0;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_add_sglist, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_submit - hand all queued submissions to the T2
 * @sq: SQ
 */
void apple_bce_sq_submit(struct apple_bce_sq *sq)
{
	/* the elements must be visible before the doorbell */
	mb();
	iowrite32(sq->tail, bce_doorbell(sq->q.bce, sq->q.qid));
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_submit, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_next_completion - fetch the next completion
 * @sq: SQ
 *
 * The completion belongs to the submission at apple_bce_sq_head(). It stays
 * valid until apple_bce_sq_complete().
 *
 * Return: the completion or NULL if none is pending.
 */
struct apple_bce_completion *apple_bce_sq_next_completion(struct apple_bce_sq *sq)
{
	struct apple_bce_completion *c;

	/* pairs with the wmb() in bce_cq_handle_one() */
	rmb();
	if (sq->completion_head == sq->completion_tail)
		return NULL;
	c = &sq->completions[sq->completion_head];
	sq->completion_head = (sq->completion_head + 1) % sq->el_count;
	return c;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_next_completion, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_complete - release the slot at the head of the queue
 * @sq: SQ
 */
void apple_bce_sq_complete(struct apple_bce_sq *sq)
{
	sq->head = (sq->head + 1) % sq->el_count;
	atomic_inc(&sq->available);
	wake_up_all(&sq->idle_wait);
	if (atomic_dec_if_positive(&sq->slot_waiters) >= 0)
		complete(&sq->slot_freed);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_complete, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_head - index of the oldest submission in flight
 * @sq: SQ
 */
u32 apple_bce_sq_head(struct apple_bce_sq *sq)
{
	return sq->head;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_head, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_tail - index the next submission is written to
 * @sq: SQ
 */
u32 apple_bce_sq_tail(struct apple_bce_sq *sq)
{
	return sq->tail;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_tail, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_size - number of elements of a queue
 * @sq: SQ
 */
u32 apple_bce_sq_size(struct apple_bce_sq *sq)
{
	return sq->el_count;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_size, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sq_idle - check whether no submission is in flight
 * @sq: SQ
 */
bool apple_bce_sq_idle(struct apple_bce_sq *sq)
{
	return atomic_read(&sq->available) == sq->el_count - 1;
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sq_idle, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sglist_create - build segment lists for a mapped scatterlist
 * @bce: bridge instance
 * @sgl: scatterlist mapped for apple_bce_dma_device()
 * @nents: number of mapped entries
 * @gfp: allocation flags
 *
 * Return: the list or an ERR_PTR().
 */
struct apple_bce_sglist *apple_bce_sglist_create(struct apple_bce *bce,
						 struct scatterlist *sgl,
						 unsigned int nents, gfp_t gfp)
{
	struct apple_bce_sglist *list;
	unsigned int count, i;
	struct scatterlist *sg;
	int ret;

	if (!sgl || !nents)
		return ERR_PTR(-EINVAL);

	count = DIV_ROUND_UP(nents, BCE_SGLIST_MAX_ELEMENTS);
	list = kzalloc_flex(*list, chunks, count, gfp);
	if (!list)
		return ERR_PTR(-ENOMEM);
	list->chunk_count = count;

	for_each_sg(sgl, sg, nents, i) {
		unsigned int idx = i % BCE_SGLIST_MAX_ELEMENTS;
		struct bce_sglist_chunk *chunk;
		struct bce_sglist_element *el;
		struct bce_sglist_header *hdr;
		size_t len = sg_dma_len(sg);

		chunk = &list->chunks[i / BCE_SGLIST_MAX_ELEMENTS];
		if (!idx) {
			chunk->data = dma_pool_zalloc(bce->sglist_pool, gfp,
						      &chunk->dma);
			if (!chunk->data) {
				ret = -ENOMEM;
				goto err;
			}
			chunk->data_offset = list->data_size;
		}

		if (!len) {
			ret = -EINVAL;
			goto err;
		}
		if (check_add_overflow(chunk->data_size, len, &chunk->data_size) ||
		    check_add_overflow(list->data_size, len, &list->data_size)) {
			ret = -EOVERFLOW;
			goto err;
		}

		hdr = chunk->data;
		el = (struct bce_sglist_element *)(hdr + 1);
		el[idx].addr = sg_dma_address(sg);
		el[idx].length = len;
		hdr->element_count++;
		hdr->data_size = chunk->data_size;
	}

	return list;

err:
	apple_bce_sglist_destroy(bce, list);
	return ERR_PTR(ret);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sglist_create, "APPLE_T2_BRIDGE");

/**
 * apple_bce_sglist_destroy - free segment lists
 * @bce: bridge instance
 * @list: list from apple_bce_sglist_create(), may be NULL
 */
void apple_bce_sglist_destroy(struct apple_bce *bce,
			      struct apple_bce_sglist *list)
{
	unsigned int i;

	if (!list)
		return;

	for (i = 0; i < list->chunk_count; i++)
		if (list->chunks[i].data)
			dma_pool_free(bce->sglist_pool, list->chunks[i].data,
				      list->chunks[i].dma);
	kfree(list);
}
EXPORT_SYMBOL_NS_GPL(apple_bce_sglist_destroy, "APPLE_T2_BRIDGE");
