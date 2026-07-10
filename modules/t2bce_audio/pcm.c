#include "pcm.h"
#include "audio.h"
#include <linux/dma-mapping.h>
#include <linux/io.h>
#include <linux/ktime.h>

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
            pr_err("t2bce_audio: unsupported bits per channel for float format: %u\n", desc->bits_per_channel);
            return 0;
        }
    }
#define DEFINE_BPC_OPTION(val, b) \
    case val: \
        if (desc->format_flags & T2AUDIO_FORMAT_FLAG_BIG_ENDIAN) { \
            if (desc->format_flags & T2AUDIO_FORMAT_FLAG_SIGNED) \
                return SNDRV_PCM_FMTBIT_S ## b ## BE; \
            else \
                return SNDRV_PCM_FMTBIT_U ## b ## BE; \
        } else { \
            if (desc->format_flags & T2AUDIO_FORMAT_FLAG_SIGNED) \
                return SNDRV_PCM_FMTBIT_S ## b ## LE; \
            else \
                return SNDRV_PCM_FMTBIT_U ## b ## LE; \
        }
    if (desc->format_flags & T2AUDIO_FORMAT_FLAG_PACKED) {
        switch (desc->bits_per_channel) {
            case 8:
            case 16:
            case 32:
                break;
            DEFINE_BPC_OPTION(24, 24_3)
            default:
                pr_err("t2bce_audio: unsupported bits per channel for packed format: %u\n", desc->bits_per_channel);
                return 0;
        }
    }
    if (desc->format_flags & T2AUDIO_FORMAT_FLAG_ALIGNED_HIGH) {
        switch (desc->bits_per_channel) {
            DEFINE_BPC_OPTION(24, 32_)
            default:
                pr_err("t2bce_audio: unsupported bits per channel for high-aligned format: %u\n", desc->bits_per_channel);
                return 0;
        }
    }
    switch (desc->bits_per_channel) {
        case 8:
            if (desc->format_flags & T2AUDIO_FORMAT_FLAG_SIGNED)
                return SNDRV_PCM_FMTBIT_S8;
            else
                return SNDRV_PCM_FMTBIT_U8;
        DEFINE_BPC_OPTION(16, 16_)
        DEFINE_BPC_OPTION(24, 24_)
        DEFINE_BPC_OPTION(32, 32_)
        default:
            pr_err("t2bce_audio: unsupported bits per channel: %u\n", desc->bits_per_channel);
            return 0;
    }
}
int t2audio_create_hw_info(struct t2audio_apple_description *desc, struct snd_pcm_hardware *alsa_hw,
        size_t buf_size)
{
    uint rate;
    alsa_hw->info = (SNDRV_PCM_INFO_MMAP |
                     SNDRV_PCM_INFO_BLOCK_TRANSFER |
                     SNDRV_PCM_INFO_MMAP_VALID |
                     SNDRV_PCM_INFO_NO_PERIOD_WAKEUP |
                     SNDRV_PCM_INFO_DOUBLE);
    if (desc->format_flags & T2AUDIO_FORMAT_FLAG_NON_MIXABLE)
        pr_warn("t2bce_audio: unsupported hw flag: NON_MIXABLE\n");
    if (!(desc->format_flags & T2AUDIO_FORMAT_FLAG_NON_INTERLEAVED))
        alsa_hw->info |= SNDRV_PCM_INFO_INTERLEAVED;
    alsa_hw->formats = t2audio_get_alsa_fmtbit(desc);
    if (!alsa_hw->formats)
        return -EINVAL;
    rate = (uint) t2audio_double_to_u64(desc->sample_rate_double);
    alsa_hw->rates = snd_pcm_rate_to_rate_bit(rate);
    alsa_hw->rate_min = rate;
    alsa_hw->rate_max = rate;
    alsa_hw->channels_min = desc->channels_per_frame;
    alsa_hw->channels_max = desc->channels_per_frame;
    alsa_hw->buffer_bytes_max = buf_size;
    alsa_hw->period_bytes_min = desc->bytes_per_packet;
    alsa_hw->period_bytes_max = desc->bytes_per_packet;
    alsa_hw->periods_min = (uint) (buf_size / desc->bytes_per_packet);
    alsa_hw->periods_max = (uint) (buf_size / desc->bytes_per_packet);
    pr_debug("t2audio_create_hw_info: format = %llu, rate = %u/%u. channels = %u, periods = %u, period size = %lu\n",
            alsa_hw->formats, alsa_hw->rate_min, alsa_hw->rates, alsa_hw->channels_min, alsa_hw->periods_min,
            alsa_hw->period_bytes_min);
    return 0;
}

