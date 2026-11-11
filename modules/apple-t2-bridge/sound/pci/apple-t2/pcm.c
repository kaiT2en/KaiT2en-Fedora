// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 audio, ALSA PCM
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "pcm.h"
#include "audio.h"
#include "timing.h"
#include <linux/dma-mapping.h>
#include <linux/io.h>
#include <linux/ktime.h>
#include <linux/mm.h>

/* integer formats: pick the variant for byte order and signedness */
static u64 t2audio_int_fmtbit(struct t2audio_apple_description *desc, u64 s_be,
			      u64 u_be, u64 s_le, u64 u_le)
{
	bool is_signed = desc->format_flags & T2AUDIO_FORMAT_FLAG_SIGNED;

	if (desc->format_flags & T2AUDIO_FORMAT_FLAG_BIG_ENDIAN)
		return is_signed ? s_be : u_be;
	return is_signed ? s_le : u_le;
}

static u64 t2audio_get_alsa_fmtbit(struct t2audio_apple_description *desc)
{
	if (desc->format_flags & T2AUDIO_FORMAT_FLAG_FLOAT) {
		if (desc->bits_per_channel == 32) {
			if (desc->format_flags & T2AUDIO_FORMAT_FLAG_BIG_ENDIAN)
				return SNDRV_PCM_FMTBIT_FLOAT_BE;
			else
				return SNDRV_PCM_FMTBIT_FLOAT_LE;
		} else if (desc->bits_per_channel == 64) {
			if (desc->format_flags & T2AUDIO_FORMAT_FLAG_BIG_ENDIAN)
				return SNDRV_PCM_FMTBIT_FLOAT64_BE;
			else
				return SNDRV_PCM_FMTBIT_FLOAT64_LE;
		} else {
			pr_err("unsupported bits per channel for float format: %u\n",
			       desc->bits_per_channel);
			return 0;
		}
	}
	if (desc->format_flags & T2AUDIO_FORMAT_FLAG_PACKED) {
		switch (desc->bits_per_channel) {
		case 8:
		case 16:
		case 32:
			break;
		case 24:
			return t2audio_int_fmtbit(desc,
						  SNDRV_PCM_FMTBIT_S24_3BE,
						  SNDRV_PCM_FMTBIT_U24_3BE,
						  SNDRV_PCM_FMTBIT_S24_3LE,
						  SNDRV_PCM_FMTBIT_U24_3LE);
		default:
			pr_err("unsupported bits per channel for packed format: %u\n",
			       desc->bits_per_channel);
			return 0;
		}
	}
	if (desc->format_flags & T2AUDIO_FORMAT_FLAG_ALIGNED_HIGH) {
		switch (desc->bits_per_channel) {
		case 24:
			return t2audio_int_fmtbit(desc, SNDRV_PCM_FMTBIT_S32_BE,
						  SNDRV_PCM_FMTBIT_U32_BE,
						  SNDRV_PCM_FMTBIT_S32_LE,
						  SNDRV_PCM_FMTBIT_U32_LE);
		default:
			pr_err("unsupported bits per channel for high-aligned format: %u\n",
			       desc->bits_per_channel);
			return 0;
		}
	}
	switch (desc->bits_per_channel) {
	case 8:
		if (desc->format_flags & T2AUDIO_FORMAT_FLAG_SIGNED)
			return SNDRV_PCM_FMTBIT_S8;
		else
			return SNDRV_PCM_FMTBIT_U8;
	case 16:
		return t2audio_int_fmtbit(desc, SNDRV_PCM_FMTBIT_S16_BE,
					  SNDRV_PCM_FMTBIT_U16_BE,
					  SNDRV_PCM_FMTBIT_S16_LE,
					  SNDRV_PCM_FMTBIT_U16_LE);
	case 24:
		return t2audio_int_fmtbit(desc, SNDRV_PCM_FMTBIT_S24_BE,
					  SNDRV_PCM_FMTBIT_U24_BE,
					  SNDRV_PCM_FMTBIT_S24_LE,
					  SNDRV_PCM_FMTBIT_U24_LE);
	case 32:
		return t2audio_int_fmtbit(desc, SNDRV_PCM_FMTBIT_S32_BE,
					  SNDRV_PCM_FMTBIT_U32_BE,
					  SNDRV_PCM_FMTBIT_S32_LE,
					  SNDRV_PCM_FMTBIT_U32_LE);
	default:
		pr_err("unsupported bits per channel: %u\n",
		       desc->bits_per_channel);
		return 0;
	}
}

