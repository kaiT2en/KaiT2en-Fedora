/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Client interface of the Apple T2 Bridge Controller.
 *
 * The bridge controller carries all host traffic to the T2 coprocessor over
 * DMA queues of its Buffer Copy Engine (BCE). A client creates completion
 * queues (CQ) and named submission queues (SQ). The name selects the T2
 * service the SQ is connected to. Submissions hand single buffers or segment
 * lists to the T2, completions report the transferred size per submission.
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#ifndef _LINUX_APPLE_T2_BRIDGE_H
#define _LINUX_APPLE_T2_BRIDGE_H

#include <linux/dma-direction.h>
#include <linux/gfp_types.h>
#include <linux/types.h>

struct device;
struct scatterlist;

struct apple_bce;
struct apple_bce_cq;
struct apple_bce_sq;
struct apple_bce_sglist;

/**
 * enum apple_bce_status - status of a completed submission
 * @APPLE_BCE_STATUS_SUCCESS: transfer done
 * @APPLE_BCE_STATUS_ERROR: transfer failed
 * @APPLE_BCE_STATUS_ABORTED: the SQ was flushed before the T2 took the buffer
 * @APPLE_BCE_STATUS_NO_SPACE: the T2 side had no room for the data
 * @APPLE_BCE_STATUS_OVERRUN: the data did not fit into the host buffer
 */
enum apple_bce_status {
	APPLE_BCE_STATUS_SUCCESS = 0,
	APPLE_BCE_STATUS_ERROR = 1,
	APPLE_BCE_STATUS_ABORTED = 2,
	APPLE_BCE_STATUS_NO_SPACE = 3,
	APPLE_BCE_STATUS_OVERRUN = 4,
};

/**
 * struct apple_bce_completion - completion of one submission
 * @status: &enum apple_bce_status
 * @data_size: bytes the T2 transferred
 * @result: service specific result word
 */
struct apple_bce_completion {
	u32 status;
	u64 data_size;
	u64 result;
};

/*
 * Called from the threaded completion interrupt whenever new completions are
 * pending on @sq. Fetch them with apple_bce_sq_next_completion() and release
 * every slot with apple_bce_sq_complete().
 */
typedef void (*apple_bce_sq_complete_t)(struct apple_bce_sq *sq);

struct apple_bce *apple_bce_get(struct device *consumer);
struct device *apple_bce_dma_device(struct apple_bce *bce);
void apple_bce_sync_completions(struct apple_bce *bce);

struct apple_bce_cq *apple_bce_cq_create(struct apple_bce *bce, u32 count);
void apple_bce_cq_destroy(struct apple_bce_cq *cq);

struct apple_bce_sq *apple_bce_sq_create(struct apple_bce *bce,
					 struct apple_bce_cq *cq,
					 const char *name, u32 count,
					 enum dma_data_direction dir,
					 apple_bce_sq_complete_t complete,
					 void *priv);
void apple_bce_sq_destroy(struct apple_bce_sq *sq);
int apple_bce_sq_flush(struct apple_bce_sq *sq);
void *apple_bce_sq_priv(struct apple_bce_sq *sq);

int apple_bce_sq_reserve(struct apple_bce_sq *sq, unsigned long *timeout);
void apple_bce_sq_unreserve(struct apple_bce_sq *sq);
void apple_bce_sq_add(struct apple_bce_sq *sq, dma_addr_t addr, size_t size);
int apple_bce_sq_add_sglist(struct apple_bce_sq *sq,
			    const struct apple_bce_sglist *list,
			    size_t offset, size_t size, size_t *added);
void apple_bce_sq_submit(struct apple_bce_sq *sq);

struct apple_bce_completion *apple_bce_sq_next_completion(struct apple_bce_sq *sq);
void apple_bce_sq_complete(struct apple_bce_sq *sq);

u32 apple_bce_sq_head(struct apple_bce_sq *sq);
u32 apple_bce_sq_tail(struct apple_bce_sq *sq);
u32 apple_bce_sq_size(struct apple_bce_sq *sq);
bool apple_bce_sq_idle(struct apple_bce_sq *sq);

struct apple_bce_sglist *apple_bce_sglist_create(struct apple_bce *bce,
						 struct scatterlist *sgl,
						 unsigned int nents, gfp_t gfp);
void apple_bce_sglist_destroy(struct apple_bce *bce,
			      struct apple_bce_sglist *list);

#endif /* _LINUX_APPLE_T2_BRIDGE_H */