static struct t2audio_stream *t2audio_pcm_stream(struct snd_pcm_substream *substream)
{
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
    if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK)
        return &sdev->out_streams[substream->number];
    else
        return &sdev->in_streams[substream->number];
}

static struct t2audio_dma_buf *t2audio_pcm_dma_buf(struct t2audio_stream *stream)
{
    if (!stream->buffer_cnt || !stream->buffers)
        return NULL;

    return &stream->buffers[0];
}

static void t2audio_dma_memset(struct t2audio_dma_buf *buf, size_t offset, int value, size_t size)
{
    if (!buf || offset >= buf->size)
        return;

    size = min(size, buf->size - offset);
    switch (buf->type) {
        case T2AUDIO_DMA_BUF_IOMEM:
            memset_io((u8 __iomem *) buf->ptr + offset, value, size);
            break;
        case T2AUDIO_DMA_BUF_COHERENT:
            memset((u8 *) buf->ptr + offset, value, size);
            break;
    }
}

static void t2audio_dma_copy_from(struct t2audio_dma_buf *buf, void *dst, size_t offset, size_t size)
{
    if (!buf || offset >= buf->size)
        return;

    size = min(size, buf->size - offset);
    switch (buf->type) {
        case T2AUDIO_DMA_BUF_IOMEM:
            memcpy_fromio(dst, (u8 __iomem *) buf->ptr + offset, size);
            break;
        case T2AUDIO_DMA_BUF_COHERENT:
            memcpy(dst, (u8 *) buf->ptr + offset, size);
            break;
    }
}

static void t2audio_dma_copy_to(struct t2audio_dma_buf *buf, size_t offset, const void *src, size_t size)
{
    if (!buf || offset >= buf->size)
        return;

    size = min(size, buf->size - offset);
    switch (buf->type) {
        case T2AUDIO_DMA_BUF_IOMEM:
            memcpy_toio((u8 __iomem *) buf->ptr + offset, src, size);
            break;
        case T2AUDIO_DMA_BUF_COHERENT:
            memcpy((u8 *) buf->ptr + offset, src, size);
            break;
    }
}

static void t2audio_pcm_zero_frames(struct snd_pcm_substream *substream,
        snd_pcm_uframes_t first, snd_pcm_uframes_t frames)
{
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    struct t2audio_dma_buf *buf = t2audio_pcm_dma_buf(stream);
    struct snd_pcm_runtime *runtime = substream->runtime;
    size_t offset;
    size_t size;

    if (!buf || !runtime || !runtime->buffer_size || !frames)
        return;

    first %= runtime->buffer_size;
    if (frames > runtime->buffer_size)
        frames = runtime->buffer_size;

    offset = frames_to_bytes(runtime, first);
    if (first + frames <= runtime->buffer_size) {
        size = frames_to_bytes(runtime, frames);
        t2audio_dma_memset(buf, offset, 0, size);
        return;
    }

    size = frames_to_bytes(runtime, runtime->buffer_size - first);
    t2audio_dma_memset(buf, offset, 0, size);
    t2audio_dma_memset(buf, 0, 0, frames_to_bytes(runtime, frames - (runtime->buffer_size - first)));
}

/*
 * How far the erase head trails the reported hardware pointer, in frames
 * (~21ms at 48kHz). The pointer is an interpolated estimate, not real DMA
 * readback, and can transiently run a few hundred frames ahead of the
 * device's true read position while the message-cadence estimate converges
 * after a real rate deviation (see msg_interval_ns in audio.h); erasing
 * right up to the estimate would zero frames the device has not played
 * yet. Trailing by this margin is immaterial for the erase head's purpose
 * (clearing already-consumed samples before route changes).
 */
#define T2AUDIO_ERASE_MARGIN_FRAMES 1024