int t2audio_create_hw_info(struct t2audio_apple_description *desc,
			   struct snd_pcm_hardware *alsa_hw, size_t buf_size)
{
	uint rate;

	alsa_hw->info =
		(SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_BLOCK_TRANSFER |
		 SNDRV_PCM_INFO_MMAP_VALID | SNDRV_PCM_INFO_NO_PERIOD_WAKEUP |
		 SNDRV_PCM_INFO_DOUBLE);
	if (desc->format_flags & T2AUDIO_FORMAT_FLAG_NON_MIXABLE)
		pr_warn("unsupported hw flag: NON_MIXABLE\n");
	if (!(desc->format_flags & T2AUDIO_FORMAT_FLAG_NON_INTERLEAVED))
		alsa_hw->info |= SNDRV_PCM_INFO_INTERLEAVED;
	alsa_hw->formats = t2audio_get_alsa_fmtbit(desc);
	if (!alsa_hw->formats)
		return -EINVAL;
	rate = (uint)t2audio_double_to_u64(desc->sample_rate_double);
	alsa_hw->rates = snd_pcm_rate_to_rate_bit(rate);
	alsa_hw->rate_min = rate;
	alsa_hw->rate_max = rate;
	alsa_hw->channels_min = desc->channels_per_frame;
	alsa_hw->channels_max = desc->channels_per_frame;
	alsa_hw->buffer_bytes_max = buf_size;
	alsa_hw->period_bytes_min = desc->bytes_per_packet;
	alsa_hw->period_bytes_max = desc->bytes_per_packet;
	alsa_hw->periods_min = (uint)(buf_size / desc->bytes_per_packet);
	alsa_hw->periods_max = (uint)(buf_size / desc->bytes_per_packet);
	pr_debug(
		"%s: format = %llu, rate = %u/%u. channels = %u, periods = %u, period size = %lu\n",
		__func__, alsa_hw->formats, alsa_hw->rate_min, alsa_hw->rates,
		alsa_hw->channels_min, alsa_hw->periods_min,
		alsa_hw->period_bytes_min);
	return 0;
}

static struct t2audio_stream *
t2audio_pcm_stream(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);

	if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK)
		return &sdev->out_streams[substream->number];
	else
		return &sdev->in_streams[substream->number];
}

static struct t2audio_dma_buf *
t2audio_pcm_dma_buf(struct t2audio_stream *stream)
{
	if (!stream->buffer_cnt || !stream->buffers)
		return NULL;

	return &stream->buffers[0];
}

enum t2audio_remote_io_state {
	T2AUDIO_REMOTE_IO_UNKNOWN = -1,
	T2AUDIO_REMOTE_IO_STOPPED,
	T2AUDIO_REMOTE_IO_STARTED,
};

static void t2audio_dma_memset(struct t2audio_dma_buf *buf, size_t offset,
			       int value, size_t size)
{
	if (!buf || offset >= buf->size)
		return;

	size = min(size, buf->size - offset);
	switch (buf->type) {
	case T2AUDIO_DMA_BUF_IOMEM:
		memset_io((u8 __iomem *)buf->ptr + offset, value, size);
		break;
	case T2AUDIO_DMA_BUF_COHERENT:
		memset((u8 *)buf->ptr + offset, value, size);
		break;
	}
}

void t2audio_pcm_quiesce_stream(struct t2audio_stream *stream)
{
	/* pairs with smp_load_acquire() in the pointer and timestamp paths */
	smp_store_release(&stream->started, 0);
}

void t2audio_pcm_reset_timing(struct t2audio_stream *stream)
{
	stream->waiting_for_first_ts = true;
	stream->accept_after_ns = 0;
	stream->last_received_ns = 0;
	stream->anchor_ns = 0;
	stream->sample_frames = 0;
	stream->sample_base = 0;
	stream->reported_frames = 0;
	stream->sample_period = 0;
	stream->timeline_fault = false;
	memset(stream->write_history, 0, sizeof(stream->write_history));
	stream->write_history_idx = 0;
	stream->reset_zeroing = false;
}

