// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 Bridge Controller mailbox and host time.
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#include <linux/bitfield.h>
#include <linux/io.h>
#include <linux/ktime.h>

#include "t2-bridge.h"

/*
 * Outgoing messages are written as four 32 bit words, of which only the first
 * two carry the message. Replies queue up in a FIFO, the number of pending
 * replies is in REG_MBOX_REPLY_COUNT.
 */
#define REG_MBOX_OUT		0x820
#define REG_MBOX_REPLY_COUNT	0x108
#define  MBOX_REPLY_COUNT	GENMASK(23, 20)
#define REG_MBOX_REPLY		0x810

#define BCE_MBOX_TIMEOUT_MS	1000

enum {
	BCE_MBOX_IDLE,
	BCE_MBOX_WAITING,
	BCE_MBOX_REPLIED,
};

void bce_mbox_init(struct bce_mbox *mb, void __iomem *regs)
{
	mb->regs = regs;
	mutex_init(&mb->lock);
	atomic_set(&mb->state, BCE_MBOX_IDLE);
	init_completion(&mb->done);
}

int bce_mbox_send(struct bce_mbox *mb, u64 msg, u64 *reply)
{
	void __iomem *out = mb->regs + REG_MBOX_OUT;
	int ret = 0;

	guard(mutex)(&mb->lock);

	if (atomic_cmpxchg(&mb->state, BCE_MBOX_IDLE, BCE_MBOX_WAITING) !=
	    BCE_MBOX_IDLE)
		return -EEXIST;
	reinit_completion(&mb->done);

	iowrite32(lower_32_bits(msg), out);
	iowrite32(upper_32_bits(msg), out + 4);
	iowrite32(0, out + 8);
	iowrite32(0, out + 12);

	wait_for_completion_timeout(&mb->done,
				    msecs_to_jiffies(BCE_MBOX_TIMEOUT_MS));
	if (atomic_read(&mb->state) != BCE_MBOX_REPLIED) {
		atomic_set(&mb->state, BCE_MBOX_IDLE);
		return -ETIMEDOUT;
	}

	*reply = mb->reply;
	atomic_set(&mb->state, BCE_MBOX_IDLE);
	return ret;
}

void bce_mbox_handle_irq(struct bce_mbox *mb)
{
	void __iomem *fifo = mb->regs + REG_MBOX_REPLY;
	unsigned int count;
	u32 lo, hi;

	count = FIELD_GET(MBOX_REPLY_COUNT,
			  ioread32(mb->regs + REG_MBOX_REPLY_COUNT));
	if (!count)
		return;

	/* Every read of the FIFO pops one entry, the last one wins. */
	while (count--) {
		lo = ioread32(fifo);
		hi = ioread32(fifo + 4);
		ioread32(fifo + 8);
		ioread32(fifo + 12);
	}
	mb->reply = ((u64)hi << 32) | lo;

	if (atomic_read(&mb->state) == BCE_MBOX_WAITING) {
		atomic_set(&mb->state, BCE_MBOX_REPLIED);
		complete(&mb->done);
	} else {
		/* a reply nobody waits for */
		atomic_set(&mb->state, BCE_MBOX_IDLE);
	}
}

/*
 * The T2 needs the host's view of time. The host writes its clock to this
 * block every 125 ms. A tick reads REG_HOSTTIME_LO first, then writes the
 * low and the high word. Instead of a time the host writes these markers to
 * REG_HOSTTIME_LO, each followed by HOSTTIME_MARK_END to REG_HOSTTIME_HI:
 * HOSTTIME_MARK_PROBE before the command queues are registered,
 * HOSTTIME_MARK_RESUME on resume and HOSTTIME_MARK_STOP before sleep.
 */
#define REG_HOSTTIME_HI		0xc000
#define REG_HOSTTIME_LO		0xc008
#define HOSTTIME_MARK_END	((u32)-1)
#define HOSTTIME_MARK_STOP	((u32)-2)
#define HOSTTIME_MARK_RESUME	((u32)-3)
#define HOSTTIME_MARK_PROBE	((u32)-4)

#define BCE_HOSTTIME_INTERVAL_NS	(125 * NSEC_PER_MSEC)
#define BCE_HOSTTIME_SLACK_NS		(200 * NSEC_PER_MSEC)

static void bce_hosttime_mark(struct bce_hosttime *ht, u32 mark)
{
	iowrite32(mark, ht->regs + REG_HOSTTIME_LO);
	iowrite32(HOSTTIME_MARK_END, ht->regs + REG_HOSTTIME_HI);
}

static void bce_hosttime_schedule(struct bce_hosttime *ht)
{
	hrtimer_start_range_ns(&ht->timer, ns_to_ktime(BCE_HOSTTIME_INTERVAL_NS),
			       BCE_HOSTTIME_SLACK_NS, HRTIMER_MODE_REL);
}

static enum hrtimer_restart bce_hosttime_tick(struct hrtimer *timer)
{
	struct bce_hosttime *ht = container_of(timer, struct bce_hosttime, timer);
	unsigned long flags;
	u64 now;

	spin_lock_irqsave(&ht->lock, flags);
	ioread32(ht->regs + REG_HOSTTIME_LO);
	/*
	 * The T2 converts audio timestamps into this clock. It has to stop
	 * during suspend and must not be slewed by NTP, which rules out
	 * CLOCK_MONOTONIC.
	 */
	now = ktime_get_raw_ns();
	iowrite32(lower_32_bits(now), ht->regs + REG_HOSTTIME_LO);
	iowrite32(upper_32_bits(now), ht->regs + REG_HOSTTIME_HI);
	if (!ht->stopped)
		bce_hosttime_schedule(ht);
	spin_unlock_irqrestore(&ht->lock, flags);

	return HRTIMER_NORESTART;
}

void bce_hosttime_init(struct bce_hosttime *ht, void __iomem *regs)
{
	ht->regs = regs;
	ht->stopped = true;
	spin_lock_init(&ht->lock);
	ioread32(ht->regs + REG_HOSTTIME_HI);
	hrtimer_setup(&ht->timer, bce_hosttime_tick, CLOCK_MONOTONIC,
		      HRTIMER_MODE_REL);
}

void bce_hosttime_start(struct bce_hosttime *ht, bool initial)
{
	bce_hosttime_mark(ht, initial ? HOSTTIME_MARK_PROBE :
					HOSTTIME_MARK_RESUME);

	scoped_guard(spinlock_irqsave, &ht->lock)
		ht->stopped = false;
	bce_hosttime_schedule(ht);
}

void bce_hosttime_stop(struct bce_hosttime *ht)
{
	scoped_guard(spinlock_irqsave, &ht->lock)
		ht->stopped = true;
	hrtimer_cancel(&ht->timer);
	bce_hosttime_mark(ht, HOSTTIME_MARK_STOP);
}