static void t2audio_pcm_erase_played_frames(struct snd_pcm_substream *substream,
        snd_pcm_uframes_t hw_ptr)
{
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    struct snd_pcm_runtime *runtime = substream->runtime;
    snd_pcm_uframes_t frames;

    if (substream->stream != SNDRV_PCM_STREAM_PLAYBACK || !runtime || !runtime->buffer_size)
        return;

    /*
     * IOAudioFamily keeps an erase head for output engines and clears samples
     * once the hardware position has passed them. Codec Output appears to be
     * sensitive to stale samples after route changes, so keep the BridgeOS
     * playback ring in the same consumed-is-silent state.
     */
    hw_ptr %= runtime->buffer_size;
    if (runtime->buffer_size > 2 * T2AUDIO_ERASE_MARGIN_FRAMES)
        hw_ptr = (hw_ptr + runtime->buffer_size - T2AUDIO_ERASE_MARGIN_FRAMES) % runtime->buffer_size;
    if (!stream->erase_head_valid) {
        stream->erase_head = 0;
        stream->erase_head_valid = true;
    }

    if (hw_ptr >= stream->erase_head)
        frames = hw_ptr - stream->erase_head;
    else
        frames = runtime->buffer_size - stream->erase_head + hw_ptr;

    /*
     * A span of more than half the ring means the margin-lagged target is
     * still *behind* the erase head (e.g. right after a restart, before
     * the pointer has advanced past the margin) — wait instead of
     * wrapping around and zeroing nearly the whole ring including
     * unplayed frames.
     */
    if (frames > runtime->buffer_size / 2)
        return;

    t2audio_pcm_zero_frames(substream, stream->erase_head, frames);
    stream->erase_head = hw_ptr;
}

static int t2audio_pcm_open(struct snd_pcm_substream *substream)
{
    pr_debug("t2audio_pcm_open\n");
    substream->runtime->hw = *t2audio_pcm_stream(substream)->alsa_hw_desc;

    return 0;
}

static int t2audio_pcm_close(struct snd_pcm_substream *substream)
{
    pr_debug("t2audio_pcm_close\n");
    return 0;
}

static int t2audio_pcm_prepare(struct snd_pcm_substream *substream)
{
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);

    stream->waiting_for_first_ts = true;
    stream->remote_timestamp = 0;
    stream->frame_min = stream->latency;
    stream->erase_head = 0;
    stream->erase_head_valid = false;

    if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK && substream->runtime->buffer_size)
        t2audio_pcm_zero_frames(substream, 0, substream->runtime->buffer_size);

    return 0;
}

static int t2audio_pcm_hw_params(struct snd_pcm_substream *substream, struct snd_pcm_hw_params *hw_params)
{
    struct t2audio_stream *astream = t2audio_pcm_stream(substream);
    pr_debug("t2audio_pcm_hw_params\n");

    if (!astream->buffer_cnt || !astream->buffers)
        return -EINVAL;

    substream->runtime->dma_area = astream->buffers[0].ptr;
    substream->runtime->dma_addr = astream->buffers[0].dma_addr;
    substream->runtime->dma_bytes = astream->buffers[0].size;
    return 0;
}

static int t2audio_pcm_hw_free(struct snd_pcm_substream *substream)
{
    pr_debug("t2audio_pcm_hw_free\n");
    return 0;
}

static int t2audio_pcm_start(struct snd_pcm_substream *substream)
{
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    struct t2audio_dma_buf *dmabuf = t2audio_pcm_dma_buf(stream);
    void *buf = NULL;
    size_t s = 0;
    ktime_t time_start, time_end;
    bool back_buffer;
    int status;

    time_start = ktime_get();

    back_buffer = (substream->stream == SNDRV_PCM_STREAM_PLAYBACK);

    if (back_buffer) {
        snd_pcm_uframes_t appl_ptr;

        if (!dmabuf)
            return -EINVAL;
        if (!substream->runtime->buffer_size)
            return -EINVAL;

        appl_ptr = substream->runtime->control->appl_ptr % substream->runtime->buffer_size;
        s = frames_to_bytes(substream->runtime, appl_ptr);
        if (s) {
            buf = kmalloc(s, GFP_KERNEL);
            if (!buf)
                return -ENOMEM;

            t2audio_dma_copy_from(dmabuf, buf, 0, s);
            time_end = ktime_get();
            pr_debug("t2bce_audio: Backed up the buffer in %lluns [%li]\n", ktime_to_ns(time_end - time_start),
                    appl_ptr);
        }
    }

    stream->waiting_for_first_ts = true;
    stream->frame_min = stream->latency;
    stream->erase_head = 0;
    stream->erase_head_valid = false;
    stream->hw_frames_since_start = 0;
    stream->hw_accum_valid = false;

    status = t2audio_cmd_start_io(sdev->a, sdev->dev_id);
    if (back_buffer && buf)
        t2audio_dma_copy_to(dmabuf, 0, buf, s);
    kfree(buf);

    if (status)
        return status;

    stream->start_io_time = ktime_get_boottime();

    time_end = ktime_get();
    pr_debug("t2bce_audio: start_io %s %lld us\n",
            sdev->uid, ktime_to_us(ktime_sub(time_end, time_start)));
    return 0;
}