static int t2audio_pcm_open(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);

	pr_debug("pcm open dev=%s direction=%s substream=%u\n", sdev->uid,
		 substream->stream == SNDRV_PCM_STREAM_PLAYBACK ? "playback" :
								  "capture",
		 substream->number);
	if (!t2audio_pcm_stream(substream)->alsa_hw_desc)
		return -ENODEV;
	substream->runtime->hw = *t2audio_pcm_stream(substream)->alsa_hw_desc;

	return 0;
}

static int t2audio_pcm_close(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);

	pr_debug("pcm close dev=%s direction=%s substream=%u\n", sdev->uid,
		 substream->stream == SNDRV_PCM_STREAM_PLAYBACK ? "playback" :
								  "capture",
		 substream->number);
	flush_work(&stream->io_work);
	return 0;
}

static int t2audio_pcm_prepare(struct snd_pcm_substream *substream)
{
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);

	/* A previous asynchronous STOP must finish before the ring is cleared. */
	flush_work(&stream->io_work);
	t2audio_pcm_reset_timing(stream);

	/* Neither direction may expose samples left over from the previous run. */
	{
		struct t2audio_dma_buf *bridge = t2audio_pcm_dma_buf(stream);

		if (bridge)
			t2audio_dma_memset(bridge, 0, 0, bridge->size);
	}

	return 0;
}

static int t2audio_pcm_hw_params(struct snd_pcm_substream *substream,
				 struct snd_pcm_hw_params *hw_params)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *astream = t2audio_pcm_stream(substream);

	pr_debug("pcm hw_params dev=%s direction=%s substream=%u\n", sdev->uid,
		 substream->stream == SNDRV_PCM_STREAM_PLAYBACK ? "playback" :
								  "capture",
		 substream->number);

	if (!astream->buffer_cnt || !astream->buffers)
		return -EINVAL;

	if (params_buffer_bytes(hw_params) != astream->buffers[0].size)
		return -EINVAL;

	/* the ALSA buffer is the ring in BAR 0 itself */
	substream->runtime->dma_area = astream->buffers[0].ptr;
	substream->runtime->dma_addr = astream->buffers[0].dma_addr;
	substream->runtime->dma_bytes = astream->buffers[0].size;
	return 0;
}

static int t2audio_pcm_hw_free(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);

	pr_debug("pcm hw_free dev=%s direction=%s substream=%u\n", sdev->uid,
		 substream->stream == SNDRV_PCM_STREAM_PLAYBACK ? "playback" :
								  "capture",
		 substream->number);
	flush_work(&stream->io_work);
	return 0;
}

static int t2audio_pcm_start_sync(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);
	ktime_t time_start, time_end;
	int status;

	time_start = ktime_get();
	/* pairs with smp_load_acquire() in the pointer and timestamp paths */
	smp_store_release(&stream->started, 0);

	if (!substream->runtime->buffer_size || !t2audio_pcm_dma_buf(stream))
		return -EINVAL;

	/* timestamps of a previous run do not apply to this one */
	t2audio_pcm_reset_timing(stream);
	/*
	 * Zero timestamps anchored after this point belong to this run,
	 * including those sent during START_IO.
	 */
	stream->accept_after_ns = ktime_get_raw_ns();
	/*
	 * The T2 sends the first zero timestamp only after the device has
	 * already been reading for some 30 ms. Until it arrives, playback counts
	 * from before START_IO. The device cannot read earlier, so this never
	 * reports less than it consumed.
	 */
	stream->anchor_ns = stream->accept_after_ns;

	/* Publish userspace's prefill before starting the remote consumer. */
	wmb();

	/*
	 * START_IO can emit the reset timestamp before its response.  Publish the
	 * stream first so deferred command handling cannot discard that anchor.
	 */
	smp_store_release(&stream->started, 1);
	status = t2audio_cmd_start_io(sdev->a, sdev->dev_id);
	if (status) {
		/* pairs with smp_load_acquire() in the pointer and timestamp paths */
		smp_store_release(&stream->started, 0);
		return status;
	}

	time_end = ktime_get();
	pr_debug("start_io %s %lld us\n", sdev->uid,
		 ktime_to_us(ktime_sub(time_end, time_start)));
	return 0;
}

