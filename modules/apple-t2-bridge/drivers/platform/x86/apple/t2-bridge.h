/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Apple T2 Bridge Controller, internal definitions.
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#ifndef _APPLE_T2_BRIDGE_H
#define _APPLE_T2_BRIDGE_H

#include <linux/apple-t2-bridge.h>
#include <linux/atomic.h>
#include <linux/bits.h>
#include <linux/completion.h>
#include <linux/hrtimer.h>
#include <linux/idr.h>
#include <linux/mutex.h>
#include <linux/spinlock.h>
#include <linux/types.h>
#include <linux/wait.h>

struct auxiliary_device;
struct dma_pool;
struct pci_dev;

/*
 * Mailbox messages carry a 6 bit type in bits 63:58 and a 58 bit value. Only
 * probe and system sleep use the mailbox. Everything else runs over the DMA
 * queues, which are themselves registered through the mailbox.
 */
#define BCE_MB_TYPE		GENMASK_ULL(63, 58)
#define BCE_MB_VALUE		GENMASK_ULL(57, 0)

enum bce_mb_type {
	BCE_MB_REGISTER_COMMAND_SQ = 0x07,	/* to T2, value: DMA addr of bce_queue_memcfg */
	BCE_MB_REGISTER_COMMAND_CQ = 0x08,	/* to T2, value: DMA addr of bce_queue_memcfg */
	BCE_MB_REGISTER_COMMAND_QUEUE_REPLY = 0x0b,
	BCE_MB_SET_FW_PROTOCOL_VERSION = 0x0c,	/* both directions, value: version */
	BCE_MB_SAVE_STATE_AND_SLEEP = 0x17,	/* to T2, value: state buffer */
	BCE_MB_SAVE_STATE_AND_SLEEP_GROW = 0x19, /* from T2, value: size needed */
	BCE_MB_SAVE_RESTORE_STATE_COMPLETE = 0x1a,
	BCE_MB_RESTORE_STATE_AND_WAKE = 0x1b,	/* to T2, value: state buffer */
};

/* Protocol version that supports SAVE_STATE_AND_SLEEP */
#define BCE_PROTOCOL_VERSION	0x20001

struct bce_mbox {
	void __iomem *regs;
	struct mutex lock;		/* the T2 handles one message at a time */
	atomic_t state;
	struct completion done;
	u64 reply;
};

struct bce_hosttime {
	void __iomem *regs;
	struct hrtimer timer;
	spinlock_t lock;		/* protects stopped */
	bool stopped;
};

enum bce_queue_type {
	BCE_QUEUE_CQ,
	BCE_QUEUE_SQ,
};

struct bce_queue {
	struct apple_bce *bce;
	u16 qid;
	enum bce_queue_type type;
};

/* Completion queue element written by the T2 */
struct bce_qe_completion {
	u64 result;
	u64 data_size;
	u16 qid;
	u16 completion_index;
	u16 status;
	u16 flags;
};

#define BCE_COMPLETION_FLAG_PENDING	BIT(15)

struct apple_bce_cq {
	struct bce_queue q;
	u32 el_count;
	dma_addr_t dma;
	struct bce_qe_completion *data;
	u32 index;
};

struct apple_bce_sq {
	struct bce_queue q;
	u32 el_size;
	u32 el_count;
	dma_addr_t dma;
	void *data;

	/*
	 * A ring of n elements holds at most n - 1 submissions, so head == tail
	 * always means empty.
	 */
	atomic_t available;
	struct completion slot_freed;
	atomic_t slot_waiters;
	wait_queue_head_t idle_wait;
	u32 head, tail;

	struct apple_bce_completion *completions;
	u32 completion_head, completion_tail;
	bool completion_queued;
	apple_bce_sq_complete_t complete;
	void *priv;
};

struct bce_cmdq_waiter {
	struct completion done;
	u32 status;
	u64 result;
	u32 slot;
	u32 generation;
};

struct bce_cmdq {
	struct apple_bce_sq *sq;
	spinlock_t lock;		/* protects waiters and generation */
	struct bce_cmdq_waiter **waiters;
	u32 *generation;
};

#define BCE_MAX_QUEUES		256
/* auxiliary devices for clients that sit on the bridge queues */
#define BCE_MAX_AUX		4
#define BCE_QID_CMD_CQ		0
#define BCE_QID_CMD_SQ		1
/* queues of clients, the highest 16 qids stay unused */
#define BCE_QID_FIRST		2
#define BCE_QID_LAST		(BCE_MAX_QUEUES - 16 - 1)

struct apple_bce {
	struct pci_dev *pdev;
	struct pci_dev *fn0;
	struct device_link *fn0_link;
	void __iomem *mbox_regs;
	void __iomem *dma_regs;
	u32 fw_version;

	struct bce_mbox mbox;
	struct bce_hosttime hosttime;

	struct bce_queue *queues[BCE_MAX_QUEUES];
	spinlock_t queues_lock;		/* protects queues[] */
	struct ida qids;
	struct apple_bce_cq *cmd_cq;
	struct bce_cmdq cmdq;
	struct dma_pool *sglist_pool;

	/* serialises completion processing between the IRQ thread and resume */
	struct mutex completion_lock;
	struct apple_bce_sq *completed[BCE_MAX_QUEUES];

	atomic_t submit_open;
	bool removing;
	bool dma_irq_disabled;

	void *state;
	size_t state_size;
	bool state_saved;

	struct auxiliary_device *aux[BCE_MAX_AUX];
};

/* t2-bridge-mailbox.c */
void bce_mbox_init(struct bce_mbox *mb, void __iomem *regs);
int bce_mbox_send(struct bce_mbox *mb, u64 msg, u64 *reply);
void bce_mbox_handle_irq(struct bce_mbox *mb);

void bce_hosttime_init(struct bce_hosttime *ht, void __iomem *regs);
void bce_hosttime_start(struct bce_hosttime *ht, bool initial);
void bce_hosttime_stop(struct bce_hosttime *ht);

/* t2-bridge-dma.c */
int bce_dma_init(struct apple_bce *bce);
void bce_dma_exit(struct apple_bce *bce);
void bce_dma_process_completions(struct apple_bce *bce);
int bce_dma_close(struct apple_bce *bce);
void bce_dma_open(struct apple_bce *bce);

#endif /* _APPLE_T2_BRIDGE_H */