static int t2audio_pcm_trigger(struct snd_pcm_substream *substream, int cmd)
{
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    int err;

    pr_debug("t2audio_pcm_trigger %x\n", cmd);

    /* bridgeOS exposes one ALSA substream per remote stream. */
    if (substream->number != 0)
        return 0;
    switch (cmd) {
        case SNDRV_PCM_TRIGGER_START:
            pr_debug("t2bce_audio: TRIGGER START %s\n", sdev->uid);
            err = t2audio_pcm_start(substream);
            if (err)
                return err;
            stream->started = 1;
            break;
        case SNDRV_PCM_TRIGGER_STOP:
            pr_debug("t2bce_audio: TRIGGER STOP %s\n", sdev->uid);
            t2audio_cmd_stop_io(sdev->a, sdev->dev_id);
            stream->started = 0;
            stream->erase_head_valid = false;
            break;
        default:
            return -EINVAL;
    }
    return 0;
}

static snd_pcm_uframes_t t2audio_pcm_pointer(struct snd_pcm_substream *substream)
{
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    ktime_t time_from_start;
    snd_pcm_sframes_t frames;
    snd_pcm_sframes_t buffer_time_length;
    s64 cycle_ns;

    if (!stream->started)
        return 0;

    /*
     * Re-anchor on every bridgeOS timestamp update (roughly once per
     * ring cycle, ~346ms at 48kHz). Until the first update arrives, fall
     * back to the host clock captured at start_io completion — the DMA
     * may have been running for tens of milliseconds by then (especially
     * on codec outputs), so reporting 0 would starve the pipeline.
     * Anchoring only once at stream start and free-running from there is
     * not an option: a single stale anchor accumulates unbounded drift
     * over the stream lifetime (observed as a silent multi-minute
     * dropout with hw_ptr advancing at a perfect, fictitious 48000Hz).
     * stream->remote_timestamp is host-clock-domain (see
     * t2audio_handle_stream_timestamp()), so re-anchoring on it here
     * never injects a cross-clock-domain error.
     */
    if (stream->waiting_for_first_ts)
        time_from_start = ktime_get_boottime() - stream->start_io_time;
    else
        time_from_start = ktime_get_boottime() - stream->remote_timestamp;

    /*
     * bridgeOS reports coarse timestamps; interpolate the ALSA pointer
     * locally. Interpolate over the *measured* message cadence
     * (msg_interval_ns, see audio.h), not the nominal buffer duration:
     * the device's real consumption rate can deviate several percent
     * from nominal for seconds around a resume, and sweeping at the
     * nominal rate through such a window pushes the estimate ahead of
     * the device's true read position within every cycle. The clamp
     * bounds the sweep rate even if the estimate is corrupted; the
     * frame_min ratchet below (holding the pointer flat instead of ever
     * moving it backward across an anchor reset) is unchanged and works
     * the same over the measured cycle length.
     */
    buffer_time_length = NSEC_PER_SEC * substream->runtime->buffer_size / substream->runtime->rate;
    cycle_ns = stream->msg_interval_ns;
    if (cycle_ns)
        cycle_ns = clamp_t(s64, cycle_ns, buffer_time_length * 3 / 4, buffer_time_length * 3 / 2);
    else
        cycle_ns = buffer_time_length;
    frames = (ktime_to_ns(time_from_start) % cycle_ns) * substream->runtime->buffer_size / cycle_ns;
    if (ktime_to_ns(time_from_start) < cycle_ns) {
        if (frames < stream->frame_min)
            frames = stream->frame_min;
        else
            stream->frame_min = 0;
    } else {
        if (ktime_to_ns(time_from_start) < 2 * cycle_ns)
            stream->frame_min = frames;
        else
            stream->frame_min = 0;
    }
    frames -= stream->latency;
    if (frames < 0)
        frames += ((-frames - 1) / substream->runtime->buffer_size + 1) * substream->runtime->buffer_size;
    frames %= substream->runtime->buffer_size;

    /*
     * The estimate above is pure clock interpolation with no readback of
     * real DMA progress, so nothing stops it from claiming more frames
     * were consumed than the application has actually written — a real
     * DMA engine can't outrun the data it was given, but this software
     * stand-in can (e.g. while the stream drains, or if the anchor is
     * off right after resume). Bound it with a monotonic accumulator fed
     * by small incremental deltas: each poll adds min(raw wrapped delta
     * since the last poll, current backlog), where backlog = appl_ptr
     * (host-side, not device-clock-dependent) minus what the accumulator
     * has claimed so far. By construction the accumulator can never
     * exceed appl_ptr, regardless of how wrong the raw estimate is, and
     * it only ever advances in small steps — so a big reported-position
     * swing can't be misread as a ring wraparound by ALSA's own hw_ptr
     * tracking.
     */
    if (substream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
        snd_pcm_uframes_t appl_now = substream->runtime->control->appl_ptr;
        snd_pcm_sframes_t raw_delta;
        u64 backlog, delta;

        if (!stream->hw_accum_valid) {
            stream->hw_accum_valid = true;
            stream->hw_accum_last_raw = 0;
            stream->hw_frames_since_start = 0;
        }

        raw_delta = (snd_pcm_sframes_t) frames - (snd_pcm_sframes_t) stream->hw_accum_last_raw;
        if (raw_delta < 0)
            raw_delta += substream->runtime->buffer_size;
        stream->hw_accum_last_raw = (snd_pcm_uframes_t) frames;

        backlog = (appl_now > stream->hw_frames_since_start) ? (appl_now - stream->hw_frames_since_start) : 0;
        delta = (u64) raw_delta < backlog ? (u64) raw_delta : backlog;
        if (delta < (u64) raw_delta)
            pr_debug_ratelimited("t2bce_audio: pointer capped %s raw_delta=%ld backlog=%llu\n",
                    sdev->uid, raw_delta, backlog);
        stream->hw_frames_since_start += delta;

        frames = (snd_pcm_sframes_t) (stream->hw_frames_since_start % substream->runtime->buffer_size);
    }

    t2audio_pcm_erase_played_frames(substream, (snd_pcm_uframes_t) frames);
    return (snd_pcm_uframes_t) frames;
}