static int t2audio_pcm_stop_sync(struct snd_pcm_substream *substream)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);
	int status;

	t2audio_pcm_quiesce_stream(stream);
	status = t2audio_cmd_stop_io(sdev->a, sdev->dev_id);
	stream->waiting_for_first_ts = true;
	stream->accept_after_ns = 0;
	return status;
}

static void t2audio_pcm_io_work(struct work_struct *work)
{
	struct t2audio_stream *stream =
		container_of(work, struct t2audio_stream, io_work);
	struct t2audio_subdevice *sdev = stream->sdev;
	unsigned long flags;

	for (;;) {
		unsigned int generation;
		bool requested;
		int target_state;
		int status;

		spin_lock_irqsave(&stream->io_lock, flags);
		generation = stream->io_generation;
		requested = stream->io_requested;
		target_state = requested ? T2AUDIO_REMOTE_IO_STARTED :
					   T2AUDIO_REMOTE_IO_STOPPED;
		if (stream->remote_io_state == target_state) {
			spin_unlock_irqrestore(&stream->io_lock, flags);
			return;
		}
		spin_unlock_irqrestore(&stream->io_lock, flags);

		status = requested ? t2audio_pcm_start_sync(stream->substream) :
				     t2audio_pcm_stop_sync(stream->substream);

		spin_lock_irqsave(&stream->io_lock, flags);
		stream->remote_io_state = status ? T2AUDIO_REMOTE_IO_UNKNOWN :
						   target_state;
		if (status || generation == stream->io_generation) {
			spin_unlock_irqrestore(&stream->io_lock, flags);
			if (status)
				dev_err(sdev->a->dev,
					"%s_IO failed for %s: %d\n",
					requested ? "START" : "STOP", sdev->uid,
					status);
			return;
		}
		spin_unlock_irqrestore(&stream->io_lock, flags);
	}
}

static void t2audio_pcm_request_io(struct t2audio_stream *stream, bool start)
{
	unsigned long flags;

	if (!start)
		t2audio_pcm_quiesce_stream(stream);

	spin_lock_irqsave(&stream->io_lock, flags);
	stream->io_requested = start;
	stream->io_generation++;
	spin_unlock_irqrestore(&stream->io_lock, flags);
	queue_work(stream->sdev->a->pcm_io_wq, &stream->io_work);
}

static int t2audio_pcm_trigger(struct snd_pcm_substream *substream, int cmd)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);

	/* bridgeOS exposes one ALSA substream per remote stream. */
	if (substream->number != 0)
		return 0;
	switch (cmd) {
	case SNDRV_PCM_TRIGGER_START:
		pr_debug("TRIGGER START %s\n", sdev->uid);
		if (!substream->runtime->buffer_size ||
		    !t2audio_pcm_dma_buf(stream))
			return -EINVAL;
		t2audio_pcm_request_io(stream, true);
		break;
	case SNDRV_PCM_TRIGGER_STOP:
		pr_debug("TRIGGER STOP %s\n", sdev->uid);
		t2audio_pcm_request_io(stream, false);
		break;
	default:
		return -EINVAL;
	}
	return 0;
}

static void t2audio_pcm_init_stream_work(struct t2audio_subdevice *sdev,
					 struct t2audio_stream *stream,
					 struct snd_pcm_substream *substream)
{
	stream->sdev = sdev;
	stream->substream = substream;
	spin_lock_init(&stream->io_lock);
	INIT_WORK(&stream->io_work, t2audio_pcm_io_work);
	stream->remote_io_state = T2AUDIO_REMOTE_IO_STOPPED;
}

