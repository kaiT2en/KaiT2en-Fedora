/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Apple T2 audio
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#ifndef _T2AUDIO_H
#define _T2AUDIO_H

#include <linux/apple-t2-bridge.h>
#include <linux/spinlock.h>
#include <linux/types.h>
#include <linux/workqueue.h>
#include <sound/pcm.h>

#include "description.h"
#include "protocol_bce.h"

/* GPR word 0: version, word 1 and the BufferStruct: this signature */
#define T2AUDIO_SIG 0x19870423

/*
 * The host writes this marker to the GPR before sleep. It reads back on
 * resume only if the T2 kept its state.
 */
#define T2AUDIO_GPR_SLEEP_MARKER 0x10
#define T2AUDIO_SLEEP_MARKER 0x19770525

#define T2AUDIO_DEVICE_MAX_UID_LEN 128
#define T2AUDIO_DEVICE_MAX_INPUT_STREAMS 1
#define T2AUDIO_DEVICE_MAX_OUTPUT_STREAMS 1
#define T2AUDIO_DEVICE_MAX_BUFFER_COUNT 1

#define T2AUDIO_BUFFER_ID_NONE 0xffu

struct snd_card;
struct snd_pcm;
struct snd_pcm_hardware;
struct snd_jack;
struct dentry;

struct __packed __aligned(4) t2audio_buffer_struct_buffer
{
	size_t address;
	size_t size;
	size_t pad[4];
};

struct t2audio_buffer_struct_stream {
	u8 num_buffers;
	struct t2audio_buffer_struct_buffer buffers[100];
	char filler[32];
};

struct t2audio_buffer_struct_device {
	char name[128];
	u8 num_input_streams;
	u8 num_output_streams;
	struct t2audio_buffer_struct_stream input_streams[5];
	struct t2audio_buffer_struct_stream output_streams[5];
	char filler[128];
};

struct t2audio_buffer_struct {
	u32 version;
	u32 signature;
	u32 flags;
	u8 num_devices;
	struct t2audio_buffer_struct_device devices[20];
};

struct t2audio_device;

struct t2audio_deferred_msg {
	struct work_struct ws;
	struct t2audio_device *a;
	struct t2audio_msg msg;
};

enum t2audio_dma_buf_type {
	T2AUDIO_DMA_BUF_IOMEM,
	T2AUDIO_DMA_BUF_COHERENT,
};

struct t2audio_dma_buf {
	dma_addr_t dma_addr;
	void *ptr;
	size_t size;
	enum t2audio_dma_buf_type type;
};

struct t2audio_stream {
	t2audio_object_id_t id;
	size_t buffer_cnt;
	struct t2audio_dma_buf *buffers;

	struct t2audio_apple_description desc;
	struct snd_pcm_hardware *alsa_hw_desc;
	u32 latency;

	bool waiting_for_first_ts;

	u64 accept_after_ns;
	u64 last_received_ns;
	u64 anchor_ns;
	u64 sample_frames;
	u64 sample_base;
	u64 reported_frames;
	u64 sample_period;
	bool timeline_fault;
	snd_pcm_uframes_t write_history[2];
	u8 write_history_idx;
	bool reset_zeroing;
	int started;

	struct t2audio_subdevice *sdev;
	struct snd_pcm_substream *substream;
	struct work_struct io_work;
	spinlock_t io_lock; /* protects the I/O request state */
	u32 io_generation;
	bool io_requested;
	int remote_io_state;
};

struct t2audio_subdevice {
	struct t2audio_device *a;
	struct list_head list;
	t2audio_device_id_t dev_id;
	u32 in_latency, out_latency;
	u32 in_safety_offset, out_safety_offset;
	u8 buf_id;
	int alsa_id;
	char uid[T2AUDIO_DEVICE_MAX_UID_LEN + 1];
	size_t in_stream_cnt;
	struct t2audio_stream in_streams[T2AUDIO_DEVICE_MAX_INPUT_STREAMS];
	size_t out_stream_cnt;
	struct t2audio_stream out_streams[T2AUDIO_DEVICE_MAX_OUTPUT_STREAMS];
	bool is_pcm;
	struct snd_pcm *pcm;
	struct snd_jack *jack;
};

struct t2audio_alsa_pcm_id_mapping {
	const char *name;
	int alsa_id;
};

struct t2audio_device {
	struct pci_dev *pci;
	struct device *dev;
	void __iomem *reg_mem_bs;
	dma_addr_t reg_mem_bs_dma;
	void __iomem *reg_mem_cfg;
	bool bs_wc;

	u32 __iomem *reg_mem_gpr;

	struct t2audio_buffer_struct *bs;
	struct dentry *debugfs_dir;

	struct apple_bce *bce;
	struct t2audio_bce bcem;

	struct snd_card *card;

	struct list_head subdevice_list;
	int next_alsa_id;

	struct completion remote_alive;
	struct work_struct resume_work;
	struct workqueue_struct *pcm_io_wq;
	struct workqueue_struct *msg_wq;
	bool resume_deferred;
	bool pm_quiesced;
	bool pm_suspended;
};

void t2audio_handle_notification(struct t2audio_device *a,
				 struct t2audio_msg *msg);
void t2audio_handle_prop_change_work(struct work_struct *ws);
void t2audio_handle_cmd_timestamp(struct t2audio_device *a,
				  struct t2audio_msg *msg);
void t2audio_handle_command(struct t2audio_device *a, struct t2audio_msg *msg);

extern struct t2audio_alsa_pcm_id_mapping t2audio_alsa_id_mappings[];

#endif /* _T2AUDIO_H */