static int t2audio_pcm_mmap(struct snd_pcm_substream *substream, struct vm_area_struct *area)
{
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);
    struct t2audio_stream *stream = t2audio_pcm_stream(substream);
    struct t2audio_dma_buf *buf;

    if (!stream->buffer_cnt || !stream->buffers)
        return -EINVAL;

    buf = &stream->buffers[0];
    switch (buf->type) {
        case T2AUDIO_DMA_BUF_IOMEM:
            return snd_pcm_lib_mmap_iomem(substream, area);
        case T2AUDIO_DMA_BUF_COHERENT:
            return dma_mmap_coherent(sdev->a->dev, area, buf->ptr, buf->dma_addr, buf->size);
        default:
            return -EINVAL;
    }
}

static struct snd_pcm_ops t2audio_pcm_ops = {
        .open =        t2audio_pcm_open,
        .close =       t2audio_pcm_close,
        .ioctl =       snd_pcm_lib_ioctl,
        .hw_params =   t2audio_pcm_hw_params,
        .hw_free =     t2audio_pcm_hw_free,
        .prepare =     t2audio_pcm_prepare,
        .trigger =     t2audio_pcm_trigger,
        .pointer =     t2audio_pcm_pointer,
        .mmap    =     t2audio_pcm_mmap
};

int t2audio_create_pcm(struct t2audio_subdevice *sdev)
{
    struct snd_pcm *pcm;
    struct t2audio_alsa_pcm_id_mapping *id_mapping;
    int err;

    if (!sdev->is_pcm || (sdev->in_stream_cnt == 0 && sdev->out_stream_cnt == 0)) {
        return -EINVAL;
    }

    for (id_mapping = t2audio_alsa_id_mappings; id_mapping->name; id_mapping++) {
        if (!strcmp(sdev->uid, id_mapping->name)) {
            sdev->alsa_id = id_mapping->alsa_id;
            break;
        }
    }
    if (!id_mapping->name)
        sdev->alsa_id = sdev->a->next_alsa_id++;
    err = snd_pcm_new(sdev->a->card, sdev->uid, sdev->alsa_id,
            (int) sdev->out_stream_cnt, (int) sdev->in_stream_cnt, &pcm);
    if (err < 0)
        return err;
    pcm->private_data = sdev;
    pcm->nonatomic = 1;
    sdev->pcm = pcm;
    strcpy(pcm->name, sdev->uid);
    snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_PLAYBACK, &t2audio_pcm_ops);
    snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_CAPTURE, &t2audio_pcm_ops);
    return 0;
}