static snd_pcm_uframes_t
t2audio_pcm_pointer(struct snd_pcm_substream *substream)
{
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);
	u64 latency = substream->stream == SNDRV_PCM_STREAM_CAPTURE ?
			      stream->latency :
			      0;
	u64 elapsed_frames, period;
	s64 elapsed_ns;

	/* pairs with smp_store_release() on start and stop */
	if (!smp_load_acquire(&stream->started))
		return 0;

	if (stream->timeline_fault)
		return SNDRV_PCM_POS_XRUN;
	if (stream->waiting_for_first_ts &&
	    substream->stream == SNDRV_PCM_STREAM_CAPTURE)
		return 0;

	/*
	 * The T2 sends one zero timestamp (sampleTime, hostTime) per ring
	 * pass. In between, the position advances at the stream rate from
	 * hostTime but stays below the next zero timestamp.
	 */
	elapsed_ns = (s64)(ktime_get_raw_ns() - stream->anchor_ns);
	elapsed_frames = elapsed_ns > 0 ?
				 mul_u64_u32_div(elapsed_ns,
						 substream->runtime->rate,
						 NSEC_PER_SEC) :
				 0;
	period = stream->sample_period ? stream->sample_period :
					 substream->runtime->buffer_size;
	elapsed_frames = min(elapsed_frames, period - 1);
	stream->reported_frames =
		max(stream->reported_frames,
		    t2audio_position_frames(stream->sample_frames -
						    stream->sample_base +
						    elapsed_frames,
					    latency));

	substream->runtime->delay = substream->stream ==
						    SNDRV_PCM_STREAM_PLAYBACK ?
					    stream->latency :
					    0;
	return stream->reported_frames % substream->runtime->buffer_size;
}

static int t2audio_pcm_mmap(struct snd_pcm_substream *substream,
			    struct vm_area_struct *area)
{
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);
	struct t2audio_dma_buf *buf;

	if (!stream->buffer_cnt || !stream->buffers)
		return -EINVAL;

	buf = &stream->buffers[0];
	switch (buf->type) {
	case T2AUDIO_DMA_BUF_IOMEM:
		if (sdev->a->bs_wc)
			area->vm_page_prot =
				pgprot_writecombine(area->vm_page_prot);
		else
			area->vm_page_prot =
				pgprot_noncached(area->vm_page_prot);
		return vm_iomap_memory(area, buf->dma_addr, buf->size);
	case T2AUDIO_DMA_BUF_COHERENT:
		return dma_mmap_coherent(sdev->a->dev, area, buf->ptr,
					 buf->dma_addr, buf->size);
	default:
		return -EINVAL;
	}
}

static const struct snd_pcm_ops t2audio_pcm_ops = {
	.open = t2audio_pcm_open,
	.close = t2audio_pcm_close,
	.ioctl = snd_pcm_lib_ioctl,
	.hw_params = t2audio_pcm_hw_params,
	.hw_free = t2audio_pcm_hw_free,
	.prepare = t2audio_pcm_prepare,
	.trigger = t2audio_pcm_trigger,
	.pointer = t2audio_pcm_pointer,
	.mmap = t2audio_pcm_mmap
};

int t2audio_create_pcm(struct t2audio_subdevice *sdev)
{
	struct snd_pcm *pcm;
	struct t2audio_alsa_pcm_id_mapping *id_mapping;
	int err;

	if (!sdev->is_pcm ||
	    (sdev->in_stream_cnt == 0 && sdev->out_stream_cnt == 0)) {
		return -EINVAL;
	}

	for (id_mapping = t2audio_alsa_id_mappings; id_mapping->name;
	     id_mapping++) {
		if (!strcmp(sdev->uid, id_mapping->name)) {
			sdev->alsa_id = id_mapping->alsa_id;
			break;
		}
	}
	if (!id_mapping->name)
		sdev->alsa_id = sdev->a->next_alsa_id++;
	err = snd_pcm_new(sdev->a->card, sdev->uid, sdev->alsa_id,
			  (int)sdev->out_stream_cnt, (int)sdev->in_stream_cnt,
			  &pcm);
	if (err < 0)
		return err;
	pcm->private_data = sdev;
	pcm->nonatomic = 1;
	sdev->pcm = pcm;
	strscpy(pcm->name, sdev->uid);
	snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_PLAYBACK, &t2audio_pcm_ops);
	snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_CAPTURE, &t2audio_pcm_ops);
	if (sdev->out_stream_cnt)
		t2audio_pcm_init_stream_work(
			sdev, &sdev->out_streams[0],
			pcm->streams[SNDRV_PCM_STREAM_PLAYBACK].substream);
	if (sdev->in_stream_cnt)
		t2audio_pcm_init_stream_work(
			sdev, &sdev->in_streams[0],
			pcm->streams[SNDRV_PCM_STREAM_CAPTURE].substream);
	return 0;
}

int t2audio_pcm_quiesce_subdevice(struct t2audio_subdevice *sdev)
{
	struct t2audio_stream *streams[2];
	unsigned long flags;
	bool need_stop = false;
	size_t count = 0;
	size_t i;
	int status = 0;

	if (sdev->out_stream_cnt && sdev->out_streams[0].substream)
		streams[count++] = &sdev->out_streams[0];
	if (sdev->in_stream_cnt && sdev->in_streams[0].substream)
		streams[count++] = &sdev->in_streams[0];

	for (i = 0; i < count; i++)
		t2audio_pcm_request_io(streams[i], false);
	for (i = 0; i < count; i++)
		flush_work(&streams[i]->io_work);

	for (i = 0; i < count; i++) {
		spin_lock_irqsave(&streams[i]->io_lock, flags);
		if (streams[i]->remote_io_state != T2AUDIO_REMOTE_IO_STOPPED)
			need_stop = true;
		spin_unlock_irqrestore(&streams[i]->io_lock, flags);
	}

	if (need_stop)
		status = t2audio_cmd_stop_io(sdev->a, sdev->dev_id);

	for (i = 0; i < count; i++) {
		spin_lock_irqsave(&streams[i]->io_lock, flags);
		streams[i]->remote_io_state =
			status ? T2AUDIO_REMOTE_IO_UNKNOWN :
				 T2AUDIO_REMOTE_IO_STOPPED;
		spin_unlock_irqrestore(&streams[i]->io_lock, flags);
	}
	return status;
}

void t2audio_pcm_cleanup_subdevice(struct t2audio_subdevice *sdev)
{
	if (sdev->out_stream_cnt && sdev->out_streams[0].substream)
		cancel_work_sync(&sdev->out_streams[0].io_work);
	if (sdev->in_stream_cnt && sdev->in_streams[0].substream)
		cancel_work_sync(&sdev->in_streams[0].io_work);
}

/*
 * When the application did not write across two timestamps, zero the output
 * ring once so the T2 plays silence instead of looping stale samples.
 * Zeroing is armed again once the application writes past the history.
 */
static void
t2audio_pcm_check_stalled_writer(struct snd_pcm_substream *substream,
				 struct t2audio_stream *stream)
{
	snd_pcm_uframes_t written =
		READ_ONCE(substream->runtime->control->appl_ptr);
	snd_pcm_uframes_t *history = stream->write_history;
	struct t2audio_dma_buf *buf = t2audio_pcm_dma_buf(stream);

	history[stream->write_history_idx] = written;
	if (!buf || !history[0] || history[0] != history[1] ||
	    stream->reset_zeroing) {
		if (written > history[0] || written > history[1])
			stream->reset_zeroing = false;
		stream->write_history_idx = !stream->write_history_idx;
		return;
	}

	pr_debug("zeroing stalled output ring dev=%s written=%lu\n",
		 stream->sdev->uid, written);
	t2audio_dma_memset(buf, 0, 0, buf->size);
	stream->reset_zeroing = true;
	stream->write_history_idx = 0;
}