static void t2audio_handle_stream_timestamp(struct snd_pcm_substream *substream,
                                            ktime_t os_timestamp, u64 dev_timestamp)
{
    unsigned long flags;
    struct t2audio_stream *stream;
    struct t2audio_subdevice *sdev = snd_pcm_substream_chip(substream);

    stream = t2audio_pcm_stream(substream);
    snd_pcm_stream_lock_irqsave(substream, flags);
    /*
     * Feed the message-cadence estimate (see msg_interval_ns in
     * audio.h). Host-domain intervals are used because the pointer
     * anchor is also host-domain (os_timestamp) — the sweep duration to
     * the next anchor must be measured in the same domain it is
     * interpolated in. Intervals outside (nominal/2, nominal*2) are
     * dropped entirely rather than averaged in: those are missed/
     * duplicated messages or restart artifacts, not a real rate change
     * (the largest genuine deviation observed is ~8%).
     */
    if (!stream->waiting_for_first_ts &&
            substream->runtime && substream->runtime->rate && substream->runtime->buffer_size) {
        s64 nominal_ns = (s64) NSEC_PER_SEC * substream->runtime->buffer_size / substream->runtime->rate;
        s64 interval_ns = ktime_to_ns(ktime_sub(os_timestamp, stream->last_os_timestamp));

        if (interval_ns > nominal_ns / 2 && interval_ns < nominal_ns * 2) {
            if (!stream->msg_interval_ns)
                stream->msg_interval_ns = interval_ns;
            else
                stream->msg_interval_ns += (interval_ns - stream->msg_interval_ns) >> 2;
        }
        pr_debug("t2bce_audio: ts interval %s interval_us=%lld cyc_est_us=%lld\n",
                sdev->uid, interval_ns / 1000, stream->msg_interval_ns / 1000);
    }
    stream->last_os_timestamp = os_timestamp;

    /*
     * Anchor on os_timestamp (host ktime_get_boottime(), captured when
     * this message was processed), not dev_timestamp: the latter is
     * bridgeOS's own clock, and using it here reinterprets a foreign
     * clock domain as boottime-ns. Right after a real suspend/resume the
     * two domains can disagree, and a device-domain anchor then makes the
     * interpolated pointer glitch, which makes PipeWire recover-restart
     * the stream every ~85ms — much faster than the ~346ms timestamp
     * message cadence, so most restarts run entirely on the
     * waiting_for_first_ts -> start_io_time fallback and the storm
     * sustains itself. With a host-domain anchor the fallback and the
     * real anchor are self-consistent by construction (both are
     * ktime_get_boottime()-domain), so the pointer jump at the handoff is
     * only ever the elapsed fallback duration, never a cross-domain
     * epoch/rate error.
     */
    stream->remote_timestamp = ktime_to_ns(os_timestamp);
    if (stream->waiting_for_first_ts) {
        pr_debug("t2bce_audio: first ts on %s (t2=%lld host=%lld)\n",
                sdev->uid, dev_timestamp, ktime_to_ns(os_timestamp));
        stream->waiting_for_first_ts = false;
        snd_pcm_stream_unlock_irqrestore(substream, flags);
        return;
    }
    snd_pcm_stream_unlock_irqrestore(substream, flags);
    if (!substream->runtime->no_period_wakeup)
        snd_pcm_period_elapsed(substream);
}

void t2audio_handle_timestamp(struct t2audio_subdevice *sdev, ktime_t os_timestamp, u64 dev_timestamp)
{
    struct snd_pcm_substream *substream;

    substream = sdev->pcm->streams[SNDRV_PCM_STREAM_PLAYBACK].substream;
    if (substream)
        t2audio_handle_stream_timestamp(substream, os_timestamp, dev_timestamp);
    substream = sdev->pcm->streams[SNDRV_PCM_STREAM_CAPTURE].substream;
    if (substream)
        t2audio_handle_stream_timestamp(substream, os_timestamp, dev_timestamp);
}