static void t2audio_handle_stream_timestamp(struct snd_pcm_substream *substream,
					    u64 received_ns,
					    u64 device_timestamp,
					    u64 update_seed, u64 sample_frames)
{
	unsigned long flags;
	struct t2audio_stream *stream = t2audio_pcm_stream(substream);
	struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
	u64 expected, window_ns;
	bool plausible;

	snd_pcm_stream_lock_irqsave(substream, flags);
	/* pairs with smp_store_release() on start and stop */
	if (!smp_load_acquire(&stream->started) || !substream->runtime)
		goto out;

	/*
	 * hostTime normally trails the local receive time by 1-3 ms. Until
	 * bridgeOS has recalibrated after a wake it can be off by far more, so
	 * an anchor outside one ring pass of the receive time uses the receive
	 * time instead.
	 */
	window_ns = mul_u64_u32_div(substream->runtime->buffer_size,
				    NSEC_PER_SEC, substream->runtime->rate);
	plausible = device_timestamp <= received_ns &&
		    received_ns - device_timestamp <= window_ns;

	/*
	 * A restart after a pause longer than one ring pass can deliver the
	 * previous run's next zero timestamp during START_IO. No zero timestamp
	 * of this run can carry a hostTime from before START_IO was sent.
	 */
	if (device_timestamp < stream->accept_after_ns) {
		pr_debug(
			"stale timestamp dev=%s sample=%llu t2=%llu host=%llu\n",
			sdev->uid, sample_frames, device_timestamp,
			received_ns);
		goto out;
	}

	/*
	 * Deferred work can run out of order. The BCE completion order is the
	 * order the T2 sent the notifications in, so never apply a
	 * notification received before the one already applied.
	 */
	if (!stream->waiting_for_first_ts &&
	    received_ns <= stream->last_received_ns)
		goto out;

	if (!stream->waiting_for_first_ts) {
		/*
		 * The T2 sends seed=1 with sampleTime=0 when it restarts the
		 * timeline. ALSA cannot move a running ring origin, so report an
		 * XRUN instead.
		 */
		if ((update_seed && stream->sample_frames) ||
		    sample_frames < stream->sample_frames) {
			stream->timeline_fault = true;
			pr_warn_ratelimited(
				"timeline reset dev=%s sample=%llu previous=%llu seed=%llu\n",
				sdev->uid, sample_frames, stream->sample_frames,
				update_seed);
			snd_pcm_period_elapsed_under_stream_lock(substream);
			goto out;
		}
		/*
		 * sampleTime advances by one timestamp period per notification.
		 * The period is not queried, the first step stands in for it.
		 */
		if (!stream->sample_period)
			stream->sample_period =
				sample_frames - stream->sample_frames;
		expected = stream->sample_frames + stream->sample_period;
		if (sample_frames != expected)
			pr_debug_ratelimited(
				"sampleTime step dev=%s sample=%llu expected=%llu\n",
				sdev->uid, sample_frames, expected);
	}

	if (!plausible)
		pr_warn_ratelimited(
			"implausible hostTime dev=%s t2=%llu host=%llu\n",
			sdev->uid, device_timestamp, received_ns);
	stream->anchor_ns = plausible ? device_timestamp : received_ns;
	/*
	 * The ring origin is the ring pass of the first zero timestamp, so the
	 * position counted before it carries on without wrapping back.
	 */
	if (stream->waiting_for_first_ts)
		stream->sample_base =
			sample_frames -
			sample_frames % substream->runtime->buffer_size;
	stream->sample_frames = sample_frames;
	stream->last_received_ns = received_ns;
	stream->waiting_for_first_ts = false;
	pr_debug(
		"anchor dev=%s direction=%d sample=%llu t2=%llu host=%llu seed=%llu\n",
		sdev->uid, substream->stream, sample_frames, device_timestamp,
		received_ns, update_seed);

	if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK)
		t2audio_pcm_check_stalled_writer(substream, stream);

	if (!substream->runtime->no_period_wakeup)
		snd_pcm_period_elapsed_under_stream_lock(substream);
out:
	snd_pcm_stream_unlock_irqrestore(substream, flags);
}

void t2audio_handle_timestamp(struct t2audio_subdevice *sdev, u64 received_ns,
			      u64 device_timestamp, u64 update_seed,
			      u64 sample_frames)
{
	struct snd_pcm_substream *substream;

	if (!sdev->pcm)
		return;
	substream = sdev->pcm->streams[SNDRV_PCM_STREAM_PLAYBACK].substream;
	if (substream)
		t2audio_handle_stream_timestamp(substream, received_ns,
						device_timestamp, update_seed,
						sample_frames);
	substream = sdev->pcm->streams[SNDRV_PCM_STREAM_CAPTURE].substream;
	if (substream)
		t2audio_handle_stream_timestamp(substream, received_ns,
						device_timestamp, update_seed,
						sample_frames);
}
